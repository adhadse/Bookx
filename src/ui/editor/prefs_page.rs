//! The *Editor* page of Preferences (CE-357, CE-400..402) and everything that applies it live: font,
//! tab width, line numbers, margins, colour scheme (with the schemes the user drops into the
//! config folder), CSS colour preview, auto-closing tags, completion, spelling underline and the
//! preview's font. The settings live in `editor_prefs::EditorLook`; a change is announced on the
//! `settings_bus::editor()` bus and every open editor window applies it.

use super::*;
use crate::editor_prefs::{self as ep, EditorLook, EditorPrefs};
use crate::ui::preferences::{register_group, register_page, PrefCtx};

#[derive(Debug, Clone)]
pub enum PrefsAct {
    /// Show the editor preferences window.
    Open,
    /// The settings changed (anywhere): apply them.
    Changed,
    ClearMarks,
    InsertSnippet,
}

// ---- decisions (no GTK display needed) --------------------------------------------------------

/// CSS for the code font of a Pango description such as "Fira Code 12"; empty for no choice.
pub fn font_css(desc: &str) -> String {
    if desc.trim().is_empty() {
        return String::new();
    }
    let d = pango::FontDescription::from_string(desc);
    let mut css = String::new();
    if let Some(f) = d.family() {
        let first = f.split(',').next().unwrap_or("").trim().replace('"', "");
        if !first.is_empty() {
            css.push_str(&format!("font-family: \"{first}\", monospace; "));
        }
    }
    if d.size() > 0 && !d.is_size_absolute() {
        css.push_str(&format!("font-size: {}pt; ", d.size() / pango::SCALE));
    }
    if css.is_empty() {
        return String::new();
    }
    format!(".code-view {{ {css}}}")
}

/// The preview's text style with the user's font and smallest size applied.
pub fn preview_style(mut st: CanvasStyle, look: &EditorLook) -> CanvasStyle {
    if !look.preview_font.trim().is_empty() {
        let d = pango::FontDescription::from_string(&look.preview_font);
        if let Some(f) = d.family() {
            st.font_family = f.to_string();
        }
        if d.size() > 0 {
            st.font_size = d.size() as f64 / pango::SCALE as f64;
        }
    }
    if look.preview_min_size > 0 {
        st.font_size = st.font_size.max(look.preview_min_size as f64);
    }
    st
}

// ---- applying ---------------------------------------------------------------------------------

thread_local! {
    static FONT_PROVIDER: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static BLOCKED: RefCell<Vec<(glib::WeakRef<sv::View>, bool)>> = const { RefCell::new(Vec::new()) };
}

/// Lets GtkSourceView find the schemes in the config folder (`styles/*.xml`).
pub fn add_scheme_dir() {
    let Some(dir) = ep::schemes_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let mgr = sv::StyleSchemeManager::default();
    if let Some(d) = dir.to_str() {
        mgr.append_search_path(d);
        mgr.force_rescan();
    }
}

fn apply_font(look: &EditorLook) {
    let css = font_css(&look.font);
    FONT_PROVIDER.with(|p| {
        let mut p = p.borrow_mut();
        let provider = p.get_or_insert_with(|| {
            let provider = gtk::CssProvider::new();
            if let Some(display) = gdk::Display::default() {
                gtk::style_context_add_provider_for_display(
                    &display,
                    &provider,
                    gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
                );
            }
            provider
        });
        provider.load_from_string(&css);
    });
}

/// Applies the view settings to the code view.
pub fn apply_view(view: &sv::View, look: &EditorLook) {
    apply_font(look);
    view.set_tab_width(look.tab_width);
    view.set_indent_width(look.tab_width as i32);
    view.set_insert_spaces_instead_of_tabs(look.insert_spaces);
    view.set_show_line_numbers(look.line_numbers);
    view.set_highlight_current_line(look.highlight_line);
    view.set_show_right_margin(look.right_margin);
    view.set_right_margin_position(look.right_margin_column);
    let completion = view.completion();
    BLOCKED.with(|b| {
        let mut b = b.borrow_mut();
        b.retain(|(w, _)| w.upgrade().is_some());
        let blocked = match b
            .iter_mut()
            .find(|(w, _)| w.upgrade().as_ref() == Some(view))
        {
            Some(e) => &mut e.1,
            None => {
                b.push((view.downgrade(), false));
                &mut b.last_mut().expect("just pushed").1
            }
        };
        if !look.completion_auto && !*blocked {
            completion.block_interactive();
            *blocked = true;
        } else if look.completion_auto && *blocked {
            completion.unblock_interactive();
            *blocked = false;
        }
    });
}

