//! Grouped editions in the library (roadmap PP-G-5): the edition-count badge on a cover, the list of
//! editions in a popover and in the details panel, with Open, Send, Prefer and Unlink per edition.
//! The grouping is `crate::work_groups`; the catalogue side is `bookx_store` (`work_links`).

use std::rc::Rc;

use relm4::adw::prelude::*;
use relm4::gtk::{self, glib};

use super::covers;
use crate::core::library::{Book, Library};
use crate::i18n::{ntr, tr, trf};
use crate::work_groups::Card;

/// What the buttons of an edition row can ask for.
#[derive(Clone)]
pub struct EditionActions {
    pub read: Rc<dyn Fn(u64)>,
    pub send: Rc<dyn Fn(u64)>,
    pub prefer: Rc<dyn Fn(u64)>,
    pub unlink: Rc<dyn Fn(u64)>,
    /// A device is connected, so Send can do something.
    pub can_send: bool,
}

/// What a row shows about one edition (pure, so it is tested).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditionInfo {
    pub id: u64,
    pub title: String,
    /// `EPUB`, or `EPUB, AZW3` when the book holds several files.
    pub formats: String,
    pub language: String,
    pub publisher: String,
    pub size: u64,
    /// Percent read, 0 when never opened.
    pub percent: u32,
    /// The edition that opens first.
    pub opens_first: bool,
    /// The edition whose progress the card shows.
    pub read_last: bool,
}

impl EditionInfo {
    pub fn of(b: &Book, card: &Card) -> EditionInfo {
        let mut formats: Vec<String> = b
            .all_formats()
            .iter()
            .map(|f| f.format.to_uppercase())
            .collect();
        formats.dedup();
        EditionInfo {
            id: b.id,
            title: b.meta.title.clone(),
            formats: formats.join(", "),
            language: b.meta.language.clone(),
            publisher: b.meta.publisher.clone(),
            size: b.size,
            percent: (b.progress * 100.0).round() as u32,
            opens_first: card.rep == b.id,
            read_last: card.progress_from == b.id && b.progress > 0.0 && card.is_group(),
        }
    }

    /// `EPUB · en · 1.2 MB`
    pub fn line(&self) -> String {
        let mut parts = vec![self.formats.clone()];
        if !self.language.trim().is_empty() {
            parts.push(self.language.trim().to_string());
        }
        parts.push(crate::library_health::size_text(self.size));
        parts.join(" · ")
    }

    /// `Pub B · Opens first · 40% read`: the publisher tells editions of one format apart.
    pub fn status(&self) -> String {
        let mut parts = Vec::new();
        if !self.publisher.trim().is_empty() {
            parts.push(self.publisher.trim().to_string());
        }
        if self.opens_first {
            parts.push(tr("Opens first"));
        }
        if self.percent > 0 {
            parts.push(trf("{}% read", &[&self.percent]));
        }
        if self.read_last {
            parts.push(tr("read last"));
        }
        parts.join(" · ")
    }

    /// The name a screen reader gives the badge-sized buttons of the row.
    pub fn name(&self) -> String {
        format!("{} ({})", self.title, self.formats)
    }
}

/// The badge text and its accessible name for a card of `n` editions.
pub fn badge_text(n: usize) -> (String, String) {
    (
        n.to_string(),
        trf(
            "{}: show the list",
            &[&ntr("{} edition", "{} editions", n as u32)],
        ),
    )
}

/// The small chip on a cover that opens the edition list: the stack icon and the count. Hidden
/// for a single book.
pub fn make_badge() -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("edition-badge");
    b.set_halign(gtk::Align::End);
    b.set_valign(gtk::Align::End);
    b.set_margin_end(6);
    b.set_margin_bottom(6);
    b.set_focus_on_click(false);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let icon = gtk::Image::from_icon_name("view-dual-symbolic");
    icon.set_pixel_size(12);
    let count = gtk::Label::new(None);
    count.add_css_class("numeric");
    row.append(&icon);
    row.append(&count);
    b.set_child(Some(&row));
    b.set_visible(false);
    b
}

