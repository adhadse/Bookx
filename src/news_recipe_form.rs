//! What the *Edit Feeds* view of a news subscription collects and how it becomes a recipe
//! (`bookx_sources::recipe::Recipe`): per feed a section title, the address, full articles or the
//! feed text, an own article limit and an on/off switch; for the whole subscription the filters
//! (patterns for titles and addresses, CSS selectors to keep or remove), the language, the title of
//! the issue and the images. The schedule stays with the subscription form. No GTK in here, so it
//! is unit tested; the view is `ui/news_recipes.rs`.

use anyhow::{bail, Result};
use bookx_sources::recipe::{FeedSpec, Recipe};

use crate::i18n::{tr, trf};
use crate::news::Subscription;

/// Most feeds in one subscription.
pub const MAX_FEEDS: usize = 40;
/// Most lines in a filter box, and the longest line.
pub const MAX_LINES: usize = 40;
pub const MAX_LINE: usize = 300;
/// Largest article limit of one feed.
pub const MAX_ARTICLES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// What the subscription says.
    #[default]
    Follow,
    /// Download the article's page.
    Full,
    /// Use the text in the feed.
    Summary,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Follow, Mode::Full, Mode::Summary];

    pub fn of(full_article: Option<bool>) -> Mode {
        match full_article {
            None => Mode::Follow,
            Some(true) => Mode::Full,
            Some(false) => Mode::Summary,
        }
    }

    pub fn full_article(self) -> Option<bool> {
        match self {
            Mode::Follow => None,
            Mode::Full => Some(true),
            Mode::Summary => Some(false),
        }
    }

    pub fn from_index(i: u32) -> Mode {
        Mode::ALL[(i as usize).min(2)]
    }

    pub fn index(self) -> u32 {
        Mode::ALL.iter().position(|m| *m == self).unwrap_or(0) as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedForm {
    pub url: String,
    pub title: String,
    pub mode: Mode,
    /// 0 follows the subscription.
    pub max_articles: usize,
    pub enabled: bool,
}

impl FeedForm {
    pub fn new(url: &str) -> FeedForm {
        FeedForm {
            url: url.to_string(),
            title: String::new(),
            mode: Mode::Follow,
            max_articles: 0,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeForm {
    pub feeds: Vec<FeedForm>,
    /// One pattern per line.
    pub include: String,
    pub exclude: String,
    /// One CSS selector per line.
    pub keep: String,
    pub remove: String,
    pub language: String,
    pub title_template: String,
    pub images: bool,
}

pub fn join_lines(v: &[String]) -> String {
    v.join("\n")
}

/// The lines of a box: trimmed, blank ones dropped.
pub fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn checked_lines(text: &str, what: &str) -> Result<Vec<String>> {
    let v = lines(text);
    if v.len() > MAX_LINES {
        bail!("{}", trf("{}: at most {} lines.", &[&what, &MAX_LINES]));
    }
    if v.iter().any(|l| l.chars().count() > MAX_LINE) {
        bail!(
            "{}",
            trf(
                "{}: a line is longer than {} characters.",
                &[&what, &MAX_LINE]
            )
        );
    }
    Ok(v)
}

/// A language tag such as `en`, `pt-BR` or `zh-Hant`.
pub fn valid_language(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or("");
    (2..=3).contains(&first.len())
        && first.chars().all(|c| c.is_ascii_alphabetic())
        && s.len() <= 35
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

impl RecipeForm {
    pub fn from_subscription(s: &Subscription) -> RecipeForm {
        let r = &s.recipe;
        RecipeForm {
            feeds: r
                .feeds
                .iter()
                .map(|f| FeedForm {
                    url: f.url.clone(),
                    title: f.title.clone().unwrap_or_default(),
                    mode: Mode::of(f.full_article),
                    max_articles: f.max_articles.unwrap_or(0),
                    enabled: !s.disabled_feeds.contains(&f.url),
                })
                .collect(),
            include: join_lines(&r.include),
            exclude: join_lines(&r.exclude),
            keep: join_lines(&r.keep_selectors),
            remove: join_lines(&r.remove_selectors),
            language: r.language.clone(),
            title_template: r.title_template.clone(),
            images: r.include_images,
        }
    }

    /// The recipe this form makes from `base` (what the form does not show stays) and the feeds
    /// that are switched off. The first problem comes back as the text to show.
    pub fn build(&self, base: &Subscription) -> Result<(Recipe, Vec<String>)> {
        if self.feeds.is_empty() {
            bail!("{}", tr("Add at least one feed."));
        }
        if self.feeds.len() > MAX_FEEDS {
            bail!("{}", trf("At most {} feeds.", &[&MAX_FEEDS]));
        }
        let mut specs: Vec<FeedSpec> = Vec::new();
        let mut disabled: Vec<String> = Vec::new();
        for f in &self.feeds {
            let url = crate::web_import::normalize_url(&f.url)?;
            if specs.iter().any(|s| s.url == url) {
                bail!("{}", trf("The feed {} is in the list twice.", &[&url]));
            }
            if f.max_articles > MAX_ARTICLES {
                bail!("{}", trf("At most {} articles per feed.", &[&MAX_ARTICLES]));
            }
            let title = f.title.trim();
            if title.chars().count() > 120 {
                bail!("{}", tr("A section title is too long."));
            }
            if !f.enabled {
                disabled.push(url.clone());
            }
            specs.push(FeedSpec {
                url,
                title: Some(title.to_string()).filter(|t| !t.is_empty()),
                max_articles: Some(f.max_articles).filter(|n| *n > 0),
                full_article: f.mode.full_article(),
            });
        }
        let language = self.language.trim();
        if !valid_language(language) {
            bail!("{}", tr("The language is a code such as en, de or pt-BR."));
        }
        let template = self.title_template.trim();
        if template.is_empty() || template.chars().count() > 120 {
            bail!("{}", tr("Give the issue a title."));
        }
        let mut r = base.recipe.clone();
        r.feeds = specs;
        r.include = checked_lines(&self.include, &tr("Keep only articles matching"))?;
        r.exclude = checked_lines(&self.exclude, &tr("Skip articles matching"))?;
        r.keep_selectors = checked_lines(&self.keep, &tr("Keep only these parts"))?;
        r.remove_selectors = checked_lines(&self.remove, &tr("Remove these parts"))?;
        r.language = language.to_string();
        r.title_template = template.to_string();
        r.include_images = self.images;
        r.validate()?;
        Ok((r, disabled))
    }
}

/// Applies the form to the subscription (it stays the same subscription: key, schedule, last run).
pub fn apply(form: &RecipeForm, base: &Subscription) -> Result<Subscription> {
    let (recipe, disabled) = form.build(base)?;
    let mut s = base.clone();
    s.recipe = recipe;
    s.disabled_feeds = disabled;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub() -> Subscription {
        let mut r = Recipe::new(
            "Daily",
            &["https://a.example/feed", "https://b.example/feed"],
        );
        r.feeds[0].full_article = Some(true);
        r.feeds[1].max_articles = Some(7);
        r.feeds[1].title = Some("B news".into());
        r.include = vec!["^rust".into()];
        r.keep_selectors = vec!["article".into()];
        let mut s = Subscription::new(r);
        s.disabled_feeds = vec!["https://b.example/feed".into()];
        s
    }

    #[test]
    fn the_form_shows_what_the_recipe_has() {
        let f = RecipeForm::from_subscription(&sub());
        assert_eq!(f.feeds.len(), 2);
        assert_eq!(f.feeds[0].mode, Mode::Full);
        assert!(f.feeds[0].enabled && !f.feeds[1].enabled);
        assert_eq!(
            (f.feeds[1].max_articles, f.feeds[1].title.as_str()),
            (7, "B news")
        );
        assert_eq!((f.include.as_str(), f.keep.as_str()), ("^rust", "article"));
        assert_eq!(f.language, "en");
        assert!(f.images);
    }

    #[test]
    fn an_unchanged_form_gives_back_the_same_subscription() {
        let s = sub();
        let back = apply(&RecipeForm::from_subscription(&s), &s).unwrap();
        assert_eq!(back.recipe, s.recipe);
        assert_eq!(back.disabled_feeds, s.disabled_feeds);
        assert_eq!(back.key, s.key);
    }

    #[test]
    fn edits_reach_the_recipe_and_what_the_form_does_not_show_stays() {
        let mut s = sub();
        s.recipe.max_image_bytes = 12345;
        let mut f = RecipeForm::from_subscription(&s);
        f.feeds[0].mode = Mode::Summary;
        f.feeds[0].max_articles = 5;
        f.feeds[0].title = "  Tech ".into();
        f.feeds[1].enabled = true;
        f.feeds.push(FeedForm::new("c.example/rss"));
        f.exclude = "sponsored\n\n  advert ".into();
        f.remove = ".ad, .promo".into();
        f.language = "pt-BR".into();
        f.images = false;
        let n = apply(&f, &s).unwrap();
        let r = &n.recipe;
        assert_eq!(r.feeds[0].full_article, Some(false));
        assert_eq!(
            (r.feeds[0].max_articles, r.feeds[0].title.as_deref()),
            (Some(5), Some("Tech"))
        );
        assert_eq!(r.feeds[2].url, "https://c.example/rss");
        assert_eq!(r.feeds[2].full_article, None);
        assert_eq!(r.exclude, vec!["sponsored", "advert"]);
        assert_eq!(r.remove_selectors, vec![".ad, .promo"]);
        assert_eq!((r.language.as_str(), r.include_images), ("pt-BR", false));
        assert_eq!(r.max_image_bytes, 12345);
        assert!(n.disabled_feeds.is_empty());
        assert_eq!(n.recipe.schedule, s.recipe.schedule);
    }

    #[test]
    fn a_switched_off_feed_is_remembered_by_its_address() {
        let s = sub();
        let mut f = RecipeForm::from_subscription(&s);
        f.feeds[0].enabled = false;
        f.feeds[0].url = "a.example/feed2".into();
        let n = apply(&f, &s).unwrap();
        assert_eq!(
            n.disabled_feeds,
            vec!["https://a.example/feed2", "https://b.example/feed"]
        );
    }

    #[test]
    fn problems_are_refused_with_a_reason() {
        let s = sub();
        let base = RecipeForm::from_subscription(&s);
        let bad = |edit: &dyn Fn(&mut RecipeForm)| {
            let mut f = base.clone();
            edit(&mut f);
            apply(&f, &s).unwrap_err().to_string()
        };
        assert!(bad(&|f| f.feeds.clear()).contains("at least one"));
        assert!(bad(&|f| f.feeds[0].url = "ftp://x.example".into()).contains("http"));
        assert!(bad(&|f| f.feeds[0].url = "not a url".into()).contains("spaces"));
        assert!(bad(&|f| f.feeds[1].url = f.feeds[0].url.clone()).contains("twice"));
        assert!(bad(&|f| f.include = "(unclosed".into()).contains("invalid regex"));
        assert!(bad(&|f| f.keep = "div[".into()).contains("selector"));
        assert!(bad(&|f| f.language = "english please".into()).contains("language"));
        assert!(bad(&|f| f.title_template = "  ".into()).contains("title"));
        assert!(bad(&|f| f.feeds[0].max_articles = MAX_ARTICLES + 1).contains("articles"));
        assert!(bad(&|f| f.exclude = "x\n".repeat(MAX_LINES + 1)).contains("lines"));
        assert!(bad(&|f| f.exclude = "x".repeat(MAX_LINE + 1)).contains("longer"));
        // the stored subscription is untouched by a refused edit
        assert_eq!(s, sub());
    }

    #[test]
    fn language_codes() {
        for ok in ["en", "de", "pt-BR", "zh-Hant", "fil"] {
            assert!(valid_language(ok), "{ok}");
        }
        for bad in ["", "e", "english", "en_US", "en-", "12", "en-toolongsubtag"] {
            assert!(!valid_language(bad), "{bad}");
        }
    }

    #[test]
    fn modes_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::of(m.full_article()), m);
            assert_eq!(Mode::from_index(m.index()), m);
        }
        assert_eq!(Mode::from_index(99), Mode::Summary);
    }
}
