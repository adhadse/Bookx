//! Three windows that help to keep a library in order: *Series* (every series with its books in
//! order, gaps and read progress), *Editions & Formats* (books that are the same work) and *Needs
//! Attention* (weak metadata and identifiers that could be tidied). Logic: `series_view`, `works`,
//! `needs_attention`.

use crate::i18n::tr;
use crate::i18n::{ntr, trf};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::core::opf::format_index;
use crate::{needs_attention as na, series_view, works};

/// Changes the library search (the window closes).
pub type Search = Rc<dyn Fn(String)>;
/// Starts *Fetch Metadata* for these books.
pub type Fetch = Rc<dyn Fn(Vec<u64>)>;
/// Saves, redraws, toasts.
pub type Notify = Rc<dyn Fn(String)>;

/// The books *Fetch Metadata* works on: the ticked ones, or every listed book when none is ticked.
pub fn fetch_targets(all: &[u64], selected: &HashSet<u64>) -> Vec<u64> {
    if selected.is_empty() {
        all.to_vec()
    } else {
        all.iter()
            .copied()
            .filter(|i| selected.contains(i))
            .collect()
    }
}

/// The label of the fetch button: with a selection it says how many books it takes.
pub fn fetch_label(selected: usize) -> String {
    if selected == 0 {
        tr("Fetch Metadata")
    } else {
        ntr(
            "Fetch Metadata for {} Book",
            "Fetch Metadata for {} Books",
            selected as u32,
        )
    }
}

/// What a list shows while rows are ticked: "N selected" (the library's selection title), else the
/// normal description.
pub fn selection_text(selected: usize, normal: &str) -> String {
    if selected == 0 {
        normal.to_string()
    } else {
        trf("{} selected", &[&selected])
    }
}

/// The live state of the Needs Attention list, for the dev hooks.
struct AttState {
    selected: Rc<RefCell<HashSet<u64>>>,
    checks: Vec<(u64, gtk::CheckButton)>,
    group: adw::PreferencesGroup,
    cancel: gtk::Button,
    fetch: gtk::Button,
}

thread_local! {
    static ATT: RefCell<Option<AttState>> = const { RefCell::new(None) };
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn window(title: &str, page: &adw::PreferencesPage, extra: &[&gtk::Button]) -> adw::Dialog {
    let dialog = super::dialog_toast::new_dialog();
    fill_window(&dialog, title, page, extra);
    dialog
}

fn fill_window(
    dialog: &adw::Dialog,
    title: &str,
    page: &adw::PreferencesPage,
    extra: &[&gtk::Button],
) {
    dialog.set_title(title);
    dialog.set_content_width(620);
    dialog.set_content_height(560);
    let header = adw::HeaderBar::new();
    for b in extra {
        header.pack_end(*b);
    }
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(page));
    dialog.set_child(Some(&view));
}

fn empty_group(page: &adw::PreferencesPage, title: &str, text: &str) {
    let g = adw::PreferencesGroup::new();
    let r = adw::ActionRow::new();
    r.set_title(&esc(title));
    r.set_subtitle(&esc(text));
    g.add(&r);
    page.add(&g);
}

