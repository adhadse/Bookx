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
        self.invalidate();
        let n = self.epoch.get();
        let (epoch, busy) = (self.epoch.clone(), self.busy.clone());
        busy.set(true);
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || {
                bookx_serve::devices::bounded(SCAN_LIMIT, move || {
                    scan_blocking(&settings, with_cloud)
                })
            })
            .await;
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
