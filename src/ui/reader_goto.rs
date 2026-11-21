//! The reader's "Go to…" dialog: a page, a printed page, a percent, a location or a chapter, with a
//! live line that says where it leads. The parsing is `crate::goto`.

use crate::i18n::tr;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

/// `check` returns the line under the field for the typed text (Err = why it does not work).
/// `go` is called with the text when the user confirms. `hint` says what the book has.
pub fn show(
    parent: &gtk::Widget,
    hint: &str,
    check: Rc<dyn Fn(&str) -> Result<String, String>>,
    go: Rc<dyn Fn(String)>,
) -> adw::AlertDialog {
    show_with(
        parent,
        &tr("Page, percent, location or chapter"),
        &tr("Go to page, percent, location or chapter"),
        hint,
        check,
        go,
    )
}

/// The same dialog with the field's placeholder and accessible name given (a comic only has pages).
pub fn show_with(
    parent: &gtk::Widget,
    placeholder: &str,
    name: &str,
    hint: &str,
    check: Rc<dyn Fn(&str) -> Result<String, String>>,
    go: Rc<dyn Fn(String)>,
) -> adw::AlertDialog {
    let d = adw::AlertDialog::new(Some(&tr("Go to")), Some(hint));
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_activates_default(true);
    super::audit::label(&entry, name);
    let line = gtk::Label::new(None);
    line.set_wrap(true);
    line.set_xalign(0.0);
    line.add_css_class("caption");
    line.add_css_class("dim-label");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&entry);
    body.append(&line);
    d.set_extra_child(Some(&body));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("go", &tr("Go"));
    d.set_response_appearance("go", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("go"));
    d.set_close_response("cancel");
    d.set_response_enabled("go", false);
    {
        let (d, line, check) = (d.clone(), line.clone(), check.clone());
        entry.connect_changed(move |e| {
            let text = e.text().to_string();
            if text.trim().is_empty() {
                line.set_text("");
                e.remove_css_class("error");
                d.set_response_enabled("go", false);
                return;
            }
            match check(&text) {
                Ok(msg) => {
                    line.set_text(&msg);
                    line.remove_css_class("error");
                    e.remove_css_class("error");
                    d.set_response_enabled("go", true);
                }
                Err(msg) => {
                    line.set_text(&msg);
                    line.add_css_class("error");
                    e.add_css_class("error");
                    d.set_response_enabled("go", false);
                }
            }
        });
    }
    {
        let entry = entry.clone();
        d.connect_response(None, move |_, r| {
            if r == "go" {
                go(entry.text().to_string());
            }
        });
    }
    d.present(Some(parent));
    entry.grab_focus();
    d
}
