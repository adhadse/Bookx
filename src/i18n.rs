//! Translations (X6, X17), without GTK and without a gettext library.
//!
//! Strings are marked in the source with `tr("English text")`, `tr_c("context", "text")`,
//! `trf("Imported {} books", &[&n])` and `ntr("{} book", "{} books", n)`. The English text is the key; a
//! language that has no entry shows the English text. Translations are gettext `.po` files in
//! `po/`, embedded in the program (so a Flatpak or a single binary needs no locale folder), and
//! optionally compiled `.mo` files in a locale directory
//! (`<dir>/<lang>/LC_MESSAGES/bookx.mo`, the layout Weblate and `msgfmt` produce) which win over
//! the embedded ones. `build-aux/i18n.sh` writes `po/bookx.pot` from the marked strings; a test
//! fails when the template is out of date.
//!
//! The language is chosen in Preferences (empty = follow the system: `LANGUAGE`, `LC_ALL`,
//! `LC_MESSAGES`, `LANG`) and applies to windows opened after the change, fully after a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

// `LANGUAGES: &[(code, name in its own language, embedded .po)]` and `REVIEWED_ONLY`, written by build.rs.
// With the feature `reviewed-locales-only` (Flathub) only English and the languages in
// `po/reviewed-locales.txt` are listed and embedded.
include!(concat!(env!("OUT_DIR"), "/languages.rs"));

/// Languages written right to left (the interface is mirrored for them).
pub fn is_rtl(code: &str) -> bool {
    matches!(base_code(code).as_str(), "ar" | "he" | "fa" | "ur" | "yi")
}

/// Whether the language in use is written right to left.
pub fn current_is_rtl() -> bool {
    is_rtl(&current())
}

/// `1234567` → `1,234,567` (English, Hindi with lakh grouping `12,34,567`), `1.234.567`
/// (German, Spanish), `1 234 567` with narrow no-break spaces (French).
pub fn fmt_int(n: i64) -> String {
    fmt_int_for(&current(), n)
}

pub fn fmt_int_for(code: &str, n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let (sep, indian) = match code {
        "de" | "es" => (".", false),
        "fr" => ("\u{202f}", false),
        "hi" => (",", true),
        _ => (",", false),
    };
    // Spanish writes 4 digit numbers without a separator.
    let group = !(code == "es" && digits.len() <= 4);
    let mut parts: Vec<&str> = Vec::new();
    let mut rest = digits.as_str();
    if group {
        let mut first = true;
        while rest.len() > 3 {
            let take = if indian && !first { 2 } else { 3 };
            let (head, tail) = rest.split_at(rest.len() - take);
            parts.push(tail);
            rest = head;
            first = false;
        }
    }
    parts.push(rest);
    parts.reverse();
    let body = parts.join(sep);
    if n < 0 {
        format!("-{body}")
    } else {
        body
    }
}

/// A decimal number with the language's separator.
#[allow(dead_code)] // for the windows that show measurements
pub fn fmt_decimal(value: f64, places: usize) -> String {
    let s = format!("{value:.places$}");
    match current().as_str() {
        "de" | "es" | "fr" => s.replace('.', ","),
        _ => s,
    }
}

/// A calendar date in the order and separators of the language (`2026-11-13` in English,
/// `13.11.2026` German, `13/11/2026` Spanish, French and Hindi).
#[allow(dead_code)] // for the windows that show plain dates
pub fn fmt_date(year: i32, month: u32, day: u32) -> String {
    fmt_date_for(&current(), year, month, day)
}

/// A local date and time for lists and details: English keeps `13 Nov 2026 14:05`, the other
/// languages use their own date order with a 24 hour clock.
pub fn fmt_when(t: &chrono::DateTime<chrono::Local>) -> String {
    use chrono::{Datelike, Timelike};
    let code = current();
    if code == "en" {
        return t.format("%d %b %Y %H:%M").to_string();
    }
    format!(
        "{} {:02}:{:02}",
        fmt_date_for(&code, t.year(), t.month(), t.day()),
        t.hour(),
        t.minute()
    )
}

pub fn fmt_date_for(code: &str, year: i32, month: u32, day: u32) -> String {
    match code {
        "de" => format!("{day:02}.{month:02}.{year}"),
        "es" | "fr" | "hi" => format!("{day:02}/{month:02}/{year}"),
        _ => format!("{year}-{month:02}-{day:02}"),
    }
}

#[derive(Default)]
struct State {
    code: String,
    /// Raw gettext catalogue: key `[ctx\x04]id[\0plural]`, value the forms joined by `\0`.
    messages: HashMap<String, String>,
    plural: PluralRule,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

/// How a language picks the plural form for a count (from the `Plural-Forms` header).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PluralRule {
    /// `n != 1`: English, German, Spanish.
    #[default]
    OneOther,
    /// `n > 1`: French, Hindi, Portuguese (Brazil); 0 and 1 are singular.
    ZeroOneOther,
    /// One form only: Japanese, Chinese, Korean.
    Single,
}

