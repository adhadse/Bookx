//! Named user dictionaries for the spell checker (CE-275): plain text files, one word per line.
//!
//! Layout inside the configuration folder:
//!
//! * `spelling-words.txt`: the dictionary called **Personal** (the file older versions already wrote);
//! * `spelling-dictionaries/<name>.txt`: any other dictionary.
//!
//! A file is UTF-8, one word per line, blank lines ignored. A line that starts with `#` is a comment, except
//! `# enabled: false`, which keeps the dictionary on disk but out of the checker.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub const PERSONAL: &str = "Personal";
const PERSONAL_FILE: &str = "spelling-words.txt";
const DIR: &str = "spelling-dictionaries";

/// The user dictionaries below one configuration folder.
#[derive(Debug, Clone)]
pub struct UserDicts {
    root: PathBuf,
}

/// A dictionary file read into memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDict {
    pub name: String,
    pub enabled: bool,
    /// Sorted, without duplicates.
    pub words: Vec<String>,
}

/// Parses the file format.
pub fn parse(text: &str) -> (bool, Vec<String>) {
    let mut enabled = true;
    let mut words = Vec::new();
    for l in text.lines() {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(c) = l.strip_prefix('#') {
            if let Some(v) = c.trim().strip_prefix("enabled:") {
                enabled = !v.trim().eq_ignore_ascii_case("false");
            }
            continue;
        }
        words.push(l.to_string());
    }
    words.sort();
    words.dedup();
    (enabled, words)
}

/// Writes the file format.
pub fn serialize(enabled: bool, words: &[String]) -> String {
    let mut out = String::new();
    if !enabled {
        out.push_str("# enabled: false\n");
    }
    for w in words {
        out.push_str(w);
        out.push('\n');
    }
    out
}

/// A name that is safe as a file name: letters, digits, space, `-`, `_`, `.` (not at the start).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty()
        && n.len() <= 60
        && !n.starts_with('.')
        && n.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

