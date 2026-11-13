//! Moving the floating cards (read aloud, speed reading) over the reader page.
//!
//! Two things make this smooth. The position of a card is a fraction of the free room in the
//! overlay, turned into a rectangle by the overlay's `get-child-position` signal, so a move
//! re-allocates only that card (the page underneath is not measured, laid out or redrawn). And the
//! drag is worked out in the *overlay's* coordinates: the pointer is translated from the card to the
//! overlay at every motion event and the grab point (where on the card the user pressed) is
//! subtracted. GTK's drag offset cannot be used as it is: it is measured in the coordinates of the
//! widget that moves, so every move changes the next offset and the card lags half-way behind the
//! pointer. The arithmetic is in [`crate::tts_player`] (unit-tested).
//!
//! The stored place of a card is the user's *wish*. The final rectangles come from
//! [`crate::card_layout::layout`] (cards never overlap each other or a visible popover) and are worked
//! out again at every allocation; a move the layout manager causes slides in about 200 ms (instant in
//! e-ink mode or with animations off), the user's own drag is never animated.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::gtk::{self, glib, graphene, prelude::*};

use crate::card_layout::{self, CardIn, Rect};
use crate::tts_player::{card_corner, drag_frac};

/// How long a move caused by the layout manager takes.
pub const SLIDE_MS: u32 = 200;

struct Card {
    id: u8,
    widget: gtk::Widget,
    pos: Pos,
    default: (f64, f64),
    /// Where it is drawn now (moves towards `target` while it slides).
    shown: Option<Rect>,
    target: Option<Rect>,
    /// A slide in progress: where it began and when.
    slide: Option<(Rect, std::time::Instant)>,
}

#[derive(Default)]
struct Mgr {
    cards: Vec<Card>,
    pops: Vec<glib::WeakRef<gtk::Popover>>,
    /// The card under the user's pointer right now.
    dragging: Option<u8>,
}

thread_local! {
    static SLIDE_TIMER: Cell<bool> = const { Cell::new(false) };
    static MGR: RefCell<Mgr> = RefCell::new(Mgr::default());
}

/// The final rectangle of each open card, `(id, x, y, w, h)`, as drawn (dev hooks and tests).
pub fn final_rects() -> Vec<(u8, Rect)> {
    MGR.with(|m| {
        m.borrow()
            .cards
            .iter()
            .filter(|c| c.widget.is_visible())
            .filter_map(|c| c.shown.map(|r| (c.id, r)))
            .collect()
    })
}

/// Where the layout manager wants the cards (the end of any slide in progress).
pub fn target_rects() -> Vec<(u8, Rect)> {
    MGR.with(|m| {
        m.borrow()
            .cards
            .iter()
            .filter(|c| c.widget.is_visible())
            .filter_map(|c| c.target.map(|r| (c.id, r)))
            .collect()
    })
}

/// The popovers that cards keep clear of while they are visible; places the cards again when they
/// show or hide.
pub fn avoid(pop: &gtk::Popover) {
    MGR.with(|m| m.borrow_mut().pops.push(pop.downgrade()));
    let relayout = |p: &gtk::Popover| {
        let p = p.clone();
        // after GTK has put the popover in place
        glib::idle_add_local_once(move || relayout_from(p.upcast_ref()));
    };
    pop.connect_show(move |p| relayout(p));
    pop.connect_closed(|p| relayout_from(p.upcast_ref()));
    pop.connect_map(|p| relayout_from(p.upcast_ref()));
    pop.connect_unmap(|p| relayout_from(p.upcast_ref()));
}

fn relayout_from(_w: &gtk::Widget) {
    let ov = MGR.with(|m| m.borrow().cards.first().and_then(|c| c.widget.parent()));
    if let Some(ov) = ov {
        ov.queue_allocate();
    }
}

