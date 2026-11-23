//! When to download a dictionary: the settings, the rules for the automatic downloads (one per language,
//! not for a language that has one, not while offline, not again at once after a failure, a big one only
//! with the user's consent) and the state the look-up popover shows. No GTK: the glue is in
//! `ui/dict_service.rs`.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::dict_catalog::{Catalog, language_name};
use crate::dict_install::Presence;
use crate::i18n::{tr, trf};

/// A download bigger than this is not started by itself: the user is asked first, once.
pub const BIG_BYTES: u64 = 50 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Download the dictionaries of the languages of my books by themselves.
    pub auto_download: bool,
    /// The first run is done: the languages of the books that were already there then were noted
    /// without downloading anything (only books that come later start a download).
    pub baseline_done: bool,
    /// Languages that were looked at once (a download was started, or the language was there already).
    pub considered: Vec<String>,
    /// Languages the user agreed to download although the dictionary is big.
    pub big_ok: Vec<String>,
    /// Languages whose download did not work (the network was away): tried again later.
    pub pending: Vec<String>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            auto_download: true,
            baseline_done: false,
            considered: Vec::new(),
            big_ok: Vec::new(),
            pending: Vec::new(),
        }
    }
}

/// `BOOKX_DICT_AUTO=on|off` overrides the setting (the tests switch it off everywhere but where they
/// test it, so no scenario goes to the network by itself).
pub fn auto_enabled(p: &Prefs) -> bool {
    auto_enabled_with(std::env::var("BOOKX_DICT_AUTO").ok().as_deref(), p)
}

/// [`auto_enabled`] with the value of the variable given (tests pass it instead of changing the process environment).
pub fn auto_enabled_with(var: Option<&str>, p: &Prefs) -> bool {
    match var {
        Some("off" | "0" | "false") => false,
        Some("on" | "1" | "true") => true,
        _ => p.auto_download,
    }
}

/// `BOOKX_OFFLINE` set: the app is told not to use the network.
pub fn offline() -> bool {
    crate::privacy::is_offline()
}

/// How long to wait after the n-th failure in a row before the next automatic try (seconds).
pub fn retry_after(failures: u32) -> u64 {
    if failures == 0 {
        return 0;
    }
    (60u64 << (failures - 1).min(10)).min(6 * 3600)
}

#[derive(Debug, Default, Clone)]
pub struct Failures {
    /// language -> (failures in a row, when the last one was, unix seconds)
    map: HashMap<String, (u32, i64)>,
}

impl Failures {
    pub fn record(&mut self, lang: &str, now: i64) {
        let e = self.map.entry(lang.to_string()).or_insert((0, now));
        e.0 += 1;
        e.1 = now;
    }

    pub fn clear(&mut self, lang: &str) {
        self.map.remove(lang);
    }

    /// Seconds still to wait before an automatic try, `None` when it may be tried now.
    pub fn wait_left(&self, lang: &str, now: i64) -> Option<u64> {
        let (n, at) = *self.map.get(lang)?;
        let due = at + retry_after(n) as i64;
        (due > now).then(|| (due - now) as u64)
    }

    #[cfg(test)]
    pub fn count(&self, lang: &str) -> u32 {
        self.map.get(lang).map_or(0, |e| e.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The catalogue has nothing for this language.
    NoSource,
    /// A dictionary for it is installed (or the user's own counts for every language).
    Have,
    /// Its download is running.
    Running,
    /// Automatic downloads are off (the user can still ask for one).
    Off,
    /// No network: tried again later.
    Offline,
    /// Failed a moment ago: tried again in this many seconds.
    Wait(u64),
    /// Big: asked once before the first automatic download.
    NeedsConsent { size: u64 },
    /// Start it.
    Start { dict: String },
}

pub struct Ask<'a> {
    pub lang: &'a str,
    /// The user asked (a button), not the app by itself: switches, waits and consent do not apply.
    pub explicit: bool,
    pub auto: bool,
    pub presence: &'a Presence,
    pub running: &'a HashSet<String>,
    pub offline: bool,
    pub failures: &'a Failures,
    pub now: i64,
    pub big_ok: &'a [String],
    pub catalog: &'a Catalog,
}

pub fn decide(a: &Ask) -> Decision {
    let Some(best) = a.catalog.best(a.lang) else {
        return Decision::NoSource;
    };
    if a.presence.has(a.lang) {
        return Decision::Have;
    }
    if a.running.contains(a.lang) {
        return Decision::Running;
    }
    if a.offline {
        return Decision::Offline;
    }
    if !a.explicit {
        if !a.auto {
            return Decision::Off;
        }
        if let Some(s) = a.failures.wait_left(a.lang, a.now) {
            return Decision::Wait(s);
        }
        if best.size > BIG_BYTES && !a.big_ok.iter().any(|l| l == a.lang) {
            return Decision::NeedsConsent { size: best.size };
        }
    }
    Decision::Start {
        dict: best.id.clone(),
    }
}

/// The languages of a library that are new: not looked at before. The first time everything that is
/// there is only noted (`baseline`): old books do not start downloads, books added later do.
pub fn new_languages(libs: &[String], p: &mut Prefs) -> Vec<String> {
    let mut fresh: Vec<String> = Vec::new();
    for l in libs {
        if !l.is_empty() && !p.considered.contains(l) && !fresh.contains(l) {
            fresh.push(l.clone());
        }
    }
    p.considered.extend(fresh.iter().cloned());
    if !p.baseline_done {
        p.baseline_done = true;
        return Vec::new();
    }
    fresh
}

pub fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb < 0.95 {
        format!("{} KB", (bytes / 1024).max(1))
    } else {
        format!("{mb:.0} MB")
    }
}

