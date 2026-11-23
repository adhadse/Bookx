//! *Notes & Highlights*: every highlight and note of the whole library in one window, grouped by
//! book, month, colour or tag, searchable, filterable, editable and exportable. A click opens the
//! book at the passage. The query is `bookx_lookup::notes_query`, the glue `crate::notes_view`.

use crate::i18n::{N_, tr};
use crate::i18n::{ntr, trf};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use bookx_lookup::notes_query::{self as nq, Filter, Group, KindFilter, NoteRow, RowKind, Sort};
use relm4::RelmWidgetExt;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib, pango};

use super::SharedRef;
use super::note_row::{NoteActions, NoteCard, NoteData};
use crate::annotation_export as export;
use crate::notes_view as nv;
use crate::readwise_sync as rw;

/// How many highlights are listed before "Show more".
const PAGE: usize = 200;

thread_local! {
    /// The highlight to scroll to once the book chosen in the window is open.
    static JUMP: RefCell<Option<(u64, String)>> = const { RefCell::new(None) };
    /// The open window's test commands.
    static DEV: RefCell<Option<Rc<dyn Fn(&str)>>> = const { RefCell::new(None) };
    /// The Edit Note dialog that is open: its text, what saving does and the dialog (test commands).
    static EDIT: RefCell<Option<(gtk::TextView, Rc<dyn Fn()>, adw::Dialog)>> = const { RefCell::new(None) };
}

/// Remembers that the reader of `book` should go to `highlight` when it opens.
pub fn set_jump(book: u64, highlight: &str) {
    JUMP.with(|j| *j.borrow_mut() = Some((book, highlight.to_string())));
}

/// The check box of a card: the first child of the row's line.
fn check_of(row: &gtk::Widget) -> Option<gtk::CheckButton> {
    row.downcast_ref::<gtk::FlowBoxChild>()?
        .child()?
        .first_child()?
        .downcast::<gtk::CheckButton>()
        .ok()
}

/// Shows the check box of a card while the pointer is over it, while it has the keyboard focus, while
/// it is checked and while anything is selected; otherwise it is not drawn (it still takes the click).
fn sync_check(row: &impl IsA<gtk::Widget>) {
    let row = row.as_ref();
    let Some(check) = check_of(row) else { return };
    let focus = gtk::StateFlags::FOCUS_VISIBLE;
    let show = row.has_css_class("hovered")
        || row.has_css_class("selecting")
        || check.is_active()
        || row.state_flags().intersects(focus)
        || check.state_flags().intersects(focus);
    let want = if show { 1.0 } else { 0.0 };
    if check.opacity() != want {
        check.set_opacity(want);
    }
}

/// `row=N x= y= w= h= rowh= titlex= cy= rowcy= opacity= visible= hovered= selecting=` of every card:
/// where the check box is, in the coordinates of its row (test hook).
fn check_geometry(list: &gtk::Widget) -> Vec<String> {
    let mut out = Vec::new();
    let mut n = 0;
    let mut c = list.first_child();
    while let Some(w) = c {
        if let Some(flow) = w.downcast_ref::<gtk::FlowBox>() {
            let mut ch = flow.first_child();
            while let Some(row) = ch {
                if let (Some(check), Some(line)) = (
                    check_of(&row),
                    row.downcast_ref::<gtk::FlowBoxChild>()
                        .and_then(|r| r.child()),
                ) {
                    // the visual row is the line inside the cell (the cell may be taller: all cells of a
                    // line of the grid are as tall as the tallest)
                    let b = check.compute_bounds(&line);
                    let quote =
                        find_class(&line, "note-quote").and_then(|q| q.compute_bounds(&line));
                    let (x, y, w, h) = b.map_or((0.0, 0.0, 0.0, 0.0), |b| {
                        (b.x(), b.y(), b.width(), b.height())
                    });
                    out.push(format!(
                        "notes_checkgeom: row={n} x={x:.0} y={y:.0} w={w:.0} h={h:.0} rowh={:.0} roww={:.0} titlex={:.0} cy={:.0} rowcy={:.0} opacity={:.1} visible={} hovered={} selecting={}",
                        line.height(),
                        line.width(),
                        quote.map_or(-1.0, |q| q.x()),
                        y + h / 2.0,
                        line.height() as f32 / 2.0,
                        check.opacity(),
                        check.is_visible(),
                        row.has_css_class("hovered"),
                        row.has_css_class("selecting"),
                    ));
                    n += 1;
                }
                ch = row.next_sibling();
            }
        }
        c = w.next_sibling();
    }
    out
}

fn find_class(w: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if w.has_css_class(class) {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        if let Some(f) = find_class(&x, class) {
            return Some(f);
        }
        c = x.next_sibling();
    }
    None
}

/// The highlight to go to in `book`, once.
pub fn take_jump(book: u64) -> Option<String> {
    JUMP.with(|j| {
        let mut j = j.borrow_mut();
        match j.as_ref() {
            Some((b, _)) if *b == book => j.take().map(|(_, h)| h),
            _ => None,
        }
    })
}

type Key = (u64, String);

struct State {
    shared: SharedRef,
    /// Any widget of the view: the parent of the dialogs it opens.
    host: gtk::Widget,
    /// Called when a passage is opened in the reader.
    on_close: Rc<dyn Fn()>,
    open: Rc<dyn Fn(u64)>,
    toast: Rc<dyn Fn(String)>,
    rows: RefCell<Vec<NoteRow>>,
    filter: RefCell<Filter>,
    group: Cell<Group>,
    sort: Cell<Sort>,
    selected: RefCell<HashSet<Key>>,
    limit: Cell<usize>,
    /// The sections: headings and flow boxes of cards.
    list: gtk::Box,
    chips: adw::WrapBox,
    badge: gtk::Label,
    has_note: gtk::DropDown,
    from: gtk::Entry,
    to: gtk::Entry,
    stack: gtk::Stack,
    summary: gtk::Label,
    /// The colours of the shown highlights, each with its swatch and count (click one in the
    /// Colour filter to narrow the list).
    legend: gtk::Label,
    export_btn: gtk::MenuButton,
    copy_btn: gtk::Button,
    del_btn: gtk::Button,
    /// Told how many highlights are selected (the library shows it in its header).
    on_selection: RefCell<Option<Rc<dyn Fn(usize)>>>,
    /// The visible matches, in display order.
    shown: RefCell<Vec<usize>>,
    colors: gtk::DropDown,
    books: gtk::DropDown,
    book_ids: RefCell<Vec<u64>>,
    color_names: RefCell<Vec<String>>,
    /// Search, Filter and View: one row.
    toolbar: gtk::Box,
    /// All, Highlights, Notes, Bookmarks: the type of entry that is listed.
    kinds: Vec<(KindFilter, gtk::ToggleButton)>,
    /// The empty page: it says what is missing for the kind that is listed.
    empty: adw::StatusPage,
    /// Where the bookmarks that devices brought came from, and whether their place is a rough one.
    marks: RefCell<HashMap<Key, (String, bool)>>,
    /// Shows a message with an Undo button.
    undo: Rc<dyn Fn(String, crate::app::UndoFn)>,
}

/// How many entries a menu shows at its first level (sections are flattened).
fn menu_top_count(m: &gio::MenuModel) -> usize {
    (0..m.n_items())
        .map(|i| m.item_link(i, "section").map_or(1, |s| menu_top_count(&s)))
        .sum()
}

/// Every label of a menu, sections and submenus included.
fn menu_labels(m: &gio::MenuModel, out: &mut Vec<String>) {
    for i in 0..m.n_items() {
        if let Some(l) = m
            .item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(String::from))
        {
            out.push(l);
        }
        let links = m.iterate_item_links(i);
        while let Some((_, sub)) = links.next() {
            menu_labels(&sub, out);
        }
    }
}

fn key_of(r: &NoteRow) -> Key {
    (r.book_id, r.highlight_id.clone())
}

fn date_text(r: &NoteRow) -> String {
    r.created.format("%Y-%m-%d").to_string()
}

impl State {
    fn reload(self: &Rc<Self>) {
        {
            let lib = self.shared.lib.borrow();
            *self.rows.borrow_mut() = nv::rows(&lib);
            *self.marks.borrow_mut() = if lib.books.iter().any(|b| !b.bookmarks.is_empty()) {
                crate::annotation_sync_ui::bookmark_infos(&lib.root)
            } else {
                HashMap::new()
            };
        }
        // forget selections of highlights that are gone
        let have: HashSet<Key> = self.rows.borrow().iter().map(key_of).collect();
        self.selected.borrow_mut().retain(|k| have.contains(k));
        self.fill_pickers();
        self.refresh();
    }

