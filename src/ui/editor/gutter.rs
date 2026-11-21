//! The gutter column of marks: a GtkSourceView `GutterRenderer` that draws what
//! `editor_marks::Marks` knows about each line: a change bar (green = new, blue = changed since
//! the last checkpoint, red = lines removed), an error/warning/info icon for findings of the book
//! check, and the number of misspelled words. Hovering a line shows the finding texts.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw;
use relm4::gtk::{self, gdk, glib, graphene, pango, prelude::*, subclass::prelude::*};
use sourceview5 as sv;
use sv::prelude::*;
use sv::subclass::prelude::*;

use crate::core::check::Severity;
use crate::editor_marks::{Change, Marks};

pub type SharedMarks = Rc<RefCell<Marks>>;

/// The colours of the gutter marks for a light or dark window, normal or high contrast, as (r, g, b) in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub added: (f32, f32, f32),
    pub modified: (f32, f32, f32),
    pub removed: (f32, f32, f32),
    pub error: (f32, f32, f32),
    pub warning: (f32, f32, f32),
    pub info: (f32, f32, f32),
    /// The window background the colours are meant to be drawn on.
    pub background: (f32, f32, f32),
}

fn hex(h: u32) -> (f32, f32, f32) {
    (
        ((h >> 16) & 0xff) as f32 / 255.0,
        ((h >> 8) & 0xff) as f32 / 255.0,
        (h & 0xff) as f32 / 255.0,
    )
}

pub fn palette(dark: bool, high_contrast: bool) -> Palette {
    match (dark, high_contrast) {
        (false, false) => Palette {
            added: hex(0x1a6b3f),
            modified: hex(0x1c60c0),
            removed: hex(0xc01c28),
            error: hex(0xc01c28),
            warning: hex(0x8a5300),
            info: hex(0x1c60c0),
            background: hex(0xfafafa),
        },
        (false, true) => Palette {
            added: hex(0x0b4d2a),
            modified: hex(0x0a3f8f),
            removed: hex(0x8f1420),
            error: hex(0x8f1420),
            warning: hex(0x5e3800),
            info: hex(0x0a3f8f),
            background: hex(0xffffff),
        },
        (true, false) => Palette {
            added: hex(0x8ff0a4),
            modified: hex(0x99c1f1),
            removed: hex(0xff7b63),
            error: hex(0xff7b63),
            warning: hex(0xf8e45c),
            info: hex(0x99c1f1),
            background: hex(0x242424),
        },
        (true, true) => Palette {
            added: hex(0xb8ffc8),
            modified: hex(0xcfe3ff),
            removed: hex(0xffc2b8),
            error: hex(0xffc2b8),
            warning: hex(0xffef8a),
            info: hex(0xcfe3ff),
            background: hex(0x000000),
        },
    }
}