/// Applies the settings that belong to one buffer: scheme, spelling underline, colour preview.
pub fn apply_buffer(buf: &sv::Buffer) {
    codeview::apply_scheme(buf, system_is_dark(), crate::eink::is_active());
    let look = ep::look();
    if let Some(t) = buf.tag_table().lookup(codeview::TAG_SPELL) {
        t.set_underline(if look.spell_underline {
            pango::Underline::Error
        } else {
            pango::Underline::None
        });
    }
    mark_css_colors(buf, look.css_colors);
}

const COLOR_TAG: &str = "css-color-";

/// Paints colour literals of a style sheet in their colour (CE-356).
pub fn mark_css_colors(buf: &sv::Buffer, on: bool) {
    let table = buf.tag_table();
    let (s, e) = buf.bounds();
    let mut old: Vec<gtk::TextTag> = Vec::new();
    table.foreach(|t| {
        if t.name().is_some_and(|n| n.starts_with(COLOR_TAG)) {
            old.push(t.clone());
        }
    });
    for t in &old {
        buf.remove_tag(t, &s, &e);
    }
    let is_css = buf.language().is_some_and(|l| l.id() == "css");
    if !on || !is_css || buf.char_count() > 400_000 {
        return;
    }
    let text = buf.text(&s, &e, false);
    for span in ep::css_colors(&text) {
        let (r, g, b) = span.rgb;
        let name = format!("{COLOR_TAG}{r:02x}{g:02x}{b:02x}");
        let tag = table.lookup(&name).unwrap_or_else(|| {
            let (fr, fg, fb) = ep::contrast_text(span.rgb);
            let t = gtk::TextTag::new(Some(&name));
            t.set_background(Some(&format!("rgb({r},{g},{b})")));
            t.set_foreground(Some(&format!("rgb({fr},{fg},{fb})")));
            table.add(&t);
            t
        });
        buf.apply_tag(
            &tag,
            &buf.iter_at_offset(span.start as i32),
            &buf.iter_at_offset(span.end as i32),
        );
    }
}

/// Typing `>` after an open tag adds the closing tag (when the preference is on).
pub fn install_typing(view: &sv::View) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = view.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let plain = !mods.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );
        if key != gdk::Key::greater || !plain || !ep::look().auto_close_tags {
            return glib::Propagation::Proceed;
        }
        let buf = v.buffer();
        let markup = buf
            .downcast_ref::<sv::Buffer>()
            .and_then(|b| b.language())
            .is_some_and(|l| matches!(l.id().as_str(), "html" | "xml"));
        if !markup || buf.selection_bounds().is_some() {
            return glib::Propagation::Proceed;
        }
        let cur = buf.iter_at_mark(&buf.get_insert());
        let mut from = cur;
        from.backward_chars(300);
        let before = buf.text(&from, &cur, false);
        let Some(close) = ep::closing_tag_for(&before) else {
            return glib::Propagation::Proceed;
        };
        buf.begin_user_action();
        let mut at = buf.iter_at_mark(&buf.get_insert());
        buf.insert(&mut at, &format!(">{close}"));
        let mut back = buf.iter_at_mark(&buf.get_insert());
        back.backward_chars(close.chars().count() as i32);
        buf.place_cursor(&back);
        buf.end_user_action();
        glib::Propagation::Stop
    });
    view.add_controller(keys);
}

impl Editor {
    pub(super) fn prefs_update(&mut self, a: PrefsAct) {
        match a {
            PrefsAct::Open => self.open_editor_prefs(),
            PrefsAct::Changed => self.apply_prefs(),
            PrefsAct::ClearMarks => {
                let paths: Vec<String> = self.marks.keys().cloned().collect();
                let n = paths.len();
                for p in paths {
                    self.clear_marks(&p);
                }
                self.toast(&if n == 0 {
                    tr("Nothing is marked")
                } else {
                    tr("Marked text cleared")
                });
            }
            PrefsAct::InsertSnippet => self.insert_snippet_dialog(),
        }
    }

