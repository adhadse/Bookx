//! Decorations of the code view: invalid markup underlined in red (CE-336), link targets underlined and missing
//! ones in red (CE-339), bold/italic text drawn bold/italic (CE-333) and colour swatches behind CSS colours
//! (CE-356). The scans are `bookx_editing::code_marks`; this file turns the byte ranges into text tags, skips
//! the work when nothing changed (no flicker, no repeated scans while scrolling) and answers the tooltips.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::code_marks::{self as marks, ErrorKind};
use serde::{Deserialize, Serialize};

pub const TAG_INVALID: &str = "markup-error";
pub const TAG_LINK: &str = "link-target";
pub const TAG_LINK_BAD: &str = "link-missing";
pub const TAG_BOLD: &str = "code-bold";
pub const TAG_ITALIC: &str = "code-italic";
const SWATCH_PREFIX: &str = "swatch-";
/// Most distinct swatch tags one buffer carries.
const MAX_SWATCH_TAGS: usize = 300;
/// Largest file the decorations are computed for (bytes).
const HINT_MAX: usize = 1_500_000;

/// What the user can switch on and off (View menu); kept in `editor-markup.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HintPrefs {
    /// Typing `</` closes the innermost open tag (CE-348).
    pub close_on_slash: bool,
    pub invalid_markup: bool,
    pub link_targets: bool,
    pub color_swatches: bool,
    /// Text inside `<b>`, `<i>`, headings... is drawn bold/italic (CE-333).
    pub emphasis: bool,
    /// Colours and alignment are written as CSS classes (rules added to the style sheet), not `style` attributes.
    pub colour_classes: bool,
}

impl Default for HintPrefs {
    fn default() -> Self {
        HintPrefs {
            close_on_slash: true,
            invalid_markup: true,
            link_targets: true,
            color_swatches: true,
            emphasis: false,
            colour_classes: false,
        }
    }
}