#[cfg(test)]
fn luminance((r, g, b): (f32, f32, f32)) -> f64 {
    let f = |c: f32| {
        let c = c as f64;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// The WCAG contrast ratio of two colours.
#[cfg(test)]
pub fn contrast(a: (f32, f32, f32), b: (f32, f32, f32)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MarksRenderer {
        pub marks: RefCell<SharedMarks>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MarksRenderer {
        const NAME: &'static str = "BookxMarksRenderer";
        type Type = super::MarksRenderer;
        type ParentType = sv::GutterRenderer;
    }

    impl ObjectImpl for MarksRenderer {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_width_request(36);
        }
    }

    impl WidgetImpl for MarksRenderer {}

    impl GutterRendererImpl for MarksRenderer {
        fn snapshot_line(&self, snapshot: &gtk::Snapshot, lines: &sv::GutterLines, line: u32) {
            let marks = self.marks.borrow();
            let marks = marks.borrow();
            let Some(info) = marks.get(line) else { return };
            let (y, h) = lines.line_yrange(line, sv::GutterRendererAlignmentMode::Cell);
            let (y, h) = (y as f32, h as f32);
            let sm = adw::StyleManager::default();
            let pal = palette(sm.is_dark(), sm.is_high_contrast());
            let rgb = |c: (f32, f32, f32)| rgba(c.0, c.1, c.2);
            if let Some(c) = info.change {
                let (color, w, hh, yy) = match c {
                    Change::Added => (rgb(pal.added), 3.0, h, y),
                    Change::Modified => (rgb(pal.modified), 3.0, h, y),
                    Change::Removed => (rgb(pal.removed), 8.0, 2.0, y),
                };
                snapshot.append_color(&color, &graphene::Rect::new(0.0, yy, w, hh));
            }
            let obj = self.obj();
            if let Some(sev) = info.severity {
                let (icon, color) = match sev {
                    Severity::Error => ("dialog-error-symbolic", rgb(pal.error)),
                    Severity::Warning => ("dialog-warning-symbolic", rgb(pal.warning)),
                    Severity::Info => ("dialog-information-symbolic", rgb(pal.info)),
                };
                let size = 14.0f32;
                if let Some(display) = gdk::Display::default() {
                    let theme = gtk::IconTheme::for_display(&display);
                    let paintable = theme.lookup_icon(
                        icon,
                        &[],
                        size as i32,
                        obj.scale_factor(),
                        gtk::TextDirection::None,
                        gtk::IconLookupFlags::empty(),
                    );
                    snapshot.save();
                    snapshot.translate(&graphene::Point::new(6.0, y + ((h - size) / 2.0).max(0.0)));
                    paintable.snapshot_symbolic(
                        snapshot,
                        size as f64,
                        size as f64,
                        &[color, color, color, color],
                    );
                    snapshot.restore();
                }
            }
            if info.misspelled > 0 {
                let layout = obj.create_pango_layout(None);
                layout.set_markup(&format!("<small>{}</small>", info.misspelled.min(99)));
                layout.set_alignment(pango::Alignment::Right);
                layout.set_width(12 * pango::SCALE);
                snapshot.save();
                snapshot.translate(&graphene::Point::new(22.0, y));
                snapshot.append_layout(&layout, &rgb(pal.error));
                snapshot.restore();
            }
        }
    }
}

glib::wrapper! {
    pub struct MarksRenderer(ObjectSubclass<imp::MarksRenderer>)
        @extends sv::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn rgba(r: f32, g: f32, b: f32) -> gdk::RGBA {
    gdk::RGBA::new(r, g, b, 1.0)
}

impl MarksRenderer {
    pub fn new(marks: SharedMarks) -> MarksRenderer {
        let r: MarksRenderer = glib::Object::new();
        *r.imp().marks.borrow_mut() = marks;
        r
    }
}

/// Adds the marks column to the left gutter of `view` (left of the line numbers) and wires the
/// hover text.
pub fn install(view: &sv::View, marks: SharedMarks) -> MarksRenderer {
    let r = MarksRenderer::new(marks.clone());
    ViewExt::gutter(view, gtk::TextWindowType::Left).insert(&r, -30);
    r.set_has_tooltip(true);
    let v = view.clone();
    r.connect_query_tooltip(move |w, x, y, _kb, tip| {
        let Some(line) = line_at(&v, w.upcast_ref(), x, y) else {
            return false;
        };
        match marks.borrow().tooltip(line) {
            Some(t) => {
                tip.set_text(Some(&t));
                true
            }
            None => false,
        }
    });
    r
}

/// The 0-based buffer line shown at (`x`, `y`) of a gutter widget.
fn line_at(view: &sv::View, gutter: &gtk::Widget, x: i32, y: i32) -> Option<u32> {
    let p = gutter.compute_point(view, &graphene::Point::new(x as f32, y as f32))?;
    let (_, by) =
        view.window_to_buffer_coords(gtk::TextWindowType::Widget, p.x() as i32, p.y() as i32);
    let (iter, _) = view.line_at_y(by);
    Some(iter.line() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gutter_colours_keep_their_contrast_in_every_mode() {
        for dark in [false, true] {
            for hc in [false, true] {
                let p = palette(dark, hc);
                let need = if hc { 7.0 } else { 4.5 };
                for (name, c) in [
                    ("added", p.added),
                    ("modified", p.modified),
                    ("removed", p.removed),
                    ("error", p.error),
                    ("warning", p.warning),
                    ("info", p.info),
                ] {
                    let r = contrast(c, p.background);
                    assert!(r >= need, "{name} dark={dark} hc={hc}: {r:.2} < {need}");
                }
            }
        }
    }
}
