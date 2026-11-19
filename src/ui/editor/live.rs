//! The live parts of the code view: the debounced pass that underlines misspelled words, refreshes
//! the gutter marks and the completion data, the highlight of the tag pair at the caret, the
//! find-bar glue to GtkSourceView's `SearchContext`, the view options (minimap, hard-to-see
//! characters) and the spelling context menu.
//!
//! Search split: the match highlight and the "n of m" count of the open file come from
//! `SearchContext`; navigating, replacing, the scopes and Find All stay with `editor_search`
//! (fancy-regex: look-around, `\1`/`$1`, multi-file), because `SearchContext` knows one buffer only.

use super::search::{byte_to_char_idx, char_to_byte_idx};
use super::*;
use crate::editor_marks::{misspelled_per_line, Marks};
use crate::editor_spell::{self, SpellState, Spot};
use crate::i18n::{tr, trf};
use bookx_editing::tags as btags;

/// Largest file that is spell checked live (bytes).
const SPELL_MAX: usize = 700_000;
/// Largest file for which change marks and the tag pair are computed.
const LIVE_MAX: usize = 1_500_000;
/// Lines above and below the window that get underlines.
const SPELL_MARGIN_LINES: i32 = 40;

#[derive(Default)]
pub struct SpellShared {
    pub state: Option<SpellState>,
    /// What is remembered for this book: ignored words and files left out of the spell check.
    pub prefs: crate::editor_spell_dicts::BookPrefs,
    pub spots: Vec<Spot>,
    pub target: Option<Spot>,
}

#[derive(Debug, Clone)]
pub enum SpellAct {
    Replace(String),
    Ignore,
    Add,
}

#[derive(Debug, Clone, Copy)]
pub enum ViewOpt {
    Minimap,
    Whitespace,
}

