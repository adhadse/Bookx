//! Developer helpers, enabled only by environment variables:
//!
//! `BOOKX_DEV_SCRIPT="wait:1500,shot:/tmp/a.png,read:1,wait:2000,shot:/tmp/b.png,quit"`
//!
//! Used to take screenshots of the real UI for documentation and to smoke-test layouts.

use relm4::gtk::{self, glib, prelude::*};

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

/// Calls `run` for each command, honouring `wait:<ms>` between them.
pub fn run_script(cmds: Vec<String>, run: impl Fn(&str) + 'static) {
    fn step(mut cmds: std::collections::VecDeque<String>, run: std::rc::Rc<dyn Fn(&str)>) {
        while let Some(c) = cmds.pop_front() {
            if let Some(ms) = c.strip_prefix("wait:") {
                let ms: u64 = ms.parse().unwrap_or(500);
                glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                    step(cmds, run)
                });
                return;
            }
            run(&c);
        }
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
