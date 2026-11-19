//! The Jobs panel: a button in the library header with a popover listing the background jobs
//! (imports, conversions, sends, metadata fetches, backups, news) with their progress, a Cancel
//! button where the job can be stopped and the log of each. The model is `crate::jobs`.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::collections::HashSet;

use relm4::adw::prelude::*;
use relm4::gtk::{self, glib};

use relm4::RelmWidgetExt;

use super::progress_ring::{ProgressRing, Ring};
use crate::jobs::{self, Job, Kind, State};

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
    static OPEN_ROWS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    /// The rings of the rows now in the panel, for the tests.
    static ROW_RINGS: RefCell<Vec<(u64, ProgressRing)>> = const { RefCell::new(Vec::new()) };
}

/// The width of the popover (bounded by the window when that is narrower).
const POPOVER_WIDTH: i32 = 380;

#[derive(Clone)]
struct Panel {
    button: glib::WeakRef<gtk::MenuButton>,
    list: glib::WeakRef<gtk::Box>,
    scroll: glib::WeakRef<gtk::ScrolledWindow>,
    body: glib::WeakRef<gtk::Box>,
    badge: glib::WeakRef<gtk::Label>,
    ring: ProgressRing,
    popover: glib::WeakRef<gtk::Popover>,
}

/// The header button. Created once per library window.
pub fn build() -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.add_css_class("flat");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let ring = ProgressRing::new(18);
    ring.widget.set_visible(false);
    content.append(&ring.widget);
    content.append(&gtk::Image::from_icon_name("view-list-symbolic"));
    let badge = gtk::Label::new(None);
    badge.add_css_class("jobs-badge");
    badge.set_visible(false);
    content.append(&badge);
    button.set_child(Some(&content));
    super::audit::label(&button, &tr("Background jobs"));

    let pop = gtk::Popover::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(10);
    body.set_margin_end(10);
    body.set_width_request(POPOVER_WIDTH);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some(&tr("Background Jobs")));
    title.add_css_class("heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&title);
    let clear = gtk::Button::with_label(&tr("Clear Finished"));
    clear.add_css_class("flat");
    clear.set_halign(gtk::Align::End);
    clear.connect_clicked(|_| jobs::clear_finished());
    head.append(&clear);
    body.append(&head);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    // vertical scrolling only: the rows always fit the width
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_width(false);
    scroll.set_min_content_height(120);
    scroll.set_max_content_height(420);
    scroll.set_propagate_natural_height(true);
    body.append(&scroll);
    pop.set_child(Some(&body));
    button.set_popover(Some(&pop));

    PANEL.with(|p| {
        *p.borrow_mut() = Some(Panel {
            button: button.downgrade(),
            list: list.downgrade(),
            scroll: scroll.downgrade(),
            body: body.downgrade(),
            badge: badge.downgrade(),
            ring,
            popover: pop.downgrade(),
        })
    });
    jobs::on_change(refresh);
    pop.connect_show(|p| {
        fit_to_window(p);
        refresh();
    });
    refresh();
    button
}

/// The popover is as wide as `POPOVER_WIDTH` unless the window is narrower, and its list is at most
/// 60 % of the window high.
fn fit_to_window(pop: &gtk::Popover) {
    let Some(win) = pop.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let width = (win.width() - 40).clamp(240, POPOVER_WIDTH);
    let (body, scroll) = PANEL.with(|p| {
        p.borrow()
            .as_ref()
            .map(|p| (p.body.upgrade(), p.scroll.upgrade()))
            .unwrap_or((None, None))
    });
    if let Some(b) = body {
        b.set_width_request(width);
    }
    if let Some(s) = scroll {
        s.set_max_content_height(((win.height() as f64 * 0.6) as i32).max(160));
    }
}

fn refresh() {
    let Some(panel) = PANEL.with(|p| p.borrow().clone()) else {
        return;
    };
    let (Some(button), Some(list), Some(badge), Some(pop)) = (
        panel.button.upgrade(),
        panel.list.upgrade(),
        panel.badge.upgrade(),
        panel.popover.upgrade(),
    ) else {
        return;
    };
    let (jobs_now, summary, running, overall) = jobs::with(|h| {
        (
            h.jobs().to_vec(),
            h.summary(),
            h.running_count(),
            h.overall(),
        )
    });
    panel.ring.widget.set_visible(running > 0);
    panel.ring.set(
        match overall {
            Some(f) => Ring::Fraction(f),
            None => Ring::Busy,
        },
        "",
    );
    badge.set_visible(running > 1);
    badge.set_label(&running.to_string());
    let summary = match overall {
        Some(f) => format!("{summary} ({:.0}%)", f * 100.0),
        None => summary,
    };
    button.set_tooltip_text(Some(&trf("Background jobs: {}", &[&summary])));
    if !pop.is_visible() {
        return;
    }
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    ROW_RINGS.with(|r| r.borrow_mut().clear());
    if jobs_now.is_empty() {
        let empty = gtk::Label::new(Some(&tr(
            "Nothing is running. Imports, conversions, sends and downloads show up here.",
        )));
        empty.set_wrap(true);
        empty.set_max_width_chars(40);
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        list.append(&empty);
        return;
    }
    for job in jobs_now.iter().rev() {
        list.append(&job_row(job));
    }
}

