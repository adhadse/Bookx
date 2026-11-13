//! The editor's code widget: a GtkSourceView `View`/`Buffer` pair and the few decisions around it.
//! Everything that needs no GTK is a plain function with a unit test (which language and style
//! scheme a file gets, which characters are hard to see); the rest builds and configures widgets.

use crate::i18n::tr;
use relm4::gtk::{self, prelude::*};
use sourceview5 as sv;
use sv::prelude::*;

use crate::ui::syntax::{self, Lang};

// ---- decisions (no GTK) ----------------------------------------------------

/// The GtkSourceView language id for a file, `None` when it has no syntax worth highlighting.
pub fn language_id(path: &str) -> Option<&'static str> {
    match crate::core::util::extension(path).as_str() {
        "xhtml" | "html" | "htm" | "xht" => Some("html"),
        "xml" | "opf" | "ncx" | "svg" | "smil" => Some("xml"),
        "css" => Some("css"),
        "js" => Some("js"),
        "md" => Some("markdown"),
        _ => None,
    }
}

/// Style scheme ids to try, best first.
pub fn scheme_candidates(dark: bool, eink: bool) -> &'static [&'static str] {
    if eink {
        &["classic", "Adwaita", "kate", "tango"]
    } else if dark {
        &["Adwaita-dark", "oblivion", "solarized-dark", "cobalt"]
    } else {
        &["Adwaita", "classic", "tango", "kate"]
    }
}

/// The first candidate that exists among `available`; else any scheme that matches the brightness
/// by name; else the first available.
pub fn pick_scheme(available: &[String], dark: bool, eink: bool) -> Option<String> {
    scheme_candidates(dark, eink)
        .iter()
        .find_map(|c| available.iter().find(|a| a.as_str() == *c).cloned())
        .or_else(|| {
            available
                .iter()
                .find(|a| a.to_lowercase().contains("dark") == (dark && !eink))
                .cloned()
        })
        .or_else(|| available.first().cloned())
}

/// Character ranges (offsets in chars) of invisible or look-alike characters that are easy to
/// miss in markup: no-break space, zero-width characters, soft hyphen, BOM, line/paragraph
/// separators (CE-338).
pub fn hidden_chars(text: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (i, c) in text.chars().enumerate() {
        let hard = matches!(
            c,
            '\u{00a0}'
                | '\u{00ad}'
                | '\u{034f}'
                | '\u{061c}'
                | '\u{180e}'
                | '\u{2000}'..='\u{200f}'
                | '\u{2028}'..='\u{202f}'
                | '\u{205f}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{3000}'
                | '\u{feff}'
        );
        if hard {
            match out.last_mut() {
                Some(l) if l.1 == i => l.1 = i + 1,
                _ => out.push((i, i + 1)),
            }
        }
    }
    out
}

/// A human name for one hard-to-see character, for the status line and tests.
pub fn describe_hidden(c: char) -> &'static str {
    match c {
        '\u{00a0}' => "no-break space",
        '\u{00ad}' => "soft hyphen",
        '\u{200b}' => "zero-width space",
        '\u{200c}' => "zero-width non-joiner",
        '\u{200d}' => "zero-width joiner",
        '\u{200e}' | '\u{200f}' => "direction mark",
        '\u{2028}' => "line separator",
        '\u{2029}' => "paragraph separator",
        '\u{3000}' => "ideographic space",
        '\u{feff}' => "byte order mark",
        _ => "invisible character",
    }
}

/// The text handed to GtkSourceView's search: its regex flavour (PCRE) has no dot-all switch in the
/// settings, so the inline flag is put in front.
pub fn search_text_for_context(query: &str, regex: bool, dot_all: bool) -> String {
    if regex && dot_all && !query.is_empty() {
        format!("(?s){query}")
    } else {
        query.to_string()
    }
}

/// The status text for the matches in the open file, from the search context's numbers
/// (`count < 0` while it is still counting; `position` 0 = the selection is not a match).
pub fn find_status(count: i32, position: i32) -> String {
    match (count, position) {
        (c, _) if c < 0 => "Counting…".into(),
        (0, _) => "No matches in this file".into(),
        (c, p) if p > 0 => format!("{p} of {c} in this file"),
        (c, _) => format!("{c} in this file"),
    }
}

// ---- widgets -----------------------------------------------------------------

pub fn init() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(sv::init);
}

/// Names of the extra text tags every buffer carries.
pub const TAG_SPELL: &str = "spell-error";
pub const TAG_PAIR: &str = "tag-pair";
pub const TAG_HIDDEN: &str = "hidden-char";