/// What the dictionary page of the look-up popover shows besides the definition.
#[derive(Debug, Clone, PartialEq)]
pub enum Prompt {
    /// A dictionary is there (or no language is known): the definition.
    Hidden,
    /// There is none for the language: offer the download.
    Offer {
        lang: String,
        size: u64,
        title: String,
    },
    /// The download runs.
    Working {
        lang: String,
        fraction: Option<f64>,
        line: String,
    },
    /// It did not work.
    Failed {
        lang: String,
        message: String,
        offline: bool,
    },
}

/// The state for a look-up. `running` is the progress of this language's download when there is one,
/// `failed` the message of the last failure.
pub fn prompt_for(
    lang: &str,
    presence: &Presence,
    catalog: &Catalog,
    running: Option<(Option<f64>, String)>,
    failed: Option<(String, bool)>,
) -> Prompt {
    let Some(best) = catalog.best(lang) else {
        return Prompt::Hidden;
    };
    if presence.has(lang) {
        return Prompt::Hidden;
    }
    if let Some((fraction, line)) = running {
        return Prompt::Working {
            lang: lang.to_string(),
            fraction,
            line,
        };
    }
    if let Some((message, offline)) = failed {
        return Prompt::Failed {
            lang: lang.to_string(),
            message,
            offline,
        };
    }
    Prompt::Offer {
        lang: lang.to_string(),
        size: best.size,
        title: best.title.clone(),
    }
}

impl Prompt {
    pub fn visible(&self) -> bool {
        *self != Prompt::Hidden
    }

    pub fn heading(&self) -> String {
        match self {
            Prompt::Hidden => String::new(),
            Prompt::Offer { lang, .. }
            | Prompt::Working { lang, .. }
            | Prompt::Failed { lang, .. } => {
                trf("No dictionary for {} yet", &[&language_name(lang)])
            }
        }
    }

    pub fn body(&self) -> String {
        match self {
            Prompt::Hidden => String::new(),
            Prompt::Offer { title, .. } => trf(
                "Bookx can download {} (free, works offline afterwards) and show the definition at once.",
                &[title],
            ),
            Prompt::Working { line, .. } => line.clone(),
            Prompt::Failed {
                message, offline, ..
            } => {
                if *offline {
                    tr(
                        "There is no connection to the internet. Bookx will try again later, or press Try again.",
                    )
                } else {
                    trf("The download did not work: {}", &[message])
                }
            }
        }
    }

    /// The text of the main button, `None` while the download runs (the button is then disabled).
    pub fn primary(&self) -> String {
        match self {
            Prompt::Offer { size, .. } => trf("Download dictionary ({})", &[&size_text(*size)]),
            Prompt::Working { .. } => tr("Downloading"),
            Prompt::Failed { .. } => tr("Try again"),
            Prompt::Hidden => String::new(),
        }
    }

    pub fn primary_enabled(&self) -> bool {
        !matches!(self, Prompt::Working { .. })
    }

    pub fn progress(&self) -> Option<f64> {
        match self {
            Prompt::Working { fraction, .. } => Some(fraction.unwrap_or(0.0)),
            _ => None,
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Prompt::Hidden => "hidden",
            Prompt::Offer { .. } => "offer",
            Prompt::Working { .. } => "working",
            Prompt::Failed { .. } => "failed",
        }
    }
}

