//! Format actions of the code view (CE-350 bold, italic, underline, strike-through, super/subscript, code),
//! split a tag at the caret (CE-352), sort CSS rules (CE-355), close the tag when `</` is typed (CE-348) and
//! the single message type of the code-editing helpers (`Input::Code`). The decisions are in
//! `bookx_editing::code_format` / `code_nav`; this file moves text in and out of the buffer.

use super::search::{byte_to_char_idx, char_to_byte_idx};
use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::code_format::{self as cf, Format, FormatError};
use bookx_editing::tags::TagEdit;

#[derive(Debug, Clone)]
pub enum CodeMsg {
    Format(Format),
    SplitTag,
    InsertImage,
    InsertLink,
    ImageDo {
        href: String,
        alt: String,
        title: String,
    },
    LinkDo {
        href: String,
        text: Option<String>,
        title: String,
    },
    SortCss,
    /// `</` was just typed.
    AutoClose,
    /// Ctrl+click at a character offset of the open file.
    CtrlClick(i32),
    /// F12: go to the definition of the link, class or id at the caret.
    GotoDef,
    RenameClass(String),
    RenameDo {
        old: String,
        new: String,
    },
    /// Open an MDN help page in the browser (the URL was built by `code_nav::mdn_url`).
    Help(String),
    Hint(markup_hints::HintOpt, bool),
}

/// The message behind an `edit.code` menu target.
pub(super) fn code_msg_for(id: &str) -> Option<CodeMsg> {
    Some(match id {
        "link" => CodeMsg::InsertLink,
        "image" => CodeMsg::InsertImage,
        "split" => CodeMsg::SplitTag,
        "goto" => CodeMsg::GotoDef,
        "sort-css" => CodeMsg::SortCss,
        f => CodeMsg::Format(Format::from_id(f)?),
    })
}

pub(super) fn format_label(f: Format) -> String {
    {
        match f {
            Format::Bold => tr("Bold"),
            Format::Italic => tr("Italic"),
            Format::Underline => tr("Underline"),
            Format::Strike => tr("Strikethrough"),
            Format::Superscript => tr("Superscript"),
            Format::Subscript => tr("Subscript"),
            Format::Code => tr("Code"),
        }
    }
}

impl Editor {
    pub(super) fn code_msg(&mut self, m: CodeMsg) {
        match m {
            CodeMsg::Format(f) => self.format(f),
            CodeMsg::SplitTag => self.split_tag_at_caret(),
            CodeMsg::InsertImage => self.open_insert_image(),
            CodeMsg::InsertLink => self.open_insert_link(),
            CodeMsg::ImageDo { href, alt, title } => self.do_insert_image(&href, &alt, &title),
            CodeMsg::LinkDo { href, text, title } => {
                self.do_insert_link(&href, text.as_deref(), &title)
            }
            CodeMsg::SortCss => self.sort_css(),
            CodeMsg::AutoClose => self.auto_close_tag(),
            CodeMsg::CtrlClick(off) => self.ctrl_click(off),
            CodeMsg::GotoDef => self.goto_definition(),
            CodeMsg::RenameClass(name) => self.open_rename_class(&name),
            CodeMsg::RenameDo { old, new } => self.do_rename_class(&old, &new),
            CodeMsg::Help(url) => self.open_url(&url),
            CodeMsg::Hint(o, on) => self.set_hint_option(o, on),
        }
    }

    /// The buffer of the open markup file when the code view is showing.
    pub(super) fn code_buffer(&self) -> Option<gtk::TextBuffer> {
        let p = self.current.as_ref()?;
        (is_text_path(p) && self.w.center.visible_child_name().as_deref() == Some("code"))
            .then(|| self.w.view.buffer())
    }

    /// `(text, caret byte, selection bytes)`.
    pub(super) fn code_state(&self, buf: &gtk::TextBuffer) -> (String, usize, (usize, usize)) {
        let text = buffer_text(buf);
        let cursor = char_to_byte_idx(&text, buf.iter_at_mark(&buf.get_insert()).offset() as usize);
        let sel = buf
            .selection_bounds()
            .map(|(a, b)| {
                (
                    char_to_byte_idx(&text, a.offset() as usize),
                    char_to_byte_idx(&text, b.offset() as usize),
                )
            })
            .unwrap_or((cursor, cursor));
        (text, cursor, sel)
    }