/// A buffer with its own tag table: the marked-region tag, the spelling underline, the tag-pair
/// highlight and the fallback highlighter's tags (used when no GtkSourceView language exists).
pub fn new_buffer(dark: bool) -> sv::Buffer {
    init();
    let table = syntax::make_tag_table(dark);
    add_extra_tags(&table, dark);
    let buf = sv::Buffer::new(Some(&table));
    buf.set_highlight_matching_brackets(true);
    buf.set_implicit_trailing_newline(false);
    buf
}

fn add_extra_tags(table: &gtk::TextTagTable, dark: bool) {
    if table.lookup(TAG_SPELL).is_none() {
        let t = gtk::TextTag::new(Some(TAG_SPELL));
        t.set_underline(gtk::pango::Underline::Error);
        table.add(&t);
    }
    if table.lookup(TAG_PAIR).is_none() {
        let t = gtk::TextTag::new(Some(TAG_PAIR));
        table.add(&t);
    }
    if table.lookup(TAG_HIDDEN).is_none() {
        let t = gtk::TextTag::new(Some(TAG_HIDDEN));
        table.add(&t);
    }
    recolor(table, dark);
}

/// Colours of the tags that depend on the brightness.
pub fn recolor(table: &gtk::TextTagTable, dark: bool) {
    syntax::update_colors(table, dark);
    if let Some(t) = table.lookup(TAG_SPELL) {
        t.set_underline_rgba(
            gtk::gdk::RGBA::parse(if dark { "#ff7b63" } else { "#e01b24" })
                .ok()
                .as_ref(),
        );
    }
    if let Some(t) = table.lookup(TAG_PAIR) {
        t.set_background(Some(if dark {
            "rgba(120,174,237,0.30)"
        } else {
            "rgba(53,132,228,0.22)"
        }));
    }
    if let Some(t) = table.lookup(TAG_HIDDEN) {
        t.set_background(Some(if dark {
            "rgba(249,240,107,0.35)"
        } else {
            "rgba(246,211,45,0.55)"
        }));
    }
}

/// The code view with the editor's behaviour: line numbers, current-line highlight, auto-indent,
/// indent on Tab, smart Home/End, no right margin guide.
pub fn new_view() -> sv::View {
    init();
    let view = sv::View::new();
    crate::ui::audit::label(&view, &tr("Book source editor"));
    view.add_css_class("code-view");
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::None);
    view.set_top_margin(8);
    view.set_bottom_margin(200);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_accepts_tab(true);
    view.set_show_line_numbers(true);
    view.set_highlight_current_line(true);
    view.set_auto_indent(true);
    view.set_indent_on_tab(true);
    view.set_insert_spaces_instead_of_tabs(true);
    view.set_tab_width(2);
    view.set_indent_width(2);
    view.set_smart_home_end(sv::SmartHomeEndType::Before);
    view.set_show_right_margin(false);
    view.set_enable_snippets(true);
    view
}

