//! A small circular progress indicator drawn with cairo: a faint track and an accent arc, the
//! percent (or an icon) in the middle. Without a known fraction the arc turns; with reduced motion
//! (GTK's animations setting off) or in e-ink mode it stays a fixed partial ring.

use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

use relm4::gtk::{self, prelude::*};

/// What the ring shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ring {
    /// 0..=1
    Fraction(f64),
    /// Running, but not knowing how far.
    Busy,
    /// A full ring (a finished job): drawn as a track only, the icon in the middle says the rest.
    Idle,
}

/// The widget and the means to change it.
#[derive(Clone)]
pub struct ProgressRing {
    pub widget: gtk::Overlay,
    area: gtk::DrawingArea,
    state: Rc<Cell<Ring>>,
    angle: Rc<Cell<f64>>,
    label: gtk::Label,
    icon: gtk::Image,
}

fn still(w: &gtk::Widget) -> bool {
    let anim = gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
    let eink = w
        .root()
        .and_then(|r| r.downcast::<gtk::Window>().ok())
        .is_some_and(|win| win.has_css_class("eink"));
    !anim || eink
}

impl ProgressRing {
    /// A ring of `size` pixels (the job rows use 36, the header button 18).
    pub fn new(size: i32) -> ProgressRing {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        let state = Rc::new(Cell::new(Ring::Idle));
        let angle = Rc::new(Cell::new(0.0));
        {
            let (state, angle) = (state.clone(), angle.clone());
            area.set_draw_func(move |a, cr, w, h| {
                let (w, h) = (w as f64, h as f64);
                let line = (w.min(h) / 9.0).max(2.0);
                let r = (w.min(h) - line) / 2.0;
                let (cx, cy) = (w / 2.0, h / 2.0);
                let fg = a.color();
                cr.set_line_width(line);
                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.18);
                cr.arc(cx, cy, r, 0.0, 2.0 * PI);
                let _ = cr.stroke();
                #[allow(deprecated)]
                let accent = a
                    .style_context()
                    .lookup_color("accent_bg_color")
                    .unwrap_or(gtk::gdk::RGBA::new(0.21, 0.52, 0.89, 1.0));
                cr.set_source_rgba(
                    accent.red() as f64,
                    accent.green() as f64,
                    accent.blue() as f64,
                    1.0,
                );
                cr.set_line_cap(gtk::cairo::LineCap::Round);
                let top = -PI / 2.0;
                match state.get() {
                    Ring::Fraction(f) => {
                        let f = f.clamp(0.0, 1.0);
                        if f > 0.0 {
                            cr.arc(cx, cy, r, top, top + 2.0 * PI * f.max(0.02));
                            let _ = cr.stroke();
                        }
                    }
                    Ring::Busy => {
                        let start = top + angle.get();
                        cr.arc(cx, cy, r, start, start + PI * 0.55);
                        let _ = cr.stroke();
                    }
                    Ring::Idle => {}
                }
            });
        }
        // the arc turns while the ring is busy (only then is a frame requested)
        {
            let (state, angle) = (state.clone(), angle.clone());
            area.add_tick_callback(move |a, clock| {
                if state.get() == Ring::Busy && a.is_mapped() && !still(a.upcast_ref()) {
                    let t = clock.frame_time() as f64 / 1_000_000.0;
                    angle.set((t * 2.0 * PI / 1.1) % (2.0 * PI));
                    a.queue_draw();
                }
                gtk::glib::ControlFlow::Continue
            });
        }
        let label = gtk::Label::new(None);
        label.add_css_class("numeric");
        label.add_css_class("caption");
        label.set_visible(false);
        let icon = gtk::Image::new();
        icon.set_pixel_size((size / 2).max(8));
        icon.set_visible(false);
        let overlay = gtk::Overlay::builder()
            .accessible_role(gtk::AccessibleRole::ProgressBar)
            .build();
        overlay.update_property(&[
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(100.0),
        ]);
        overlay.set_child(Some(&area));
        overlay.add_overlay(&label);
        overlay.add_overlay(&icon);
        overlay.set_halign(gtk::Align::Center);
        overlay.set_valign(gtk::Align::Center);
        overlay.set_size_request(size, size);
        for w in [label.upcast_ref::<gtk::Widget>(), icon.upcast_ref()] {
            w.set_halign(gtk::Align::Center);
            w.set_valign(gtk::Align::Center);
            w.set_can_target(false);
        }
        ProgressRing {
            widget: overlay,
            area,
            state,
            angle,
            label,
            icon,
        }
    }

    /// Sets the ring and what is in the middle: the percent when the fraction is known and the ring
    /// is big enough, else `icon` (an icon name, may be empty).
    pub fn set(&self, ring: Ring, icon: &str) {
        self.state.set(ring);
        if ring == Ring::Busy && still(self.area.upcast_ref()) {
            // a fixed partial ring: a quarter turn from the top
            self.angle.set(0.0);
        }
        let big = self.area.content_width() >= 28;
        match ring {
            Ring::Fraction(f) if big => {
                self.label
                    .set_label(&format!("{:.0}", f.clamp(0.0, 1.0) * 100.0));
                self.label.set_visible(true);
                self.icon.set_visible(false);
            }
            _ => {
                self.label.set_visible(false);
                self.icon.set_visible(!icon.is_empty() && big);
                if !icon.is_empty() {
                    self.icon.set_icon_name(Some(icon));
                }
            }
        }
        if let Ring::Fraction(f) = ring {
            self.widget
                .update_property(&[gtk::accessible::Property::ValueNow(
                    f.clamp(0.0, 1.0) * 100.0,
                )]);
        }
        self.area.queue_draw();
    }

    /// What the ring shows now (for the tests).
    pub fn describe(&self) -> String {
        match self.state.get() {
            Ring::Fraction(f) => format!("fraction {:.0}%", f * 100.0),
            Ring::Busy => format!(
                "busy {}",
                if still(self.area.upcast_ref()) {
                    "static"
                } else {
                    "turning"
                }
            ),
            Ring::Idle => "idle".into(),
        }
    }
}
