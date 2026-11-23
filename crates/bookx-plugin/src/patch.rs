//! User patches (roadmap X9): small TOML files in a per-user folder that tweak a book at fixed
//! points of its life with regular expressions and CSS rules. They follow the same rules as
//! WebAssembly plugins: **off until the user switches them on**, no file or network access (a patch
//! can only rewrite text files inside the book it is applied to), hard limits on everything, and a
//! dry-run that shows what a patch would change before it is allowed to.
//!
//! # File format
//!
//! ```toml
//! id = "straight-quotes"            # optional, default: the file name without .toml
//! name = "Straight quotes"
//! description = "Replaces curly double quotes by plain ones"
//! hook = "convert-before"           # import-after | convert-before | convert-after | open-before
//! files = ["*.xhtml", "*.html"]     # optional default for the rules; * and ? globs on book paths
//!
//! [[regex]]
//! find = '[“”]'                     # Rust regex syntax (linear time, no look-around)
//! replace = '"'                     # $1, ${name} refer to groups; $$ is a dollar sign
//! flags = "i"                       # optional: i (ignore case) s (dot matches newline) m (multi line)
//! files = ["OEBPS/*.xhtml"]         # optional, overrides the default above
//!
//! [[css]]
//! selector = "p"                    # appended to every matching style sheet as `p { ... }`
//! declarations = "text-indent: 0"
//! files = ["*.css"]                 # optional (default `*.css`)
//! ```
//!
//! Hooks: `import-after` (a book has just been converted to EPUB for the library), `convert-before`
//! (Convert To, before Bookx's own transforms), `convert-after` (after them, before the output is
//! written) and `open-before` (the reader opens the book; the stored file is never changed).
//!
//! Patches only touch `.xhtml .html .htm .css .svg .txt .ncx .xml` files, never `mimetype`,
//! `META-INF/` or the package file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use bookx_core::archive::Archive;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

pub const MAX_PATCH_BYTES: u64 = 64 * 1024;
pub const MAX_RULES: usize = 100;
pub const MAX_PATTERN_CHARS: usize = 2000;
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_REPLACEMENTS_PER_FILE: usize = 100_000;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
const EXCERPT: usize = 60;
const PATCH_EXTENSIONS: &[&str] = &["xhtml", "html", "htm", "css", "svg", "txt", "ncx", "xml"];

/// What a patch is allowed to do, as shown to the user.
pub const PERMISSIONS: &str = "Edits text and style sheets inside the book only. No access to other files, the network or programs.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Hook {
    ImportAfter,
    ConvertBefore,
    ConvertAfter,
    OpenBefore,
}

