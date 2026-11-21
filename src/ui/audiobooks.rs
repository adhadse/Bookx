//! Audiobooks linked to books: the *Audiobooks* window (find `.m4b` files and folders of audio
//! parts next to the books or in a folder you name, pair them, link after a preview) and the
//! *Audiobook* row of the details panel with an Open button that hands the item to the system's
//! player. Bookx plays nothing itself. Logic: `crate::audiobook_link`.

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
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

fn link_label(n: usize) -> String {
    trf("Link Selected ({})", &[&n])
}

fn refresh_link(checks: &[gtk::CheckButton], button: &gtk::Button) {
    let n = checks.iter().filter(|c| c.is_active()).count();
    button.set_label(&link_label(n));
    button.set_sensitive(n > 0);
}

/// The proposals whose box is ticked (a missing flag counts as ticked).
fn pick<T: Clone>(all: &[T], on: &[bool]) -> Vec<T> {
    all.iter()
        .enumerate()
        .filter(|(i, _)| on.get(*i).copied().unwrap_or(true))
        .map(|(_, p)| p.clone())
        .collect()
}

/// What the window holds; the search runs on a worker thread and fills this in place.
struct Win {
    parent: gtk::Window,
    shared: SharedRef,
    notify: Notify,
    dialog: adw::Dialog,
    stack: gtk::Stack,
    page: adw::PreferencesPage,
    group: RefCell<Option<adw::PreferencesGroup>>,
    link: gtk::Button,
    extra: RefCell<Vec<PathBuf>>,
    proposals: RefCell<Vec<al::Proposal>>,
    checks: RefCell<Vec<gtk::CheckButton>>,
    /// Bumped by every search so a slow, older one is ignored.
    generation: Cell<u64>,
}

impl Win {
    /// Scans off the UI thread (the spinner page shows meanwhile), then lists the result in place.
    fn reload(self: &Rc<Self>) {
        self.stack.set_visible_child_name("loading");
        self.link.set_sensitive(false);
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let root = self.shared.lib.borrow().root.clone();
        let extra = self.extra.borrow().clone();
        let me = self.clone();
        glib::spawn_future_local(async move {
            let cands = gio::spawn_blocking(move || al::scan_candidates(&root, &extra))
                .await
                .unwrap_or_default();
            if me.generation.get() != generation {
                return;
            }
            let found = al::pair_candidates(&me.shared.lib.borrow(), &cands);
            me.fill(found);
        });
    }

    fn fill(self: &Rc<Self>, proposals: Vec<al::Proposal>) {
        if let Some(old) = self.group.borrow_mut().take() {
            self.page.remove(&old);
        }
        self.checks.borrow_mut().clear();
        if proposals.is_empty() {
            self.proposals.borrow_mut().clear();
            self.link.set_label(&link_label(0));
            self.link.set_sensitive(false);
            self.stack.set_visible_child_name("empty");
            return;
        }
        let g = adw::PreferencesGroup::new();
        g.set_title(&tr("Audiobooks Found"));
        g.set_description(Some(&trf(
            "{} found. Audio files and folders next to the books or in a folder you choose, paired by ISBN, title and author. Bookx only links them; Open starts your audio player.",
            &[&proposals.len()],
        )));
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
            let check = gtk::CheckButton::new();
            check.set_active(true);
            check.set_valign(gtk::Align::Center);
            super::audit::label(&check, &trf("Link {}", &[&p.book_title]));
            r.add_prefix(&check);
            r.set_activatable_widget(Some(&check));
            {
                let me = Rc::downgrade(self);
                check.connect_toggled(move |_| {
                    if let Some(me) = me.upgrade() {
                        refresh_link(&me.checks.borrow(), &me.link);
                    }
                });
            }
            self.checks.borrow_mut().push(check);
            g.add(&r);
        }
        self.page.add(&g);
        *self.group.borrow_mut() = Some(g);
        *self.proposals.borrow_mut() = proposals;
        refresh_link(&self.checks.borrow(), &self.link);
        self.stack.set_visible_child_name("list");
    }

    fn choose_folder(self: &Rc<Self>) {
        let d = gtk::FileDialog::builder()
            .title(tr("Folder with Audiobooks"))
            .build();
        let me = self.clone();
        d.select_folder(Some(&self.parent), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                me.extra.borrow_mut().push(p);
                me.reload();
            }
        });
    }
}