    /// The colour and book pickers follow what the library has.
    fn fill_pickers(self: &Rc<Self>) {
        let rows = self.rows.borrow();
        let colors = nq::colors_of(&rows);
        let mut names = vec![tr("All colours")];
        names.extend(colors.iter().map(|c| capitalise(c)));
        let sel = self.colors.selected();
        self.colors.set_model(Some(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        self.colors
            .set_selected(if (sel as usize) < names.len() { sel } else { 0 });
        *self.color_names.borrow_mut() = colors;

        let mut ids: Vec<(u64, String)> = Vec::new();
        for r in rows.iter() {
            if !ids.iter().any(|(i, _)| *i == r.book_id) {
                ids.push((r.book_id, r.book_title.clone()));
            }
        }
        ids.sort_by_key(|a| a.1.to_lowercase());
        let mut names = vec![tr("All books")];
        names.extend(ids.iter().map(|(_, t)| t.clone()));
        let current = self.filter.borrow().book;
        self.books.set_model(Some(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        let pos = current
            .and_then(|b| ids.iter().position(|(i, _)| *i == b))
            .map_or(0, |p| p as u32 + 1);
        self.books.set_selected(pos);
        *self.book_ids.borrow_mut() = ids.into_iter().map(|(i, _)| i).collect();
    }

    /// What an export covers: the selected highlights, else everything shown.
    fn targets(&self) -> Vec<NoteRow> {
        let rows = self.rows.borrow();
        let sel = self.selected.borrow();
        if sel.is_empty() {
            return self
                .shown
                .borrow()
                .iter()
                .map(|&i| rows[i].clone())
                .collect();
        }
        nq::query(&rows, &Filter::default(), self.sort.get())
            .into_iter()
            .filter(|&i| sel.contains(&key_of(&rows[i])))
            .map(|i| rows[i].clone())
            .collect()
    }

    fn refresh(self: &Rc<Self>) {
        let rows = self.rows.borrow();
        let order = nq::query(&rows, &self.filter.borrow(), self.sort.get());
        *self.shown.borrow_mut() = order.clone();
        let sections = nq::group(&rows, &order, self.group.get());

        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut left = self.limit.get();
        let mut hidden = 0usize;
        for s in &sections {
            if left == 0 {
                hidden += s.rows.len();
                continue;
            }
            let head = gtk::Label::new(Some(&format!(
                "{} · {}",
                s.title.to_uppercase(),
                s.rows.len()
            )));
            head.set_xalign(0.0);
            head.add_css_class("caption-heading");
            head.add_css_class("notes-section");
            head.set_margin_top(14);
            head.set_margin_bottom(4);
            head.set_margin_start(2);
            head.set_ellipsize(pango::EllipsizeMode::End);
            self.list.append(&head);
            let mut flow: Option<gtk::FlowBox> = None;
            for &i in &s.rows {
                if left == 0 {
                    hidden += 1;
                    continue;
                }
                left -= 1;
                let f = flow.get_or_insert_with(|| {
                    let f = self.new_flow();
                    self.list.append(&f);
                    f
                });
                f.append(&self.build_row(&rows[i]));
            }
        }
        if hidden > 0 {
            let more = gtk::Button::with_label(&format!("{} ({hidden})", tr("Show more")));
            more.add_css_class("flat");
            more.set_margin_top(8);
            more.set_margin_bottom(8);
            super::audit::label(&more, &tr("Show more highlights"));
            let st = self.clone();
            more.connect_clicked(move |_| {
                st.limit.set(st.limit.get() + PAGE);
                st.refresh();
            });
            self.list.append(&more);
        }

        drop(rows);
        self.update_chips();
        let rows = self.rows.borrow();
        let total = rows.len();
        self.stack
            .set_visible_child_name(if order.is_empty() { "empty" } else { "list" });
        let books: HashSet<u64> = order.iter().map(|&i| rows[i].book_id).collect();
        let with_notes = order
            .iter()
            .filter(|&&i| !rows[i].note.trim().is_empty())
            .count();
        let any_bookmark = rows.iter().any(|r| r.kind == RowKind::Bookmark);
        self.summary.set_label(&match self.filter.borrow().kind {
            KindFilter::Bookmarks => trf(
                "{} of {} bookmarks · {} books",
                &[
                    &order.len(),
                    &rows.iter().filter(|r| r.kind == RowKind::Bookmark).count(),
                    &books.len(),
                ],
            ),
            _ if any_bookmark => trf(
                "{} of {} entries · {} books · {} with notes",
                &[&order.len(), &total, &books.len(), &with_notes],
            ),
            _ => trf(
                "{} of {} highlights · {} books · {} with notes",
                &[&order.len(), &total, &books.len(), &with_notes],
            ),
        });
        let (title, text) = match self.filter.borrow().kind {
            KindFilter::Bookmarks => (
                tr("No bookmarks"),
                tr(
                    "Bookmarks you add while reading, and the ones synced from your e-reader, appear here.",
                ),
            ),
            _ => (
                tr("No highlights"),
                tr(
                    "Highlights and notes you make while reading appear here, with the books they come from.",
                ),
            ),
        };
        self.empty.set_title(&title);
        self.empty.set_description(Some(&text));
        let mut counts: Vec<(String, usize)> = Vec::new();
        for &i in order.iter() {
            let c = rows[i].color.clone();
            match counts.iter_mut().find(|(n, _)| *n == c) {
                Some(e) => e.1 += 1,
                None => counts.push((c, 1)),
            }
        }
        let legend: Vec<String> = counts
            .iter()
            .filter(|(c, _)| !c.is_empty())
            .map(|(c, n)| {
                let (r, g, b) = super::reader::color_of(c);
                format!(
                    "<span foreground=\"#{:02x}{:02x}{:02x}\">●</span> {} {}",
                    (r * 255.0) as u8,
                    (g * 255.0) as u8,
                    (b * 255.0) as u8,
                    glib::markup_escape_text(&capitalise(c)),
                    n
                )
            })
            .collect();
        self.legend.set_markup(&legend.join("   "));
        self.legend.set_visible(!legend.is_empty());
        drop(rows);
        self.selection_changed();
    }

    /// The bar, the Export menu and the owner of the header follow the selection.
    fn selection_changed(self: &Rc<Self>) {
        let n = self.selected.borrow().len();
        self.copy_btn.set_visible(n > 0);
        self.del_btn.set_visible(n > 0);
        self.export_btn.set_menu_model(Some(&export_menu(n)));
        self.export_btn.set_tooltip_text(Some(&export_heading(n)));
        // the check boxes are shown on every card while something is selected
        let mut c = self.list.first_child();
        while let Some(w) = c {
            if let Some(flow) = w.downcast_ref::<gtk::FlowBox>() {
                let mut ch = flow.first_child();
                while let Some(card) = ch {
                    if n > 0 {
                        card.add_css_class("selecting");
                    } else {
                        card.remove_css_class("selecting");
                    }
                    sync_check(&card);
                    ch = card.next_sibling();
                }
            }
            c = w.next_sibling();
        }
        let cb = self.on_selection.borrow().clone();
        if let Some(cb) = cb {
            cb(n);
        }
    }

    /// How many cards are in "selection mode" (their check box is shown).
    /// The least width the toolbar needs.
    fn toolbar_min(&self) -> i32 {
        self.toolbar.measure(gtk::Orientation::Horizontal, -1).0
    }

    fn selecting_cards(&self) -> usize {
        let mut n = 0;
        let mut c = self.list.first_child();
        while let Some(w) = c {
            if let Some(flow) = w.downcast_ref::<gtk::FlowBox>() {
                let mut ch = flow.first_child();
                while let Some(card) = ch {
                    if card.has_css_class("selecting") {
                        n += 1;
                    }
                    ch = card.next_sibling();
                }
            }
            c = w.next_sibling();
        }
        n
    }

    fn select_all(self: &Rc<Self>) {
        let keys: Vec<Key> = {
            let rows = self.rows.borrow();
            self.shown
                .borrow()
                .iter()
                .map(|&i| key_of(&rows[i]))
                .collect()
        };
        self.selected.borrow_mut().extend(keys);
        self.refresh();
    }

    fn clear_selection(self: &Rc<Self>) {
        if !self.selected.borrow().is_empty() {
            self.selected.borrow_mut().clear();
            self.refresh();
        }
    }

    /// The selected highlights as text: quote, then the note, one blank line between.
    fn copy_selection(self: &Rc<Self>) {
        let rows = self.targets();
        let text = copy_text(&rows);
        self.host.clipboard().set_text(&text);
        (self.toast)(if rows.iter().any(|r| r.kind == RowKind::Bookmark) {
            ntr("Copied {} entry", "Copied {} entries", rows.len())
        } else {
            ntr("Copied {} highlight", "Copied {} highlights", rows.len())
        });
    }

    fn confirm_delete_selected(self: &Rc<Self>) {
        let keys: Vec<Key> = self.selected.borrow().iter().cloned().collect();
        if keys.is_empty() {
            return;
        }
        let st = self.clone();
        let marks: HashSet<Key> = self
            .rows
            .borrow()
            .iter()
            .filter(|r| r.kind == RowKind::Bookmark)
            .map(key_of)
            .collect();
        let (title, body) = if marks.is_empty() {
            (
                ntr("Delete {} Highlight?", "Delete {} Highlights?", keys.len()),
                tr(
                    "The highlights and their notes are removed from their books. This cannot be undone.",
                ),
            )
        } else {
            (
                ntr("Delete {} Entry?", "Delete {} Entries?", keys.len()),
                tr(
                    "The highlights, notes and bookmarks are removed from their books. This cannot be undone.",
                ),
            )
        };
        super::alert(&self.host, &title, &body, &tr("Delete"), true, move || {
            {
                let mut lib = st.shared.lib.borrow_mut();
                for k in &keys {
                    if marks.contains(k) {
                        nv::delete_bookmark(&mut lib.books, k.0, &k.1);
                    } else {
                        nv::delete(&mut lib.books, k.0, &k.1);
                    }
                }
                let _ = lib.save();
            }
            st.selected.borrow_mut().clear();
            st.reload();
        });
    }

    /// One highlight: a check box for export, the shared card (quote in its colour and style,
    /// the note in its block, where and when), a click opens the book at the passage.
    fn build_row(self: &Rc<Self>, r: &NoteRow) -> gtk::FlowBoxChild {
        let k = key_of(r);
        let actions = {
            let (s1, s2, s3, s4, k1, k2, k3, k4) = (
                self.clone(),
                self.clone(),
                self.clone(),
                self.clone(),
                k.clone(),
                k.clone(),
                k.clone(),
                k.clone(),
            );
            let can_look = r.page.is_none() && r.kind == RowKind::Highlight;
            let bookmark = r.kind == RowKind::Bookmark;
            NoteActions {
                edit: Rc::new(move || s1.edit_note(&k1)),
                delete: Rc::new(move || {
                    if bookmark {
                        s2.delete_bookmark(&k2)
                    } else {
                        s2.confirm_delete(&k2)
                    }
                }),
                open: bookmark.then(|| {
                    let (s6, k6) = (self.clone(), k.clone());
                    Rc::new(move || s6.open_at(&k6)) as Rc<dyn Fn()>
                }),
                recolor: can_look.then(|| {
                    Rc::new(move |c: &str| s3.apply_look(&k3, Some(c), None)) as Rc<dyn Fn(&str)>
                }),
                restyle: can_look.then(|| {
                    Rc::new(move |st: &str| s4.apply_look(&k4, None, Some(st))) as Rc<dyn Fn(&str)>
                }),
                relocate: (r.unplaced && !bookmark).then(|| {
                    let (s5, k5) = (self.clone(), k.clone());
                    Rc::new(move || s5.relocate(&k5)) as Rc<dyn Fn()>
                }),
            }
        };
        let card = NoteCard::compact(&actions);
        if r.kind == RowKind::Bookmark {
            let from = self.marks.borrow().get(&k).cloned();
            let mut meta = format!("{} · {}", r.place(), date_text(r));
            if let Some((src, approx)) = from {
                if !src.is_empty() {
                    meta.push_str(&format!(" · {src}"));
                }
                if approx {
                    meta.push_str(&format!(" · {}", tr("approximate place")));
                }
            }
            card.set(&NoteData {
                text: r.text.trim().to_string(),
                meta,
                bookmark: true,
                ..Default::default()
            });
        } else {
            card.set(&NoteData {
                color: r.color.clone(),
                style: self
                    .shared
                    .lib
                    .borrow()
                    .get(r.book_id)
                    .and_then(|b| b.highlights.iter().find(|h| h.id == r.highlight_id))
                    .map(|h| h.style.clone())
                    .unwrap_or_default(),
                text: r.text.trim().to_string(),
                note: r.note.clone(),
                meta: {
                    let m = format!(
                        "{} · {}",
                        if r.unplaced {
                            tr("Could not be located in the book")
                        } else {
                            r.place()
                        },
                        date_text(r)
                    );
                    match crate::annotation_sync_ui::source_label(r.book_id, &r.highlight_id) {
                        Some(src) => format!("{m} · {src}"),
                        None => m,
                    }
                },
                bookmark: false,
            });
        }
        let row = gtk::FlowBoxChild::new();
        row.set_widget_name(&format!("{}\t{}", k.0, k.1));
        row.set_size_request(360, -1);
        row.set_hexpand(true);
        // selection mode as GNOME does it: the check box has its own column at the start of the row,
        // centred on it, and never covers the text; it is only drawn on hover, on keyboard focus and
        // while anything is selected
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let check = gtk::CheckButton::new();
        check.add_css_class("selection-mode");
        check.add_css_class("note-check");
        check.set_halign(gtk::Align::Center);
        check.set_valign(gtk::Align::Center);
        check.set_margin_start(2);
        check.set_active(self.selected.borrow().contains(&k));
        super::audit::label(&check, &tr("Select this highlight"));
        {
            let (st, k) = (self.clone(), k.clone());
            check.connect_toggled(move |c| {
                let was = st.selected.borrow().len();
                if c.is_active() {
                    st.selected.borrow_mut().insert(k.clone());
                } else {
                    st.selected.borrow_mut().remove(&k);
                }
                let now = st.selected.borrow().len();
                if (was == 0) != (now == 0) {
                    st.selection_changed();
                } else if let Some(cb) = st.on_selection.borrow().clone() {
                    cb(now);
                }
            });
        }
        card.root.set_hexpand(true);
        line.append(&check);
        line.append(&card.root);
        // no check boxes at rest: they appear on hover, on focus and while something is selected
        line.add_css_class("note-row");
        line.set_valign(gtk::Align::Start);
        {
            let motion = gtk::EventControllerMotion::new();
            let c = row.clone();
            motion.connect_enter(move |_, _, _| {
                c.add_css_class("hovered");
                sync_check(&c);
            });
            let c = row.clone();
            motion.connect_leave(move |_| {
                c.remove_css_class("hovered");
                sync_check(&c);
            });
            row.add_controller(motion);
        }
        if !self.selected.borrow().is_empty() {
            row.add_css_class("selecting");
        }
        row.set_child(Some(&line));
        {
            let c = row.clone();
            row.connect_state_flags_changed(move |_, _| sync_check(&c));
            let c = row.clone();
            check.connect_state_flags_changed(move |_, _| sync_check(&c));
            let c = row.clone();
            check.connect_toggled(move |_| sync_check(&c));
        }
        sync_check(&row);
        row
    }

    /// Changes colour or style of a highlight.
    /// Looks for the text of a note without a place in its book again.
    fn relocate(self: &Rc<Self>, k: &Key) {
        let ok = {
            let mut lib = self.shared.lib.borrow_mut();
            crate::highlight_repair::relocate_one(&mut lib, k.0, &k.1)
        };
        if ok {
            (self.toast)(tr("Found it: the highlight is back on the page"));
            self.reload();
        } else {
            (self.toast)(tr(
                "The text is not in this book. Select the passage in the book and highlight it again, then delete this note",
            ));
        }
    }

    fn apply_look(self: &Rc<Self>, k: &Key, color: Option<&str>, style: Option<&str>) {
        let ok = {
            let mut lib = self.shared.lib.borrow_mut();
            let ok = nv::set_look(&mut lib.books, k.0, &k.1, color, style);
            if ok {
                let _ = lib.save();
            }
            ok
        };
        if ok {
            self.reload();
        }
    }

    /// A row of cards: as many columns as fit, a click or Enter opens the book at the passage.
    fn new_flow(self: &Rc<Self>) -> gtk::FlowBox {
        let f = gtk::FlowBox::new();
        f.set_selection_mode(gtk::SelectionMode::None);
        f.set_homogeneous(true);
        f.set_hexpand(true);
        f.set_min_children_per_line(1);
        f.set_max_children_per_line(4);
        f.set_column_spacing(8);
        f.set_row_spacing(6);
        f.set_valign(gtk::Align::Start);
        f.set_activate_on_single_click(true);
        let st = self.clone();
        f.connect_child_activated(move |_, child| {
            if let Some((b, h)) = child.widget_name().split_once('\t') {
                if let Ok(b) = b.parse::<u64>() {
                    st.open_at(&(b, h.to_string()));
                }
            }
        });
        f
    }

    /// The chips of the active filters under the toolbar, and the count on the Filter button.
    fn update_chips(self: &Rc<Self>) {
        while let Some(c) = self.chips.first_child() {
            self.chips.remove(&c);
        }
        let mut n = 0;
        let mut chip = |label: String, clear: Box<dyn Fn()>| {
            n += 1;
            let b = gtk::Button::new();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            row.append(&gtk::Label::new(Some(&label)));
            row.append(&gtk::Image::from_icon_name("window-close-symbolic"));
            b.set_child(Some(&row));
            b.add_css_class("pill");
            b.add_css_class("notes-chip");
            let tip = trf("Remove the filter {}", &[&label]);
            b.set_tooltip_text(Some(&tip));
            super::audit::label(&b, &tip);
            b.connect_clicked(move |_| clear());
            self.chips.append(&b);
        };
        if self.colors.selected() > 0 {
            let name = self
                .colors
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default();
            let d = self.colors.clone();
            chip(name, Box::new(move || d.set_selected(0)));
        }
        if self.books.selected() > 0 {
            let name = self
                .books
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default();
            let d = self.books.clone();
            chip(name, Box::new(move || d.set_selected(0)));
        }
        if self.has_note.selected() > 0 {
            let name = self
                .has_note
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_default();
            let d = self.has_note.clone();
            chip(name, Box::new(move || d.set_selected(0)));
        }
        for (label, e) in [(tr("From"), &self.from), (tr("To"), &self.to)] {
            let t = e.text().to_string();
            if nq::parse_day(&t).is_some() {
                let e = e.clone();
                chip(format!("{label} {t}"), Box::new(move || e.set_text("")));
            }
        }
        self.chips.set_visible(n > 0);
        self.badge.set_label(&n.to_string());
        self.badge.set_visible(n > 0);
    }

    fn open_at(&self, k: &Key) {
        set_jump(k.0, &k.1);
        (self.on_close)();
        (self.open)(k.0);
    }

    fn current_note(&self, k: &Key) -> Option<String> {
        self.rows
            .borrow()
            .iter()
            .find(|r| key_of(r) == *k)
            .map(|r| r.note.clone())
    }

    fn apply_note(self: &Rc<Self>, k: &Key, note: &str) {
        let ok = {
            let mut lib = self.shared.lib.borrow_mut();
            let ok = nv::set_note(&mut lib.books, k.0, &k.1, note);
            if ok {
                let _ = lib.save();
            }
            ok
        };
        eprintln!("notes_edit: ok={ok} note={:?}", note.trim());
        self.reload();
    }

    /// Takes a bookmark away with an Undo that puts it back. A bookmark that came from a device is
    /// not brought back by the next sync: the sync's ledger remembers it was handled.
    fn delete_bookmark(self: &Rc<Self>, k: &Key) {
        let removed = {
            let mut lib = self.shared.lib.borrow_mut();
            let r = nv::delete_bookmark(&mut lib.books, k.0, &k.1);
            if r.is_some() {
                let _ = lib.save();
            }
            r
        };
        self.selected.borrow_mut().remove(k);
        self.reload();
        let Some(r) = removed else { return };
        eprintln!("notes_delete: ok=true bookmark=true");
        let stash = Rc::new(RefCell::new(Some(r)));
        let (st, book) = (self.clone(), k.0);
        let undo = crate::app::UndoFn::new(move || {
            let Some(r) = stash.borrow_mut().take() else {
                return;
            };
            {
                let mut lib = st.shared.lib.borrow_mut();
                if let Some(b) = lib.get_mut(book) {
                    crate::remove_undo::restore(&mut b.highlights, &mut b.bookmarks, r);
                }
                let _ = lib.save();
            }
            st.reload();
        });
        (self.undo)(tr("Bookmark removed"), undo);
    }

    fn apply_delete(self: &Rc<Self>, k: &Key) {
        if self
            .rows
            .borrow()
            .iter()
            .any(|r| key_of(r) == *k && r.kind == RowKind::Bookmark)
        {
            self.delete_bookmark(k);
            return;
        }
        let ok = {
            let mut lib = self.shared.lib.borrow_mut();
            let ok = nv::delete(&mut lib.books, k.0, &k.1);
            if ok {
                let _ = lib.save();
            }
            ok
        };
        eprintln!("notes_delete: ok={ok}");
        self.selected.borrow_mut().remove(k);
        self.reload();
    }

    fn edit_note(self: &Rc<Self>, k: &Key) {
        let Some(old) = self.current_note(k) else {
            return;
        };
        let tv = gtk::TextView::new();
        tv.set_wrap_mode(gtk::WrapMode::WordChar);
        tv.buffer().set_text(&old);
        tv.set_top_margin(8);
        tv.set_bottom_margin(8);
        tv.set_left_margin(8);
        tv.set_right_margin(8);
        super::audit::label(&tv, &tr("Note"));
        let sw = gtk::ScrolledWindow::new();
        sw.set_child(Some(&tv));
        sw.set_min_content_height(160);
        sw.set_vexpand(true);
        sw.set_margin_all(12);
        sw.add_css_class("card");

        let d = super::dialog_toast::new_dialog();
        d.set_title(&tr("Edit Note"));
        d.set_content_width(480);
        d.set_follows_content_size(false);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let save = gtk::Button::with_label(&tr("Save"));
        save.add_css_class("suggested-action");
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        header.pack_start(&cancel);
        header.pack_end(&save);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&sw));
        d.set_child(Some(&view));
        d.set_default_widget(Some(&save));
        {
            let d = d.clone();
            cancel.connect_clicked(move |_| {
                d.close();
            });
        }
        let do_save = {
            let (st, k, tv, d) = (self.clone(), k.clone(), tv.clone(), d.clone());
            Rc::new(move || {
                let b = tv.buffer();
                let text = b.text(&b.start_iter(), &b.end_iter(), false);
                d.close();
                st.apply_note(&k, text.as_str());
            })
        };
        {
            let f = do_save.clone();
            save.connect_clicked(move |_| f());
        }
        // Ctrl+Enter saves from inside the text
        let sc = gtk::ShortcutController::new();
        sc.set_scope(gtk::ShortcutScope::Local);
        {
            let f = do_save.clone();
            sc.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string("<Control>Return|<Control>KP_Enter"),
                Some(gtk::CallbackAction::new(move |_, _| {
                    f();
                    glib::Propagation::Stop
                })),
            ));
        }
        tv.add_controller(sc);
        EDIT.with(|e| *e.borrow_mut() = Some((tv.clone(), do_save, d.clone())));
        d.present(Some(&self.host));
        tv.grab_focus();
    }

