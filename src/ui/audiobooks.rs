//! Audiobooks linked to books: the *Audiobooks* window (find `.m4b` files and folders of audio
//! parts next to the books or in a folder you name, pair them, link after a preview) and the
//! *Audiobook* row of the details panel with an Open button that hands the item to the system's
//! player. Bookx plays nothing itself. Logic: `crate::audiobook_link`.

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::SharedRef;
use crate::audiobook_link as al;
use crate::core::formats::AUDIOBOOK;
use crate::core::library::Book;

/// Saves, redraws, toasts.
pub type Notify = Rc<dyn Fn(String)>;

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

fn open_item(path: &str) {
    super::open_uri(&gio::File::for_path(path).uri());
}

/// The audiobook rows of the details panel; `None` when the book has none.
pub fn details_group(
    b: &Book,
    changed: Notify,
    shared: &SharedRef,
) -> Option<adw::PreferencesGroup> {
    let items: Vec<_> = b.resources.iter().filter(|r| r.kind == AUDIOBOOK).collect();
    if items.is_empty() {
        return None;
    }
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Audiobook"));
    g.set_margin_top(10);
    for r in items {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&r.label));
        row.set_subtitle(&esc(&al::describe(r)));
        let open = gtk::Button::with_label(&tr("Open"));
        open.add_css_class("flat");
        open.set_valign(gtk::Align::Center);
        open.set_sensitive(std::path::Path::new(&r.path).exists());
        let path = r.path.clone();
        open.connect_clicked(move |_| open_item(&path));
        let unlink = gtk::Button::with_label(&tr("Unlink"));
        unlink.add_css_class("flat");
        unlink.set_valign(gtk::Align::Center);
        unlink.set_tooltip_text(Some(&tr(
            "Takes the link off; the files stay where they are",
        )));
        let (shared, changed, id, path) = (shared.clone(), changed.clone(), b.id, r.path.clone());
        unlink.connect_clicked(move |_| {
            match al::unlink(&mut shared.lib.borrow_mut(), id, &path) {
                Ok(()) => changed("Audiobook unlinked".into()),
                Err(e) => changed(trf("Could not unlink: {}", &[&format!("{:#}", e)])),
            }
        });
        row.add_suffix(&open);
        row.add_suffix(&unlink);
        g.add(&row);
    }
    Some(g)
}

/// The window.
pub fn show(parent: &gtk::Window, shared: &SharedRef, notify: Notify, extra: Vec<PathBuf>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Audiobooks"));
    dialog.set_content_width(640);
    dialog.set_content_height(640);
    let page = adw::PreferencesPage::new();
    let proposals = al::find(&shared.lib.borrow(), &extra);
    let g = adw::PreferencesGroup::new();
    g.set_title(&trf("{} found", &[&proposals.len()]));
    g.set_description(Some(
        "Audio files and folders next to your books or in a folder you choose, paired by ISBN, title and author. Bookx only links them; Open starts your audio player.",
    ));
    let more = gtk::Button::with_label(&tr("Search Another Folder"));
    more.add_css_class("flat");
    let link_all = gtk::Button::with_label(&tr("Link All"));
    link_all.add_css_class("suggested-action");
    link_all.set_sensitive(!proposals.is_empty());
    let hb = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hb.append(&more);
    hb.append(&link_all);
    g.set_header_suffix(Some(&hb));
    if proposals.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title(&tr("No new audiobooks found"));
        r.set_subtitle(&tr(
            "Books that already have a linked audiobook are not offered again.",
        ));
        g.add(&r);
    }
    for p in &proposals {
        let r = adw::ActionRow::new();
        r.set_title(&esc(&p.book_title));
        r.set_subtitle(&esc(&trf(
            "{} · {} · {}",
            &[
                &p.candidate.path.display(),
                &ntr("{} file", "{} files", p.candidate.files),
                &p.reason,
            ],
        )));
        g.add(&r);
    }
    page.add(&g);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    let proposals = Rc::new(RefCell::new(proposals));
    {
        let (shared, notify, dialog, proposals) = (
            shared.clone(),
            notify.clone(),
            dialog.clone(),
            proposals.clone(),
        );
        link_all.connect_clicked(move |_| {
            let list = proposals.borrow().clone();
            match al::link(&mut shared.lib.borrow_mut(), &list) {
                Ok(n) => notify(trf(
                    "Linked {}. Undo takes each back.",
                    &[&ntr("{} audiobook", "{} audiobooks", n)],
                )),
                Err(e) => notify(trf("Could not link: {}", &[&format!("{:#}", e)])),
            }
            dialog.close();
        });
    }
    {
        let (parent, shared, notify, dialog, extra) = (
            parent.clone(),
            shared.clone(),
            notify.clone(),
            dialog.clone(),
            extra,
        );
        more.connect_clicked(move |_| {
            let d = gtk::FileDialog::builder()
                .title(tr("Folder with Audiobooks"))
                .build();
            let (parent2, shared, notify, dialog, extra) = (
                parent.clone(),
                shared.clone(),
                notify.clone(),
                dialog.clone(),
                extra.clone(),
            );
            d.select_folder(Some(&parent), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    let mut e = extra.clone();
                    e.push(p);
                    dialog.close();
                    show(&parent2, &shared, notify.clone(), e);
                }
            });
        });
    }
    dialog.present(Some(parent));
}

/// `audiobooks:<command>` of the developer script.
pub fn dev(parent: &gtk::Window, shared: &SharedRef, notify: Notify, cmd: &str) {
    match cmd {
        "open" => show(parent, shared, notify, Vec::new()),
        c if c.starts_with("open=") => show(parent, shared, notify, vec![PathBuf::from(&c[5..])]),
        // `find=/dir` prints the proposals, `link=/dir` links them all
        c if c.starts_with("find=") || c.starts_with("link=") => {
            let link = c.starts_with("link=");
            let dir = PathBuf::from(&c[5..]);
            let found = al::find(&shared.lib.borrow(), std::slice::from_ref(&dir));
            for p in &found {
                eprintln!(
                    "audiobook: book={:?} path={} reason={:?}",
                    p.book_title,
                    p.candidate.path.display(),
                    p.reason
                );
            }
            eprintln!("audiobook: {} found", found.len());
            if link {
                match al::link(&mut shared.lib.borrow_mut(), &found) {
                    Ok(n) => eprintln!("audiobook: linked {n}"),
                    Err(e) => eprintln!("audiobook: link failed {e:#}"),
                }
                notify(tr("Linked"));
            }
        }
        "dump" => {
            for b in &shared.lib.borrow().books {
                for r in b.resources.iter().filter(|r| r.kind == AUDIOBOOK) {
                    eprintln!(
                        "audiobook_link: book={:?} {}",
                        b.meta.title,
                        al::describe(r)
                    );
                }
            }
        }
        _ => {}
    }
}
