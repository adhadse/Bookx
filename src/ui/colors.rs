//! The colour controls shared by the reader's menu and the editor's preview: Theme / Original /
//! Custom, and the two pickers of the custom colours.

use crate::i18n::{tr, N_};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::gtk::{self, gdk, prelude::*};

use crate::config::{parse_hex, to_hex, ColorMode, ReaderPrefs};

pub struct ColorControls {
    pub root: gtk::Box,
    mode_btns: Vec<(ColorMode, gtk::ToggleButton)>,
    custom_row: gtk::Box,
    fg_btn: gtk::ColorDialogButton,
    bg_btn: gtk::ColorDialogButton,
    syncing: Rc<Cell<bool>>,
}

fn rgba_of(hex: &str, fallback: (f64, f64, f64)) -> gdk::RGBA {
    let c = parse_hex(hex).unwrap_or(fallback);
    gdk::RGBA::new(c.0 as f32, c.1 as f32, c.2 as f32, 1.0)
}

fn hex_of(c: &gdk::RGBA) -> String {
    to_hex((c.red() as f64, c.green() as f64, c.blue() as f64))
}

fn picker(tooltip: &str) -> gtk::ColorDialogButton {
    let dialog = gtk::ColorDialog::new();
    dialog.set_with_alpha(false);
    dialog.set_title(tooltip);
    let b = gtk::ColorDialogButton::new(Some(dialog));
    b.set_tooltip_text(Some(tooltip));
    b.set_valign(gtk::Align::Center);
    b
}

impl ColorControls {
    /// `cur` is the preferences copy the controls edit; `emit` is called after every change.
    pub fn new(cur: Rc<RefCell<ReaderPrefs>>, emit: Rc<dyn Fn()>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let modes = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        modes.add_css_class("linked");
        modes.set_halign(gtk::Align::Center);
        let syncing = Rc::new(Cell::new(false));
        let mut first: Option<gtk::ToggleButton> = None;
        let mut mode_btns = Vec::new();
        let initial = cur.borrow().colors;
        for (mode, label, tip) in [
            (ColorMode::Theme, "Theme", "Colours of the chosen theme"),
            (
                ColorMode::Original,
                "Original",
                "Show the colours the book itself asks for",
            ),
            (
                ColorMode::Custom,
                "Custom",
                "Pick your own text and page colours",
            ),
        ] {
            let b = gtk::ToggleButton::with_label(label);
            b.set_tooltip_text(Some(tip));
            b.set_active(mode == initial);
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            modes.append(&b);
            mode_btns.push((mode, b));
        }
        root.append(&modes);

        let custom_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        custom_row.set_halign(gtk::Align::Center);
        let fg_btn = picker("Text colour");
        let bg_btn = picker("Page colour");
        for (label, b) in [(N_("Text"), &fg_btn), (N_("Page"), &bg_btn)] {
            let l = gtk::Label::new(Some(&tr(label)));
            l.add_css_class("dim-label");
            custom_row.append(&l);
            custom_row.append(b);
        }
        custom_row.set_visible(initial == ColorMode::Custom);
        root.append(&custom_row);

        for (mode, b) in &mode_btns {
            let (cur, emit, row, syncing, mode) = (
                cur.clone(),
                emit.clone(),
                custom_row.clone(),
                syncing.clone(),
                *mode,
            );
            b.connect_toggled(move |b| {
                if syncing.get() || !b.is_active() || cur.borrow().colors == mode {
                    return;
                }
                cur.borrow_mut().colors = mode;
                row.set_visible(mode == ColorMode::Custom);
                emit();
            });
        }
        let custom_mode = mode_btns
            .iter()
            .find(|(m, _)| *m == ColorMode::Custom)
            .map(|(_, b)| b.clone());
        for (btn, is_fg) in [(&fg_btn, true), (&bg_btn, false)] {
            let (cur, emit, syncing, custom_mode) = (
                cur.clone(),
                emit.clone(),
                syncing.clone(),
                custom_mode.clone(),
            );
            btn.connect_rgba_notify(move |b| {
                if syncing.get() {
                    return;
                }
                let hex = hex_of(&b.rgba());
                {
                    let mut p = cur.borrow_mut();
                    let same = if is_fg {
                        p.custom_fg == hex
                    } else {
                        p.custom_bg == hex
                    };
                    if same && p.colors == ColorMode::Custom {
                        return;
                    }
                    if is_fg {
                        p.custom_fg = hex;
                    } else {
                        p.custom_bg = hex;
                    }
                    p.colors = ColorMode::Custom;
                }
                if let Some(m) = &custom_mode {
                    syncing.set(true);
                    m.set_active(true);
                    syncing.set(false);
                }
                emit();
            });
        }
        let c = Self {
            root,
            mode_btns,
            custom_row,
            fg_btn,
            bg_btn,
            syncing,
        };
        c.sync(&cur.borrow());
        c
    }

    /// Shows `p` without sending anything.
    pub fn sync(&self, p: &ReaderPrefs) {
        self.syncing.set(true);
        for (m, b) in &self.mode_btns {
            if *m == p.colors && !b.is_active() {
                b.set_active(true);
            }
        }
        self.custom_row.set_visible(p.colors == ColorMode::Custom);
        self.fg_btn
            .set_rgba(&rgba_of(&p.custom_fg, (0.17, 0.17, 0.17)));
        self.bg_btn
            .set_rgba(&rgba_of(&p.custom_bg, (0.98, 0.97, 0.93)));
        self.syncing.set(false);
    }
}