impl UserDicts {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, name: &str) -> PathBuf {
        if name == PERSONAL {
            self.root.join(PERSONAL_FILE)
        } else {
            self.root.join(DIR).join(format!("{name}.txt"))
        }
    }

    /// Names of all dictionaries: Personal first (always present), then the others alphabetically.
    pub fn names(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.root.join(DIR)) {
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().to_string();
                if let Some(stem) = f.strip_suffix(".txt") {
                    if valid_name(stem) && stem != PERSONAL {
                        v.push(stem.to_string());
                    }
                }
            }
        }
        v.sort_by_key(|n| n.to_lowercase());
        v.insert(0, PERSONAL.to_string());
        v
    }

    pub fn read(&self, name: &str) -> UserDict {
        let (enabled, words) = std::fs::read_to_string(self.path(name))
            .map(|t| parse(&t))
            .unwrap_or((true, Vec::new()));
        UserDict {
            name: name.to_string(),
            enabled,
            words,
        }
    }

    pub fn all(&self) -> Vec<UserDict> {
        self.names().iter().map(|n| self.read(n)).collect()
    }

    /// Every word of every enabled dictionary.
    pub fn enabled_words(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|d| d.enabled)
            .flat_map(|d| d.words)
            .collect()
    }

    fn write(&self, d: &UserDict) -> Result<()> {
        let p = self.path(&d.name);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&p, serialize(d.enabled, &d.words))
            .with_context(|| format!("writing {}", p.display()))
    }

    pub fn create(&self, name: &str) -> Result<()> {
        let name = name.trim();
        if !valid_name(name) {
            bail!("Use letters, digits, spaces, - and _ in the name");
        }
        if self.names().iter().any(|n| n.eq_ignore_ascii_case(name)) {
            bail!("There is already a dictionary called “{name}”");
        }
        self.write(&UserDict {
            name: name.to_string(),
            enabled: true,
            words: Vec::new(),
        })
    }

    /// Deletes a dictionary. Personal cannot be deleted, only emptied.
    pub fn remove(&self, name: &str) -> Result<()> {
        if name == PERSONAL {
            bail!("The Personal dictionary cannot be deleted");
        }
        std::fs::remove_file(self.path(name)).with_context(|| format!("deleting “{name}”"))
    }

    pub fn add_word(&self, name: &str, word: &str) -> Result<()> {
        let w = word.trim();
        if w.is_empty() || w.contains(['\n', '\r']) || w.starts_with('#') {
            bail!("“{word}” cannot be added");
        }
        let mut d = self.read(name);
        if !d.words.iter().any(|x| x == w) {
            d.words.push(w.to_string());
            d.words.sort();
        }
        self.write(&d)
    }

    pub fn remove_word(&self, name: &str, word: &str) -> Result<bool> {
        let mut d = self.read(name);
        let before = d.words.len();
        d.words.retain(|w| w != word.trim());
        let changed = d.words.len() != before;
        if changed {
            self.write(&d)?;
        }
        Ok(changed)
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<()> {
        let mut d = self.read(name);
        d.enabled = enabled;
        self.write(&d)
    }

    /// Copies the words of a text file (one per line) into a dictionary. Returns the number of new words.
    pub fn import_words(&self, name: &str, file: &Path) -> Result<usize> {
        let text = std::fs::read_to_string(file)
            .with_context(|| format!("reading {} (UTF-8 text expected)", file.display()))?;
        let (_, words) = parse(&text);
        let mut d = self.read(name);
        let before = d.words.len();
        d.words.extend(words);
        d.words.sort();
        d.words.dedup();
        self.write(&d)?;
        Ok(d.words.len() - before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-udict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn file_format_round_trips() {
        let (en, w) = parse("# a comment\nzebra\n\n  apple \napple\n# enabled: false\n");
        assert!(!en);
        assert_eq!(w, ["apple", "zebra"]);
        assert_eq!(serialize(false, &w), "# enabled: false\napple\nzebra\n");
        assert_eq!(parse(&serialize(true, &w)), (true, w));
        assert!(parse("").0);
        assert!(!parse("# enabled: FALSE\n").0);
    }

    #[test]
    fn names_are_checked() {
        assert!(valid_name("Fantasy names"));
        assert!(valid_name("de_DE-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("../x"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(".hidden"));
        assert!(!valid_name(&"x".repeat(61)));
    }

    #[test]
    fn several_dictionaries_live_side_by_side() {
        let root = dir();
        let u = UserDicts::new(&root);
        assert_eq!(u.names(), [PERSONAL]);
        u.add_word(PERSONAL, "Bookx").unwrap();
        assert!(
            root.join("spelling-words.txt").is_file(),
            "same file as before"
        );
        u.create("Fantasy").unwrap();
        u.create("astronomy").unwrap();
        assert!(
            u.create("fantasy").is_err(),
            "names are unique ignoring case"
        );
        assert!(u.create("../evil").is_err());
        u.add_word("Fantasy", "Gandalf").unwrap();
        u.add_word("Fantasy", "Gandalf").unwrap();
        u.add_word("astronomy", "exoplanet").unwrap();
        assert_eq!(u.names(), [PERSONAL, "astronomy", "Fantasy"]);
        assert_eq!(u.read("Fantasy").words, ["Gandalf"], "no duplicates");
        let mut all = u.enabled_words();
        all.sort();
        assert_eq!(all, ["Bookx", "Gandalf", "exoplanet"]);
        u.set_enabled("Fantasy", false).unwrap();
        assert!(!u.read("Fantasy").enabled);
        assert_eq!(
            u.read("Fantasy").words,
            ["Gandalf"],
            "disabling keeps the words"
        );
        assert!(!u.enabled_words().contains(&"Gandalf".to_string()));
        assert!(u.remove_word("astronomy", "exoplanet").unwrap());
        assert!(!u.remove_word("astronomy", "exoplanet").unwrap());
        u.remove("astronomy").unwrap();
        assert!(u.remove(PERSONAL).is_err());
        assert_eq!(u.names(), [PERSONAL, "Fantasy"]);
        assert!(u.add_word(PERSONAL, "  ").is_err());
        assert!(u.add_word(PERSONAL, "# x").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn words_import_from_a_text_file() {
        let root = dir();
        let u = UserDicts::new(&root);
        let f = root.join("list.txt");
        std::fs::write(&f, "alpha\nbeta\n# note\nalpha\n").unwrap();
        u.add_word(PERSONAL, "beta").unwrap();
        assert_eq!(u.import_words(PERSONAL, &f).unwrap(), 1);
        assert_eq!(u.read(PERSONAL).words, ["alpha", "beta"]);
        assert!(u.import_words(PERSONAL, &root.join("none.txt")).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