/// The Series window.
pub fn show_series(parent: &gtk::Window, shared: &SharedRef, search: Search, fetch: Fetch) {
    let page = adw::PreferencesPage::new();
    let list = series_view::summaries(&shared.lib.borrow().books);
    let dialog = window(&tr("Series"), &page, &[]);
    if list.is_empty() {
        empty_group(
            &page,
            &tr("No series yet"),
            &tr("Books with a series name in their metadata are grouped here, in reading order."),
        );
    }
    for s in list.iter().take(300) {
        let g = adw::PreferencesGroup::new();
        g.set_title(&esc(&s.name));
        let mut desc = vec![s.progress_text()];
        if !s.missing.is_empty() {
            desc.push(s.missing_text());
        }
        desc.extend(s.problems.iter().cloned());
        g.set_description(Some(&esc(&desc.join(" · "))));
        let show = gtk::Button::with_label(&tr("Show"));
        show.add_css_class("flat");
        let fetch_btn = gtk::Button::with_label(&tr("Fetch Metadata"));
        fetch_btn.add_css_class("flat");
        fetch_btn.set_tooltip_text(Some(&tr(
            "Look these books up online; series names and numbers appear in the review",
        )));
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        b.append(&show);
        b.append(&fetch_btn);
        g.set_header_suffix(Some(&b));
        for e in &s.entries {
            let r = adw::ActionRow::new();
            let n = if e.index > 0.0 {
                format!("#{}", format_index(e.index))
            } else {
                "no number".into()
            };
            r.set_title(&esc(&e.title));
            r.set_subtitle(&esc(&n));
            let st = gtk::Label::new(Some(match e.status {
                "finished" => "read",
                "reading" => "reading",
                _ => "unread",
            }));
            st.add_css_class("dim-label");
            r.add_suffix(&st);
            g.add(&r);
        }
        if !s.missing.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&esc(&s.missing_text()));
            r.add_css_class("dim-label");
            g.add(&r);
        }
        page.add(&g);
        let ids: Vec<u64> = s.entries.iter().map(|e| e.id).collect();
        {
            let (d, search, name) = (dialog.clone(), search.clone(), s.name.clone());
            show.connect_clicked(move |_| {
                search(format!("series:=\"{name}\""));
                d.close();
            });
        }
        {
            let (d, fetch) = (dialog.clone(), fetch.clone());
            fetch_btn.connect_clicked(move |_| {
                fetch(ids.clone());
                d.close();
            });
        }
    }
    dialog.present(Some(parent));
}

/// The Editions & Formats window.
pub fn show_works(parent: &gtk::Window, shared: &SharedRef, search: Search, notify: Notify) {
    let dialog = super::dialog_toast::new_dialog();
    let page = works_page(&dialog, parent, shared, search, notify);
    fill_window(&dialog, &tr("Editions & Formats"), &page, &[]);
    dialog.present(Some(parent));
}

