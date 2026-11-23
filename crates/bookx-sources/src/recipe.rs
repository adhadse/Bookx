//! News recipes (S4/S5): a declarative description of a periodic news book, the scheduler maths,
//! and [`build_news_epub`] which turns feeds (and optionally the full articles) into an EPUB with
//! one section per feed.
//!
//! All network access goes through a [`Fetcher`]; time is always passed in (`now`), so builds and
//! schedules are reproducible in tests.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::{Context, Result, anyhow, bail};
use bookx_core::archive::Archive;
use bookx_core::convert::{Chapter, create_epub};
use bookx_core::opf::Metadata;
use bookx_core::util::xml_escape;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc, Weekday};
use regex::Regex;
use scraper::Selector;
use serde::{Deserialize, Serialize};

use crate::cover::generate_cover;
use crate::feed::{Entry, Feed, decode_text, parse_feed};
use crate::fetch::Fetcher;
use crate::readability::{
    ExtractOptions, ImageLimits, add_images_to_epub, clean_fragment, extract_with, fetch_images,
    rewrite_images, write_full_metadata,
};

// ---------------------------------------------------------------------------------------------
// data model
// ---------------------------------------------------------------------------------------------

fn d_max_articles() -> usize {
    25
}
fn d_oldest_days() -> u32 {
    7
}
fn d_title_template() -> String {
    "{title} - {date}".into()
}
fn d_true() -> bool {
    true
}
fn d_max_image_bytes() -> usize {
    1024 * 1024
}
fn d_max_images() -> usize {
    80
}
fn d_language() -> String {
    "en".into()
}

/// One feed of a recipe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedSpec {
    pub url: String,
    /// Section title; defaults to the feed's own title.
    #[serde(default)]
    pub title: Option<String>,
    /// Overrides the recipe's per-feed limit.
    #[serde(default)]
    pub max_articles: Option<usize>,
    /// Overrides [`Recipe::use_full_article`] for this feed: `Some(true)` downloads each article's
    /// page, `Some(false)` uses the feed text, `None` follows the recipe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_article: Option<bool>,
}

impl FeedSpec {
    /// Whether the articles of this feed are downloaded in full under `recipe`.
    pub fn wants_full_article(&self, recipe: &Recipe) -> bool {
        self.full_article.unwrap_or(recipe.use_full_article)
    }
}

/// When a recipe runs. Times are UTC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    Hourly {
        #[serde(default)]
        minute: u32,
    },
    Daily {
        hour: u32,
        #[serde(default)]
        minute: u32,
    },
    Weekly {
        weekday: Weekday,
        hour: u32,
        #[serde(default)]
        minute: u32,
    },
    /// Five fields: `minute hour day-of-month month day-of-week`; see [`CronSpec`].
    Cron { expr: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub title: String,
    pub feeds: Vec<FeedSpec>,
    /// Articles per feed.
    #[serde(default = "d_max_articles")]
    pub max_articles: usize,
    /// Entries older than this many days are skipped; `0` keeps everything. Undated entries are kept.
    #[serde(default = "d_oldest_days")]
    pub oldest_days: u32,
    /// Regexes; when non-empty an entry must match one of them (title or link).
    #[serde(default)]
    pub include: Vec<String>,
    /// Regexes; entries matching any (title or link) are skipped.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Download each article's web page and extract it, instead of using the feed text.
    #[serde(default)]
    pub use_full_article: bool,
    /// CSS selectors: when given, only these parts of a full article are kept.
    #[serde(default)]
    pub keep_selectors: Vec<String>,
    /// CSS selectors removed from full articles.
    #[serde(default)]
    pub remove_selectors: Vec<String>,
    /// `{title}`, `{date}`, `{date:%d %B %Y}`, `{datetime}`, `{year}`, `{month}`, `{day}`, `{weekday}`.
    #[serde(default = "d_title_template")]
    pub title_template: String,
    #[serde(default = "d_true")]
    pub include_images: bool,
    #[serde(default = "d_max_image_bytes")]
    pub max_image_bytes: usize,
    /// Images in the whole book.
    #[serde(default = "d_max_images")]
    pub max_images: usize,
    #[serde(default = "d_language")]
    pub language: String,
    #[serde(default)]
    pub schedule: Option<Schedule>,
}

impl Recipe {
    pub fn new(title: &str, feed_urls: &[&str]) -> Recipe {
        Recipe {
            title: title.into(),
            feeds: feed_urls
                .iter()
                .map(|u| FeedSpec {
                    url: u.to_string(),
                    title: None,
                    max_articles: None,
                    full_article: None,
                })
                .collect(),
            max_articles: d_max_articles(),
            oldest_days: d_oldest_days(),
            include: vec![],
            exclude: vec![],
            use_full_article: false,
            keep_selectors: vec![],
            remove_selectors: vec![],
            title_template: d_title_template(),
            include_images: true,
            max_image_bytes: d_max_image_bytes(),
            max_images: d_max_images(),
            language: d_language(),
            schedule: None,
        }
    }

    pub fn from_toml(s: &str) -> Result<Recipe> {
        let r: Recipe = toml::from_str(s).context("invalid recipe TOML")?;
        r.validate()?;
        Ok(r)
    }

    pub fn from_json(s: &str) -> Result<Recipe> {
        let r: Recipe = serde_json::from_str(s).context("invalid recipe JSON")?;
        r.validate()?;
        Ok(r)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self).context("serialising recipe")
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("serialising recipe")
    }

    /// Checks everything that can be checked without the network.
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            bail!("recipe has no title");
        }
        if self.feeds.is_empty() {
            bail!("recipe has no feeds");
        }
        for f in &self.feeds {
            let u = url::Url::parse(&f.url)
                .with_context(|| format!("feed URL `{}` is invalid", f.url))?;
            if !matches!(u.scheme(), "http" | "https") {
                bail!("feed URL `{}` must be http or https", f.url);
            }
        }
        if self.max_articles == 0 {
            bail!("max_articles must be at least 1");
        }
        for r in self.include.iter().chain(&self.exclude) {
            Regex::new(r).with_context(|| format!("invalid regex `{r}`"))?;
        }
        for s in self.keep_selectors.iter().chain(&self.remove_selectors) {
            Selector::parse(s).map_err(|e| anyhow!("invalid CSS selector `{s}`: {e}"))?;
        }
        if let Some(s) = &self.schedule {
            s.to_cron().context("invalid schedule")?;
        }
        Ok(())
    }

    /// The book title for `now`, from `title_template`.
    pub fn render_title(&self, now: DateTime<Utc>) -> String {
        render_title_template(&self.title_template, &self.title, now)
    }
}