impl PluralRule {
    /// The rule named by a header such as `nplurals=2; plural=(n > 1);`.
    pub fn from_header(header: &str) -> PluralRule {
        let h: String = header.chars().filter(|c| !c.is_whitespace()).collect();
        if h.contains("nplurals=1") {
            PluralRule::Single
        } else if h.contains("plural=(n>1)") || h.contains("plural=n>1") {
            PluralRule::ZeroOneOther
        } else {
            PluralRule::OneOther
        }
    }

    pub fn index(self, n: u64) -> usize {
        match self {
            PluralRule::OneOther => usize::from(n != 1),
            PluralRule::ZeroOneOther => usize::from(n > 1),
            PluralRule::Single => 0,
        }
    }
}

fn lookup(key: &str) -> Option<String> {
    let g = STATE.read().ok()?;
    g.as_ref()?.messages.get(key).cloned()
}

/// Translates a marked string; the text itself when there is no translation.
pub fn tr(msgid: &str) -> String {
    lookup(msgid).unwrap_or_else(|| msgid.to_string())
}

/// Translates a string that needs a context to tell homonyms apart ("Open" the verb in a menu or
/// "Open" the state of a device).
#[allow(dead_code)] // for words that are the same in English but differ in other languages
pub fn tr_c(ctx: &str, msgid: &str) -> String {
    lookup(&format!("{ctx}\u{4}{msgid}")).unwrap_or_else(|| msgid.to_string())
}

/// Translates a string with `{}` placeholders and fills them in order. Marked like `tr`.
pub fn trf(msgid: &str, args: &[&dyn std::fmt::Display]) -> String {
    fill(&tr(msgid), args)
}

/// Picks the singular or plural form for `n` in the current language and replaces every `{}`
/// with the count. `ntr("{} book", "{} books", n)`.
pub fn ntr(singular: &str, plural: &str, n: impl Count) -> String {
    let n = n.count();
    let (rule, found) = match STATE.read() {
        Ok(g) => match g.as_ref() {
            Some(s) => (
                s.plural,
                s.messages.get(&format!("{singular}\0{plural}")).cloned(),
            ),
            None => (PluralRule::OneOther, None),
        },
        Err(_) => (PluralRule::OneOther, None),
    };
    let template = found
        .and_then(|f| f.split('\0').nth(rule.index(n)).map(str::to_string))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            if PluralRule::OneOther.index(n) == 0 {
                singular.to_string()
            } else {
                plural.to_string()
            }
        });
    template.replace("{}", &fmt_int(n as i64))
}

/// Whole numbers `ntr` accepts, so call sites do not need casts.
pub trait Count {
    fn count(self) -> u64;
}

macro_rules! count_unsigned {
    ($($t:ty),*) => {$(impl Count for $t { fn count(self) -> u64 { self as u64 } })*};
}
macro_rules! count_signed {
    ($($t:ty),*) => {$(impl Count for $t { fn count(self) -> u64 { self.max(0) as u64 } })*};
}
count_unsigned!(usize, u64, u32, u16, u8);
count_signed!(i64, i32, isize);

/// Replaces `{}` placeholders in order; extra placeholders stay, extra arguments are ignored.
pub fn fill(template: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut rest = template;
    let mut it = args.iter();
    while let Some(p) = rest.find("{}") {
        out.push_str(&rest[..p]);
        match it.next() {
            Some(a) => out.push_str(&a.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[p + 2..];
    }
    out.push_str(rest);
    out
}

/// The language in use (`en` when nothing else matched).
pub fn current() -> String {
    STATE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.code.clone()))
        .unwrap_or_else(|| "en".into())
}

/// Chooses the language: `setting` is a code or empty for the system language.
pub fn set_language(setting: &str) {
    let code = resolve(setting, &|k| std::env::var(k).ok());
    let messages = load(&code, &locale_dirs());
    let plural = messages
        .get("")
        .map(|h| PluralRule::from_header(h))
        .unwrap_or_default();
    if let Ok(mut g) = STATE.write() {
        *g = Some(State {
            code,
            messages,
            plural,
        });
    }
}

/// The code to use for a setting and an environment: the setting if it names a known language,
/// else the first system language we have, else `en`.
pub fn resolve(setting: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let setting = setting.trim();
    if !setting.is_empty() {
        let base = base_code(setting);
        if known(&base) {
            return base;
        }
        return "en".into();
    }
    if let Some(over) = env("BOOKX_LANG") {
        let base = base_code(&over);
        if known(&base) {
            return base;
        }
    }
    let list = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| env(k))
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX");
    if let Some(list) = list {
        for part in list.split(':') {
            let base = base_code(part);
            if known(&base) {
                return base;
            }
        }
    }
    "en".into()
}

