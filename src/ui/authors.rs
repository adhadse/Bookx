//! The *Authors* window: people instead of spellings (pen names, `Last, First` against
//! `First Last`), renaming an author on every book with a preview, and merging spellings. A rename
//! is one entry of the operation log. Logic: `crate::authors_view`.

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::rc::Rc;

use bookx_sources::aliases::AuthorGroup;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::authors_view as av;

/// Changes the library search (the window closes).
pub type Search = Rc<dyn Fn(String)>;
/// Saves, redraws, toasts.
pub type Notify = Rc<dyn Fn(String)>;

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// Asks for the new name, shows how many books change, and renames when confirmed.
fn rename_dialog(
    parent: &gtk::Window,
    shared: &SharedRef,
    from: Vec<String>,
    initial: String,
    notify: Notify,
    reopen: Rc<dyn Fn()>,
) {
    let entry = gtk::Entry::new();
    entry.set_text(&initial);
    let preview = gtk::Label::new(None);
    preview.set_wrap(true);
    preview.set_xalign(0.0);
    preview.add_css_class("dim-label");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&entry);
    body.append(&preview);
    let d = adw::AlertDialog::new(
        Some(&tr("Rename author")),
        Some(&tr("Every book that lists one of these spellings gets the new name. Undo takes it all back in one step.")),
    );
    d.set_extra_child(Some(&body));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("rename", &tr("Rename"));
    d.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    let update = {
        let (shared, from, preview, d) = (shared.clone(), from.clone(), preview.clone(), d.clone());
        move |e: &gtk::Entry| {
            let plan = av::preview(&shared.lib.borrow().books, &from, e.text().as_str());
            let titles: Vec<String> = plan.items.iter().take(5).map(|i| i.title.clone()).collect();
            preview.set_text(&format!(
                "{}{}",
                plan.summary(),
                if titles.is_empty() {
                    String::new()
                } else {
                    format!(
                        ": {}{}",
                        titles.join(", "),
                        if plan.items.len() > 5 { ", …" } else { "" }
                    )
                }
            ));
            d.set_response_enabled("rename", !e.text().trim().is_empty());
            eprintln!("authors: preview {}", preview.text());
        }
    };
    update(&entry);
    entry.connect_changed(update);
    let shared = shared.clone();
    d.connect_response(None, move |_, r| {
        if r != "rename" {
            return;
        }
        let to = entry.text().trim().to_string();
        let res = {
            let guard = shared.guard.borrow();
            av::merge(&mut shared.lib.borrow_mut(), &guard.store, &from, &to)
        };
        match res {
            Ok(n) => notify(trf(
                "Renamed on {}. Undo takes it back.",
                &[&ntr("{} book", "{} books", n)],
            )),
            Err(e) => notify(trf("Could not rename: {}", &[&format!("{:#}", e)])),
        }
        reopen();
    });
    d.present(Some(parent));
}

fn group_row(
    parent: &gtk::Window,
    shared: &SharedRef,
    g: &AuthorGroup,
    search: &Search,
    notify: &Notify,
    reopen: &Rc<dyn Fn()>,
    dialog: &adw::Dialog,
) -> adw::ExpanderRow {
    let ex = adw::ExpanderRow::new();
    ex.set_title(&esc(&g.canonical));
    let mut sub = ntr("{} book", "{} books", g.books);
    if g.variants.len() > 1 {
        sub.push_str(&trf(" · {} spellings", &[&g.variants.len()]));
    }
    ex.set_subtitle(&sub);
    let show = gtk::Button::with_label(&tr("Show"));
    show.add_css_class("flat");
    show.set_valign(gtk::Align::Center);
    let rename = gtk::Button::with_label(&tr("Rename"));
    rename.add_css_class("flat");
    rename.set_valign(gtk::Align::Center);
    ex.add_suffix(&show);
    ex.add_suffix(&rename);
    {
        let (q, search, d) = (g.search(), search.clone(), dialog.clone());
        show.connect_clicked(move |_| {
            search(q.clone());
            d.close();
        });
    }
    let spellings: Vec<String> = g.variants.iter().map(|(v, _)| v.clone()).collect();
    {
        let (parent, shared, notify, reopen, from, name) = (
            parent.clone(),
            shared.clone(),
            notify.clone(),
            reopen.clone(),
            spellings.clone(),
            g.canonical.clone(),
        );
        rename.connect_clicked(move |_| {
            rename_dialog(
                &parent,
                &shared,
                from.clone(),
                name.clone(),
                notify.clone(),
                reopen.clone(),
            )
        });
    }
    if g.variants.len() > 1 {
        for (v, n) in &g.variants {
            let r = adw::ActionRow::new();
            r.set_title(&esc(v));
            r.set_subtitle(&ntr("{} book", "{} books", *n));
            if *v != g.canonical || g.variants.len() > 1 {
                let use_btn = gtk::Button::with_label(&tr("Use This Name"));
                use_btn.add_css_class("flat");
                use_btn.set_valign(gtk::Align::Center);
                let (shared, notify, reopen, all, name) = (
                    shared.clone(),
                    notify.clone(),
                    reopen.clone(),
                    spellings.clone(),
                    v.clone(),
                );
                use_btn.connect_clicked(move |_| {
                    let res = {
                        let guard = shared.guard.borrow();
                        av::merge(&mut shared.lib.borrow_mut(), &guard.store, &all, &name)
                    };
                    match res {
                        Ok(n) => notify(trf(
                            "All spellings are now \"{}\" ({} books changed)",
                            &[&name, &n],
                        )),
                        Err(e) => notify(trf("Could not merge: {}", &[&format!("{:#}", e)])),
                    }
                    reopen();
                });
                r.add_suffix(&use_btn);
            }
            ex.add_row(&r);
        }
    }
    ex
}

