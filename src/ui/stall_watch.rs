//! The GTK timer behind `diag_stall`: ticks every 100 ms on the main loop and records a stall when it
//! ran late. Installed once at start-up; costs one cheap wake-up ten times a second.

use relm4::gtk::glib;

use crate::diag_stall::{StallMeter, TICK_MS};

pub fn install() {
    let t0 = std::time::Instant::now();
    let mut meter = StallMeter::default();
    glib::timeout_add_local(std::time::Duration::from_millis(TICK_MS), move || {
        if let Some(late) = meter.tick(t0.elapsed().as_millis() as u64) {
            crate::diag_stall::record(late);
        }
        glib::ControlFlow::Continue
    });
}