/// The style scheme ids this installation has.
pub fn available_schemes() -> Vec<String> {
    sv::StyleSchemeManager::default()
        .scheme_ids()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

/// Applies the scheme that matches the app's brightness and e-ink mode.
pub fn apply_scheme(buf: &sv::Buffer, dark: bool, eink: bool) {
    let mgr = sv::StyleSchemeManager::default();
    let id = pick_scheme(&available_schemes(), dark, eink);
    buf.set_style_scheme(id.and_then(|i| mgr.scheme(&i)).as_ref());
}

/// Sets the highlighting language of the buffer from the file name. Returns `false` when
/// GtkSourceView has no language for it; the caller then falls back to the built-in highlighter.
pub fn apply_language(buf: &sv::Buffer, path: &str) -> bool {
    let lang = language_id(path).and_then(|id| sv::LanguageManager::default().language(id));
    match lang {
        Some(l) => {
            buf.set_language(Some(&l));
            buf.set_highlight_syntax(true);
            // the old highlighter's tags would paint over the real ones
            let (s, e) = buf.bounds();
            for t in syntax::TAGS {
                buf.remove_tag_by_name(t, &s, &e);
            }
            true
        }
        None => {
            buf.set_language(None::<&sv::Language>);
            false
        }
    }
}

/// Re-runs the fallback highlighter when the buffer has no GtkSourceView language.
pub fn fallback_highlight(buf: &sv::Buffer, path: &str) {
    if buf.language().is_none() {
        syntax::apply(buf.upcast_ref(), Lang::for_path(path));
    }
}

/// Draws (or hides) spaces, tabs, line breaks and no-break spaces.
pub fn set_whitespace(view: &sv::View, on: bool) {
    let drawer = view.space_drawer();
    drawer.set_enable_matrix(true);
    drawer.set_types_for_locations(
        sv::SpaceLocationFlags::ALL,
        if on {
            sv::SpaceTypeFlags::ALL
        } else {
            sv::SpaceTypeFlags::NONE
        },
    );
    view.queue_draw();
}

/// Whether whitespace drawing is on.
pub fn whitespace_on(view: &sv::View) -> bool {
    view.space_drawer()
        .types_for_locations(sv::SpaceLocationFlags::ALL)
        != sv::SpaceTypeFlags::NONE
}

/// The current style scheme id of a buffer (for the dev hook).
pub fn scheme_id(buf: &sv::Buffer) -> String {
    buf.style_scheme()
        .map(|s| s.id().to_string())
        .unwrap_or_default()
}

/// Highlights the hard-to-see characters of the buffer, or clears the highlight.
pub fn mark_hidden_chars(buf: &gtk::TextBuffer, on: bool) {
    let (s, e) = buf.bounds();
    buf.remove_tag_by_name(TAG_HIDDEN, &s, &e);
    if !on {
        return;
    }
    let text = buf.text(&s, &e, false);
    if text.len() > 2_000_000 {
        return;
    }
    for (a, b) in hidden_chars(&text) {
        buf.apply_tag_by_name(
            TAG_HIDDEN,
            &buf.iter_at_offset(a as i32),
            &buf.iter_at_offset(b as i32),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_a_language() {
        assert_eq!(language_id("OEBPS/ch1.xhtml"), Some("html"));
        assert_eq!(language_id("a.HTML"), Some("html"));
        assert_eq!(language_id("OEBPS/content.opf"), Some("xml"));
        assert_eq!(language_id("toc.ncx"), Some("xml"));
        assert_eq!(language_id("cover.svg"), Some("xml"));
        assert_eq!(language_id("style.css"), Some("css"));
        assert_eq!(language_id("readme.txt"), None);
        assert_eq!(language_id("mimetype"), None);
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_scheme_follows_brightness_and_eink() {
        let all = ids(&["classic", "Adwaita", "Adwaita-dark", "kate", "oblivion"]);
        assert_eq!(pick_scheme(&all, false, false).as_deref(), Some("Adwaita"));
        assert_eq!(
            pick_scheme(&all, true, false).as_deref(),
            Some("Adwaita-dark")
        );
        assert_eq!(
            pick_scheme(&all, true, true).as_deref(),
            Some("classic"),
            "e-ink wants black on white, even when the system is dark"
        );
        let few = ids(&["solarized-light", "my-dark-one"]);
        assert_eq!(
            pick_scheme(&few, true, false).as_deref(),
            Some("my-dark-one")
        );
        assert_eq!(
            pick_scheme(&few, false, false).as_deref(),
            Some("solarized-light")
        );
        assert_eq!(pick_scheme(&[], false, false), None);
        assert_eq!(pick_scheme(&ids(&["x"]), true, false).as_deref(), Some("x"));
    }

    #[test]
    fn search_text_and_status() {
        assert_eq!(search_text_for_context("a.b", true, true), "(?s)a.b");
        assert_eq!(search_text_for_context("a.b", true, false), "a.b");
        assert_eq!(search_text_for_context("a.b", false, true), "a.b");
        assert_eq!(search_text_for_context("", true, true), "");
        assert_eq!(find_status(-1, 0), "Counting…");
        assert_eq!(find_status(0, 0), "No matches in this file");
        assert_eq!(find_status(5, 2), "2 of 5 in this file");
        assert_eq!(find_status(5, 0), "5 in this file");
    }

    #[test]
    fn hard_to_see_characters_are_found() {
        let t = "a\u{00a0}b\u{200b}\u{200b}c\u{00ad}d\u{feff}";
        assert_eq!(hidden_chars(t), vec![(1, 2), (3, 5), (6, 7), (8, 9)]);
        assert!(hidden_chars("plain text, nothing odd – “quotes” é").is_empty());
        assert_eq!(describe_hidden('\u{00a0}'), "no-break space");
        assert_eq!(describe_hidden('\u{200b}'), "zero-width space");
    }

    #[test]
    fn this_installation_knows_the_languages_and_schemes_we_ask_for() {
        // GtkSourceView needs no display for its data; skip quietly when it cannot start.
        if std::panic::catch_unwind(init).is_err() {
            return;
        }
        let lm = sv::LanguageManager::default();
        for id in ["html", "xml", "css"] {
            assert!(lm.language(id).is_some(), "language {id} is missing");
        }
        let schemes = available_schemes();
        assert!(pick_scheme(&schemes, false, false).is_some());
        assert!(
            schemes.iter().any(|s| s == "Adwaita-dark"),
            "the dark scheme: {schemes:?}"
        );
    }
}