fn is_spelled_path(path: &str) -> bool {
    matches!(extension(path).as_str(), "xhtml" | "html" | "htm" | "xht")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

impl Editor {
    pub(super) fn schedule_live(&self) {
        if let Some(t) = self.live_timer.borrow_mut().take() {
            t.remove();
        }
        let tx = self.sender.input_sender().clone();
        let slot = self.live_timer.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            slot.borrow_mut().take();
            tx.emit(Input::Live);
        });
        *self.live_timer.borrow_mut() = Some(id);
    }

    /// The debounced pass over the open file.
    pub(super) fn run_live(&mut self) {
        let Some(path) = self.current.clone().filter(|p| is_text_path(p)) else {
            return;
        };
        if self.w.center.visible_child_name().as_deref() != Some("code") {
            return;
        }
        let buf = self.buffer_for(&path);
        let text = buffer_text(&buf);
        self.update_spelling(&path, &buf, &text);
        self.update_marks(&path, &buf, &text);
        codeview::mark_hidden_chars(buf.upcast_ref(), self.prefs.show_whitespace);
        prefs_page::mark_css_colors(&buf, crate::editor_prefs::look().css_colors);
        self.update_completion_data(&path, &text);
        self.update_hints(&path, &buf, &text);
        self.update_pair_highlight();
    }

    // ---- spelling -----------------------------------------------------------

    pub(super) fn ensure_spell(&self) {
        if self.spell.borrow().state.is_some() {
            return;
        }
        let dirs = crate::editor_spell_dicts::search_dirs();
        let (lang, langs) = {
            let b = self.book.borrow();
            (
                b.package.metadata.language.clone(),
                bookx_editing::spell_session::languages_in_book(&b, &HashSet::new()),
            )
        };
        let mut st = SpellState::new(&lang, &dirs, editor_spell::user_words_path());
        st.load_languages(&langs, &dirs);
        let prefs = crate::editor_spell_dicts::BookPrefs::load(self.book_id);
        st.ignore_all(prefs.ignored.iter());
        let mut sh = self.spell.borrow_mut();
        sh.state = Some(st);
        sh.prefs = prefs;
    }

    fn update_spelling(&mut self, path: &str, buf: &sv::Buffer, text: &str) {
        let (s, e) = buf.bounds();
        buf.remove_tag_by_name(codeview::TAG_SPELL, &s, &e);
        if !is_spelled_path(path) || text.len() > SPELL_MAX {
            self.spell.borrow_mut().spots.clear();
            return;
        }
        self.ensure_spell();
        if self.spell.borrow().prefs.excluded.contains(path) {
            self.spell.borrow_mut().spots.clear();
            return;
        }
        let spots = self
            .spell
            .borrow()
            .state
            .as_ref()
            .map(|st| st.check(text))
            .unwrap_or_default();
        // only the window (plus a margin) is underlined
        let rect = self.w.view.visible_rect();
        let mut top = self
            .w
            .view
            .iter_at_location(0, rect.y())
            .unwrap_or_else(|| buf.start_iter());
        let mut bottom = self
            .w
            .view
            .iter_at_location(0, rect.y() + rect.height())
            .unwrap_or_else(|| buf.end_iter());
        top.set_line((top.line() - SPELL_MARGIN_LINES).max(0));
        let last = buf.line_count() - 1;
        bottom.set_line((bottom.line() + SPELL_MARGIN_LINES).min(last));
        bottom.forward_to_line_end();
        let from = char_to_byte_idx(text, top.offset() as usize);
        let to = char_to_byte_idx(text, bottom.offset() as usize);
        for (a, b) in editor_spell::char_spans(text, &spots, from, to) {
            buf.apply_tag_by_name(
                codeview::TAG_SPELL,
                &buf.iter_at_offset(a as i32),
                &buf.iter_at_offset(b as i32),
            );
        }
        self.spell.borrow_mut().spots = spots;
    }

    /// The right-click menu for the word under the pointer is built by the click handler of
    /// [`install_spell_menu`]; these are the actions it triggers.
    pub(super) fn spell_act(&mut self, act: SpellAct) {
        let Some(spot) = self.spell.borrow().target.clone() else {
            return;
        };
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let fresh = text
            .get(spot.start..spot.end)
            .is_some_and(|raw| bookx_editing::markup::decode_entities(raw) == spot.word);
        if !fresh {
            self.toast(&tr("The text changed; try again"));
            return;
        }
        match act {
            SpellAct::Replace(with) => {
                let a = buf.iter_at_offset(byte_to_char_idx(&text, spot.start) as i32);
                let b = buf.iter_at_offset(byte_to_char_idx(&text, spot.end) as i32);
                let (mut a, mut b) = (a, b);
                buf.begin_user_action();
                buf.delete(&mut a, &mut b);
                buf.insert(&mut a, &xml_escape(&with));
                buf.end_user_action();
            }
            SpellAct::Ignore => {
                if let Some(st) = self.spell.borrow_mut().state.as_mut() {
                    st.ignore(&spot.word);
                }
                self.toast(&trf(
                    "“{}” is ignored until the editor closes",
                    &[&spot.word],
                ));
                self.run_live();
            }
            SpellAct::Add => {
                let res = self
                    .spell
                    .borrow_mut()
                    .state
                    .as_mut()
                    .map(|st| st.add_to_dictionary(&spot.word));
                match res {
                    Some(Err(e)) => self.toast(&trf("Could not save the word: {}", &[&e])),
                    _ => self.toast(&trf("“{}” added to your dictionary", &[&spot.word])),
                }
                self.run_live();
            }
        }
    }

    // ---- gutter marks -------------------------------------------------------------

    /// The text of `path` at the last checkpoint, or as the editor found it when none exists.
    fn baseline_text(&mut self, path: &str) -> Option<String> {
        let last = self.checkpoints.list().last().map(|c| c.id);
        match last {
            Some(id) => {
                let stale = self.baseline.as_ref().is_none_or(|(i, _)| *i != id);
                if stale {
                    self.baseline = self.checkpoints.restore(id).ok().map(|a| (id, a));
                }
                self.baseline.as_ref().and_then(|(_, a)| a.get_string(path))
            }
            None => self.original.get(path).cloned(),
        }
    }

    fn update_marks(&mut self, path: &str, buf: &sv::Buffer, text: &str) {
        let baseline = if text.len() <= LIVE_MAX {
            self.baseline_text(path)
        } else {
            None
        };
        let findings: Vec<(u32, Severity, String)> = self
            .issue_marks
            .get(path)
            .map(|v| {
                v.iter()
                    .filter(|(m, _, _)| !m.is_deleted())
                    .map(|(m, sev, t)| (buf.iter_at_mark(m).line() as u32 + 1, *sev, t.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let spans: Vec<(usize, usize)> = self
            .spell
            .borrow()
            .spots
            .iter()
            .map(|s| (s.start, s.end))
            .collect();
        let m = Marks::build(
            baseline.as_deref(),
            text,
            &findings,
            &misspelled_per_line(text, &spans),
        );
        *self.gutter_marks.borrow_mut() = m;
        self.w.marks_renderer.queue_draw();
    }

    /// Remembers the findings of a book check as marks that follow the text.
    pub(super) fn set_issue_marks(&mut self, issues: &[crate::core::check::Issue]) {
        for list in std::mem::take(&mut self.issue_marks).into_values() {
            for (m, _, _) in list {
                if !m.is_deleted() {
                    if let Some(b) = m.buffer() {
                        b.delete_mark(&m);
                    }
                }
            }
        }
        let mut files: Vec<String> = issues
            .iter()
            .filter(|i| i.line.is_some() && is_text_path(&i.file))
            .map(|i| i.file.clone())
            .collect();
        files.sort();
        files.dedup();
        files.truncate(60);
        for i in issues.iter().filter(|i| files.contains(&i.file)) {
            let Some(line) = i.line else { continue };
            let buf = self.buffer_for(&i.file);
            let Some(at) = buf.iter_at_line(line.saturating_sub(1) as i32) else {
                continue;
            };
            let mark = buf.create_mark(None, &at, true);
            self.issue_marks.entry(i.file.clone()).or_default().push((
                mark,
                i.severity,
                i.message.clone(),
            ));
        }
        self.schedule_live();
    }

    // ---- completion data -----------------------------------------------------------

    pub(super) fn update_completion_data(&self, path: &str, text: &str) {
        let book = self.book.borrow();
        let css: Vec<String> = bookx_editing::markup::css_files(&book.archive);
        let mut classes: Vec<String> = Vec::new();
        for f in &css {
            let t = if self.current.as_deref() == Some(f) {
                text.to_string()
            } else {
                book.archive.get_string(f).unwrap_or_default()
            };
            classes.extend(crate::editor_complete::classes_of(&t));
        }
        classes.sort();
        classes.dedup();
        let mut d = self.comp.borrow_mut();
        d.current = path.to_string();
        d.files = book.file_list();
        d.classes = classes;
        d.words = if is_spelled_path(path) {
            crate::editor_complete::words_of(text)
        } else {
            Vec::new()
        };
        d.edited_ids
            .insert(path.to_string(), bookx_editing::markup::defined_ids(text));
        if is_spelled_path(path) && text.len() <= LIVE_MAX {
            d.edited_snips.insert(
                path.to_string(),
                bookx_editing::code_marks::anchor_snippets(text),
            );
        }
    }

    // ---- tag pair under the caret ----------------------------------------------------

    pub(super) fn update_pair_highlight(&self) {
        let buf = self.w.view.buffer();
        let (s, e) = buf.bounds();
        buf.remove_tag_by_name(codeview::TAG_PAIR, &s, &e);
        let Some(path) = &self.current else { return };
        if !matches!(
            extension(path).as_str(),
            "xhtml" | "html" | "htm" | "xht" | "xml" | "opf" | "ncx" | "svg"
        ) || self.w.center.visible_child_name().as_deref() != Some("code")
        {
            return;
        }
        let text = buffer_text(&buf);
        if text.len() > LIVE_MAX {
            return;
        }
        let at = char_to_byte_idx(&text, buf.iter_at_mark(&buf.get_insert()).offset() as usize);
        if let Some((open, close)) = btags::highlight_pair(&text, at) {
            for r in [open, close] {
                let a = byte_to_char_idx(&text, r.start) as i32;
                let b = byte_to_char_idx(&text, r.end) as i32;
                buf.apply_tag_by_name(
                    codeview::TAG_PAIR,
                    &buf.iter_at_offset(a),
                    &buf.iter_at_offset(b),
                );
            }
        }
    }

    /// The text carrying a tag, segments joined by `|` (dev hook).
    pub(super) fn tag_segments(&self, name: &str) -> String {
        let buf = self.w.view.buffer();
        let Some(tag) = buf.tag_table().lookup(name) else {
            return String::new();
        };
        let mut out: Vec<String> = Vec::new();
        let mut it = buf.start_iter();
        loop {
            if it.starts_tag(Some(&tag)) {
                let mut end = it;
                end.forward_to_tag_toggle(Some(&tag));
                out.push(buf.text(&it, &end, false).to_string());
            }
            if !it.forward_to_tag_toggle(Some(&tag)) {
                break;
            }
        }
        out.join("|")
    }

    // ---- search context --------------------------------------------------------------

    pub(super) fn search_context(&self) -> Option<sv::SearchContext> {
        self.current
            .as_ref()
            .and_then(|p| self.contexts.get(p))
            .cloned()
    }

    /// The find entry or an option changed: update the highlight of every match in the open file.
    pub(super) fn find_changed(&mut self) {
        let q = self.settings();
        let own = self.own_highlight();
        let st = &self.search_settings;
        st.set_regex_enabled(q.mode.is_regex());
        st.set_case_sensitive(q.case_sensitive);
        st.set_at_word_boundaries(q.whole_word);
        st.set_search_text(Some(&if own {
            String::new()
        } else {
            codeview::search_text_for_context(&q.find, q.mode.is_regex(), q.dot_all)
        }));
        self.update_own_highlight();
        self.refresh_find_status();
    }

    /// "n of m in this file" from the search context; the engine counts when the context cannot
    /// (the "Marked text" scope, or a pattern GtkSourceView's regex flavour rejects).
    pub(super) fn refresh_find_status(&mut self) {
        let Some(re) = self.pattern() else { return };
        let ctx = self.search_context();
        match ctx {
            Some(c)
                if self.scope() != Scope::MarkedText
                    && !self.own_highlight()
                    && c.regex_error().is_none() =>
            {
                let buf = self.w.view.buffer();
                let pos = buf
                    .selection_bounds()
                    .map(|(a, b)| c.occurrence_position(&a, &b))
                    .unwrap_or(0);
                self.w
                    .find_status
                    .set_label(&codeview::find_status(c.occurrences_count(), pos));
            }
            _ => self.update_find_count(&re),
        }
    }

    /// Highlights matches in the open file only.
    pub(super) fn focus_search_context(&self) {
        for (p, c) in &self.contexts {
            c.set_highlight(self.current.as_deref() == Some(p));
        }
        self.update_own_highlight();
    }

    // ---- view options ------------------------------------------------------------------

    pub(super) fn set_view_option(&mut self, opt: ViewOpt, on: bool) {
        match opt {
            ViewOpt::Minimap => {
                self.prefs.minimap = on;
                self.w.map.set_visible(on);
            }
            ViewOpt::Whitespace => {
                self.prefs.show_whitespace = on;
                codeview::set_whitespace(&self.w.view, on);
                let buf = self.w.view.buffer();
                codeview::mark_hidden_chars(&buf, on);
            }
        }
        self.prefs.save();
    }
}

use crate::editor_search::Scope;

/// Adds the spelling entries to the view's context menu when the pointer is on a misspelled word.
pub(super) fn install_spell_menu(
    view: &sv::View,
    spell: Rc<RefCell<SpellShared>>,
    tx: relm4::Sender<Input>,
) {
    let menu = gio::Menu::new();
    view.set_extra_menu(Some(&menu));
    let actions = gio::SimpleActionGroup::new();
    let replace = gio::SimpleAction::new("replace", Some(glib::VariantTy::STRING));
    let t = tx.clone();
    replace.connect_activate(move |_, p| {
        if let Some(w) = p.and_then(|v| v.str()) {
            t.emit(Input::Spell(SpellAct::Replace(w.to_string())));
        }
    });
    actions.add_action(&replace);
    for (name, act) in [("ignore", SpellAct::Ignore), ("add", SpellAct::Add)] {
        let a = gio::SimpleAction::new(name, None);
        let t = tx.clone();
        a.connect_activate(move |_, _| t.emit(Input::Spell(act.clone())));
        actions.add_action(&a);
    }
    view.insert_action_group("spell", Some(&actions));

    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = view.clone();
    click.connect_pressed(move |_, _, x, y| {
        menu.remove_all();
        spell.borrow_mut().target = None;
        let (bx, by) = v.window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let Some(iter) = v.iter_at_location(bx, by) else {
            return;
        };
        let buf = v.buffer();
        if !buf.has_selection() {
            buf.place_cursor(&iter);
        }
        let text = buffer_text(&buf);
        let at = char_to_byte_idx(&text, iter.offset() as usize);
        build_spell_menu(&menu, &spell, &text, at);
        super::navigate::append_code_section(&menu, &v, &text, at, iter.offset());
    });
    view.add_controller(click);
}

/// Fills `menu` with the spelling entries for the word at byte `at` of `text`, if it is a misspelled
/// word (and remembers it as the target of the `spell.*` actions).
pub(super) fn build_spell_menu(
    menu: &gio::Menu,
    spell: &Rc<RefCell<SpellShared>>,
    text: &str,
    at: usize,
) {
    let mut sh = spell.borrow_mut();
    sh.target = None;
    menu.remove_all();
    let Some(spot) = editor_spell::spot_at(&sh.spots, at).cloned() else {
        return;
    };
    let fresh = text
        .get(spot.start..spot.end)
        .is_some_and(|raw| bookx_editing::markup::decode_entities(raw) == spot.word);
    if !fresh {
        return;
    }
    let suggestions: Vec<String> = sh
        .state
        .as_ref()
        .map(|st| st.suggest(&spot))
        .unwrap_or_default();
    let section = gio::Menu::new();
    if !suggestions.is_empty() {
        let sub = gio::Menu::new();
        for s in suggestions.iter().take(8) {
            let it = gio::MenuItem::new(Some(s), None);
            it.set_action_and_target_value(Some("spell.replace"), Some(&s.to_variant()));
            sub.append_item(&it);
        }
        section.append_submenu(Some(&tr("Replace with")), &sub);
    }
    section.append(
        Some(&trf("Ignore “{}”", &[&spot.word])),
        Some("spell.ignore"),
    );
    section.append(Some(&tr("Add to Dictionary")), Some("spell.add"));
    menu.append_section(None, &section);
    sh.target = Some(spot);
}