fn dim(text: &str, wrap: bool) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("caption");
    l.add_css_class("dim-label");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    l.set_width_chars(8);
    l.set_max_width_chars(60);
    if wrap {
        l.set_wrap(true);
        l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        l.set_lines(2);
    }
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_visible(!text.is_empty());
    l
}

/// One job: a ring (the percent, or an icon when it ends) at the left, the title (two lines at
/// most), what it is doing and the time left (or how long it took), and the Cancel or Retry button
/// at the right; a click on the row shows its log.
fn job_row(job: &Job) -> gtk::Widget {
    let now = chrono::Utc::now().timestamp_millis();
    let lines = jobs::status_lines(job, now);
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.add_css_class("job-row");
    outer.set_widget_name("job-row");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let ring = ProgressRing::new(36);
    let failed = matches!(job.state, State::Failed(_));
    match &job.state {
        State::Running => ring.set(
            match job.progress {
                Some(f) => Ring::Fraction(f),
                None => Ring::Busy,
            },
            job.kind.icon(),
        ),
        State::Done => ring.set(Ring::Idle, "object-select-symbolic"),
        State::Failed(_) => ring.set(Ring::Idle, "dialog-warning-symbolic"),
        State::Cancelled => ring.set(Ring::Idle, "process-stop-symbolic"),
    }
    if matches!(job.state, State::Done) {
        ring.widget.add_css_class("success");
    }
    if failed {
        ring.widget.add_css_class("warning");
    }
    ROW_RINGS.with(|r| r.borrow_mut().push((job.id, ring.clone())));
    super::audit::label(
        &ring.widget,
        &if job.running() {
            trf("Progress of {}", &[&job.title])
        } else {
            lines.primary.clone()
        },
    );
    row.append(&ring.widget);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&job.title));
    title.add_css_class("job-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(2);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_width_chars(8);
    title.set_max_width_chars(48);
    title.set_tooltip_text(Some(&job.title));
    text.append(&title);
    let primary = dim(&lines.primary, failed);
    primary.add_css_class("job-primary");
    text.append(&primary);
    let secondary = dim(&lines.secondary, false);
    secondary.add_css_class("job-secondary");
    text.append(&secondary);
    row.append(&text);

    if job.running() && job.cancellable {
        let cancel = gtk::Button::from_icon_name("window-close-symbolic");
        cancel.add_css_class("flat");
        cancel.add_css_class("circular");
        cancel.set_valign(gtk::Align::Center);
        cancel.set_sensitive(!job.cancel_asked());
        cancel.set_tooltip_text(Some(&tr("Cancel")));
        super::audit::label(&cancel, &trf("Cancel {}", &[&job.title]));
        let id = job.id;
        cancel.connect_clicked(move |_| {
            jobs::cancel(id);
        });
        row.append(&cancel);
    } else if failed && jobs::can_retry(job.id) {
        let retry = gtk::Button::with_label(&tr("Retry"));
        retry.set_valign(gtk::Align::Center);
        retry.set_hexpand(false);
        super::audit::label(&retry, &trf("Retry {}", &[&job.title]));
        let id = job.id;
        retry.connect_clicked(move |_| {
            jobs::retry(id);
        });
        row.append(&retry);
    }
    outer.append(&row);

    // the log, shown by a click on the row
    let log = if job.log.is_empty() {
        tr("No messages")
    } else {
        job.log.join("\n")
    };
    let text = gtk::Label::new(Some(&log));
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.set_selectable(true);
    text.set_width_chars(8);
    text.set_max_width_chars(60);
    text.add_css_class("caption");
    text.add_css_class("monospace");
    text.set_margin_all(6);
    let revealer = gtk::Revealer::new();
    revealer.set_child(Some(&text));
    revealer.set_reveal_child(OPEN_ROWS.with(|o| o.borrow().contains(&job.id)));
    outer.append(&revealer);
    let click = gtk::GestureClick::new();
    click.set_button(gdk_primary());
    {
        let (id, revealer) = (job.id, revealer.clone());
        click.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let on = !revealer.reveals_child();
            revealer.set_reveal_child(on);
            OPEN_ROWS.with(|o| {
                if on {
                    o.borrow_mut().insert(id);
                } else {
                    o.borrow_mut().remove(&id);
                }
            });
        });
    }
    row.add_controller(click);
    outer.upcast()
}

fn gdk_primary() -> u32 {
    1
}

fn parse_kind(s: &str) -> Kind {
    match s {
        "convert" => Kind::Convert,
        "send" => Kind::Send,
        "fetch" => Kind::Fetch,
        "backup" => Kind::Backup,
        "news" => Kind::News,
        "voice" => Kind::Voice,
        "dictionary" => Kind::Dictionary,
        _ => Kind::Import,
    }
}

