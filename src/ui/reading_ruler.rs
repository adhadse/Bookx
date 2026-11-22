//! The reading ruler overlay of the reader (`crate::reading_ruler`): the page is dimmed except for
//! a band that follows the pointer, or moves line by line with Alt+Up / Alt+Down. Menu → Reading
//! Ruler (or the command palette) turns it on; Preferences → Reading sets lines and dimming.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;
use super::canvas::BookCanvas;
use super::preferences::{PrefCtx, register_group};
use crate::i18n::tr;
use crate::reading_ruler as rr;
use crate::settings_bus::Bus;

thread_local! {
    static BUS: Rc<Bus> = Rc::new(Bus::default());
    static SWITCH: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
    static ROWS: RefCell<Option<(adw::SpinRow, adw::SpinRow)>> = const { RefCell::new(None) };
}

fn bus() -> Rc<Bus> {
    BUS.with(|b| b.clone())
}

/// Calls `f` when the ruler is turned on or off or its settings change (the menu's check item).
pub fn subscribe(f: impl Fn() + 'static) -> crate::settings_bus::Subscription {
    bus().subscribe(f)
}

/// Whether the ruler is on.
pub fn is_on(shared: &SharedRef) -> bool {
    shared.cfg.borrow().ruler.enabled
}

pub fn register() {
    register_group("reading", 70, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Reading ruler"));
    g.set_description(Some(&tr(
        "Dims the page except for a band that follows the pointer, or moves with Alt+Up and Alt+Down. It helps to keep your place.",
    )));
    let p = ctx.shared.cfg.borrow().ruler.clamped();
    let on = adw::SwitchRow::new();
    on.set_title(&tr("Show the reading ruler"));
    on.set_active(p.enabled);
    let lines = adw::SpinRow::with_range(rr::MIN_LINES as f64, rr::MAX_LINES as f64, 1.0);
    lines.set_title(&tr("Lines in the band"));
    lines.set_subtitle(&tr("Lines"));
    lines.set_value(p.lines as f64);
    let dim = adw::SpinRow::with_range(rr::MIN_DIM as f64, rr::MAX_DIM as f64, 5.0);
    dim.set_title(&tr("Dimming"));
    dim.set_subtitle(&tr("Percent"));
    dim.set_value(p.dim as f64);
    let save = {
        let (shared, on, lines, dim) = (ctx.shared.clone(), on.clone(), lines.clone(), dim.clone());
        Rc::new(move || {
            let mut c = shared.cfg.borrow_mut();
            c.ruler = rr::RulerPrefs {
                enabled: on.is_active(),
                lines: lines.value() as u8,
                dim: dim.value() as u8,
            }
            .clamped();
            c.save();
            drop(c);
            bus().notify();
        })
    };
    {
        let s = save.clone();
        on.connect_active_notify(move |_| s());
        let s = save.clone();
        lines.connect_value_notify(move |_| s());
        let s = save.clone();
        dim.connect_value_notify(move |_| s());
    }
    // the band settings do nothing while the ruler is off
    for r in [&lines, &dim] {
        on.bind_property("active", r, "sensitive")
            .sync_create()
            .build();
    }
    g.add(&on);
    g.add(&lines);
    g.add(&dim);
    ROWS.with(|r| *r.borrow_mut() = Some((lines.clone(), dim.clone())));
    SWITCH.with(|s| *s.borrow_mut() = Some(on));
    g
}

/// The ruler of one open book.
pub struct Ruler {
    area: gtk::DrawingArea,
    y: Cell<f64>,
    shared: SharedRef,
    canvas: BookCanvas,
    _sub: RefCell<Option<crate::settings_bus::Subscription>>,
}

impl Ruler {
    /// Puts the ruler over `overlay` (the page) and follows the pointer on it.
    pub fn install(overlay: &gtk::Overlay, canvas: &BookCanvas, shared: &SharedRef) -> Rc<Ruler> {
        let area = gtk::DrawingArea::new();
        area.set_can_target(false);
        area.set_focusable(false);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_accessible_role(gtk::AccessibleRole::Presentation);
        overlay.add_overlay(&area);
        let ruler = Rc::new(Ruler {
            area: area.clone(),
            y: Cell::new(-1.0),
            shared: shared.clone(),
            canvas: canvas.clone(),
            _sub: RefCell::new(None),
        });
        {
            let r = ruler.clone();
            area.set_draw_func(move |_, cr, w, h| r.draw(cr, w as f64, h as f64));
        }
        let motion = gtk::EventControllerMotion::new();
        motion.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let r = Rc::downgrade(&ruler);
            motion.connect_motion(move |_, _, y| {
                if let Some(r) = r.upgrade() {
                    r.y.set(y);
                    if r.prefs().enabled {
                        r.area.queue_draw();
                    }
                }
            });
        }
        overlay.add_controller(motion);
        // a finger has no pointer to follow: dragging the band moves it, and a tap puts it there
        // (the tap still turns the page: it is not claimed)
        let drag = gtk::GestureDrag::new();
        drag.set_touch_only(true);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let from = Rc::new(Cell::new(None::<f64>));
        {
            let (r, f1) = (Rc::downgrade(&ruler), from.clone());
            drag.connect_drag_begin(move |g, _, y| {
                if let Some(r) = r.upgrade() {
                    if r.grab(y) {
                        f1.set(Some(r.mid()));
                        g.set_state(gtk::EventSequenceState::Claimed);
                    }
                }
            });
            let (r, f2) = (Rc::downgrade(&ruler), from.clone());
            drag.connect_drag_update(move |_, _, dy| {
                if let (Some(r), Some(f)) = (r.upgrade(), f2.get()) {
                    r.place(rr::drag_to(f, dy, r.area.height() as f64));
                }
            });
            drag.connect_drag_end(move |_, _, _| from.set(None));
        }
        overlay.add_controller(drag);
        let tap = gtk::GestureClick::new();
        tap.set_touch_only(true);
        tap.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let r = Rc::downgrade(&ruler);
            tap.connect_pressed(move |g, _, _, y| {
                g.set_state(gtk::EventSequenceState::None);
                if let Some(r) = r.upgrade() {
                    if r.prefs().enabled {
                        r.place(y);
                    }
                }
            });
        }
        overlay.add_controller(tap);
        let weak = Rc::downgrade(&ruler);
        let sub = bus().subscribe(move || {
            if let Some(r) = weak.upgrade() {
                r.refresh();
            }
        });
        *ruler._sub.borrow_mut() = Some(sub);
        ruler.refresh();
        ruler
    }

    fn prefs(&self) -> rr::RulerPrefs {
        self.shared.cfg.borrow().ruler.clamped()
    }

    /// The middle of the band now.
    fn mid(&self) -> f64 {
        let h = self.area.height() as f64;
        if self.y.get() < 0.0 {
            h / 2.0
        } else {
            self.y.get()
        }
    }

    /// Puts the middle of the band at `y` (a finger, a tap).
    pub fn place(&self, y: f64) {
        self.y.set(y.clamp(0.0, self.area.height() as f64));
        self.area.queue_draw();
    }

    /// Whether a touch at `y` is on the band (the ruler must be on).
    fn grab(&self, y: f64) -> bool {
        let p = self.prefs();
        if !p.enabled {
            return false;
        }
        let h = self.area.height() as f64;
        let (top, bottom) = rr::band(self.mid(), h, self.line_h(), p.lines);
        rr::hit_band(y, top, bottom, self.line_h())
    }

    fn line_h(&self) -> f64 {
        let s = self.canvas.style();
        rr::line_height_px(s.font_size, s.line_spacing)
    }

    fn refresh(&self) {
        self.area.set_visible(self.prefs().enabled);
        self.area.queue_draw();
    }

    fn draw(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let p = self.prefs();
        if !p.enabled || h <= 0.0 {
            return;
        }
        let y = if self.y.get() < 0.0 {
            h / 2.0
        } else {
            self.y.get()
        };
        let (top, bottom) = rr::band(y, h, self.line_h(), p.lines);
        cr.set_source_rgba(0.0, 0.0, 0.0, p.alpha());
        cr.rectangle(0.0, 0.0, w, top);
        cr.rectangle(0.0, bottom, w, h - bottom);
        let _ = cr.fill();
    }

    /// Turns the ruler on or off (Reading Ruler in the menu and the command palette).
    pub fn toggle(&self) {
        {
            let mut c = self.shared.cfg.borrow_mut();
            c.ruler.enabled = !c.ruler.enabled;
            c.save();
        }
        // the switch of an open Preferences, and every other open book
        bus().notify();
    }

    /// Moves the band one line (`dir` -1 up, +1 down); turns the ruler on when it was off.
    pub fn step(&self, dir: i32) {
        if !self.prefs().enabled {
            self.toggle();
        }
        let h = self.area.height() as f64;
        let from = if self.y.get() < 0.0 {
            h / 2.0
        } else {
            self.y.get()
        };
        self.y.set(rr::step(from, dir, self.line_h(), h));
        self.area.queue_draw();
    }

    /// `reader:ruler=…` dev command: `toggle`, `up`, `down`, `state`.
    pub fn dev(&self, cmd: &str) -> String {
        match cmd {
            "toggle" => self.toggle(),
            "up" => self.step(-1),
            "down" => self.step(1),
            // `place:Y` is a tap, `drag:Y0:DY` a finger that grabs the band at Y0 and moves DY
            c if c.starts_with("place:") => self.place(c[6..].parse().unwrap_or(0.0)),
            c if c.starts_with("drag:") => {
                let mut it = c[5..].split(':');
                let y0: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                let dy: f64 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
                if self.grab(y0) {
                    self.place(rr::drag_to(self.mid(), dy, self.area.height() as f64));
                }
            }
            _ => {}
        }
        let p = self.prefs();
        let h = self.area.height() as f64;
        let y = if self.y.get() < 0.0 {
            h / 2.0
        } else {
            self.y.get()
        };
        let (top, bottom) = rr::band(y, h, self.line_h(), p.lines);
        format!(
            "enabled={} visible={} lines={} dim={} band={:.0}..{:.0} view_h={:.0} parent_h={} mapped={}",
            p.enabled,
            self.area.is_visible(),
            p.lines,
            p.dim,
            top,
            bottom,
            h,
            self.area.parent().map(|p| p.height()).unwrap_or(-1),
            self.area.is_mapped()
        )
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Ruler>>> = const { RefCell::new(None) };
}

