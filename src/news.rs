//! News subscriptions (roadmap S4/S5): a list of recipes with a schedule, kept in a JSON file next
//! to the config. Each run builds a news book with `bookx-sources`, tags it so the issues of one
//! subscription can be found again, and old issues beyond the keep limit are pruned. The network
//! is behind the `Fetcher` trait; nothing here needs GTK.

use crate::i18n::ntr;
use crate::i18n::tr;
use crate::i18n::trf;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use bookx_sources::feed::parse_feed;
use bookx_sources::fetch::Fetcher;
use bookx_sources::recipe::{build_news_epub, is_due, FeedSpec, Recipe, Schedule};
use chrono::{DateTime, FixedOffset, Utc, Weekday};
use serde::{Deserialize, Serialize};

use crate::core::book::OpenBook;
use crate::core::library::{prepare_from_archive, Book, Prepared};
use crate::core::opf::Identifier;

/// Scheme of the identifier that ties an issue to its subscription.
pub const ID_SCHEME: &str = "bookx-news";
pub const DEFAULT_KEEP: usize = 5;

fn d_keep() -> usize {
    DEFAULT_KEEP
}
fn d_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Subscription {
    /// Stable identity (slug of the first title); renaming does not change it.
    pub key: String,
    pub recipe: Recipe,
    #[serde(default = "d_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_status: String,
    #[serde(default = "d_keep")]
    pub keep_issues: usize,
    /// How many times it was run (successfully or not).
    #[serde(default)]
    pub attempts: u32,
    /// Feeds of the recipe that are switched off: kept in the list, skipped when downloading.
    #[serde(default)]
    pub disabled_feeds: Vec<String>,
}

impl Subscription {
    pub fn new(recipe: Recipe) -> Subscription {
        Subscription {
            key: slug(&recipe.title),
            recipe,
            enabled: true,
            last_run: None,
            last_status: String::new(),
            keep_issues: DEFAULT_KEEP,
            attempts: 0,
            disabled_feeds: Vec::new(),
        }
    }

    /// The recipe as it runs: without the switched-off feeds.
    pub fn active_recipe(&self) -> Recipe {
        let mut r = self.recipe.clone();
        r.feeds.retain(|f| !self.disabled_feeds.contains(&f.url));
        r
    }

    /// The feed addresses for the edit form, one per line; a switched-off feed starts with `# `.
    pub fn feeds_text(&self) -> String {
        let lines: Vec<String> = self
            .recipe
            .feeds
            .iter()
            .map(|f| {
                if self.disabled_feeds.contains(&f.url) {
                    format!("# {}", f.url)
                } else {
                    f.url.clone()
                }
            })
            .collect();
        lines.join("\n")
    }

    /// Whether the scheduler should run it at `now`. Manual (no schedule) and disabled never.
    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        match &self.recipe.schedule {
            None => false,
            Some(s) => is_due(s, self.last_run, now).unwrap_or(false),
        }
    }

    /// Records the end of a run. Failures count too, so a broken feed waits for its next slot
    /// instead of being retried every minute.
    pub fn finish(&mut self, now: DateTime<Utc>, status: impl Into<String>) {
        self.last_run = Some(now);
        self.last_status = status.into();
        self.attempts += 1;
    }
}