/// The area a visible popover covers, in the overlay's coordinates, as far as it can be known: its
/// size and the side it opens on (flipped when it would leave the overlay, as GTK does).
fn popover_rect(pop: &gtk::Popover, ov: &gtk::Overlay) -> Option<Rect> {
    if !pop.is_visible() || !pop.is_mapped() {
        return None;
    }
    let parent = pop.parent()?;
    let b = parent.compute_bounds(ov)?;
    let (px, py, pw, ph) = match pop.pointing_to() {
        (true, r) => (
            b.x() as f64 + r.x() as f64,
            b.y() as f64 + r.y() as f64,
            r.width() as f64,
            r.height() as f64,
        ),
        _ => (
            b.x() as f64,
            b.y() as f64,
            b.width() as f64,
            b.height() as f64,
        ),
    };
    let child = pop.child()?;
    let w = child.measure(gtk::Orientation::Horizontal, -1).1 as f64 + 16.0;
    let h = child.measure(gtk::Orientation::Vertical, w as i32 - 16).1 as f64 + 16.0;
    let (ow, oh) = (ov.width() as f64, ov.height() as f64);
    let mut pos = pop.position();
    let fits = |p: gtk::PositionType| match p {
        gtk::PositionType::Top => py - h >= 0.0,
        gtk::PositionType::Bottom => py + ph + h <= oh,
        gtk::PositionType::Left => px - w >= 0.0,
        _ => px + pw + w <= ow,
    };
    if !fits(pos) {
        pos = match pos {
            gtk::PositionType::Top => gtk::PositionType::Bottom,
            gtk::PositionType::Bottom => gtk::PositionType::Top,
            gtk::PositionType::Left => gtk::PositionType::Right,
            _ => gtk::PositionType::Left,
        };
    }
    let cx = px + pw / 2.0 - w / 2.0;
    let cy = py + ph / 2.0 - h / 2.0;
    let (x, y) = match pos {
        gtk::PositionType::Top => (cx, py - h),
        gtk::PositionType::Bottom => (cx, py + ph),
        gtk::PositionType::Left => (px - w, cy),
        _ => (px + pw, cy),
    };
    // a popover is moved along its side to stay in the window
    let x = x.clamp(0.0, (ow - w).max(0.0));
    let y = y.clamp(0.0, (oh - h).max(0.0));
    Some(Rect::new(x, y, w, h))
}

/// Works out the final rectangles of the open cards and starts the slides.
fn sync(ov: &gtk::Overlay) {
    let area = (ov.width() as f64, ov.height() as f64);
    if area.0 < 1.0 || area.1 < 1.0 {
        return;
    }
    let obstacles: Vec<Rect> = MGR.with(|m| {
        m.borrow()
            .pops
            .iter()
            .filter_map(|w| w.upgrade())
            .filter_map(|p| popover_rect(&p, ov))
            .collect()
    });
    let instant =
        crate::eink::is_active() || !ov.settings().is_gtk_enable_animations() || !ov.is_mapped();
    let top = top();
    let mut sliding_now = false;
    MGR.with(|m| {
        let mut m = m.borrow_mut();
        let dragging = m.dragging;
        let open: Vec<usize> = (0..m.cards.len())
            .filter(|&i| m.cards[i].widget.is_visible())
            .collect();
        let ins: Vec<CardIn> = open
            .iter()
            .map(|&i| {
                let c = &m.cards[i];
                let pos = c.pos.get();
                CardIn {
                    id: c.id,
                    size: card_size(&c.widget, area.0),
                    wish: (pos != c.default).then_some(pos),
                    default_frac: c.default,
                    rank: if dragging == Some(c.id) {
                        0
                    } else if top == c.id {
                        1
                    } else {
                        2
                    },
                }
            })
            .collect();
        let out = card_layout::layout(area, &ins, &obstacles);
        for (&i, (_, r)) in open.iter().zip(out) {
            let c = &mut m.cards[i];
            let r = Rect::new(r.x.round(), r.y.round(), r.w.round(), r.h.round());
            if c.target == Some(r) {
                continue;
            }
            c.target = Some(r);
            let user = dragging == Some(c.id);
            match c.shown {
                Some(from) if !instant && !user && from != r => {
                    c.slide = Some((from, std::time::Instant::now()));
                    sliding_now = true;
                }
                _ => {
                    c.slide = None;
                    c.shown = Some(r);
                }
            }
        }
        for c in m.cards.iter_mut().filter(|c| !c.widget.is_visible()) {
            c.shown = None;
            c.target = None;
            c.slide = None;
        }
    });
    if sliding_now && !SLIDE_TIMER.with(|t| t.replace(true)) {
        // a timer rather than the frame clock: it moves 60 times a second whatever draws
        let weak = ov.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let more = MGR.with(|m| {
                let mut m = m.borrow_mut();
                let mut more = false;
                for c in m.cards.iter_mut() {
                    let (Some((from, t0)), Some(to)) = (c.slide, c.target) else {
                        continue;
                    };
                    let t = (t0.elapsed().as_millis() as f64 / SLIDE_MS as f64).min(1.0);
                    if t >= 1.0 {
                        c.shown = Some(to);
                        c.slide = None;
                    } else {
                        let e = 1.0 - (1.0 - t).powi(3);
                        let lerp = |a: f64, b: f64| a + (b - a) * e;
                        c.shown = Some(Rect::new(
                            lerp(from.x, to.x),
                            lerp(from.y, to.y),
                            lerp(from.w, to.w),
                            lerp(from.h, to.h),
                        ));
                        more = true;
                    }
                }
                more
            });
            let alive = weak.upgrade();
            if let Some(ov) = &alive {
                ov.queue_allocate();
            }
            if more && alive.is_some() {
                glib::ControlFlow::Continue
            } else {
                SLIDE_TIMER.with(|t| t.set(false));
                glib::ControlFlow::Break
            }
        });
    }
}

