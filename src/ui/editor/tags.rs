//! Markup editing actions of the Edit menu: matching tags, closing, wrapping, renaming, headings, case.
//! The decisions live in `bookx_editing::tags`; this file only moves text in and out of the buffer.

use super::search::{byte_to_char_idx, char_to_byte_idx};
use super::*;
use crate::i18n::tr;
use bookx_editing::tags::{self, CaseChange, TagEdit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagAction {
    SelectMatching,
    JumpMatching,
    CloseTag,
    Wrap,
    Unwrap,
    Rename,
    Heading(u8),
    Case(CaseChange),
    Comment,
}

impl TagAction {
    /// Stable name for menu targets and dev commands.
    pub fn id(self) -> String {
        match self {
            TagAction::SelectMatching => "select".into(),
            TagAction::JumpMatching => "jump".into(),
            TagAction::CloseTag => "close".into(),
            TagAction::Wrap => "wrap".into(),
            TagAction::Unwrap => "unwrap".into(),
            TagAction::Rename => "rename".into(),
            TagAction::Heading(n) => format!("heading{n}"),
            TagAction::Comment => "comment".into(),
            TagAction::Case(CaseChange::Upper) => "upper".into(),
            TagAction::Case(CaseChange::Lower) => "lower".into(),
            TagAction::Case(CaseChange::Title) => "title".into(),
            TagAction::Case(CaseChange::Swap) => "swap".into(),
        }
    }

    pub fn from_id(id: &str) -> Option<TagAction> {
        Some(match id {
            "select" => TagAction::SelectMatching,
            "jump" => TagAction::JumpMatching,
            "close" => TagAction::CloseTag,
            "wrap" => TagAction::Wrap,
            "unwrap" => TagAction::Unwrap,
            "comment" => TagAction::Comment,
            "rename" => TagAction::Rename,
            "upper" => TagAction::Case(CaseChange::Upper),
            "lower" => TagAction::Case(CaseChange::Lower),
            "title" => TagAction::Case(CaseChange::Title),
            "swap" => TagAction::Case(CaseChange::Swap),
            h => TagAction::Heading(
                h.strip_prefix("heading")?
                    .parse()
                    .ok()
                    .filter(|n| *n <= 6)?,
            ),
        })
    }
}

impl Editor {
    fn markup_buffer(&self) -> Option<gtk::TextBuffer> {
        let p = self.current.as_ref()?;
        (is_text_path(p) && self.w.center.visible_child_name().as_deref() == Some("code"))
            .then(|| self.w.view.buffer())
    }

    pub(super) fn tag_action(&mut self, action: TagAction, arg: Option<String>) {
        let Some(buf) = self.markup_buffer() else {
            return;
        };
        let text = buffer_text(&buf);
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
        let arg = arg.filter(|a| !a.trim().is_empty());
        let edit: Option<TagEdit> = match action {
            TagAction::SelectMatching => {
                match tags::select_to_matching(&text, cursor) {
                    Some(r) => self.select_bytes(&buf, &text, r.start, r.end),
                    None => self.toast(&tr("The cursor is not in a tag pair")),
                }
                return;
            }
            TagAction::JumpMatching => {
                match tags::jump_to_matching(&text, cursor) {
                    Some(p) => self.select_bytes(&buf, &text, p, p),
                    None => self.toast(&tr("No matching tag here")),
                }
                return;
            }
            TagAction::CloseTag => tags::close_tag(&text, cursor),
            TagAction::Wrap => {
                let Some(tag) = arg else {
                    self.prompt_tag(
                        "Wrap in Tag",
                        "Tag name, for example span or div class=\"note\"",
                        action,
                    );
                    return;
                };
                tags::wrap_in_tag(&text, sel.0..sel.1, &tag)
            }
            TagAction::Unwrap => tags::remove_surrounding(&text, cursor),
            TagAction::Rename => {
                let Some(name) = arg else {
                    self.prompt_tag("Rename Tag", "New tag name", action);
                    return;
                };
                tags::rename_tag(&text, cursor, &name)
            }
            TagAction::Heading(n) => tags::change_block_to_heading(&text, cursor, n),
            TagAction::Case(how) => tags::change_case(&text, sel.0..sel.1, how),
            TagAction::Comment => {
                let css = self
                    .current
                    .as_deref()
                    .is_some_and(|p| extension(p) == "css");
                tags::toggle_comment(&text, sel.0..sel.1, css)
            }
        };
        let Some(edit) = edit else {
            self.toast(match action {
                TagAction::CloseTag => "Nothing is open here",
                TagAction::Wrap => "That is not a valid tag name",
                TagAction::Rename => "The cursor is not in a tag pair, or the name is not valid",
                TagAction::Case(_) => "Select some text first",
                TagAction::Comment => "There is nothing to comment out here",
                _ => "The cursor is not in a matching block",
            });
            return;
        };
        let new_text = edit.apply(&text);
        let a = buf.iter_at_offset(byte_to_char_idx(&text, edit.range.start) as i32);
        let b = buf.iter_at_offset(byte_to_char_idx(&text, edit.range.end) as i32);
        let mut a = a;
        buf.begin_user_action();
        let mut b = b;
        buf.delete(&mut a, &mut b);
        let mut at = buf.iter_at_offset(byte_to_char_idx(&text, edit.range.start) as i32);
        buf.insert(&mut at, &edit.replacement);
        buf.end_user_action();
        let keep_selection = matches!(
            action,
            TagAction::Wrap | TagAction::Case(_) | TagAction::Comment
        );
        let (s, e) = if keep_selection {
            (edit.select.start, edit.select.end)
        } else {
            (edit.select.start, edit.select.start)
        };
        self.select_bytes(&buf, &new_text, s, e);
    }

    fn select_bytes(&self, buf: &gtk::TextBuffer, text: &str, a: usize, b: usize) {
        let ia = buf.iter_at_offset(byte_to_char_idx(text, a) as i32);
        let ib = buf.iter_at_offset(byte_to_char_idx(text, b) as i32);
        buf.select_range(&ia, &ib);
        let mut it = ia;
        self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
        self.w.view.grab_focus();
    }

    fn prompt_tag(&self, title: &str, hint: &str, action: TagAction) {
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(hint));
        entry.set_activates_default(true);
        let tx = self.sender.input_sender().clone();
        let e = entry.clone();
        self.ask(title, hint, &entry, "Apply", move || {
            tx.emit(Input::Tag(action, Some(e.text().to_string())))
        });
    }
}
