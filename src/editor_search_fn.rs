//! Function mode of the editor's search & replace (CE-100..CE-113): a user script decides, for every match of
//! a regular expression, what it is replaced with. Calibre's functions are Python; Bookx's are written in
//! Rhai (a pure Rust scripting language), run in a sandbox, and have the same shape:
//!
//! ```rhai
//! fn replace(m, number, file, meta, data) {
//!     m.group(1).to_upper()      // returned text replaces the match; () keeps the match
//! }
//! ```
//!
//! * `m`: the match: `m.group(n)`, `m.group("name")`, `m.text`, `m.start`, `m.end` (bytes), `m.groups`;
//!   it is `()` in the extra call after the last match (see `call_after_last_match`);
//! * `number`: 1, 2, 3 … counted across the whole run, files included;
//! * `file`: path of the file inside the book (`""` for marked text);
//! * `meta`: map with `title`, `authors` (array), `language`, `publisher`, `series`, `series_index`, `tags`,
//!   `identifiers` (map), `description`, `rights`;
//! * `data`: a store that lives for one run: `data["total"] = 5;` / `data["total"]` (`()` when unset).
//!
//! Helpers: `print(x)` (collected in the debug output), `dict_recognized(word)` (the book's spelling
//! dictionary), `text_upper/text_lower/text_capitalize/text_swap_case/text_title(s)` (tags are left alone),
//! `smarten_hyphens(s)`, `strip_tags(s)`, `escape_html(s)`.
//!
//! * `functions` (optional sixth parameter): the other saved functions, `functions["name"].invoke(m, number,
//!   file, meta, data)` runs one of them and returns what it returned; they share `data`, and a called
//!   function cannot call further ones (CE-107).
//!
//! Optional header comments in the script set the options of Calibre's `replace.*` attributes:
//! `// @file_order spine` (or `spine-reverse`), `// @call_after_last_match`, `// @suppress_result_dialog`,
//! `// @append_final_output_to_marked` (the final output is added after the marked text, CE-110) and
//! `// @persistent_data` (`data` is kept per library between runs, CE-107).
//!
//! Sandbox: Rhai has no file or network access of its own and nothing here adds any; modules, `eval` and
//! custom syntax are off. Each call may run at most [`Limits::max_ops`] operations, a whole run at most
//! [`Limits::max_time`]; strings, arrays and maps have size limits, call depth and expression depth are
//! limited. No GTK in here.

use crate::i18n::ntr;
use crate::i18n::trf;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map, Scope};
use serde::{Deserialize, Serialize};

use crate::editor_search::{MatchInfo, Pattern};

// ---------------------------------------------------------------------------
// Saved functions
// ---------------------------------------------------------------------------

/// In which order Replace All visits the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FileOrder {
    #[default]
    Normal,
    Spine,
    SpineReverse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source: String,
    /// The regular expression this function is meant for (put in the Find box when the function is chosen).
    #[serde(default)]
    pub find: String,
    #[serde(default)]
    pub file_order: FileOrder,
    #[serde(default)]
    pub call_after_last_match: bool,
    #[serde(default)]
    pub suppress_result_dialog: bool,
    /// The final output is appended after the marked text of the open file (CE-110).
    #[serde(default)]
    pub append_final_output_to_marked: bool,
    /// `data` keeps its content between runs (per library).
    #[serde(default)]
    pub persistent_data: bool,
    #[serde(default)]
    pub builtin: bool,
}

impl Function {
    pub fn new(name: &str, find: &str, source: &str) -> Function {
        let mut f = Function {
            name: name.into(),
            description: String::new(),
            source: source.into(),
            find: find.into(),
            file_order: FileOrder::Normal,
            call_after_last_match: false,
            suppress_result_dialog: false,
            append_final_output_to_marked: false,
            persistent_data: false,
            builtin: false,
        };
        f.read_header();
        f
    }

    /// Reads `// @file_order …`, `// @call_after_last_match`, `// @suppress_result_dialog` from the script.
    pub fn read_header(&mut self) {
        let mut order = FileOrder::Normal;
        let (mut after, mut quiet) = (false, false);
        let (mut append, mut persistent) = (false, false);
        for line in self.source.lines() {
            let Some(rest) = line.trim().strip_prefix("//") else {
                if line.trim().is_empty() {
                    continue;
                }
                break;
            };
            let rest = rest.trim();
            if let Some(v) = rest.strip_prefix("@file_order") {
                order = match v.trim() {
                    "spine" => FileOrder::Spine,
                    "spine-reverse" => FileOrder::SpineReverse,
                    _ => FileOrder::Normal,
                };
            } else if rest.starts_with("@call_after_last_match") {
                after = true;
            } else if rest.starts_with("@suppress_result_dialog") {
                quiet = true;
            } else if rest.starts_with("@append_final_output_to_marked") {
                append = true;
            } else if rest.starts_with("@persistent_data") {
                persistent = true;
            }
        }
        self.file_order = order;
        self.call_after_last_match = after;
        self.suppress_result_dialog = quiet;
        self.append_final_output_to_marked = append;
        self.persistent_data = persistent;
    }
}

/// Inserts `output` on a new line after the last marked range (byte offsets) of `text`; returns the
/// new text and the range of the inserted output, which becomes marked too (CE-110). `None` when
/// nothing is marked.
pub fn append_after_marked(
    text: &str,
    marked: &[(usize, usize)],
    output: &str,
) -> Option<(String, (usize, usize))> {
    let end = marked.iter().map(|m| m.1).max()?;
    if end > text.len() || !text.is_char_boundary(end) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + output.len() + 1);
    out.push_str(&text[..end]);
    out.push('\n');
    let start = out.len();
    out.push_str(output);
    let stop = out.len();
    out.push_str(&text[end..]);
    Some((out, (start, stop)))
}

// ---- data kept between runs (CE-107) ----

fn to_json_value(d: &Dynamic) -> Option<serde_json::Value> {
    use serde_json::Value;
    if d.is_unit() {
        return Some(Value::Null);
    }
    if let Ok(b) = d.as_bool() {
        return Some(Value::Bool(b));
    }
    if let Ok(i) = d.as_int() {
        return Some(Value::from(i));
    }
    if let Ok(f) = d.as_float() {
        return serde_json::Number::from_f64(f).map(Value::Number);
    }
    if d.is_string() {
        return Some(Value::String(d.clone().into_string().ok()?));
    }
    if let Some(a) = d.clone().try_cast::<Array>() {
        return Some(Value::Array(a.iter().filter_map(to_json_value).collect()));
    }
    if let Some(m) = d.clone().try_cast::<Map>() {
        return Some(Value::Object(
            m.iter()
                .filter_map(|(k, v)| to_json_value(v).map(|v| (k.to_string(), v)))
                .collect(),
        ));
    }
    None
}

fn from_json_value(v: &serde_json::Value) -> Dynamic {
    use serde_json::Value;
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().unwrap_or(0.0).into(),
        },
        Value::String(s) => s.clone().into(),
        Value::Array(a) => a.iter().map(from_json_value).collect::<Array>().into(),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| (k.as_str().into(), from_json_value(v)))
            .collect::<Map>()
            .into(),
    }
}

/// The file that keeps the data of function `name` in `library` (a library key such as its folder),
/// inside `dir`. Names are hashed so nothing in them can leave the folder.
pub fn data_file(dir: &Path, library: &str, name: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    library.hash(&mut h);
    let lib = h.finish();
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();
    dir.join(format!("{lib:016x}-{safe}.json"))
}