/// `es_ES.UTF-8@euro` → `es`.
fn base_code(s: &str) -> String {
    s.split(['.', '@', '_', '-'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn known(code: &str) -> bool {
    LANGUAGES.iter().any(|(c, _, _)| *c == code)
        || locale_dirs().iter().any(|d| mo_path(d, code).exists())
}

fn mo_path(dir: &Path, code: &str) -> PathBuf {
    dir.join(code).join("LC_MESSAGES").join("bookx.mo")
}

fn locale_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("BOOKX_LOCALE_DIR") {
        v.push(PathBuf::from(d));
    }
    v.push(PathBuf::from("/app/share/locale"));
    v.push(PathBuf::from("/usr/share/locale"));
    if let Some(d) = directories::BaseDirs::new() {
        v.push(d.data_dir().join("locale"));
    }
    v
}

/// The messages of a language: the embedded `.po`, overridden by a `.mo` file when one exists.
/// The header (`Plural-Forms`) is under the key `""`.
pub fn load(code: &str, dirs: &[PathBuf]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some((_, _, Some(po))) = LANGUAGES.iter().find(|(c, _, _)| *c == code) {
        let (header, m) = parse_po_raw(po);
        out = m;
        out.insert(String::new(), header);
    }
    for d in dirs {
        if let Ok(bytes) = std::fs::read(mo_path(d, code)) {
            if let Ok(m) = parse_mo(&bytes) {
                out.extend(m);
                break;
            }
        }
    }
    out
}

/// Languages the user can pick: the embedded ones, then any language with a `.mo` in a locale dir.
pub fn available() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = LANGUAGES
        .iter()
        .map(|(c, n, _)| (c.to_string(), n.to_string()))
        .collect();
    for d in locale_dirs() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let code = e.file_name().to_string_lossy().to_string();
                if code.len() <= 3
                    && mo_path(&d, &code).exists()
                    && !v.iter().any(|(c, _)| *c == code)
                {
                    v.push((code.clone(), code));
                }
            }
        }
    }
    v
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('r') => out.push('\r'),
            // Rust source: `\u{23ce}` and a backslash at the end of a line (skips the indent)
            Some('u') if it.as_str().starts_with('{') => {
                let rest = it.as_str();
                if let Some(end) = rest.find('}') {
                    if let Some(c) = u32::from_str_radix(&rest[1..end], 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        out.push(c);
                        for _ in 0..=end {
                            it.next();
                        }
                        continue;
                    }
                }
                out.push_str("\\u");
            }
            Some('\n') => {
                while it.as_str().starts_with([' ', '\t', '\n']) {
                    it.next();
                }
            }
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// Reads a `.po` file into `parse_po_raw` form without the header.
#[cfg(test)]
pub fn parse_po(text: &str) -> HashMap<String, String> {
    parse_po_raw(text).1
}

/// Reads a `.po` file: the header text and the catalogue in gettext's raw form
/// (`[ctx\x04]id[\0plural]` → forms joined by `\0`). Fuzzy entries and entries with an empty
/// translation are skipped.
pub fn parse_po_raw(text: &str) -> (String, HashMap<String, String>) {
    #[derive(Default)]
    struct Entry {
        ctx: Option<String>,
        id: Option<String>,
        plural: Option<String>,
        forms: Vec<String>,
        fuzzy: bool,
    }
    #[derive(PartialEq, Clone, Copy)]
    enum Cur {
        None,
        Ctx,
        Id,
        Plural,
        Str(usize),
    }
    fn flush(e: &mut Entry, header: &mut String, out: &mut HashMap<String, String>) {
        let e = std::mem::take(e);
        let Some(id) = e.id else { return };
        if e.fuzzy || e.forms.is_empty() || e.forms.iter().all(|f| f.is_empty()) {
            return;
        }
        if id.is_empty() && e.ctx.is_none() {
            *header = e.forms[0].clone();
            return;
        }
        let mut key = String::new();
        if let Some(c) = &e.ctx {
            key.push_str(c);
            key.push('\u{4}');
        }
        key.push_str(&id);
        if let Some(p) = &e.plural {
            key.push('\0');
            key.push_str(p);
            if e.forms.iter().any(|f| f.is_empty()) {
                return;
            }
        }
        out.insert(key, e.forms.join("\0"));
    }
    let mut out = HashMap::new();
    let mut header = String::new();
    let mut e = Entry::default();
    let mut cur = Cur::None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            flush(&mut e, &mut header, &mut out);
            cur = Cur::None;
        } else if let Some(c) = line.strip_prefix("#,") {
            if c.contains("fuzzy") {
                e.fuzzy = true;
            }
        } else if line.starts_with('#') {
        } else if let Some(r) = line.strip_prefix("msgctxt") {
            if e.id.is_some() {
                flush(&mut e, &mut header, &mut out);
            }
            e.ctx = Some(quoted(r).map(|s| unescape(&s)).unwrap_or_default());
            cur = Cur::Ctx;
        } else if let Some(r) = line.strip_prefix("msgid_plural") {
            e.plural = Some(quoted(r).map(|s| unescape(&s)).unwrap_or_default());
            cur = Cur::Plural;
        } else if let Some(r) = line.strip_prefix("msgid") {
            if e.id.is_some() {
                flush(&mut e, &mut header, &mut out);
            }
            e.id = Some(quoted(r).map(|s| unescape(&s)).unwrap_or_default());
            cur = Cur::Id;
        } else if let Some(r) = line.strip_prefix("msgstr[") {
            let (n, rest) = r.split_once(']').unwrap_or(("0", ""));
            let n: usize = n.parse().unwrap_or(0);
            if e.forms.len() <= n {
                e.forms.resize(n + 1, String::new());
            }
            e.forms[n] = quoted(rest).map(|s| unescape(&s)).unwrap_or_default();
            cur = Cur::Str(n);
        } else if let Some(r) = line.strip_prefix("msgstr") {
            e.forms = vec![quoted(r).map(|s| unescape(&s)).unwrap_or_default()];
            cur = Cur::Str(0);
        } else if line.starts_with('"') {
            let piece = quoted(line).map(|s| unescape(&s)).unwrap_or_default();
            match cur {
                Cur::Ctx => e.ctx.get_or_insert_with(String::new).push_str(&piece),
                Cur::Id => e.id.get_or_insert_with(String::new).push_str(&piece),
                Cur::Plural => e.plural.get_or_insert_with(String::new).push_str(&piece),
                Cur::Str(n) => {
                    if let Some(f) = e.forms.get_mut(n) {
                        f.push_str(&piece)
                    }
                }
                Cur::None => {}
            }
        }
    }
    flush(&mut e, &mut header, &mut out);
    (header, out)
}