    fn confirm_delete(self: &Rc<Self>, k: &Key) {
        let (st, k) = (self.clone(), k.clone());
        super::alert(
            &self.host,
            &tr("Delete this highlight?"),
            &tr("The highlight and its note are removed from the book. This cannot be undone."),
            &tr("Delete"),
            true,
            move || st.apply_delete(&k),
        );
    }

    // ---- exports ----------------------------------------------------------------------------

    fn build_export(&self, id: &str) -> anyhow::Result<(Vec<u8>, String)> {
        let rows = self.targets();
        if rows.is_empty() && !id.starts_with("vocab-") {
            anyhow::bail!("There are no highlights to export");
        }
        let refs: Vec<&NoteRow> = rows.iter().collect();
        let items = nv::items_for(&self.shared.lib.borrow(), &refs);
        let now = chrono::Utc::now();
        if let Some(spec) = id
            .strip_prefix("vocab-")
            .or_else(|| id.strip_prefix("anki-"))
        {
            let (file, kind) = nv::AnkiFile::parse(spec)
                .ok_or_else(|| anyhow::anyhow!("unknown Anki format {id}"))?;
            let vocab = id.starts_with("vocab-");
            let hub = self.shared.vocab.borrow();
            let lib = self.shared.lib.borrow();
            let title = |bid: &str| {
                bid.parse::<u64>()
                    .ok()
                    .and_then(|i| lib.get(i))
                    .map(|b| b.meta.title.clone())
            };
            let (bytes, n) = nv::anki_bytes(
                file,
                kind,
                if vocab {
                    "Bookx::Vocabulary"
                } else {
                    "Bookx::Highlights"
                },
                &items,
                vocab.then_some((&hub.store, &title as &dyn Fn(&str) -> Option<String>)),
                now,
            )?;
            if n == 0 {
                anyhow::bail!("There is nothing to turn into cards");
            }
            let name = format!(
                "{}.{}",
                if vocab {
                    "Bookx vocabulary"
                } else {
                    "Bookx highlights"
                },
                file.extension()
            );
            return Ok((bytes, name));
        }
        let text = export::render(&items, id, now)
            .ok_or_else(|| anyhow::anyhow!("unknown export format {id}"))?;
        let books: HashSet<u64> = rows.iter().map(|r| r.book_id).collect();
        let only = (books.len() == 1).then(|| rows[0].book_title.clone());
        Ok((
            text.into_bytes(),
            nv::file_name(id, books.len(), only.as_deref()),
        ))
    }