/// Shows the number of editions on the badge (hides it for one).
pub fn set_badge(b: &gtk::Button, n: usize) {
    b.set_visible(n > 1);
    if n > 1 {
        let (text, name) = badge_text(n);
        if let Some(l) = b
            .child()
            .and_then(|c| c.last_child())
            .and_downcast::<gtk::Label>()
        {
            l.set_label(&text);
        }
        b.set_tooltip_text(Some(&name));
        super::audit::label(b, &name);
    }
}

fn icon_button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tip));
    super::audit::label(&b, tip);
    b
}

/// One row: cover, format line, status, and the buttons. `done` runs after a button was used (the
/// popover closes).
fn edition_row(
    lib: &Library,
    b: &Book,
    info: &EditionInfo,
    acts: &EditionActions,
    can_unlink: bool,
    compact: bool,
    done: Rc<dyn Fn()>,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_margin_top(6);
    line.set_margin_bottom(6);
    line.set_margin_start(8);
    line.set_margin_end(8);
    let pic = gtk::Picture::new();
    pic.set_size_request(40, 60);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_can_shrink(true);
    pic.add_css_class("book-cover");
    pic.set_valign(gtk::Align::Center);
    pic.set_halign(gtk::Align::Start);
    if let Some(t) = lib.cover_path(b).and_then(|p| covers::thumbnail(&p, 20)) {
        pic.set_paintable(Some(&t));
    }
    line.append(&pic);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&info.line()));
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_lines(2);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.add_css_class("heading");
    let status = info.status();
    let sub = gtk::Label::new(Some(if status.is_empty() {
        &info.title
    } else {
        &status
    }));
    sub.set_xalign(0.0);
    sub.set_wrap(true);
    sub.add_css_class("dim-label");
    sub.add_css_class("caption");
    text.append(&title);
    text.append(&sub);
    line.append(&text);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    buttons.set_halign(gtk::Align::End);

    let (id, name) = (info.id, info.name());
    // a child label (not `with_label`) so that the accessible name below wins
    let open = gtk::Button::new();
    open.set_child(Some(&gtk::Label::new(Some(&tr("Open")))));
    open.add_css_class("flat");
    open.set_valign(gtk::Align::Center);
    open.set_tooltip_text(Some(&trf("Open {}", &[&name])));
    super::audit::label(&open, &trf("Open {}", &[&name]));
    {
        let (f, done) = (acts.read.clone(), done.clone());
        open.connect_clicked(move |_| {
            done();
            f(id);
        });
    }
    buttons.append(&open);

    let send = icon_button("document-send-symbolic", &trf("Send {}", &[&name]));
    send.set_sensitive(acts.can_send);
    {
        let (f, done) = (acts.send.clone(), done.clone());
        send.connect_clicked(move |_| {
            done();
            f(id);
        });
    }
    buttons.append(&send);

    let prefer = icon_button("object-select-symbolic", &trf("Open {} first", &[&name]));
    prefer.set_sensitive(!info.opens_first);
    {
        let (f, done) = (acts.prefer.clone(), done.clone());
        prefer.connect_clicked(move |_| {
            done();
            f(id);
        });
    }
    buttons.append(&prefer);

    let unlink = icon_button("list-remove-symbolic", &trf("Unlink {}", &[&name]));
    unlink.set_sensitive(can_unlink);
    {
        let (f, done) = (acts.unlink.clone(), done);
        unlink.connect_clicked(move |_| {
            done();
            f(id);
        });
    }
    buttons.append(&unlink);
    if compact {
        // the details panel is narrow: the buttons go under the text
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.append(&line);
        buttons.set_margin_end(8);
        buttons.set_margin_bottom(4);
        col.append(&buttons);
        row.set_child(Some(&col));
    } else {
        line.append(&buttons);
        row.set_child(Some(&line));
    }
    row
}

fn rows_list(
    lib: &Library,
    card: &Card,
    acts: &EditionActions,
    compact: bool,
    done: Rc<dyn Fn()>,
) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    super::audit::label(&list, &tr("Editions"));
    for id in &card.members {
        let Some(b) = lib.get(*id) else { continue };
        let info = EditionInfo::of(b, card);
        list.append(&edition_row(
            lib,
            b,
            &info,
            acts,
            true,
            compact,
            done.clone(),
        ));
    }
    list
}

