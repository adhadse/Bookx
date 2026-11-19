//! Snippets beyond Tab in the code view: the *Insert Snippet* list (wraps the selected text),
//! expansion inside the Search & replace fields without tab stops (CE-387), and the scratch view
//! of the Test box in the snippet editor (CE-386).

use super::*;
use crate::editor_snippets::{self as snip, Snippet};

impl Editor {
    /// Lists the snippets that fit the open file; picking one expands it at the caret, around the
    /// selected text when it has a `${N*}` stop.
    pub(super) fn insert_snippet_dialog(&mut self) {
        let Some(path) = self.current.clone().filter(|p| is_text_path(p)) else {
            self.toast(&tr("Open a text file first"));
            return;
        };
        let css = crate::core::util::extension(&path) == "css";
        let all: Vec<Snippet> = self
            .comp
            .borrow()
            .snippets
            .iter()
            .filter(|s| s.applies_to_file(css))
            .cloned()
            .collect();
        let d = adw::Dialog::new();
        d.set_title(&tr("Insert Snippet"));
        d.set_content_width(420);
        d.set_content_height(460);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&tr("Search snippets")));
        crate::ui::audit::label(&search, &tr("Search snippets"));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        crate::ui::audit::label(&list, &tr("Snippets"));
        for s in &all {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&s.name))
                .subtitle(glib::markup_escape_text(&s.trigger))
                .activatable(true)
                .build();
            list.append(&row);
        }
        {
            let (all, search) = (all.clone(), search.clone());
            list.set_filter_func(move |row| {
                let q = search.text().to_lowercase();
                let s = &all[row.index() as usize];
                q.is_empty()
                    || s.name.to_lowercase().contains(&q)
                    || s.trigger.to_lowercase().contains(&q)
            });
        }
        {
            let list = list.clone();
            search.connect_search_changed(move |_| list.invalidate_filter());
        }
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list));
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        col.set_margin_all(12);
        col.append(&search);
        col.append(&scroll);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&col));
        d.set_child(Some(&tb));
        let view = self.w.view.clone();
        let comp = self.comp.clone();
        {
            let (d2, all) = (d.clone(), all.clone());
            list.connect_row_activated(move |_, row| {
                let Some(s) = all.get(row.index() as usize).cloned() else {
                    return;
                };
                d2.close();
                let region = comp.borrow().region.clone();
                let v = view.clone();
                glib::idle_add_local_once(move || {
                    v.grab_focus();
                    completion::expand(&v, &region, &s, 0);
                });
            });
        }
        d.set_focus(Some(&search));
        d.present(Some(&self.w.win));
    }
}

/// Tab in a Search & replace field expands the snippet whose trigger stands before the caret into
/// plain text: tab stops become their default text, nothing jumps (CE-387).
pub fn install_entry(
    entry: &(impl IsA<gtk::Editable> + IsA<gtk::Widget>),
    data: completion::SharedData,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let e = entry.clone().upcast::<gtk::Editable>();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let plain = !mods.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );
        if key != gdk::Key::Tab || !plain {
            return glib::Propagation::Proceed;
        }
        let snippets = data.borrow().snippets.clone();
        if expand_in_entry(&e, &snippets) {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(keys);
}

/// Replaces the trigger before the caret (and the selection after it) by the flat snippet text.
pub fn expand_in_entry(e: &gtk::Editable, snippets: &[Snippet]) -> bool {
    let (sel, end) = match e.selection_bounds() {
        Some((a, b)) => (Some((a, b)), b),
        None => (None, e.position()),
    };
    let start = sel.map(|(a, _)| a).unwrap_or(end);
    let text = e.text().to_string();
    let chars: Vec<char> = text.chars().collect();
    let before: String = chars[..(start as usize).min(chars.len())].iter().collect();
    let Some((s, trig)) = snip::find_for(snippets, &before, false) else {
        return false;
    };
    let selected: String = match sel {
        Some((a, b)) => chars[a as usize..(b as usize).min(chars.len())]
            .iter()
            .collect(),
        None => String::new(),
    };
    let flat = snip::flatten(&s.body, &selected);
    let from = start - trig.chars().count() as i32;
    e.delete_text(from, end);
    let mut pos = from;
    e.insert_text(&flat, &mut pos);
    e.set_position(pos);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_expand_without_a_trigger() {
        let b = snip::builtins();
        // the pure parts are tested in editor_snippets; here only the lookup used by the entry
        assert!(snip::find_for(&b, "plain text ", false).is_none());
        assert!(snip::find_for(&b, "x <i", false).is_some());
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn every_built_in_body_parses_in_gtksourceview() {
        if std::panic::catch_unwind(codeview::init).is_err() {
            return;
        }
        for s in snip::builtins() {
            for sel in ["", "word", "a$b}c\\d"] {
                let body = snip::with_selection(&s.body, sel);
                assert!(
                    sv::Snippet::new_parsed(&body).is_ok(),
                    "{} with {sel:?}: {body}",
                    s.name
                );
            }
        }
    }
}