    /// Writes an export of the selection (or everything shown) to `path`.
    fn export_to(&self, id: &str, path: &Path) -> String {
        match self.build_export(id).and_then(|(bytes, _)| {
            std::fs::write(path, &bytes)?;
            Ok(bytes.len())
        }) {
            Ok(n) => {
                eprintln!("notes_export: id={id} bytes={n}");
                trf("Exported to {}", &[&path.display()])
            }
            Err(e) => {
                eprintln!("notes_export: error: {e:#}");
                format!("{e:#}")
            }
        }
    }

    fn export_dialog(self: &Rc<Self>, id: &str) {
        let (_, name) = match self.build_export(id) {
            Ok(v) => v,
            Err(e) => {
                (self.toast)(format!("{e:#}"));
                return;
            }
        };
        let st = self.clone();
        let id = id.to_string();
        let fd = gtk::FileDialog::builder()
            .title(tr("Export"))
            .initial_name(name)
            .build();
        let parent = self.host.root().and_downcast::<gtk::Window>();
        fd.save(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                (st.toast)(st.export_to(&id, &p));
            }
        });
    }

    // ---- Anki -------------------------------------------------------------------------------

    /// Asks what the cards are made of, then exports.
    fn anki_dialog(self: &Rc<Self>, file: &str) {
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let kind = adw::ComboRow::new();
        kind.set_title(&tr("Card type"));
        kind.set_model(Some(&gtk::StringList::new(&[
            &tr("Front and back"),
            &tr("Cloze deletion"),
        ])));
        let from = adw::ComboRow::new();
        from.set_title(&tr("Cards from"));
        from.set_model(Some(&gtk::StringList::new(&[
            &tr("Highlights"),
            &tr("Vocabulary"),
        ])));
        list.append(&kind);
        list.append(&from);
        let d = adw::AlertDialog::new(
            Some(&tr("Export Anki Flashcards")),
            Some(&match file {
                "apkg" => tr("An Anki package with the cards and their deck."),
                "tsv" => tr("A text file Anki can import, tab separated."),
                _ => tr("A CSV file Anki can import."),
            }),
        );
        d.set_widget_name("anki-dialog");
        d.set_extra_child(Some(&list));
        d.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Export"))]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        d.set_close_response("cancel");
        let (st, file) = (self.clone(), file.to_string());
        d.connect_response(None, move |_, resp| {
            if resp == "ok" {
                if let Some(id) = anki_id(&file, kind.selected() == 1, from.selected() == 1) {
                    st.export_dialog(&id);
                }
            }
        });
        d.present(Some(&self.host));
    }

    // ---- Readwise ---------------------------------------------------------------------------

    fn ask_token(self: &Rc<Self>, then: impl Fn(&Rc<State>) + 'static) {
        let entry = adw::PasswordEntryRow::builder()
            .title(tr("Access token"))
            .build();
        let lb = gtk::ListBox::new();
        lb.add_css_class("boxed-list");
        lb.set_selection_mode(gtk::SelectionMode::None);
        lb.append(&entry);
        let d = adw::AlertDialog::new(
            Some(&tr("Readwise Access Token")),
            Some(&tr(
                "Paste the token from readwise.io/access_token. It is kept in the system keyring.",
            )),
        );
        d.set_extra_child(Some(&lb));
        d.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Save"))]);
        d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        d.set_close_response("cancel");
        let st = self.clone();
        d.connect_response(None, move |_, resp| {
            if resp == "ok" {
                {
                    let mut c = st.shared.cfg.borrow_mut();
                    c.readwise_token = entry.text().trim().to_string();
                    c.save();
                }
                if !st.shared.cfg.borrow().readwise_token.is_empty() {
                    then(&st);
                }
            }
        });
        d.present(Some(&self.host));
    }

    fn readwise(self: &Rc<Self>) {
        if self.shared.cfg.borrow().readwise_token.trim().is_empty() {
            self.ask_token(|st| st.readwise());
            return;
        }
        let rows = self.targets();
        if rows.is_empty() {
            (self.toast)(tr("There are no highlights to send"));
            return;
        }
        let refs: Vec<&NoteRow> = rows.iter().collect();
        let items = nv::items_for(&self.shared.lib.borrow(), &refs);
        let token = self.shared.cfg.borrow().readwise_token.clone();
        let url = rw::endpoint();
        let ledger_file = rw::ledger_path();

        // the Jobs panel is the one place for background work: progress, Cancel and the log
        let (job, cancel) = crate::jobs::start(
            crate::jobs::Kind::Send,
            &tr("Send Highlights to Readwise"),
            true,
        );
        crate::jobs::log(job, &trf("{} highlights to send", &[&items.len()]));

        enum Msg {
            Progress(usize, usize),
            Done(Result<rw::Outcome, String>),
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let flag = cancel.clone();
        std::thread::spawn(move || {
            let mut ledger = ledger_file
                .as_deref()
                .map(rw::Ledger::load)
                .unwrap_or_default();
            let tx2 = tx.clone();
            let r = rw::send(
                &rw::Http,
                &token,
                &url,
                &items,
                &mut ledger,
                ledger_file.as_deref(),
                &mut |d, t| {
                    let _ = tx2.send(Msg::Progress(d, t));
                },
                &|| flag.is_cancelled(),
            )
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Done(r));
        });
        let toast = self.toast.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
            loop {
                match rx.try_recv() {
                    Ok(Msg::Progress(d, t)) => {
                        crate::jobs::progress(
                            job,
                            Some(d as f64 / t.max(1) as f64),
                            &format!("{d} of {t}"),
                        );
                    }
                    Ok(Msg::Done(r)) => {
                        let msg = match &r {
                            Ok(o) => rw::summary(o),
                            Err(e) => e.clone(),
                        };
                        crate::jobs::log(job, &msg);
                        crate::jobs::finish(job, r.map(|_| ()));
                        eprintln!("readwise: {msg}");
                        toast(msg);
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(_) => return glib::ControlFlow::Break,
                }
            }
        });
    }
}

