//! What the user chose for conversions: how many run at a time (C10) and the options of the
//! formats that have an options dialog (C20): EPUB, KEPUB, MOBI, AZW3 and FB2. Stored in
//! `~/.config/bookx/convert.json`. No GTK in here, so it is unit tested; the dialog is
//! `ui/convert_options.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use bookx_convert::queue::{clamp_parallel, default_parallel, Limiter};
use bookx_convert::ConvertOptions;
use serde::{Deserialize, Serialize};

/// The formats of the *Convert To* menu that have options of their own (`export::TARGETS` ids).
pub const FORMATS: [&str; 5] = ["epub", "kepub", "mobi", "azw3", "fb2"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConvertPrefs {
    /// Books converted at the same time; `None` is half the processors.
    pub parallel: Option<usize>,
    /// Options chosen per format id; a format that is missing uses its built-in options.
    pub formats: BTreeMap<String, ConvertOptions>,
}

impl ConvertPrefs {
    pub fn from_json(text: &str) -> ConvertPrefs {
        let mut p: ConvertPrefs = serde_json::from_str(text).unwrap_or_default();
        p.formats.retain(|k, _| FORMATS.contains(&k.as_str()));
        p.parallel = p.parallel.map(clamp_parallel);
        p
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Conversions at a time on a machine with `cpus` processors.
    pub fn parallel_for(&self, cpus: usize) -> usize {
        self.parallel
            .map(clamp_parallel)
            .unwrap_or_else(|| default_parallel(cpus))
    }

    /// The options saved for a format, when it has any.
    pub fn options_for(&self, format: &str) -> Option<&ConvertOptions> {
        self.formats.get(format)
    }

    /// Saves the options of a format; options equal to `builtin` are forgotten instead (the
    /// format follows the built-in options again, also when those change in a new version).
    pub fn set_options(&mut self, format: &str, options: ConvertOptions, builtin: &ConvertOptions) {
        if !FORMATS.contains(&format) {
            return;
        }
        if &options == builtin {
            self.formats.remove(format);
        } else {
            self.formats.insert(format.to_string(), options);
        }
    }
}

fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("convert.json"))
}

pub fn load_from(p: &Path) -> ConvertPrefs {
    std::fs::read_to_string(p)
        .map(|t| ConvertPrefs::from_json(&t))
        .unwrap_or_default()
}

pub fn save_to(p: &Path, prefs: &ConvertPrefs) -> std::io::Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, prefs.to_json())?;
    std::fs::rename(&tmp, p)
}

pub fn load() -> ConvertPrefs {
    path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(prefs: &ConvertPrefs) {
    if let Some(p) = path() {
        let _ = save_to(&p, prefs);
    }
}

fn cpus() -> usize {
    std::thread::available_parallelism().map_or(2, |n| n.get())
}

static LIMITER: OnceLock<Limiter> = OnceLock::new();

/// The gate every conversion of the app goes through.
pub fn limiter() -> &'static Limiter {
    LIMITER.get_or_init(|| Limiter::new(load().parallel_for(cpus())))
}

/// Applies a new "convert N at a time" to the running gate and stores it.
pub fn set_parallel(n: usize) {
    let mut p = load();
    p.parallel = Some(clamp_parallel(n));
    save(&p);
    limiter().set_limit(n);
}

/// How many books are converted at a time now.
pub fn parallel_now() -> usize {
    limiter().limit()
}

/// Options for a format: the user's, else `None` (the built-in ones apply).
pub fn user_options(format: &str) -> Option<ConvertOptions> {
    if !FORMATS.contains(&format) {
        return None;
    }
    load().options_for(format).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_half_the_processors_and_a_choice_is_clamped() {
        let mut p = ConvertPrefs::default();
        assert_eq!(p.parallel_for(8), 4);
        assert_eq!(p.parallel_for(1), 1);
        p.parallel = Some(0);
        assert_eq!(p.parallel_for(8), 1);
        p.parallel = Some(3);
        assert_eq!(p.parallel_for(64), 3);
    }

    #[test]
    fn options_equal_to_the_built_in_ones_are_not_stored() {
        let mut p = ConvertPrefs::default();
        let builtin = ConvertOptions::default();
        let mut o = builtin.clone();
        o.smarten_punctuation = true;
        p.set_options("epub", o.clone(), &builtin);
        assert_eq!(p.options_for("epub"), Some(&o));
        p.set_options("epub", builtin.clone(), &builtin);
        assert_eq!(p.options_for("epub"), None);
        p.set_options("docx", o, &builtin);
        assert!(p.formats.is_empty(), "only the five formats have options");
    }

    #[test]
    fn a_file_round_trips_and_damage_or_strangers_are_dropped() {
        let dir = std::env::temp_dir().join(format!("bookx-convprefs-{}", uuid::Uuid::new_v4()));
        let file = dir.join("convert.json");
        let mut p = ConvertPrefs {
            parallel: Some(2),
            ..Default::default()
        };
        let o = ConvertOptions {
            remove_unused_css: true,
            ..Default::default()
        };
        p.formats.insert("mobi".into(), o);
        save_to(&file, &p).unwrap();
        assert_eq!(load_from(&file), p);
        std::fs::write(&file, "{ not json").unwrap();
        assert_eq!(load_from(&file), ConvertPrefs::default());
        let strange = ConvertPrefs::from_json(
            r#"{"parallel":9999,"formats":{"evil":{},"epub":{"smarten_punctuation":true}}}"#,
        );
        assert_eq!(strange.parallel, Some(bookx_convert::queue::MAX_PARALLEL));
        assert!(strange.formats.contains_key("epub") && !strange.formats.contains_key("evil"));
        assert!(load_from(&dir.join("missing.json")) == ConvertPrefs::default());
    }
}
