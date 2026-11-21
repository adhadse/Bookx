//! The card of one highlight, the same in the reader's sidebar and in the library's Notes &
//! Highlights view: the quote drawn the way the highlight looks on the page (colour and style),
//! the written note in its own accent block with a note icon, a small dim line with where and when,
//! and a popover (Edit, Copy and Delete as icons, then colour and style) behind the row's menu
//! button, the right mouse button and the Menu key.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::prelude::*;
use relm4::gtk::{self, glib, pango};

use super::reader::HIGHLIGHT_COLORS;
use super::reader_side::{quote_attrs, style_label};
use crate::i18n::{tr, trf};
use bookx_text::highlight::HighlightStyle;

/// What a card shows.
#[derive(Debug, Clone, Default)]
pub struct NoteData {
    pub color: String,
    pub style: String,
    pub text: String,
    pub note: String,
    /// The small dim line: `Book · Chapter · 13 Nov 2026`.
    pub meta: String,
    /// A bookmark: a bookmark icon before the words, which are not painted like a highlight.
    pub bookmark: bool,
}

/// What the buttons of the card do. `recolor` and `restyle` are `None` for highlights whose look
/// cannot be changed (PDF highlights).
#[derive(Clone)]
pub struct NoteActions {
    pub edit: Rc<dyn Fn()>,
    pub delete: Rc<dyn Fn()>,
    pub recolor: Option<Rc<dyn Fn(&str)>>,
    pub restyle: Option<Rc<dyn Fn(&str)>>,
    /// A note whose text could not be found in the book: look for it again.
    pub relocate: Option<Rc<dyn Fn()>>,
    /// Set for a bookmark: the menu is *Open* and *Delete* (nothing to edit, no colour or style).
    pub open: Option<Rc<dyn Fn()>>,
}

/// The pieces of a card that change when a row is reused for another highlight.
pub struct NoteCard {
    pub root: gtk::Box,
    icon: gtk::Image,
    menu: gtk::MenuButton,
    quote: gtk::Label,
    note_block: gtk::Box,
    note: gtk::Label,
    meta: gtk::Label,
    colors: Vec<(&'static str, gtk::Button)>,
    styles: Vec<(HighlightStyle, gtk::Button)>,
    full: Rc<RefCell<String>>,
}

fn mark(b: &gtk::Button, on: bool) {
    if on {
        b.add_css_class("active");
    } else {
        b.remove_css_class("active");
    }
    b.update_state(&[gtk::accessible::State::Pressed(if on {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    })]);
}

fn icon_btn(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(tip));
    super::audit::label(&b, tip);
    b
}

impl NoteCard {
    /// The roomy card (reader sidebar).
    pub fn new(actions: &NoteActions) -> NoteCard {
        Self::build(actions, false)
    }

    /// The compact card of the library's list: a quote of one or two lines, the note in one
    /// accent line below it, and the place and date on the line of the menu button.
    pub fn compact(actions: &NoteActions) -> NoteCard {
        Self::build(actions, true)
    }