/// The export id of an Anki file: `file` is `apkg`, `tsv` or `csv`; the cards are front/back or
/// cloze, made from the highlights or from the vocabulary.
pub fn anki_id(file: &str, cloze: bool, vocab: bool) -> Option<String> {
    if !matches!(file, "apkg" | "tsv" | "csv") {
        return None;
    }
    Some(format!(
        "{}-{file}-{}",
        if vocab { "vocab" } else { "anki" },
        if cloze { "cloze" } else { "basic" }
    ))
}

/// What Export covers: the selection, or everything shown while nothing is selected.
pub fn export_heading(selected: usize) -> String {
    if selected == 0 {
        tr("Export Everything Shown")
    } else {
        ntr("Export {} Selected", "Export {} Selected", selected)
    }
}

/// The Export menu: the first section names what is exported.
fn export_menu(selected: usize) -> gio::Menu {
    let menu = gio::Menu::new();
    let fm = gio::Menu::new();
    // the Readwise body is what *Send to Readwise* sends: not a file to keep
    for f in export::FORMATS.iter().filter(|f| f.id != "readwise") {
        let it = gio::MenuItem::new(Some(f.label), None);
        it.set_action_and_target_value(Some("notes.export"), Some(&f.id.to_variant()));
        fm.append_item(&it);
    }
    menu.append_section(Some(&export_heading(selected)), &fm);
    // three entries; what the cards are made of is asked in a small dialog
    let am = gio::Menu::new();
    for (id, label) in [
        ("apkg", N_("Anki Package")),
        ("tsv", N_("Anki Text File")),
        ("csv", N_("Anki CSV")),
    ] {
        let it = gio::MenuItem::new(Some(&tr(label)), None);
        it.set_action_and_target_value(Some("notes.anki"), Some(&id.to_variant()));
        am.append_item(&it);
    }
    menu.append_section(Some(&tr("Anki Flashcards")), &am);
    let rm = gio::Menu::new();
    rm.append(Some(&tr("Send to Readwise")), Some("notes.readwise"));
    rm.append(Some(&tr("Readwise Access Token")), Some("notes.token"));
    menu.append_section(Some(&tr("Readwise")), &rm);
    menu
}