/// Expands `{title}`, `{date}`, `{date:FORMAT}`, `{datetime}`, `{year}`, `{month}`, `{day}`, `{weekday}`.
/// Unknown placeholders and invalid date formats are left as they are.
pub fn render_title_template(template: &str, title: &str, now: DateTime<Utc>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let Some(len) = rest[start..].find('}') else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = &rest[start + 1..start + len];
        let replaced = match tag {
            "title" => Some(title.to_string()),
            "date" => Some(now.format("%Y-%m-%d").to_string()),
            "datetime" => Some(now.format("%Y-%m-%d %H:%M").to_string()),
            "year" => Some(now.format("%Y").to_string()),
            "month" => Some(now.format("%B").to_string()),
            "day" => Some(now.format("%d").to_string()),
            "weekday" => Some(now.format("%A").to_string()),
            t => t.strip_prefix("date:").and_then(|f| {
                let bad =
                    chrono::format::StrftimeItems::new(f).any(|i| i == chrono::format::Item::Error);
                if bad {
                    None
                } else {
                    let mut s = String::new();
                    write!(s, "{}", now.format(f)).ok().map(|_| s)
                }
            }),
        };
        match replaced {
            Some(r) => out.push_str(&r),
            None => out.push_str(&rest[start..start + len + 1]),
        }
        rest = &rest[start + len + 1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------------------------
// scheduling
// ---------------------------------------------------------------------------------------------

/// A parsed five-field cron expression (UTC). Supports `*`, `n`, `a-b`, `a,b`, `*/n`, `a-b/n`,
/// month names (`jan`) and weekday names (`mon`); weekday `0` and `7` are Sunday. When both
/// day-of-month and weekday are restricted, either may match (classic cron).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    minutes: u64,
    hours: u32,
    dom: u32,
    months: u16,
    dow: u8,
    dom_any: bool,
    dow_any: bool,
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DOW_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn parse_field(
    s: &str,
    min: u32,
    max: u32,
    names: &[&str],
    name_base: u32,
    what: &str,
) -> Result<(u64, bool)> {
    let value = |t: &str| -> Result<u32> {
        let t = t.trim();
        if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(t)) {
            return Ok(i as u32 + name_base);
        }
        let v: u32 = t.parse().map_err(|_| anyhow!("bad {what} value `{t}`"))?;
        Ok(v)
    };
    let mut mask = 0u64;
    let mut star = false;
    for item in s.split(',') {
        if item.trim().is_empty() {
            bail!("empty item in {what} field `{s}`");
        }
        let (range, step) = match item.split_once('/') {
            Some((r, st)) => {
                let step: u32 = st
                    .trim()
                    .parse()
                    .map_err(|_| anyhow!("bad step `{st}` in {what} field"))?;
                if step == 0 {
                    bail!("step 0 in {what} field");
                }
                (r, step)
            }
            None => (item, 1),
        };
        let (lo, hi) = if range.trim() == "*" {
            if item.trim() == "*" {
                star = true;
            }
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let v = value(range)?;
            if item.contains('/') { (v, max) } else { (v, v) }
        };
        if lo > hi || lo < min || hi > max {
            bail!("{what} value out of range in `{item}` (allowed {min}-{max})");
        }
        let mut v = lo;
        while v <= hi {
            mask |= 1u64 << v;
            v += step;
        }
    }
    Ok((mask, star))
}

impl CronSpec {
    pub fn parse(expr: &str) -> Result<CronSpec> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            bail!(
                "cron expression needs 5 fields (minute hour day month weekday), got {}",
                fields.len()
            );
        }
        let (minutes, _) = parse_field(fields[0], 0, 59, &[], 0, "minute")?;
        let (hours, _) = parse_field(fields[1], 0, 23, &[], 0, "hour")?;
        let (dom, dom_star) = parse_field(fields[2], 1, 31, &[], 0, "day-of-month")?;
        let (months, _) = parse_field(fields[3], 1, 12, &MONTH_NAMES, 1, "month")?;
        let (mut dow, dow_star) = parse_field(fields[4], 0, 7, &DOW_NAMES, 0, "weekday")?;
        if dow & (1 << 7) != 0 {
            dow = (dow & !(1 << 7)) | 1;
        }
        Ok(CronSpec {
            minutes,
            hours: hours as u32,
            dom: dom as u32,
            months: months as u16,
            dow: dow as u8,
            dom_any: dom_star,
            dow_any: dow_star,
        })
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if self.months & (1 << d.month()) == 0 {
            return false;
        }
        let dom_ok = self.dom & (1 << d.day()) != 0;
        let dow_ok = self.dow & (1 << d.weekday().num_days_from_sunday()) != 0;
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (false, true) => dom_ok,
            (true, false) => dow_ok,
            (false, false) => dom_ok || dow_ok,
        }
    }

    /// First matching minute strictly after `t`, or `None` when the expression never fires
    /// within nine years (`0 0 31 2 *`).
    pub fn next_after(&self, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = t.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let first_day = start.date_naive();
        for offset in 0..(366 * 9) {
            let day = first_day + Duration::days(offset);
            if !self.day_matches(day) {
                continue;
            }
            let (h0, m0) = if offset == 0 {
                (start.hour(), start.minute())
            } else {
                (0, 0)
            };
            for h in h0..24 {
                if self.hours & (1 << h) == 0 {
                    continue;
                }
                let from = if h == h0 { m0 } else { 0 };
                if let Some(m) = (from..60).find(|m| self.minutes & (1u64 << m) != 0) {
                    return day.and_hms_opt(h, m, 0).map(|d| d.and_utc());
                }
            }
        }
        None
    }
}

impl Schedule {
    /// The equivalent cron expression; errors for out-of-range times.
    pub fn to_cron(&self) -> Result<CronSpec> {
        let check = |hour: u32, minute: u32| -> Result<()> {
            if hour > 23 {
                bail!("hour {hour} is out of range (0-23)");
            }
            if minute > 59 {
                bail!("minute {minute} is out of range (0-59)");
            }
            Ok(())
        };
        match self {
            Schedule::Hourly { minute } => {
                check(0, *minute)?;
                CronSpec::parse(&format!("{minute} * * * *"))
            }
            Schedule::Daily { hour, minute } => {
                check(*hour, *minute)?;
                CronSpec::parse(&format!("{minute} {hour} * * *"))
            }
            Schedule::Weekly {
                weekday,
                hour,
                minute,
            } => {
                check(*hour, *minute)?;
                CronSpec::parse(&format!(
                    "{minute} {hour} * * {}",
                    weekday.num_days_from_sunday()
                ))
            }
            Schedule::Cron { expr } => CronSpec::parse(expr),
        }
    }

    /// First scheduled time strictly after `t`.
    pub fn next_after(&self, t: DateTime<Utc>) -> Result<DateTime<Utc>> {
        self.to_cron()?
            .next_after(t)
            .ok_or_else(|| anyhow!("schedule never fires"))
    }
}

/// When to run next. A recipe that never ran is due immediately; otherwise the first slot after
/// `last_run`, and if that slot has already passed the recipe is overdue and due `now` (missed
/// runs are not repeated).
pub fn next_run(
    schedule: &Schedule,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    match last_run {
        None => {
            schedule.to_cron()?;
            Ok(now)
        }
        Some(last) => {
            let slot = schedule.next_after(last)?;
            Ok(if slot <= now { now } else { slot })
        }
    }
}

/// Whether a run is due at `now`.
pub fn is_due(
    schedule: &Schedule,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<bool> {
    Ok(next_run(schedule, last_run, now)? <= now)
}

// ---------------------------------------------------------------------------------------------
// building
// ---------------------------------------------------------------------------------------------

/// What happened during a build.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewsStats {
    pub feeds_ok: usize,
    pub feeds_failed: usize,
    pub articles: usize,
    pub full_articles: usize,
    pub skipped_old: usize,
    pub skipped_filtered: usize,
    pub skipped_duplicate: usize,
    pub images: usize,
}

#[derive(Debug, Clone)]
pub struct NewsEpub {
    pub archive: Archive,
    pub title: String,
    pub stats: NewsStats,
    /// Non-fatal problems (a feed that failed, an article that could not be extracted).
    pub errors: Vec<String>,
}

fn compile_all(list: &[String]) -> Result<Vec<Regex>> {
    list.iter()
        .map(|r| Regex::new(r).with_context(|| format!("invalid regex `{r}`")))
        .collect()
}