fn quoted(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix('"')?;
    let s = s.strip_suffix('"')?;
    Some(s.to_string())
}

/// Reads a compiled `.mo` file (either byte order) into the raw form (see `parse_po_raw`); the
/// header entry is under the key `""`.
pub fn parse_mo(b: &[u8]) -> Result<HashMap<String, String>, String> {
    if b.len() < 28 {
        return Err("too short for a .mo file".into());
    }
    let magic = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let le = match magic {
        0x9504_12de => true,
        0xde12_0495 => false,
        _ => return Err("not a .mo file".into()),
    };
    let rd = |o: usize| -> Result<u32, String> {
        let s = b.get(o..o + 4).ok_or("truncated .mo file")?;
        let a = [s[0], s[1], s[2], s[3]];
        Ok(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    };
    let n = rd(8)? as usize;
    let (otab, ttab) = (rd(12)? as usize, rd(16)? as usize);
    if n > 1_000_000 {
        return Err("implausible .mo file".into());
    }
    let mut out = HashMap::new();
    for i in 0..n {
        let (ol, oo) = (rd(otab + i * 8)? as usize, rd(otab + i * 8 + 4)? as usize);
        let (tl, to) = (rd(ttab + i * 8)? as usize, rd(ttab + i * 8 + 4)? as usize);
        let o = b.get(oo..oo + ol).ok_or("truncated .mo file")?;
        let t = b.get(to..to + tl).ok_or("truncated .mo file")?;
        if t.is_empty() {
            continue;
        }
        out.insert(
            String::from_utf8_lossy(o).to_string(),
            String::from_utf8_lossy(t).to_string(),
        );
    }
    Ok(out)
}

/// Writes a `.mo` file (little endian, no hash table); used by the tests and by tooling.
#[cfg(test)]
pub fn write_mo(messages: &HashMap<String, String>) -> Vec<u8> {
    let mut keys: Vec<&String> = messages.keys().collect();
    keys.sort();
    let n = keys.len();
    let head = 28usize;
    let otab = head;
    let ttab = otab + n * 8;
    let mut strings = Vec::new();
    let base = ttab + n * 8;
    let (mut ot, mut tt) = (Vec::new(), Vec::new());
    for k in &keys {
        ot.push((k.len() as u32, (base + strings.len()) as u32));
        strings.extend_from_slice(k.as_bytes());
        strings.push(0);
    }
    for k in &keys {
        let v = &messages[*k];
        tt.push((v.len() as u32, (base + strings.len()) as u32));
        strings.extend_from_slice(v.as_bytes());
        strings.push(0);
    }
    let mut out = Vec::new();
    for w in [0x9504_12deu32, 0, n as u32, otab as u32, ttab as u32, 0, 0] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    for (l, o) in ot.iter().chain(tt.iter()) {
        out.extend_from_slice(&l.to_le_bytes());
        out.extend_from_slice(&o.to_le_bytes());
    }
    out.extend_from_slice(&strings);
    out
}

/// Marks a string in a constant (where `tr` cannot be called); the text is translated where it is
/// used, with `tr(text)`. Same role as gettext's `N_`.
#[allow(non_snake_case)]
pub const fn N_(s: &'static str) -> &'static str {
    s
}

/// A marked string as the template lists it.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Msg {
    pub ctx: Option<String>,
    pub id: String,
    pub plural: Option<String>,
}

#[cfg(test)]
impl Msg {
    fn simple(id: &str) -> Msg {
        Msg {
            ctx: None,
            id: id.to_string(),
            plural: None,
        }
    }

    /// The key in the raw catalogue (see `parse_po_raw`).
    pub fn key(&self) -> String {
        let mut k = String::new();
        if let Some(c) = &self.ctx {
            k.push_str(c);
            k.push('\u{4}');
        }
        k.push_str(&self.id);
        if let Some(p) = &self.plural {
            k.push('\0');
            k.push_str(p);
        }
        k
    }
}

/// The strings marked with `tr("…")`, `N_("…")`, `trf("…", …)`, `tr_c("ctx", "…")` and
/// `ntr("one", "many", n)` in a source file, without duplicates.
#[cfg(test)]
pub fn extract(source: &str) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::new();
    for (needle, strings, rest) in [
        ("tr(", 1, Rest::Close),
        ("N_(", 1, Rest::Close),
        ("trf(", 1, Rest::Comma),
        ("tr_c(", 2, Rest::Close),
        ("ntr(", 2, Rest::Comma),
    ] {
        extract_with(source, needle, strings, rest, &mut out);
    }
    out
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum Rest {
    /// `)` (a trailing comma is fine) follows the literals.
    Close,
    /// `,` follows the literals (more arguments).
    Comma,
}

/// A string literal starting at `from` (after optional whitespace): its text and the index after
/// the closing quote.
#[cfg(test)]
fn literal(source: &str, from: usize) -> Option<(String, usize)> {
    let rest = source[from..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let bytes = source.as_bytes();
    let first = source.len() - rest.len() + 1;
    let mut j = first;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return Some((unescape(&source[first..j]), j + 1)),
            _ => j += 1,
        }
    }
    None
}