/// One line for the dev hook `cards-rects`: the overlay, the cards as drawn and as targeted, the popovers.
pub fn dump() -> String {
    let ov = MGR.with(|m| m.borrow().cards.first().and_then(|c| c.widget.parent()));
    let Some(ov) = ov.and_downcast::<gtk::Overlay>() else {
        return "no overlay".into();
    };
    // the hook can run between two frames: work out the places now (the same call as at an allocation)
    sync(&ov);
    let list = |v: Vec<(u8, Rect)>| {
        v.iter()
            .map(|(id, r)| format!("{id}:{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.w, r.h))
            .collect::<Vec<_>>()
            .join(";")
    };
    let pops: Vec<(u8, Rect)> = MGR
        .with(|m| {
            m.borrow()
                .pops
                .iter()
                .filter_map(|w| w.upgrade())
                .collect::<Vec<_>>()
        })
        .iter()
        .filter_map(|p| popover_rect(p, &ov))
        .enumerate()
        .map(|(i, r)| (i as u8, r))
        .collect();
    format!(
        "area={}x{} sliding={} shown=[{}] target=[{}] pops=[{}]",
        ov.width(),
        ov.height(),
        u8::from(sliding()),
        list(final_rects()),
        list(target_rects()),
        list(pops)
    )
}

/// Have the cards reached their places (the places are worked out first, as at an allocation)?
pub fn settled() -> bool {
    let ov = MGR.with(|m| m.borrow().cards.first().and_then(|c| c.widget.parent()));
    if let Some(ov) = ov.and_downcast::<gtk::Overlay>() {
        sync(&ov);
    }
    !sliding()
}

/// Is a slide in progress (tests wait for the end)?
pub fn sliding() -> bool {
    MGR.with(|m| {
        m.borrow()
            .cards
            .iter()
            .any(|c| c.shown != c.target && c.widget.is_visible())
    })
}

/// A card's place: fractions (0..=1) of the room inside the overlay.
pub type Pos = Rc<Cell<(f64, f64)>>;

thread_local! {
    /// Which card was dragged last (0 = read aloud, 1 = speed reading): it is drawn on top.
    static TOP: Cell<u8> = const { Cell::new(0) };
}

/// The card that is on top.
pub fn top() -> u8 {
    TOP.with(Cell::get)
}

/// Draws `widget` above the other cards of its overlay and remembers it as the top one.
pub fn raise(widget: &gtk::Widget, id: u8) {
    TOP.with(|t| t.set(id));
    if let Some(ov) = widget.parent().and_downcast::<gtk::Overlay>() {
        // the last overlay child is drawn last
        let w = widget.clone();
        ov.remove_overlay(&w);
        ov.add_overlay(&w);
    }
}

/// Makes the overlay place `cards` (widget, wish, default place) with the layout manager.
pub fn install(overlay: &gtk::Overlay, cards: Vec<(gtk::Widget, Pos, (f64, f64))>) {
    MGR.with(|m| {
        let mut m = m.borrow_mut();
        m.cards = cards
            .into_iter()
            .enumerate()
            .map(|(i, (widget, pos, default))| Card {
                id: i as u8,
                widget,
                pos,
                default,
                shown: None,
                target: None,
                slide: None,
            })
            .collect();
    });
    overlay.connect_get_child_position(move |ov, child| {
        sync(ov);
        MGR.with(|m| {
            let m = m.borrow();
            let r = m.cards.iter().find(|c| &c.widget == child)?.shown?;
            Some(gtk::Allocation::new(
                r.x.round() as i32,
                r.y.round() as i32,
                r.w.round() as i32,
                r.h.round() as i32,
            ))
        })
    });
}

/// The size the overlay gives the card: its natural size, no wider than the window. The drag uses
/// the same number, so that a place means the same rectangle when it is made and when it is shown.
fn card_size(child: &impl IsA<gtk::Widget>, area_w: f64) -> (f64, f64) {
    let w = child
        .measure(gtk::Orientation::Horizontal, -1)
        .1
        .min(area_w as i32)
        .max(1);
    let h = child.measure(gtk::Orientation::Vertical, w).1.max(1);
    (w as f64, h as f64)
}

/// Puts the card at `frac`; only the card is allocated again.
pub fn set_pos(widget: &gtk::Widget, pos: &Pos, frac: (f64, f64)) {
    let f = |v: f64| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.5
        }
    };
    pos.set((f(frac.0), f(frac.1)));
    if let Some(p) = widget.parent() {
        p.queue_allocate();
    }
}

