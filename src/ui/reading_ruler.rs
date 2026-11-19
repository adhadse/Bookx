//! The reading ruler overlay of the reader (`crate::reading_ruler`): the page is dimmed except for
//! a band that follows the pointer, or moves line by line with Alt+Up / Alt+Down. Menu → Reading
//! Ruler (or the command palette) turns it on; Preferences → Reading sets lines and dimming.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::canvas::BookCanvas;
use super::preferences::{register_group, PrefCtx};
use super::SharedRef;
use crate::i18n::tr;
use crate::reading_ruler as rr;
use crate::settings_bus::Bus;

thread_local! {
    static BUS: Rc<Bus> = Rc::new(Bus::default());
    static SWITCH: RefCell<Option<adw::SwitchRow>> = const { RefCell::new(None) };
}

fn bus() -> Rc<Bus> {
    BUS.with(|b| b.clone())
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
    lines.set_value(p.lines as f64);
    let dim = adw::SpinRow::with_range(rr::MIN_DIM as f64, rr::MAX_DIM as f64, 5.0);
    dim.set_title(&tr("Dimming (percent)"));
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
    g.add(&on);
    g.add(&lines);
    g.add(&dim);
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

/// Dev hook `prefs:ruler-on|off`: the switch of the open Preferences.
pub fn dev_prefs(arg: &str) {
    SWITCH.with(|s| {
        if let Some(r) = s.borrow().as_ref() {
            r.set_active(arg != "off");
        }
    });
}
