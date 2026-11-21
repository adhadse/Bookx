//! The selection handles: two blue teardrops at the ends of the selected text that a finger or the
//! mouse can drag to make the selection longer or shorter. They are drawn on a layer over the page
//! (a `GtkOverlay` child that takes no events); a drag gesture on the overlay picks a handle up when
//! the press is on one and leaves every other press to the page. The geometry is in
//! `selection_handles`; the canvas moves the selection (`handle_drag_*`).

use std::cell::RefCell;
use std::rc::Rc;

use relm4::gtk::{self, gdk, prelude::*};

use super::canvas::BookCanvas;
use crate::selection_handles::{self as geo, Dir, Handle, Kind};

struct Drag {
    /// The handle in hand is at the start of the selection.
    start: bool,
    /// Where the pointer is relative to the ball when it was picked up.
    grab: (f64, f64),
    line_h: f64,
    word: bool,
}

struct St {
    handles: Vec<Handle>,
    drag: Option<Drag>,
    /// The two ends of the picked-up selection, for the dev dump.
    moves: u32,
}

#[derive(Clone)]
pub struct Handles {
    layer: gtk::DrawingArea,
    canvas: BookCanvas,
    st: Rc<RefCell<St>>,
    /// The drag begins / ends: the reader puts the action popover away and brings it back.
    on_begin: Rc<dyn Fn()>,
    on_end: Rc<dyn Fn()>,
}

fn draw(cr: &gtk::cairo::Context, h: &Handle, eink: bool) {
    let (cx, cy) = h.ball;
    let r = geo::RADIUS;
    let dir = if h.kind == Kind::Start { 1.0 } else { -1.0 };
    let pi = std::f64::consts::PI;
    // a circle with a pointed corner towards the text: the tip of a teardrop
    let path = |cr: &gtk::cairo::Context| {
        cr.new_path();
        if h.kind == Kind::Start {
            cr.arc(cx, cy, r, 0.75 * pi, 2.25 * pi);
        } else {
            cr.arc(cx, cy, r, -0.25 * pi, 1.25 * pi);
        }
        cr.line_to(cx, cy + dir * geo::BALL_DIST);
        cr.close_path();
    };
    path(cr);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
    cr.set_line_width(3.5);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    let _ = cr.stroke_preserve();
    if eink {
        cr.set_source_rgb(0.0, 0.0, 0.0);
    } else {
        cr.set_source_rgb(0.21, 0.52, 0.89);
    }
    let _ = cr.fill();
}

