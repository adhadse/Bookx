//! Ctrl+F while the preview has the focus searches the preview (CE-086): the find bar of the
//! preview opens, matches are highlighted in the canvas and Enter / Shift+Enter step through them.

use super::*;

/// Whether `focus` is inside `pane` (or is it).
pub fn focus_in(pane: &gtk::Widget, focus: Option<gtk::Widget>) -> bool {
    focus.is_some_and(|f| f == *pane || f.is_ancestor(pane))
}

impl Editor {
    /// Handles Ctrl+F when the preview is focused; `false` leaves it to the code view's find bar.
    pub(super) fn find_in_preview_if_focused(&mut self) -> bool {
        let focus = gtk::prelude::GtkWindowExt::focus(&self.w.win);
        if !focus_in(self.w.preview_pane.upcast_ref(), focus) {
            return false;
        }
        if self.pv.find_rev.reveals_child() {
            self.pv.find_entry.grab_focus();
        } else {
            self.pv_msg(preview::PvMsg::FindToggle);
        }
        true
    }
}