#[cfg(test)]
fn extract_with(source: &str, needle: &str, strings: usize, rest: Rest, out: &mut Vec<Msg>) {
    let bytes = source.as_bytes();
    let mut i = 0;
    while let Some(p) = source[i..].find(needle) {
        let start = i + p;
        i = start + needle.len();
        // not part of a longer name (`str(`, `entr(`) or a method call
        if start > 0 {
            let prev = bytes[start - 1] as char;
            if prev.is_alphanumeric() || prev == '_' || prev == '.' {
                continue;
            }
        }
        let mut lits = Vec::new();
        let mut at = i;
        let mut ok = true;
        for k in 0..strings {
            if k > 0 {
                let t = source[at..].trim_start();
                let Some(t) = t.strip_prefix(',') else {
                    ok = false;
                    break;
                };
                at = source.len() - t.len();
            }
            match literal(source, at) {
                Some((s, end)) => {
                    lits.push(s);
                    at = end;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let after = source[at..].trim_start();
        let good = match rest {
            Rest::Close => {
                let a = after.strip_prefix(',').unwrap_or(after).trim_start();
                a.starts_with(')')
            }
            Rest::Comma => after.starts_with(','),
        };
        if !good || lits[0].is_empty() {
            continue;
        }
        let msg = match needle {
            "tr_c(" => Msg {
                ctx: Some(lits[0].clone()),
                id: lits[1].clone(),
                plural: None,
            },
            "ntr(" => Msg {
                ctx: None,
                id: lits[0].clone(),
                plural: Some(lits[1].clone()),
            },
            _ => Msg::simple(&lits[0]),
        };
        if !out.contains(&msg) {
            out.push(msg);
        }
        i = at.max(i);
    }
}

/// The text of `po/bookx.pot` for these strings (sorted, so adding a string changes one place).
#[cfg(test)]
pub fn pot_text(msgs: &[Msg]) -> String {
    let mut sorted = msgs.to_vec();
    sorted.sort();
    let mut s = String::from(
        "# Bookx translation template.\n# Generated by build-aux/i18n.sh; do not edit by hand.\nmsgid \"\"\nmsgstr \"\"\n\"Project-Id-Version: Bookx\\n\"\n\"Content-Type: text/plain; charset=UTF-8\\n\"\n\"Plural-Forms: nplurals=2; plural=(n != 1);\\n\"\n\n",
    );
    for m in &sorted {
        if let Some(c) = &m.ctx {
            s.push_str(&format!("msgctxt \"{}\"\n", escape(c)));
        }
        s.push_str(&format!("msgid \"{}\"\n", escape(&m.id)));
        match &m.plural {
            Some(p) => s.push_str(&format!(
                "msgid_plural \"{}\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n\n",
                escape(p)
            )),
            None => s.push_str("msgstr \"\"\n\n"),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(a, _)| *a == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    #[cfg(not(feature = "reviewed-locales-only"))]
    fn the_setting_wins_over_the_system() {
        let e = env(&[("LANG", "de_DE.UTF-8")]);
        assert_eq!(resolve("es", &e), "es");
        assert_eq!(resolve("", &e), "de");
        assert_eq!(resolve("xx", &e), "en");
        assert_eq!(resolve("es_MX.UTF-8", &e), "es");
    }

    #[test]
    #[cfg(not(feature = "reviewed-locales-only"))]
    fn the_system_language_list_is_read_in_order() {
        assert_eq!(
            resolve(
                "",
                &env(&[("LANGUAGE", "xx:es:en"), ("LANG", "en_US.UTF-8")])
            ),
            "es"
        );
        assert_eq!(resolve("", &env(&[("LANG", "C")])), "en");
        assert_eq!(resolve("", &env(&[])), "en");
        assert_eq!(
            resolve("", &env(&[("LC_ALL", "es_ES.UTF-8"), ("LANG", "de_DE")])),
            "es"
        );
        assert_eq!(
            resolve("", &env(&[("BOOKX_LANG", "de"), ("LANG", "es")])),
            "de"
        );
    }

    #[test]
    fn po_files_are_parsed() {
        let po = r#"
# comment
msgid ""
msgstr ""
"Content-Type: text/plain; charset=UTF-8\n"

msgid "Open"
msgstr "Abrir"

#, fuzzy
msgid "Close"
msgstr "Cerrar mal"

msgid "Two "
"lines"
msgstr "Dos "
"líneas\n"

msgid "Untranslated"
msgstr ""

msgctxt "menu"
msgid "File"
msgstr "Archivo"

msgid "Quote \"x\""
msgstr "Comilla \"x\""
"#;
        let m = parse_po(po);
        assert_eq!(m.get("Open").unwrap(), "Abrir");
        assert!(!m.contains_key("Close"));
        assert_eq!(m.get("Two lines").unwrap(), "Dos líneas\n");
        assert!(!m.contains_key("Untranslated"));
        assert_eq!(m.get("menu\u{4}File").unwrap(), "Archivo");
        assert_eq!(m.get("Quote \"x\"").unwrap(), "Comilla \"x\"");
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn mo_files_round_trip_and_bad_ones_are_errors() {
        let mut m = HashMap::new();
        m.insert("Open".to_string(), "Abrir".to_string());
        m.insert("Ünï".to_string(), "çödé".to_string());
        let bytes = write_mo(&m);
        assert_eq!(parse_mo(&bytes).unwrap(), m);
        assert!(parse_mo(b"short").is_err());
        assert!(parse_mo(&[0u8; 40]).is_err());
        let mut cut = bytes.clone();
        cut.truncate(bytes.len() - 6);
        assert!(parse_mo(&cut).is_err());
        // big endian files from other tools
        let mut be = bytes.clone();
        for chunk in be[..60].chunks_mut(4) {
            chunk.reverse();
        }
        assert_eq!(parse_mo(&be).unwrap(), m);
    }

    #[test]
    #[cfg(not(feature = "reviewed-locales-only"))]
    fn a_mo_file_in_a_locale_dir_overrides_the_embedded_text() {
        let dir = std::env::temp_dir().join(format!("bookx-loc-{}", uuid::Uuid::new_v4()));
        let p = mo_path(&dir, "es");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut m = HashMap::new();
        m.insert("Library Health…".to_string(), "Salud (mo)".to_string());
        std::fs::write(&p, write_mo(&m)).unwrap();
        let loaded = load("es", std::slice::from_ref(&dir));
        assert_eq!(loaded["Library Health…"], "Salud (mo)");
        // the embedded ones are still there
        assert!(loaded.len() > 5);
        assert!(load("en", &[]).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn marked_strings_are_found() {
        let src = r#"
            let a = tr("Open");
            let b = tr("Say \"hi\"\n");
            let c = entr("no");
            let d = x.tr("no");
            let e = tr("Open");
            let f = tr("a") + tr("b");
            let g = tr(&name);
            let h = tr("with … dots");
            let i = tr(
                "Long one",
            );
            const X: &str = N_("Marked");
            let j = trf("Imported {} books", &[&n]);
            let k = tr_c("menu", "Open");
            let l = ntr("{} book", "{} books", n);
            let m = ntr("{} one", pick, n);
        "#;
        let keys: Vec<String> = extract(src).iter().map(|m| m.key()).collect();
        assert_eq!(
            keys,
            vec![
                "Open",
                "Say \"hi\"\n",
                "a",
                "b",
                "with … dots",
                "Long one",
                "Marked",
                "Imported {} books",
                "menu\u{4}Open",
                "{} book\0{} books",
            ]
        );
    }

    #[test]
    fn rust_escapes_in_marked_strings_are_decoded() {
        let src = "let a = tr(\"Back \\u{23ce}\");\nlet b = tr(\"one \\\n        two\");";
        let got: Vec<String> = extract(src).iter().map(|m| m.id.clone()).collect();
        assert_eq!(
            got,
            vec!["Back \u{23ce}".to_string(), "one two".to_string()]
        );
    }

    #[test]
    fn plural_forms_follow_the_language_rule() {
        let po = r#"
msgid ""
msgstr ""
"Plural-Forms: nplurals=2; plural=(n > 1);\n"

msgid "{} book"
msgid_plural "{} books"
msgstr[0] "{} livre"
msgstr[1] "{} livres"

msgctxt "state"
msgid "Open"
msgstr "Ouvert"
"#;
        let (header, m) = parse_po_raw(po);
        assert_eq!(PluralRule::from_header(&header), PluralRule::ZeroOneOther);
        assert_eq!(m["{} book\0{} books"], "{} livre\0{} livres");
        assert_eq!(m["state\u{4}Open"], "Ouvert");
        assert_eq!(PluralRule::ZeroOneOther.index(0), 0);
        assert_eq!(PluralRule::ZeroOneOther.index(1), 0);
        assert_eq!(PluralRule::ZeroOneOther.index(2), 1);
        assert_eq!(PluralRule::OneOther.index(0), 1);
        assert_eq!(PluralRule::OneOther.index(1), 0);
        assert_eq!(PluralRule::Single.index(7), 0);
        assert_eq!(
            PluralRule::from_header("nplurals=1; plural=0;"),
            PluralRule::Single
        );
    }

    #[test]
    fn plural_and_context_entries_survive_the_mo_format() {
        let (_, m) = parse_po_raw(
            "msgid \"a\"\nmsgid_plural \"b\"\nmsgstr[0] \"x\"\nmsgstr[1] \"y\"\n\nmsgctxt \"c\"\nmsgid \"d\"\nmsgstr \"z\"\n",
        );
        assert_eq!(parse_mo(&write_mo(&m)).unwrap(), m);
    }

    #[test]
    fn ntr_without_a_catalogue_uses_the_english_rule_and_fills_the_count() {
        // The language state is process wide, so only English-safe expectations here.
        assert_eq!(ntr("{} book", "{} books", 1), "1 book");
        assert_eq!(ntr("{} book", "{} books", 0), "0 books");
        assert_eq!(ntr("{} book", "{} books", 1234), "1,234 books");
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(fill("{} of {}", &[&3, &"nine"]), "3 of nine");
        assert_eq!(fill("{} and {}", &[&1]), "1 and {}");
        assert_eq!(fill("no holes", &[&1]), "no holes");
        assert_eq!(trf("Imported {} books", &[&5]), "Imported 5 books");
    }

    #[test]
    fn numbers_and_dates_follow_the_language() {
        assert_eq!(fmt_int_for("en", 1234567), "1,234,567");
        assert_eq!(fmt_int_for("en", -999), "-999");
        assert_eq!(fmt_int_for("de", 1234567), "1.234.567");
        assert_eq!(fmt_int_for("es", 1234), "1234");
        assert_eq!(fmt_int_for("es", 12345), "12.345");
        assert_eq!(fmt_int_for("fr", 1234567), "1\u{202f}234\u{202f}567");
        assert_eq!(fmt_int_for("hi", 1234567), "12,34,567");
        assert_eq!(fmt_int_for("hi", 999), "999");
        assert_eq!(fmt_date_for("en", 2026, 11, 3), "2026-11-03");
        assert_eq!(fmt_date_for("de", 2026, 11, 3), "03.11.2026");
        assert_eq!(fmt_date_for("fr", 2026, 11, 3), "03/11/2026");
    }

    #[test]
    fn dates_with_a_time_follow_the_language() {
        use chrono::TimeZone;
        let t = chrono::Local
            .with_ymd_and_hms(2026, 11, 3, 14, 5, 0)
            .unwrap();
        // the process language is English unless another test changed it
        assert_eq!(fmt_when(&t), "03 Nov 2026 14:05");
    }

    #[test]
    fn right_to_left_languages_are_known() {
        assert!(is_rtl("ar"));
        assert!(is_rtl("he_IL.UTF-8"));
        assert!(is_rtl("fa"));
        assert!(!is_rtl("en"));
        assert!(!is_rtl("hi"));
    }

    #[test]
    fn untranslated_text_comes_back_unchanged() {
        assert_eq!(
            tr("Nothing like this is translated"),
            "Nothing like this is translated"
        );
    }

    fn source_strings() -> Vec<Msg> {
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else if p.extension().is_some_and(|x| x == "rs") {
                        out.push(p);
                    }
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        walk(&root.join("src"), &mut files);
        files.sort();
        let mut ids: Vec<Msg> = Vec::new();
        for f in files {
            if f.ends_with("i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for s in extract(&text) {
                if !ids.contains(&s) {
                    ids.push(s);
                }
            }
        }
        ids
    }

    #[test]
    fn the_template_lists_every_marked_string() {
        let ids = source_strings();
        assert!(ids.len() > 20, "only {} strings are marked", ids.len());
        let want = pot_text(&ids);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("po/bookx.pot");
        if std::env::var_os("BOOKX_UPDATE_POT").is_some() {
            std::fs::write(&path, &want).unwrap();
        }
        let have = std::fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            have, want,
            "po/bookx.pot is out of date: run build-aux/i18n.sh"
        );
    }

    fn reviewed_codes() -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("po/reviewed-locales.txt");
        std::fs::read_to_string(path)
            .expect("po/reviewed-locales.txt")
            .lines()
            .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    #[test]
    fn reviewed_locales_exist_and_are_complete() {
        let keys: Vec<String> = source_strings().iter().map(|m| m.key()).collect();
        let po_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("po");
        for code in reviewed_codes() {
            assert!(
                code != "en" && LANGUAGES_ALL.contains(&code.as_str()),
                "po/reviewed-locales.txt lists unknown language {code}"
            );
            let text = std::fs::read_to_string(po_dir.join(format!("{code}.po")))
                .unwrap_or_else(|_| panic!("po/{code}.po is missing"));
            let (_, m) = parse_po_raw(&text);
            let missing: Vec<&String> = keys.iter().filter(|k| !m.contains_key(*k)).collect();
            assert!(
                missing.is_empty(),
                "{code} is listed as reviewed but lacks {} strings",
                missing.len()
            );
        }
    }

    /// Every `po/LINGUAS` code is known to the build (so a new language is added to build.rs too).
    const LANGUAGES_ALL: [&str; 4] = ["es", "de", "fr", "hi"];

    #[test]
    fn build_table_covers_the_linguas_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("po/LINGUAS");
        let text = std::fs::read_to_string(path).unwrap();
        let mut codes: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        codes.sort_unstable();
        let mut all = LANGUAGES_ALL.to_vec();
        all.sort_unstable();
        assert_eq!(codes, all, "po/LINGUAS and the table in build.rs differ");
    }

    #[test]
    fn the_language_table_follows_the_gate() {
        let embedded: Vec<&str> = LANGUAGES.iter().map(|(c, _, _)| *c).collect();
        assert_eq!(embedded[0], "en");
        if REVIEWED_ONLY {
            let mut want = vec!["en".to_string()];
            want.extend(reviewed_codes().into_iter().filter(|c| c != "en"));
            let mut have: Vec<String> = embedded.iter().map(|s| s.to_string()).collect();
            want.sort();
            have.sort();
            assert_eq!(have, want, "only reviewed languages may be embedded");
            // an unreviewed language is neither listed nor translated
            for code in LANGUAGES_ALL {
                if !want.iter().any(|w| w == code) {
                    assert!(!available().iter().any(|(c, _)| c == code) || locale_dirs_have(code));
                    assert!(
                        load(code, &[]).is_empty(),
                        "{code} must not be embedded in the reviewed-only build"
                    );
                }
            }
        } else {
            for code in LANGUAGES_ALL {
                assert!(
                    embedded.contains(&code),
                    "{code} missing from the full build"
                );
            }
        }
    }

    /// The Flathub build: an unreviewed language is not selectable, so the setting and the system
    /// language fall back to English, and nothing is embedded for it.
    #[test]
    #[cfg(feature = "reviewed-locales-only")]
    fn an_unreviewed_language_falls_back_to_english() {
        let e = env(&[("LANG", "de_DE.UTF-8")]);
        for code in LANGUAGES_ALL {
            if reviewed_codes().iter().any(|r| r == code) {
                continue;
            }
            assert_eq!(resolve(code, &e), "en");
        }
        assert_eq!(resolve("", &env(&[("LANGUAGE", "xx:fr:en")])), "en");
        let dir = std::env::temp_dir().join(format!("bookx-loc-{}", uuid::Uuid::new_v4()));
        assert!(load("es", std::slice::from_ref(&dir)).is_empty());
    }

    /// Flathub refuses machine translations: the desktop file and the metainfo carry English text only
    /// (a translated key would need the same review as the interface languages).
    #[test]
    fn desktop_and_metainfo_have_no_translated_text() {
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        let desktop = std::fs::read_to_string(data.join("com.adhadse.Bookx.desktop")).unwrap();
        let bad: Vec<&str> = desktop
            .lines()
            .filter(|l| !l.starts_with('[') && l.split('=').next().is_some_and(|k| k.contains('[')))
            .collect();
        assert!(bad.is_empty(), "translated desktop keys: {bad:?}");
        let metainfo =
            std::fs::read_to_string(data.join("com.adhadse.Bookx.metainfo.xml")).unwrap();
        assert!(
            !metainfo.contains("xml:lang"),
            "the metainfo has translated text (xml:lang)"
        );
    }

    fn locale_dirs_have(code: &str) -> bool {
        locale_dirs().iter().any(|d| mo_path(d, code).exists())
    }

    #[test]
    fn every_shipped_language_translates_every_marked_string() {
        let ids = source_strings();
        let keys: Vec<String> = ids.iter().map(|m| m.key()).collect();
        for (code, _, po) in LANGUAGES.iter() {
            let Some(po) = po else { continue };
            let (header, m) = parse_po_raw(po);
            assert!(
                header.contains("Plural-Forms"),
                "{code} has no Plural-Forms"
            );
            let missing: Vec<&String> = keys.iter().filter(|k| !m.contains_key(*k)).collect();
            // French and Hindi are starting files: the most visible strings only.
            if matches!(*code, "fr" | "hi") {
                assert!(m.len() >= 150, "{code} has only {} strings", m.len());
            } else {
                assert!(missing.is_empty(), "{code} lacks: {missing:?}");
            }
            let stale: Vec<&String> = m.keys().filter(|k| !keys.contains(k)).collect();
            assert!(
                stale.is_empty(),
                "{code} has strings the program no longer uses: {stale:?}"
            );
            let forms = if header.replace(' ', "").contains("nplurals=1") {
                1
            } else {
                2
            };
            for (k, v) in m.iter().filter(|(k, _)| k.contains('\0')) {
                assert_eq!(
                    v.split('\0').count(),
                    forms,
                    "{code}: wrong number of plural forms for {k:?}"
                );
                assert!(
                    v.split('\0').all(|f| f.contains("{}")),
                    "{code}: a plural form lost its count in {k:?}"
                );
            }
            for (k, v) in m.iter().filter(|(k, _)| !k.contains('\0')) {
                let id = k.rsplit('\u{4}').next().unwrap_or(k);
                assert_eq!(
                    id.matches("{}").count(),
                    v.matches("{}").count(),
                    "{code}: placeholder mismatch in {k:?}"
                );
            }
        }
    }
}
