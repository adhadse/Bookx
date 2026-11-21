//! A QR code of a server address, so a phone or e-reader can open it by scanning instead of typing.
//! The symbol comes from `bookx_serve::qr` (pure, tested); this file only draws it (black modules on
//! a white square with the four-module quiet zone the standard asks for, whatever the theme).

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use crate::i18n::{tr, trf};
use bookx_serve::qr::{Ecc, QrCode};

thread_local! {
    /// What the last dialog showed (the dev hook reads it).
    static LAST: RefCell<Option<(String, usize, usize, usize)>> = const { RefCell::new(None) };
}

/// The longest address Bookx puts in a code (a code of this many bytes is still easy to scan).
const MAX_TEXT: usize = 300;

/// The symbol for `text`, or `None` for an empty or very long text.
pub fn code_for(text: &str) -> Option<QrCode> {
    if text.is_empty() || text.len() > MAX_TEXT {
        return None;
    }
    QrCode::encode_text(text, Ecc::Medium)
}

/// Draws `code` into a `w` by `h` area: centred, an integer number of pixels per module.
pub fn paint(cr: &gtk::cairo::Context, code: &QrCode, w: f64, h: f64) {
    let quiet = 4usize;
    let cells = code.size() + 2 * quiet;
    let side = w.min(h);
    let unit = (side / cells as f64).floor().max(1.0);
    let total = unit * cells as f64;
    let (ox, oy) = ((w - total) / 2.0, (h - total) / 2.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.rectangle(ox, oy, total, total);
    let _ = cr.fill();
    cr.set_source_rgb(0.0, 0.0, 0.0);
    for y in 0..code.size() {
        for x in 0..code.size() {
            if code.dark(x, y) {
                cr.rectangle(
                    ox + (x + quiet) as f64 * unit,
                    oy + (y + quiet) as f64 * unit,
                    unit,
                    unit,
                );
            }
        }
    }
    let _ = cr.fill();
}

/// Opens the dialog for `text` over `parent`. Nothing happens when the text cannot be encoded.
pub fn show(parent: &gtk::Widget, heading: &str, text: &str) {
    let Some(code) = code_for(text) else {
        return;
    };
    let code = Rc::new(code);
    LAST.with(|l| {
        let dark = (0..code.size())
            .flat_map(|y| (0..code.size()).map(move |x| (x, y)))
            .filter(|&(x, y)| code.dark(x, y))
            .count();
        *l.borrow_mut() = Some((text.to_string(), code.version(), code.size(), dark));
    });
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("QR code"));
    dialog.set_content_width(360);
    let area = gtk::DrawingArea::new();
    area.set_content_width(280);
    area.set_content_height(280);
    area.set_halign(gtk::Align::Center);
    area.set_accessible_role(gtk::AccessibleRole::Img);
    area.update_property(&[gtk::accessible::Property::Label(&trf(
        "QR code of the address {}",
        &[&text],
    ))]);
    {
        let code = code.clone();
        area.set_draw_func(move |_, cr, w, h| paint(cr, &code, w as f64, h as f64));
    }
    let title = gtk::Label::new(Some(heading));
    title.set_wrap(true);
    title.add_css_class("heading");
    let addr = gtk::Label::new(Some(text));
    addr.set_selectable(true);
    addr.set_wrap(true);
    addr.set_wrap_mode(gtk::pango::WrapMode::Char);
    addr.add_css_class("monospace");
    let hint = gtk::Label::new(Some(&tr(
        "Point the camera of your phone or the scanner of your reader app at the code.",
    )));
    hint.set_wrap(true);
    hint.add_css_class("dim-label");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    for w in [
        title.upcast_ref::<gtk::Widget>(),
        area.upcast_ref(),
        addr.upcast_ref(),
        hint.upcast_ref(),
    ] {
        b.append(w);
    }
    b.set_margin_top(12);
    b.set_margin_bottom(24);
    b.set_margin_start(24);
    b.set_margin_end(24);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&b));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// A flat button that shows the QR code of what `text` returns (nothing when it returns `None`).
pub fn button(heading: String, text: impl Fn() -> Option<String> + 'static) -> gtk::Button {
    let b = gtk::Button::with_label(&tr("QR code"));
    b.set_valign(gtk::Align::Center);
    b.add_css_class("flat");
    b.set_tooltip_text(Some(&tr("Show the address as a QR code")));
    b.connect_clicked(move |b| {
        if let Some(t) = text() {
            show(b.upcast_ref(), &heading, &t);
        }
    });
    b
}

/// Dev hook: `prefs:sync-qr=ADDRESS` opens the dialog over `parent`; `prefs:sync-qr-dump` prints
/// `qr: text=.. version=N size=N dark=N` for the last one.
pub fn dev(arg: &str, parent: Option<gtk::Widget>) {
    if let Some(t) = arg.strip_prefix('=') {
        match parent {
            Some(p) => show(&p, &tr("Address"), t),
            None => eprintln!("qr: no page"),
        }
        return;
    }
    LAST.with(|l| match l.borrow().as_ref() {
        Some((t, v, s, d)) => eprintln!("qr: text={t} version={v} size={s} dark={d}"),
        None => eprintln!("qr: none"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_get_a_code_and_nonsense_does_not() {
        let c = code_for("http://192.168.1.20:8080/bookx").unwrap();
        assert_eq!(c.size(), c.version() * 4 + 17);
        assert!(code_for("").is_none());
        assert!(code_for(&"x".repeat(MAX_TEXT + 1)).is_none());
        assert!(code_for(&"x".repeat(MAX_TEXT)).is_some());
    }
}