/// `"The Daily Garden!"` becomes `"the-daily-garden"`; never empty.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "news".into()
    } else {
        out
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Doc {
    #[serde(default)]
    subscriptions: Vec<Subscription>,
}

#[derive(Debug, Default)]
pub struct NewsStore {
    pub subs: Vec<Subscription>,
    path: Option<PathBuf>,
}

impl NewsStore {
    #[cfg(test)]
    pub fn in_memory() -> NewsStore {
        NewsStore::default()
    }

    /// Loads the file. A missing file is an empty store; a corrupt one is moved to `.bak` and
    /// replaced by an empty store, so the app always starts.
    pub fn load(path: Option<PathBuf>) -> NewsStore {
        let Some(p) = path else {
            return NewsStore::default();
        };
        let subs = match std::fs::read_to_string(&p) {
            Ok(text) => match serde_json::from_str::<Doc>(&text) {
                Ok(d) => d.subscriptions,
                Err(e) => {
                    tracing::warn!("news subscriptions file is corrupt: {e}");
                    let _ = std::fs::rename(&p, p.with_extension("json.bak"));
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        NewsStore {
            subs,
            path: Some(p),
        }
    }

    /// Writes through a temporary file and a rename, so a crash never leaves half a file.
    pub fn save(&self) -> Result<()> {
        let Some(p) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&Doc {
            subscriptions: self.subs.clone(),
        })?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn get(&self, key: &str) -> Option<&Subscription> {
        self.subs.iter().find(|s| s.key == key)
    }

    /// Adds a subscription, making its key unique (`name`, `name-2`, ...). Returns the key.
    pub fn add(&mut self, mut sub: Subscription) -> Result<String> {
        sub.recipe.validate()?;
        let base = if sub.key.is_empty() {
            slug(&sub.recipe.title)
        } else {
            sub.key.clone()
        };
        let mut key = base.clone();
        let mut n = 2;
        while self.get(&key).is_some() {
            key = format!("{base}-{n}");
            n += 1;
        }
        sub.key = key.clone();
        self.subs.push(sub);
        self.save()?;
        Ok(key)
    }

    /// Replaces the subscription with the same key.
    pub fn update(&mut self, sub: Subscription) -> Result<()> {
        sub.recipe.validate()?;
        match self.subs.iter_mut().find(|s| s.key == sub.key) {
            Some(s) => *s = sub,
            None => bail!("no subscription `{}`", sub.key),
        }
        self.save()
    }

    pub fn remove(&mut self, key: &str) -> Result<bool> {
        let before = self.subs.len();
        self.subs.retain(|s| s.key != key);
        let removed = self.subs.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Indexes of the subscriptions to run at `now`.
    pub fn due(&self, now: DateTime<Utc>) -> Vec<usize> {
        self.subs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_due(now))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn due_keys(&self, now: DateTime<Utc>) -> Vec<String> {
        self.due(now)
            .into_iter()
            .map(|i| self.subs[i].key.clone())
            .collect()
    }

    /// Records the outcome of a run and saves.
    pub fn record(&mut self, key: &str, now: DateTime<Utc>, status: &str) -> Result<()> {
        if let Some(s) = self.subs.iter_mut().find(|s| s.key == key) {
            s.finish(now, status);
            self.save()?;
        }
        Ok(())
    }
}

/// Builds the next issue of a subscription: the book (tagged `News`, with the `bookx-news`
/// identifier) and a line for the user.
pub fn run_subscription(
    sub: &Subscription,
    fetcher: &dyn Fetcher,
    now: DateTime<Utc>,
) -> Result<(Prepared, String)> {
    let recipe = sub.active_recipe();
    if recipe.feeds.is_empty() {
        bail!("every feed of this subscription is switched off");
    }
    let news = build_news_epub(&recipe, fetcher, now)?;
    let mut book = OpenBook::from_archive(news.archive)?;
    let mut meta = book.metadata().clone();
    if !meta.tags.iter().any(|t| t.eq_ignore_ascii_case("News")) {
        meta.tags.push("News".into());
    }
    meta.identifiers.retain(|i| i.scheme != ID_SCHEME);
    meta.identifiers.push(Identifier {
        scheme: ID_SCHEME.into(),
        value: sub.key.clone(),
        xml_id: None,
    });
    book.set_metadata(meta)?;
    let prepared = prepare_from_archive(book.archive, Path::new(&news.title))?;
    let mut summary = trf(
        "{}: {}",
        &[
            &sub.recipe.title,
            &ntr("{} article", "{} articles", news.stats.articles),
        ],
    );
    if !news.errors.is_empty() {
        summary.push_str(&format!(", {} problem(s) skipped", news.errors.len()));
    }
    Ok((prepared, summary))
}

/// The ids of the issues of `key` to delete so that `keep` (at least 1) newest remain.
pub fn prune_plan(books: &[Book], key: &str, keep: usize) -> Vec<u64> {
    let mut issues: Vec<&Book> = books
        .iter()
        .filter(|b| b.meta.identifier(ID_SCHEME) == Some(key))
        .collect();
    issues.sort_by(|a, b| b.added.cmp(&a.added).then(b.id.cmp(&a.id)));
    issues.into_iter().skip(keep.max(1)).map(|b| b.id).collect()
}

/// The title of the feed at `url`, to prefill the name of a new subscription.
pub fn feed_title(url: &str, fetcher: &dyn Fetcher) -> Result<String> {
    let url = crate::web_import::normalize_url(url)?;
    let body = fetcher.get(&url, &[])?;
    let feed = parse_feed(&body)?;
    let t = feed.title.trim();
    if t.is_empty() {
        bail!("the feed has no title");
    }
    Ok(t.to_string())
}

/// Splits the feed box of the form: the addresses with `#` marks removed (one per line, for
/// [`recipe_from_form`]) and the normalised addresses of the lines that start with `#` (switched off).
pub fn feed_lines(text: &str) -> Result<(String, Vec<String>)> {
    let mut all = Vec::new();
    let mut off = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match line.strip_prefix('#') {
            Some(rest) => {
                let rest = rest.trim();
                if rest.is_empty() {
                    continue;
                }
                off.push(crate::web_import::normalize_url(rest)?);
                all.push(rest.to_string());
            }
            None => all.push(line.to_string()),
        }
    }
    Ok((all.join("\n"), off))
}

/// The feeds of an OPML file as `(title, address)`; outlines without `xmlUrl` (folders) are
/// searched for feeds inside them, duplicates dropped.
pub fn parse_opml(text: &str) -> Result<Vec<(String, String)>> {
    let doc = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .context("this is not an OPML file")?;
    let mut out: Vec<(String, String)> = Vec::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("outline")) {
        let attr = |name: &str| {
            n.attributes()
                .find(|a| a.name().eq_ignore_ascii_case(name))
                .map(|a| a.value().trim().to_string())
        };
        let Some(url) = attr("xmlUrl").filter(|u| !u.is_empty()) else {
            continue;
        };
        if out.iter().any(|(_, u)| *u == url) {
            continue;
        }
        let title = attr("title")
            .or_else(|| attr("text"))
            .unwrap_or_else(|| url.clone());
        out.push((title, url));
    }
    if out.is_empty() {
        bail!("the OPML file lists no feeds");
    }
    Ok(out)
}

/// Builds a recipe from what the edit dialog collects. `feeds` is one URL per line (blank lines
/// ignored); every URL is normalised. `base` is the recipe being edited, if any.
pub fn recipe_from_form(
    title: &str,
    feeds: &str,
    max_articles: usize,
    full_articles: bool,
    schedule: Option<Schedule>,
    base: Option<&Recipe>,
) -> Result<Recipe> {
    let title = title.trim();
    if title.is_empty() {
        bail!("Give the subscription a name.");
    }
    let mut specs = Vec::new();
    for line in feeds.lines().map(str::trim).filter(|l| !l.is_empty()) {
        specs.push(FeedSpec {
            url: crate::web_import::normalize_url(line)?,
            title: None,
            max_articles: None,
        });
    }
    if specs.is_empty() {
        bail!("Add at least one feed address.");
    }
    // an edited subscription keeps what the form does not show (filters, image limits, ...)
    let mut r = base.cloned().unwrap_or_else(|| Recipe::new(title, &[]));
    r.title = title.to_string();
    r.feeds = specs;
    r.max_articles = max_articles.max(1);
    r.use_full_article = full_articles;
    r.schedule = schedule;
    r.validate()?;
    Ok(r)
}

// ---- schedule choice for the UI ---------------------------------------------------------------

/// What the user can pick; times are local.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleChoice {
    Manual,
    Hourly,
    Daily {
        hour: u32,
        minute: u32,
    },
    Weekly {
        weekday: Weekday,
        hour: u32,
        minute: u32,
    },
    /// A cron expression set by hand in a file; kept as is.
    Custom(String),
}

const DAY_MINUTES: i64 = 24 * 60;

/// Moves a time of day by `delta` minutes, carrying the weekday over midnight.
fn shift(weekday: Weekday, hour: u32, minute: u32, delta: i64) -> (Weekday, u32, u32) {
    let total = hour as i64 * 60 + minute as i64 + delta;
    let days = total.div_euclid(DAY_MINUTES);
    let rest = total.rem_euclid(DAY_MINUTES);
    let wd = (weekday.num_days_from_monday() as i64 + days).rem_euclid(7) as u32;
    let weekday = Weekday::try_from(wd as u8).unwrap_or(weekday);
    (weekday, (rest / 60) as u32, (rest % 60) as u32)
}

impl ScheduleChoice {
    /// The stored schedule (UTC) for a choice made in local time at `offset`.
    pub fn to_schedule(&self, offset: FixedOffset) -> Option<Schedule> {
        let delta = -(offset.local_minus_utc() as i64) / 60;
        match self {
            ScheduleChoice::Manual => None,
            ScheduleChoice::Hourly => Some(Schedule::Hourly { minute: 0 }),
            ScheduleChoice::Daily { hour, minute } => {
                let (_, hour, minute) = shift(Weekday::Mon, *hour, *minute, delta);
                Some(Schedule::Daily { hour, minute })
            }
            ScheduleChoice::Weekly {
                weekday,
                hour,
                minute,
            } => {
                let (weekday, hour, minute) = shift(*weekday, *hour, *minute, delta);
                Some(Schedule::Weekly {
                    weekday,
                    hour,
                    minute,
                })
            }
            ScheduleChoice::Custom(expr) => Some(Schedule::Cron { expr: expr.clone() }),
        }
    }

    /// The choice shown for a stored schedule, in local time at `offset`.
    pub fn from_schedule(s: Option<&Schedule>, offset: FixedOffset) -> ScheduleChoice {
        let delta = offset.local_minus_utc() as i64 / 60;
        match s {
            None => ScheduleChoice::Manual,
            Some(Schedule::Hourly { .. }) => ScheduleChoice::Hourly,
            Some(Schedule::Daily { hour, minute }) => {
                let (_, hour, minute) = shift(Weekday::Mon, *hour, *minute, delta);
                ScheduleChoice::Daily { hour, minute }
            }
            Some(Schedule::Weekly {
                weekday,
                hour,
                minute,
            }) => {
                let (weekday, hour, minute) = shift(*weekday, *hour, *minute, delta);
                ScheduleChoice::Weekly {
                    weekday,
                    hour,
                    minute,
                }
            }
            Some(Schedule::Cron { expr }) => ScheduleChoice::Custom(expr.clone()),
        }
    }

    /// `Every day at 07:30`.
    pub fn text(&self) -> String {
        match self {
            ScheduleChoice::Manual => tr("Manual only"),
            ScheduleChoice::Hourly => tr("Every hour"),
            ScheduleChoice::Daily { hour, minute } => {
                format!("Every day at {hour:02}:{minute:02}")
            }
            ScheduleChoice::Weekly {
                weekday,
                hour,
                minute,
            } => format!("Every {} at {hour:02}:{minute:02}", weekday_name(*weekday)),
            ScheduleChoice::Custom(e) => format!("Cron {e}"),
        }
    }

    /// Parses `manual`, `hourly`, `daily 07:30`, `weekly mon 07:30` (used by the dev hooks).
    pub fn parse(s: &str) -> Result<ScheduleChoice> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        let time = |t: &str| -> Result<(u32, u32)> {
            let (h, m) = t.split_once(':').context("time must look like 07:30")?;
            let (h, m): (u32, u32) = (h.parse()?, m.parse()?);
            if h > 23 || m > 59 {
                bail!("time {t} is out of range");
            }
            Ok((h, m))
        };
        match parts.as_slice() {
            [] | ["manual"] => Ok(ScheduleChoice::Manual),
            ["hourly"] => Ok(ScheduleChoice::Hourly),
            ["daily", t] => {
                let (hour, minute) = time(t)?;
                Ok(ScheduleChoice::Daily { hour, minute })
            }
            ["weekly", d, t] => {
                let weekday: Weekday = d
                    .parse()
                    .map_err(|_| anyhow::anyhow!("`{d}` is not a weekday"))?;
                let (hour, minute) = time(t)?;
                Ok(ScheduleChoice::Weekly {
                    weekday,
                    hour,
                    minute,
                })
            }
            _ => bail!("unknown schedule `{s}`"),
        }
    }
}

pub fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// One line for the list: the schedule in words.
pub fn schedule_text(sub: &Subscription, offset: FixedOffset) -> String {
    let t = ScheduleChoice::from_schedule(sub.recipe.schedule.as_ref(), offset).text();
    if sub.enabled || sub.recipe.schedule.is_none() {
        t
    } else {
        format!("{t} (paused)")
    }
}

/// "never run", "2026-11-12 09:00: ..." for the list.
pub fn status_text(sub: &Subscription, offset: FixedOffset) -> String {
    match sub.last_run {
        None => tr("Not downloaded yet"),
        Some(t) => format!(
            "{} · {}",
            t.with_timezone(&offset).format("%Y-%m-%d %H:%M"),
            sub.last_status
        ),
    }
}

/// Weekday used by the day pickers, Monday first.
pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

#[cfg(test)]
fn _weekday_index(w: Weekday) -> usize {
    w.num_days_from_monday() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::opf::Metadata;
    use bookx_sources::fetch::{MockFetcher, MockReply};
    use chrono::Datelike;
    use chrono::TimeZone;

    fn t(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-news-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("news.json")
    }

    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Garden News</title>
        <link>https://news.example/</link>
        <item><title>Spring planting</title><link>https://news.example/spring</link>
          <description>&lt;p&gt;Plant the early peas now while the soil is still cool and moist enough.&lt;/p&gt;</description></item>
        <item><title>Compost tips</title><link>https://news.example/compost</link>
          <description>&lt;p&gt;Turn the heap weekly and keep it damp but never soggy at all.&lt;/p&gt;</description></item></channel></rss>"#;

    fn sub(title: &str, sched: Option<Schedule>) -> Subscription {
        let mut r = Recipe::new(title, &["https://news.example/feed.xml"]);
        r.schedule = sched;
        Subscription::new(r)
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }
    fn off(h: i32, m: i32) -> FixedOffset {
        FixedOffset::east_opt(h * 3600 + m * 60).unwrap()
    }

    // ---- slug ----

    #[test]
    fn slugs_are_lowercase_dashed_and_never_empty() {
        assert_eq!(slug("The Daily Garden!"), "the-daily-garden");
        assert_eq!(slug("  A -- B  "), "a-b");
        assert_eq!(slug("Çafé Ünï"), "çafé-ünï");
        assert_eq!(slug("!!!"), "news");
        assert_eq!(slug(""), "news");
    }

    // ---- due ----

    #[test]
    fn manual_and_disabled_subscriptions_are_never_due() {
        let manual = sub("M", None);
        assert!(!manual.is_due(t(12, 9, 0)));
        let mut off = sub("D", Some(Schedule::Hourly { minute: 0 }));
        off.enabled = false;
        assert!(!off.is_due(t(12, 9, 0)));
    }

    #[test]
    fn a_scheduled_subscription_that_never_ran_is_due() {
        let s = sub("H", Some(Schedule::Daily { hour: 7, minute: 0 }));
        assert!(s.is_due(t(12, 3, 0)));
    }

    #[test]
    fn after_a_run_it_waits_for_the_next_slot() {
        let mut s = sub("H", Some(Schedule::Daily { hour: 7, minute: 0 }));
        s.finish(t(12, 7, 5), "ok");
        assert!(!s.is_due(t(12, 20, 0)));
        assert!(!s.is_due(t(13, 6, 59)));
        assert!(s.is_due(t(13, 7, 0)));
        // missed runs are not repeated: one run is due after a long pause
        assert!(s.is_due(t(20, 12, 0)));
    }

    #[test]
    fn a_failed_run_also_waits_for_the_next_slot() {
        let mut s = sub("F", Some(Schedule::Hourly { minute: 0 }));
        s.finish(t(12, 9, 10), "failed: 404");
        assert_eq!(s.attempts, 1);
        assert!(!s.is_due(t(12, 9, 11)));
        assert!(!s.is_due(t(12, 9, 59)));
        assert!(s.is_due(t(12, 10, 0)));
    }

    #[test]
    fn an_invalid_schedule_is_not_due_instead_of_panicking() {
        let s = sub(
            "Bad",
            Some(Schedule::Cron {
                expr: "nonsense".into(),
            }),
        );
        assert!(!s.is_due(t(12, 9, 0)));
    }

    #[test]
    fn due_lists_the_indexes_of_due_subscriptions() {
        let mut st = NewsStore::in_memory();
        st.add(sub("A", None)).unwrap();
        st.add(sub("B", Some(Schedule::Hourly { minute: 0 })))
            .unwrap();
        let mut c = sub("C", Some(Schedule::Hourly { minute: 0 }));
        c.enabled = false;
        st.add(c).unwrap();
        assert_eq!(st.due(t(12, 9, 0)), vec![1]);
        assert_eq!(st.due_keys(t(12, 9, 0)), vec!["b".to_string()]);
        st.record("b", t(12, 9, 0), "ok").unwrap();
        assert!(st.due(t(12, 9, 30)).is_empty());
    }

    // ---- store ----

    #[test]
    fn keys_are_made_unique() {
        let mut st = NewsStore::in_memory();
        assert_eq!(st.add(sub("Same", None)).unwrap(), "same");
        assert_eq!(st.add(sub("Same", None)).unwrap(), "same-2");
        assert_eq!(st.add(sub("Same", None)).unwrap(), "same-3");
        assert_eq!(st.subs.len(), 3);
    }

    #[test]
    fn invalid_recipes_are_refused() {
        let mut st = NewsStore::in_memory();
        let mut s = sub("X", None);
        s.recipe.feeds.clear();
        assert!(st.add(s).is_err());
        assert!(st.subs.is_empty());
        let mut s = sub("X", None);
        s.recipe.feeds[0].url = "ftp://x".into();
        assert!(st.add(s).is_err());
    }

    #[test]
    fn update_keeps_the_key_and_unknown_keys_fail() {
        let mut st = NewsStore::in_memory();
        let k = st.add(sub("Old", None)).unwrap();
        let mut s = st.get(&k).unwrap().clone();
        s.recipe.title = "Renamed".into();
        s.keep_issues = 2;
        st.update(s).unwrap();
        assert_eq!(st.get("old").unwrap().recipe.title, "Renamed");
        assert_eq!(st.get("old").unwrap().keep_issues, 2);
        let mut ghost = sub("Ghost", None);
        ghost.key = "ghost".into();
        assert!(st.update(ghost).is_err());
    }

    #[test]
    fn remove_reports_whether_something_was_removed() {
        let mut st = NewsStore::in_memory();
        let k = st.add(sub("A", None)).unwrap();
        assert!(st.remove(&k).unwrap());
        assert!(!st.remove(&k).unwrap());
        assert!(st.subs.is_empty());
    }

    #[test]
    fn the_store_survives_a_restart() {
        let p = tmp("roundtrip");
        let mut st = NewsStore::load(Some(p.clone()));
        let mut s = sub("Garden", Some(Schedule::Daily { hour: 6, minute: 5 }));
        s.keep_issues = 3;
        s.recipe.use_full_article = true;
        let k = st.add(s).unwrap();
        st.record(&k, t(12, 6, 5), "3 articles").unwrap();
        let back = NewsStore::load(Some(p.clone()));
        assert_eq!(back.subs, st.subs);
        assert_eq!(back.get("garden").unwrap().last_status, "3 articles");
        assert_eq!(back.get("garden").unwrap().last_run, Some(t(12, 6, 5)));
        assert!(!p.with_extension("json.tmp").exists(), "no temp file left");
    }

    #[test]
    fn a_missing_file_is_an_empty_store() {
        let p = tmp("missing");
        assert!(NewsStore::load(Some(p)).subs.is_empty());
        assert!(NewsStore::load(None).subs.is_empty());
        assert!(NewsStore::in_memory().save().is_ok());
    }

    #[test]
    fn a_corrupt_file_is_replaced_by_an_empty_store_and_kept_as_bak() {
        let p = tmp("corrupt");
        std::fs::write(&p, "{ this is not json").unwrap();
        let mut st = NewsStore::load(Some(p.clone()));
        assert!(st.subs.is_empty());
        let bak = p.with_extension("json.bak");
        assert_eq!(std::fs::read_to_string(bak).unwrap(), "{ this is not json");
        // and it is usable again
        st.add(sub("New", None)).unwrap();
        assert_eq!(NewsStore::load(Some(p)).subs.len(), 1);
    }

    #[test]
    fn old_files_with_missing_fields_still_load() {
        let p = tmp("sparse");
        let recipe = serde_json::to_value(Recipe::new("S", &["https://a.example/f"])).unwrap();
        std::fs::write(
            &p,
            serde_json::json!({"subscriptions":[{"key":"s","recipe":recipe}]}).to_string(),
        )
        .unwrap();
        let st = NewsStore::load(Some(p));
        let s = st.get("s").unwrap();
        assert!(s.enabled);
        assert_eq!(s.keep_issues, DEFAULT_KEEP);
        assert_eq!(s.last_run, None);
    }

    #[test]
    fn save_creates_missing_directories() {
        let p = tmp("dirs");
        let nested = p.parent().unwrap().join("a/b/news.json");
        let mut st = NewsStore::load(Some(nested.clone()));
        st.add(sub("N", None)).unwrap();
        assert!(nested.exists());
        assert_eq!(st.path(), Some(nested.as_path()));
    }

    // ---- running ----

    #[test]
    fn a_run_builds_a_tagged_book() {
        let f = MockFetcher::new().with_body("https://news.example/feed.xml", FEED);
        let s = sub("Garden Daily", None);
        let (p, summary) = run_subscription(&s, &f, t(12, 9, 0)).unwrap();
        assert!(p.meta.tags.iter().any(|x| x == "News"), "{:?}", p.meta.tags);
        assert_eq!(p.meta.identifier(ID_SCHEME), Some("garden-daily"));
        assert!(p.meta.title.contains("Garden Daily"), "{}", p.meta.title);
        assert!(summary.contains("Garden Daily: 2 articles"), "{summary}");
        // the tag and identifier are in the file itself, not only in the Prepared struct
        let book =
            OpenBook::from_archive(crate::core::archive::Archive::from_bytes(p.bytes).unwrap())
                .unwrap();
        assert_eq!(book.metadata().identifier(ID_SCHEME), Some("garden-daily"));
        assert_eq!(
            book.metadata()
                .tags
                .iter()
                .filter(|x| x.as_str() == "News")
                .count(),
            1
        );
    }

    #[test]
    fn a_run_fails_when_no_article_can_be_downloaded() {
        let f = MockFetcher::new().on("https://news.example/feed.xml", MockReply::Status(500));
        let s = sub("Down", None);
        assert!(run_subscription(&s, &f, t(12, 9, 0)).is_err());
    }

    #[test]
    fn feed_titles_prefill_and_failures_are_errors() {
        let f = MockFetcher::new()
            .with_body("https://news.example/feed.xml", FEED)
            .with_body("https://notfeed.example/", "<html>no</html>");
        assert_eq!(
            feed_title("news.example/feed.xml", &f).unwrap(),
            "Garden News"
        );
        assert!(feed_title("notfeed.example", &f).is_err());
        assert!(feed_title("unknown.example/x", &f).is_err());
        assert!(feed_title("", &f).is_err());
    }

    // ---- pruning ----

    fn book(id: u64, key: Option<&str>, day: u32) -> Book {
        let mut meta = Metadata::default();
        if let Some(k) = key {
            meta.identifiers.push(Identifier {
                scheme: ID_SCHEME.into(),
                value: k.into(),
                xml_id: None,
            });
        }
        Book {
            id,
            file: String::new(),
            cover: None,
            meta,
            added: t(day, 8, 0),
            modified: t(day, 8, 0),
            last_read: None,
            size: 0,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
        }
    }

    #[test]
    fn prune_removes_only_the_oldest_issues_of_that_subscription() {
        let books = vec![
            book(1, Some("a"), 1),
            book(2, Some("a"), 5),
            book(3, Some("b"), 2),
            book(4, Some("a"), 3),
            book(5, None, 1),
            book(6, Some("a"), 9),
        ];
        assert_eq!(prune_plan(&books, "a", 2), vec![4, 1]);
        assert_eq!(prune_plan(&books, "a", 4), Vec::<u64>::new());
        assert_eq!(prune_plan(&books, "a", 10), Vec::<u64>::new());
        assert_eq!(prune_plan(&books, "b", 1), Vec::<u64>::new());
        assert_eq!(prune_plan(&books, "none", 1), Vec::<u64>::new());
    }

    #[test]
    fn prune_never_deletes_the_newest_issue_even_with_keep_zero() {
        let books = vec![book(1, Some("a"), 1), book(2, Some("a"), 2)];
        assert_eq!(prune_plan(&books, "a", 0), vec![1]);
    }

    #[test]
    fn prune_breaks_ties_by_id() {
        let books = vec![book(1, Some("a"), 1), book(2, Some("a"), 1)];
        assert_eq!(prune_plan(&books, "a", 1), vec![1]);
    }

    // ---- form ----

    #[test]
    fn the_form_builds_a_recipe() {
        let r = recipe_from_form(
            " Mine ",
            "news.example/feed.xml\n\n  https://b.example/rss  \n",
            7,
            true,
            Some(Schedule::Hourly { minute: 0 }),
            None,
        )
        .unwrap();
        assert_eq!(r.title, "Mine");
        assert_eq!(r.feeds.len(), 2);
        assert_eq!(r.feeds[0].url, "https://news.example/feed.xml");
        assert_eq!(r.max_articles, 7);
        assert!(r.use_full_article);
        assert!(r.schedule.is_some());
        assert_eq!(
            recipe_from_form("T", "a.example/f", 0, false, None, None)
                .unwrap()
                .max_articles,
            1
        );
    }

    #[test]
    fn editing_keeps_the_settings_the_form_does_not_show() {
        let mut base = Recipe::new("Old", &["https://old.example/f"]);
        base.exclude = vec!["sport".into()];
        base.oldest_days = 3;
        base.max_images = 9;
        let r = recipe_from_form("New", "new.example/f", 4, false, None, Some(&base)).unwrap();
        assert_eq!(r.title, "New");
        assert_eq!(r.feeds[0].url, "https://new.example/f");
        assert_eq!(r.exclude, vec!["sport".to_string()]);
        assert_eq!((r.oldest_days, r.max_images, r.max_articles), (3, 9, 4));
    }

    #[test]
    fn the_form_explains_what_is_missing() {
        let e = |t: &str, f: &str| {
            format!(
                "{:#}",
                recipe_from_form(t, f, 5, false, None, None).err().unwrap()
            )
        };
        assert!(e("", "a.example/f").contains("name"));
        assert!(e("T", "  \n ").contains("at least one feed"));
        assert!(e("T", "two words").contains("spaces"));
        assert!(e("T", "ftp://a.example/f").contains("http"));
    }

    // ---- per-feed switch and OPML ----

    #[test]
    fn switched_off_feeds_are_skipped_when_running() {
        let f = MockFetcher::new()
            .with_body("https://news.example/feed.xml", FEED)
            .on("https://bad.example/feed.xml", MockReply::Status(500));
        let mut r = Recipe::new("Two", &["https://news.example/feed.xml"]);
        r.feeds.push(FeedSpec {
            url: "https://bad.example/feed.xml".into(),
            title: None,
            max_articles: None,
        });
        let mut s = Subscription::new(r);
        s.disabled_feeds = vec!["https://bad.example/feed.xml".into()];
        assert_eq!(s.active_recipe().feeds.len(), 1);
        let (_, summary) = run_subscription(&s, &f, t(12, 9, 0)).unwrap();
        assert!(!summary.contains("problem"), "{summary}");
        s.disabled_feeds
            .push("https://news.example/feed.xml".into());
        let e = run_subscription(&s, &f, t(12, 9, 0)).err().unwrap();
        assert!(format!("{e:#}").contains("switched off"));
    }

    #[test]
    fn feed_lines_mark_and_unmark_feeds() {
        let (clean, off) = feed_lines("a.example/f\n# b.example/g\n#\n\n  c.example/h ").unwrap();
        assert_eq!(clean, "a.example/f\nb.example/g\nc.example/h");
        assert_eq!(off, vec!["https://b.example/g".to_string()]);
        assert!(feed_lines("# two words").is_err());
        let mut r = Recipe::new("X", &["https://a.example/f", "https://b.example/g"]);
        r.schedule = None;
        let mut s = Subscription::new(r);
        s.disabled_feeds = vec!["https://b.example/g".into()];
        let text = s.feeds_text();
        assert_eq!(text, "https://a.example/f\n# https://b.example/g");
        let (clean, off) = feed_lines(&text).unwrap();
        assert_eq!(off, s.disabled_feeds);
        assert!(clean.contains("b.example/g"));
    }

    #[test]
    fn subscriptions_without_the_field_load_with_every_feed_on() {
        let p = tmp("nofield");
        let recipe = serde_json::to_value(Recipe::new("S", &["https://a.example/f"])).unwrap();
        std::fs::write(
            &p,
            serde_json::json!({"subscriptions":[{"key":"s","recipe":recipe}]}).to_string(),
        )
        .unwrap();
        assert!(NewsStore::load(Some(p))
            .get("s")
            .unwrap()
            .disabled_feeds
            .is_empty());
    }

    #[test]
    fn opml_feeds_are_found_inside_folders_without_duplicates() {
        let opml = r#"<?xml version="1.0"?><opml version="2.0"><head><title>x</title></head><body>
            <outline text="Tech"><outline type="rss" text="Alpha" xmlUrl="https://a.example/feed"/>
              <outline type="rss" title="Beta" text="beta" XMLURL="https://b.example/rss"/></outline>
            <outline type="rss" text="Alpha again" xmlUrl="https://a.example/feed"/>
            <outline text="No address"/><outline xmlUrl="https://c.example/atom"/></body></opml>"#;
        let feeds = parse_opml(opml).unwrap();
        assert_eq!(
            feeds,
            vec![
                ("Alpha".to_string(), "https://a.example/feed".to_string()),
                ("Beta".to_string(), "https://b.example/rss".to_string()),
                (
                    "https://c.example/atom".to_string(),
                    "https://c.example/atom".to_string()
                ),
            ]
        );
        assert!(parse_opml("<opml><body/></opml>").is_err());
        assert!(parse_opml("not xml").is_err());
    }

    // ---- schedule choices ----

    #[test]
    fn manual_and_hourly_do_not_depend_on_the_offset() {
        assert_eq!(ScheduleChoice::Manual.to_schedule(off(5, 30)), None);
        assert_eq!(
            ScheduleChoice::Hourly.to_schedule(off(5, 30)),
            Some(Schedule::Hourly { minute: 0 })
        );
        assert_eq!(
            ScheduleChoice::from_schedule(None, off(2, 0)),
            ScheduleChoice::Manual
        );
        assert_eq!(
            ScheduleChoice::from_schedule(Some(&Schedule::Hourly { minute: 20 }), off(2, 0)),
            ScheduleChoice::Hourly
        );
    }

    #[test]
    fn daily_times_convert_between_local_and_utc() {
        let c = ScheduleChoice::Daily {
            hour: 7,
            minute: 30,
        };
        assert_eq!(
            c.to_schedule(utc()),
            Some(Schedule::Daily {
                hour: 7,
                minute: 30
            })
        );
        assert_eq!(
            c.to_schedule(off(2, 0)),
            Some(Schedule::Daily {
                hour: 5,
                minute: 30
            })
        );
        assert_eq!(
            c.to_schedule(off(-5, 0)),
            Some(Schedule::Daily {
                hour: 12,
                minute: 30
            })
        );
        // half-hour zone: 07:30 in India is 02:00 UTC
        assert_eq!(
            c.to_schedule(off(5, 30)),
            Some(Schedule::Daily { hour: 2, minute: 0 })
        );
    }

    #[test]
    fn times_wrap_around_midnight() {
        let c = ScheduleChoice::Daily { hour: 1, minute: 0 };
        assert_eq!(
            c.to_schedule(off(3, 0)),
            Some(Schedule::Daily {
                hour: 22,
                minute: 0
            })
        );
        let c = ScheduleChoice::Daily {
            hour: 23,
            minute: 30,
        };
        assert_eq!(
            c.to_schedule(off(-2, 0)),
            Some(Schedule::Daily {
                hour: 1,
                minute: 30
            })
        );
    }

    #[test]
    fn weekly_schedules_move_the_weekday_with_midnight() {
        let c = ScheduleChoice::Weekly {
            weekday: Weekday::Mon,
            hour: 1,
            minute: 0,
        };
        // Monday 01:00 at UTC+3 is Sunday 22:00 UTC
        assert_eq!(
            c.to_schedule(off(3, 0)),
            Some(Schedule::Weekly {
                weekday: Weekday::Sun,
                hour: 22,
                minute: 0
            })
        );
        let c = ScheduleChoice::Weekly {
            weekday: Weekday::Sun,
            hour: 23,
            minute: 0,
        };
        // Sunday 23:00 at UTC-2 is Monday 01:00 UTC
        assert_eq!(
            c.to_schedule(off(-2, 0)),
            Some(Schedule::Weekly {
                weekday: Weekday::Mon,
                hour: 1,
                minute: 0
            })
        );
        assert_eq!(
            c.to_schedule(utc()),
            Some(Schedule::Weekly {
                weekday: Weekday::Sun,
                hour: 23,
                minute: 0
            })
        );
    }

    #[test]
    fn every_choice_round_trips_in_every_offset() {
        let offsets = [
            utc(),
            off(1, 0),
            off(-8, 0),
            off(5, 30),
            off(-3, -30),
            off(12, 45),
            off(14, 0),
            off(-12, 0),
        ];
        for o in offsets {
            for wd in WEEKDAYS {
                for (h, m) in [(0, 0), (0, 59), (7, 30), (12, 0), (23, 59)] {
                    for c in [
                        ScheduleChoice::Daily { hour: h, minute: m },
                        ScheduleChoice::Weekly {
                            weekday: wd,
                            hour: h,
                            minute: m,
                        },
                    ] {
                        let s = c.to_schedule(o);
                        assert_eq!(ScheduleChoice::from_schedule(s.as_ref(), o), c, "{o}");
                    }
                }
            }
        }
        let custom = ScheduleChoice::Custom("0 6 * * 1-5".into());
        let s = custom.to_schedule(off(2, 0));
        assert_eq!(ScheduleChoice::from_schedule(s.as_ref(), off(9, 0)), custom);
    }

    #[test]
    fn converted_schedules_fire_at_the_local_time() {
        // 07:30 at UTC+2 is 05:30 UTC
        let s = ScheduleChoice::Daily {
            hour: 7,
            minute: 30,
        }
        .to_schedule(off(2, 0))
        .unwrap();
        assert_eq!(s.next_after(t(12, 6, 0)).unwrap(), t(13, 5, 30));
    }

    #[test]
    fn schedule_texts_are_plain_language() {
        assert_eq!(ScheduleChoice::Manual.text(), "Manual only");
        assert_eq!(ScheduleChoice::Hourly.text(), "Every hour");
        assert_eq!(
            ScheduleChoice::Daily {
                hour: 7,
                minute: 30
            }
            .text(),
            "Every day at 07:30"
        );
        assert_eq!(
            ScheduleChoice::Weekly {
                weekday: Weekday::Fri,
                hour: 18,
                minute: 5
            }
            .text(),
            "Every Friday at 18:05"
        );
        assert_eq!(
            ScheduleChoice::Custom("0 6 * * *".into()).text(),
            "Cron 0 6 * * *"
        );
        for w in WEEKDAYS {
            assert!(!weekday_name(w).is_empty());
            assert_eq!(WEEKDAYS[_weekday_index(w)], w);
        }
    }

    #[test]
    fn list_texts_use_the_local_offset_and_show_pauses() {
        let mut s = sub("T", Some(Schedule::Daily { hour: 5, minute: 0 }));
        assert_eq!(schedule_text(&s, off(2, 0)), "Every day at 07:00");
        s.enabled = false;
        assert_eq!(schedule_text(&s, off(2, 0)), "Every day at 07:00 (paused)");
        s.recipe.schedule = None;
        assert_eq!(schedule_text(&s, utc()), "Manual only");
        assert_eq!(status_text(&s, utc()), "Not downloaded yet");
        s.finish(t(12, 9, 5), "2 articles");
        assert_eq!(status_text(&s, off(1, 0)), "2026-11-12 10:05 · 2 articles");
    }

    #[test]
    fn dev_schedules_parse() {
        assert_eq!(ScheduleChoice::parse("").unwrap(), ScheduleChoice::Manual);
        assert_eq!(
            ScheduleChoice::parse("manual").unwrap(),
            ScheduleChoice::Manual
        );
        assert_eq!(
            ScheduleChoice::parse("hourly").unwrap(),
            ScheduleChoice::Hourly
        );
        assert_eq!(
            ScheduleChoice::parse("daily 07:30").unwrap(),
            ScheduleChoice::Daily {
                hour: 7,
                minute: 30
            }
        );
        assert_eq!(
            ScheduleChoice::parse("weekly mon 06:00").unwrap(),
            ScheduleChoice::Weekly {
                weekday: Weekday::Mon,
                hour: 6,
                minute: 0
            }
        );
        for bad in [
            "daily",
            "daily 25:00",
            "daily 7",
            "daily 07:61",
            "weekly someday 07:00",
            "yearly",
        ] {
            assert!(ScheduleChoice::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn weekday_arithmetic_matches_chrono() {
        // 2026-11-12 is a Thursday
        assert_eq!(t(12, 0, 0).weekday(), Weekday::Thu);
        assert_eq!(shift(Weekday::Thu, 0, 0, -1), (Weekday::Wed, 23, 59));
        assert_eq!(shift(Weekday::Thu, 23, 59, 1), (Weekday::Fri, 0, 0));
        assert_eq!(shift(Weekday::Mon, 0, 0, -1), (Weekday::Sun, 23, 59));
        assert_eq!(shift(Weekday::Thu, 12, 0, 0), (Weekday::Thu, 12, 0));
    }
}