    /// Applies an edit as one undo step and selects `edit.select` of the new text.
    pub(super) fn apply_code_edit(&self, buf: &gtk::TextBuffer, text: &str, edit: &TagEdit) {
        let new_text = edit.apply(text);
        let a = buf.iter_at_offset(byte_to_char_idx(text, edit.range.start) as i32);
        let b = buf.iter_at_offset(byte_to_char_idx(text, edit.range.end) as i32);
        let (mut a, mut b) = (a, b);
        buf.begin_user_action();
        if a != b {
            buf.delete(&mut a, &mut b);
        }
        let mut at = buf.iter_at_offset(byte_to_char_idx(text, edit.range.start) as i32);
        buf.insert(&mut at, &edit.replacement);
        buf.end_user_action();
        let ia = buf.iter_at_offset(byte_to_char_idx(&new_text, edit.select.start) as i32);
        let ib = buf.iter_at_offset(byte_to_char_idx(&new_text, edit.select.end) as i32);
        buf.select_range(&ia, &ib);
        let mut it = ia;
        self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
        self.w.view.grab_focus();
    }

    fn format(&mut self, f: Format) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        if !self.current.as_deref().is_some_and(|p| {
            bookx_editing::markup::is_markup_path(p)
                || matches!(extension(p).as_str(), "xml" | "svg")
        }) {
            self.toast(&tr("Formatting works in HTML pages"));
            return;
        }
        let (text, _, sel) = self.code_state(&buf);
        match cf::toggle_format(&text, sel.0..sel.1, f) {
            Ok(edit) => self.apply_code_edit(&buf, &text, &edit),
            Err(FormatError::Unbalanced) => self.toast(&tr(
                "The selection cuts through tags; select whole elements or plain text",
            )),
            Err(FormatError::NotText) => self.toast(&tr("The cursor is inside a tag or comment")),
        }
    }

    fn split_tag_at_caret(&mut self) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        let (text, cursor, _) = self.code_state(&buf);
        match bookx_editing::code_format::split_tag(&text, cursor) {
            Some(edit) => self.apply_code_edit(&buf, &text, &edit),
            None => self.toast(&tr("The cursor is not inside an element")),
        }
    }

    fn auto_close_tag(&mut self) {
        if !self.hints.prefs.borrow().close_on_slash {
            return;
        }
        let Some(buf) = self.code_buffer() else {
            return;
        };
        if buf.has_selection() {
            return;
        }
        let path = self.current.clone().unwrap_or_default();
        if !(bookx_editing::markup::is_markup_path(&path)
            || matches!(extension(&path).as_str(), "xml" | "svg"))
        {
            return;
        }
        let (text, cursor, _) = self.code_state(&buf);
        if let Some(edit) = cf::auto_close(&text, cursor) {
            // the caret ends after `>`; no selection
            let mut at = buf.iter_at_offset(byte_to_char_idx(&text, cursor) as i32);
            buf.begin_user_action();
            buf.insert(&mut at, &edit.replacement);
            buf.end_user_action();
            *self.hints.pending.borrow_mut() = Some(cf::Pending {
                caret: buf.cursor_position().max(0) as usize,
                rest: edit.replacement.clone(),
            });
        }
    }

    fn sort_css(&mut self) {
        let Some(buf) = self.code_buffer() else {
            return;
        };
        if self.current.as_deref().map(extension).as_deref() != Some("css") {
            self.toast(&tr("Open a style sheet to sort its rules"));
            return;
        }
        let text = buffer_text(&buf);
        match bookx_editing::code_nav::sort_css_rules(&text) {
            Some(sorted) => {
                let cursor = buf.iter_at_mark(&buf.get_insert()).offset();
                buf.begin_user_action();
                let (mut s, mut e) = buf.bounds();
                buf.delete(&mut s, &mut e);
                let mut at = buf.start_iter();
                buf.insert(&mut at, &sorted);
                buf.end_user_action();
                let n = bookx_editing::code_nav::css_rules(&sorted)
                    .iter()
                    .filter(|r| r.top && r.at.is_none())
                    .count();
                buf.place_cursor(&buf.iter_at_offset(cursor.min(buf.char_count())));
                self.toast(&trf("Sorted {} rules by selector", &[&n]));
            }
            None => self.toast(&tr("The rules are already in order")),
        }
    }

    pub(super) fn open_url(&self, url: &str) {
        let launcher = gtk::UriLauncher::new(url);
        launcher.launch(Some(&self.w.win), gio::Cancellable::NONE, |_| {});
    }

    /// Dev hooks `code:`.
    pub(super) fn dev_code(&mut self, c: &str) -> bool {
        let Some(c) = c.strip_prefix("code:") else {
            return false;
        };
        if self.dev_hints(c) || self.dev_navigate(c) || self.dev_inserts(c) {
            return true;
        }
        if let Some(f) = c.strip_prefix("format=") {
            if let Some(f) = Format::from_id(f) {
                self.format(f);
                let buf = self.w.view.buffer();
                eprintln!(
                    "editor_code: format {} toast={}",
                    self.code_dump(&buf),
                    self.last_toast.borrow()
                );
            }
        } else if c == "split" {
            self.split_tag_at_caret();
            eprintln!(
                "editor_code: split {}",
                self.code_dump(&self.w.view.buffer())
            );
        } else if c == "sort-css" {
            self.sort_css();
            eprintln!(
                "editor_code: sort {} toast={}",
                self.code_dump(&self.w.view.buffer()),
                self.last_toast.borrow()
            );
        } else if let Some(t) = c.strip_prefix("autoclose=") {
            // types the text at the caret and lets the auto-close run, as the key handler would
            self.w
                .view
                .buffer()
                .insert_at_cursor(&t.replace("\\n", "\n"));
            self.auto_close_tag();
            eprintln!(
                "editor_code: autoclose {}",
                self.code_dump(&self.w.view.buffer())
            );
        } else if let Some(k) = c.strip_prefix("type=") {
            // a typed character, as the key handler sees it
            let swallowed = try_type_over(&self.w.view, &self.hints, k.chars().next());
            eprintln!("editor_code: type swallowed={swallowed}");
        } else if c == "dump" {
            eprintln!(
                "editor_code: dump {}",
                self.code_dump(&self.w.view.buffer())
            );
        } else {
            return false;
        }
        true
    }

    /// The buffer text with `\n` spelled out and the selection shown as `⟦…⟧` (or the caret as `|`).
    pub(super) fn code_dump(&self, buf: &gtk::TextBuffer) -> String {
        let text = buffer_text(buf);
        let sel = buf
            .selection_bounds()
            .map(|(a, b)| (a.offset(), b.offset()));
        let cur = buf.iter_at_mark(&buf.get_insert()).offset();
        let mut out = String::new();
        for (i, ch) in text.chars().enumerate() {
            let i = i as i32;
            match sel {
                Some((a, _)) if a == i => out.push('⟦'),
                Some((_, b)) if b == i => out.push('⟧'),
                None if cur == i => out.push('|'),
                _ => {}
            }
            if ch == '\n' {
                out.push_str("\\n");
            } else {
                out.push(ch);
            }
        }
        let n = text.chars().count() as i32;
        match sel {
            Some((_, b)) if b == n => out.push('⟧'),
            None if cur == n => out.push('|'),
            _ => {}
        }
        out
    }
}