/// The window.
pub fn show(parent: &gtk::Window, shared: &SharedRef, notify: Notify, extra: Vec<PathBuf>) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Audiobooks"));
    dialog.set_content_width(640);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();

    let more = gtk::Button::with_label(&tr("Search Another Folder"));
    more.add_css_class("flat");
    let link = gtk::Button::with_label(&link_label(0));
    link.add_css_class("suggested-action");
    link.set_sensitive(false);
    let header = adw::HeaderBar::new();
    header.pack_start(&more);
    header.pack_end(&link);
    dialog.set_default_widget(Some(&link));

    let loading = adw::StatusPage::new();
    loading.set_title(&tr("Looking for Audiobooks"));
    loading.set_description(Some(&tr(
        "Searching the library folder and the folders you chose.",
    )));
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.set_size_request(32, 32);
    loading.set_child(Some(&spinner));

    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("audio-x-generic-symbolic"));
    empty.set_title(&tr("No Audiobooks Found"));
    empty.set_description(Some(&tr(
        "Books that already have a linked audiobook are not offered again.",
    )));
    let empty_search = gtk::Button::with_label(&tr("Search Another Folder"));
    empty_search.add_css_class("suggested-action");
    empty_search.add_css_class("pill");
    empty_search.set_halign(gtk::Align::Center);
    empty.set_child(Some(&empty_search));

    let stack = gtk::Stack::new();
    stack.add_named(&loading, Some("loading"));
    stack.add_named(&page, Some("list"));
    stack.add_named(&empty, Some("empty"));
    stack.set_visible_child_name("loading");

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&stack));
    dialog.set_child(Some(&view));

    let win = Rc::new(Win {
        parent: parent.clone(),
        shared: shared.clone(),
        notify,
        dialog: dialog.clone(),
        stack,
        page,
        group: RefCell::new(None),
        link: link.clone(),
        extra: RefCell::new(extra),
        proposals: RefCell::new(Vec::new()),
        checks: RefCell::new(Vec::new()),
        generation: Cell::new(0),
    });
    {
        let w = Rc::downgrade(&win);
        link.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            let on: Vec<bool> = w.checks.borrow().iter().map(|c| c.is_active()).collect();
            let list = pick(&w.proposals.borrow(), &on);
            match al::link(&mut w.shared.lib.borrow_mut(), &list) {
                Ok(n) => (w.notify)(trf(
                    "Linked {}. Undo takes each back.",
                    &[&ntr("{} audiobook", "{} audiobooks", n)],
                )),
                Err(e) => (w.notify)(trf("Could not link: {}", &[&format!("{:#}", e)])),
            }
            w.dialog.close();
        });
    }
    for b in [&more, &empty_search] {
        let w = Rc::downgrade(&win);
        b.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.choose_folder();
            }
        });
    }
    DEV.with(|d| *d.borrow_mut() = Some(win.clone()));
    {
        let weak = Rc::downgrade(&win);
        dialog.connect_closed(move |_| {
            // a later search must not fill a window that is gone
            if let Some(w) = weak.upgrade() {
                w.generation.set(w.generation.get() + 1);
            }
            DEV.with(|d| *d.borrow_mut() = None);
        });
    }
    win.reload();
    dialog.present(Some(parent));
}

thread_local! {
    static DEV: RefCell<Option<Rc<Win>>> = const { RefCell::new(None) };
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
        // `unselect=N` unticks proposal N (0-based) of the open window, `state` prints the window
        c if c.starts_with("unselect=") || c == "state" => {
            let n: usize = c
                .strip_prefix("unselect=")
                .map_or(usize::MAX, |v| v.parse().unwrap_or(usize::MAX));
            DEV.with(|d| {
                if let Some(w) = d.borrow().as_ref() {
                    if let Some(c) = w.checks.borrow().get(n) {
                        c.set_active(false);
                    }
                    eprintln!(
                        "audiobooks_window: page={} rows={} link={:?} sensitive={}",
                        w.stack.visible_child_name().unwrap_or_default(),
                        w.checks.borrow().len(),
                        w.link.label().unwrap_or_default(),
                        w.link.is_sensitive()
                    );
                }
            });
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

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn only_ticked_proposals_are_linked() {
        assert_eq!(pick(&["a", "b", "c"], &[true, false, true]), ["a", "c"]);
        assert_eq!(pick(&["a", "b"], &[false, false]), Vec::<&str>::new());
        assert_eq!(pick(&["a", "b"], &[]), ["a", "b"]);
    }
}
