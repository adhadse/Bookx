//! The *Find Duplicates* window: groups of books that look like the same one, with a choice of
//! which copy to keep. Merging unites metadata, notes and reading progress.

use crate::i18n::tr;
use crate::i18n::trf;
use std::cell::Cell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use crate::duplicates::{self, Group};

pub fn show(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, on_changed: Rc<dyn Fn(String)>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Duplicate Books"));
    dialog.set_content_width(560);
    dialog.set_content_height(620);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page(shared, on_changed)));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// The list of duplicate groups with their Merge buttons (the Duplicates tab of Library Health).
pub fn page(shared: &SharedRef, on_changed: Rc<dyn Fn(String)>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let groups: Vec<Group> = duplicates::find(&shared.lib.borrow().books);
    let status = adw::StatusPage::new();
    status.set_icon_name(Some("object-select-symbolic"));
    status.set_title(&tr("No duplicates found"));
    status.set_description(Some(
        "Books with the same title and author, or the same ISBN, would show up here.",
    ));
    status.add_css_class("compact");
    let remaining = Rc::new(Cell::new(groups.len()));
    status.set_visible(groups.is_empty());

    for g in groups {
        let (title, rows): (String, Vec<(u64, String, String)>) = {
            let lib = shared.lib.borrow();
            let t = lib
                .get(g.keep)
                .map(|b| b.meta.title.clone())
                .unwrap_or_default();
            let rows = g
                .ids
                .iter()
                .filter_map(|id| lib.get(*id))
                .map(|b| (b.id, b.meta.title.clone(), duplicates::describe(b)))
                .collect();
            (t, rows)
        };
        let group = adw::PreferencesGroup::new();
        group.set_title(&relm4::gtk::glib::markup_escape_text(&title));
        // copies that differ in format are one book in several files: keep them all
        let formats = g.differ_in_format(&shared.lib.borrow().books);
        group.set_description(Some(&if formats {
            format!(
                "{} files in different formats. Merge keeps every file as a format of one book.",
                rows.len()
            )
        } else {
            format!("{} copies. Pick the one to keep.", rows.len())
        }));
        let keep = Rc::new(Cell::new(g.keep));
        let merge_btn = gtk::Button::with_label(
            &(if formats {
                tr("Merge Formats")
            } else {
                tr("Merge")
            }),
        );
        merge_btn.add_css_class("suggested-action");
        merge_btn.set_valign(gtk::Align::Center);
        group.set_header_suffix(Some(&merge_btn));
        let mut first: Option<gtk::CheckButton> = None;
        for (id, t, sub) in &rows {
            let row = adw::ActionRow::new();
            row.set_title(&relm4::gtk::glib::markup_escape_text(t));
            row.set_subtitle(&relm4::gtk::glib::markup_escape_text(sub));
            let radio = gtk::CheckButton::new();
            match &first {
                Some(f) => radio.set_group(Some(f)),
                None => first = Some(radio.clone()),
            }
            radio.set_active(*id == g.keep);
            let (keep, id) = (keep.clone(), *id);
            radio.connect_toggled(move |r| {
                if r.is_active() {
                    keep.set(id);
                }
            });
            row.add_prefix(&radio);
            row.set_activatable_widget(Some(&radio));
            group.add(&row);
        }
        page.add(&group);
        {
            let (shared, ids, keep, group, status, remaining, on_changed) = (
                shared.clone(),
                g.ids.clone(),
                keep,
                group.clone(),
                status.clone(),
                remaining.clone(),
                on_changed.clone(),
            );
            merge_btn.connect_clicked(move |b| {
                let res = if formats {
                    let others: Vec<u64> =
                        ids.iter().copied().filter(|i| *i != keep.get()).collect();
                    let guard = shared.guard.borrow();
                    crate::book_formats::merge(
                        &mut shared.lib.borrow_mut(),
                        &guard.store,
                        keep.get(),
                        &others,
                    )
                } else {
                    duplicates::merge(&mut shared.lib.borrow_mut(), &ids, keep.get())
                };
                match res {
                    Ok(n) => {
                        group.set_visible(false);
                        b.set_sensitive(false);
                        remaining.set(remaining.get().saturating_sub(1));
                        status.set_visible(remaining.get() == 0);
                        on_changed(if formats {
                            trf("Merged {} files as formats of one book", &[&(n + 1)])
                        } else {
                            trf("Merged {} copies into one", &[&(n + 1)])
                        });
                    }
                    Err(e) => on_changed(trf("Could not merge: {}", &[&format!("{:#}", e)])),
                }
            });
        }
    }
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&status);
    wrap.append(&page);
    page.set_vexpand(true);
    wrap.upcast()
}
