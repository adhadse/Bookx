//! Developer helpers, enabled only by environment variables:
//!
//! `BOOKX_DEV_SCRIPT="wait:1500,shot:/tmp/a.png,read:1,wait:2000,shot:/tmp/b.png,quit"`
//!
//! `until:<name>[:<ms>]` waits (at most `<ms>`, default 15000) until the named probe holds instead of
//! sleeping a fixed time (`until:!name` waits for it to stop holding, e.g. `until:!reader` after `close:`); it prints `until: <name> ok|timeout` and goes on either way.
//!
//! Used to take screenshots of the real UI for documentation and to smoke-test layouts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use relm4::gtk::{self, glib, prelude::*};

type Probe = Rc<dyn Fn() -> bool>;

thread_local! {
    static PROBES: RefCell<HashMap<String, Probe>> = RefCell::new(HashMap::new());
}

/// Registers a condition that `until:<name>[:<timeout ms>]` in a dev script waits for (the newest
/// registration of a name wins, so a reader opened later replaces the one that was left).
pub fn probe(name: &str, f: impl Fn() -> bool + 'static) {
    PROBES.with(|p| p.borrow_mut().insert(name.to_string(), Rc::new(f)));
}

fn probe_holds(name: &str) -> bool {
    // `!name` holds while the probe does not (an unknown probe does not hold)
    if let Some(rest) = name.strip_prefix('!') {
        return !probe_holds(rest);
    }
    let f = PROBES.with(|p| p.borrow().get(name).cloned());
    f.is_some_and(|f| f())
}

pub fn script() -> Vec<String> {
    std::env::var("BOOKX_DEV_SCRIPT")
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

thread_local! {
    static STALL_LABEL: RefCell<String> = const { RefCell::new(String::new()) };
}

/// `BOOKX_STALL_WATCH=MS`: reports every stretch longer than MS (default 100) in which the main loop did not
/// run, with the dev command that was last started (`stall: 340 ms after library:sort=title`). Started by
/// `run_script`; the numbers are of the build that runs (debug builds are several times slower than release).
fn watch_stalls() {
    let Some(limit) = std::env::var("BOOKX_STALL_WATCH")
        .ok()
        .map(|v| v.parse::<u128>().unwrap_or(100))
    else {
        return;
    };
    let last = std::rc::Rc::new(std::cell::Cell::new(std::time::Instant::now()));
    glib::timeout_add_local(std::time::Duration::from_millis(10), move || {
        let late = last.get().elapsed().as_millis().saturating_sub(10);
        last.set(std::time::Instant::now());
        if late >= limit {
            let what = STALL_LABEL.with(|l| l.borrow().clone());
            eprintln!("stall: {late} ms after {what}");
        }
        glib::ControlFlow::Continue
    });
}

/// Calls `run` for each command, honouring `wait:<ms>` between them.
pub fn run_script(cmds: Vec<String>, run: impl Fn(&str) + 'static) {
    watch_stalls();
    fn step(mut cmds: std::collections::VecDeque<String>, run: std::rc::Rc<dyn Fn(&str)>) {
        while let Some(c) = cmds.pop_front() {
            if let Some(ms) = c.strip_prefix("wait:") {
                let ms: u64 = ms.parse().unwrap_or(500);
                glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                    step(cmds, run)
                });
                return;
            }
            if let Some(spec) = c.strip_prefix("until:") {
                let (name, ms) = match spec.split_once(':') {
                    Some((n, ms)) => (n.to_string(), ms.parse().unwrap_or(15_000)),
                    None => (spec.to_string(), 15_000u64),
                };
                poll_until(name, ms, std::time::Instant::now(), cmds, run);
                return;
            }
            STALL_LABEL.with(|l| *l.borrow_mut() = c.clone());
            run(&c);
        }
    }
    fn poll_until(
        name: String,
        ms: u64,
        start: std::time::Instant,
        cmds: std::collections::VecDeque<String>,
        run: std::rc::Rc<dyn Fn(&str)>,
    ) {
        let ok = probe_holds(&name);
        if ok || start.elapsed().as_millis() as u64 >= ms {
            eprintln!("until: {name} {}", if ok { "ok" } else { "timeout" });
            step(cmds, run);
            return;
        }
        glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
            poll_until(name, ms, start, cmds, run)
        });
    }
    step(cmds.into(), std::rc::Rc::new(run));
}

pub fn screenshot(path: &str, which: usize) {
    let wins: Vec<gtk::Window> = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok())
        .filter(|w| w.is_visible())
        .collect();
    let win = wins.get(which).or_else(|| wins.first());
    let Some(win) = win else {
        eprintln!("screenshot: no visible window");
        return;
    };
    let (w, h) = (win.width(), win.height());
    eprintln!(
        "screenshot: {} windows, using {} {}x{}",
        wins.len(),
        win.type_().name(),
        w,
        h
    );
    eprintln!(
        "screenshot: mapped={} realized={} visible={} child_visible={}",
        win.is_mapped(),
        win.is_realized(),
        win.is_visible(),
        win.is_child_visible()
    );
    let paintable = gtk::WidgetPaintable::new(Some(win));
    eprintln!(
        "screenshot: paintable {}x{}",
        paintable.intrinsic_width(),
        paintable.intrinsic_height()
    );
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, w as f64, h as f64);
    let Some(node) = snapshot.to_node() else {
        eprintln!("screenshot: empty node");
        return;
    };
    let Some(renderer) = win.native().and_then(|n| n.renderer()) else {
        eprintln!("screenshot: no renderer");
        return;
    };
    let texture = renderer.render_texture(&node, None);
    if let Err(e) = texture.save_to_png(path) {
        eprintln!("screenshot failed: {e}");
    } else {
        eprintln!("screenshot saved: {path} ({w}x{h})");
    }
}
