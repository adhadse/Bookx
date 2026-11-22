//! The catalogue of dictionaries Bookx can download (`data/dictionaries.json`, made and checked by
//! `build-aux/dict-catalog.py`), and the mapping from the language of a book to one of its entries.
//! No GTK.

use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Format {
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "tar.bz2")]
    TarBz2,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Origin {
    pub url: String,
    pub format: Format,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Dict {
    pub id: String,
    pub title: String,
    /// `monolingual` or `bilingual`.
    pub kind: String,
    /// The language the definitions are in.
    #[serde(default)]
    pub gloss: String,
    pub source: String,
    pub licence: String,
    pub licence_url: String,
    /// Bytes to download (of the first origin).
    pub size: u64,
    /// Places to get it from, in order: the next one is tried when one fails.
    pub origins: Vec<Origin>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Lang {
    pub code: String,
    pub name: String,
    pub dictionaries: Vec<Dict>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub languages: Vec<Lang>,
}

impl Catalog {
    /// Reads and checks a catalogue. `allow_http` is for the tests (a server on this computer).
    pub fn parse(json: &str, allow_http: bool) -> Result<Catalog> {
        let c: Catalog =
            serde_json::from_str(json).context("the dictionary catalogue is not valid")?;
        if c.version != 1 {
            bail!("unknown catalogue version {}", c.version);
        }
        let mut ids = std::collections::HashSet::new();
        for l in &c.languages {
            if l.code.len() < 2
                || l.code.len() > 3
                || !l.code.chars().all(|c| c.is_ascii_lowercase())
            {
                bail!("bad language code {:?}", l.code);
            }
            if l.dictionaries.is_empty() {
                bail!("{}: no dictionary", l.code);
            }
            for d in &l.dictionaries {
                if !ids.insert(d.id.clone()) {
                    bail!("duplicate dictionary id {}", d.id);
                }
                if !d
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    bail!("bad dictionary id {:?}", d.id);
                }
                if d.origins.is_empty() {
                    bail!("{}: no origin", d.id);
                }
                for o in &d.origins {
                    let ok = o.url.starts_with("https://")
                        || (allow_http && o.url.starts_with("http://127.0.0.1"));
                    if !ok {
                        bail!("{}: {} is not an https address", d.id, o.url);
                    }
                    if o.sha256.len() != 64 || !o.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                        bail!("{}: bad SHA-256", d.id);
                    }
                    if o.size == 0 {
                        bail!("{}: size is 0", d.id);
                    }
                }
            }
        }
        Ok(c)
    }

    pub fn language(&self, code: &str) -> Option<&Lang> {
        self.languages.iter().find(|l| l.code == code)
    }

    /// The best dictionary of a language: the first listed.
    pub fn best(&self, code: &str) -> Option<&Dict> {
        self.language(code).and_then(|l| l.dictionaries.first())
    }

    pub fn find(&self, id: &str) -> Option<(&Lang, &Dict)> {
        self.languages
            .iter()
            .find_map(|l| l.dictionaries.iter().find(|d| d.id == id).map(|d| (l, d)))
    }
}

const BUILTIN: &str = include_str!("../data/dictionaries.json");

/// The catalogue of the app, or the one in `$BOOKX_DICT_CATALOG` (tests point it at a mock server).
pub fn catalog() -> &'static Catalog {
    static C: OnceLock<Catalog> = OnceLock::new();
    C.get_or_init(|| {
        if let Some(p) = std::env::var_os("BOOKX_DICT_CATALOG") {
            if let Ok(t) = std::fs::read_to_string(p) {
                if let Ok(c) = Catalog::parse(&t, true) {
                    return c;
                }
            }
        }
        Catalog::parse(BUILTIN, false).expect("data/dictionaries.json is valid")
    })
}