pub fn load_data(path: &Path) -> Map {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| from_json_value(&v))
        .and_then(|d| d.try_cast::<Map>())
        .unwrap_or_default()
}

pub fn save_data(path: &Path, data: &Map) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let v = to_json_value(&Dynamic::from(data.clone())).unwrap_or(serde_json::Value::Null);
    std::fs::write(path, serde_json::to_string(&v).unwrap_or_default())
}

/// Where the data of persistent functions lives.
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("search-function-data"))
}

/// Orders `files` for a run: `Spine` visits the spine documents first in reading order, then the rest.
pub fn order_files(files: &[String], spine: &[String], order: FileOrder) -> Vec<String> {
    if order == FileOrder::Normal {
        return files.to_vec();
    }
    let mut v: Vec<String> = spine
        .iter()
        .filter(|s| files.contains(s))
        .cloned()
        .collect();
    let rest: Vec<String> = files.iter().filter(|f| !v.contains(f)).cloned().collect();
    v.extend(rest);
    if order == FileOrder::SpineReverse {
        v.reverse();
    }
    v
}

/// `<config>/Bookx/search-functions.json`.
pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("search-functions.json"))
}

pub fn load_from(path: &Path) -> Vec<Function> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| parse_json(&t).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, fns: &[Function]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let user: Vec<&Function> = fns.iter().filter(|f| !f.builtin).collect();
    std::fs::write(
        path,
        serde_json::to_string_pretty(&user).unwrap_or_default(),
    )
}

pub fn parse_json(text: &str) -> Result<Vec<Function>, String> {
    let mut v: Vec<Function> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    for f in &mut v {
        f.builtin = false;
        f.read_header();
    }
    Ok(v)
}

pub fn to_json(fns: &[Function]) -> String {
    let user: Vec<&Function> = fns.iter().filter(|f| !f.builtin).collect();
    serde_json::to_string_pretty(&user).unwrap_or_default()
}

/// Built-in functions followed by the user's, a user function replacing a built-in of the same name.
pub fn merged(user: &[Function]) -> Vec<Function> {
    let mut all: Vec<Function> = builtins()
        .into_iter()
        .filter(|b| !user.iter().any(|u| u.name == b.name))
        .collect();
    all.extend(user.iter().cloned().map(|mut f| {
        f.builtin = false;
        f
    }));
    all
}

fn builtin(name: &str, description: &str, find: &str, source: &str) -> Function {
    let mut f = Function::new(name, find, source);
    f.description = description.into();
    f.builtin = true;
    f
}

/// The functions that ship with Bookx; they are also the worked examples of the manual.
pub fn builtins() -> Vec<Function> {
    vec![
        builtin(
            "Title-case text (ignoring tags)",
            "Capitalises each word of the matched text (every heading by default); small words stay lower case; tags are left alone.",
            r"(?s)<h[1-6][^>]*>.*?</h[1-6]>",
            "fn replace(m, number, file, meta, data) {\n    text_title(m.text)\n}\n",
        ),
        builtin(
            "Upper-case text (ignoring tags)",
            "Upper-cases the matched text but not the tags inside it.",
            r"(?s)<h[1-6][^>]*>.*?</h[1-6]>",
            "fn replace(m, number, file, meta, data) {\n    text_upper(m.text)\n}\n",
        ),
        builtin(
            "Lower-case text (ignoring tags)",
            "Lower-cases the matched text but not the tags inside it.",
            r"(?s)<h[1-6][^>]*>.*?</h[1-6]>",
            "fn replace(m, number, file, meta, data) {\n    text_lower(m.text)\n}\n",
        ),
        builtin(
            "Capitalize text (ignoring tags)",
            "Upper-cases the first letter of the matched text, lower-cases the rest.",
            r"(?s)<h[1-6][^>]*>.*?</h[1-6]>",
            "fn replace(m, number, file, meta, data) {\n    text_capitalize(m.text)\n}\n",
        ),
        builtin(
            "Swap the case of text (ignoring tags)",
            "Upper case becomes lower case and the other way round.",
            r"(?s)(?<=>)[^<]+(?=<)",
            "fn replace(m, number, file, meta, data) {\n    text_swap_case(m.text)\n}\n",
        ),
        builtin(
            "Smarten hyphens and dashes",
            "--- becomes an em dash, -- an en dash, and a lone hyphen between spaces an en dash.",
            r"(?s)(?<=>)[^<]+(?=<)",
            "fn replace(m, number, file, meta, data) {\n    smarten_hyphens(m.text)\n}\n",
        ),
        builtin(
            "De-hyphenate with the dictionary",
            "Joins words split over a line (“exam-\\nple”) when the joined word is in the spelling dictionary.",
            r"(\w+)-\s*\n\s*(\w+)",
            "fn replace(m, number, file, meta, data) {\n    let joined = m.group(1) + m.group(2);\n    if dict_recognized(joined) {\n        joined\n    } else {\n        print(\"kept: \" + m.text);\n        m.text\n    }\n}\n",
        ),
        builtin(
            "Auto-number sections",
            "Puts “1.”, “2.” … in front of every <h2> heading, counting across the whole book.",
            r"(<h2[^>]*>)(.*?)(</h2>)",
            "fn replace(m, number, file, meta, data) {\n    m.group(1) + number + \". \" + m.group(2) + m.group(3)\n}\n",
        ),
        builtin(
            "Generate a table of contents from headings",
            "Collects every <h2> heading; the final call returns a list of links you can paste into a contents page. Change nothing in the text.",
            r"<h2[^>]*\bid=\x22([^\x22]+)\x22[^>]*>(.*?)</h2>",
            "// @call_after_last_match\nfn replace(m, number, file, meta, data) {\n    if m == () {\n        return \"<ol>\\n\" + data[\"toc\"] + \"</ol>\";\n    }\n    if data[\"toc\"] == () { data[\"toc\"] = \"\"; }\n    data[\"toc\"] += \"<li><a href=\\\"\" + file + \"#\" + m.group(1) + \"\\\">\" + strip_tags(m.group(2)) + \"</a></li>\\n\";\n    ()\n}\n",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Pure text helpers (also registered in the engine)
// ---------------------------------------------------------------------------

/// Applies `f` to the text outside `<…>` tags and keeps the tags.
pub fn map_outside_tags(s: &str, f: &dyn Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        match rest.find('<') {
            Some(i) => {
                out.push_str(&f(&rest[..i]));
                let end = rest[i..].find('>').map(|e| i + e + 1).unwrap_or(rest.len());
                out.push_str(&rest[i..end]);
                rest = &rest[end..];
            }
            None => {
                out.push_str(&f(rest));
                break;
            }
        }
    }
    out
}

const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "en", "for", "if", "in", "nor", "of", "on", "or",
    "per", "the", "to", "via", "vs", "up",
];

fn cap_word(w: &str) -> String {
    let mut it = w.chars();
    match it.next() {
        Some(c) => c.to_uppercase().collect::<String>() + it.as_str().to_lowercase().as_str(),
        None => String::new(),
    }
}

fn title_run(text: &str, first_run: bool, last_run: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let words: Vec<(usize, &str)> = {
        let mut v = Vec::new();
        let mut start = None;
        for (i, c) in text.char_indices() {
            let inside = c.is_alphanumeric() || c == '\'' || c == '’';
            match (start, inside) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    v.push((s, &text[s..i]));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            v.push((s, &text[s..]));
        }
        v
    };
    let mut last = 0;
    for (n, (i, w)) in words.iter().enumerate() {
        out.push_str(&text[last..*i]);
        let letters = w.chars().filter(|c| c.is_alphabetic()).count();
        let acronym = letters <= 4 && letters > 1 && w.chars().all(|c| !c.is_lowercase());
        let camel =
            w.chars().skip(1).any(|c| c.is_uppercase()) && w.chars().any(|c| c.is_lowercase());
        let inner_caps = acronym || camel;
        let small = SMALL_WORDS.contains(&w.to_lowercase().as_str());
        let is_first = n == 0 && first_run;
        let is_last = n + 1 == words.len() && last_run;
        if inner_caps {
            out.push_str(w);
        } else if small && !is_first && !is_last {
            out.push_str(&w.to_lowercase());
        } else {
            out.push_str(&cap_word(w));
        }
        last = i + w.len();
    }
    out.push_str(&text[last..]);
    out
}

/// "the war of the worlds" → "The War of the Worlds"; words with capitals inside (iPhone, NASA) stay.
/// Tags are left alone.
pub fn title_case(s: &str) -> String {
    let runs = Cell::new(0usize);
    map_outside_tags(s, &|t| {
        if t.chars().any(char::is_alphanumeric) {
            runs.set(runs.get() + 1);
        }
        t.to_string()
    });
    let total = runs.get();
    let seen = Cell::new(0usize);
    map_outside_tags(s, &|t| {
        if !t.chars().any(char::is_alphanumeric) {
            return t.to_string();
        }
        let n = seen.get();
        seen.set(n + 1);
        title_run(t, n == 0, n + 1 == total)
    })
}

pub fn upper_text(s: &str) -> String {
    map_outside_tags(s, &|t| t.to_uppercase())
}

pub fn lower_text(s: &str) -> String {
    map_outside_tags(s, &|t| t.to_lowercase())
}

/// First letter upper case, the rest lower case (of the first text run).
pub fn capitalize_text(s: &str) -> String {
    let done = Cell::new(false);
    map_outside_tags(s, &|t| {
        if !done.get() && t.chars().any(char::is_alphanumeric) {
            done.set(true);
            let mut it = t.chars();
            let mut out = String::new();
            let mut started = false;
            for c in it.by_ref() {
                if !started && c.is_alphanumeric() {
                    out.extend(c.to_uppercase());
                    started = true;
                } else if started {
                    out.extend(c.to_lowercase());
                } else {
                    out.push(c);
                }
            }
            out
        } else if done.get() {
            t.to_lowercase()
        } else {
            t.to_string()
        }
    })
}

pub fn swap_case_text(s: &str) -> String {
    map_outside_tags(s, &|t| {
        t.chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<Vec<_>>()
                } else if c.is_lowercase() {
                    c.to_uppercase().collect::<Vec<_>>()
                } else {
                    vec![c]
                }
            })
            .collect()
    })
}

