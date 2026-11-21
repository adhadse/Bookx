//! The *Saved Searches* window: name a search, run it, use it as a virtual library (a restriction
//! on the whole library), delete it. Logic: `crate::saved_searches`.

use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::i18n::tr;
use crate::i18n::trf;
use crate::saved_searches as ss;

/// Changes the library search (the window closes).
pub type Search = Rc<dyn Fn(String)>;
/// Redraws the library and shows a toast.
pub type Notify = Rc<dyn Fn(String)>;

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

pub fn show(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    current_query: &str,
    search: Search,
    notify: Notify,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Saved Searches"));
    dialog.set_content_width(560);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();

    let list = adw::PreferencesGroup::new();
    list.set_title(&tr("Saved Searches"));
    list.set_description(Some(&tr(
        "Run one from here, or type search:name in the search bar.",
    )));
    page.add(&list);
    let empty = adw::PreferencesGroup::new();
    let empty_page = adw::StatusPage::new();
    empty_page.set_icon_name(Some("system-search-symbolic"));
    empty_page.set_title(&tr("No Saved Searches"));
    empty_page.set_description(Some(&tr("Type a search below and save it")));
    empty_page.add_css_class("compact");
    empty.add(&empty_page);
    page.add(&empty);

    let form = adw::PreferencesGroup::new();
    form.set_title(&tr("New Search"));
    let name = adw::EntryRow::new();
    name.set_title(&tr("Name"));
    let query = adw::EntryRow::new();
    query.set_title(&tr("Search"));
    query.set_text(current_query);
    let add = gtk::Button::with_label(&tr("Save Search"));
    add.add_css_class("suggested-action");
    add.set_sensitive(ss::can_save(&name.text(), &query.text()));
    add.set_halign(gtk::Align::End);
    add.set_margin_top(6);
    let message = gtk::Label::new(None);
    message.set_xalign(0.0);
    message.set_wrap(true);
    message.add_css_class("error");
    message.set_visible(false);
    for row in [&name, &query] {
        let add = add.clone();
        let (name, query) = (name.clone(), query.clone());
        row.connect_changed(move |_| add.set_sensitive(ss::can_save(&name.text(), &query.text())));
    }
    for row in [&name, &query] {
        row.set_activates_default(true);
    }
    form.add(&name);
    form.add(&query);
    let tail = gtk::Box::new(gtk::Orientation::Vertical, 4);
    tail.append(&message);
    tail.append(&add);
    form.add(&tail);
    page.add(&form);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let toasts = super::dialog_toast::overlay(&view);

    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Default::default();
    let rebuild: Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>> = Default::default();
    let build = {
        let (shared, list, empty, toasts, rows, rebuild, dialog) = (
            shared.clone(),
            list.clone(),
            empty.clone(),
            toasts.clone(),
            rows.clone(),
            rebuild.clone(),
            dialog.clone(),
        );
        let (search, notify) = (search.clone(), notify.clone());
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                list.remove(&r);
            }
            let (items, active) = {
                let c = shared.cfg.borrow();
                (c.saved_searches.clone(), c.virtual_library.clone())
            };
            list.set_visible(!items.is_empty());
            empty.set_visible(items.is_empty());
            for s in items {
                let row = adw::ActionRow::new();
                row.set_title(&esc(&s.name));
                row.set_subtitle(&esc(&s.query));
                row.set_activatable(true);
                let is_vl = active
                    .as_deref()
                    .is_some_and(|a| a.eq_ignore_ascii_case(&s.name));
                let vl = gtk::ToggleButton::new();
                vl.set_icon_name("view-pin-symbolic");
                vl.set_valign(gtk::Align::Center);
                vl.add_css_class("flat");
                vl.set_active(is_vl);
                vl.set_tooltip_text(Some(&tr(
                    "Use as Virtual Library: narrows every view until switched off",
                )));
                vl.update_property(&[gtk::accessible::Property::Label(&trf(
                    "Use {} as virtual library",
                    &[&s.name],
                ))]);
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.set_valign(gtk::Align::Center);
                del.add_css_class("flat");
                del.set_tooltip_text(Some(&tr("Delete")));
                del.update_property(&[gtk::accessible::Property::Label(&trf(
                    "Delete {}",
                    &[&s.name],
                ))]);
                row.add_suffix(&vl);
                row.add_suffix(&del);
                {
                    let (shared, notify, rebuild, n) = (
                        shared.clone(),
                        notify.clone(),
                        rebuild.clone(),
                        s.name.clone(),
                    );
                    vl.connect_toggled(move |b| {
                        set_virtual(&shared, &n, b.is_active());
                        notify(if b.is_active() {
                            trf("Virtual library: {}", &[&n])
                        } else {
                            tr("Virtual library off")
                        });
                        if let Some(f) = rebuild.borrow().clone() {
                            glib::idle_add_local_once(move || f());
                        }
                    });
                }
                {
                    let (shared, toasts, rebuild, n) = (
                        shared.clone(),
                        toasts.clone(),
                        rebuild.clone(),
                        s.name.clone(),
                    );
                    del.connect_clicked(move |_| {
                        let taken = delete(&shared, &n);
                        let undo_shared = shared.clone();
                        let undo_rebuild = rebuild.clone();
                        super::dialog_toast::show_undo(
                            &toasts,
                            &trf("Deleted “{}”", &[&n]),
                            move || {
                                if let Some(t) = taken.clone() {
                                    undo_delete(&undo_shared, t);
                                }
                                if let Some(f) = undo_rebuild.borrow().clone() {
                                    f();
                                }
                            },
                        );
                        if let Some(f) = rebuild.borrow().clone() {
                            glib::idle_add_local_once(move || f());
                        }
                    });
                }
                {
                    let (search, dialog, n) = (search.clone(), dialog.clone(), s.name.clone());
                    row.connect_activated(move |_| {
                        search(ss::reference_for(&n));
                        dialog.close();
                    });
                }
                list.add(&row);
                rows.borrow_mut().push(row);
            }
        })
    };
    *rebuild.borrow_mut() = Some(build.clone() as Rc<dyn Fn()>);
    build();

    {
        let (shared, notify, name, query, message, build) = (
            shared.clone(),
            notify.clone(),
            name.clone(),
            query.clone(),
            message.clone(),
            build.clone(),
        );
        add.connect_clicked(move |_| {
            match save(&shared, name.text().as_str(), query.text().as_str()) {
                Ok(()) => {
                    message.set_visible(false);
                    notify(trf("Saved “{}”", &[&(name.text().trim())]));
                    name.set_text("");
                    build();
                }
                Err(e) => {
                    message.set_text(&e);
                    message.set_visible(true);
                }
            }
        });
    }

    dialog.set_default_widget(Some(&add));
    dialog.set_child(Some(&toasts));
    dialog.present(Some(parent));
}

