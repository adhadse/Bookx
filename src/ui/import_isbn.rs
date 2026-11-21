//! *Add by ISBN* and *Add Empty Book* (roadmap L14): a record with only metadata, to which the
//! file is added later. The ISBN record is then looked up online by the caller.

use crate::i18n::tr;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use crate::core::opf::Metadata;
use crate::library_add as la;

/// `done(Some(id), message)` after the record exists; the ISBN flow wants a lookup next.
pub type Done = Rc<dyn Fn(u64, String)>;

fn dialog(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    rows: &[&adw::EntryRow],
    error: &gtk::Label,
    add_label: &str,
    on_add: Rc<dyn Fn() -> Result<(), String>>,
) {
    let d = super::dialog_toast::new_dialog();
    d.set_title(title);
    d.set_content_width(440);
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::new();
    for r in rows {
        g.add(*r);
    }
    g.add(error);
    page.add(&g);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let add = gtk::Button::with_label(add_label);
    add.add_css_class("suggested-action");
    add.set_widget_name("record-add");
    header.pack_start(&cancel);
    header.pack_end(&add);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    d.set_child(Some(&toolbar));
    d.set_default_widget(Some(&add));
    {
        let d = d.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let (d, error) = (d.clone(), error.clone());
        add.connect_clicked(move |_| match on_add() {
            Ok(()) => {
                d.close();
            }
            Err(e) => {
                error.set_label(&e);
                error.set_visible(true);
            }
        });
    }
    d.present(Some(parent));
}

fn error_label() -> gtk::Label {
    let l = gtk::Label::new(None);
    l.add_css_class("error");
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_visible(false);
    l
}

/// Asks for an ISBN, adds the record and calls `done`.
pub fn add_by_isbn(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, done: Done) {
    let entry = adw::EntryRow::builder().title(tr("ISBN")).build();
    entry.set_widget_name("record-isbn");
    entry.set_input_purpose(gtk::InputPurpose::Digits);
    let error = error_label();
    let shared = shared.clone();
    let e2 = entry.clone();
    dialog(
        parent,
        &tr("Add by ISBN"),
        &[&entry],
        &error,
        &tr("Add"),
        Rc::new(move || {
            let meta = la::isbn_record(&e2.text()).map_err(|m| tr(&m))?;
            let id = shared
                .lib
                .borrow_mut()
                .add_empty(meta)
                .map_err(|e| format!("{e:#}"))?;
            done(id, tr("Record added; looking it up"));
            Ok(())
        }),
    );
}

/// Asks for a title and authors and adds a record without a file.
pub fn add_empty(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, done: Done) {
    let title = adw::EntryRow::builder().title(tr("Title")).build();
    title.set_widget_name("record-title");
    let authors = adw::EntryRow::builder()
        .title(tr("Authors (separate with &amp;)"))
        .build();
    let error = error_label();
    let shared = shared.clone();
    let (t2, a2) = (title.clone(), authors.clone());
    dialog(
        parent,
        &tr("Add Empty Book"),
        &[&title, &authors],
        &error,
        &tr("Add"),
        Rc::new(move || {
            let meta = Metadata {
                title: t2.text().trim().to_string(),
                authors: super::metadata::split_list(&a2.text(), &['&', ';']),
                ..Default::default()
            };
            if meta.title.is_empty() {
                return Err(tr("A record needs a title"));
            }
            let id = shared
                .lib
                .borrow_mut()
                .add_empty(meta)
                .map_err(|e| format!("{e:#}"))?;
            done(id, tr("Empty book added"));
            Ok(())
        }),
    );
}

/// Test hook: `library:record=empty|Title|Authors` or `library:record=isbn|0441013597`.
pub fn dev_add(shared: &SharedRef, spec: &str) -> Result<u64, String> {
    let mut p = spec.split('|');
    let meta = match p.next() {
        Some("isbn") => la::isbn_record(p.next().unwrap_or(""))?,
        _ => Metadata {
            title: p.next().unwrap_or("").to_string(),
            authors: super::metadata::split_list(p.next().unwrap_or(""), &['&']),
            ..Default::default()
        },
    };
    shared
        .lib
        .borrow_mut()
        .add_empty(meta)
        .map_err(|e| format!("{e:#}"))
}