    /// `// @append_final_output_to_marked`: the final output of a function run goes on a new line after
    /// the marked text of the open file, and is marked too (CE-110).
    pub(super) fn append_final_to_marked(&mut self, output: &str) -> bool {
        let Some(path) = self.current.clone() else {
            return false;
        };
        let buf = self.buffer_for(&path);
        let text = buffer_text(&buf);
        let ranges: Vec<(usize, usize)> = self
            .marks
            .get(&path)
            .map(|ms| {
                ms.iter()
                    .filter(|(a, b)| !a.is_deleted() && !b.is_deleted())
                    .map(|(a, b)| {
                        (
                            search::char_to_byte_idx(&text, buf.iter_at_mark(a).offset() as usize),
                            search::char_to_byte_idx(&text, buf.iter_at_mark(b).offset() as usize),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some((new, (a, b))) =
            crate::editor_search_fn::append_after_marked(&text, &ranges, output)
        else {
            self.toast(&tr("Nothing is marked, so the final output was not added"));
            return false;
        };
        let (ca, cb) = (
            search::byte_to_char_idx(&new, a) as i32,
            search::byte_to_char_idx(&new, b) as i32,
        );
        buf.begin_user_action();
        let (mut s, mut e) = buf.bounds();
        buf.delete(&mut s, &mut e);
        let mut s = buf.start_iter();
        buf.insert(&mut s, &new);
        buf.end_user_action();
        let (ia, ib) = (buf.iter_at_offset(ca), buf.iter_at_offset(cb));
        let (ma, mb) = (
            gtk::TextMark::new(None, true),
            gtk::TextMark::new(None, false),
        );
        buf.add_mark(&ma, &ia);
        buf.add_mark(&mb, &ib);
        buf.apply_tag_by_name("marked-region", &ia, &ib);
        self.marks.entry(path).or_default().push((ma, mb));
        self.toast(&tr("The final output was added after the marked text"));
        true
    }

    /// The settings of the editor changed: bring this window in line.
    pub(super) fn apply_prefs(&mut self) {
        let look = ep::look();
        apply_view(&self.w.view, &look);
        for b in self.buffers.values() {
            apply_buffer(b);
        }
        let mut st: CanvasStyle = self
            .shared
            .cfg
            .borrow()
            .reader
            .canvas_style(system_is_dark());
        st.layout = LayoutMode::Scroll;
        st.margin = 24.0;
        self.w.preview.set_style(preview_style(st, &look));
        self.refresh_preview_now();
        // the toggles that other menus own: flip them through their actions so the menus follow
        let file = EditorPrefs::load();
        if file.show_whitespace != codeview::whitespace_on(&self.w.view) {
            let _ = gtk::prelude::WidgetExt::activate_action(&self.w.win, "view.whitespace", None);
        }
        if file.minimap != self.w.map.is_visible() {
            let _ = gtk::prelude::WidgetExt::activate_action(&self.w.win, "view.minimap", None);
        }
        let wrap = crate::editor_state::WindowPrefs::load().wrap_lines;
        if wrap != self.ext.prefs.borrow().wrap_lines {
            let _ = gtk::prelude::WidgetExt::activate_action(&self.w.win, "ext.wrap", None);
        }
        self.prefs.convert_entities = file.convert_entities;
        self.prefs.check_accessibility = file.check_accessibility;
        self.prefs.ignored_checks = file.ignored_checks;
    }

    fn open_editor_prefs(&mut self) {
        let ctx = PrefCtx {
            parent: self.w.win.clone().upcast(),
            shared: self.shared.clone(),
            on_change: Rc::new(|_| {}),
        };
        let page = adw::PreferencesPage::new();
        page.set_title(&tr("Editor"));
        for g in groups(&ctx) {
            page.add(&g);
        }
        let d = adw::PreferencesDialog::new();
        d.set_title(&tr("Editor Preferences"));
        d.add(&page);
        d.present(Some(&self.w.win));
    }

    /// Dev hooks `editor:prefs-…`; `true` when the command was one of them.
    pub(super) fn prefs_dev(&mut self, c: &str) -> bool {
        if let Some(kv) = c.strip_prefix("prefs-set=") {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            let (k, v) = (k.to_string(), v.to_string());
            EditorLook::update(|l| set_field(l, &k, &v));
            return true;
        }
        if let Some(id) = c.strip_prefix("prefs-write-scheme=") {
            // a user scheme dropped into the config folder
            let xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<style-scheme id=\"{id}\" name=\"{id}\" version=\"1.0\">\n  <style name=\"text\" foreground=\"#101010\" background=\"#ffeecc\"/>\n  <style name=\"search-match\" background=\"#ffdd00\"/>\n</style-scheme>\n"
            );
            let ok = ep::schemes_dir().is_some_and(|d| {
                let _ = std::fs::create_dir_all(&d);
                std::fs::write(d.join(format!("{id}.xml")), xml).is_ok()
            });
            add_scheme_dir();
            sv::StyleSchemeManager::default().force_rescan();
            eprintln!(
                "editor_scheme: written={ok} listed={}",
                codeview::available_schemes().iter().any(|s| s == id)
            );
            return true;
        }
        if let Some(kv) = c.strip_prefix("keys-set=") {
            let (action, accel) = kv.split_once('=').unwrap_or((kv, ""));
            let r = crate::shortcuts::update(|s| s.bind(action, accel));
            eprintln!("editor_keys: bound={}", r.is_ok());
            return true;
        }
        if let Some(title) = c.strip_prefix("keys-palette=") {
            let shown = self
                .palette_entries()
                .into_iter()
                .find(|e| e.title == title)
                .map(|e| e.shortcut)
                .unwrap_or_else(|| "(not listed)".into());
            eprintln!("editor_keys: palette {title}={shown}");
            return true;
        }
        if c == "clear-marks" {
            self.prefs_update(PrefsAct::ClearMarks);
            return true;
        }
        if c == "prefs-open" {
            self.open_editor_prefs();
            return true;
        }
        if c == "prefs-dump" {
            let v = &self.w.view;
            let buf = v.buffer();
            let scheme = buf
                .downcast_ref::<sv::Buffer>()
                .map(codeview::scheme_id)
                .unwrap_or_default();
            let colors = buf
                .downcast_ref::<sv::Buffer>()
                .map(|b| {
                    let mut tags = Vec::new();
                    b.tag_table().foreach(|t| {
                        if t.name().is_some_and(|n| n.starts_with(COLOR_TAG)) {
                            tags.push(t.clone());
                        }
                    });
                    tags.iter()
                        .filter(|t| {
                            let mut it = b.start_iter();
                            it.starts_tag(Some(*t)) || it.forward_to_tag_toggle(Some(*t))
                        })
                        .count()
                })
                .unwrap_or(0);
            eprintln!(
                "editor_prefs: tab={} spaces={} lines={} hl={} margin={} col={} scheme={} font_css={:?} colors={} wrap={} minimap={} ws={}",
                v.tab_width(),
                v.is_insert_spaces_instead_of_tabs(),
                v.shows_line_numbers(),
                v.is_highlight_current_line(),
                v.shows_right_margin(),
                v.right_margin_position(),
                scheme,
                font_css(&ep::look().font),
                colors,
                v.wrap_mode() != gtk::WrapMode::None,
                self.w.map.is_visible(),
                codeview::whitespace_on(v),
            );
            return true;
        }
        false
    }
}

fn set_field(l: &mut EditorLook, k: &str, v: &str) {
    let b = || v == "true" || v == "1";
    match k {
        "font" => l.font = v.into(),
        "tab_width" => l.tab_width = v.parse().unwrap_or(2),
        "insert_spaces" => l.insert_spaces = b(),
        "line_numbers" => l.line_numbers = b(),
        "highlight_line" => l.highlight_line = b(),
        "right_margin" => l.right_margin = b(),
        "right_margin_column" => l.right_margin_column = v.parse().unwrap_or(80),
        "scheme" => l.scheme = v.into(),
        "css_colors" => l.css_colors = b(),
        "auto_close_tags" => l.auto_close_tags = b(),
        "completion_auto" => l.completion_auto = b(),
        "spell_underline" => l.spell_underline = b(),
        "preview_font" => l.preview_font = v.into(),
        "preview_min_size" => l.preview_min_size = v.parse().unwrap_or(0),
        "preview_follow" => l.preview_follow = b(),
        _ => {}
    }
}

// ---- the page ---------------------------------------------------------------------------------

/// Adds the Editor page to Preferences (called once by its registry).
pub fn register() {
    register_page("editor", &tr("Editor"), "document-edit-symbolic", 45);
    for (i, f) in [
        text_group as fn(&PrefCtx) -> adw::PreferencesGroup,
        scheme_group,
        typing_group,
        preview_group,
    ]
    .into_iter()
    .enumerate()
    {
        register_group("editor", 10 * (i as i32 + 1), f);
    }
}

fn groups(ctx: &PrefCtx) -> Vec<adw::PreferencesGroup> {
    vec![
        text_group(ctx),
        scheme_group(ctx),
        typing_group(ctx),
        preview_group(ctx),
    ]
}

fn switch(
    title: &str,
    subtitle: &str,
    on: bool,
    set: impl Fn(&mut EditorLook, bool) + 'static,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(glib::markup_escape_text(title))
        .active(on)
        .build();
    if !subtitle.is_empty() {
        row.set_subtitle(&glib::markup_escape_text(subtitle));
    }
    row.connect_active_notify(move |r| {
        let on = r.is_active();
        EditorLook::update(|l| set(l, on));
    });
    row
}

fn spin(
    title: &str,
    value: u32,
    min: u32,
    max: u32,
    set: impl Fn(&mut EditorLook, u32) + 'static,
) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(min as f64, max as f64, 1.0);
    row.set_title(&glib::markup_escape_text(title));
    row.set_value(value as f64);
    row.connect_value_notify(move |r| {
        let v = r.value() as u32;
        EditorLook::update(|l| set(l, v));
    });
    row
}

fn font_row(
    title: &str,
    desc: &str,
    set: impl Fn(&mut EditorLook, String) + 'static,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .build();
    let btn = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    btn.set_valign(gtk::Align::Center);
    btn.set_level(gtk::FontLevel::Font);
    if !desc.is_empty() {
        btn.set_font_desc(&pango::FontDescription::from_string(desc));
    }
    crate::ui::audit::label(&btn, title);
    btn.connect_font_desc_notify(move |b| {
        let d = b.font_desc().map(|d| d.to_string()).unwrap_or_default();
        EditorLook::update(|l| set(l, d.clone()));
    });
    let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
    reset.add_css_class("flat");
    reset.set_valign(gtk::Align::Center);
    reset.set_tooltip_text(Some(&tr("Use the default font")));
    crate::ui::audit::label(&reset, &trf("Use the default {}", &[&title.to_lowercase()]));
    {
        let btn = btn.clone();
        reset.connect_clicked(move |_| btn.set_font_desc(&pango::FontDescription::new()));
    }
    row.add_suffix(&reset);
    row.add_suffix(&btn);
    row
}

fn text_group(_: &PrefCtx) -> adw::PreferencesGroup {
    let look = ep::look();
    let file = EditorPrefs::load();
    let wrap = crate::editor_state::WindowPrefs::load().wrap_lines;
    let g = adw::PreferencesGroup::builder()
        .title(tr("Code"))
        .description(tr(
            "How the text of the editor looks. Changes apply to every open editor window.",
        ))
        .build();
    g.add(&font_row(&tr("Font"), &look.font, |l, d| l.font = d));
    g.add(&spin(&tr("Tab width"), look.tab_width, 1, 16, |l, v| {
        l.tab_width = v
    }));
    g.add(&switch(
        &tr("Insert spaces instead of tabs"),
        "",
        look.insert_spaces,
        |l, on| l.insert_spaces = on,
    ));
    g.add(&switch(
        &tr("Line numbers"),
        "",
        look.line_numbers,
        |l, on| l.line_numbers = on,
    ));
    g.add(&switch(
        &tr("Highlight the current line"),
        "",
        look.highlight_line,
        |l, on| l.highlight_line = on,
    ));
    let rm = switch(
        &tr("Right margin guide"),
        &tr("A vertical line at a column, to keep lines short"),
        look.right_margin,
        |l, on| l.right_margin = on,
    );
    g.add(&rm);
    g.add(&spin(
        &tr("Right margin column"),
        look.right_margin_column,
        20,
        200,
        |l, v| l.right_margin_column = v,
    ));
    // the three toggles that the View menu owns too
    let wrap_row = adw::SwitchRow::builder()
        .title(tr("Wrap long lines"))
        .active(wrap)
        .build();
    wrap_row.connect_active_notify(|r| {
        let mut p = crate::editor_state::WindowPrefs::load();
        p.wrap_lines = r.is_active();
        p.save();
        crate::settings_bus::editor().notify();
    });
    g.add(&wrap_row);
    let ws = adw::SwitchRow::builder()
        .title(tr("Show spaces and tabs"))
        .active(file.show_whitespace)
        .build();
    ws.connect_active_notify(|r| {
        let mut p = EditorPrefs::load();
        p.show_whitespace = r.is_active();
        p.save();
        crate::settings_bus::editor().notify();
    });
    g.add(&ws);
    let mm = adw::SwitchRow::builder()
        .title(tr("Overview map at the right edge"))
        .active(file.minimap)
        .build();
    mm.connect_active_notify(|r| {
        let mut p = EditorPrefs::load();
        p.minimap = r.is_active();
        p.save();
        crate::settings_bus::editor().notify();
    });
    g.add(&mm);
    g
}

/// The scheme previews are sample code in text views; they are decoration for the scheme
/// buttons, so they are neither focusable nor announced as unnamed text boxes.
fn hide_previews_from_a11y(w: &gtk::Widget) {
    if w.is::<sv::View>() {
        w.set_focusable(false);
        w.set_can_target(false);
        crate::ui::audit::label(w, &tr("Colour scheme preview"));
        return;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        hide_previews_from_a11y(&child);
        c = child.next_sibling();
    }
}

fn scheme_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    add_scheme_dir();
    let look = ep::look();
    let g = adw::PreferencesGroup::builder()
        .title(tr("Colour scheme"))
        .description(tr(
            "Your own schemes (GtkSourceView .xml files) go into the styles folder of the Bookx configuration.",
        ))
        .build();
    let auto = adw::SwitchRow::builder()
        .title(tr("Follow light and dark mode"))
        .subtitle(tr(
            "Pick a scheme that fits the system style, black on white for e-ink",
        ))
        .active(look.scheme.is_empty())
        .build();
    let chooser = sv::StyleSchemeChooserWidget::new();
    chooser.set_sensitive(!auto.is_active());
    crate::ui::audit::label(&chooser, &tr("Colour schemes"));
    hide_previews_from_a11y(chooser.upcast_ref());
    // the chooser builds its previews again when the scheme list changes or it is shown
    chooser.connect_map(|c| hide_previews_from_a11y(c.upcast_ref()));
    chooser.connect_style_scheme_notify(|c| hide_previews_from_a11y(c.upcast_ref()));
    {
        let weak = chooser.downgrade();
        let id = sv::StyleSchemeManager::default().connect_notify_local(
            Some("scheme-ids"),
            move |_, _| {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(c) = weak.upgrade() {
                        hide_previews_from_a11y(c.upcast_ref());
                    }
                });
            },
        );
        let mgr_id = std::cell::RefCell::new(Some(id));
        chooser.connect_destroy(move |_| {
            if let Some(id) = mgr_id.borrow_mut().take() {
                sv::StyleSchemeManager::default().disconnect(id);
            }
        });
    }
    if let Some(s) = sv::StyleSchemeManager::default().scheme(&look.scheme) {
        chooser.set_style_scheme(&s);
    }
    let frame = gtk::ScrolledWindow::new();
    frame.set_min_content_height(220);
    frame.set_max_content_height(260);
    frame.set_has_frame(true);
    frame.set_child(Some(&chooser));
    {
        let (chooser,) = (chooser.clone(),);
        auto.connect_active_notify(move |r| {
            let on = r.is_active();
            chooser.set_sensitive(!on);
            let id = if on {
                String::new()
            } else {
                sv::prelude::StyleSchemeChooserExt::style_scheme(&chooser)
                    .id()
                    .to_string()
            };
            EditorLook::update(|l| l.scheme = id.clone());
        });
    }
    {
        let auto = auto.clone();
        chooser.connect_style_scheme_notify(move |c| {
            if auto.is_active() {
                return;
            }
            let id = sv::prelude::StyleSchemeChooserExt::style_scheme(c)
                .id()
                .to_string();
            EditorLook::update(|l| l.scheme = id.clone());
        });
    }
    g.add(&auto);
    g.add(&frame);
    let open = adw::ActionRow::builder()
        .title(tr("Open the styles folder"))
        .activatable(true)
        .build();
    open.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    let parent = ctx.parent.clone();
    open.connect_activated(move |_| {
        if let Some(dir) = ep::schemes_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(dir)));
            launcher.launch(
                parent.root().and_downcast_ref::<gtk::Window>(),
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    });
    g.add(&open);
    g
}