/// `---` to an em dash, `--` to an en dash, ` - ` to a spaced en dash. Tags are left alone.
pub fn smarten_hyphens(s: &str) -> String {
    map_outside_tags(s, &|t| {
        t.replace("---", "\u{2014}")
            .replace("--", "\u{2013}")
            .replace(" - ", " \u{2013} ")
    })
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match (inside, c) {
            (false, '<') => inside = true,
            (true, '>') => inside = false,
            (false, c) => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn escape_html(s: &str) -> String {
    bookx_editing::search_ignore::escape_text(s).replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------

/// Errors of compiling or running a function, with the script line when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnError {
    pub line: Option<usize>,
    pub message: String,
}

impl std::fmt::Display for FnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Limits of the sandbox.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Operations of one call of `replace`.
    pub max_ops: u64,
    /// Wall-clock time of one run (all files).
    pub max_time: Duration,
    pub max_string: usize,
    pub max_array: usize,
    pub max_map: usize,
    pub max_output_lines: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_ops: 500_000,
            max_time: Duration::from_secs(30),
            max_string: 4 * 1024 * 1024,
            max_array: 100_000,
            max_map: 10_000,
            max_output_lines: 2_000,
        }
    }
}

/// The book facts a function can read.
#[derive(Clone, Default)]
pub struct Env {
    pub title: String,
    pub authors: Vec<String>,
    pub language: String,
    pub publisher: String,
    pub series: String,
    pub series_index: f64,
    pub tags: Vec<String>,
    pub identifiers: Vec<(String, String)>,
    pub description: String,
    pub rights: String,
    /// `dict_recognized`.
    pub dict: Option<Rc<dyn Fn(&str) -> bool>>,
    /// The saved functions a function can call through `functions` (CE-107).
    pub functions: Vec<Function>,
}

impl Env {
    pub fn from_metadata(m: &crate::core::opf::Metadata) -> Env {
        Env {
            title: m.title.clone(),
            authors: m.authors.clone(),
            language: m.language.clone(),
            publisher: m.publisher.clone(),
            series: m.series.clone(),
            series_index: m.series_index,
            tags: m.tags.clone(),
            identifiers: m
                .identifiers
                .iter()
                .map(|i| (i.scheme.clone(), i.value.clone()))
                .collect(),
            description: m.description.clone(),
            rights: m.rights.clone(),
            dict: None,
            functions: Vec::new(),
        }
    }

    fn to_map(&self) -> Map {
        let mut m = Map::new();
        m.insert("title".into(), self.title.clone().into());
        m.insert(
            "authors".into(),
            self.authors
                .iter()
                .map(|a| Dynamic::from(a.clone()))
                .collect::<Array>()
                .into(),
        );
        m.insert("language".into(), self.language.clone().into());
        m.insert("publisher".into(), self.publisher.clone().into());
        m.insert("series".into(), self.series.clone().into());
        m.insert("series_index".into(), self.series_index.into());
        m.insert(
            "tags".into(),
            self.tags
                .iter()
                .map(|a| Dynamic::from(a.clone()))
                .collect::<Array>()
                .into(),
        );
        let mut ids = Map::new();
        for (k, v) in &self.identifiers {
            ids.insert(k.as_str().into(), v.clone().into());
        }
        m.insert("identifiers".into(), ids.into());
        m.insert("description".into(), self.description.clone().into());
        m.insert("rights".into(), self.rights.clone().into());
        m
    }
}

#[derive(Clone)]
struct Mt(MatchInfo);

#[derive(Clone, Default)]
struct Data(Rc<RefCell<Map>>);

/// The `functions` argument: the other saved functions by name.
#[derive(Clone)]
struct Fns(Rc<FnTable>);

struct FnTable {
    functions: Vec<Function>,
    limits: Limits,
    out: Rc<RefCell<Vec<String>>>,
    deadline: Rc<Cell<Instant>>,
    env: Env,
    /// Compiled functions, by name, with the engine that runs them (no `functions` inside).
    cache: RefCell<std::collections::HashMap<String, Rc<(Engine, AST)>>>,
}

#[derive(Clone)]
struct FnRef {
    table: Rc<FnTable>,
    name: String,
}