/// Lets the user drag `widget` (the overlay child) by any part of `surface` that is not a control
/// of its own; a double click puts it back at `default`.
pub fn make_draggable(
    surface: &gtk::Widget,
    widget: &gtk::Widget,
    pos: Pos,
    default: (f64, f64),
    id: u8,
    on_end: Rc<dyn Fn()>,
) {
    let drag = gtk::GestureDrag::new();
    let grab = Rc::new(Cell::new((0.0, 0.0)));
    {
        let (surface, widget, grab) = (surface.clone(), widget.clone(), grab.clone());
        drag.connect_drag_begin(move |_, x, y| {
            // where on the moving widget the user took hold, in the widget's own coordinates
            let g = surface
                .compute_point(&widget, &graphene::Point::new(x as f32, y as f32))
                .map_or((x, y), |p| (p.x() as f64, p.y() as f64));
            grab.set(g);
        });
    }
    {
        let (surface, widget, pos2, grab) =
            (surface.clone(), widget.clone(), pos.clone(), grab.clone());
        drag.connect_drag_update(move |g, dx, dy| {
            let (Some((sx, sy)), Some(ov)) = (g.start_point(), widget.parent()) else {
                return;
            };
            // the pointer now, in the card's coordinates as they are now, then in the overlay's
            let Some(p) = surface.compute_point(
                &ov,
                &graphene::Point::new((sx + dx) as f32, (sy + dy) as f32),
            ) else {
                return;
            };
            let grab = grab.get();
            // the grab point is in `widget` coordinates; the overlay position of the widget is
            // the pointer minus that
            let top_left = card_corner((p.x() as f64, p.y() as f64), grab);
            let area = (ov.width() as f64, ov.height() as f64);
            let size = card_size(&widget, area.0);
            let frac = drag_frac(top_left, area, size);
            MGR.with(|m| m.borrow_mut().dragging = Some(id));
            if frac != pos2.get() {
                pos2.set(frac);
                ov.queue_allocate();
            }
        });
    }
    {
        let (widget, on_end) = (widget.clone(), on_end.clone());
        drag.connect_drag_end(move |_, dx, dy| {
            MGR.with(|m| m.borrow_mut().dragging = None);
            if let Some(ov) = widget.parent() {
                ov.queue_allocate();
            }
            if dx != 0.0 || dy != 0.0 {
                // the one that was moved last is on top (raised after the gesture, not during it)
                raise(&widget, id);
            }
            on_end();
        });
    }
    surface.add_controller(drag);
    let click = gtk::GestureClick::new();
    {
        let (widget, pos) = (widget.clone(), pos);
        click.connect_pressed(move |g, n, _, _| {
            if n == 2 {
                set_pos(&widget, &pos, default);
                on_end();
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        });
    }
    surface.add_controller(click);
}