/// The edition list popover of a card. It hangs on `parent` (a stable widget such as the grid,
/// not a recycled card) and points at `anchor`, the badge or the card.
pub fn show_popover(
    lib: &Library,
    card: &Card,
    acts: &EditionActions,
    parent: &gtk::Widget,
    anchor: &gtk::Widget,
) {
    let pop = gtk::Popover::new();
    pop.set_parent(parent);
    if let Some(b) = anchor.compute_bounds(parent) {
        pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            b.x() as i32,
            b.y() as i32,
            b.width() as i32,
            b.height() as i32,
        )));
    }
    let title = lib.get(card.rep).map(|b| b.meta.title.clone());
    let head = gtk::Label::new(Some(&trf(
        "{}: {}",
        &[
            &ntr("{} edition", "{} editions", card.members.len() as u32),
            &title.unwrap_or_default(),
        ],
    )));
    head.add_css_class("heading");
    head.set_xalign(0.0);
    head.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.set_max_width_chars(44);
    let p = pop.clone();
    let list = rows_list(lib, card, acts, false, Rc::new(move || p.popdown()));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(360);
    scroll.set_min_content_width(440);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.set_width_request(460);
    col.set_margin_top(10);
    col.set_margin_bottom(10);
    col.set_margin_start(10);
    col.set_margin_end(10);
    col.append(&head);
    col.append(&scroll);
    pop.set_child(Some(&col));
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
    if let Some(first) = list.first_child() {
        first.grab_focus();
    }
}

/// The *Editions* part of the details panel (no popover: the rows stay in the panel).
pub fn details_group(lib: &Library, card: &Card, acts: &EditionActions) -> Option<gtk::Box> {
    if !card.is_group() {
        return None;
    }
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_margin_top(10);
    let head = gtk::Label::new(Some(&ntr(
        "{} edition",
        "{} editions",
        card.members.len() as u32,
    )));
    head.add_css_class("heading");
    head.set_xalign(0.0);
    col.append(&head);
    col.append(&rows_list(lib, card, acts, true, Rc::new(|| {})));
    Some(col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: u64, file: &str, lang: &str, progress: f64, size: u64) -> Book {
        let mut b: Book = serde_json::from_str(
            r#"{"id":1,"file":"a.epub","meta":{"title":"Dune","title_sort":"T","authors":["A"],"author_sort":"A","publisher":"","language":"","description":"","tags":[],"series":"","series_index":0.0,"identifiers":[],"published":"","rights":"","rating":0},"added":"2026-01-01T00:00:00Z","modified":"2026-01-01T00:00:00Z","size":1}"#,
        )
        .unwrap();
        b.id = id;
        b.file = file.into();
        b.meta.language = lang.into();
        b.meta.publisher = String::new();
        b.progress = progress;
        b.size = size;
        b
    }

    #[test]
    fn the_row_text_names_format_language_size_and_state() {
        let card = Card {
            rep: 1,
            members: vec![1, 2],
            linked: false,
            progress_from: 2,
        };
        let a = EditionInfo::of(&book(1, "a/Dune.epub", "en", 0.0, 2 * 1024 * 1024), &card);
        assert_eq!(a.line(), "EPUB · en · 2.0 MB");
        assert!(a.opens_first && !a.read_last);
        assert_eq!(a.status(), "Opens first");
        let b = EditionInfo::of(&book(2, "b/Dune.azw3", "", 0.4, 100 * 1024), &card);
        assert_eq!(b.line(), "AZW3 · 100 KB");
        assert!(b.read_last && !b.opens_first);
        assert_eq!(b.status(), "40% read · read last");
        assert_eq!(b.name(), "Dune (AZW3)");
    }

    #[test]
    fn the_publisher_tells_editions_of_one_format_apart() {
        let card = Card {
            rep: 1,
            members: vec![1],
            linked: false,
            progress_from: 1,
        };
        let mut b = book(1, "a.epub", "en", 0.0, 1024);
        b.meta.publisher = " Pub B ".into();
        let i = EditionInfo::of(&b, &card);
        assert_eq!(i.line(), "EPUB · en · 1 KB");
        assert_eq!(i.status(), "Pub B · Opens first");
    }

    #[test]
    fn the_badge_names_the_count() {
        let (t, name) = badge_text(3);
        assert_eq!(t, "3");
        assert!(name.contains("3 editions"));
    }
}