impl FnTable {
    fn compiled(&self, name: &str) -> Result<Rc<(Engine, AST)>, String> {
        if let Some(c) = self.cache.borrow().get(name) {
            return Ok(c.clone());
        }
        let f = self
            .functions
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("there is no saved function called “{name}”"))?;
        let e = engine(
            self.limits,
            self.out.clone(),
            self.deadline.clone(),
            &self.env,
        );
        let ast = e
            .compile(&f.source)
            .map_err(|pe| format!("{name}: {}", pe.0))?;
        has_replace(&ast).map_err(|e| format!("{name}: {e}"))?;
        let c = Rc::new((e, ast));
        self.cache.borrow_mut().insert(name.to_string(), c.clone());
        Ok(c)
    }
}

fn friendly(e: &EvalAltResult, limits: &Limits) -> FnError {
    let mut inner = e;
    while let EvalAltResult::ErrorInFunctionCall(_, _, i, _) = inner {
        inner = i;
    }
    let line = inner.position().line();
    let message = match inner {
        EvalAltResult::ErrorTooManyOperations(_) => format!(
            "the function ran too long (more than {} operations in one call); is there a loop that never ends?",
            limits.max_ops
        ),
        EvalAltResult::ErrorTerminated(..) => format!(
            "the function was stopped after {} seconds",
            limits.max_time.as_secs()
        ),
        EvalAltResult::ErrorDataTooLarge(what, _) => {
            format!("a {} grew too large (limit reached)", what.to_lowercase())
        }
        EvalAltResult::ErrorStackOverflow(_) => "the function calls itself too deeply".into(),
        other => {
            let s = other.to_string();
            // Rhai appends " (line N, position M)"; the line is shown separately
            match s.rfind(" (line ") {
                Some(i) => s[..i].to_string(),
                None => s,
            }
        }
    };
    FnError { line, message }
}

fn engine(
    limits: Limits,
    out: Rc<RefCell<Vec<String>>>,
    deadline: Rc<Cell<Instant>>,
    env: &Env,
) -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(limits.max_ops);
    e.set_max_string_size(limits.max_string);
    e.set_max_array_size(limits.max_array);
    e.set_max_map_size(limits.max_map);
    e.set_max_call_levels(24);
    e.set_max_expr_depths(64, 32);
    e.disable_symbol("eval");
    e.on_progress(move |ops| {
        if ops % 256 == 0 && Instant::now() > deadline.get() {
            Some(Dynamic::UNIT)
        } else {
            None
        }
    });
    let cap = limits.max_output_lines;
    let o = out.clone();
    e.on_print(move |s| {
        let mut v = o.borrow_mut();
        if v.len() < cap {
            v.push(s.chars().take(4000).collect());
        } else if v.len() == cap {
            v.push("… (more output not shown)".into());
        }
    });
    let o = out;
    e.on_debug(move |s, _, pos| {
        let mut v = o.borrow_mut();
        if v.len() < cap {
            v.push(format!("[debug {pos}] {s}"));
        }
    });

    e.register_type_with_name::<Mt>("Match");
    e.register_fn("group", |m: &mut Mt, n: i64| -> Dynamic {
        usize::try_from(n)
            .ok()
            .and_then(|n| m.0.groups.get(n))
            .and_then(|g| g.clone())
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });
    e.register_fn("group", |m: &mut Mt, name: &str| -> Dynamic {
        m.0.names
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, i)| m.0.groups.get(*i))
            .and_then(|g| g.clone())
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });
    e.register_get("text", |m: &mut Mt| {
        m.0.groups[0].clone().unwrap_or_default()
    });
    e.register_get("start", |m: &mut Mt| m.0.start as i64);
    e.register_get("end", |m: &mut Mt| m.0.end as i64);
    e.register_get("groups", |m: &mut Mt| -> Array {
        m.0.groups
            .iter()
            .map(|g| g.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT))
            .collect()
    });
    e.register_fn("to_string", |m: &mut Mt| {
        m.0.groups[0].clone().unwrap_or_default()
    });

    e.register_type_with_name::<Data>("Data");
    e.register_indexer_get(|d: &mut Data, k: &str| -> Dynamic {
        d.0.borrow().get(k).cloned().unwrap_or(Dynamic::UNIT)
    });
    e.register_indexer_set(|d: &mut Data, k: &str, v: Dynamic| {
        d.0.borrow_mut().insert(k.into(), v);
    });
    e.register_fn("contains", |d: &mut Data, k: &str| {
        d.0.borrow().contains_key(k)
    });
    e.register_fn("keys", |d: &mut Data| -> Array {
        d.0.borrow()
            .keys()
            .map(|k| Dynamic::from(k.to_string()))
            .collect()
    });

    e.register_type_with_name::<Fns>("Functions");
    e.register_type_with_name::<FnRef>("SavedFunction");
    e.register_indexer_get(|f: &mut Fns, name: &str| -> Dynamic {
        if f.0.functions.iter().any(|x| x.name == name) {
            Dynamic::from(FnRef {
                table: f.0.clone(),
                name: name.to_string(),
            })
        } else {
            Dynamic::UNIT
        }
    });
    e.register_fn("contains", |f: &mut Fns, name: &str| {
        f.0.functions.iter().any(|x| x.name == name)
    });
    e.register_fn("keys", |f: &mut Fns| -> Array {
        f.0.functions
            .iter()
            .map(|x| Dynamic::from(x.name.clone()))
            .collect()
    });
    e.register_fn(
        "invoke",
        |f: &mut FnRef,
         m: Dynamic,
         number: i64,
         file: ImmutableString,
         meta: Map,
         data: Data|
         -> Result<Dynamic, Box<EvalAltResult>> {
            let c = f.table.compiled(&f.name).map_err(|msg| {
                Box::new(EvalAltResult::ErrorRuntime(
                    msg.into(),
                    rhai::Position::NONE,
                ))
            })?;
            let mut scope = Scope::new();
            c.0.call_fn::<Dynamic>(
                &mut scope,
                &c.1,
                "replace",
                (m, number, file.to_string(), meta, data),
            )
        },
    );
    e.register_fn("text_upper", |s: &str| upper_text(s));
    e.register_fn("text_lower", |s: &str| lower_text(s));
    e.register_fn("text_capitalize", |s: &str| capitalize_text(s));
    e.register_fn("text_swap_case", |s: &str| swap_case_text(s));
    e.register_fn("text_title", |s: &str| title_case(s));
    e.register_fn("smarten_hyphens", |s: &str| smarten_hyphens(s));
    e.register_fn("strip_tags", |s: &str| strip_tags(s));
    e.register_fn("escape_html", |s: &str| escape_html(s));
    let dict = env.dict.clone();
    e.register_fn("dict_recognized", move |w: ImmutableString| -> bool {
        dict.as_ref().is_some_and(|d| d(w.as_str()))
    });
    e
}

fn to_error(pe: &rhai::ParseError) -> FnError {
    FnError {
        line: pe.1.line(),
        message: pe.0.to_string(),
    }
}

/// Checks that `source` compiles and defines `replace` with five parameters (six with `functions`).
pub fn check(source: &str) -> Result<(), FnError> {
    let e = Engine::new();
    let ast = e.compile(source).map_err(|pe| to_error(&pe))?;
    has_replace(&ast)
}