impl Handles {
    /// Builds the layer and puts it over `overlay`'s child (the page).
    pub fn build(
        overlay: &gtk::Overlay,
        canvas: &BookCanvas,
        on_begin: impl Fn() + 'static,
        on_end: impl Fn() + 'static,
    ) -> Handles {
        let layer = gtk::DrawingArea::builder()
            .accessible_role(gtk::AccessibleRole::Presentation)
            .build();
        layer.set_can_target(false);
        layer.set_can_focus(false);
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        let st = Rc::new(RefCell::new(St {
            handles: Vec::new(),
            drag: None,
            moves: 0,
        }));
        {
            let st = st.clone();
            layer.set_draw_func(move |_, cr, _, _| {
                let eink = crate::eink::is_active();
                for h in &st.borrow().handles {
                    draw(cr, h, eink);
                }
            });
        }
        overlay.add_overlay(&layer);
        let this = Handles {
            layer,
            canvas: canvas.clone(),
            st,
            on_begin: Rc::new(on_begin),
            on_end: Rc::new(on_end),
        };
        // a drag that starts on a handle is ours, any other press belongs to the page
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let t = this.clone();
            drag.connect_drag_begin(move |g, x, y| {
                let touch = g
                    .current_event_device()
                    .is_some_and(|d| d.source() == gdk::InputSource::Touchscreen);
                let word = touch
                    || g.current_event_state()
                        .contains(gdk::ModifierType::CONTROL_MASK);
                if t.begin(x, y, word) {
                    g.set_state(gtk::EventSequenceState::Claimed);
                } else {
                    g.set_state(gtk::EventSequenceState::Denied);
                }
            });
            let t = this.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                if let Some((sx, sy)) = g.start_point() {
                    t.update(sx + dx, sy + dy);
                }
            });
            let t = this.clone();
            drag.connect_drag_end(move |_, _, _| t.end());
        }
        overlay.add_controller(drag);
        this
    }

    fn dir(&self, rtl: bool, vertical: bool) -> Dir {
        if vertical {
            Dir::Vertical
        } else if rtl {
            Dir::Rtl
        } else {
            Dir::Ltr
        }
    }

    /// Puts the handles where the selection is now (and takes them away when there is none).
    pub fn refresh(&self) {
        let w = self.layer.width() as f64;
        let h = self.layer.height() as f64;
        let mut hs = Vec::new();
        {
            if let Some(e) = self.canvas.selection_ends() {
                let dir = self.dir(e.rtl, e.vertical);
                if let Some(r) = e.start {
                    hs.push(geo::place(Kind::Start, r, dir, (w, h)));
                }
                if let Some(r) = e.end {
                    hs.push(geo::place(Kind::End, r, dir, (w, h)));
                }
            }
        }
        self.st.borrow_mut().handles = hs;
        self.layer.queue_draw();
    }

    /// The area the action popover should point at for a selection with this rectangle.
    pub fn popover_rect(&self, rect: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
        if self.st.borrow().handles.is_empty() {
            return rect;
        }
        geo::popover_rect(rect, (self.layer.width(), self.layer.height()))
    }

    pub fn begin(&self, x: f64, y: f64, word: bool) -> bool {
        let picked = {
            let st = self.st.borrow();
            geo::hit(&st.handles, (x, y)).map(|i| st.handles[i])
        };
        let Some(h) = picked else {
            return false;
        };
        let start = h.kind == Kind::Start;
        if !self.canvas.handle_drag_begin(start) {
            return false;
        }
        self.st.borrow_mut().drag = Some(Drag {
            start,
            grab: (x - h.ball.0, y - h.ball.1),
            line_h: h.line_h,
            word,
        });
        (self.on_begin)();
        true
    }

    pub fn update(&self, x: f64, y: f64) {
        let (start, grab, line_h, word) = {
            let st = self.st.borrow();
            let Some(d) = &st.drag else {
                return;
            };
            (d.start, d.grab, d.line_h, d.word)
        };
        let kind = if start { Kind::Start } else { Kind::End };
        let ball = (x - grab.0, y - grab.1);
        let p = geo::probe(kind, ball, line_h);
        let Some(now_start) = self.canvas.handle_drag_to(p.0, p.1, word) else {
            return;
        };
        self.refresh();
        let st = &mut *self.st.borrow_mut();
        st.moves += 1;
        if let Some(d) = st.drag.as_mut() {
            if d.start != now_start {
                // the handle crossed the other end: it is the other kind now, under the same finger
                d.start = now_start;
                let want = if now_start { Kind::Start } else { Kind::End };
                if let Some(h) = st.handles.iter().find(|h| h.kind == want) {
                    d.grab = (x - h.ball.0, y - h.ball.1);
                    d.line_h = h.line_h;
                }
            }
        }
    }

    /// Whether a handle is being dragged now.
    pub fn is_dragging(&self) -> bool {
        self.st.borrow().drag.is_some()
    }

    pub fn end(&self) {
        if self.st.borrow_mut().drag.take().is_none() {
            return;
        }
        self.canvas.handle_drag_end();
        self.refresh();
        (self.on_end)();
    }

    /// The handles for the dev dump / tests: `n=2 start=(x,y)/(x,y) end=(x,y)/(x,y)` (tip / ball).
    pub fn dev_dump(&self) -> String {
        let st = self.st.borrow();
        let f = |k: Kind| {
            st.handles
                .iter()
                .find(|h| h.kind == k)
                .map(|h| {
                    format!(
                        "({:.0},{:.0})/({:.0},{:.0})",
                        h.tip.0, h.tip.1, h.ball.0, h.ball.1
                    )
                })
                .unwrap_or_else(|| "none".into())
        };
        format!(
            "n={} start={} end={} dragging={} moves={}",
            st.handles.len(),
            f(Kind::Start),
            f(Kind::End),
            st.drag.is_some(),
            st.moves
        )
    }

    /// Dev hook: a drag of the handle `which` by (dx, dy), through the same code as a finger.
    pub fn dev_drag(&self, start: bool, dx: f64, dy: f64) -> bool {
        let want = if start { Kind::Start } else { Kind::End };
        let Some(h) = self
            .st
            .borrow()
            .handles
            .iter()
            .find(|h| h.kind == want)
            .copied()
        else {
            return false;
        };
        if !self.begin(h.ball.0, h.ball.1, false) {
            return false;
        }
        let steps = 4;
        for i in 1..=steps {
            let f = i as f64 / steps as f64;
            self.update(h.ball.0 + dx * f, h.ball.1 + dy * f);
        }
        self.end();
        true
    }
}