impl Hook {
    pub const ALL: [Hook; 4] = [
        Hook::ImportAfter,
        Hook::ConvertBefore,
        Hook::ConvertAfter,
        Hook::OpenBefore,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Hook::ImportAfter => "import-after",
            Hook::ConvertBefore => "convert-before",
            Hook::ConvertAfter => "convert-after",
            Hook::OpenBefore => "open-before",
        }
    }

    pub fn parse(s: &str) -> Result<Hook> {
        Hook::ALL
            .into_iter()
            .find(|h| h.as_str() == s)
            .ok_or_else(|| {
                anyhow!("unknown hook `{s}` (use import-after, convert-before, convert-after or open-before)")
            })
    }

    /// Name for menus.
    pub fn label(self) -> &'static str {
        match self {
            Hook::ImportAfter => "after importing",
            Hook::ConvertBefore => "before converting",
            Hook::ConvertAfter => "after converting",
            Hook::OpenBefore => "when opening",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPatch {
    id: Option<String>,
    name: Option<String>,
    #[serde(default)]
    description: String,
    hook: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    regex: Vec<RawRegex>,
    #[serde(default)]
    css: Vec<RawCss>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRegex {
    find: String,
    #[serde(default)]
    replace: String,
    #[serde(default)]
    flags: String,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCss {
    selector: String,
    declarations: String,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Rule {
    Regex {
        re: Regex,
        replace: String,
        files: Vec<String>,
    },
    Css {
        text: String,
        files: Vec<String>,
    },
}

/// A validated patch.
#[derive(Debug, Clone)]
pub struct Patch {
    pub id: String,
    pub name: String,
    pub description: String,
    pub hook: Hook,
    pub rules: Vec<Rule>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn valid_glob(g: &str) -> bool {
    !g.is_empty() && g.len() <= 200 && !g.contains("..") && !g.starts_with('/')
}

fn globs(own: Vec<String>, default: &[String], fallback: &[&str]) -> Result<Vec<String>> {
    let v: Vec<String> = if !own.is_empty() {
        own
    } else if !default.is_empty() {
        default.to_vec()
    } else {
        fallback.iter().map(|s| s.to_string()).collect()
    };
    for g in &v {
        if !valid_glob(g) {
            bail!("`{g}` is not a valid file pattern");
        }
    }
    Ok(v)
}

/// `*` (any run of characters), `?` (one character); the pattern is matched against the whole
/// path, or against the file name when it has no `/`. Case-insensitive.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    fn go(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => (0..=s.len()).any(|i| go(&p[1..], &s[i..])),
            Some('?') => !s.is_empty() && go(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && go(&p[1..], &s[1..]),
        }
    }
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let target = if pattern.contains('/') {
        path
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };
    let s: Vec<char> = target.to_lowercase().chars().collect();
    go(&p, &s)
}

fn editable(name: &str) -> bool {
    if name == "mimetype" || name.starts_with("META-INF/") || name.ends_with(".opf") {
        return false;
    }
    name.rsplit_once('.')
        .is_some_and(|(_, e)| PATCH_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

impl Patch {
    /// Parses and validates `text`; `default_id` is used when the file has no `id`.
    pub fn parse(text: &str, default_id: &str) -> Result<Patch> {
        if text.len() as u64 > MAX_PATCH_BYTES {
            bail!(
                "the patch file is larger than {} KiB",
                MAX_PATCH_BYTES / 1024
            );
        }
        let raw: RawPatch = toml::from_str(text).context("not a valid patch file")?;
        let id = raw.id.unwrap_or_else(|| default_id.to_string());
        if !valid_id(&id) {
            bail!("`{id}` is not a valid patch id (letters, digits, `.`, `-`, `_`)");
        }
        let hook = Hook::parse(&raw.hook)?;
        if raw.regex.len() + raw.css.len() > MAX_RULES {
            bail!("a patch can have at most {MAX_RULES} rules");
        }
        if raw.regex.is_empty() && raw.css.is_empty() {
            bail!("the patch has no [[regex]] or [[css]] rule");
        }
        let mut rules = Vec::new();
        for (i, r) in raw.regex.into_iter().enumerate() {
            if r.find.is_empty() || r.find.chars().count() > MAX_PATTERN_CHARS {
                bail!(
                    "regex rule {}: `find` must be 1..{MAX_PATTERN_CHARS} characters",
                    i + 1
                );
            }
            let mut b = RegexBuilder::new(&r.find);
            b.size_limit(REGEX_SIZE_LIMIT)
                .dfa_size_limit(REGEX_SIZE_LIMIT);
            for f in r.flags.chars() {
                match f {
                    'i' => b.case_insensitive(true),
                    's' => b.dot_matches_new_line(true),
                    'm' => b.multi_line(true),
                    other => bail!("regex rule {}: unknown flag `{other}` (use i, s, m)", i + 1),
                };
            }
            let re = b
                .build()
                .map_err(|e| anyhow!("regex rule {}: {e}", i + 1))?;
            rules.push(Rule::Regex {
                re,
                replace: r.replace,
                files: globs(r.files, &raw.files, &["*.xhtml", "*.html", "*.htm"])?,
            });
        }
        for (i, c) in raw.css.into_iter().enumerate() {
            let bad = |s: &str| s.contains('{') || s.contains('}') || s.contains("</");
            if c.selector.trim().is_empty() || bad(&c.selector) || bad(&c.declarations) {
                bail!(
                    "css rule {}: selector and declarations must be plain text without braces",
                    i + 1
                );
            }
            rules.push(Rule::Css {
                text: format!("\n{} {{ {} }}\n", c.selector.trim(), c.declarations.trim()),
                files: globs(c.files, &[], &["*.css"])?,
            });
        }
        Ok(Patch {
            name: raw
                .name
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| id.clone()),
            id,
            description: raw.description,
            hook,
            rules,
        })
    }

    /// Applies the patch to the book. Returns what changed (nothing when no file matched).
    pub fn apply(&self, archive: &mut Archive) -> PatchReport {
        let mut changes: Vec<FileChange> = Vec::new();
        let names: Vec<String> = archive.names().to_vec();
        for name in names.iter().filter(|n| editable(n)) {
            let Some(bytes) = archive.get(name) else {
                continue;
            };
            if bytes.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(original) = std::str::from_utf8(bytes) else {
                continue;
            };
            let mut text = original.to_string();
            let mut count = 0usize;
            for rule in &self.rules {
                match rule {
                    Rule::Regex { re, replace, files } => {
                        if !files.iter().any(|g| glob_match(g, name)) {
                            continue;
                        }
                        let n = re.find_iter(&text).take(MAX_REPLACEMENTS_PER_FILE).count();
                        if n > 0 {
                            text = re
                                .replacen(&text, MAX_REPLACEMENTS_PER_FILE, replace.as_str())
                                .into_owned();
                            count += n;
                        }
                    }
                    Rule::Css { text: css, files } => {
                        if files.iter().any(|g| glob_match(g, name)) && !text.contains(css.trim()) {
                            text.push_str(css);
                            count += 1;
                        }
                    }
                }
            }
            if text != original {
                let (before, after) = excerpts(original, &text);
                changes.push(FileChange {
                    file: name.clone(),
                    changes: count,
                    before,
                    after,
                });
                archive.set(name, text.into_bytes());
            }
        }
        PatchReport { files: changes }
    }

    /// What [`apply`](Self::apply) would do, without touching `archive`.
    pub fn preview(&self, archive: &Archive) -> PatchReport {
        let mut copy = archive.clone();
        self.apply(&mut copy)
    }
}

/// Short pieces of `a` and `b` around the first difference.
fn excerpts(a: &str, b: &str) -> (String, String) {
    let start = a
        .char_indices()
        .zip(b.chars())
        .find(|((_, x), y)| x != y)
        .map(|((i, _), _)| i)
        .unwrap_or_else(|| a.len().min(b.len()));
    let cut = |s: &str| -> String {
        let begin = s[..start.min(s.len())]
            .char_indices()
            .rev()
            .nth(EXCERPT / 3)
            .map(|(i, _)| i)
            .unwrap_or(0);
        s[begin..]
            .chars()
            .take(EXCERPT)
            .collect::<String>()
            .replace('\n', " ")
    };
    (cut(a), cut(b))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub file: String,
    /// Replacements (regex) or appended rules (css) in this file.
    pub changes: usize,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchReport {
    pub files: Vec<FileChange>,
}

impl PatchReport {
    pub fn total_changes(&self) -> usize {
        self.files.iter().map(|f| f.changes).sum()
    }

    /// One line per file for a preview dialog.
    pub fn summary(&self) -> String {
        if self.files.is_empty() {
            return "No changes: nothing in this book matches the patch.".into();
        }
        let mut s = format!(
            "{} change{} in {} file{}",
            self.total_changes(),
            if self.total_changes() == 1 { "" } else { "s" },
            self.files.len(),
            if self.files.len() == 1 { "" } else { "s" }
        );
        for f in self.files.iter().take(8) {
            s.push_str(&format!(
                "\n{}: {} → {}",
                f.file,
                f.before.trim(),
                f.after.trim()
            ));
        }
        s
    }
}

// ---- the folder ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    enabled: BTreeSet<String>,
    /// Application order; patches not listed come after, by file name.
    #[serde(default)]
    order: Vec<String>,
}

struct Entry {
    file: PathBuf,
    id: String,
    patch: std::result::Result<Patch, String>,
}

/// One row of the manager UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub hook: Option<Hook>,
    pub file: PathBuf,
    pub enabled: bool,
    pub rules: usize,
    pub error: Option<String>,
}

/// The enabled patches in order, detached from the folder (cheap to share between threads).
#[derive(Debug, Clone, Default)]
pub struct PatchSet {
    patches: Vec<Patch>,
}

impl PatchSet {
    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }

    pub fn has_hook(&self, hook: Hook) -> bool {
        self.patches.iter().any(|p| p.hook == hook)
    }

    pub fn len(&self) -> usize {
        self.patches.len()
    }

    /// Runs the patches of `hook` in order. Returns `(patch id, report)` for those that changed
    /// something.
    pub fn apply(&self, hook: Hook, archive: &mut Archive) -> Vec<(String, PatchReport)> {
        self.patches
            .iter()
            .filter(|p| p.hook == hook)
            .filter_map(|p| {
                let r = p.apply(archive);
                (!r.files.is_empty()).then(|| (p.id.clone(), r))
            })
            .collect()
    }
}

pub struct PatchManager {
    dir: PathBuf,
    state_path: PathBuf,
    entries: Vec<Entry>,
    state: State,
    state_error: Option<String>,
}

impl PatchManager {
    /// Reads the folder and the state file; a missing folder just means no patches.
    pub fn new(dir: impl Into<PathBuf>, state_path: impl Into<PathBuf>) -> PatchManager {
        let mut m = PatchManager {
            dir: dir.into(),
            state_path: state_path.into(),
            entries: Vec::new(),
            state: State::default(),
            state_error: None,
        };
        match std::fs::read_to_string(&m.state_path) {
            Ok(t) => match serde_json::from_str(&t) {
                Ok(s) => m.state = s,
                Err(e) => m.state_error = Some(format!("patch settings unreadable: {e}")),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => m.state_error = Some(format!("patch settings unreadable: {e}")),
        }
        m.scan();
        m
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn state_error(&self) -> Option<&str> {
        self.state_error.as_deref()
    }

    /// Re-reads the `.toml` files of the folder.
    pub fn scan(&mut self) {
        self.entries.clear();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.extension().is_some_and(|e| e == "toml")
                            && !p
                                .file_name()
                                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let mut seen = BTreeSet::new();
        for file in files {
            let stem = file
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parsed = std::fs::metadata(&file)
                .map_err(|e| e.to_string())
                .and_then(|m| {
                    if m.len() > MAX_PATCH_BYTES {
                        Err(format!("larger than {} KiB", MAX_PATCH_BYTES / 1024))
                    } else {
                        std::fs::read_to_string(&file).map_err(|e| e.to_string())
                    }
                })
                .and_then(|t| Patch::parse(&t, &stem).map_err(|e| format!("{e:#}")));
            let (id, patch) = match parsed {
                Ok(p) if !seen.insert(p.id.clone()) => (
                    p.id.clone(),
                    Err(format!("another patch already uses the id `{}`", p.id)),
                ),
                Ok(p) => (p.id.clone(), Ok(p)),
                Err(e) => (stem, Err(e)),
            };
            self.entries.push(Entry { file, id, patch });
        }
    }

    fn ordered_indices(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.entries.len()).collect();
        let pos = |i: usize| {
            self.state
                .order
                .iter()
                .position(|o| *o == self.entries[i].id)
                .unwrap_or(usize::MAX)
        };
        idx.sort_by_key(|&i| (pos(i), i));
        idx
    }

    pub fn list(&self) -> Vec<PatchStatus> {
        self.ordered_indices()
            .into_iter()
            .map(|i| {
                let e = &self.entries[i];
                let enabled = self.state.enabled.contains(&e.id);
                match &e.patch {
                    Ok(p) => PatchStatus {
                        id: e.id.clone(),
                        name: p.name.clone(),
                        description: p.description.clone(),
                        hook: Some(p.hook),
                        file: e.file.clone(),
                        enabled,
                        rules: p.rules.len(),
                        error: None,
                    },
                    Err(err) => PatchStatus {
                        id: e.id.clone(),
                        name: e.id.clone(),
                        description: String::new(),
                        hook: None,
                        file: e.file.clone(),
                        enabled: false,
                        rules: 0,
                        error: Some(err.clone()),
                    },
                }
            })
            .collect()
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.state.enabled.contains(id)
    }

    fn entry(&self, id: &str) -> Result<&Entry> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("no patch `{id}`"))
    }

    fn save(&self) -> Result<()> {
        if let Some(d) = self.state_path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.state_path, serde_json::to_string_pretty(&self.state)?)
            .with_context(|| format!("writing {}", self.state_path.display()))
    }

    /// Switches a patch on or off (a broken patch cannot be switched on).
    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        let e = self.entry(id)?;
        if on {
            if let Err(err) = &e.patch {
                bail!("patch `{id}` has a problem: {err}");
            }
            self.state.enabled.insert(id.to_string());
        } else {
            self.state.enabled.remove(id);
        }
        self.save()
    }

    /// Moves a patch `delta` places in the application order (negative = earlier).
    pub fn move_by(&mut self, id: &str, delta: isize) -> Result<()> {
        self.entry(id)?;
        let mut order: Vec<String> = self
            .ordered_indices()
            .into_iter()
            .map(|i| self.entries[i].id.clone())
            .collect();
        let from = order.iter().position(|o| o == id).unwrap_or(0);
        let to = (from as isize + delta).clamp(0, order.len() as isize - 1) as usize;
        let item = order.remove(from);
        order.insert(to, item);
        self.state.order = order;
        self.save()
    }

    /// The enabled patches in order.
    pub fn enabled_set(&self) -> PatchSet {
        PatchSet {
            patches: self
                .ordered_indices()
                .into_iter()
                .filter_map(|i| {
                    let e = &self.entries[i];
                    match &e.patch {
                        Ok(p) if self.state.enabled.contains(&e.id) => Some(p.clone()),
                        _ => None,
                    }
                })
                .collect(),
        }
    }

    /// Dry run of one patch (enabled or not) on `archive`.
    pub fn preview(&self, id: &str, archive: &Archive) -> Result<PatchReport> {
        match &self.entry(id)?.patch {
            Ok(p) => Ok(p.preview(archive)),
            Err(e) => bail!("patch `{id}` has a problem: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::convert::{Chapter, create_epub};
    use bookx_core::opf::Metadata;

    fn book() -> Archive {
        create_epub(
            &Metadata {
                title: "T".into(),
                authors: vec!["A".into()],
                language: "en".into(),
                ..Default::default()
            },
            &[Chapter {
                title: "One".into(),
                body: "<p>He said “hi” and “bye”.</p>".into(),
            }],
            None,
        )
    }

    fn chapter(a: &Archive) -> String {
        let n = a
            .names()
            .iter()
            .find(|n| n.ends_with(".xhtml") && !n.contains("nav"))
            .unwrap()
            .clone();
        a.get_string(&n).unwrap()
    }

    const QUOTES: &str = r#"
name = "Straight quotes"
hook = "convert-before"
[[regex]]
find = '[“”]'
replace = '"'
"#;

    #[test]
    fn a_regex_patch_rewrites_text_files() {
        let p = Patch::parse(QUOTES, "quotes").unwrap();
        assert_eq!(
            (p.id.as_str(), p.name.as_str()),
            ("quotes", "Straight quotes")
        );
        let mut a = book();
        let r = p.apply(&mut a);
        assert_eq!(r.total_changes(), 4);
        assert!(chapter(&a).contains("He said \"hi\" and \"bye\"."));
        assert!(r.summary().contains("4 changes in 1 file"));
        assert!(p.apply(&mut a).files.is_empty(), "idempotent");
    }

    #[test]
    fn groups_flags_and_per_rule_files() {
        let p = Patch::parse(
            r#"
hook = "open-before"
[[regex]]
find = '(?P<w>HI)'
replace = '<b>${w}</b>'
flags = "i"
"#,
            "x",
        )
        .unwrap();
        let mut a = book();
        p.apply(&mut a);
        assert!(chapter(&a).contains("<b>hi</b>"));
        let none = Patch::parse(
            "hook=\"open-before\"\n[[regex]]\nfind='hi'\nreplace='X'\nfiles=['*.css']",
            "x",
        )
        .unwrap();
        assert!(none.apply(&mut book()).files.is_empty());
    }

    #[test]
    fn css_rules_are_appended_once() {
        let p = Patch::parse(
            "hook=\"convert-after\"\n[[css]]\nselector='p'\ndeclarations='text-indent: 0'\n",
            "css",
        )
        .unwrap();
        let mut a = book();
        let r = p.apply(&mut a);
        assert!(!r.files.is_empty());
        let css = r.files[0].file.clone();
        assert!(a.get_string(&css).unwrap().contains("p { text-indent: 0 }"));
        assert!(p.apply(&mut a).files.is_empty());
    }

    #[test]
    fn protected_files_are_never_touched() {
        let p = Patch::parse(
            "hook=\"open-before\"\n[[regex]]\nfind='.+'\nreplace='x'\nfiles=['*']",
            "all",
        )
        .unwrap();
        let mut a = book();
        let mime = a.get("mimetype").unwrap().to_vec();
        let container = a.get("META-INF/container.xml").unwrap().to_vec();
        p.apply(&mut a);
        assert_eq!(a.get("mimetype").unwrap(), mime);
        assert_eq!(a.get("META-INF/container.xml").unwrap(), container);
        assert!(bookx_core::book::OpenBook::from_archive(a).is_ok());
    }

    #[test]
    fn bad_patches_are_refused_with_a_reason() {
        let e = |t: &str| Patch::parse(t, "x").unwrap_err().to_string();
        assert!(
            e("hook=\"later\"\n[[css]]\nselector='p'\ndeclarations='a:b'").contains("unknown hook")
        );
        assert!(e("hook=\"open-before\"").contains("no [[regex]]"));
        assert!(e("hook=\"open-before\"\n[[regex]]\nfind='('").contains("regex rule 1"));
        assert!(
            e("hook=\"open-before\"\nid=\"a b\"\n[[regex]]\nfind='a'").contains("valid patch id")
        );
        assert!(e("hook=\"open-before\"\n[[regex]]\nfind='a'\nflags='x'").contains("unknown flag"));
        assert!(
            e("hook=\"open-before\"\n[[regex]]\nfind='a'\nfiles=['../x']").contains("file pattern")
        );
        assert!(
            e("hook=\"open-before\"\n[[css]]\nselector='p {'\ndeclarations='a:b'")
                .contains("braces")
        );
        assert!(
            e("hook=\"open-before\"\nrun=\"rm\"\n[[regex]]\nfind='a'")
                .contains("not a valid patch")
        );
        assert!(Patch::parse(&"#".repeat(70_000), "x").is_err());
        let huge = format!(
            "hook=\"open-before\"\n[[regex]]\nfind='{}'",
            "a".repeat(3000)
        );
        assert!(e(&huge).contains("find"));
    }

    #[test]
    fn a_pathological_regex_is_rejected_or_fast() {
        // nested repetition blows past the compiled size limit rather than the memory
        let t = "hook=\"open-before\"\n[[regex]]\nfind='((((a{50}){50}){50}){50})'\n";
        assert!(Patch::parse(t, "x").is_err());
        // classic exponential pattern is linear time in the regex crate
        let p = Patch::parse(
            "hook=\"open-before\"\n[[regex]]\nfind='(a+)+$'\nreplace='b'\nfiles=['*.txt']",
            "x",
        )
        .unwrap();
        let mut a = Archive::default();
        a.set("x.txt", format!("{}!", "a".repeat(100_000)).into_bytes());
        let t0 = std::time::Instant::now();
        p.apply(&mut a);
        assert!(t0.elapsed().as_secs() < 5);
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.xhtml", "OEBPS/ch1.xhtml"));
        assert!(glob_match("OEBPS/*.XHTML", "OEBPS/ch1.xhtml"));
        assert!(glob_match("ch?.xhtml", "a/ch1.xhtml"));
        assert!(!glob_match("OEBPS/*.css", "other/a.css"));
        assert!(!glob_match("*.css", "a.xhtml"));
    }

    #[test]
    fn preview_does_not_change_the_book() {
        let p = Patch::parse(QUOTES, "q").unwrap();
        let a = book();
        let before = chapter(&a);
        let r = p.preview(&a);
        assert_eq!(chapter(&a), before);
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].before.contains('“') && r.files[0].after.contains('"'));
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "bookx-patch-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_folder_lists_orders_and_remembers() {
        let d = tmp("mgr");
        std::fs::write(d.join("a-quotes.toml"), QUOTES).unwrap();
        std::fs::write(
            d.join("b-wrap.toml"),
            "hook=\"convert-before\"\n[[regex]]\nfind='said'\nreplace='SAID'\n",
        )
        .unwrap();
        std::fs::write(d.join("c-broken.toml"), "hook = ").unwrap();
        std::fs::write(d.join("notes.txt"), "ignored").unwrap();
        let state = d.join("state/patches.json");
        let mut m = PatchManager::new(&d, &state);
        let ids = |m: &PatchManager| m.list().into_iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids(&m), ["a-quotes", "b-wrap", "c-broken"]);
        assert!(m.list().iter().all(|p| !p.enabled), "off by default");
        assert!(m.enabled_set().is_empty());
        assert!(m.set_enabled("c-broken", true).is_err());
        assert!(m.set_enabled("nope", true).is_err());
        m.set_enabled("a-quotes", true).unwrap();
        m.set_enabled("b-wrap", true).unwrap();
        m.move_by("b-wrap", -1).unwrap();
        assert_eq!(ids(&m), ["b-wrap", "a-quotes", "c-broken"]);
        m.move_by("b-wrap", -5).unwrap();
        assert_eq!(ids(&m)[0], "b-wrap");
        let set = m.enabled_set();
        assert_eq!(set.len(), 2);
        assert!(set.has_hook(Hook::ConvertBefore) && !set.has_hook(Hook::OpenBefore));
        let mut a = book();
        let reports = set.apply(Hook::ConvertBefore, &mut a);
        assert_eq!(
            reports.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
            ["b-wrap", "a-quotes"]
        );
        assert!(chapter(&a).contains("He SAID \"hi\""));

        let m2 = PatchManager::new(&d, &state);
        assert_eq!(ids(&m2), ["b-wrap", "a-quotes", "c-broken"]);
        assert!(m2.is_enabled("a-quotes") && m2.is_enabled("b-wrap"));
        let mut m2 = m2;
        m2.set_enabled("a-quotes", false).unwrap();
        assert_eq!(m2.enabled_set().len(), 1);
        assert!(m2.preview("a-quotes", &book()).unwrap().total_changes() > 0);
        assert!(m2.preview("c-broken", &book()).is_err());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn duplicate_ids_and_a_missing_folder() {
        let d = tmp("dup");
        std::fs::write(d.join("one.toml"), format!("id=\"same\"\n{QUOTES}")).unwrap();
        std::fs::write(d.join("two.toml"), format!("id=\"same\"\n{QUOTES}")).unwrap();
        let m = PatchManager::new(&d, d.join("s.json"));
        let l = m.list();
        assert_eq!(l.len(), 2);
        assert_eq!(l.iter().filter(|p| p.error.is_some()).count(), 1);
        let none = PatchManager::new(d.join("missing"), d.join("s2.json"));
        assert!(none.list().is_empty() && none.state_error().is_none());
        std::fs::write(d.join("bad.json"), "{{{").unwrap();
        assert!(
            PatchManager::new(&d, d.join("bad.json"))
                .state_error()
                .is_some()
        );
        let _ = std::fs::remove_dir_all(d);
    }
}