/// The text copied from the selection: each quote, then its note, a blank line between highlights.
fn copy_text(rows: &[NoteRow]) -> String {
    rows.iter()
        .map(|r| {
            let note = r.note.trim();
            if note.is_empty() {
                r.text.trim().to_string()
            } else {
                format!("{}\n{}", r.text.trim(), note)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

const GROUPS: [(&str, Group); 5] = [
    (N_("Group by book"), Group::Book),
    (N_("Group by month"), Group::Month),
    (N_("Group by colour"), Group::Color),
    (N_("Group by book tag"), Group::Tag),
    (N_("No grouping"), Group::None),
];

const SORTS: [(&str, Sort); 3] = [
    (N_("Newest first"), Sort::Newest),
    (N_("Oldest first"), Sort::Oldest),
    (N_("In book order"), Sort::InBook),
];

/// The notes view, to put in the content area of the library window.
pub struct View {
    pub widget: gtk::Widget,
    pub search: gtk::SearchEntry,
    /// Reads the highlights again (call when the view is shown).
    pub reload: Rc<dyn Fn()>,
    /// Called with the number of selected highlights whenever it changes.
    pub on_selection: Rc<dyn Fn(Rc<dyn Fn(usize)>)>,
    pub select_all: Rc<dyn Fn()>,
    pub clear_selection: Rc<dyn Fn()>,
}

/// Builds the view of every highlight of the library, with search, filters and export. `open`
/// reads a book (the library's Read), `toast` shows a message.
pub fn build(
    shared: &SharedRef,
    open: Rc<dyn Fn(u64)>,
    toast: Rc<dyn Fn(String)>,
    undo: Rc<dyn Fn(String, crate::app::UndoFn)>,
) -> View {
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Search quotes, notes, books")));
    search.set_hexpand(true);
    super::audit::label(&search, &tr("Search highlights and notes"));

    let dd = |labels: &[&str], name: &str| {
        let d = gtk::DropDown::from_strings(labels);
        super::audit::label(&d, name);
        d
    };
    let group_labels: Vec<String> = GROUPS.iter().map(|g| tr(g.0)).collect();
    let group = dd(
        &group_labels.iter().map(String::as_str).collect::<Vec<_>>(),
        &tr("Grouping"),
    );
    let sort_labels: Vec<String> = SORTS.iter().map(|g| tr(g.0)).collect();
    let sort = dd(
        &sort_labels.iter().map(String::as_str).collect::<Vec<_>>(),
        &tr("Order"),
    );
    let colors = dd(&[], &tr("Colour filter"));
    let books = dd(&[], &tr("Book filter"));
    let notes_labels = [tr("Notes: any"), tr("With a note"), tr("Without a note")];
    let has_note = dd(
        &notes_labels.iter().map(String::as_str).collect::<Vec<_>>(),
        &tr("Note filter"),
    );
    // the dates are kept in two hidden entries (the filter and the test commands read them); the
    // user picks them with the calendar buttons
    let from = gtk::Entry::new();
    let to = gtk::Entry::new();

    // ---- one compact toolbar: search, Filter (with a count), View (order and grouping) -----------
    let badge = gtk::Label::new(None);
    badge.add_css_class("notes-badge");
    badge.set_visible(false);
    let filter_btn = gtk::MenuButton::new();
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&gtk::Image::from_icon_name("funnel-symbolic"));
        row.append(&gtk::Label::new(Some(&tr("Filter"))));
        row.append(&badge);
        filter_btn.set_child(Some(&row));
        super::audit::label(&filter_btn, &tr("Filter highlights"));
    }
    {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        col.set_margin_all(12);
        col.set_width_request(280);
        let field = |name: &str, w: &gtk::Widget| {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(name));
            l.set_xalign(0.0);
            l.set_width_chars(8);
            l.add_css_class("dim-label");
            w.set_hexpand(true);
            r.append(&l);
            r.append(w);
            r
        };
        col.append(&field(&tr("Colour"), colors.upcast_ref()));
        col.append(&field(&tr("Book"), books.upcast_ref()));
        col.append(&field(&tr("Notes"), has_note.upcast_ref()));
        col.append(&date_picker(&tr("From"), &from));
        col.append(&date_picker(&tr("To"), &to));
        let clear = gtk::Button::with_label(&tr("Clear Filters"));
        clear.set_margin_top(4);
        {
            let (c, b, h, f, t) = (
                colors.clone(),
                books.clone(),
                has_note.clone(),
                from.clone(),
                to.clone(),
            );
            clear.connect_clicked(move |_| {
                c.set_selected(0);
                b.set_selected(0);
                h.set_selected(0);
                f.set_text("");
                t.set_text("");
            });
        }
        col.append(&clear);
        let pop = gtk::Popover::new();
        pop.set_child(Some(&col));
        filter_btn.set_popover(Some(&pop));
    }
    let view_btn = view_button(&sort, &sort_labels, &group, &group_labels);
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    toolbar.set_margin_start(12);
    toolbar.set_margin_end(12);
    toolbar.set_margin_top(10);
    toolbar.append(&search);
    toolbar.append(&filter_btn);
    toolbar.append(&view_btn);
    let history_btn = gtk::Button::from_icon_name("document-open-recent-symbolic");
    history_btn.set_tooltip_text(Some(&tr("History of Deleted and Changed Notes")));
    super::audit::label(&history_btn, &tr("History"));
    {
        let (shared, open) = (shared.clone(), open.clone());
        history_btn.connect_clicked(move |b| {
            let open = open.clone();
            super::annotation_history::show(
                &shared,
                b,
                None,
                Some(Rc::new(move |book, _| open(book))),
            );
        });
    }
    toolbar.append(&history_btn);

    // the type of entry: a segmented control of four toggle buttons, the choice is remembered
    let saved_kind = KindFilter::parse(&shared.cfg.borrow().notes_kind);
    let kinds_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    kinds_box.add_css_class("linked");
    kinds_box.set_halign(gtk::Align::Start);
    kinds_box.set_margin_start(12);
    kinds_box.set_margin_end(12);
    kinds_box.set_margin_top(8);
    super::audit::label(&kinds_box, &tr("Show"));
    let mut kinds: Vec<(KindFilter, gtk::ToggleButton)> = Vec::new();
    for k in KindFilter::ALL {
        let label = match k {
            KindFilter::All => tr("All"),
            KindFilter::Highlights => tr("Highlights"),
            KindFilter::Notes => tr("Notes"),
            KindFilter::Bookmarks => tr("Bookmarks"),
        };
        let b = gtk::ToggleButton::with_label(&label);
        b.set_active(k == saved_kind);
        b.set_widget_name(&format!("notes-kind-{}", k.name()));
        super::audit::label(&b, &trf("Show {}", &[&label]));
        if let Some((_, first)) = kinds.first() {
            b.set_group(Some(first));
        }
        kinds_box.append(&b);
        kinds.push((k, b));
    }

    let chips = adw::WrapBox::new();
    chips.set_child_spacing(6);
    chips.set_line_spacing(4);
    chips.set_hexpand(true);
    chips.set_visible(false);
    let summary = gtk::Label::new(None);
    summary.add_css_class("dim-label");
    summary.add_css_class("caption");
    summary.set_xalign(1.0);
    summary.set_valign(gtk::Align::Start);
    summary.set_ellipsize(pango::EllipsizeMode::End);
    let info = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    info.set_margin_start(12);
    info.set_margin_end(12);
    info.set_margin_top(6);
    info.append(&chips);
    summary.set_hexpand(true);
    info.append(&summary);
    let legend = gtk::Label::new(None);
    legend.set_use_markup(true);
    legend.set_xalign(0.0);
    legend.set_wrap(true);
    legend.add_css_class("caption");
    legend.set_margin_start(12);
    legend.set_margin_end(12);
    legend.set_margin_top(2);
    super::audit::label(&legend, &tr("Colour legend"));

    // the list is as wide as the window: cards in as many columns as fit
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_bottom(12);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&list));
    sw.set_vexpand(true);
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    let empty = adw::StatusPage::builder()
        .icon_name("document-edit-symbolic")
        .title(tr("No highlights"))
        .description(tr("Highlights and notes you make while reading appear here, with the books they come from."))
        .build();
    {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_halign(gtk::Align::Center);
        let open = gtk::Button::with_label(&tr("Open Library"));
        open.add_css_class("pill");
        open.set_action_name(Some("lib.view-home"));
        let import = gtk::Button::with_label(&tr("Import Highlights"));
        import.add_css_class("pill");
        import.set_action_name(Some("lib.import-highlights"));
        row.append(&open);
        row.append(&import);
        empty.set_child(Some(&row));
    }
    let stack = gtk::Stack::new();
    stack.add_named(&sw, Some("list"));
    // scrolled, so that large text cannot make the empty page taller than the window (XC-7)
    let empty_scroll = gtk::ScrolledWindow::builder()
        .child(&empty)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    stack.add_named(&empty_scroll, Some("empty"));

    let copy_btn = gtk::Button::with_label(&tr("Copy"));
    copy_btn.set_visible(false);
    copy_btn.set_tooltip_text(Some(&tr("Copy the Selected Highlights")));
    let del_btn = gtk::Button::with_label(&tr("Delete"));
    del_btn.add_css_class("destructive-action");
    del_btn.set_visible(false);
    let export_btn = gtk::MenuButton::new();
    export_btn.set_label(&tr("Export"));
    export_btn.add_css_class("suggested-action");
    super::audit::label(&export_btn, &tr("Export or send the highlights"));
    export_btn.set_menu_model(Some(&export_menu(0)));

    let bar = gtk::ActionBar::new();
    bar.pack_start(&copy_btn);
    bar.pack_end(&export_btn);
    bar.pack_end(&del_btn);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_css_class("notes-view");
    body.append(&toolbar);
    body.append(&kinds_box);
    body.append(&info);
    body.append(&legend);
    body.append(&stack);

    body.set_vexpand(true);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&body);
    root.append(&bar);
    let dialog = root.clone();

    let st = Rc::new(State {
        shared: shared.clone(),
        host: root.clone().upcast(),
        on_close: Rc::new(|| {}),
        open,
        toast,
        rows: RefCell::new(Vec::new()),
        filter: RefCell::new(Filter {
            kind: saved_kind,
            ..Default::default()
        }),
        group: Cell::new(Group::Book),
        sort: Cell::new(Sort::Newest),
        selected: RefCell::new(HashSet::new()),
        limit: Cell::new(PAGE),
        list,
        chips,
        badge,
        has_note: has_note.clone(),
        from: from.clone(),
        to: to.clone(),
        stack,
        summary,
        legend: legend.clone(),
        export_btn: export_btn.clone(),
        copy_btn: copy_btn.clone(),
        del_btn: del_btn.clone(),
        on_selection: RefCell::new(None),
        shown: RefCell::new(Vec::new()),
        colors: colors.clone(),
        books: books.clone(),
        book_ids: RefCell::new(Vec::new()),
        color_names: RefCell::new(Vec::new()),
        toolbar: toolbar.clone(),
        kinds: kinds.clone(),
        empty: empty.clone(),
        marks: RefCell::new(HashMap::new()),
        undo,
    });
    for (k, b) in &kinds {
        let (st, k) = (st.clone(), *k);
        b.connect_toggled(move |b| {
            if !b.is_active() {
                return;
            }
            st.filter.borrow_mut().kind = k;
            {
                let mut c = st.shared.cfg.borrow_mut();
                if c.notes_kind != k.name() {
                    c.notes_kind = k.name().to_string();
                    c.save();
                }
            }
            st.limit.set(PAGE);
            st.refresh();
        });
    }

    let changed = |st: &Rc<State>| {
        st.limit.set(PAGE);
        st.refresh();
    };
    {
        let st = st.clone();
        search.connect_search_changed(move |e| {
            st.filter.borrow_mut().text = e.text().to_string();
            changed(&st);
        });
    }
    {
        let st = st.clone();
        group.connect_selected_notify(move |d| {
            st.group.set(
                GROUPS
                    .get(d.selected() as usize)
                    .map_or(Group::Book, |g| g.1),
            );
            changed(&st);
        });
    }
    {
        let st = st.clone();
        sort.connect_selected_notify(move |d| {
            st.sort.set(
                SORTS
                    .get(d.selected() as usize)
                    .map_or(Sort::Newest, |g| g.1),
            );
            changed(&st);
        });
    }
    {
        let st = st.clone();
        colors.connect_selected_notify(move |d| {
            let names = st.color_names.borrow();
            st.filter.borrow_mut().colors = match d.selected() {
                0 => vec![],
                n => names.get(n as usize - 1).cloned().into_iter().collect(),
            };
            drop(names);
            changed(&st);
        });
    }
    {
        let st = st.clone();
        books.connect_selected_notify(move |d| {
            let ids = st.book_ids.borrow();
            st.filter.borrow_mut().book = match d.selected() {
                0 => None,
                n => ids.get(n as usize - 1).copied(),
            };
            drop(ids);
            changed(&st);
        });
    }
    {
        let st = st.clone();
        has_note.connect_selected_notify(move |d| {
            st.filter.borrow_mut().has_note = match d.selected() {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            };
            changed(&st);
        });
    }
    for (e, is_from) in [(&from, true), (&to, false)] {
        let st = st.clone();
        e.connect_changed(move |e| {
            let day = nq::parse_day(&e.text());
            e.set_css_classes(if day.is_none() && !e.text().trim().is_empty() {
                &["error"]
            } else {
                &[]
            });
            let mut f = st.filter.borrow_mut();
            if is_from {
                f.from = day;
            } else {
                f.to = day;
            }
            drop(f);
            changed(&st);
        });
    }
    {
        let st2 = st.clone();
        copy_btn.connect_clicked(move |_| st2.copy_selection());
        let st2 = st.clone();
        del_btn.connect_clicked(move |_| st2.confirm_delete_selected());
    }

    let actions = gio::SimpleActionGroup::new();
    {
        let a = gio::SimpleAction::new("export", Some(glib::VariantTy::STRING));
        let s1 = st.clone();
        a.connect_activate(move |_, p| {
            if let Some(id) = p.and_then(|v| v.str()) {
                s1.export_dialog(id);
            }
        });
        actions.add_action(&a);
        let a = gio::SimpleAction::new("anki", Some(glib::VariantTy::STRING));
        let s1 = st.clone();
        a.connect_activate(move |_, p| {
            if let Some(file) = p.and_then(|v| v.str()) {
                s1.anki_dialog(file);
            }
        });
        actions.add_action(&a);
        let a = gio::SimpleAction::new("readwise", None);
        let s2 = st.clone();
        a.connect_activate(move |_, _| s2.readwise());
        actions.add_action(&a);
        let a = gio::SimpleAction::new("token", None);
        let s2 = st.clone();
        a.connect_activate(move |_, _| s2.ask_token(|_| {}));
        actions.add_action(&a);
    }
    dialog.insert_action_group("notes", Some(&actions));

    st.reload();

    // ---- test commands ----------------------------------------------------------------------
    {
        let st = st.clone();
        let (search, group, colors, books, has_note, from) = (
            search.clone(),
            group.clone(),
            colors.clone(),
            books.clone(),
            has_note.clone(),
            from.clone(),
        );
        let cmd: Rc<dyn Fn(&str)> = Rc::new(move |c: &str| {
            let (name, arg) = c.split_once('=').unwrap_or((c, ""));
            match name {
                "search" => {
                    st.filter.borrow_mut().text = arg.to_string();
                    st.limit.set(PAGE);
                    st.refresh();
                    search.set_text(arg);
                }
                "group" => group.set_selected(
                    GROUPS
                        .iter()
                        .position(|g| format!("{:?}", g.1).eq_ignore_ascii_case(arg))
                        .unwrap_or(0) as u32,
                ),
                "color" => {
                    let pos = st.color_names.borrow().iter().position(|c| c == arg);
                    colors.set_selected(pos.map_or(0, |p| p as u32 + 1));
                }
                "book" => {
                    let pos = arg
                        .parse::<u64>()
                        .ok()
                        .and_then(|b| st.book_ids.borrow().iter().position(|i| *i == b));
                    books.set_selected(pos.map_or(0, |p| p as u32 + 1));
                }
                "hasnote" => has_note.set_selected(match arg {
                    "yes" => 1,
                    "no" => 2,
                    _ => 0,
                }),
                "from" => from.set_text(arg),
                // `library:notes:kind=bookmarks` picks the type of entry (all, highlights, notes, bookmarks)
                "kind" => {
                    let want = KindFilter::parse(arg);
                    if let Some((_, b)) = st.kinds.iter().find(|(k, _)| *k == want) {
                        b.set_active(true);
                    }
                }
                // `library:notes:kinds` prints the type buttons and the one that is on
                "kinds" => eprintln!(
                    "notes_kinds: {} on={}",
                    st.kinds
                        .iter()
                        .map(|(_, b)| b.label().unwrap_or_default().to_string())
                        .collect::<Vec<_>>()
                        .join("|"),
                    st.kinds
                        .iter()
                        .find(|(_, b)| b.is_active())
                        .map_or("none", |(k, _)| k.name())
                ),
                "select-all" => st.select_all(),
                "select-first" => {
                    let k = st
                        .shown
                        .borrow()
                        .first()
                        .map(|&i| key_of(&st.rows.borrow()[i]));
                    if let Some(k) = k {
                        eprintln!(
                            "notes_select: book={} id={} shown={} filter={:?}",
                            k.0,
                            k.1,
                            st.shown.borrow().len(),
                            st.filter.borrow().text
                        );
                        st.selected.borrow_mut().insert(k);
                        st.refresh();
                    }
                }
                "clear" => st.clear_selection(),
                "checkgeom" => {
                    for l in check_geometry(st.list.upcast_ref()) {
                        eprintln!("{l}");
                    }
                }
                // `library:notes:hover=N` marks card N as under the pointer (`hover=-1` clears it)
                "hover" => {
                    let n: i64 = arg.parse().unwrap_or(-1);
                    let mut i = 0;
                    let mut c = st.list.first_child();
                    while let Some(w) = c {
                        if let Some(flow) = w.downcast_ref::<gtk::FlowBox>() {
                            let mut ch = flow.first_child();
                            while let Some(row) = ch {
                                if i == n {
                                    row.add_css_class("hovered");
                                } else {
                                    row.remove_css_class("hovered");
                                }
                                sync_check(&row);
                                i += 1;
                                ch = row.next_sibling();
                            }
                        }
                        c = w.next_sibling();
                    }
                }
                "open-first" => {
                    let k = st
                        .shown
                        .borrow()
                        .first()
                        .map(|&i| key_of(&st.rows.borrow()[i]));
                    if let Some(k) = k {
                        st.open_at(&k);
                    }
                }
                "edit-first" => {
                    let k = st
                        .shown
                        .borrow()
                        .first()
                        .map(|&i| key_of(&st.rows.borrow()[i]));
                    if let Some(k) = k {
                        st.apply_note(&k, arg);
                    }
                }
                "delete-first" => {
                    let k = st
                        .shown
                        .borrow()
                        .first()
                        .map(|&i| key_of(&st.rows.borrow()[i]));
                    if let Some(k) = k {
                        st.apply_delete(&k);
                    }
                }
                "export" => {
                    if let Some((id, path)) = arg.split_once(':') {
                        eprintln!("notes_export_result: {}", st.export_to(id, Path::new(path)));
                    }
                }
                "token" => {
                    let mut c = st.shared.cfg.borrow_mut();
                    c.readwise_token = arg.to_string();
                    c.save();
                }
                "readwise" => st.readwise(),
                // the Edit Note dialog of the first note: open, type, press Ctrl+Enter, cancel
                "edit-dialog" => {
                    let k = st
                        .shown
                        .borrow()
                        .first()
                        .map(|&i| key_of(&st.rows.borrow()[i]));
                    if let Some(k) = k {
                        st.edit_note(&k);
                    }
                }
                "edit-type" => EDIT.with(|e| {
                    if let Some((tv, _, _)) = e.borrow().as_ref() {
                        tv.buffer().set_text(arg);
                    }
                }),
                "edit-save" => {
                    let f = EDIT.with(|e| e.borrow().as_ref().map(|(_, f, _)| f.clone()));
                    if let Some(f) = f {
                        f();
                    }
                }
                "edit-cancel" => EDIT.with(|e| {
                    if let Some((_, _, d)) = e.borrow_mut().take() {
                        d.close();
                    }
                }),
                // the keys that save the note from inside the text
                "edit-keys" => EDIT.with(|e| {
                    if let Some((tv, _, _)) = e.borrow().as_ref() {
                        let list = tv.observe_controllers();
                        let keys: Vec<String> = (0..list.n_items())
                            .filter_map(|i| list.item(i).and_downcast::<gtk::ShortcutController>())
                            .flat_map(|c| {
                                (0..c.n_items())
                                    .filter_map(|j| c.item(j).and_downcast::<gtk::Shortcut>())
                                    .filter_map(|s| s.trigger().map(|t| t.to_str().to_string()))
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        eprintln!("notes_editdialog_keys: {}", keys.join(" "));
                    }
                }),
                "edit-state" => EDIT.with(|e| {
                    let open = e.borrow().as_ref().map(|(tv, _, d)| {
                        let b = tv.buffer();
                        (
                            d.is_mapped(),
                            b.text(&b.start_iter(), &b.end_iter(), false).to_string(),
                        )
                    });
                    eprintln!("notes_editdialog: {open:?}");
                }),
                // the Anki options dialog of a file type, and the export id of a choice
                "anki-dialog" => st.anki_dialog(if arg.is_empty() { "apkg" } else { arg }),
                "anki-id" => {
                    let mut p = arg.split(':');
                    let (file, cloze, vocab) = (
                        p.next().unwrap_or(""),
                        p.next() == Some("cloze"),
                        p.next() == Some("vocab"),
                    );
                    eprintln!("notes_anki: id={:?}", anki_id(file, cloze, vocab));
                }
                // the Export menu as the labels of its entries
                "export-menu" => {
                    let m = export_menu(0);
                    let mut labels = Vec::new();
                    menu_labels(m.upcast_ref(), &mut labels);
                    eprintln!(
                        "notes_menu: top={} anki={} labels={}",
                        menu_top_count(m.upcast_ref()),
                        labels
                            .iter()
                            .filter(|l| l.starts_with("Anki ") && **l != tr("Anki Flashcards"))
                            .count(),
                        labels.join("|")
                    );
                }
                // the toolbar at the width it has: the least it needs, and what it has
                "toolbar" => eprintln!(
                    "notes_toolbar: min={} width={}",
                    st.toolbar_min(),
                    st.host.width()
                ),
                "dump" => {
                    let rows = st.rows.borrow();
                    let shown = st.shown.borrow();
                    let sections = nq::group(&rows, &shown, st.group.get());
                    let titles: Vec<String> = sections
                        .iter()
                        .map(|s| format!("{}={}", s.title, s.rows.len()))
                        .collect();
                    let first = shown.first().map(|&i| {
                        let r = &rows[i];
                        format!("{}|{}|{}", r.book_title, r.place(), r.note)
                    });
                    eprintln!(
                        "notes_state: total={} shown={} selected={} sections=[{}] first={:?} bookmarks={} kind={}",
                        rows.len(),
                        shown.len(),
                        st.selected.borrow().len(),
                        titles.join(";"),
                        first,
                        shown
                            .iter()
                            .filter(|&&i| rows[i].kind == RowKind::Bookmark)
                            .count(),
                        st.filter.borrow().kind.name()
                    );
                }
                "selbar" => eprintln!(
                    "notes_bar: selected={} copy={} delete={} export_tip={:?} selecting_cards={}",
                    st.selected.borrow().len(),
                    st.copy_btn.is_visible(),
                    st.del_btn.is_visible(),
                    st.export_btn.tooltip_text().unwrap_or_default(),
                    st.selecting_cards()
                ),
                "copy-selected" => st.copy_selection(),
                "delete-selected" => st.confirm_delete_selected(),
                "close" => {}
                other => eprintln!("notes: unknown command {other}"),
            }
        });
        DEV.with(|d| *d.borrow_mut() = Some(cmd));
    }
    let (st2, st3, st4, st5) = (st.clone(), st.clone(), st.clone(), st.clone());
    View {
        widget: root.upcast(),
        search,
        reload: Rc::new(move || st2.reload()),
        on_selection: Rc::new(move |f| *st3.on_selection.borrow_mut() = Some(f)),
        select_all: Rc::new(move || st4.select_all()),
        clear_selection: Rc::new(move || st5.clear_selection()),
    }
}

/// Dev hook `library:notes=…` (test infrastructure): `open` is handled by the library, the rest
/// goes to the open window.
pub fn dev_command(cmd: &str) {
    match DEV.with(|d| d.borrow().clone()) {
        Some(f) => f(cmd),
        None => eprintln!("notes: no window"),
    }
}

/// A button that shows the day kept in `entry` (or "Any time") and opens a calendar to change it.
/// The entry holds `YYYY-MM-DD` or nothing, which is what the filter reads.
fn date_picker(label: &str, entry: &gtk::Entry) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let l = gtk::Label::new(Some(label));
    l.add_css_class("dim-label");
    row.append(&l);
    let btn = gtk::MenuButton::new();
    let any = tr("Any time");
    btn.set_label(&any);
    super::audit::label(&btn, &format!("{label}: {any}"));
    let cal = gtk::Calendar::new();
    let clear = gtk::Button::with_label(&tr("Any time"));
    clear.add_css_class("flat");
    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.set_margin_all(6);
    col.append(&cal);
    col.append(&clear);
    let pop = gtk::Popover::new();
    pop.set_child(Some(&col));
    btn.set_popover(Some(&pop));
    {
        let (e, p) = (entry.clone(), pop.clone());
        cal.connect_day_selected(move |c| {
            let d = c.date();
            e.set_text(&format!(
                "{:04}-{:02}-{:02}",
                d.year(),
                d.month(),
                d.day_of_month()
            ));
            p.popdown();
        });
        let (e, p) = (entry.clone(), pop.clone());
        clear.connect_clicked(move |_| {
            e.set_text("");
            p.popdown();
        });
    }
    {
        let (b, any, label) = (btn.clone(), any, label.to_string());
        entry.connect_changed(move |e| {
            let t = e.text();
            let shown = if t.trim().is_empty() {
                any.clone()
            } else {
                t.to_string()
            };
            b.set_label(&shown);
            super::audit::label(&b, &format!("{label}: {shown}"));
        });
    }
    row.append(&btn);
    row
}