/// The Authors window.
pub fn show(parent: &gtk::Window, shared: &SharedRef, search: Search, notify: Notify) {
    let page = adw::PreferencesPage::new();
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Authors"));
    dialog.set_content_width(640);
    dialog.set_content_height(560);
    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    let (groups, suggestions) = {
        let lib = shared.lib.borrow();
        let guard = shared.guard.borrow();
        (
            av::groups(&lib.books, &guard.store),
            av::suggestions(&lib.books, &guard.store),
        )
    };
    let reopen: Rc<dyn Fn()> = {
        let (d, parent, shared, search, notify) = (
            dialog.clone(),
            parent.clone(),
            shared.clone(),
            search.clone(),
            notify.clone(),
        );
        Rc::new(move || {
            d.close();
            show(&parent, &shared, search.clone(), notify.clone());
        })
    };
    if !suggestions.is_empty() {
        let g = adw::PreferencesGroup::new();
        g.set_title(&tr("Probably the same person"));
        g.set_description(Some(&tr(
            "Initials against full names. Nothing changes until you press Merge.",
        )));
        for (from, to) in suggestions.iter().take(50) {
            let r = adw::ActionRow::new();
            r.set_title(&esc(&format!("{from}  →  {to}")));
            let b = gtk::Button::with_label(&tr("Merge"));
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            let (shared, notify, reopen, from, to) = (
                shared.clone(),
                notify.clone(),
                reopen.clone(),
                from.clone(),
                to.clone(),
            );
            b.connect_clicked(move |_| {
                let res = {
                    let guard = shared.guard.borrow();
                    av::merge(
                        &mut shared.lib.borrow_mut(),
                        &guard.store,
                        std::slice::from_ref(&from),
                        &to,
                    )
                };
                match res {
                    Ok(n) => notify(trf(
                        "Merged \"{}\" into \"{}\" ({} books)",
                        &[&from, &to, &n],
                    )),
                    Err(e) => notify(trf("Could not merge: {}", &[&format!("{:#}", e)])),
                }
                reopen();
            });
            r.add_suffix(&b);
            g.add(&r);
        }
        page.add(&g);
    }
    let g = adw::PreferencesGroup::new();
    g.set_title(&trf("{} authors", &[&groups.len()]));
    g.set_description(Some(&tr(
        "Grouped by person: spellings and pen names you merged count as one.",
    )));
    if groups.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title(&tr("No authors yet"));
        g.add(&r);
    }
    for gr in groups.iter().take(500) {
        g.add(&group_row(
            parent, shared, gr, &search, &notify, &reopen, &dialog,
        ));
    }
    page.add(&g);
    dialog.present(Some(parent));
}

/// `authors:<command>` of the developer script.
pub fn dev(parent: &gtk::Window, shared: &SharedRef, search: Search, notify: Notify, cmd: &str) {
    let list = |s: &str| -> Vec<String> { s.split('|').map(|x| x.trim().to_string()).collect() };
    match cmd {
        "open" => show(parent, shared, search, notify),
        "dump" => {
            let lib = shared.lib.borrow();
            let guard = shared.guard.borrow();
            for g in av::groups(&lib.books, &guard.store) {
                eprintln!(
                    "authors: {} | books={} | variants={}",
                    g.canonical,
                    g.books,
                    g.variants
                        .iter()
                        .map(|(v, _)| v.as_str())
                        .collect::<Vec<_>>()
                        .join(";")
                );
            }
            for (a, b) in av::suggestions(&lib.books, &guard.store) {
                eprintln!("authors_suggest: {a} -> {b}");
            }
            eprintln!("authors_aliases: {}", guard.store.author_aliases().len());
        }
        // `merge=A|B|C>>Canonical`
        c if c.starts_with("merge=") => {
            if let Some((from, to)) = c[6..].split_once(">>") {
                let res = {
                    let guard = shared.guard.borrow();
                    av::merge(
                        &mut shared.lib.borrow_mut(),
                        &guard.store,
                        &list(from),
                        to.trim(),
                    )
                };
                match res {
                    Ok(n) => eprintln!("authors: merged {n} books"),
                    Err(e) => eprintln!("authors: merge failed: {e:#}"),
                }
                notify(tr("Merged"));
            }
        }
        // `alias=Alias>>Canonical`
        c if c.starts_with("alias=") => {
            if let Some((a, to)) = c[6..].split_once(">>") {
                let r = av::declare_alias(&shared.guard.borrow().store, a.trim(), to.trim());
                eprintln!(
                    "authors: alias {}",
                    if r.is_ok() { "ok" } else { "refused" }
                );
                notify(tr("Alias"));
            }
        }
        // `rename-preview=A|B>>Name` opens the dialog
        c if c.starts_with("rename-preview=") => {
            if let Some((from, to)) = c[15..].split_once(">>") {
                rename_dialog(
                    parent,
                    shared,
                    list(from),
                    to.trim().to_string(),
                    notify,
                    Rc::new(|| {}),
                );
            }
        }
        _ => {}
    }
}