    fn build(actions: &NoteActions, compact: bool) -> NoteCard {
        let root = gtk::Box::new(gtk::Orientation::Vertical, if compact { 3 } else { 8 });
        root.add_css_class("note-card");
        if compact {
            root.add_css_class("compact");
        }
        let mark_icon = gtk::Image::from_icon_name("user-bookmarks-symbolic");
        mark_icon.add_css_class("note-bookmark-icon");
        mark_icon.set_valign(gtk::Align::Start);
        mark_icon.set_margin_top(3);
        mark_icon.set_visible(false);
        let quote = gtk::Label::new(None);
        quote.add_css_class("note-quote");
        quote.set_xalign(0.0);
        quote.set_wrap(true);
        quote.set_wrap_mode(pango::WrapMode::WordChar);
        quote.set_lines(if compact { 2 } else { 5 });
        quote.set_ellipsize(pango::EllipsizeMode::End);
        quote.set_halign(gtk::Align::Start);
        if !compact {
            root.append(&quote);
        }

        // the note: an accent block with the word Note, so it is not mistaken for the quote
        let note_block = gtk::Box::new(
            if compact {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            },
            if compact { 8 } else { 2 },
        );
        note_block.add_css_class("note-block");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = gtk::Image::from_icon_name("document-edit-symbolic");
        icon.add_css_class("note-block-title");
        let word = gtk::Label::new(Some(&tr("Note")));
        word.add_css_class("note-block-title");
        word.add_css_class("caption-heading");
        head.append(&icon);
        head.append(&word);
        let note = gtk::Label::new(None);
        note.add_css_class("note-text");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_wrap_mode(pango::WrapMode::WordChar);
        note.set_lines(if compact { 1 } else { 6 });
        note.set_hexpand(true);
        note.set_ellipsize(pango::EllipsizeMode::End);
        note.set_selectable(false);
        note_block.append(&head);
        note_block.append(&note);
        let note_block_w = note_block.clone();

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let meta = gtk::Label::new(None);
        meta.add_css_class("note-meta");
        meta.set_hexpand(true);
        meta.set_xalign(0.0);
        meta.set_ellipsize(pango::EllipsizeMode::End);
        if compact {
            // quote, place and date, and the menu on one line; the note under it
            quote.set_hexpand(true);
            quote.set_max_width_chars(34);
            note.set_max_width_chars(40);
            meta.set_hexpand(false);
            meta.set_valign(gtk::Align::Start);
            meta.set_max_width_chars(24);
            meta.set_margin_top(3);
            foot.append(&mark_icon);
            foot.append(&quote);
        }
        foot.append(&meta);
        let menu = gtk::MenuButton::new();
        menu.set_icon_name("view-more-symbolic");
        menu.add_css_class("flat");
        menu.set_valign(gtk::Align::Center);
        menu.set_tooltip_text(Some(&tr("Highlight Actions")));
        super::audit::label(&menu, &tr("Highlight actions"));
        foot.append(&menu);
        if compact {
            root.append(&foot);
            root.append(&note_block_w);
        } else {
            root.append(&note_block_w);
            root.append(&foot);
        }

        let full = Rc::new(RefCell::new(String::new()));
        let pop = gtk::Popover::new();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        col.set_margin_start(6);
        col.set_margin_end(6);
        col.set_margin_top(6);
        col.set_margin_bottom(6);
        let close = |pop: &gtk::Popover| {
            let p = pop.clone();
            move || p.popdown()
        };
        let edit = icon_btn("document-edit-symbolic", &tr("Edit note"));
        let copy = icon_btn("edit-copy-symbolic", &tr("Copy text"));
        let delete = icon_btn("user-trash-symbolic", &tr("Delete"));
        delete.add_css_class("destructive-action");
        {
            let (f, p) = (actions.edit.clone(), close(&pop));
            edit.connect_clicked(move |_| {
                p();
                f();
            });
            let (full, p) = (full.clone(), close(&pop));
            copy.connect_clicked(move |b| {
                p();
                b.clipboard().set_text(&full.borrow());
            });
            let (f, p) = (actions.delete.clone(), close(&pop));
            delete.connect_clicked(move |_| {
                p();
                f();
            });
        }
        let acts = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        acts.set_halign(gtk::Align::Center);
        if let Some(open) = &actions.open {
            let b = gtk::Button::with_label(&tr("Open"));
            b.add_css_class("flat");
            b.set_tooltip_text(Some(&tr("Open the book at this bookmark")));
            super::audit::label(&b, &tr("Open the book at this bookmark"));
            let (f, p) = (open.clone(), close(&pop));
            b.connect_clicked(move |_| {
                p();
                f();
            });
            col.append(&b);
        } else {
            acts.append(&edit);
            acts.append(&copy);
        }
        acts.append(&delete);
        col.append(&acts);
        if let Some(relocate) = &actions.relocate {
            let b = gtk::Button::with_label(&tr("Locate Again"));
            b.add_css_class("flat");
            b.set_tooltip_text(Some(&tr(
                "Search the book for this text again and put the highlight back on the page",
            )));
            super::audit::label(&b, &tr("Locate Again"));
            let (f, p) = (relocate.clone(), close(&pop));
            b.connect_clicked(move |_| {
                p();
                f();
            });
            col.append(&b);
        }

        let mut colors = Vec::new();
        let mut styles = Vec::new();
        if let (Some(recolor), Some(restyle)) = (&actions.recolor, &actions.restyle) {
            col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let section = |t: &str| {
                let l = gtk::Label::new(Some(t));
                l.add_css_class("caption-heading");
                l.add_css_class("dim-label");
                l.set_xalign(0.0);
                l.set_margin_start(4);
                l
            };
            col.append(&section(&tr("Colour")));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            row.set_halign(gtk::Align::Center);
            for (name, _) in HIGHLIGHT_COLORS {
                let tip = trf("Change colour to {}", &[&name]);
                let b = gtk::Button::new();
                b.add_css_class("hl-swatch");
                b.add_css_class(&format!("hl-{name}"));
                b.set_size_request(26, 26);
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some(&tip));
                super::audit::label(&b, &tip);
                let (f, p) = (recolor.clone(), close(&pop));
                b.connect_clicked(move |_| {
                    p();
                    f(name);
                });
                row.append(&b);
                colors.push((name, b));
            }
            col.append(&row);
            col.append(&section(&tr("Style")));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
            row.set_halign(gtk::Align::Center);
            for st in [
                HighlightStyle::Background,
                HighlightStyle::Underline,
                HighlightStyle::Strikethrough,
                HighlightStyle::Squiggly,
            ] {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                b.add_css_class("side-style");
                let sample = gtk::Label::new(Some("Aa"));
                sample.set_attributes(Some(&quote_attrs("yellow", st.name())));
                b.set_child(Some(&sample));
                b.set_tooltip_text(Some(&style_label(st.name())));
                super::audit::label(&b, &style_label(st.name()));
                let (f, p) = (restyle.clone(), close(&pop));
                b.connect_clicked(move |_| {
                    p();
                    f(st.name());
                });
                row.append(&b);
                styles.push((st, b));
            }
            col.append(&row);
        }
        pop.set_child(Some(&col));
        menu.set_popover(Some(&pop));