impl HintPrefs {
    pub fn path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("com", "adhadse", "Bookx")
            .map(|d| d.config_dir().join("editor-markup.json"))
    }

    pub fn load_from(path: &std::path::Path) -> HintPrefs {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(
            path,
            serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into()),
        )
    }

    pub fn get(&self, o: HintOpt) -> bool {
        match o {
            HintOpt::AutoClose => self.close_on_slash,
            HintOpt::Invalid => self.invalid_markup,
            HintOpt::Links => self.link_targets,
            HintOpt::Colors => self.color_swatches,
            HintOpt::Emphasis => self.emphasis,
            HintOpt::ColourClasses => self.colour_classes,
        }
    }

    pub fn set(&mut self, o: HintOpt, on: bool) {
        match o {
            HintOpt::AutoClose => self.close_on_slash = on,
            HintOpt::Invalid => self.invalid_markup = on,
            HintOpt::Links => self.link_targets = on,
            HintOpt::Colors => self.color_swatches = on,
            HintOpt::Emphasis => self.emphasis = on,
            HintOpt::ColourClasses => self.colour_classes = on,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintOpt {
    AutoClose,
    Invalid,
    Links,
    Colors,
    Emphasis,
    ColourClasses,
}

impl HintOpt {
    pub const ALL: [(HintOpt, &'static str); 6] = [
        (HintOpt::AutoClose, "auto-close"),
        (HintOpt::Invalid, "invalid"),
        (HintOpt::Links, "links"),
        (HintOpt::Colors, "colors"),
        (HintOpt::Emphasis, "emphasis"),
        (HintOpt::ColourClasses, "colour-classes"),
    ];

    pub fn from_name(n: &str) -> Option<HintOpt> {
        HintOpt::ALL.iter().find(|(_, s)| *s == n).map(|(o, _)| *o)
    }
}

/// What was applied to one buffer the last time (to skip identical passes and avoid flicker).
#[derive(Default)]
struct Applied {
    fingerprint: u64,
    spans: HashMap<&'static str, Vec<(i32, i32)>>,
    swatches: Vec<(i32, i32, String)>,
    swatch_tags: HashSet<String>,
}

#[derive(Default)]
struct Tips {
    path: String,
    list: Vec<(i32, i32, String)>,
}

/// Shared state of the decorations.
pub struct Hints {
    pub prefs: RefCell<HintPrefs>,
    applied: RefCell<HashMap<String, Applied>>,
    tips: RefCell<Tips>,
    /// Counts of the last pass, for the dev hook.
    pub last: RefCell<String>,
    /// The rest of a tag that `</` completed (typed characters that match it are swallowed).
    pub pending: RefCell<Option<bookx_editing::code_format::Pending>>,
}

impl Hints {
    pub fn new() -> Rc<Hints> {
        Rc::new(Hints {
            prefs: RefCell::new(
                HintPrefs::path()
                    .map(|p| HintPrefs::load_from(&p))
                    .unwrap_or_default(),
            ),
            applied: RefCell::default(),
            tips: RefCell::default(),
            last: RefCell::default(),
            pending: RefCell::default(),
        })
    }

    fn save(&self) {
        if let Some(p) = HintPrefs::path() {
            let _ = self.prefs.borrow().save_to(&p);
        }
    }
}

// ---- tags ---------------------------------------------------------------------------------

pub fn add_tags(table: &gtk::TextTagTable) {
    let mk = |name: &str, f: &dyn Fn(&gtk::TextTag)| {
        if table.lookup(name).is_none() {
            let t = gtk::TextTag::new(Some(name));
            f(&t);
            table.add(&t);
        }
    };
    mk(TAG_INVALID, &|t| {
        t.set_underline(gtk::pango::Underline::Error)
    });
    mk(TAG_LINK, &|t| {
        t.set_underline(gtk::pango::Underline::Single)
    });
    mk(TAG_LINK_BAD, &|t| {
        t.set_underline(gtk::pango::Underline::Single)
    });
    mk(TAG_BOLD, &|t| t.set_weight(700));
    mk(TAG_ITALIC, &|t| t.set_style(gtk::pango::Style::Italic));
}

pub fn recolor(table: &gtk::TextTagTable, dark: bool) {
    let rgba = |s: &str| gtk::gdk::RGBA::parse(s).ok();
    if let Some(t) = table.lookup(TAG_INVALID) {
        t.set_underline_rgba(rgba(if dark { "#ff7b63" } else { "#e01b24" }).as_ref());
    }
    if let Some(t) = table.lookup(TAG_LINK) {
        t.set_underline_rgba(rgba(if dark { "#78aeed" } else { "#1c71d8" }).as_ref());
    }
    if let Some(t) = table.lookup(TAG_LINK_BAD) {
        t.set_underline_rgba(rgba(if dark { "#ff7b63" } else { "#e01b24" }).as_ref());
    }
}

// ---- offsets ------------------------------------------------------------------------------

/// Byte offset to character offset in O(1) for ASCII text and O(log n) otherwise.
struct CharIndex<'a> {
    text: &'a str,
    marks: Option<Vec<(usize, usize)>>,
}

impl<'a> CharIndex<'a> {
    fn new(text: &'a str) -> Self {
        if text.is_ascii() {
            return CharIndex { text, marks: None };
        }
        let mut marks = vec![(0usize, 0usize)];
        for (n, (b, _)) in text.char_indices().enumerate() {
            if n > 0 && n % 2048 == 0 {
                marks.push((b, n));
            }
        }
        CharIndex {
            text,
            marks: Some(marks),
        }
    }

    fn at(&self, mut byte: usize) -> i32 {
        byte = byte.min(self.text.len());
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        let Some(m) = &self.marks else {
            return byte as i32;
        };
        let i = m.partition_point(|(b, _)| *b <= byte).saturating_sub(1);
        let (mb, mc) = m[i];
        (mc + self.text[mb..byte].chars().count()) as i32
    }
}

fn fingerprint(text: &str, prefs: &HintPrefs, extra: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    prefs.hash_bits().hash(&mut h);
    extra.hash(&mut h);
    h.finish()
}

impl HintPrefs {
    fn hash_bits(&self) -> u8 {
        (self.close_on_slash as u8)
            | (self.invalid_markup as u8) << 1
            | (self.link_targets as u8) << 2
            | (self.color_swatches as u8) << 3
            | (self.emphasis as u8) << 4
            | (self.colour_classes as u8) << 5
    }
}

fn describe(kind: &ErrorKind) -> String {
    match kind {
        ErrorKind::Unclosed(n) => trf("<{}> is never closed", &[n]),
        ErrorKind::UnexpectedClose(n) => trf("</{}> has no matching start tag", &[n]),
        ErrorKind::UnfinishedTag => tr("This tag has no closing >"),
        ErrorKind::DuplicateAttribute(n) => trf("The attribute {} appears twice", &[n]),
        ErrorKind::UnquotedAttribute(n) => trf("The value of {} needs quotes", &[n]),
        ErrorKind::BareAmpersand => tr("& must be written as &amp;"),
        ErrorKind::StrayBrace => tr("} without a matching {"),
        ErrorKind::UnclosedBrace => tr("{ is never closed"),
        ErrorKind::UnclosedComment => tr("This comment is never closed"),
    }
}

fn is_xml_strict(path: &str) -> Option<bool> {
    match extension(path).as_str() {
        "xhtml" | "xht" | "ncx" | "opf" | "xml" | "svg" => Some(true),
        "html" | "htm" => Some(false),
        _ => None,
    }
}

impl Editor {
    /// Recomputes the decorations of the open file (called by the live pass).
    pub(super) fn update_hints(&mut self, path: &str, buf: &sv::Buffer, text: &str) {
        let hints = self.hints.clone();
        let prefs = hints.prefs.borrow().clone();
        let css = extension(path) == "css";
        let markup = is_xml_strict(path).is_some();
        let mut applied_map = hints.applied.borrow_mut();
        let ap = applied_map.entry(path.to_string()).or_default();
        // a buffer with one huge line (no colouring) gets no hints either: every tag applied to such a line
        // splits it again, 20 000 links take half a minute
        if text.len() > HINT_MAX || (!css && !markup) || super::codeview::is_plain(buf) {
            clear_all(buf, ap);
            hints.tips.borrow_mut().list.clear();
            *hints.last.borrow_mut() = "skipped".into();
            return;
        }
        let files_fp = self.book.borrow().archive.names().len() as u64;
        let fp = fingerprint(text, &prefs, files_fp);
        hints.tips.borrow_mut().path = path.to_string();
        if ap.fingerprint == fp {
            return;
        }
        ap.fingerprint = fp;
        let ci = CharIndex::new(text);
        let mut tips: Vec<(i32, i32, String)> = Vec::new();

        // invalid markup
        let mut invalid: Vec<(i32, i32)> = Vec::new();
        if prefs.invalid_markup {
            let errs = if css {
                marks::css_errors(text)
            } else {
                marks::markup_errors(text, is_xml_strict(path).unwrap_or(false))
            };
            for e in errs {
                let (a, b) = (
                    ci.at(e.range.start),
                    ci.at(e.range.end).max(ci.at(e.range.start) + 1),
                );
                invalid.push((a, b));
                tips.push((a, b, describe(&e.kind)));
            }
        }
        // link targets
        let (mut ok, mut bad): (Vec<(i32, i32)>, Vec<(i32, i32)>) = (Vec::new(), Vec::new());
        if prefs.link_targets {
            let book = self.book.borrow();
            let comp = self.comp.clone();
            let id_cache: RefCell<HashMap<String, Vec<String>>> = RefCell::default();
            let exists = |p: &str| book.archive.contains(p);
            let anchor = |file: &str, frag: &str| -> Option<bool> {
                if !bookx_editing::markup::is_markup_path(file) {
                    return None;
                }
                let mut c = id_cache.borrow_mut();
                let ids = c
                    .entry(file.to_string())
                    .or_insert_with(|| comp.borrow().ids_of(file));
                Some(ids.iter().any(|i| i == frag))
            };
            let marksv = marks::link_marks(text, path, css, &exists, &anchor);
            for m in marksv {
                let (a, b) = (ci.at(m.range.start), ci.at(m.range.end));
                if m.ok {
                    ok.push((a, b));
                } else {
                    bad.push((a, b));
                    let target = text[m.range.clone()].to_string();
                    tips.push((a, b, trf("There is no such file or anchor: {}", &[&target])));
                }
            }
        }
        // bold / italic
        let (mut bold, mut italic): (Vec<(i32, i32)>, Vec<(i32, i32)>) = (Vec::new(), Vec::new());
        if prefs.emphasis && !css {
            for (r, e) in marks::emphasis_spans(text) {
                let s = (ci.at(r.start), ci.at(r.end));
                if e.bold {
                    bold.push(s);
                }
                if e.italic {
                    italic.push(s);
                }
            }
        }
        // colour swatches
        let mut swatches: Vec<(i32, i32, String)> = Vec::new();
        let mut tag_specs: Vec<(String, (u8, u8, u8), (u8, u8, u8))> = Vec::new();
        if prefs.color_swatches {
            for c in marks::css_colors(text, css) {
                let name = format!(
                    "{SWATCH_PREFIX}{:02x}{:02x}{:02x}{:02x}",
                    c.rgba.0, c.rgba.1, c.rgba.2, c.rgba.3
                );
                if !tag_specs.iter().any(|t| t.0 == name) {
                    if tag_specs.len() >= MAX_SWATCH_TAGS {
                        continue;
                    }
                    let a = c.rgba.3 as u32;
                    let mix = |v: u8| ((v as u32 * a + 255 * (255 - a)) / 255) as u8;
                    tag_specs.push((
                        name.clone(),
                        (mix(c.rgba.0), mix(c.rgba.1), mix(c.rgba.2)),
                        c.text,
                    ));
                }
                swatches.push((ci.at(c.start), ci.at(c.end), name));
            }
        }

        let counts = format!(
            "invalid={} link_ok={} link_missing={} colors={} bold={} italic={}",
            invalid.len(),
            ok.len(),
            bad.len(),
            swatches.len(),
            bold.len(),
            italic.len()
        );
        for (tag, spans) in [
            (TAG_INVALID, invalid),
            (TAG_LINK, ok),
            (TAG_LINK_BAD, bad),
            (TAG_BOLD, bold),
            (TAG_ITALIC, italic),
        ] {
            if ap.spans.get(tag) != Some(&spans) {
                let (s, e) = buf.bounds();
                buf.remove_tag_by_name(tag, &s, &e);
                for (a, b) in &spans {
                    buf.apply_tag_by_name(tag, &buf.iter_at_offset(*a), &buf.iter_at_offset(*b));
                }
                ap.spans.insert(tag, spans);
            }
        }
        if ap.swatches != swatches {
            let (s, e) = buf.bounds();
            for t in ap.swatch_tags.drain() {
                buf.remove_tag_by_name(&t, &s, &e);
            }
            let table = buf.tag_table();
            for (name, bg, fg) in &tag_specs {
                if table.lookup(name).is_none() {
                    let t = gtk::TextTag::new(Some(name));
                    t.set_background_rgba(Some(&gtk::gdk::RGBA::new(
                        bg.0 as f32 / 255.0,
                        bg.1 as f32 / 255.0,
                        bg.2 as f32 / 255.0,
                        1.0,
                    )));
                    t.set_foreground_rgba(Some(&gtk::gdk::RGBA::new(
                        fg.0 as f32 / 255.0,
                        fg.1 as f32 / 255.0,
                        fg.2 as f32 / 255.0,
                        1.0,
                    )));
                    table.add(&t);
                }
            }
            for (a, b, name) in &swatches {
                buf.apply_tag_by_name(name, &buf.iter_at_offset(*a), &buf.iter_at_offset(*b));
                ap.swatch_tags.insert(name.clone());
            }
            ap.swatches = swatches;
        }
        hints.tips.borrow_mut().list = tips;
        *hints.last.borrow_mut() = counts;
    }

    pub(super) fn set_hint_option(&mut self, opt: HintOpt, on: bool) {
        self.hints.prefs.borrow_mut().set(opt, on);
        self.hints.save();
        self.run_live();
    }
}

fn clear_all(buf: &sv::Buffer, ap: &mut Applied) {
    if ap.spans.values().all(|v| v.is_empty()) && ap.swatches.is_empty() {
        ap.fingerprint = 0;
        return;
    }
    let (s, e) = buf.bounds();
    for tag in [TAG_INVALID, TAG_LINK, TAG_LINK_BAD, TAG_BOLD, TAG_ITALIC] {
        buf.remove_tag_by_name(tag, &s, &e);
    }
    for t in ap.swatch_tags.drain() {
        buf.remove_tag_by_name(&t, &s, &e);
    }
    ap.spans.clear();
    ap.swatches.clear();
    ap.fingerprint = 0;
}

/// Tooltips with the explanation of an underline.
pub(super) fn install_tooltips(view: &sv::View, hints: Rc<Hints>) {
    view.set_has_tooltip(true);
    view.connect_query_tooltip(move |v, x, y, _kb, tip| {
        let (bx, by) = v.window_to_buffer_coords(gtk::TextWindowType::Widget, x, y);
        let Some(it) = v.iter_at_location(bx, by) else {
            return false;
        };
        let off = it.offset();
        let t = hints.tips.borrow();
        match t.list.iter().find(|(a, b, _)| off >= *a && off < *b) {
            Some((_, _, msg)) => {
                tip.set_text(Some(msg));
                true
            }
            None => false,
        }
    });
}

/// The `hint-*` toggles of the View menu.
pub(super) fn view_actions(
    group: &gio::SimpleActionGroup,
    tx: &relm4::Sender<Input>,
    hints: &Hints,
) {
    for (opt, name) in HintOpt::ALL {
        let on = hints.prefs.borrow().get(opt);
        let act = gio::SimpleAction::new_stateful(&format!("hint-{name}"), None, &on.to_variant());
        let tx = tx.clone();
        act.connect_activate(move |a, _| {
            let now = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&now.to_variant());
            tx.emit(Input::Code(super::format_actions::CodeMsg::Hint(opt, now)));
        });
        group.add_action(&act);
    }
}

pub(super) fn append_view_menu(menu: &gio::Menu) {
    menu.append(
        Some(&tr("Close Tags When Typing </")),
        Some("view.hint-auto-close"),
    );
    menu.append(
        Some(&tr("Underline Invalid Markup")),
        Some("view.hint-invalid"),
    );
    menu.append(Some(&tr("Mark Link Targets")), Some("view.hint-links"));
    menu.append(Some(&tr("Show Colour Swatches")), Some("view.hint-colors"));
    menu.append(
        Some(&tr("Draw Bold and Italic Text")),
        Some("view.hint-emphasis"),
    );
    menu.append(
        Some(&tr("Use CSS Classes for Colours and Alignment")),
        Some("view.hint-colour-classes"),
    );
}

impl Editor {
    /// Dev hook output of the last pass.
    pub(super) fn dev_hints(&mut self, c: &str) -> bool {
        match c {
            "hints" => {
                self.run_live();
                eprintln!("editor_code: hints {}", self.hints.last.borrow());
                true
            }
            "hints-tags" => {
                let buf = self.w.view.buffer();
                for t in [TAG_INVALID, TAG_LINK, TAG_LINK_BAD, TAG_BOLD, TAG_ITALIC] {
                    eprintln!("editor_code: tag {t} {}", self.tag_segments(t));
                }
                let table = buf.tag_table();
                let mut sw = Vec::new();
                table.foreach(|t| {
                    if let Some(n) = t.name().filter(|n| n.starts_with(SWATCH_PREFIX)) {
                        sw.push(n.to_string());
                    }
                });
                sw.sort();
                for n in sw {
                    eprintln!("editor_code: swatch {n} {}", self.tag_segments(&n));
                }
                true
            }
            _ => {
                if let Some(r) = c.strip_prefix("hint-opt=") {
                    if let Some((n, v)) = r.split_once(':') {
                        if let Some(o) = HintOpt::from_name(n) {
                            self.set_hint_option(o, v == "on");
                        }
                    }
                    return true;
                }
                if let Some(r) = c.strip_prefix("tooltip=") {
                    let off: i32 = self.dev_offset(r).map(|o| o.0).unwrap_or(-1);
                    let t = self.hints.tips.borrow();
                    let msg = t
                        .list
                        .iter()
                        .find(|(a, b, _)| off >= *a && off < *b)
                        .map(|x| x.2.clone());
                    eprintln!("editor_code: tooltip {off} {}", msg.unwrap_or_default());
                    return true;
                }
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_defaults_round_trip_and_damage() {
        let dir = std::env::temp_dir().join(format!("bookx-hints-{}", uuid::Uuid::new_v4()));
        let f = dir.join("editor-markup.json");
        let d = HintPrefs::load_from(&f);
        assert!(d.close_on_slash && d.invalid_markup && d.link_targets && d.color_swatches);
        assert!(!d.emphasis, "bold/italic rendering is opt in");
        let mut p = d.clone();
        p.set(HintOpt::Emphasis, true);
        p.set(HintOpt::AutoClose, false);
        p.save_to(&f).unwrap();
        assert_eq!(HintPrefs::load_from(&f), p);
        std::fs::write(&f, "{\"emphasis\": true}").unwrap();
        let q = HintPrefs::load_from(&f);
        assert!(
            q.emphasis && q.close_on_slash,
            "missing keys keep their defaults"
        );
        std::fs::write(&f, "junk").unwrap();
        assert_eq!(HintPrefs::load_from(&f), d);
        for (o, n) in HintOpt::ALL {
            assert_eq!(HintOpt::from_name(n), Some(o));
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn char_index_maps_bytes_to_chars() {
        let ascii = "abc<p>";
        assert_eq!(CharIndex::new(ascii).at(4), 4);
        let mut s = String::new();
        for i in 0..5000 {
            s.push_str(if i % 7 == 0 { "é" } else { "x" });
        }
        let ci = CharIndex::new(&s);
        for (n, (b, _)) in s.char_indices().enumerate().step_by(97) {
            assert_eq!(ci.at(b), n as i32);
        }
        assert_eq!(ci.at(s.len()), 5000);
        assert_eq!(ci.at(s.len() + 10), 5000);
        assert_eq!(ci.at(1), 0, "inside a multi-byte character rounds down");
    }
}
