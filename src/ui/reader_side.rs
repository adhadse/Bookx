//! The reader's sidebar: the book (cover, title, author, Properties) over a switcher for the
//! contents, the bookmarks and the notes & highlights.
//!
//! All three lists are `GtkListView`s on list stores: only the rows on the screen exist as widgets,
//! so opening the sidebar of a book with thousands of contents entries and hundreds of highlights
//! costs the same as opening it on a short book (the old `GtkListBox` made a widget per row, which
//! made opening and closing slow). A list that is not on the screen is not rebuilt until it is.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib, pango};

use super::reader::HIGHLIGHT_COLORS;
use crate::i18n::tr;
use crate::side_notes::{self, Entry, Filter, Row};
use bookx_text::highlight::HighlightStyle;

/// What the sidebar asks the reader to do.
#[derive(Debug, Clone)]
pub enum Cmd {
    GoHref(String),
    GoBookmark(String),
    RemoveBookmark(String),
    GoHighlight(String),
    DeleteHighlight(String),
    EditNote(String),
    Recolor(String, String),
    Restyle(String, String),
    Properties,
}

#[derive(Debug, Clone)]
pub struct TocItem {
    pub title: String,
    pub depth: usize,
    pub href: String,
}

#[derive(Debug, Clone)]
pub struct BmItem {
    pub id: String,
    pub title: String,
    pub sub: String,
}

pub struct Head {
    pub title: String,
    pub author: String,
    pub cover: Option<PathBuf>,
}

type Send = Rc<dyn Fn(Cmd)>;

struct State {
    toc_store: gio::ListStore,
    toc_sel: gtk::SingleSelection,
    toc_view: gtk::ListView,
    bm_store: gio::ListStore,
    bm_view: gtk::ListView,
    notes_view: gtk::ListView,
    bm_stack: gtk::Stack,
    notes_store: gio::ListStore,
    notes_stack: gtk::Stack,
    notes_status: adw::StatusPage,
    notes_page: gtk::Box,
    summary: gtk::Label,
    filter_btn: gtk::MenuButton,
    entries: RefCell<Vec<Entry>>,
    filter: RefCell<Filter>,
    dirty: Cell<bool>,
    rebuilds: Cell<u32>,
    toc_current: Cell<Option<u32>>,
}

#[derive(Clone)]
pub struct Side {
    pub root: gtk::Box,
    pub stack: adw::ViewStack,
    st: Rc<State>,
}

fn rgb16(c: (f64, f64, f64)) -> (u16, u16, u16) {
    let f = |v: f64| (v.clamp(0.0, 1.0) * 65535.0) as u16;
    (f(c.0), f(c.1), f(c.2))
}

fn colour_rgb(name: &str) -> (f64, f64, f64) {
    super::reader::color_of(name)
}

/// The Pango attributes that draw a quote the way the highlight is drawn on the page.
pub fn quote_attrs(color: &str, style: &str) -> pango::AttrList {
    let (r, g, b) = rgb16(colour_rgb(color));
    let al = pango::AttrList::new();
    match HighlightStyle::from_name(style).unwrap_or_default() {
        HighlightStyle::Background => {
            al.insert(pango::AttrColor::new_background(r, g, b));
            al.insert(pango::AttrInt::new_background_alpha(42000));
        }
        HighlightStyle::Underline => {
            al.insert(pango::AttrInt::new_underline(pango::Underline::Single));
            al.insert(pango::AttrColor::new_underline_color(r, g, b));
        }
        HighlightStyle::Strikethrough => {
            al.insert(pango::AttrInt::new_strikethrough(true));
            al.insert(pango::AttrColor::new_strikethrough_color(r, g, b));
        }
        HighlightStyle::Squiggly => {
            al.insert(pango::AttrInt::new_underline(pango::Underline::Error));
            al.insert(pango::AttrColor::new_underline_color(r, g, b));
        }
        HighlightStyle::Invert => {
            al.insert(pango::AttrColor::new_background(r / 4, g / 4, b / 4));
            al.insert(pango::AttrColor::new_foreground(65535, 65535, 65535));
        }
    }
    al
}