/// A typed character that repeats what the closing-tag completion inserted is swallowed.
pub(super) fn try_type_over(
    view: &sv::View,
    hints: &markup_hints::Hints,
    typed: Option<char>,
) -> bool {
    let caret = view.buffer().cursor_position().max(0) as usize;
    cf::type_over(&mut hints.pending.borrow_mut(), caret, typed)
}

/// Keys of the code view: `/` after `<` closes the tag (CE-348).
pub(super) fn install_keys(
    view: &sv::View,
    tx: relm4::Sender<Input>,
    hints: Rc<markup_hints::Hints>,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let v = view.clone();
    keys.connect_key_pressed(move |_, key, _, state| {
        let plain =
            !state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK);
        if hints.pending.borrow().is_some() {
            let is_modifier = matches!(
                key,
                gdk::Key::Shift_L
                    | gdk::Key::Shift_R
                    | gdk::Key::Control_L
                    | gdk::Key::Control_R
                    | gdk::Key::Alt_L
                    | gdk::Key::Alt_R
                    | gdk::Key::ISO_Level3_Shift
            );
            if !is_modifier {
                let typed = if plain { key.to_unicode() } else { None };
                if try_type_over(&v, &hints, typed) {
                    return glib::Propagation::Stop;
                }
            }
        }
        if key == gdk::Key::slash && plain {
            let buf = v.buffer();
            let mut it = buf.iter_at_mark(&buf.get_insert());
            if !buf.has_selection() && it.backward_char() && it.char() == '<' {
                tx.emit(Input::Code(CodeMsg::AutoClose));
            }
        }
        glib::Propagation::Proceed
    });
    view.add_controller(keys);
}
