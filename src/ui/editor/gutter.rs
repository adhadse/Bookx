//! The gutter column of marks: a GtkSourceView `GutterRenderer` that draws what
//! `editor_marks::Marks` knows about each line: a change bar (green = new, blue = changed since
//! the last checkpoint, red = lines removed), an error/warning/info icon for findings of the book
//! check, and the number of misspelled words. Hovering a line shows the finding texts.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::gtk::{self, gdk, glib, graphene, pango, prelude::*, subclass::prelude::*};
use sourceview5 as sv;
use sv::prelude::*;
use sv::subclass::prelude::*;

use crate::core::check::Severity;
use crate::editor_marks::{Change, Marks};

pub type SharedMarks = Rc<RefCell<Marks>>;

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
            if let Some(c) = info.change {
                let (color, w, hh, yy) = match c {
                    Change::Added => (rgba(0.15, 0.64, 0.41), 3.0, h, y),
                    Change::Modified => (rgba(0.21, 0.52, 0.89), 3.0, h, y),
                    Change::Removed => (rgba(0.88, 0.11, 0.14), 8.0, 2.0, y),
                };
                snapshot.append_color(&color, &graphene::Rect::new(0.0, yy, w, hh));
            }
            let obj = self.obj();
            if let Some(sev) = info.severity {
                let (icon, color) = match sev {
                    Severity::Error => ("dialog-error-symbolic", rgba(0.88, 0.11, 0.14)),
                    Severity::Warning => ("dialog-warning-symbolic", rgba(0.82, 0.55, 0.0)),
                    Severity::Info => ("dialog-information-symbolic", rgba(0.21, 0.52, 0.89)),
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
                snapshot.append_layout(&layout, &rgba(0.88, 0.11, 0.14));
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