pub fn style_label(name: &str) -> String {
    match HighlightStyle::from_name(name).unwrap_or_default() {
        HighlightStyle::Background => tr("Background"),
        HighlightStyle::Underline => tr("Underline"),
        HighlightStyle::Strikethrough => tr("Strikethrough"),
        HighlightStyle::Squiggly => tr("Squiggly"),
        HighlightStyle::Invert => tr("Inverted"),
    }
}

fn obj<T: 'static>(v: T) -> glib::BoxedAnyObject {
    glib::BoxedAnyObject::new(v)
}

fn style_sample(style: HighlightStyle) -> gtk::Label {
    let l = gtk::Label::new(Some("Aa"));
    l.set_attributes(Some(&quote_attrs("yellow", style.name())));
    l
}

impl Side {
    pub fn build(head: &Head, send: Send) -> Side {
        let stack = adw::ViewStack::new();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // the sidebar follows the app's colours, not the book's reading theme (see style.css)
        root.add_css_class("reader-side");

        // ---- the book -------------------------------------------------------
        let book = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        book.set_margin_start(12);
        book.set_margin_end(8);
        book.set_margin_top(10);
        book.set_margin_bottom(6);
        let cover = gtk::Picture::new();
        cover.set_size_request(40, 58);
        cover.set_content_fit(gtk::ContentFit::Cover);
        cover.set_can_shrink(true);
        cover.set_halign(gtk::Align::Start);
        cover.set_valign(gtk::Align::Center);
        cover.add_css_class("card");
        cover.set_accessible_role(gtk::AccessibleRole::Img);
        super::audit::label(&cover, &tr("Cover"));
        match head
            .cover
            .as_ref()
            .and_then(|p| super::covers::thumbnail(p, 40))
        {
            Some(t) => cover.set_paintable(Some(&t)),
            None => cover.set_visible(false),
        }
        book.append(&cover);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_hexpand(true);
        names.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&head.title));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_lines(2);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_selectable(false);
        names.append(&title);
        let author = gtk::Label::new(Some(&head.author));
        author.add_css_class("dim-label");
        author.add_css_class("caption");
        author.set_xalign(0.0);
        author.set_ellipsize(pango::EllipsizeMode::End);
        author.set_visible(!head.author.trim().is_empty());
        names.append(&author);
        book.append(&names);
        let props_btn = gtk::Button::from_icon_name("document-properties-symbolic");
        props_btn.add_css_class("flat");
        props_btn.set_valign(gtk::Align::Center);
        props_btn.set_tooltip_text(Some(&tr("Properties")));
        super::audit::label(&props_btn, &tr("Properties"));
        {
            let s = send.clone();
            props_btn.connect_clicked(move |_| s(Cmd::Properties));
        }
        book.append(&props_btn);
        root.append(&book);

        // ---- the switcher -----------------------------------------------------
        let switcher = adw::InlineViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_display_mode(adw::InlineViewSwitcherDisplayMode::Labels);
        switcher.set_homogeneous(true);
        switcher.set_margin_start(8);
        switcher.set_margin_end(8);
        switcher.set_margin_bottom(6);
        root.append(&switcher);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        // ---- contents -----------------------------------------------------------
        let toc_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let toc_sel = gtk::SingleSelection::new(Some(toc_store.clone()));
        toc_sel.set_autoselect(false);
        toc_sel.set_can_unselect(true);
        toc_sel.set_selected(gtk::INVALID_LIST_POSITION);
        let f = gtk::SignalListItemFactory::new();
        f.connect_setup(|_, li| {
            let Some(li) = li.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_wrap(true);
            l.set_wrap_mode(pango::WrapMode::WordChar);
            l.set_margin_end(8);
            l.set_margin_top(6);
            l.set_margin_bottom(6);
            li.set_child(Some(&l));
            let l2 = l.clone();
            li.connect_item_notify(move |li| {
                let Some(o) = li.item().and_downcast::<glib::BoxedAnyObject>() else {
                    return;
                };
                let t = o.borrow::<TocItem>();
                l2.set_label(&t.title);
                l2.set_margin_start(10 + 14 * t.depth as i32);
                if t.depth == 0 {
                    l2.add_css_class("heading");
                } else {
                    l2.remove_css_class("heading");
                }
            });
        });
        let toc_view = gtk::ListView::new(Some(toc_sel.clone()), Some(f));
        toc_view.add_css_class("navigation-sidebar");
        toc_view.set_single_click_activate(true);
        super::audit::label(&toc_view, &tr("Contents"));
        {
            let s = send.clone();
            toc_view.connect_activate(move |v, pos| {
                if let Some(o) = v
                    .model()
                    .and_then(|m| m.item(pos))
                    .and_downcast::<glib::BoxedAnyObject>()
                {
                    s(Cmd::GoHref(o.borrow::<TocItem>().href.clone()));
                }
            });
        }
        let toc_scroll = gtk::ScrolledWindow::new();
        toc_scroll.set_child(Some(&toc_view));
        toc_scroll.set_vexpand(true);
        let toc_empty = adw::StatusPage::new();
        toc_empty.set_icon_name(Some("view-list-symbolic"));
        toc_empty.set_title(&tr("No table of contents"));
        toc_empty.set_description(Some(&tr("This book does not define one")));
        toc_empty.add_css_class("compact");
        let toc_stack = gtk::Stack::new();
        toc_stack.add_named(&toc_scroll, Some("list"));
        toc_stack.add_named(&toc_empty, Some("empty"));
        toc_stack.set_visible_child_name("empty");
        {
            let ts = toc_stack.clone();
            toc_store.connect_items_changed(move |s, _, _, _| {
                ts.set_visible_child_name(if s.n_items() == 0 { "empty" } else { "list" });
            });
        }
        stack.add_titled_with_icon(
            &toc_stack,
            Some("toc"),
            &tr("Contents"),
            "view-list-symbolic",
        );

        // ---- bookmarks ---------------------------------------------------------------
        let bm_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let f = gtk::SignalListItemFactory::new();
        {
            let send = send.clone();
            f.connect_setup(move |_, li| {
                let Some(li) = li.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                row.set_margin_start(10);
                row.set_margin_end(4);
                row.set_margin_top(6);
                row.set_margin_bottom(6);
                let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
                col.set_hexpand(true);
                let t = gtk::Label::new(None);
                t.set_xalign(0.0);
                t.set_wrap(true);
                let sub = gtk::Label::new(None);
                sub.set_xalign(0.0);
                sub.add_css_class("dim-label");
                sub.add_css_class("caption");
                col.append(&t);
                col.append(&sub);
                row.append(&col);
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk::Align::Center);
                del.set_tooltip_text(Some(&tr("Remove bookmark")));
                row.append(&del);
                li.set_child(Some(&row));
                let cur = Rc::new(RefCell::new(String::new()));
                {
                    let (cur, send) = (cur.clone(), send.clone());
                    del.connect_clicked(move |_| send(Cmd::RemoveBookmark(cur.borrow().clone())));
                }
                li.connect_item_notify(move |li| {
                    let Some(o) = li.item().and_downcast::<glib::BoxedAnyObject>() else {
                        return;
                    };
                    let b = o.borrow::<BmItem>();
                    t.set_label(&b.title);
                    sub.set_label(&b.sub);
                    *cur.borrow_mut() = b.id.clone();
                });
            });
        }
        let bm_view =
            gtk::ListView::new(Some(gtk::NoSelection::new(Some(bm_store.clone()))), Some(f));
        bm_view.add_css_class("navigation-sidebar");
        bm_view.set_single_click_activate(true);
        super::audit::label(&bm_view, &tr("Bookmarks"));
        {
            let s = send.clone();
            bm_view.connect_activate(move |v, pos| {
                if let Some(o) = v
                    .model()
                    .and_then(|m| m.item(pos))
                    .and_downcast::<glib::BoxedAnyObject>()
                {
                    s(Cmd::GoBookmark(o.borrow::<BmItem>().id.clone()));
                }
            });
        }
        let bm_scroll = gtk::ScrolledWindow::new();
        bm_scroll.set_child(Some(&bm_view));
        bm_scroll.set_vexpand(true);
        let bm_empty = adw::StatusPage::new();
        bm_empty.set_icon_name(Some("user-bookmarks-symbolic"));
        bm_empty.set_title(&tr("No bookmarks yet"));
        bm_empty.set_description(Some(&tr("Press Ctrl+B to bookmark the current page")));
        bm_empty.add_css_class("compact");
        let bm_stack = gtk::Stack::new();
        bm_stack.add_named(&bm_scroll, Some("list"));
        bm_stack.add_named(&bm_empty, Some("empty"));
        bm_stack.set_visible_child_name("empty");
        stack.add_titled_with_icon(
            &bm_stack,
            Some("bookmarks"),
            &tr("Bookmarks"),
            "user-bookmarks-symbolic",
        );

        // ---- notes & highlights ------------------------------------------------------
        let notes_store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let notes_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.set_margin_start(8);
        bar.set_margin_end(8);
        bar.set_margin_top(8);
        let search = gtk::SearchEntry::new();
        search.set_hexpand(true);
        search.set_placeholder_text(Some(&tr("Search notes and highlights")));
        super::audit::label(&search, &tr("Search notes and highlights"));
        bar.append(&search);
        let filter_btn = gtk::MenuButton::new();
        filter_btn.set_label(&tr("Filter"));
        filter_btn.set_tooltip_text(Some(&tr("Filter by colour, style or notes")));
        bar.append(&filter_btn);
        notes_page.append(&bar);
        let summary = gtk::Label::new(None);
        summary.add_css_class("dim-label");
        summary.add_css_class("caption");
        summary.set_xalign(0.0);
        summary.set_margin_start(10);
        summary.set_margin_top(4);
        summary.set_margin_bottom(4);
        notes_page.append(&summary);
        let notes_status = adw::StatusPage::new();
        notes_status.add_css_class("compact");
        notes_status.set_icon_name(Some("document-edit-symbolic"));
        let notes_stack = gtk::Stack::new();
        notes_stack.set_vexpand(true);

        let f = gtk::SignalListItemFactory::new();
        {
            let send = send.clone();
            f.connect_setup(move |_, li| {
                let Some(li) = li.downcast_ref::<gtk::ListItem>() else {
                    return;
                };
                build_note_row(li, send.clone());
            });
        }
        let notes_view = gtk::ListView::new(
            Some(gtk::NoSelection::new(Some(notes_store.clone()))),
            Some(f),
        );
        notes_view.add_css_class("navigation-sidebar");
        notes_view.set_single_click_activate(true);
        super::audit::label(&notes_view, &tr("Notes and highlights"));
        {
            let s = send.clone();
            notes_view.connect_activate(move |v, pos| {
                if let Some(o) = v
                    .model()
                    .and_then(|m| m.item(pos))
                    .and_downcast::<glib::BoxedAnyObject>()
                {
                    if let NoteLine::Entry(e) = &*o.borrow::<NoteLine>() {
                        s(Cmd::GoHighlight(e.id.clone()));
                    }
                }
            });
        }
        let notes_scroll = gtk::ScrolledWindow::new();
        notes_scroll.set_child(Some(&notes_view));
        notes_stack.add_named(&notes_scroll, Some("list"));
        notes_stack.add_named(&notes_status, Some("empty"));
        notes_stack.set_visible_child_name("empty");
        notes_page.append(&notes_stack);
        stack.add_titled_with_icon(
            &notes_page,
            Some("notes"),
            &tr("Notes"),
            "document-edit-symbolic",
        );
        stack.set_vexpand(true);
        root.append(&stack);

        let st = Rc::new(State {
            toc_store,
            toc_sel,
            toc_view,
            bm_store,
            bm_view,
            notes_view,
            bm_stack,
            notes_store,
            notes_stack,
            notes_status,
            notes_page: notes_page.clone(),
            summary,
            filter_btn: filter_btn.clone(),
            entries: RefCell::new(Vec::new()),
            filter: RefCell::new(Filter::default()),
            dirty: Cell::new(false),
            rebuilds: Cell::new(0),
            toc_current: Cell::new(None),
        });
        let side = Side { root, stack, st };

        // the notes are rebuilt when the page is on the screen, not before
        {
            let s = side.clone();
            notes_page.connect_map(move |_| {
                if s.st.dirty.get() {
                    s.rebuild_notes();
                }
            });
            let s = side.clone();
            side.st.toc_view.connect_map(move |_| s.scroll_to_current());
        }
        {
            let s = side.clone();
            search.connect_search_changed(move |e| {
                s.st.filter.borrow_mut().text = e.text().to_string();
                s.rebuild_notes();
            });
        }
        filter_btn.set_popover(Some(&side.filter_popover()));
        side
    }

    fn filter_popover(&self) -> gtk::Popover {
        let pop = gtk::Popover::new();
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.set_margin_start(10);
        b.set_margin_end(10);
        b.set_margin_top(10);
        b.set_margin_bottom(10);
        let heading = |t: &str| {
            let l = gtk::Label::new(Some(t));
            l.add_css_class("caption-heading");
            l.set_xalign(0.0);
            l
        };
        b.append(&heading(&tr("Colour")));
        let colors = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let mut color_btns: Vec<(String, gtk::ToggleButton)> = Vec::new();
        for (name, _) in HIGHLIGHT_COLORS {
            let t = gtk::ToggleButton::new();
            t.add_css_class("hl-swatch");
            t.add_css_class(&format!("hl-{name}"));
            t.set_size_request(26, 26);
            t.set_tooltip_text(Some(&side_notes::colour_label(name)));
            super::audit::label(&t, &side_notes::colour_label(name));
            colors.append(&t);
            color_btns.push((name.to_string(), t));
        }
        b.append(&colors);
        b.append(&heading(&tr("Style")));
        let styles = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let mut style_btns: Vec<(String, gtk::ToggleButton)> = Vec::new();
        for st in [
            HighlightStyle::Background,
            HighlightStyle::Underline,
            HighlightStyle::Strikethrough,
            HighlightStyle::Squiggly,
        ] {
            let t = gtk::ToggleButton::new();
            t.set_child(Some(&style_sample(st)));
            t.add_css_class("flat");
            t.set_tooltip_text(Some(&style_label(st.name())));
            super::audit::label(&t, &style_label(st.name()));
            styles.append(&t);
            style_btns.push((st.name().to_string(), t));
        }
        b.append(&styles);
        let has_note = gtk::CheckButton::with_label(&tr("Only with a note"));
        b.append(&has_note);
        let clear = gtk::Button::with_label(&tr("Clear filters"));
        clear.add_css_class("flat");
        b.append(&clear);
        pop.set_child(Some(&b));

        let apply = {
            let side = self.clone();
            let color_btns = color_btns.clone();
            let style_btns = style_btns.clone();
            let has_note = has_note.clone();
            Rc::new(move || {
                {
                    let mut f = side.st.filter.borrow_mut();
                    f.colors = color_btns
                        .iter()
                        .filter(|(_, t)| t.is_active())
                        .map(|(n, _)| n.clone())
                        .collect();
                    f.styles = style_btns
                        .iter()
                        .filter(|(_, t)| t.is_active())
                        .map(|(n, _)| n.clone())
                        .collect();
                    f.has_note = has_note.is_active();
                }
                side.rebuild_notes();
            })
        };
        for (_, t) in color_btns.iter().chain(style_btns.iter()) {
            let a = apply.clone();
            t.connect_toggled(move |_| a());
        }
        {
            let a = apply.clone();
            has_note.connect_toggled(move |_| a());
        }
        {
            let (cb, sb, hn) = (color_btns, style_btns, has_note);
            clear.connect_clicked(move |_| {
                for (_, t) in cb.iter().chain(sb.iter()) {
                    t.set_active(false);
                }
                hn.set_active(false);
            });
        }
        pop
    }

    /// Puts the keyboard focus on the list of the page that is showing.
    pub fn focus_current(&self) {
        let st = &self.st;
        match self.stack.visible_child_name().as_deref() {
            Some("bookmarks") => st.bm_view.grab_focus(),
            Some("notes") => st.notes_view.grab_focus(),
            _ => st.toc_view.grab_focus(),
        };
    }

    pub fn set_toc(&self, items: &[TocItem]) {
        let objs: Vec<glib::BoxedAnyObject> = items.iter().cloned().map(obj).collect();
        self.st
            .toc_store
            .splice(0, self.st.toc_store.n_items(), &objs);
        self.st.toc_current.set(None);
    }

    /// Marks the contents entry of the chapter on the page.
    pub fn select_toc(&self, idx: Option<usize>) {
        let pos = idx.map(|i| i as u32);
        if self.st.toc_current.get() == pos {
            return;
        }
        self.st.toc_current.set(pos);
        self.st
            .toc_sel
            .set_selected(pos.unwrap_or(gtk::INVALID_LIST_POSITION));
        if self.st.toc_view.is_mapped() {
            self.scroll_to_current();
        }
    }

    fn scroll_to_current(&self) {
        if let Some(p) = self.st.toc_current.get() {
            if p < self.st.toc_store.n_items() {
                self.st
                    .toc_view
                    .scroll_to(p, gtk::ListScrollFlags::NONE, None);
            }
        }
    }

    pub fn set_bookmarks(&self, items: Vec<BmItem>) {
        let objs: Vec<glib::BoxedAnyObject> = items.into_iter().map(obj).collect();
        self.st
            .bm_stack
            .set_visible_child_name(if objs.is_empty() { "empty" } else { "list" });
        self.st
            .bm_store
            .splice(0, self.st.bm_store.n_items(), &objs);
    }

    /// The highlights of the book. The list is rebuilt now when it is on the screen, else when
    /// the page is shown.
    pub fn set_entries(&self, entries: Vec<Entry>) {
        *self.st.entries.borrow_mut() = entries;
        if self.st.notes_page.is_mapped() {
            self.rebuild_notes();
        } else {
            self.st.dirty.set(true);
        }
    }

    fn rebuild_notes(&self) {
        let st = &self.st;
        st.dirty.set(false);
        st.rebuilds.set(st.rebuilds.get() + 1);
        let entries = st.entries.borrow();
        let filter = st.filter.borrow();
        let rows = side_notes::rows(&entries, &filter);
        let n = side_notes::count(&rows);
        let objs: Vec<glib::BoxedAnyObject> = rows
            .into_iter()
            .map(|r| {
                obj(match r {
                    Row::Chapter(t, n) => NoteLine::Chapter(t, n),
                    Row::Entry(i) => NoteLine::Entry(entries[i].clone()),
                })
            })
            .collect();
        st.notes_store.splice(0, st.notes_store.n_items(), &objs);
        let total = entries.len();
        if total == 0 {
            st.notes_status.set_title(&tr("No highlights yet"));
            st.notes_status
                .set_description(Some(&tr("Select some text to highlight it or add a note")));
        } else if n == 0 {
            st.notes_status.set_title(&tr("Nothing found"));
            st.notes_status
                .set_description(Some(&tr("No highlight matches the search and filters")));
        }
        st.notes_stack
            .set_visible_child_name(if n == 0 { "empty" } else { "list" });
        st.summary.set_label(&if total == 0 {
            String::new()
        } else if filter.is_empty() {
            crate::i18n::trf("{} highlights", &[&total.to_string()])
        } else {
            crate::i18n::trf("{} of {} highlights", &[&n.to_string(), &total.to_string()])
        });
        st.filter_btn.set_label(&if filter.colors.is_empty()
            && filter.styles.is_empty()
            && !filter.has_note
        {
            tr("Filter")
        } else {
            let k = filter.colors.len() + filter.styles.len() + usize::from(filter.has_note);
            crate::i18n::trf("Filter ({})", &[&k.to_string()])
        });
    }

    /// For the dev hooks: `toc=2000 bookmarks=3 notes_rows=510 notes=500 rebuilds=2 ...`.
    pub fn dev_summary(&self) -> String {
        let st = &self.st;
        format!(
            "toc={} bookmarks={} notes_rows={} notes={} rebuilds={} dirty={} selected_toc={:?} page={} filter={:?}",
            st.toc_store.n_items(),
            st.bm_store.n_items(),
            st.notes_store.n_items(),
            st.entries.borrow().len(),
            st.rebuilds.get(),
            st.dirty.get(),
            st.toc_current.get(),
            self.stack.visible_child_name().unwrap_or_default(),
            st.filter.borrow(),
        )
        .replace("Filter ", "")
    }

    /// Dev hook: sets the search text and filters, as the user does.
    pub fn dev_filter(&self, text: &str, color: &str, style: &str, has_note: bool) -> usize {
        {
            let mut f = self.st.filter.borrow_mut();
            f.text = text.to_string();
            f.colors = if color.is_empty() {
                Vec::new()
            } else {
                vec![color.to_string()]
            };
            f.styles = if style.is_empty() {
                Vec::new()
            } else {
                vec![style.to_string()]
            };
            f.has_note = has_note;
        }
        self.rebuild_notes();
        (0..self.st.notes_store.n_items())
            .filter_map(|i| {
                self.st
                    .notes_store
                    .item(i)
                    .and_downcast::<glib::BoxedAnyObject>()
            })
            .filter(|o| matches!(&*o.borrow::<NoteLine>(), NoteLine::Entry(_)))
            .count()
    }
}

