//! The reader menu's custom theme swatches: your own themes next to Light / Sepia / Dark / Black,
//! a "+" swatch that makes a new one and a pencil on each to edit it (the editor is
//! `reader_theme_editor`; the data is `crate::reader_themes`).

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::gtk::{self, prelude::*};

use super::SharedRef;
use crate::config::{parse_hex, ColorMode, ReaderPrefs};
use crate::reader_themes::Theme;

/// True when the prefs show exactly this custom theme.
pub fn is_active(t: &Theme, p: &ReaderPrefs) -> bool {
    p.colors == ColorMode::Custom && p.custom_fg == t.fg && p.custom_bg == t.bg
}

pub struct ThemeRow {
    pub root: gtk::FlowBox,
    shared: SharedRef,
    cur: Rc<RefCell<ReaderPrefs>>,
    emit: Rc<dyn Fn()>,
    edit: Rc<dyn Fn(Option<String>)>,
    group: gtk::ToggleButton,
    toggles: RefCell<Vec<(Theme, gtk::ToggleButton)>>,
}

fn dot(t: &Theme) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(32);
    a.set_content_height(32);
    let (fg, bg) = (
        parse_hex(&t.fg).unwrap_or((0.0, 0.0, 0.0)),
        parse_hex(&t.bg).unwrap_or((1.0, 1.0, 1.0)),
    );
    a.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        cr.arc(
            w / 2.0,
            h / 2.0,
            w.min(h) / 2.0 - 1.0,
            0.0,
            std::f64::consts::TAU,
        );
        cr.set_source_rgb(bg.0, bg.1, bg.2);
        let _ = cr.fill();
        cr.set_source_rgb(fg.0, fg.1, fg.2);
        cr.select_font_face(
            "Serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Bold,
        );
        cr.set_font_size(h * 0.5);
        cr.move_to(w * 0.3, h * 0.66);
        let _ = cr.show_text("A");
    });
    a
}

impl ThemeRow {
    /// `group` is the first built-in swatch (the radio group); `edit(None)` opens the editor for a
    /// new theme, `edit(Some(name))` for that theme.
    pub fn new(
        shared: SharedRef,
        cur: Rc<RefCell<ReaderPrefs>>,
        emit: Rc<dyn Fn()>,
        group: gtk::ToggleButton,
        edit: Rc<dyn Fn(Option<String>)>,
    ) -> Rc<Self> {
        let root = gtk::FlowBox::new();
        root.set_selection_mode(gtk::SelectionMode::None);
        root.set_max_children_per_line(6);
        root.set_homogeneous(false);
        root.set_halign(gtk::Align::Center);
        root.set_column_spacing(6);
        root.set_row_spacing(4);
        root.set_margin_bottom(6);
        let row = Rc::new(ThemeRow {
            root,
            shared,
            cur,
            emit,
            edit,
            group,
            toggles: RefCell::new(Vec::new()),
        });
        row.rebuild();
        row
    }

    /// Draws the swatches again from the saved themes.
    pub fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.root.first_child() {
            self.root.remove(&c);
        }
        self.toggles.borrow_mut().clear();
        let themes = self.shared.cfg.borrow().reader_themes.list.clone();
        for t in themes {
            let item = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let b = gtk::ToggleButton::new();
            b.add_css_class("theme-swatch");
            b.add_css_class("custom-theme-swatch");
            b.set_child(Some(&dot(&t)));
            b.set_group(Some(&self.group));
            b.set_tooltip_text(Some(&t.name));
            super::audit::label(&b, &trf("Theme {}", &[&t.name]));
            b.set_active(is_active(&t, &self.cur.borrow()));
            {
                let (row, t) = (self.clone(), t.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() && !is_active(&t, &row.cur.borrow()) {
                        let applied = t.apply(&row.cur.borrow());
                        *row.cur.borrow_mut() = applied;
                        (row.emit)();
                    }
                });
            }
            let pen = gtk::Button::from_icon_name("document-edit-symbolic");
            pen.add_css_class("flat");
            pen.add_css_class("circular");
            pen.set_valign(gtk::Align::Start);
            pen.set_tooltip_text(Some(&trf("Edit theme {}", &[&t.name])));
            super::audit::label(&pen, &trf("Edit theme {}", &[&t.name]));
            {
                let (edit, name) = (self.edit.clone(), t.name.clone());
                pen.connect_clicked(move |_| edit(Some(name.clone())));
            }
            item.append(&b);
            item.append(&pen);
            self.root.append(&item);
            self.toggles.borrow_mut().push((t, b));
        }
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("circular");
        add.set_valign(gtk::Align::Center);
        add.set_tooltip_text(Some(&tr("New custom theme")));
        super::audit::label(&add, &tr("New custom theme"));
        {
            let edit = self.edit.clone();
            add.connect_clicked(move |_| edit(None));
        }
        self.root.append(&add);
    }

    /// Marks the swatch that matches the settings.
    pub fn sync(&self, p: &ReaderPrefs) {
        for (t, b) in self.toggles.borrow().iter() {
            if is_active(t, p) && !b.is_active() {
                b.set_active(true);
            }
        }
    }

    /// `name:active` per custom swatch, for the test hooks.
    pub fn dump(&self) -> String {
        self.toggles
            .borrow()
            .iter()
            .map(|(t, b)| format!("{}:{}", t.name, if b.is_active() { "on" } else { "off" }))
            .collect::<Vec<_>>()
            .join(",")
    }
}