fn typing_group(_: &PrefCtx) -> adw::PreferencesGroup {
    let look = ep::look();
    let g = adw::PreferencesGroup::builder()
        .title(tr("Editing"))
        .description(tr(
            "Keyboard shortcuts of the editor are under Keyboard Shortcuts.",
        ))
        .build();
    g.add(&switch(
        &tr("Close tags automatically"),
        &tr("Typing > after an opening tag adds the closing tag"),
        look.auto_close_tags,
        |l, on| l.auto_close_tags = on,
    ));
    g.add(&switch(
        &tr("Suggest completions while typing"),
        &tr("Off: completions open with Ctrl+Space"),
        look.completion_auto,
        |l, on| l.completion_auto = on,
    ));
    g.add(&switch(
        &tr("Show colours in style sheets"),
        &tr("Colour codes like #1a2b3c are painted in their colour"),
        look.css_colors,
        |l, on| l.css_colors = on,
    ));
    g.add(&switch(
        &tr("Underline misspelled words"),
        &tr("Words that are not in the dictionary get a wavy red line"),
        look.spell_underline,
        |l, on| l.spell_underline = on,
    ));
    let file = EditorPrefs::load();
    let ent = adw::SwitchRow::builder()
        .title(tr("Turn typed entities into characters"))
        .subtitle(glib::markup_escape_text(&tr(
            "Typing ; after &name; inserts the character",
        )))
        .active(file.convert_entities)
        .build();
    ent.connect_active_notify(|r| {
        let mut p = EditorPrefs::load();
        p.convert_entities = r.is_active();
        p.save();
        crate::settings_bus::editor().notify();
    });
    g.add(&ent);
    g
}