/// Applies date, include/exclude and duplicate filters and the per-feed limit; newest first.
pub fn select_entries(
    recipe: &Recipe,
    feed: &Feed,
    limit: usize,
    now: DateTime<Utc>,
    seen: &mut HashSet<String>,
    stats: &mut NewsStats,
) -> Result<Vec<Entry>> {
    let include = compile_all(&recipe.include)?;
    let exclude = compile_all(&recipe.exclude)?;
    let cutoff = (recipe.oldest_days > 0).then(|| now - Duration::days(recipe.oldest_days as i64));
    let mut picked: Vec<Entry> = Vec::new();
    for e in &feed.entries {
        if e.title.trim().is_empty() && e.link.trim().is_empty() {
            continue;
        }
        if let (Some(c), Some(d)) = (cutoff, e.date) {
            if d < c {
                stats.skipped_old += 1;
                continue;
            }
        }
        let hit = |res: &[Regex]| {
            res.iter()
                .any(|r| r.is_match(&e.title) || r.is_match(&e.link))
        };
        if (!include.is_empty() && !hit(&include)) || hit(&exclude) {
            stats.skipped_filtered += 1;
            continue;
        }
        let key = if e.link.is_empty() {
            e.id.clone()
        } else {
            e.link.clone()
        };
        if !key.is_empty() && seen.contains(&key) {
            stats.skipped_duplicate += 1;
            continue;
        }
        picked.push(e.clone());
    }
    // newest first; undated entries keep their feed order after the dated ones
    picked.sort_by(|a, b| match (a.date, b.date) {
        (Some(x), Some(y)) => y.cmp(&x),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    picked.truncate(limit);
    for e in &picked {
        let key = if e.link.is_empty() {
            e.id.clone()
        } else {
            e.link.clone()
        };
        if !key.is_empty() {
            seen.insert(key);
        }
    }
    Ok(picked)
}

struct Item {
    title: String,
    author: String,
    date: Option<DateTime<Utc>>,
    link: String,
    content: String,
}

struct Section {
    title: String,
    items: Vec<Item>,
}

fn esc(s: &str) -> String {
    xml_escape(s).replace('<', "&lt;").replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

fn item_body(it: &Item) -> String {
    let mut meta: Vec<String> = Vec::new();
    if !it.author.is_empty() {
        meta.push(esc(&it.author));
    }
    if let Some(d) = it.date {
        meta.push(d.format("%Y-%m-%d").to_string());
    }
    let mut b = format!("<h1>{}</h1>\n", esc(&it.title));
    if !meta.is_empty() {
        let _ = writeln!(
            b,
            "<p class=\"byline\"><em>{}</em></p>",
            meta.join(" \u{00B7} ")
        );
    }
    if !it.link.is_empty() {
        let _ = writeln!(
            b,
            "<p class=\"source\"><a href=\"{}\">{}</a></p>",
            escape_attr(&it.link),
            esc(&it.link)
        );
    }
    b.push_str(&it.content);
    b
}

/// Builds the nested navigation document and NCX for sections of `(section file, [article files])`.
/// `(section title, section file, [(article title, article file)])`
type TocSection = (String, String, Vec<(String, String)>);

fn write_nested_toc(archive: &mut Archive, sections: &[TocSection]) -> Result<()> {
    let uid = archive
        .get_string("OEBPS/toc.ncx")
        .and_then(|n| {
            Regex::new(r#"name="dtb:uid" content="([^"]*)""#)
                .ok()?
                .captures(&n)
                .map(|c| c[1].to_string())
        })
        .ok_or_else(|| anyhow!("generated EPUB has no NCX uid"))?;
    let mut nav = String::new();
    let mut ncx = String::new();
    let mut order = 0;
    for (title, file, kids) in sections {
        order += 1;
        let _ = write!(nav, "      <li><a href=\"{file}\">{}</a>", esc(title));
        let _ = writeln!(
            ncx,
            "    <navPoint id=\"np{order}\" playOrder=\"{order}\"><navLabel><text>{}</text></navLabel><content src=\"{file}\"/>",
            esc(title)
        );
        if !kids.is_empty() {
            nav.push_str("\n        <ol>\n");
            for (kt, kf) in kids {
                order += 1;
                let _ = writeln!(nav, "          <li><a href=\"{kf}\">{}</a></li>", esc(kt));
                let _ = writeln!(
                    ncx,
                    "      <navPoint id=\"np{order}\" playOrder=\"{order}\"><navLabel><text>{}</text></navLabel><content src=\"{kf}\"/></navPoint>",
                    esc(kt)
                );
            }
            nav.push_str("        </ol>\n      ");
        }
        nav.push_str("</li>\n");
        ncx.push_str("    </navPoint>\n");
    }
    let title = archive
        .get_string("OEBPS/toc.ncx")
        .and_then(|n| {
            Regex::new(r"<docTitle><text>(.*?)</text></docTitle>")
                .ok()?
                .captures(&n)
                .map(|c| c[1].to_string())
        })
        .unwrap_or_default();
    archive.set_string(
        "OEBPS/nav.xhtml",
        &format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head><title>Contents</title></head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n    <ol>\n{nav}    </ol>\n  </nav>\n</body>\n</html>\n"
        ),
    );
    archive.set_string(
        "OEBPS/toc.ncx",
        &format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\">\n  <head>\n    <meta name=\"dtb:uid\" content=\"{uid}\"/>\n    <meta name=\"dtb:depth\" content=\"2\"/>\n  </head>\n  <docTitle><text>{title}</text></docTitle>\n  <navMap>\n{ncx}  </navMap>\n</ncx>\n"
        ),
    );
    Ok(())
}

fn section_index_body(title: &str, items: &[(String, String)]) -> String {
    let mut b = format!("<h1>{}</h1>\n<ul>\n", esc(title));
    for (t, f) in items {
        let _ = writeln!(b, "<li><a href=\"{f}\">{}</a></li>", esc(t));
    }
    b.push_str("</ul>\n");
    b
}

/// Fetches the recipe's feeds (and articles), and builds the EPUB: a cover, one section per feed
/// listing its articles, then the articles. The table of contents is nested (feed → articles).
///
/// Feeds or articles that fail are recorded in `errors` and skipped; the build fails only when
/// not a single article could be collected.
pub fn build_news_epub(
    recipe: &Recipe,
    fetcher: &dyn Fetcher,
    now: DateTime<Utc>,
) -> Result<NewsEpub> {
    recipe.validate()?;
    let mut stats = NewsStats::default();
    let mut errors: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut sections: Vec<Section> = Vec::new();
    let opts = ExtractOptions {
        keep: recipe.keep_selectors.clone(),
        remove: recipe.remove_selectors.clone(),
        ..Default::default()
    };

    for spec in &recipe.feeds {
        let feed = match fetcher.get(&spec.url, &[]).and_then(|b| parse_feed(&b)) {
            Ok(f) => {
                stats.feeds_ok += 1;
                f
            }
            Err(e) => {
                stats.feeds_failed += 1;
                errors.push(format!("feed {}: {e:#}", spec.url));
                continue;
            }
        };
        let limit = spec.max_articles.unwrap_or(recipe.max_articles).max(1);
        let entries = select_entries(recipe, &feed, limit, now, &mut seen, &mut stats)?;
        let section_title = spec
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| Some(feed.title.clone()).filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| {
                url::Url::parse(&spec.url)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_else(|| spec.url.clone())
            });
        let base = if feed.link.is_empty() {
            spec.url.clone()
        } else {
            feed.link.clone()
        };
        let mut items = Vec::new();
        for e in entries {
            let link_base = if e.link.is_empty() {
                base.clone()
            } else {
                e.link.clone()
            };
            let mut content = String::new();
            if spec.wants_full_article(recipe) && !e.link.is_empty() {
                match fetcher
                    .get(&e.link, &[])
                    .map(|b| decode_text(&b))
                    .and_then(|html| extract_with(&html, &e.link, &opts))
                {
                    Ok(a) => {
                        stats.full_articles += 1;
                        content = a.content;
                    }
                    Err(err) => errors.push(format!("article {}: {err:#}", e.link)),
                }
            }
            if content.is_empty() {
                let frag = clean_fragment(e.body(), &link_base);
                content = frag.content;
                if frag.text_len == 0 && !content.contains("<img") {
                    content = clean_fragment(&e.summary, &link_base).content;
                }
            }
            if !content.contains("<img") {
                if let Some(img) = e.enclosures.iter().find(|x| x.mime.starts_with("image/")) {
                    let resolved = crate::readability::resolve_url(&link_base, &img.url);
                    if let Some(u) = resolved {
                        content = format!(
                            "<p><img src=\"{}\" alt=\"\"/></p>\n{content}",
                            escape_attr(&u)
                        );
                    }
                }
            }
            if content.trim().is_empty() {
                content = if e.link.is_empty() {
                    "<p>(no text)</p>\n".into()
                } else {
                    format!(
                        "<p><a href=\"{}\">{}</a></p>\n",
                        escape_attr(&e.link),
                        esc(&e.link)
                    )
                };
            }
            items.push(Item {
                title: if e.title.trim().is_empty() {
                    e.link.clone()
                } else {
                    e.title.clone()
                },
                author: e.author.clone(),
                date: e.date,
                link: e.link.clone(),
                content,
            });
        }
        if !items.is_empty() {
            stats.articles += items.len();
            sections.push(Section {
                title: section_title,
                items,
            });
        }
    }

    if stats.articles == 0 {
        bail!(
            "no articles collected from {} feed(s){}",
            recipe.feeds.len(),
            if errors.is_empty() {
                String::new()
            } else {
                format!(": {}", errors.join("; "))
            }
        );
    }

    // chapter list: section index page, then its articles
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut toc: Vec<TocSection> = Vec::new();
    for s in &sections {
        let section_no = chapters.len() + 1;
        let file = |n: usize| format!("chapter-{n}.xhtml");
        let kids: Vec<(String, String)> = s
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.title.clone(), file(section_no + 1 + i)))
            .collect();
        chapters.push(Chapter {
            title: s.title.clone(),
            body: section_index_body(&s.title, &kids),
        });
        for it in &s.items {
            chapters.push(Chapter {
                title: it.title.clone(),
                body: item_body(it),
            });
        }
        toc.push((s.title.clone(), file(section_no), kids));
    }

    let title = recipe.render_title(now);
    let author_names: Vec<String> = sections.iter().take(3).map(|s| s.title.clone()).collect();
    let md = Metadata {
        title: title.clone(),
        authors: if author_names.is_empty() {
            vec!["Bookx News".into()]
        } else {
            author_names
        },
        publisher: "Bookx".into(),
        published: now.format("%Y-%m-%d").to_string(),
        language: recipe.language.clone(),
        description: format!("{} articles from {} feeds", stats.articles, sections.len()),
        tags: vec!["News".into()],
        ..Default::default()
    };
    let cover = generate_cover(
        &recipe.title,
        &now.format("%Y-%m-%d").to_string(),
        "",
        600,
        900,
    )?;
    let mut archive = create_epub(&md, &chapters, Some((cover, "png")));
    write_full_metadata(&mut archive, &md)?;
    write_nested_toc(&mut archive, &toc)?;

    if recipe.include_images {
        let mut urls: BTreeSet<(usize, String)> = BTreeSet::new();
        let mut order = 0;
        let re = Regex::new(r#"<img src="([^"]*)""#)?;
        for ch in &chapters {
            for c in re.captures_iter(&ch.body) {
                let u = c[1].replace("&amp;", "&");
                if !urls.iter().any(|(_, x)| *x == u) {
                    urls.insert((order, u));
                    order += 1;
                }
            }
        }
        let list: Vec<String> = urls.into_iter().map(|(_, u)| u).collect();
        let limits = ImageLimits {
            max_bytes_each: recipe.max_image_bytes,
            max_count: recipe.max_images,
        };
        let fetched = fetch_images(fetcher, &list, &limits);
        stats.images = fetched.len();
        let local: HashMap<String, String> = add_images_to_epub(&mut archive, &fetched, "news")?;
        for n in 1..=chapters.len() {
            let path = format!("OEBPS/chapter-{n}.xhtml");
            if let Some(text) = archive.get_string(&path) {
                archive.set_string(&path, &rewrite_images(&text, &local));
            }
        }
    } else {
        for n in 1..=chapters.len() {
            let path = format!("OEBPS/chapter-{n}.xhtml");
            if let Some(text) = archive.get_string(&path) {
                archive.set_string(&path, &rewrite_images(&text, &HashMap::new()));
            }
        }
    }

    Ok(NewsEpub {
        archive,
        title,
        stats,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};
    use bookx_core::book::OpenBook;
    use chrono::TimeZone;

    fn t(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    // ---- model ----

    const TOML: &str = r#"
title = "Morning Digest"
max_articles = 10
oldest_days = 2
include = ["(?i)rust"]
exclude = ["sponsored"]
use_full_article = true
keep_selectors = ["article"]
remove_selectors = [".ad"]
title_template = "{title} {date:%d %B %Y}"
language = "de"

[[feeds]]
url = "https://a.example/feed.xml"
title = "A"
max_articles = 3

[[feeds]]
url = "https://b.example/atom"

[schedule]
kind = "weekly"
weekday = "Mon"
hour = 6
minute = 30
"#;

    #[test]
    fn toml_recipe_parses_with_defaults() {
        let r = Recipe::from_toml(TOML).unwrap();
        assert_eq!(r.title, "Morning Digest");
        assert_eq!(r.feeds.len(), 2);
        assert_eq!(r.feeds[0].max_articles, Some(3));
        assert_eq!(r.feeds[1].title, None);
        assert_eq!((r.max_articles, r.oldest_days), (10, 2));
        assert!(r.use_full_article && r.include_images);
        assert_eq!(r.max_image_bytes, 1024 * 1024);
        assert_eq!(r.language, "de");
        assert_eq!(
            r.schedule,
            Some(Schedule::Weekly {
                weekday: Weekday::Mon,
                hour: 6,
                minute: 30
            })
        );
        let minimal =
            Recipe::from_toml("title = \"T\"\n[[feeds]]\nurl = \"https://x.org/f\"\n").unwrap();
        assert_eq!((minimal.max_articles, minimal.oldest_days), (25, 7));
        assert_eq!(minimal.title_template, "{title} - {date}");
        assert!(
            minimal.include.is_empty() && !minimal.use_full_article && minimal.schedule.is_none()
        );
    }

    #[test]
    fn recipe_roundtrips_through_toml_and_json() {
        let r = Recipe::from_toml(TOML).unwrap();
        assert_eq!(Recipe::from_toml(&r.to_toml().unwrap()).unwrap(), r);
        assert_eq!(Recipe::from_json(&r.to_json().unwrap()).unwrap(), r);
        let mut c = Recipe::new("Cron", &["https://x.org/f"]);
        c.schedule = Some(Schedule::Cron {
            expr: "*/15 6-8 * jan,feb mon-fri".into(),
        });
        assert_eq!(Recipe::from_toml(&c.to_toml().unwrap()).unwrap(), c);
        c.schedule = Some(Schedule::Hourly { minute: 5 });
        assert_eq!(Recipe::from_json(&c.to_json().unwrap()).unwrap(), c);
        c.schedule = Some(Schedule::Daily { hour: 7, minute: 0 });
        assert_eq!(Recipe::from_toml(&c.to_toml().unwrap()).unwrap(), c);
    }

    #[test]
    fn validation_errors() {
        let ok = Recipe::new("T", &["https://x.org/f"]);
        assert!(ok.validate().is_ok());
        type Mutation = Box<dyn Fn(&mut Recipe)>;
        let cases: Vec<(Mutation, &str)> = vec![
            (Box::new(|r| r.title = "  ".into()), "no title"),
            (Box::new(|r| r.feeds.clear()), "no feeds"),
            (
                Box::new(|r| r.feeds[0].url = "ftp://x.org/f".into()),
                "http or https",
            ),
            (Box::new(|r| r.feeds[0].url = "not a url".into()), "invalid"),
            (Box::new(|r| r.max_articles = 0), "max_articles"),
            (Box::new(|r| r.include = vec!["(".into()]), "invalid regex"),
            (Box::new(|r| r.exclude = vec!["[".into()]), "invalid regex"),
            (
                Box::new(|r| r.keep_selectors = vec!["div[".into()]),
                "invalid CSS selector",
            ),
            (
                Box::new(|r| r.remove_selectors = vec!["[[".into()]),
                "invalid CSS selector",
            ),
            (
                Box::new(|r| {
                    r.schedule = Some(Schedule::Daily {
                        hour: 24,
                        minute: 0,
                    })
                }),
                "invalid schedule",
            ),
            (
                Box::new(|r| r.schedule = Some(Schedule::Cron { expr: "bad".into() })),
                "invalid schedule",
            ),
        ];
        for (mutate, msg) in cases {
            let mut r = ok.clone();
            mutate(&mut r);
            let e = format!("{:#}", r.validate().unwrap_err());
            assert!(e.contains(msg), "expected `{msg}` in `{e}`");
        }
    }

    #[test]
    fn invalid_documents_are_errors() {
        assert!(Recipe::from_toml("title = ").is_err());
        assert!(
            Recipe::from_toml("title = \"x\"")
                .unwrap_err()
                .to_string()
                .contains("invalid recipe TOML")
        );
        assert!(Recipe::from_json("{}").is_err());
        assert!(
            Recipe::from_json("{\"title\":\"t\",\"feeds\":[]}")
                .unwrap_err()
                .to_string()
                .contains("no feeds")
        );
        assert!(Recipe::from_toml("title=\"t\"\n[[feeds]]\nurl=\"https://x.org\"\n[schedule]\nkind=\"hourly\"\nminute=99\n").is_err());
    }

    #[test]
    fn title_templates() {
        let now = t(2024, 3, 5, 14, 7);
        let r = |tpl: &str| render_title_template(tpl, "News", now);
        assert_eq!(r("{title} - {date}"), "News - 2024-03-05");
        assert_eq!(r("{title} {date:%d %B %Y}"), "News 05 March 2024");
        assert_eq!(r("{datetime}"), "2024-03-05 14:07");
        assert_eq!(r("{weekday} {day} {month} {year}"), "Tuesday 05 March 2024");
        assert_eq!(r("{unknown} {title}"), "{unknown} News");
        assert_eq!(
            r("{date:%Q%}"),
            "{date:%Q%}",
            "invalid format is left alone"
        );
        assert_eq!(r("open {brace"), "open {brace");
        assert_eq!(r(""), "");
        assert_eq!(r("日本 {title}"), "日本 News");
        assert_eq!(
            Recipe::new("X", &["https://x.org"]).render_title(now),
            "X - 2024-03-05"
        );
    }

    // ---- cron / schedule ----

    #[test]
    fn cron_field_parsing() {
        let c = CronSpec::parse("*/15 6-8,20 1,15 jan-mar mon,fri").unwrap();
        assert_eq!(c.minutes, (1 << 0) | (1 << 15) | (1 << 30) | (1 << 45));
        assert_eq!(c.hours, (1 << 6) | (1 << 7) | (1 << 8) | (1 << 20));
        assert_eq!(c.dom, (1 << 1) | (1 << 15));
        assert_eq!(c.months, (1 << 1) | (1 << 2) | (1 << 3));
        assert_eq!(c.dow, (1 << 1) | (1 << 5));
        assert!(!c.dom_any && !c.dow_any);
        let any = CronSpec::parse("0 0 * * *").unwrap();
        assert!(any.dom_any && any.dow_any);
        assert_eq!(CronSpec::parse("0 0 * * 7").unwrap().dow, 1, "7 is Sunday");
        assert_eq!(CronSpec::parse("0 0 * * sun").unwrap().dow, 1);
        assert_eq!(
            CronSpec::parse("5/20 * * * *").unwrap().minutes,
            (1 << 5) | (1 << 25) | (1 << 45)
        );
        assert_eq!(
            CronSpec::parse("0-10/5 * * * *").unwrap().minutes,
            (1 << 0) | (1 << 5) | (1 << 10)
        );
    }

    #[test]
    fn cron_errors() {
        for (expr, msg) in [
            ("", "5 fields"),
            ("* * * *", "5 fields"),
            ("* * * * * *", "5 fields"),
            ("60 * * * *", "out of range"),
            ("* 24 * * *", "out of range"),
            ("* * 0 * *", "out of range"),
            ("* * 32 * *", "out of range"),
            ("* * * 13 *", "out of range"),
            ("* * * * 8", "out of range"),
            ("5-1 * * * *", "out of range"),
            ("x * * * *", "bad minute"),
            ("*/0 * * * *", "step 0"),
            ("*/x * * * *", "bad step"),
            ("1,,2 * * * *", "empty item"),
            ("* * * foo *", "bad month"),
        ] {
            let e = CronSpec::parse(expr).unwrap_err().to_string();
            assert!(e.contains(msg), "`{expr}`: {e}");
        }
    }

    #[test]
    fn hourly_daily_weekly_next_slots() {
        let now = t(2024, 3, 5, 10, 20); // Tuesday
        let h = Schedule::Hourly { minute: 30 };
        assert_eq!(h.next_after(now).unwrap(), t(2024, 3, 5, 10, 30));
        assert_eq!(
            h.next_after(t(2024, 3, 5, 10, 30)).unwrap(),
            t(2024, 3, 5, 11, 30),
            "strictly after"
        );
        assert_eq!(
            h.next_after(t(2024, 3, 5, 23, 45)).unwrap(),
            t(2024, 3, 6, 0, 30)
        );
        let d = Schedule::Daily {
            hour: 6,
            minute: 15,
        };
        assert_eq!(d.next_after(now).unwrap(), t(2024, 3, 6, 6, 15));
        assert_eq!(
            d.next_after(t(2024, 3, 5, 5, 0)).unwrap(),
            t(2024, 3, 5, 6, 15)
        );
        assert_eq!(
            d.next_after(t(2024, 12, 31, 7, 0)).unwrap(),
            t(2025, 1, 1, 6, 15),
            "year rollover"
        );
        let w = Schedule::Weekly {
            weekday: Weekday::Mon,
            hour: 8,
            minute: 0,
        };
        assert_eq!(w.next_after(now).unwrap(), t(2024, 3, 11, 8, 0));
        let sun = Schedule::Weekly {
            weekday: Weekday::Sun,
            hour: 0,
            minute: 0,
        };
        assert_eq!(sun.next_after(now).unwrap(), t(2024, 3, 10, 0, 0));
        let same_day = Schedule::Weekly {
            weekday: Weekday::Tue,
            hour: 23,
            minute: 59,
        };
        assert_eq!(same_day.next_after(now).unwrap(), t(2024, 3, 5, 23, 59));
    }

    #[test]
    fn seconds_in_the_reference_time_are_ignored() {
        let with_secs = Utc.with_ymd_and_hms(2024, 3, 5, 10, 29, 59).unwrap();
        let s = Schedule::Hourly { minute: 30 };
        assert_eq!(s.next_after(with_secs).unwrap(), t(2024, 3, 5, 10, 30));
        let after = Utc.with_ymd_and_hms(2024, 3, 5, 10, 30, 1).unwrap();
        assert_eq!(s.next_after(after).unwrap(), t(2024, 3, 5, 11, 30));
    }

    #[test]
    fn cron_lite_slots() {
        let c = Schedule::Cron {
            expr: "*/20 9-10 * * 1-5".into(),
        };
        assert_eq!(
            c.next_after(t(2024, 3, 8, 10, 40)).unwrap(),
            t(2024, 3, 11, 9, 0),
            "Friday evening → Monday"
        );
        assert_eq!(
            c.next_after(t(2024, 3, 5, 9, 1)).unwrap(),
            t(2024, 3, 5, 9, 20)
        );
        let month_end = Schedule::Cron {
            expr: "0 12 31 * *".into(),
        };
        assert_eq!(
            month_end.next_after(t(2024, 4, 1, 0, 0)).unwrap(),
            t(2024, 5, 31, 12, 0),
            "April has no 31st"
        );
        let leap = Schedule::Cron {
            expr: "0 0 29 2 *".into(),
        };
        assert_eq!(
            leap.next_after(t(2024, 3, 1, 0, 0)).unwrap(),
            t(2028, 2, 29, 0, 0)
        );
        let never = Schedule::Cron {
            expr: "0 0 31 2 *".into(),
        };
        assert!(
            never
                .next_after(t(2024, 1, 1, 0, 0))
                .unwrap_err()
                .to_string()
                .contains("never fires")
        );
    }

    #[test]
    fn cron_day_of_month_or_weekday_semantics() {
        // the 13th of the month OR any Friday
        let c = CronSpec::parse("0 0 13 * fri").unwrap();
        assert_eq!(
            c.next_after(t(2024, 3, 1, 0, 0)).unwrap(),
            t(2024, 3, 8, 0, 0),
            "Friday"
        );
        assert_eq!(
            c.next_after(t(2024, 3, 8, 0, 0)).unwrap(),
            t(2024, 3, 13, 0, 0),
            "the 13th is a Wednesday"
        );
        assert_eq!(
            c.next_after(t(2024, 3, 13, 0, 0)).unwrap(),
            t(2024, 3, 15, 0, 0)
        );
        // only day-of-month restricted: weekday is ignored
        let only_dom = CronSpec::parse("0 0 13 * *").unwrap();
        assert_eq!(
            only_dom.next_after(t(2024, 3, 1, 0, 0)).unwrap(),
            t(2024, 3, 13, 0, 0)
        );
    }

    #[test]
    fn next_run_semantics() {
        let daily = Schedule::Daily { hour: 6, minute: 0 };
        let now = t(2024, 3, 5, 10, 0);
        assert_eq!(
            next_run(&daily, None, now).unwrap(),
            now,
            "never ran → due now"
        );
        assert!(is_due(&daily, None, now).unwrap());
        let last = t(2024, 3, 5, 6, 0);
        assert_eq!(
            next_run(&daily, Some(last), now).unwrap(),
            t(2024, 3, 6, 6, 0)
        );
        assert!(!is_due(&daily, Some(last), now).unwrap());
        let long_ago = t(2024, 2, 1, 6, 0);
        assert_eq!(
            next_run(&daily, Some(long_ago), now).unwrap(),
            now,
            "overdue → now, missed runs collapse"
        );
        assert!(is_due(&daily, Some(long_ago), now).unwrap());
        assert!(
            is_due(&daily, Some(t(2024, 3, 4, 6, 0)), t(2024, 3, 5, 6, 0)).unwrap(),
            "due exactly at the slot"
        );
        // a run that happened mid-day does not make the same day's earlier slot due
        assert_eq!(
            next_run(&daily, Some(t(2024, 3, 5, 9, 0)), now).unwrap(),
            t(2024, 3, 6, 6, 0)
        );
    }

    #[test]
    fn next_run_rejects_invalid_schedules() {
        let bad = Schedule::Hourly { minute: 75 };
        assert!(next_run(&bad, None, t(2024, 1, 1, 0, 0)).is_err());
        assert!(next_run(&bad, Some(t(2024, 1, 1, 0, 0)), t(2024, 1, 2, 0, 0)).is_err());
        assert!(
            Schedule::Weekly {
                weekday: Weekday::Sun,
                hour: 1,
                minute: 99
            }
            .to_cron()
            .is_err()
        );
    }

    // ---- build ----

    const NOW_S: (i32, u32, u32, u32, u32) = (2024, 3, 10, 12, 0);

    fn now() -> DateTime<Utc> {
        t(NOW_S.0, NOW_S.1, NOW_S.2, NOW_S.3, NOW_S.4)
    }

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&[7; 200]);
        v
    }

    fn rss_a() -> String {
        r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:dc="http://purl.org/dc/elements/1.1/"><channel>
<title>Alpha Times</title><link>https://alpha.example/</link>
<item><title>Rust 2024 released</title><link>https://alpha.example/rust-2024</link><pubDate>Sat, 09 Mar 2024 08:00:00 GMT</pubDate><dc:creator>Ann</dc:creator>
<description>Summary one</description><content:encoded><![CDATA[<p>The Rust team shipped, with <b>new</b> features.</p><img src="/img/rust.png" alt="logo">]]></content:encoded></item>
<item><title>Sponsored: buy things</title><link>https://alpha.example/ad</link><pubDate>Sat, 09 Mar 2024 09:00:00 GMT</pubDate><description>buy</description></item>
<item><title>Ancient history</title><link>https://alpha.example/old</link><pubDate>Mon, 01 Jan 2024 09:00:00 GMT</pubDate><description>old news</description></item>
<item><title>Weather today</title><link>https://alpha.example/weather</link><pubDate>Sun, 10 Mar 2024 06:00:00 GMT</pubDate><description>Sunny &amp; warm</description></item>
<item><title>Shared story</title><link>https://shared.example/story</link><pubDate>Sun, 10 Mar 2024 07:00:00 GMT</pubDate><description>Same story in two feeds</description></item>
</channel></rss>"#
            .into()
    }

    fn atom_b() -> String {
        r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Beta Daily</title><link href="https://beta.example/"/>
<entry><title>Shared story</title><link href="https://shared.example/story"/><id>b1</id><updated>2024-03-10T07:00:00Z</updated><summary>dup</summary></entry>
<entry><title>Beta only</title><link href="https://beta.example/only"/><id>b2</id><updated>2024-03-09T07:00:00Z</updated><summary type="html">&lt;p&gt;Beta body text&lt;/p&gt;</summary></entry>
</feed>"#
            .into()
    }

    fn page(body: &str) -> String {
        format!(
            "<html><head><title>x</title></head><body><nav>menu junk</nav><article>{body}<aside class=\"ad\">BUY</aside></article></body></html>"
        )
    }

    fn mock() -> MockFetcher {
        MockFetcher::new()
            .with_body("https://alpha.example/feed", rss_a())
            .with_body("https://beta.example/atom", atom_b())
            .with_body("https://alpha.example/img/rust.png", png())
    }

    fn recipe() -> Recipe {
        let mut r = Recipe::new(
            "Daily",
            &["https://alpha.example/feed", "https://beta.example/atom"],
        );
        r.exclude = vec!["(?i)^sponsored".into()];
        r
    }

    #[test]
    fn select_entries_filters_sorts_and_dedupes() {
        let feed = parse_feed(rss_a().as_bytes()).unwrap();
        let mut stats = NewsStats::default();
        let mut seen = HashSet::new();
        let got = select_entries(&recipe(), &feed, 10, now(), &mut seen, &mut stats).unwrap();
        let titles: Vec<&str> = got.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles,
            vec!["Shared story", "Weather today", "Rust 2024 released"],
            "newest first"
        );
        assert_eq!(
            (
                stats.skipped_old,
                stats.skipped_filtered,
                stats.skipped_duplicate
            ),
            (1, 1, 0)
        );
        assert_eq!(seen.len(), 3);
        // a second pass over the same entries sees duplicates
        let again = select_entries(&recipe(), &feed, 10, now(), &mut seen, &mut stats).unwrap();
        assert!(again.is_empty());
        assert_eq!(stats.skipped_duplicate, 3);
    }

    #[test]
    fn select_entries_limit_include_and_undated() {
        let mut feed = parse_feed(rss_a().as_bytes()).unwrap();
        feed.entries.push(Entry {
            title: "Undated".into(),
            link: "https://alpha.example/u".into(),
            ..Default::default()
        });
        feed.entries.push(Entry::default());
        let mut r = recipe();
        r.include = vec!["(?i)rust".into(), "(?i)undated".into()];
        let got = select_entries(
            &r,
            &feed,
            10,
            now(),
            &mut HashSet::new(),
            &mut NewsStats::default(),
        )
        .unwrap();
        assert_eq!(
            got.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Rust 2024 released", "Undated"],
            "dated first, undated kept"
        );
        let one = select_entries(
            &recipe(),
            &feed,
            1,
            now(),
            &mut HashSet::new(),
            &mut NewsStats::default(),
        )
        .unwrap();
        assert_eq!(one.len(), 1);
        r.oldest_days = 0;
        r.include.clear();
        r.exclude.clear();
        let all = select_entries(
            &r,
            &feed,
            100,
            now(),
            &mut HashSet::new(),
            &mut NewsStats::default(),
        )
        .unwrap();
        assert_eq!(
            all.len(),
            6,
            "no age limit keeps the old entry; the empty entry is dropped"
        );
        r.include = vec!["(".into()];
        assert!(
            select_entries(
                &r,
                &feed,
                1,
                now(),
                &mut HashSet::new(),
                &mut NewsStats::default()
            )
            .is_err()
        );
    }

    fn open(n: &NewsEpub) -> OpenBook {
        OpenBook::from_archive(Archive::from_bytes(n.archive.to_bytes().unwrap()).unwrap()).unwrap()
    }

    fn parse_xml(s: &str) -> roxmltree::Document<'_> {
        let o = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        };
        roxmltree::Document::parse_with_options(s, o).unwrap_or_else(|e| panic!("{e}\n{s}"))
    }

    #[test]
    fn builds_a_readable_epub_from_feeds() {
        let f = mock();
        let n = build_news_epub(&recipe(), &f, now()).unwrap();
        assert_eq!(n.title, "Daily - 2024-03-10");
        assert_eq!(n.stats.feeds_ok, 2);
        assert_eq!(
            n.stats.articles, 4,
            "3 from alpha + 1 beta (shared story kept once)"
        );
        assert_eq!(n.stats.skipped_duplicate, 1);
        assert_eq!(n.stats.images, 1);
        assert!(n.errors.is_empty(), "{:?}", n.errors);

        // independent checks on the zip and XML
        let bytes = n.archive.to_bytes().unwrap();
        assert_eq!(
            u16::from_le_bytes([bytes[8], bytes[9]]),
            0,
            "mimetype stored"
        );
        assert_eq!(&bytes[30..38], b"mimetype");
        let arc = Archive::from_bytes(bytes).unwrap();
        for name in arc.names() {
            if name.ends_with(".xhtml") || name.ends_with(".opf") || name.ends_with(".ncx") {
                parse_xml(&arc.get_string(name).unwrap());
            }
        }

        let book = open(&n);
        let md = book.metadata();
        assert_eq!(md.title, "Daily - 2024-03-10");
        assert_eq!(md.authors, vec!["Alpha Times", "Beta Daily"]);
        assert_eq!(md.published, "2024-03-10");
        assert_eq!(md.tags, vec!["News"]);
        assert!(md.description.starts_with("4 articles"));
        assert!(book.cover_bytes().is_some());
        // 2 sections + 4 articles
        assert_eq!(book.spine_paths().len(), 6);
        let toc = book.toc();
        assert_eq!(toc.len(), 2, "one top-level entry per feed");
        assert_eq!(toc[0].title, "Alpha Times");
        assert_eq!(
            toc[0]
                .children
                .iter()
                .map(|c| c.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Shared story", "Weather today", "Rust 2024 released"]
        );
        assert_eq!(toc[1].title, "Beta Daily");
        assert_eq!(toc[1].children.len(), 1);
        assert_eq!(toc[1].children[0].title, "Beta only");
        // every toc target exists in the spine
        let spine = book.spine_paths();
        for e in toc.iter().chain(toc.iter().flat_map(|e| e.children.iter())) {
            let path = e.href.split('#').next().unwrap();
            assert!(spine.iter().any(|s| s == path), "{path} not in spine");
        }
    }

    #[test]
    fn article_chapters_hold_text_images_and_metadata() {
        let n = build_news_epub(&recipe(), &mock(), now()).unwrap();
        let book = open(&n);
        let spine = book.spine_paths();
        let rust = spine
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .find(|s| s.contains("Rust 2024 released") && s.contains("The Rust team shipped"))
            .expect("rust article");
        assert!(rust.contains("<strong>new</strong>"));
        assert!(rust.contains("Ann"));
        assert!(rust.contains("2024-03-09"));
        assert!(rust.contains("https://alpha.example/rust-2024"));
        assert!(rust.contains("src=\"images/news1.png\""), "{rust}");
        assert!(
            book.file_list()
                .iter()
                .any(|f| f.ends_with("images/news1.png"))
        );
        let beta = spine
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .find(|s| s.contains("Beta body text"))
            .unwrap();
        assert!(beta.contains("<p>Beta body text</p>"));
        let weather = spine
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .find(|s| s.contains("Sunny"))
            .unwrap();
        assert!(weather.contains("Sunny &amp; warm"));
        // section page lists the articles with working relative links
        let section = book.chapter_source(&spine[0]).unwrap();
        assert!(
            section.contains("<h1>Alpha Times</h1>")
                && section.contains("href=\"chapter-2.xhtml\"")
        );
    }

    #[test]
    fn full_articles_use_readability_with_selectors() {
        let f = mock()
            .with_body(
                "https://alpha.example/rust-2024",
                page("<p>Full article text about Rust, with commas, quite long indeed, for the extractor.</p><div class=\"noise\">NOISE</div><img src=\"/img/rust.png\" alt=\"\">"),
            )
            .with_body("https://alpha.example/weather", page("<p>Detailed forecast, with commas, and plenty of words in it to be sure.</p>"))
            .on("https://shared.example/story", MockReply::Status(500))
            .with_body("https://beta.example/only", "<html><body><nav>x</nav></body></html>");
        let mut r = recipe();
        r.use_full_article = true;
        r.remove_selectors = vec![".noise".into()];
        let n = build_news_epub(&r, &f, now()).unwrap();
        assert_eq!(n.stats.full_articles, 2);
        assert_eq!(n.errors.len(), 2, "{:?}", n.errors);
        assert!(
            n.errors
                .iter()
                .any(|e| e.contains("shared.example") && e.contains("HTTP error 500"))
        );
        assert!(
            n.errors
                .iter()
                .any(|e| e.contains("beta.example/only") && e.contains("no readable content"))
        );
        let book = open(&n);
        let all: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(all.contains("Full article text about Rust"));
        assert!(!all.contains("NOISE") && !all.contains("BUY") && !all.contains("menu junk"));
        assert!(all.contains("Detailed forecast"));
        assert!(
            all.contains("Same story in two feeds"),
            "failed download falls back to the feed text"
        );
        assert!(
            all.contains("Beta body text"),
            "unextractable page falls back to the feed text"
        );
        assert_eq!(n.stats.articles, 4);
        // the full article fetched the page, not just the feed
        assert!(
            f.urls()
                .contains(&"https://alpha.example/rust-2024".to_string())
        );
    }

    #[test]
    fn a_feed_can_override_the_full_article_setting() {
        let page = "<html><body><p>Full article text, with commas, and enough words for the extractor.</p></body></html>";
        let pages = |f: MockFetcher| {
            f.with_body("https://alpha.example/rust-2024", page)
                .with_body("https://alpha.example/weather", page)
                .with_body("https://beta.example/only", page)
                .with_body("https://shared.example/story", page)
        };
        // recipe says summaries, the first feed asks for full articles
        let mut r = recipe();
        r.feeds[0].full_article = Some(true);
        let f = pages(mock());
        let n = build_news_epub(&r, &f, now()).unwrap();
        assert!(n.stats.full_articles >= 1);
        assert!(
            f.urls()
                .contains(&"https://alpha.example/rust-2024".to_string())
        );
        assert!(
            !f.urls().contains(&"https://beta.example/only".to_string()),
            "the second feed follows the recipe"
        );
        // recipe says full articles, the second feed opts out
        let mut r = recipe();
        r.use_full_article = true;
        r.feeds[1].full_article = Some(false);
        let f = pages(mock());
        build_news_epub(&r, &f, now()).unwrap();
        assert!(
            f.urls()
                .contains(&"https://alpha.example/rust-2024".to_string())
        );
        assert!(!f.urls().contains(&"https://beta.example/only".to_string()));
        // nothing written when there is no override, and it round-trips
        let t = recipe().to_toml().unwrap();
        assert!(!t.lines().any(|l| l.starts_with("full_article")));
        let mut r = recipe();
        r.feeds[0].full_article = Some(false);
        let back = Recipe::from_toml(&r.to_toml().unwrap()).unwrap();
        assert_eq!(back.feeds[0].full_article, Some(false));
        assert_eq!(back.feeds[1].full_article, None);
    }

    #[test]
    fn keep_selectors_restrict_full_articles() {
        let f = mock().with_body(
            "https://alpha.example/rust-2024",
            "<html><body><div id=\"a\"><p>Kept part, with commas, and enough words.</p></div><div id=\"b\"><p>Dropped part, with commas, and enough words.</p></div></body></html>",
        );
        let mut r = recipe();
        r.use_full_article = true;
        r.keep_selectors = vec!["#a".into()];
        let n = build_news_epub(&r, &f, now()).unwrap();
        let book = open(&n);
        let all: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(all.contains("Kept part") && !all.contains("Dropped part"));
    }

    #[test]
    fn failing_feed_is_reported_and_others_still_build() {
        let f = MockFetcher::new()
            .with_body("https://alpha.example/feed", rss_a())
            .on("https://beta.example/atom", MockReply::Timeout);
        let n = build_news_epub(&recipe(), &f, now()).unwrap();
        assert_eq!((n.stats.feeds_ok, n.stats.feeds_failed), (1, 1));
        assert!(n.errors[0].contains("beta.example") && n.errors[0].contains("timed out"));
        assert_eq!(open(&n).toc().len(), 1);
    }

    #[test]
    fn invalid_feed_body_counts_as_failure() {
        let f = MockFetcher::new()
            .with_body("https://alpha.example/feed", "<html>not a feed</html>")
            .with_body("https://beta.example/atom", atom_b());
        let n = build_news_epub(&recipe(), &f, now()).unwrap();
        assert_eq!(n.stats.feeds_failed, 1);
        assert!(n.errors[0].contains("not a feed"));
    }

    #[test]
    fn nothing_collected_is_an_error() {
        let f = MockFetcher::new().on_prefix("https://", MockReply::Status(503));
        let e = build_news_epub(&recipe(), &f, now())
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("no articles collected") && e.contains("HTTP error 503"),
            "{e}"
        );
        // everything filtered out
        let mut r = recipe();
        r.include = vec!["zzzz-no-match".into()];
        let e = build_news_epub(&r, &mock(), now()).unwrap_err().to_string();
        assert!(e.contains("no articles collected"), "{e}");
        // invalid recipe
        let mut bad = recipe();
        bad.feeds.clear();
        assert!(
            build_news_epub(&bad, &mock(), now())
                .unwrap_err()
                .to_string()
                .contains("no feeds")
        );
    }

    #[test]
    fn images_can_be_disabled_limited_and_size_capped() {
        let mut r = recipe();
        r.include_images = false;
        let n = build_news_epub(&r, &mock(), now()).unwrap();
        assert_eq!(n.stats.images, 0);
        let book = open(&n);
        assert!(!book.file_list().iter().any(|f| f.contains("images/news")));
        let all: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(
            !all.contains("<img"),
            "unfetched images are removed from the text"
        );

        let mut small = recipe();
        small.max_image_bytes = 50;
        let n = build_news_epub(&small, &mock(), now()).unwrap();
        assert_eq!(n.stats.images, 0, "image above the size cap is skipped");

        let mut zero = recipe();
        zero.max_images = 0;
        assert_eq!(
            build_news_epub(&zero, &mock(), now()).unwrap().stats.images,
            0
        );
    }

    #[test]
    fn per_feed_limit_and_section_title_override() {
        let mut r = recipe();
        r.feeds[0].max_articles = Some(1);
        r.feeds[0].title = Some("My Alpha".into());
        let n = build_news_epub(&r, &mock(), now()).unwrap();
        assert_eq!(n.stats.articles, 2);
        let toc = open(&n).toc();
        assert_eq!(toc[0].title, "My Alpha");
        assert_eq!(toc[0].children.len(), 1);
    }

    #[test]
    fn enclosure_image_is_used_when_the_text_has_none() {
        let rss = r#"<rss version="2.0"><channel><title>E</title><item><title>With pic</title><link>https://e.example/p</link><description>Just words here</description><enclosure url="/pic.png" type="image/png" length="1"/></item></channel></rss>"#;
        let f = MockFetcher::new()
            .with_body("https://e.example/feed", rss)
            .with_body("https://e.example/pic.png", png());
        let mut r = Recipe::new("E", &["https://e.example/feed"]);
        r.oldest_days = 0;
        let n = build_news_epub(&r, &f, now()).unwrap();
        assert_eq!(n.stats.images, 1);
        let book = open(&n);
        let all: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(all.contains("images/news1.png") && all.contains("Just words here"));
    }

    #[test]
    fn hostile_feed_text_is_escaped() {
        let rss = "<rss version=\"2.0\"><channel><title>&lt;script&gt;x&lt;/script&gt;</title><item><title>&lt;b&gt;Bold &amp; \"quoted\"&lt;/b&gt;</title><link>https://h.example/a?x=1&amp;y=\"2\"</link><description>&lt;script&gt;alert(1)&lt;/script&gt;&lt;p&gt;ok text&lt;/p&gt;</description></item></channel></rss>";
        let f = MockFetcher::new().with_body("https://h.example/f", rss);
        let mut r = Recipe::new("H <&> T", &["https://h.example/f"]);
        r.oldest_days = 0;
        let n = build_news_epub(&r, &f, now()).unwrap();
        let arc = Archive::from_bytes(n.archive.to_bytes().unwrap()).unwrap();
        for name in arc.names() {
            if name.ends_with(".xhtml") || name.ends_with(".opf") || name.ends_with(".ncx") {
                parse_xml(&arc.get_string(name).unwrap());
            }
        }
        let book = open(&n);
        let all: String = book
            .spine_paths()
            .iter()
            .filter_map(|p| book.chapter_source(p))
            .collect();
        assert!(!all.contains("<script") && !all.contains("alert(1)"));
        assert!(all.contains("ok text"));
        assert_eq!(book.metadata().title, "H <&> T - 2024-03-10");
    }

    #[test]
    fn undated_entries_and_missing_titles_build() {
        let rss = "<rss version=\"2.0\"><channel><item><link>https://u.example/1</link><description>Body of untitled</description></item><item><title>Titled</title></item></channel></rss>";
        let f = MockFetcher::new().with_body("https://u.example/f", rss);
        let n = build_news_epub(&Recipe::new("U", &["https://u.example/f"]), &f, now()).unwrap();
        let toc = open(&n).toc();
        assert_eq!(
            toc[0].title, "u.example",
            "section named after the host when the feed has no title"
        );
        assert_eq!(toc[0].children.len(), 2);
        assert_eq!(toc[0].children[0].title, "https://u.example/1");
        let all: String = open(&n)
            .spine_paths()
            .iter()
            .filter_map(|p| open(&n).chapter_source(p))
            .collect();
        assert!(
            all.contains("(no text)"),
            "entry with neither text nor link"
        );
    }

    #[test]
    fn build_is_deterministic_apart_from_ids() {
        let a = build_news_epub(&recipe(), &mock(), now()).unwrap();
        let b = build_news_epub(&recipe(), &mock(), now()).unwrap();
        let chapter = |n: &NewsEpub| n.archive.get_string("OEBPS/chapter-3.xhtml");
        assert_eq!(chapter(&a), chapter(&b));
        assert_eq!(
            a.archive.get("OEBPS/images/cover.png"),
            b.archive.get("OEBPS/images/cover.png")
        );
    }
}