/// A download error that looks like "no network" rather than "the file is wrong".
pub fn looks_offline(message: &str) -> bool {
    let m = message.to_lowercase();
    [
        "dns",
        "resolve",
        "connect",
        "timed out",
        "timeout",
        "network",
        "unreachable",
        "offline",
    ]
    .iter()
    .any(|k| m.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict_install::Presence;

    fn cat() -> Catalog {
        crate::dict_catalog::catalog().clone()
    }

    fn base<'a>(
        lang: &'a str,
        presence: &'a Presence,
        running: &'a HashSet<String>,
        failures: &'a Failures,
        catalog: &'a Catalog,
    ) -> Ask<'a> {
        Ask {
            lang,
            explicit: false,
            auto: true,
            presence,
            running,
            offline: false,
            failures,
            now: 1000,
            big_ok: &[],
            catalog,
        }
    }

    fn go<'a>(
        c: &'a Catalog,
        presence: &'a Presence,
        running: &'a HashSet<String>,
        fails: &'a Failures,
        f: impl FnOnce(&mut Ask<'a>),
    ) -> Decision {
        let mut a = base("de", presence, running, fails, c);
        f(&mut a);
        decide(&a)
    }

    #[test]
    fn the_rules_for_a_download() {
        let c = cat();
        let none = Presence::default();
        let running = HashSet::new();
        let fails = Failures::default();
        let start = Decision::Start {
            dict: "wiktionary-de".into(),
        };
        assert_eq!(go(&c, &none, &running, &fails, |_| {}), start);
        assert_eq!(
            go(&c, &none, &running, &fails, |a| a.lang = "xx"),
            Decision::NoSource
        );
        assert_eq!(
            go(&c, &none, &running, &fails, |a| a.auto = false),
            Decision::Off
        );
        assert_eq!(
            go(&c, &none, &running, &fails, |a| {
                a.auto = false;
                a.explicit = true
            }),
            start,
            "a click is always allowed"
        );
        assert_eq!(
            go(&c, &none, &running, &fails, |a| a.offline = true),
            Decision::Offline
        );
        assert_eq!(
            go(&c, &none, &running, &fails, |a| {
                a.offline = true;
                a.explicit = true
            }),
            Decision::Offline,
            "no network, no download, even when asked"
        );
        let have = Presence {
            langs: ["de".to_string()].into(),
            universal: false,
        };
        assert_eq!(go(&c, &have, &running, &fails, |_| {}), Decision::Have);
        let any = Presence {
            langs: HashSet::new(),
            universal: true,
        };
        assert_eq!(
            go(&c, &any, &running, &fails, |_| {}),
            Decision::Have,
            "the user's own dictionary counts"
        );
        let run: HashSet<String> = ["de".to_string()].into();
        assert_eq!(
            go(&c, &none, &run, &fails, |_| {}),
            Decision::Running,
            "one job per language"
        );
    }

    #[test]
    fn a_failure_makes_the_next_automatic_try_wait_longer() {
        assert_eq!(retry_after(0), 0);
        assert_eq!(
            [retry_after(1), retry_after(2), retry_after(3)],
            [60, 120, 240]
        );
        assert_eq!(retry_after(40), 6 * 3600);
        let mut f = Failures::default();
        assert_eq!(f.wait_left("de", 1000), None);
        f.record("de", 1000);
        assert_eq!(f.wait_left("de", 1010), Some(50));
        assert_eq!(f.wait_left("de", 1060), None);
        f.record("de", 1100);
        assert_eq!(f.wait_left("de", 1100), Some(120));
        assert_eq!(f.count("de"), 2);
        let c = cat();
        let (none, running) = (Presence::default(), HashSet::new());
        let mut a = base("de", &none, &running, &f, &c);
        a.now = 1100;
        assert_eq!(decide(&a), Decision::Wait(120));
        a.explicit = true;
        assert!(
            matches!(decide(&a), Decision::Start { .. }),
            "a click does not wait"
        );
        f.clear("de");
        assert_eq!(f.wait_left("de", 1100), None);
    }

    #[test]
    fn a_big_dictionary_needs_the_users_word_once() {
        let mut c = cat();
        c.languages[0].dictionaries[0].size = BIG_BYTES + 1;
        let code = c.languages[0].code.clone();
        let (none, running, fails) = (Presence::default(), HashSet::new(), Failures::default());
        let mut a = base(&code, &none, &running, &fails, &c);
        assert_eq!(
            decide(&a),
            Decision::NeedsConsent {
                size: BIG_BYTES + 1
            }
        );
        let ok = [code.clone()];
        a.big_ok = &ok;
        assert!(matches!(decide(&a), Decision::Start { .. }));
        a.big_ok = &[];
        a.explicit = true;
        assert!(
            matches!(decide(&a), Decision::Start { .. }),
            "pressing the button is the consent"
        );
    }

    #[test]
    fn old_books_do_not_start_downloads_new_ones_do() {
        let mut p = Prefs::default();
        let libs = vec!["en".to_string(), "de".to_string(), "en".to_string()];
        assert!(
            new_languages(&libs, &mut p).is_empty(),
            "the first run only notes what is there"
        );
        assert_eq!(p.considered, ["en", "de"]);
        assert!(new_languages(&libs, &mut p).is_empty());
        let more = vec![
            "en".to_string(),
            "fr".to_string(),
            "fr".to_string(),
            String::new(),
        ];
        assert_eq!(
            new_languages(&more, &mut p),
            ["fr"],
            "a new language: one request"
        );
        assert!(new_languages(&more, &mut p).is_empty(), "and not again");
    }

    #[test]
    fn the_settings_default_to_on_and_old_files_read() {
        assert!(Prefs::default().auto_download);
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.auto_download && !p.baseline_done);
        let p: Prefs =
            serde_json::from_str(r#"{"auto_download":false,"considered":["en"]}"#).unwrap();
        assert!(!p.auto_download);
        assert_eq!(p.considered, ["en"]);
    }

    #[test]
    fn the_popover_state_model() {
        let c = cat();
        let none = Presence::default();
        let offer = prompt_for("de", &none, &c, None, None);
        assert!(matches!(&offer, Prompt::Offer { lang, .. } if lang == "de"));
        assert_eq!(offer.heading(), "No dictionary for German yet");
        assert!(
            offer.primary().starts_with("Download dictionary ("),
            "{}",
            offer.primary()
        );
        assert!(offer.primary().contains("MB"));
        assert!(offer.primary_enabled() && offer.progress().is_none());
        assert_eq!(offer.key(), "offer");
        // running: the progress, the button disabled
        let w = prompt_for("de", &none, &c, Some((Some(0.4), "3 of 7 MB".into())), None);
        assert_eq!(
            (w.key(), w.progress(), w.primary_enabled()),
            ("working", Some(0.4), false)
        );
        assert_eq!(w.body(), "3 of 7 MB");
        let w0 = prompt_for("de", &none, &c, Some((None, "Starting".into())), None);
        assert_eq!(w0.progress(), Some(0.0));
        // failed
        let f = prompt_for("de", &none, &c, None, Some(("boom".into(), false)));
        assert_eq!((f.key(), f.primary().as_str()), ("failed", "Try again"));
        assert!(f.body().contains("boom"));
        let off = prompt_for("de", &none, &c, None, Some(("dns".into(), true)));
        assert!(
            off.body().contains("no connection") || off.body().contains("There is no connection")
        );
        // nothing to offer: hidden
        assert_eq!(prompt_for("xx", &none, &c, None, None), Prompt::Hidden);
        let have = Presence {
            langs: ["de".to_string()].into(),
            universal: false,
        };
        assert_eq!(prompt_for("de", &have, &c, None, None), Prompt::Hidden);
        assert!(!Prompt::Hidden.visible() && offer.visible());
    }

    #[test]
    fn sizes_and_offline_errors() {
        assert_eq!(size_text(1_478_878), "1 MB");
        assert_eq!(size_text(21_839_699), "21 MB");
        assert_eq!(size_text(300_000), "292 KB");
        assert!(looks_offline(
            "error sending request: Dns Failed: failed to lookup address"
        ));
        assert!(looks_offline("Connection refused (os error 111)"));
        assert!(!looks_offline("the dictionary is empty"));
    }

    #[test]
    fn the_environment_overrides_the_setting() {
        let p = Prefs::default();
        assert!(auto_enabled_with(None, &p));
        let off = Prefs {
            auto_download: false,
            ..Prefs::default()
        };
        assert!(!auto_enabled_with(None, &off));
        assert!(!auto_enabled_with(Some("off"), &p));
        assert!(!auto_enabled_with(Some("0"), &p));
        assert!(auto_enabled_with(Some("on"), &off));
        assert!(auto_enabled_with(Some("true"), &off));
        assert!(auto_enabled_with(Some("anything else"), &p));
    }
}