/// One choice of a drop-down as a column of radio buttons (the drop-down stays the model and is not
/// shown). Returns the column and a function that shows the drop-down's choice.
fn radio_column(dd: &gtk::DropDown, labels: &[String]) -> (gtk::Box, Rc<dyn Fn(&gtk::DropDown)>) {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let mut radios: Vec<gtk::CheckButton> = Vec::new();
    for (i, l) in labels.iter().enumerate() {
        let r = gtk::CheckButton::with_label(l);
        if let Some(first) = radios.first() {
            r.set_group(Some(first));
        }
        let dd = dd.clone();
        r.connect_toggled(move |r| {
            if r.is_active() {
                dd.set_selected(i as u32);
            }
        });
        col.append(&r);
        radios.push(r);
    }
    let sync: Rc<dyn Fn(&gtk::DropDown)> = Rc::new(move |d: &gtk::DropDown| {
        if let Some(r) = radios.get(d.selected() as usize) {
            r.set_active(true);
        }
    });
    sync(dd);
    (col, sync)
}

/// The View button of the toolbar: one popover with the order and the grouping.
fn view_button(
    sort: &gtk::DropDown,
    sort_labels: &[String],
    group: &gtk::DropDown,
    group_labels: &[String],
) -> gtk::MenuButton {
    let btn = gtk::MenuButton::new();
    btn.set_icon_name("view-sort-descending-symbolic");
    btn.set_tooltip_text(Some(&tr("View")));
    super::audit::label(&btn, &tr("View options of the highlights"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_all(12);
    for (title, dd, labels) in [
        (tr("Order"), sort, sort_labels),
        (tr("Group by"), group, group_labels),
    ] {
        let head = gtk::Label::new(Some(&title));
        head.add_css_class("heading");
        head.set_xalign(0.0);
        body.append(&head);
        let (col, sync) = radio_column(dd, labels);
        body.append(&col);
        dd.connect_selected_notify(move |d| sync(d));
    }
    let pop = gtk::Popover::new();
    pop.set_child(Some(&body));
    btn.set_popover(Some(&pop));
    btn
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn row(text: &str, note: &str) -> NoteRow {
        NoteRow {
            kind: RowKind::Highlight,
            frac: 0.0,
            book_id: 1,
            highlight_id: "h".into(),
            book_title: "B".into(),
            book_author: None,
            book_tags: vec![],
            spine: 0,
            chapter: None,
            text: text.into(),
            note: note.into(),
            color: "yellow".into(),
            style: bookx_lookup::export::HighlightStyle::Highlight,
            created: Utc::now(),
            page: None,
            unplaced: false,
        }
    }

    #[test]
    fn export_says_what_it_covers() {
        assert_eq!(export_heading(0), "Export Everything Shown");
        assert_eq!(export_heading(3), "Export 3 Selected");
    }

    #[test]
    fn copied_text_has_the_quote_then_its_note() {
        let t = copy_text(&[row(" one ", ""), row("two", " mine ")]);
        assert_eq!(t, "one\n\ntwo\nmine");
        assert_eq!(copy_text(&[]), "");
    }
}