/// The ruler of the book that was opened last (the reader's actions act on it).
pub fn set_current(r: Rc<Ruler>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(r));
}

/// `reader.ruler` (0: toggle), `reader.ruler-up` (-1) and `reader.ruler-down` (1).
pub fn act(dir: i32) {
    if let Some(r) = CURRENT.with(|c| c.borrow().clone()) {
        if dir == 0 {
            r.toggle();
        } else {
            r.step(dir);
        }
    }
}

/// `reader:ruler=toggle|up|down|state`.
pub fn dev_reader(cmd: &str) -> String {
    match CURRENT.with(|c| c.borrow().clone()) {
        Some(r) => r.dev(cmd),
        None => "no book".into(),
    }
}

/// Dev hook `prefs:ruler-on|off`: the switch of the open Preferences; `prefs:ruler-dump` prints
/// `prefs_ruler: on=B lines_sensitive=B dim_sensitive=B`.
pub fn dev_prefs(arg: &str) {
    if arg == "dump" {
        let on = SWITCH.with(|s| s.borrow().as_ref().is_some_and(|r| r.is_active()));
        let (lines, dim) = ROWS.with(|r| {
            r.borrow().as_ref().map_or((false, false), |(l, d)| {
                (l.is_sensitive(), d.is_sensitive())
            })
        });
        eprintln!("prefs_ruler: on={on} lines_sensitive={lines} dim_sensitive={dim}");
        return;
    }
    SWITCH.with(|s| {
        if let Some(r) = s.borrow().as_ref() {
            r.set_active(arg != "off");
        }
    });
}