fn has_replace(ast: &AST) -> Result<(), FnError> {
    let found = ast
        .iter_functions()
        .find(|f| f.name == "replace")
        .ok_or_else(|| FnError {
            line: None,
            message: "define fn replace(m, number, file, meta, data) { … }".into(),
        })?;
    if found.params.len() != 5 && found.params.len() != 6 {
        return Err(FnError {
            line: None,
            message: format!(
                "replace takes 5 parameters (m, number, file, meta, data) or 6 with functions, not {}",
                found.params.len()
            ),
        });
    }
    Ok(())
}

/// One run of a function over some files.
pub struct Runner {
    engine: Engine,
    ast: AST,
    limits: Limits,
    data: Data,
    meta: Map,
    pub number: i64,
    output: Rc<RefCell<Vec<String>>>,
    deadline: Rc<Cell<Instant>>,
    pub func: Function,
    /// `replace` also takes the `functions` argument.
    takes_functions: bool,
    fns: Fns,
    /// The first changes of the run as `(before, after)`, for a preview.
    pub changes: Vec<(String, String)>,
}

impl Runner {
    pub fn new(func: &Function, env: &Env) -> Result<Runner, FnError> {
        Runner::with_limits(func, env, Limits::default())
    }

    pub fn with_limits(func: &Function, env: &Env, limits: Limits) -> Result<Runner, FnError> {
        let output = Rc::new(RefCell::new(Vec::new()));
        let deadline = Rc::new(Cell::new(Instant::now() + limits.max_time));
        let engine = engine(limits, output.clone(), deadline.clone(), env);
        let ast = engine.compile(&func.source).map_err(|pe| to_error(&pe))?;
        has_replace(&ast)?;
        let takes_functions = ast
            .iter_functions()
            .any(|f| f.name == "replace" && f.params.len() == 6);
        let fns = Fns(Rc::new(FnTable {
            functions: env
                .functions
                .iter()
                .filter(|f| f.name != func.name)
                .cloned()
                .collect(),
            limits,
            out: output.clone(),
            deadline: deadline.clone(),
            env: Env {
                functions: Vec::new(),
                ..env.clone()
            },
            cache: Default::default(),
        }));
        Ok(Runner {
            engine,
            ast,
            limits,
            data: Data::default(),
            meta: env.to_map(),
            number: 0,
            output,
            deadline,
            func: func.clone(),
            takes_functions,
            fns,
            changes: Vec::new(),
        })
    }

    /// Sets the content of `data` (what a persistent function left at the end of its last run).
    pub fn set_data(&mut self, m: Map) {
        *self.data.0.borrow_mut() = m;
    }

    /// The content of `data` now.
    pub fn data_map(&self) -> Map {
        self.data.0.borrow().clone()
    }

    fn call(&mut self, m: Dynamic, number: i64, file: &str) -> Result<Dynamic, FnError> {
        self.deadline.set(Instant::now() + self.limits.max_time);
        let mut scope = Scope::new();
        let args = (
            m,
            number,
            file.to_string(),
            self.meta.clone(),
            self.data.clone(),
        );
        let r = if self.takes_functions {
            self.engine.call_fn::<Dynamic>(
                &mut scope,
                &self.ast,
                "replace",
                (args.0, args.1, args.2, args.3, args.4, self.fns.clone()),
            )
        } else {
            self.engine
                .call_fn::<Dynamic>(&mut scope, &self.ast, "replace", args)
        };
        r.map_err(|e| friendly(&e, &self.limits))
    }

    /// Replaces the matches of `pat` in `text` (inside `ranges`) with what the function returns.
    pub fn run_text(
        &mut self,
        pat: &Pattern,
        text: &str,
        ranges: Option<&[(usize, usize)]>,
        file: &str,
    ) -> Result<(String, usize), FnError> {
        let (out, n) = {
            let this = &mut *self;
            pat.replace_with(text, ranges, &mut |m| {
                this.number += 1;
                let number = this.number;
                let r = this.call(Dynamic::from(Mt(m.clone())), number, file)?;
                let rep = if r.is_unit() {
                    None
                } else if r.is_string() {
                    Some(r.into_string().unwrap_or_default())
                } else {
                    Some(r.to_string())
                };
                if let Some(rep) = &rep {
                    if this.changes.len() < 50 && *rep != m.groups[0].clone().unwrap_or_default() {
                        this.changes
                            .push((m.groups[0].clone().unwrap_or_default(), rep.clone()));
                    }
                }
                Ok(rep)
            })?
        };
        Ok((out, n))
    }

    /// The extra call after the last match, when the function asks for it. Returns what it returned.
    pub fn finish(&mut self, file: &str) -> Result<Option<String>, FnError> {
        if !self.func.call_after_last_match {
            return Ok(None);
        }
        let n = self.number + 1;
        let r = self.call(Dynamic::UNIT, n, file)?;
        Ok(if r.is_unit() {
            None
        } else {
            Some(r.to_string())
        })
    }

    /// What `print()` wrote.
    pub fn output(&self) -> Vec<String> {
        self.output.borrow().clone()
    }
}

/// The outcome of trying a function on some text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRun {
    pub text: String,
    pub count: usize,
    pub changes: Vec<(String, String)>,
    pub output: Vec<String>,
    pub final_output: Option<String>,
    pub error: Option<FnError>,
}

/// Runs `func` for the matches of `pat` in `text` without touching anything else (the dialog's Test button).
pub fn test_run(func: &Function, env: &Env, pat: &Pattern, text: &str, file: &str) -> TestRun {
    let mut run = match Runner::new(func, env) {
        Ok(r) => r,
        Err(e) => {
            return TestRun {
                text: text.into(),
                count: 0,
                changes: vec![],
                output: vec![],
                final_output: None,
                error: Some(e),
            };
        }
    };
    let res = run.run_text(pat, text, None, file);
    let (new, count, mut error) = match res {
        Ok((t, n)) => (t, n, None),
        Err(e) => (text.to_string(), 0, Some(e)),
    };
    let final_output = if error.is_none() {
        match run.finish(file) {
            Ok(f) => f,
            Err(e) => {
                error = Some(e);
                None
            }
        }
    } else {
        None
    };
    TestRun {
        text: new,
        count,
        changes: run.changes.clone(),
        output: run.output(),
        final_output,
        error,
    }
}