        // right click and the Menu key open it too
        let click = gtk::GestureClick::new();
        click.set_button(3);
        {
            let m = menu.clone();
            click.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                m.popup();
            });
        }
        root.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        {
            let m = menu.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                use gtk::gdk::Key;
                if key == Key::Menu
                    || (key == Key::F10 && mods.contains(gtk::gdk::ModifierType::SHIFT_MASK))
                {
                    m.popup();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        root.add_controller(keys);

        NoteCard {
            root,
            icon: mark_icon,
            menu,
            quote,
            note_block,
            note,
            meta,
            colors,
            styles,
            full,
        }
    }

    /// Shows a highlight in the card.
    pub fn set(&self, d: &NoteData) {
        *self.full.borrow_mut() = d.text.clone();
        self.quote
            .set_label(&crate::side_notes::shorten(&d.text, 500));
        if d.bookmark {
            self.quote.set_attributes(None);
        } else {
            self.quote
                .set_attributes(Some(&quote_attrs(&d.color, &d.style)));
        }
        self.icon.set_visible(d.bookmark);
        let tip = if d.bookmark {
            tr("Bookmark Actions")
        } else {
            tr("Highlight Actions")
        };
        self.menu.set_tooltip_text(Some(&tip));
        self.note.set_label(d.note.trim());
        self.note.set_tooltip_text(Some(d.note.trim()));
        self.quote.set_tooltip_text(Some(d.text.trim()));
        self.note_block.set_visible(!d.note.trim().is_empty());
        self.meta.set_label(&d.meta);
        for (name, b) in &self.colors {
            mark(b, *name == d.color);
        }
        for (st, b) in &self.styles {
            mark(
                b,
                HighlightStyle::from_name(&d.style).unwrap_or_default() == *st,
            );
        }
    }
}