fn preview_group(_: &PrefCtx) -> adw::PreferencesGroup {
    let look = ep::look();
    let file = EditorPrefs::load();
    let g = adw::PreferencesGroup::builder()
        .title(tr("Preview"))
        .description(tr("The live preview next to the code."))
        .build();
    g.add(&font_row(&tr("Text font"), &look.preview_font, |l, d| {
        l.preview_font = d
    }));
    g.add(&spin(
        &tr("Smallest text size"),
        look.preview_min_size,
        0,
        48,
        |l, v| l.preview_min_size = v,
    ));
    g.add(&switch(
        &tr("Follow the cursor"),
        &tr("The preview scrolls to the element at the cursor when the editor opens"),
        look.preview_follow,
        |l, on| l.preview_follow = on,
    ));
    let both = adw::SwitchRow::builder()
        .title(tr("Scrolling the preview scrolls the code"))
        .active(file.scroll_both)
        .build();
    both.connect_active_notify(|r| {
        let mut p = EditorPrefs::load();
        p.scroll_both = r.is_active();
        p.save();
        crate::settings_bus::editor().notify();
    });
    g.add(&both);
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_description_becomes_css() {
        assert_eq!(font_css(""), "");
        let css = font_css("Fira Code 12");
        assert!(css.contains("font-family: \"Fira Code\""), "{css}");
        assert!(css.contains("font-size: 12pt"), "{css}");
        assert!(css.starts_with(".code-view"));
        assert_eq!(
            font_css("Monospace"),
            ".code-view { font-family: \"Monospace\", monospace; }"
        );
    }

    #[test]
    fn the_preview_style_follows_the_font_and_minimum() {
        let base = CanvasStyle::default();
        let same = preview_style(base.clone(), &EditorLook::default());
        assert_eq!(same.font_family, base.font_family);
        assert_eq!(same.font_size, base.font_size);
        let l = EditorLook {
            preview_font: "Sans 9".into(),
            preview_min_size: 11,
            ..Default::default()
        };
        let st = preview_style(base.clone(), &l);
        assert_eq!(st.font_family, "Sans");
        assert_eq!(st.font_size, 11.0, "the minimum wins over a smaller font");
        let l = EditorLook {
            preview_font: "Serif 20".into(),
            preview_min_size: 11,
            ..Default::default()
        };
        assert_eq!(preview_style(base, &l).font_size, 20.0);
    }

    #[test]
    fn dev_fields_set_the_look() {
        let mut l = EditorLook::default();
        set_field(&mut l, "tab_width", "8");
        set_field(&mut l, "line_numbers", "false");
        set_field(&mut l, "scheme", "kate");
        set_field(&mut l, "unknown", "x");
        assert_eq!(
            (l.tab_width, l.line_numbers, l.scheme.as_str()),
            (8, false, "kate")
        );
    }
}