enum NoteLine {
    Chapter(String, usize),
    Entry(Entry),
}

/// One row of the notes list: a chapter heading, or a highlight with its quote, note, date and menu.
fn build_note_row(li: &gtk::ListItem, send: Send) {
    use super::note_row::{NoteActions, NoteCard, NoteData};
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    // chapter heading
    let head = gtk::Label::new(None);
    head.add_css_class("heading");
    head.set_xalign(0.0);
    head.set_wrap(true);
    head.set_margin_start(10);
    head.set_margin_end(10);
    head.set_margin_top(12);
    head.set_margin_bottom(2);
    outer.append(&head);
    // entry: the card the library's Notes & Highlights view uses too
    let cur = Rc::new(RefCell::new(String::new()));
    let actions = {
        let (c1, s1) = (cur.clone(), send.clone());
        let (c2, s2) = (cur.clone(), send.clone());
        let (c3, s3) = (cur.clone(), send.clone());
        let (c4, s4) = (cur.clone(), send.clone());
        NoteActions {
            edit: Rc::new(move || s1(Cmd::EditNote(c1.borrow().clone()))),
            delete: Rc::new(move || s2(Cmd::DeleteHighlight(c2.borrow().clone()))),
            recolor: Some(Rc::new(move |n: &str| {
                s3(Cmd::Recolor(c3.borrow().clone(), n.to_string()))
            })),
            restyle: Some(Rc::new(move |n: &str| {
                s4(Cmd::Restyle(c4.borrow().clone(), n.to_string()))
            })),
            relocate: None,
        }
    };
    let card = NoteCard::new(&actions);
    card.root.set_margin_start(10);
    card.root.set_margin_end(6);
    card.root.set_margin_top(4);
    card.root.set_margin_bottom(4);
    outer.append(&card.root);
    li.set_child(Some(&outer));

    li.connect_item_notify(move |li| {
        let Some(o) = li.item().and_downcast::<glib::BoxedAnyObject>() else {
            return;
        };
        let line = o.borrow::<NoteLine>();
        match &*line {
            NoteLine::Chapter(t, n) => {
                head.set_label(&format!("{t}  ({n})"));
                head.set_visible(true);
                card.root.set_visible(false);
                li.set_activatable(false);
                li.set_selectable(false);
            }
            NoteLine::Entry(e) => {
                head.set_visible(false);
                card.root.set_visible(true);
                li.set_activatable(true);
                li.set_selectable(false);
                *cur.borrow_mut() = e.id.clone();
                card.set(&NoteData {
                    color: e.color.clone(),
                    style: e.style.clone(),
                    text: e.text.clone(),
                    note: e.note.clone(),
                    meta: if e.start < 0 {
                        format!(
                            "{} · {}",
                            tr("Not located in the book"),
                            side_notes::date_text(e.created)
                        )
                    } else {
                        side_notes::date_text(e.created)
                    },
                });
            }
        }
    });
}

/// A dialog to write the note of a highlight; `save` gets the text.
pub fn edit_note_dialog(
    parent: &impl IsA<gtk::Widget>,
    initial: &str,
    save: impl Fn(String) + 'static,
) {
    let d = adw::AlertDialog::new(Some(&tr("Note")), None);
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("save", &tr("Save"));
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("save"));
    d.set_close_response("cancel");
    let tv = gtk::TextView::new();
    super::audit::label(&tv, &tr("Note"));
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    tv.set_top_margin(6);
    tv.set_bottom_margin(6);
    tv.set_left_margin(6);
    tv.set_right_margin(6);
    tv.buffer().set_text(initial);
    let sc = gtk::ScrolledWindow::new();
    sc.set_min_content_height(120);
    sc.set_min_content_width(300);
    sc.add_css_class("card");
    sc.set_child(Some(&tv));
    d.set_extra_child(Some(&sc));
    let tv2 = tv.clone();
    d.connect_response(None, move |_, r| {
        if r == "save" {
            let b = tv2.buffer();
            let (s, e) = b.bounds();
            save(b.text(&s, &e, false).to_string());
        }
    });
    d.present(Some(parent));
    tv.grab_focus();
}