/// Adds or replaces a saved search and keeps the settings file in step.
pub fn save(shared: &SharedRef, name: &str, query: &str) -> Result<(), String> {
    {
        let mut c = shared.cfg.borrow_mut();
        ss::upsert(&mut c.saved_searches, name, query)?;
        c.save();
    }
    Ok(())
}

/// Removes a saved search; the result is what `undo_delete` needs (position, search, whether it
/// was the virtual library).
pub fn delete(shared: &SharedRef, name: &str) -> Option<Deleted> {
    let mut c = shared.cfg.borrow_mut();
    let (at, item) = ss::take(&mut c.saved_searches, name)?;
    let was_vl = c
        .virtual_library
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case(name));
    if was_vl {
        c.virtual_library = None;
    }
    c.save();
    Some((at, item, was_vl))
}

/// What `delete` removed: position, the search, and whether it was the virtual library.
pub type Deleted = (usize, ss::SavedSearch, bool);

/// Puts a deleted search back (and the virtual library with it).
pub fn undo_delete(shared: &SharedRef, (at, item, was_vl): Deleted) {
    let mut c = shared.cfg.borrow_mut();
    let name = item.name.clone();
    ss::restore(&mut c.saved_searches, at, item);
    if was_vl {
        c.virtual_library = Some(name);
    }
    c.save();
}

pub fn set_virtual(shared: &SharedRef, name: &str, on: bool) {
    let mut c = shared.cfg.borrow_mut();
    if on {
        c.virtual_library = Some(name.to_string());
    } else if c
        .virtual_library
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case(name))
    {
        c.virtual_library = None;
    }
    c.save();
}

/// Test hook (`library:saved=…`): `open`, `add=Name|query`, `delete=Name`, `vl=Name`, `vl-off`,
/// `dump`.
pub fn dev(
    parent: &gtk::Window,
    shared: &SharedRef,
    search: Search,
    notify: Notify,
    cmd: &str,
) -> bool {
    match cmd {
        "open" => show(parent, shared, "", search, notify),
        "dump" => {
            let c = shared.cfg.borrow();
            for s in &c.saved_searches {
                eprintln!("saved: {} | {}", s.name, s.query);
            }
            eprintln!(
                "saved_vl: {}",
                c.virtual_library.clone().unwrap_or_else(|| "-".into())
            );
        }
        "vl-off" => {
            shared.cfg.borrow_mut().virtual_library = None;
            shared.cfg.borrow().save();
            notify(tr("Virtual library off"));
        }
        c if c.starts_with("add=") => {
            let (n, q) = c[4..].split_once('|').unwrap_or((&c[4..], ""));
            match save(shared, n, q) {
                Ok(()) => notify(trf("Saved “{}”", &[&n])),
                Err(e) => eprintln!("saved_error: {e}"),
            }
        }
        c if c.starts_with("delete=") => {
            delete(shared, &c[7..]);
            notify(tr("Deleted"));
        }
        // `undo-delete=Name|query|index`: puts a deleted search back, as the toast's Undo does
        c if c.starts_with("undo-delete=") => {
            let mut p = c[12..].splitn(3, '|');
            let (n, q) = (p.next().unwrap_or(""), p.next().unwrap_or(""));
            let at = p.next().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
            let item = ss::SavedSearch {
                name: n.into(),
                query: q.into(),
            };
            undo_delete(shared, (at, item, false));
        }
        c if c.starts_with("vl=") => {
            set_virtual(shared, &c[3..], true);
            notify(trf("Virtual library: {}", &[&(&c[3..])]));
        }
        _ => return false,
    }
    true
}
