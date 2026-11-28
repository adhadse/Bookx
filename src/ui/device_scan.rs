//! Looking for devices without blocking the window (issue #57: `ui/stall` of 280 to 2218 ms in
//! the Devices window). Every look at `/proc/mounts` and sysfs happens on a worker thread; the
//! answer comes back to the main loop and is dropped when a newer look was started meanwhile.

use std::cell::Cell;
use std::rc::Rc;

use super::gtk::{gio, glib};

use crate::devices::{Detected, DeviceSettings, remember, scan_blocking};

/// Starts scans on a worker and delivers each answer on the main loop, unless a newer scan was
/// started (or [`Scanner::invalidate`] called) in the meantime: a late answer is dropped. A scan
/// that takes longer than `SCAN_LIMIT` is abandoned (a hung mount never freezes anything).
#[derive(Clone, Default)]
pub struct Scanner {
    epoch: Rc<Cell<u64>>,
    busy: Rc<Cell<bool>>,
}

const SCAN_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

impl Scanner {
    pub fn new() -> Scanner {
        Scanner::default()
    }

    /// Drops the answer of any scan that is still running.
    pub fn invalidate(&self) {
        self.epoch.set(self.epoch.get() + 1);
    }

    /// Starts a scan and calls `deliver` with its answer on the main loop.
    pub fn spawn(
        &self,
        settings: DeviceSettings,
        with_cloud: bool,
        deliver: impl FnOnce(Detected) + 'static,
    ) {
        self.spawn_with(move || scan_blocking(&settings, with_cloud), deliver);
    }

    /// [`Scanner::spawn`] with the scan itself given (tests).
    pub fn spawn_with(
        &self,
        scan: impl FnOnce() -> Detected + Send + 'static,
        deliver: impl FnOnce(Detected) + 'static,
    ) {
        self.invalidate();
        let n = self.epoch.get();
        let (epoch, busy) = (self.epoch.clone(), self.busy.clone());
        busy.set(true);
        glib::spawn_future_local(async move {
            let r =
                gio::spawn_blocking(move || bookx_serve::devices::bounded(SCAN_LIMIT, scan)).await;
            busy.set(false);
            let Ok(Some(d)) = r else {
                crate::trace::event(crate::trace::Area::Devices, "scan_abandoned").ok();
                return;
            };
            if epoch.get() != n {
                crate::trace::op(crate::trace::Area::Devices, "scan_dropped")
                    .quiet()
                    .ok();
                return;
            }
            remember(&d);
            deliver(d);
        });
    }

    /// Like [`Scanner::spawn`], but does nothing while a scan is still running (a timer).
    pub fn spawn_if_idle(
        &self,
        settings: DeviceSettings,
        with_cloud: bool,
        deliver: impl FnOnce(Detected) + 'static,
    ) {
        if !self.busy.get() {
            self.spawn(settings, with_cloud, deliver);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn named(n: &str) -> Detected {
        Detected {
            devices: vec![bookx_core::contracts::DeviceInfo {
                id: n.into(),
                name: n.into(),
                mount: n.into(),
                parent: None,
            }],
            ..Default::default()
        }
    }

    /// Runs the main loop of a private context until `done` says so (or a second passes).
    fn run(ctx: &glib::MainContext, done: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !done() && start.elapsed() < Duration::from_secs(5) {
            ctx.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_late_answer_of_an_overtaken_scan_is_dropped() {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let s = Scanner::new();
            let got: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
            let g = got.clone();
            s.spawn_with(
                || {
                    std::thread::sleep(Duration::from_millis(300));
                    named("old")
                },
                move |d| g.borrow_mut().push(d.devices[0].id.clone()),
            );
            let g = got.clone();
            s.spawn_with(
                || named("new"),
                move |d| g.borrow_mut().push(d.devices[0].id.clone()),
            );
            run(&ctx, || !got.borrow().is_empty() && !s.busy.get());
            std::thread::sleep(Duration::from_millis(400));
            ctx.iteration(false);
            assert_eq!(
                *got.borrow(),
                vec!["new".to_string()],
                "the older answer never arrives"
            );
        })
        .unwrap();
    }

    #[test]
    fn invalidate_drops_the_answer_of_a_closed_window() {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let s = Scanner::new();
            let got = Rc::new(Cell::new(false));
            let g = got.clone();
            s.spawn_with(|| named("x"), move |_| g.set(true));
            s.invalidate();
            run(&ctx, || !s.busy.get());
            assert!(!got.get());
        })
        .unwrap();
    }

    #[test]
    fn a_finished_scan_frees_the_scanner() {
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(|| {
            let s = Scanner::new();
            assert!(!s.busy.get());
            let got = Rc::new(Cell::new(false));
            let g = got.clone();
            s.spawn_with(|| named("fast"), move |_| g.set(true));
            assert!(s.busy.get());
            run(&ctx, || got.get());
            assert!(got.get() && !s.busy.get());
        })
        .unwrap();
    }
}