/// `jobs:` dev commands (see `_developer/docs.md`); printed as `jobs: …`.
pub fn dev(cmd: &str) -> String {
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    let mut parts = arg.split('|');
    let mut next = || parts.next().unwrap_or("");
    match name {
        "start" => {
            let (kind, title, cancellable) = (next().to_string(), next().to_string(), next());
            let (id, _) = jobs::start(parse_kind(&kind), &title, cancellable == "cancel");
            format!("started {id}")
        }
        "progress" => {
            let id: u64 = next().parse().unwrap_or(0);
            let f: f64 = next().parse().unwrap_or(0.0);
            let detail = next().to_string();
            jobs::progress(id, Some(f), &detail);
            format!("progress {id}")
        }
        "log" => {
            let id: u64 = next().parse().unwrap_or(0);
            jobs::log(id, next());
            format!("logged {id}")
        }
        "finish" => {
            let id: u64 = next().parse().unwrap_or(0);
            let how = next().to_string();
            let r = if how.is_empty() || how == "ok" {
                Ok(())
            } else {
                Err(how)
            };
            jobs::finish(id, r);
            format!("finished {id}")
        }
        "cancel" => {
            let id: u64 = arg.parse().unwrap_or(0);
            format!("cancel {id} accepted={}", jobs::cancel(id))
        }
        "clear" => {
            jobs::clear_finished();
            "cleared".into()
        }
        "open" => {
            let b = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.button.upgrade()));
            match b {
                Some(b) => {
                    b.popup();
                    "opened".into()
                }
                None => "no panel".into(),
            }
        }
        "close" => {
            if let Some(b) = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.button.upgrade())) {
                b.popdown();
            }
            "closed".into()
        }
        "rows" => {
            // what the open panel shows: one line per job row (title / what it does / the second line)
            let list = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.list.upgrade()));
            let mut out = Vec::new();
            if let Some(list) = list {
                collect_rows(list.upcast_ref(), &mut out);
            }
            format!("rows {}", out.join(" ; "))
        }
        // the width of what the panel shows against the width of the popover: no sideways scrolling
        "overflow" => {
            let (scroll, body, pop) = PANEL.with(|p| {
                p.borrow()
                    .as_ref()
                    .map(|p| (p.scroll.upgrade(), p.body.upgrade(), p.popover.upgrade()))
                    .unwrap_or((None, None, None))
            });
            match (scroll, body, pop) {
                (Some(s), Some(b), Some(p)) => {
                    let a = s.hadjustment();
                    format!(
                        "overflow upper={:.0} page={:.0} body={} popover={} scroll_h={} rows_w={}",
                        a.upper(),
                        a.page_size(),
                        b.width(),
                        p.width(),
                        s.height(),
                        s.child()
                            .and_then(|c| c.first_child())
                            .map_or(0, |r| r.width())
                    )
                }
                _ => "overflow no panel".into(),
            }
        }
        // the state of the rings: the header one and one per row
        "rings" => {
            let header = PANEL.with(|p| p.borrow().as_ref().map(|p| p.ring.describe()));
            let rows: Vec<String> = ROW_RINGS.with(|r| {
                r.borrow()
                    .iter()
                    .map(|(id, r)| format!("{id}={}", r.describe()))
                    .collect()
            });
            format!(
                "rings header={} rows=[{}]",
                header.unwrap_or_default(),
                rows.join(", ")
            )
        }
        "retryable" => {
            let id: u64 = arg.parse().unwrap_or(0);
            jobs::set_retry(id, std::rc::Rc::new(move || jobs::log(id, "retried")));
            format!("retryable {id}")
        }
        "retry" => {
            let id: u64 = arg.parse().unwrap_or(0);
            format!("retry {id} ran={}", jobs::retry(id))
        }
        "dump" => {
            let lines: Vec<String> = jobs::with(|h| {
                h.jobs()
                    .iter()
                    .map(|j| {
                        format!(
                            "{}|{}|{}|{}|{}|cancellable={}|log={}",
                            j.id,
                            j.kind.label(),
                            j.state_text(),
                            j.progress.map_or("-".into(), |f| format!("{:.2}", f)),
                            j.title,
                            j.cancellable,
                            j.log.len()
                        )
                    })
                    .collect()
            });
            format!(
                "running={} {}",
                jobs::with(|h| h.running_count()),
                lines.join(" ; ")
            )
        }
        _ => format!("unknown {cmd}"),
    }
}

fn collect_rows(w: &gtk::Widget, out: &mut Vec<String>) {
    if w.widget_name() == "job-row" {
        let mut parts: Vec<String> = Vec::new();
        for class in ["job-title", "job-primary", "job-secondary"] {
            if let Some(l) = find_class(w, class).and_then(|l| l.downcast::<gtk::Label>().ok()) {
                if !l.label().is_empty() {
                    parts.push(l.label().to_string());
                }
            }
        }
        out.push(parts.join(" / "));
        return;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        collect_rows(&child, out);
        c = child.next_sibling();
    }
}

fn find_class(w: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if w.has_css_class(class) {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(f) = find_class(&child, class) {
            return Some(f);
        }
        c = child.next_sibling();
    }
    None
}