/// The Editions & Formats list; `dialog` is what the buttons close (the Library Health window
/// shows this page as one of its tabs).
pub fn works_page(
    dialog: &adw::Dialog,
    parent: &gtk::Window,
    shared: &SharedRef,
    search: Search,
    notify: Notify,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let list = works::group(&shared.lib.borrow().books);
    let note = adw::PreferencesGroup::new();
    note.set_description(Some(&tr(
        "Books with the same title and author, whatever the format or edition. Merge as Formats keeps every file in one book (you see a preview first and can undo it); Find Duplicates merges plain copies.",
    )));
    page.add(&note);
    if list.is_empty() {
        empty_group(
            &page,
            &tr("Every book is its own work"),
            &tr("When two books share a title and author, they are listed together here."),
        );
    }
    for w in list.iter().take(300) {
        let ex = adw::ExpanderRow::new();
        ex.set_title(&esc(&w.title));
        ex.set_subtitle(&esc(&format!("{} · {}", w.authors, w.subtitle())));
        for e in &w.editions {
            let r = adw::ActionRow::new();
            let all = std::iter::once(e.format.clone())
                .chain(e.more.iter().cloned())
                .collect::<Vec<_>>()
                .join(" + ");
            r.set_title(&esc(&format!(
                "{} · {}",
                all,
                crate::library_health::size_text(e.size)
            )));
            let detail: Vec<&str> = [e.publisher.as_str(), e.published.as_str(), e.isbn.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            r.set_subtitle(&esc(&detail.join(" · ")));
            ex.add_row(&r);
        }
        let g = adw::PreferencesGroup::new();
        g.add(&ex);
        page.add(&g);
        let show = gtk::Button::with_label(&tr("Show"));
        show.add_css_class("flat");
        show.set_valign(gtk::Align::Center);
        let dialog = dialog.clone();
        if w.editions.len() > 1 {
            let merge = gtk::Button::with_label(&tr("Merge as Formats"));
            merge.add_css_class("flat");
            merge.set_valign(gtk::Align::Center);
            ex.add_suffix(&merge);
            let (d, shared, notify, parent, ids) = (
                dialog.clone(),
                shared.clone(),
                notify.clone(),
                parent.clone(),
                w.ids(),
            );
            merge.connect_clicked(move |_| {
                d.close();
                crate::ui::formats::show_merge(&parent, &shared, ids.clone(), notify.clone());
            });
        }
        ex.add_suffix(&show);
        let (d, search, title) = (dialog.clone(), search.clone(), w.title.clone());
        show.connect_clicked(move |_| {
            search(format!("title:\"{title}\""));
            d.close();
        });
    }
    page
}

/// The Needs Attention window.
pub fn show_attention(
    parent: &gtk::Window,
    shared: &SharedRef,
    search: Search,
    fetch: Fetch,
    notify: Notify,
) {
    let dialog = super::dialog_toast::new_dialog();
    let (page, fetch_all) = attention_page(&dialog, shared, search, fetch, notify);
    fill_window(&dialog, &tr("Needs Attention"), &page, &[&fetch_all]);
    dialog.present(Some(parent));
}

/// The Needs Attention list and its Fetch Metadata button (the header button of the window, or
/// the button of the Library Health tab).
pub fn attention_page(
    dialog: &adw::Dialog,
    shared: &SharedRef,
    search: Search,
    fetch: Fetch,
    notify: Notify,
) -> (adw::PreferencesPage, gtk::Button) {
    let dialog = dialog.clone();
    let page = adw::PreferencesPage::new();
    let (items, counts, ids) = {
        let lib = shared.lib.borrow();
        (
            na::items(&lib.books),
            na::counts(&lib.books),
            na::identifier_suggestions(&lib.books),
        )
    };
    let fetch_all = gtk::Button::with_label(&tr("Fetch Metadata"));
    fetch_all.add_css_class("suggested-action");
    let att_selected: RefCell<Option<Rc<RefCell<HashSet<u64>>>>> = RefCell::new(None);
    {
        let g = adw::PreferencesGroup::new();
        g.set_title(&tr("Weak metadata"));
        g.set_description(Some(&esc(&counts
            .iter()
            .map(|(w, n)| format!("{n} {}", w.label()))
            .collect::<Vec<_>>()
            .join(" · "))));
        let show = gtk::Button::with_label(&tr("Show in Library"));
        show.add_css_class("flat");
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        cancel.add_css_class("flat");
        cancel.set_visible(false);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        head.append(&cancel);
        head.append(&show);
        g.set_header_suffix(Some(&head));
        let (d, search) = (dialog.clone(), search.clone());
        show.connect_clicked(move |_| {
            search(na::QUERY.to_string());
            d.close();
        });
        if items.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title(&tr("Nothing needs attention"));
            r.set_subtitle(&tr(
                "Every book has an author, a proper title, a cover and a language.",
            ));
            g.add(&r);
        }
        // the library's selection pattern: a check box on every row that shows on hover, focus or
        // once ticked; with a book ticked every box shows, the group says "N selected" and Cancel
        // clears the selection
        let normal = counts
            .iter()
            .map(|(w, n)| format!("{n} {}", w.label()))
            .collect::<Vec<_>>()
            .join(" · ");
        let selected: Rc<RefCell<HashSet<u64>>> = Rc::new(RefCell::new(HashSet::new()));
        let mut checks: Vec<(u64, gtk::CheckButton)> = Vec::new();
        for it in items.iter().take(300) {
            let r = adw::ActionRow::new();
            r.set_title(&esc(&it.title));
            r.set_subtitle(&esc(&it.reasons_text()));
            let check = gtk::CheckButton::new();
            check.set_valign(gtk::Align::Center);
            super::audit::label(&check, &trf("Select {}", &[&it.title]));
            check.set_opacity(0.0);
            r.add_prefix(&check);
            let motion = gtk::EventControllerMotion::new();
            {
                let entered = check.clone();
                motion.connect_enter(move |_, _, _| entered.set_opacity(1.0));
                let (check, selected) = (check.clone(), selected.clone());
                motion.connect_leave(move |_| {
                    if selected.borrow().is_empty() && !check.has_focus() {
                        check.set_opacity(0.0);
                    }
                });
            }
            r.add_controller(motion);
            checks.push((it.id, check));
            g.add(&r);
        }
        let checks_rc = Rc::new(checks.clone());
        let update = {
            let (selected, checks, g, cancel, fetch_all, normal) = (
                selected.clone(),
                checks_rc.clone(),
                g.clone(),
                cancel.clone(),
                fetch_all.clone(),
                normal.clone(),
            );
            Rc::new(move || {
                let n = selected.borrow().len();
                g.set_description(Some(&esc(&selection_text(n, &normal))));
                cancel.set_visible(n > 0);
                fetch_all.set_label(&fetch_label(n));
                for (id, c) in checks.iter() {
                    c.set_opacity(if n > 0 || selected.borrow().contains(id) {
                        1.0
                    } else {
                        0.0
                    });
                }
            })
        };
        for (id, c) in checks_rc.iter() {
            let (selected, update, id) = (selected.clone(), update.clone(), *id);
            c.connect_toggled(move |c| {
                if c.is_active() {
                    selected.borrow_mut().insert(id);
                } else {
                    selected.borrow_mut().remove(&id);
                }
                update();
            });
        }
        {
            let (checks, selected, update) = (checks_rc.clone(), selected.clone(), update.clone());
            cancel.connect_clicked(move |_| {
                selected.borrow_mut().clear();
                for (_, c) in checks.iter() {
                    c.set_active(false);
                }
                update();
            });
        }
        att_selected.replace(Some(selected.clone()));
        ATT.with(|a| {
            *a.borrow_mut() = Some(AttState {
                selected: selected.clone(),
                checks: checks_rc.as_ref().clone(),
                group: g.clone(),
                cancel: cancel.clone(),
                fetch: fetch_all.clone(),
            })
        });
        page.add(&g);
    }
    if !ids.is_empty() {
        let g = adw::PreferencesGroup::new();
        g.set_title(&tr("Identifiers"));
        g.set_description(Some(&tr(
            "Suggestions only; nothing changes until you press Clean Up.",
        )));
        let clean = gtk::Button::with_label(&tr("Clean Up"));
        clean.add_css_class("flat");
        g.set_header_suffix(Some(&clean));
        for (_, title, sugg) in ids.iter().take(200) {
            for s in sugg {
                let r = adw::ActionRow::new();
                r.set_title(&esc(title));
                r.set_subtitle(&esc(&format!(
                    "{}{}",
                    s.text,
                    if s.fixable() {
                        ""
                    } else {
                        " (needs your attention)"
                    }
                )));
                g.add(&r);
            }
        }
        page.add(&g);
        let (shared, d, notify) = (shared.clone(), dialog.clone(), notify.clone());
        clean.connect_clicked(move |_| {
            let n = {
                let mut lib = shared.lib.borrow_mut();
                let guard = shared.guard.borrow();
                na::clean_identifiers(&mut lib, &guard.store, &[], chrono::Utc::now().timestamp())
            };
            eprintln!("attention: cleaned {n}");
            notify(trf("Identifiers tidied in {} books", &[&n]));
            d.close();
        });
    }
    {
        let ids: Vec<u64> = items.iter().map(|i| i.id).collect();
        let d = dialog.clone();
        let selected = att_selected.take();
        fetch_all.connect_clicked(move |_| {
            let picked = selected.as_ref().map(|s| s.borrow().clone());
            fetch(fetch_targets(&ids, &picked.unwrap_or_default()));
            d.close();
        });
    }
    (page, fetch_all)
}