/// Language names for languages the catalogue does not list (and for the prompt).
pub const LANGUAGE_NAMES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("it", "Italian"),
    ("pt", "Portuguese"),
    ("ru", "Russian"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("tr", "Turkish"),
    ("sv", "Swedish"),
    ("cs", "Czech"),
    ("ja", "Japanese"),
    ("zh", "Chinese"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("ko", "Korean"),
    ("el", "Greek"),
    ("he", "Hebrew"),
    ("uk", "Ukrainian"),
    ("da", "Danish"),
    ("fi", "Finnish"),
    ("no", "Norwegian"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("bg", "Bulgarian"),
    ("la", "Latin"),
    ("id", "Indonesian"),
    ("vi", "Vietnamese"),
    ("th", "Thai"),
    ("fa", "Persian"),
    ("ca", "Catalan"),
];

pub fn language_name(code: &str) -> String {
    LANGUAGE_NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| n.to_string())
        .unwrap_or_else(|| code.to_string())
}

/// The two-letter code of a language tag as books write it: `en-GB` and `en_US` are `en`, `pt-BR` is
/// `pt`, `zh-Hans-CN` is `zh`, the three-letter codes (`deu`, `ger`, `fra`, `fre`) map to two letters,
/// `iw` is `he`, `nb`/`nn` are `no`. `None` for an empty or unknown tag.
pub fn normalize_language(tag: &str) -> Option<String> {
    let t = tag.trim().to_ascii_lowercase().replace('_', "-");
    let primary = t.split('-').next().unwrap_or("");
    if primary.is_empty() || !primary.chars().all(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let two = match primary {
        "iw" => "he",
        "nb" | "nn" => "no",
        "in" => "id",
        "eng" => "en",
        "spa" => "es",
        "fra" | "fre" => "fr",
        "deu" | "ger" => "de",
        "ita" => "it",
        "por" => "pt",
        "rus" => "ru",
        "nld" | "dut" => "nl",
        "pol" => "pl",
        "tur" => "tr",
        "swe" => "sv",
        "ces" | "cze" => "cs",
        "jpn" => "ja",
        "zho" | "chi" | "cmn" => "zh",
        "ara" => "ar",
        "hin" => "hi",
        "kor" => "ko",
        "ell" | "gre" => "el",
        "heb" => "he",
        "ukr" => "uk",
        "dan" => "da",
        "fin" => "fi",
        "nor" => "no",
        "hun" => "hu",
        "ron" | "rum" => "ro",
        "bul" => "bg",
        "lat" => "la",
        "ind" => "id",
        "vie" => "vi",
        "tha" => "th",
        "fas" | "per" => "fa",
        "cat" => "ca",
        p if p.len() == 2 => p,
        _ => return None,
    };
    Some(two.to_string())
}

/// The language of a book: its `dc:language` (region and script dropped), else a guess from a sample of
/// the text. Empty when neither tells.
pub fn book_language(metadata: &str, sample: &str) -> String {
    if let Some(l) = normalize_language(metadata) {
        return l;
    }
    normalize_language(&bookx_tts::text::book_language("", sample)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_of_the_app_is_valid_and_covers_the_top_languages() {
        let c = catalog();
        for code in "en es fr de it pt ru nl pl tr sv cs ja zh ar hi ko el he uk".split(' ') {
            let d = c
                .best(code)
                .unwrap_or_else(|| panic!("no dictionary for {code}"));
            assert!(!d.origins.is_empty());
            assert!(d.licence.len() > 3 && d.licence_url.starts_with("https://"));
            assert!(d.size > 100_000 && d.size < 50 << 20, "{code}: {}", d.size);
        }
        assert_eq!(c.languages.len(), 20);
        assert!(
            c.language("en").unwrap().dictionaries.len() >= 2,
            "English has an alternative"
        );
        assert_eq!(c.best("de").unwrap().kind, "bilingual");
        assert_eq!(c.best("en").unwrap().kind, "monolingual");
    }

    fn json(extra: &str) -> String {
        format!(
            r#"{{"version":1,"languages":[{{"code":"xx","name":"X","dictionaries":[{{"id":"a-1","title":"T","kind":"bilingual","source":"s","licence":"l","licence_url":"https://l","size":10,"origins":[{{"url":"{extra}","format":"zip","sha256":"{}","size":10}}]}}]}}]}}"#,
            "a".repeat(64)
        )
    }

    #[test]
    fn a_catalogue_is_checked_when_it_is_read() {
        assert!(Catalog::parse(&json("https://e.org/a.zip"), false).is_ok());
        assert!(
            Catalog::parse(&json("http://e.org/a.zip"), false).is_err(),
            "plain http"
        );
        assert!(Catalog::parse(&json("http://127.0.0.1:1/a.zip"), false).is_err());
        assert!(
            Catalog::parse(&json("http://127.0.0.1:1/a.zip"), true).is_ok(),
            "tests may use this computer"
        );
        assert!(Catalog::parse(&json("http://evil.org/a.zip"), true).is_err());
        assert!(
            Catalog::parse(
                &json("https://e.org/a.zip").replace(&"a".repeat(64), "abc"),
                false
            )
            .is_err()
        );
        assert!(
            Catalog::parse(
                &json("https://e.org/a.zip").replace("\"a-1\"", "\"../x\""),
                false
            )
            .is_err()
        );
        assert!(
            Catalog::parse(
                &json("https://e.org/a.zip").replace("\"xx\"", "\"X\""),
                false
            )
            .is_err()
        );
        assert!(Catalog::parse("{", false).is_err());
        assert!(
            Catalog::parse(
                &json("https://e.org/a.zip").replace("\"version\":1", "\"version\":2"),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn region_and_script_variants_map_to_the_language() {
        for (tag, want) in [
            ("en", "en"),
            ("en-GB", "en"),
            ("en_US", "en"),
            ("EN-us", "en"),
            ("pt-BR", "pt"),
            ("pt-PT", "pt"),
            ("zh-Hans-CN", "zh"),
            ("zh-TW", "zh"),
            ("de-AT", "de"),
            ("deu", "de"),
            ("ger", "de"),
            ("fra", "fr"),
            ("iw", "he"),
            ("nb-NO", "no"),
            (" ja ", "ja"),
        ] {
            assert_eq!(normalize_language(tag).as_deref(), Some(want), "{tag}");
        }
        for bad in ["", "  ", "123", "x-klingon-", "-"] {
            assert_eq!(normalize_language(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_book_language_falls_back_to_a_guess_from_the_text() {
        assert_eq!(book_language("fr-CA", "the quick brown fox"), "fr");
        let en = "The old man and the sea were there, and it was a long time that he had been out of the house with the boy and the dog, for they were not at home.";
        assert_eq!(book_language("", en), "en");
        let de = "Der alte Mann und das Meer waren da, und es war eine lange Zeit, dass er mit dem Jungen und dem Hund nicht zu Hause war, weil sie nicht da waren.";
        assert_eq!(book_language("", de), "de");
        assert_eq!(book_language("", "zzz"), "");
    }

    #[test]
    fn names_for_the_prompt() {
        assert_eq!(language_name("de"), "German");
        assert_eq!(language_name("qq"), "qq");
    }

    #[test]
    fn the_best_and_a_specific_dictionary_are_found() {
        let c = catalog();
        assert_eq!(c.best("en").unwrap().id, "wiktionary-en");
        let (l, d) = c.find("webster-1913").unwrap();
        assert_eq!(
            (l.code.as_str(), d.licence.as_str()),
            ("en", "Public domain")
        );
        assert!(c.find("nope").is_none());
        assert!(c.best("xx").is_none());
    }
}