/// The text of the dialog's result area for a test run.
pub fn describe_test(t: &TestRun) -> String {
    let mut s = String::new();
    if let Some(e) = &t.error {
        s.push_str(&format!("Error: {e}\n"));
    } else {
        s.push_str(&trf(
            "{}\n",
            &[&ntr("{} replacement", "{} replacements", t.count)],
        ));
    }
    if !t.changes.is_empty() {
        s.push_str("\nChanges:\n");
        for (a, b) in t.changes.iter().take(20) {
            let one = |x: &str| -> String {
                let x = x.replace('\n', "⏎");
                if x.chars().count() > 60 {
                    format!("{}…", x.chars().take(59).collect::<String>())
                } else {
                    x
                }
            };
            s.push_str(&format!("  {} → {}\n", one(a), one(b)));
        }
        if t.changes.len() > 20 {
            s.push_str(&format!("  … and {} more\n", t.changes.len() - 20));
        }
    }
    if let Some(f) = &t.final_output {
        s.push_str(&format!("\nFinal output:\n{f}\n"));
    }
    if !t.output.is_empty() {
        s.push_str("\nDebug output:\n");
        for l in &t.output {
            s.push_str(l);
            s.push('\n');
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor_search::Opts;

    fn rx(q: &str) -> Pattern {
        Pattern::new(
            q,
            Opts {
                regex: true,
                case_sensitive: true,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn f(src: &str) -> Function {
        Function::new("t", "", src)
    }

    fn run(src: &str, pat: &str, text: &str) -> TestRun {
        test_run(&f(src), &Env::default(), &rx(pat), text, "OEBPS/c1.xhtml")
    }

    #[test]
    fn the_match_object_and_return_value() {
        let t = run(
            "fn replace(m, number, file, meta, data) { m.group(2) + m.group(1) + m.start + \"-\" + m.end + m.text }",
            r"(\w)(\w)",
            "ab cd",
        );
        assert_eq!(t.error, None);
        assert_eq!(t.text, "ba0-2ab dc3-5cd");
        assert_eq!(t.count, 2);
    }

    #[test]
    fn named_groups_and_missing_groups() {
        let t = run(
            r#"fn replace(m, number, file, meta, data) { if m.group(2) == () { m.group("w") + "!" } else { m.group("w") } }"#,
            r"(?P<w>\w)(\d)?",
            "a1 b",
        );
        assert_eq!(t.text, "a b!");
    }

    #[test]
    fn unit_keeps_the_match_and_other_types_are_stringified() {
        let t = run(
            "fn replace(m, number, file, meta, data) { if number == 2 { () } else { number } }",
            "x",
            "x x x",
        );
        assert_eq!(t.text, "1 x 3");
        assert_eq!(t.count, 2);
    }

    #[test]
    fn number_counts_across_files() {
        let mut r = Runner::new(
            &f("fn replace(m, number, file, meta, data) { `${number}@${file}` }"),
            &Env::default(),
        )
        .unwrap();
        let p = rx("x");
        assert_eq!(
            r.run_text(&p, "x x", None, "a.xhtml").unwrap().0,
            "1@a.xhtml 2@a.xhtml"
        );
        assert_eq!(r.run_text(&p, "x", None, "b.xhtml").unwrap().0, "3@b.xhtml");
        assert_eq!(r.number, 3);
    }

    #[test]
    fn file_is_empty_for_marked_text() {
        let t = test_run(
            &f("fn replace(m, number, file, meta, data) { \"[\" + file + \"]\" }"),
            &Env::default(),
            &rx("x"),
            "x",
            "",
        );
        assert_eq!(t.text, "[]");
    }

    #[test]
    fn meta_is_readable() {
        let env = Env {
            title: "Dune".into(),
            authors: vec!["Frank Herbert".into(), "Brian".into()],
            language: "en".into(),
            identifiers: vec![("isbn".into(), "123".into())],
            ..Default::default()
        };
        let t = test_run(
            &f(
                "fn replace(m, number, file, meta, data) { meta.title + \"|\" + meta.authors[0] + \"|\" + meta.authors.len + \"|\" + meta.language + \"|\" + meta.identifiers.isbn }",
            ),
            &env,
            &rx("x"),
            "x",
            "f",
        );
        assert_eq!(t.error, None);
        assert_eq!(t.text, "Dune|Frank Herbert|2|en|123");
    }

    #[test]
    fn data_persists_through_one_run_and_not_between_runs() {
        let src = "fn replace(m, number, file, meta, data) { if data[\"n\"] == () { data[\"n\"] = 0; } data[\"n\"] += 10; data[\"n\"] }";
        let t = run(src, "x", "x x x");
        assert_eq!(t.text, "10 20 30");
        let t2 = run(src, "x", "x");
        assert_eq!(t2.text, "10");
    }

    #[test]
    fn data_holds_arrays() {
        let src = "fn replace(m, number, file, meta, data) { if data[\"a\"] == () { data[\"a\"] = []; } data[\"a\"].push(m.text); data[\"a\"].len }";
        let t = run(src, "x", "x x");
        assert_eq!(t.error, None);
        assert_eq!(t.text, "1 2");
    }

    #[test]
    fn dict_recognized_uses_the_dictionary() {
        let env = Env {
            dict: Some(Rc::new(|w: &str| w == "example")),
            ..Default::default()
        };
        let d = builtins()
            .into_iter()
            .find(|b| b.name.starts_with("De-hyphenate"))
            .unwrap();
        let t = test_run(
            &d,
            &env,
            &rx(&d.find),
            "an exam-\n  ple and a foo-\nbar",
            "f",
        );
        assert_eq!(t.error, None);
        assert_eq!(t.text, "an example and a foo-\nbar");
        assert_eq!(t.output, vec!["kept: foo-\nbar"]);
    }

    #[test]
    fn print_is_collected() {
        let t = run(
            "fn replace(m, number, file, meta, data) { print(\"hello \" + number); debug(m.text); () }",
            "x",
            "x x",
        );
        assert_eq!(t.output[0], "hello 1");
        assert!(t.output[1].starts_with("[debug"));
        assert_eq!(t.output.len(), 4);
        assert_eq!(t.text, "x x");
    }

    #[test]
    fn call_after_last_match_gets_unit_and_its_output_is_kept() {
        let src = "// @call_after_last_match\nfn replace(m, number, file, meta, data) { if m == () { return \"total \" + data[\"n\"]; } data[\"n\"] = number; () }";
        let func = f(src);
        assert!(func.call_after_last_match);
        let t = test_run(&func, &Env::default(), &rx("x"), "x x x", "f");
        assert_eq!(t.error, None);
        assert_eq!(t.final_output.as_deref(), Some("total 3"));
        assert_eq!(t.text, "x x x");
    }

    #[test]
    fn header_options_are_read() {
        let func = f(
            "// @file_order spine-reverse\n// @suppress_result_dialog\nfn replace(m, number, file, meta, data) { () }",
        );
        assert_eq!(func.file_order, FileOrder::SpineReverse);
        assert!(func.suppress_result_dialog);
        assert!(!func.call_after_last_match);
        let plain = f("fn replace(m, number, file, meta, data) { () }");
        assert_eq!(plain.file_order, FileOrder::Normal);
    }

    #[test]
    fn file_order_follows_the_spine() {
        let files: Vec<String> = ["a.css", "c2.xhtml", "c1.xhtml", "c3.xhtml"]
            .map(String::from)
            .to_vec();
        let spine: Vec<String> = ["c1.xhtml", "c2.xhtml", "c3.xhtml", "gone.xhtml"]
            .map(String::from)
            .to_vec();
        assert_eq!(order_files(&files, &spine, FileOrder::Normal), files);
        assert_eq!(
            order_files(&files, &spine, FileOrder::Spine),
            ["c1.xhtml", "c2.xhtml", "c3.xhtml", "a.css"]
        );
        assert_eq!(
            order_files(&files, &spine, FileOrder::SpineReverse),
            ["a.css", "c3.xhtml", "c2.xhtml", "c1.xhtml"]
        );
    }

    #[test]
    fn errors_carry_a_line_number() {
        let t = run(
            "fn replace(m, number, file, meta, data) {\n    let a = 1;\n    undefined_function(a)\n}",
            "x",
            "x",
        );
        let e = t.error.unwrap();
        assert_eq!(e.line, Some(3));
        assert!(e.message.contains("undefined_function"), "{}", e.message);
        assert_eq!(t.text, "x");
    }

    #[test]
    fn syntax_errors_and_wrong_signatures_are_reported() {
        let e = check("fn replace(m, number, file, meta, data) {\n let = ;\n}").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(
            check("fn other() { 1 }")
                .unwrap_err()
                .message
                .contains("define fn replace")
        );
        assert!(
            check("fn replace(m) { 1 }")
                .unwrap_err()
                .message
                .contains("5 parameters")
        );
        assert!(check("fn replace(m, number, file, meta, data) { m.text }").is_ok());
    }

    #[test]
    fn an_endless_loop_is_stopped() {
        let t0 = Instant::now();
        let t = run(
            "fn replace(m, number, file, meta, data) { loop { } }",
            "x",
            "x",
        );
        let e = t.error.unwrap();
        assert!(e.message.contains("ran too long"), "{}", e.message);
        assert!(t0.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_slow_script_hits_the_time_limit() {
        let limits = Limits {
            max_ops: u64::MAX,
            max_time: Duration::from_millis(100),
            ..Limits::default()
        };
        let mut r = Runner::with_limits(
            &f("fn replace(m, number, file, meta, data) { let i = 0; while true { i += 1; } }"),
            &Env::default(),
            limits,
        )
        .unwrap();
        let e = r.run_text(&rx("x"), "x", None, "f").unwrap_err();
        assert!(e.message.contains("stopped after"), "{}", e.message);
    }

    #[test]
    fn big_allocations_are_refused() {
        let t = run(
            "fn replace(m, number, file, meta, data) { let s = \"x\"; for i in 0..40 { s += s; } s }",
            "x",
            "x",
        );
        assert!(t.error.unwrap().message.contains("too large"));
        let t = run(
            "fn replace(m, number, file, meta, data) { let a = []; for i in 0..1000000 { a.push(i); } a.len }",
            "x",
            "x",
        );
        assert!(t.error.is_some());
        let t = run(
            "fn replace(m, number, file, meta, data) { let s = \"\"; s.pad(100000000, 'x'); s }",
            "x",
            "x",
        );
        assert!(t.error.is_some());
    }

    #[test]
    fn runaway_recursion_is_refused() {
        let t = run(
            "fn r(n) { r(n + 1) }\nfn replace(m, number, file, meta, data) { r(0) }",
            "x",
            "x",
        );
        assert!(t.error.is_some());
    }

    #[test]
    fn there_is_no_file_network_or_eval() {
        for src in [
            "fn replace(m, number, file, meta, data) { eval(\"1\") }",
            "import \"x\" as y;\nfn replace(m, number, file, meta, data) { 1 }",
            "fn replace(m, number, file, meta, data) { open(\"/etc/passwd\") }",
            "fn replace(m, number, file, meta, data) { read_file(\"/etc/passwd\") }",
        ] {
            let t = run(src, "x", "x");
            assert!(t.error.is_some(), "{src}");
            assert_eq!(t.text, "x");
        }
    }

    #[test]
    fn output_is_capped() {
        let t = run(
            "fn replace(m, number, file, meta, data) { for i in 0..5000 { print(i); } () }",
            "x",
            "x",
        );
        assert_eq!(t.error, None);
        assert!(t.output.len() <= Limits::default().max_output_lines + 1);
    }

    // ---- text helpers -------------------------------------------------------------

    #[test]
    fn title_case_rules() {
        assert_eq!(title_case("the war of the worlds"), "The War of the Worlds");
        assert_eq!(title_case("a tale to remember"), "A Tale to Remember");
        assert_eq!(title_case("what is it for"), "What Is It For");
        assert_eq!(title_case("using NASA and iPhone"), "Using NASA and iPhone");
        assert_eq!(title_case("don't stop"), "Don't Stop");
        assert_eq!(title_case("SHOUTING WORDS"), "Shouting Words");
        assert_eq!(title_case("the end of the"), "The End of The");
    }

    #[test]
    fn case_helpers_leave_tags_alone() {
        let s = "<h1 class=\"Big\">hello <i>wORLD</i></h1>";
        assert_eq!(upper_text(s), "<h1 class=\"Big\">HELLO <i>WORLD</i></h1>");
        assert_eq!(lower_text("<B>HeLLo</B>"), "<B>hello</B>");
        assert_eq!(
            capitalize_text("<p>hELLO <i>wORLD</i></p>"),
            "<p>Hello <i>world</i></p>"
        );
        assert_eq!(swap_case_text("<p>aB</p>"), "<p>Ab</p>");
        assert_eq!(
            title_case("<p>the <i>old</i> man</p>"),
            "<p>The <i>Old</i> Man</p>"
        );
    }

    #[test]
    fn hyphens_are_smartened() {
        assert_eq!(
            smarten_hyphens("a---b--c - d-e"),
            "a\u{2014}b\u{2013}c \u{2013} d-e"
        );
        assert_eq!(
            smarten_hyphens("<a href=\"x--y\">p--q</a>"),
            "<a href=\"x--y\">p\u{2013}q</a>"
        );
    }

    #[test]
    fn strip_and_escape() {
        assert_eq!(strip_tags("a <b>c</b> d"), "a c d");
        assert_eq!(escape_html("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }

    // ---- the shipped examples ----------------------------------------------------------

    fn example(prefix: &str) -> Function {
        builtins()
            .into_iter()
            .find(|b| b.name.starts_with(prefix))
            .unwrap()
    }

    #[test]
    fn every_builtin_compiles() {
        for b in builtins() {
            assert!(check(&b.source).is_ok(), "{}", b.name);
            assert!(
                Pattern::new(
                    &b.find,
                    Opts {
                        regex: true,
                        ..Default::default()
                    }
                )
                .is_ok(),
                "{}",
                b.name
            );
        }
        let names: Vec<String> = builtins().into_iter().map(|b| b.name).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn example_title_case_headings_ignores_tags() {
        let b = example("Title-case");
        let t = test_run(
            &b,
            &Env::default(),
            &rx(&b.find),
            "<h1 class=\"big\">the war of <i>the</i> worlds</h1>",
            "f",
        );
        assert_eq!(
            t.text,
            "<h1 class=\"big\">The War of <i>the</i> Worlds</h1>"
        );
    }

    #[test]
    fn example_smarten_dashes() {
        let b = example("Smarten");
        let t = test_run(
            &b,
            &Env::default(),
            &rx(&b.find),
            "<p>wait---what--no</p>",
            "f",
        );
        assert_eq!(t.text, "<p>wait\u{2014}what\u{2013}no</p>");
    }

    #[test]
    fn example_auto_number_sections_counts_across_files() {
        let b = example("Auto-number");
        let mut r = Runner::new(&b, &Env::default()).unwrap();
        let p = rx(&b.find);
        assert_eq!(
            r.run_text(&p, "<h2 id=\"a\">One</h2><h2>Two</h2>", None, "c1")
                .unwrap()
                .0,
            "<h2 id=\"a\">1. One</h2><h2>2. Two</h2>"
        );
        assert_eq!(
            r.run_text(&p, "<h2>Three</h2>", None, "c2").unwrap().0,
            "<h2>3. Three</h2>"
        );
    }

    #[test]
    fn example_toc_from_headings_builds_a_list_in_the_final_call() {
        let b = example("Generate a table");
        assert!(b.call_after_last_match);
        let mut r = Runner::new(&b, &Env::default()).unwrap();
        let p = rx(&b.find);
        let text = "<h2 id=\"a\">One <i>x</i></h2><p>t</p><h2 id=\"b\">Two</h2>";
        let (out, _) = r.run_text(&p, text, None, "c1.xhtml").unwrap();
        assert_eq!(out, text);
        let fin = r.finish("").unwrap().unwrap();
        assert_eq!(
            fin,
            "<ol>\n<li><a href=\"c1.xhtml#a\">One x</a></li>\n<li><a href=\"c1.xhtml#b\">Two</a></li>\n</ol>"
        );
    }

    #[test]
    fn test_run_description_lists_changes_output_and_errors() {
        let t = run(
            "fn replace(m, number, file, meta, data) { print(\"hi\"); m.text + \"!\" }",
            "x",
            "x x",
        );
        let d = describe_test(&t);
        assert!(d.starts_with("2 replacements\n"));
        assert!(d.contains("x → x!"));
        assert!(d.contains("Debug output:\nhi\nhi\n"));
        let e = run(
            "fn replace(m, number, file, meta, data) {\n  nope()\n}",
            "x",
            "x",
        );
        let d = describe_test(&e);
        assert!(d.starts_with("Error: line 2:"), "{d}");
        let one = run(
            "fn replace(m, number, file, meta, data) { \"y\" }",
            "x",
            "x",
        );
        assert!(describe_test(&one).starts_with("1 replacement\n"));
    }

    // ---- store -----------------------------------------------------------------------

    #[test]
    fn functions_round_trip_through_json_and_files() {
        let mut mine =
            f("// @call_after_last_match\nfn replace(m, number, file, meta, data) { () }");
        mine.name = "Mine".into();
        mine.find = "x".into();
        let json = to_json(&[mine.clone(), builtins().remove(0)]);
        let back = parse_json(&json).unwrap();
        assert_eq!(back.len(), 1, "built-ins are not exported");
        assert_eq!(back[0].name, "Mine");
        assert!(back[0].call_after_last_match);
        assert!(parse_json("nope").is_err());
        let dir = std::env::temp_dir().join(format!("bookx-fn-{}", std::process::id()));
        let file = dir.join("fns.json");
        save_to(&file, &[mine.clone()]).unwrap();
        assert_eq!(load_from(&file)[0].name, "Mine");
        assert!(load_from(&dir.join("missing.json")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_functions_replace_built_ins_of_the_same_name() {
        let mut mine = f("fn replace(m, number, file, meta, data) { 1 }");
        mine.name = builtins()[0].name.clone();
        let all = merged(&[mine]);
        assert_eq!(all.len(), builtins().len());
        assert!(!all.last().unwrap().builtin);
        assert!(all.last().unwrap().source.contains("{ 1 }"));
    }

    #[test]
    fn a_function_calls_another_saved_function() {
        let helper = Function::new(
            "wrap",
            "",
            "fn replace(m, number, file, meta, data) { \"[\" + m.text + \"]\" }",
        );
        let main = Function::new(
            "main",
            "",
            "fn replace(m, number, file, meta, data, functions) {\n    functions[\"wrap\"].invoke(m, number, file, meta, data) + \"!\"\n}",
        );
        let env = Env {
            functions: vec![helper, main.clone()],
            ..Env::default()
        };
        let t = test_run(&main, &env, &rx("[a-z]+"), "ab cd", "f");
        assert_eq!(t.error, None);
        assert_eq!(t.text, "[ab]! [cd]!");
    }

    #[test]
    fn called_functions_share_data_and_report_missing_ones() {
        let counter = Function::new(
            "count",
            "",
            "fn replace(m, number, file, meta, data) {\n    if data[\"n\"] == () { data[\"n\"] = 0; }\n    data[\"n\"] += 1;\n    data[\"n\"]\n}",
        );
        let main = Function::new(
            "main",
            "",
            "fn replace(m, number, file, meta, data, functions) {\n    functions[\"count\"].invoke(m, number, file, meta, data)\n}",
        );
        let env = Env {
            functions: vec![counter],
            ..Env::default()
        };
        let t = test_run(&main, &env, &rx("x"), "x x x", "");
        assert_eq!(t.text, "1 2 3");
        let missing = Function::new(
            "m2",
            "",
            "fn replace(m, number, file, meta, data, functions) {\n    functions[\"nope\"].invoke(m, number, file, meta, data)\n}",
        );
        let t = test_run(&missing, &env, &rx("x"), "x", "");
        assert!(
            t.error.is_some(),
            "an unknown function is an error, not a crash"
        );
        // five-parameter functions keep working
        let old = Function::new(
            "old",
            "",
            "fn replace(m, number, file, meta, data) { \"o\" }",
        );
        assert_eq!(test_run(&old, &env, &rx("x"), "x", "").text, "o");
        assert!(check("fn replace(a, b) { a }").is_err());
    }

    #[test]
    fn persistent_data_survives_between_runs() {
        let dir = std::env::temp_dir().join(format!("bookx-fndata-{}", uuid::Uuid::new_v4()));
        let mut f = Function::new(
            "total",
            "",
            "// @persistent_data\nfn replace(m, number, file, meta, data) {\n    if data[\"sum\"] == () { data[\"sum\"] = 0; }\n    data[\"sum\"] += 1;\n    data[\"list\"] = [1, \"a\", true];\n    data[\"sum\"]\n}",
        );
        assert!(f.persistent_data);
        f.read_header();
        let file = data_file(&dir, "/lib/one", &f.name);
        let mut r = Runner::new(&f, &Env::default()).unwrap();
        r.set_data(load_data(&file));
        let (out, _) = r.run_text(&rx("x"), "x x", None, "").unwrap();
        assert_eq!(out, "1 2");
        save_data(&file, &r.data_map()).unwrap();
        // the next run starts where the last one stopped
        let mut r2 = Runner::new(&f, &Env::default()).unwrap();
        r2.set_data(load_data(&file));
        let (out, _) = r2.run_text(&rx("x"), "x", None, "").unwrap();
        assert_eq!(out, "3");
        // another library has its own data
        let other = data_file(&dir, "/lib/two", &f.name);
        assert_ne!(file, other);
        assert!(load_data(&other).is_empty());
        // odd names cannot leave the folder
        let evil = data_file(&dir, "l", "../../etc/passwd");
        assert!(evil.starts_with(&dir));
        assert!(!evil.to_string_lossy().contains(".."));
        std::fs::write(&file, "garbage").unwrap();
        assert!(load_data(&file).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_final_output_is_added_after_the_marked_text() {
        let text = "<p>one</p>\n<p>two</p>\n<p>three</p>";
        let marked = [(0, 10), (11, 21)];
        let (out, range) = append_after_marked(text, &marked, "TOTAL").unwrap();
        assert_eq!(out, "<p>one</p>\n<p>two</p>\nTOTAL\n<p>three</p>");
        assert_eq!(&out[range.0..range.1], "TOTAL");
        assert!(append_after_marked(text, &[], "x").is_none());
        assert!(
            append_after_marked("é", &[(0, 1)], "x").is_none(),
            "not on a character boundary"
        );
        let mut f = Function::new(
            "a",
            "",
            "// @append_final_output_to_marked\nfn replace(m, number, file, meta, data) { () }",
        );
        assert!(f.append_final_output_to_marked);
        f.source = "fn replace(m, number, file, meta, data) { () }".into();
        f.read_header();
        assert!(!f.append_final_output_to_marked);
    }
}