/// `organize:<command>` of the developer script.
pub fn dev(
    parent: &gtk::Window,
    shared: &SharedRef,
    search: Search,
    fetch: Fetch,
    notify: Notify,
    cmd: &str,
) {
    match cmd {
        "series" => show_series(parent, shared, search, fetch),
        "works" => show_works(parent, shared, search, notify),
        "attention" => show_attention(parent, shared, search, fetch, notify),
        "series-dump" => {
            for s in series_view::summaries(&shared.lib.borrow().books) {
                eprintln!(
                    "series: {} | {} | {} | order={} | problems={}",
                    s.name,
                    s.progress_text(),
                    if s.missing.is_empty() {
                        "complete".to_string()
                    } else {
                        s.missing_text()
                    },
                    s.entries
                        .iter()
                        .map(|e| format!("{}={}", format_index(e.index), e.title))
                        .collect::<Vec<_>>()
                        .join(","),
                    s.problems.join(";")
                );
            }
        }
        "works-dump" => {
            let w = works::group(&shared.lib.borrow().books);
            eprintln!("works: {}", w.len());
            for x in &w {
                eprintln!(
                    "work: {} | {} | ids={:?}",
                    x.title,
                    x.subtitle(),
                    x.editions.iter().map(|e| e.id).collect::<Vec<_>>()
                );
            }
        }
        // `attention-tick=N` ticks (or unticks) the N-th row (1-based); `attention-cancel` presses Cancel;
        // `attention-state` prints `attention_sel: selected=N description=.. cancel=B fetch=.. visible_checks=V`
        c if c.starts_with("attention-tick=") => {
            let n: usize = c[15..].parse().unwrap_or(1);
            let check = ATT.with(|a| {
                a.borrow()
                    .as_ref()
                    .and_then(|s| s.checks.get(n.saturating_sub(1)).map(|c| c.1.clone()))
            });
            if let Some(c) = check {
                c.set_active(!c.is_active());
            }
        }
        "attention-cancel" => {
            let b = ATT.with(|a| a.borrow().as_ref().map(|s| s.cancel.clone()));
            if let Some(b) = b {
                b.emit_clicked();
            }
        }
        "attention-state" => ATT.with(|a| match a.borrow().as_ref() {
            Some(s) => eprintln!(
                "attention_sel: selected={} description={:?} cancel={} fetch={:?} visible_checks={} checks={}",
                s.selected.borrow().len(),
                s.group.description().unwrap_or_default(),
                s.cancel.is_visible(),
                s.fetch.label().unwrap_or_default(),
                s.checks.iter().filter(|c| c.1.opacity() > 0.5).count(),
                s.checks.len()
            ),
            None => eprintln!("attention_sel: closed"),
        }),
        "attention-dump" => {
            let lib = shared.lib.borrow();
            let items = na::items(&lib.books);
            eprintln!("attention: {} books", items.len());
            for i in &items {
                eprintln!("attention_book: {} | {}", i.title, i.reasons_text());
            }
            for (id, t, s) in na::identifier_suggestions(&lib.books) {
                for x in s {
                    eprintln!(
                        "attention_id: {id} {t} | {} | fixable={}",
                        x.text,
                        x.fixable()
                    );
                }
            }
        }
        "idclean" => {
            let n = {
                let mut lib = shared.lib.borrow_mut();
                let guard = shared.guard.borrow();
                na::clean_identifiers(&mut lib, &guard.store, &[], chrono::Utc::now().timestamp())
            };
            eprintln!("attention: cleaned {n}");
            notify(trf("Identifiers tidied in {} books", &[&n]));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_takes_the_ticked_books_or_all_of_them() {
        let all = [1, 2, 3, 4];
        assert_eq!(fetch_targets(&all, &HashSet::new()), vec![1, 2, 3, 4]);
        assert_eq!(fetch_targets(&all, &HashSet::from([4, 2])), vec![2, 4]);
        assert_eq!(fetch_targets(&all, &HashSet::from([9])), Vec::<u64>::new());
    }

    #[test]
    fn the_group_says_how_many_are_selected() {
        assert_eq!(selection_text(0, "3 no cover"), "3 no cover");
        assert_eq!(selection_text(2, "3 no cover"), "2 selected");
        assert_eq!(fetch_label(0), "Fetch Metadata");
        assert_eq!(fetch_label(1), "Fetch Metadata for 1 Book");
        assert_eq!(fetch_label(3), "Fetch Metadata for 3 Books");
    }
}
