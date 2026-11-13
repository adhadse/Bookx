//! The Jobs panel: a button in the library header with a popover listing the background jobs
//! (imports, conversions, sends, metadata fetches, backups, news) with their progress, a Cancel
//! button where the job can be stopped and the log of each. The model is `crate::jobs`.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::collections::HashSet;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use relm4::RelmWidgetExt;

use crate::jobs::{self, Job, Kind, State};

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
    static OPEN_ROWS: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

#[derive(Clone)]
struct Panel {
    button: glib::WeakRef<gtk::MenuButton>,
    list: glib::WeakRef<gtk::Box>,
    spinner: glib::WeakRef<gtk::Spinner>,
    popover: glib::WeakRef<gtk::Popover>,
}

/// The header button. Created once per library window.
pub fn build() -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.add_css_class("flat");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let spinner = gtk::Spinner::new();
    spinner.set_visible(false);
    content.append(&spinner);
    content.append(&gtk::Image::from_icon_name("view-list-symbolic"));
    button.set_child(Some(&content));
    super::audit::label(&button, &tr("Background jobs"));

    let pop = gtk::Popover::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let title = gtk::Label::new(Some(&tr("Background Jobs")));
    title.add_css_class("heading");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    head.append(&title);
    let clear = gtk::Button::with_label(&tr("Clear Finished"));
    clear.add_css_class("flat");
    clear.connect_clicked(|_| jobs::clear_finished());
    head.append(&clear);
    body.append(&head);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_min_content_width(380);
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
            spinner: spinner.downgrade(),
            popover: pop.downgrade(),
        })
    });
    jobs::on_change(refresh);
    pop.connect_show(|_| refresh());
    refresh();
    button
}

fn refresh() {
    let Some(panel) = PANEL.with(|p| p.borrow().clone()) else {
        return;
    };
    let (Some(button), Some(list), Some(spinner), Some(pop)) = (
        panel.button.upgrade(),
        panel.list.upgrade(),
        panel.spinner.upgrade(),
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
    spinner.set_visible(running > 0);
    spinner.set_spinning(running > 0);
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
    if jobs_now.is_empty() {
        let empty = gtk::Label::new(Some(
            "Nothing is running. Imports, conversions, sends and downloads show up here.",
        ));
        empty.set_wrap(true);
        empty.set_max_width_chars(40);
        empty.add_css_class("dim-label");
        list.append(&empty);
        return;
    }
    let group = adw::PreferencesGroup::new();
    for job in jobs_now.iter().rev() {
        group.add(&job_row(job));
    }
    list.append(&group);
}

fn job_row(job: &Job) -> adw::ExpanderRow {
    let now = chrono::Utc::now().timestamp_millis();
    let mut sub = job.state_text();
    if !job.detail.is_empty() && job.running() {
        sub = format!("{sub} · {}", job.detail);
    }
    sub = format!("{sub} · {}", jobs::duration_text(job.elapsed_ms(now)));
    let row = adw::ExpanderRow::builder()
        .title(glib::markup_escape_text(&job.title))
        .subtitle(glib::markup_escape_text(&sub))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(job.kind.icon()));
    row.set_expanded(OPEN_ROWS.with(|o| o.borrow().contains(&job.id)));
    {
        let id = job.id;
        row.connect_expanded_notify(move |r| {
            OPEN_ROWS.with(|o| {
                if r.is_expanded() {
                    o.borrow_mut().insert(id);
                } else {
                    o.borrow_mut().remove(&id);
                }
            })
        });
    }
    if job.running() {
        let bar = gtk::ProgressBar::new();
        bar.set_valign(gtk::Align::Center);
        bar.set_width_request(90);
        match job.progress {
            Some(f) => bar.set_fraction(f),
            None => bar.pulse(),
        }
        super::audit::label(&bar, &trf("Progress of {}", &[&job.title]));
        row.add_suffix(&bar);
    } else if matches!(job.state, State::Failed(_)) {
        row.add_suffix(&gtk::Image::from_icon_name("dialog-error-symbolic"));
    } else if job.state == State::Done {
        row.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    if job.running() && job.cancellable {
        let cancel = gtk::Button::from_icon_name("process-stop-symbolic");
        cancel.add_css_class("flat");
        cancel.set_valign(gtk::Align::Center);
        cancel.set_sensitive(!job.cancel_asked());
        cancel.set_tooltip_text(Some(&tr("Cancel")));
        super::audit::label(&cancel, &trf("Cancel {}", &[&job.title]));
        let id = job.id;
        cancel.connect_clicked(move |_| {
            jobs::cancel(id);
        });
        row.add_suffix(&cancel);
    }
    let log = if job.log.is_empty() {
        "No messages".to_string()
    } else {
        job.log.join("\n")
    };
    let text = gtk::Label::new(Some(&log));
    text.set_xalign(0.0);
    text.set_wrap(true);
    text.set_selectable(true);
    text.add_css_class("caption");
    text.add_css_class("monospace");
    text.set_margin_all(8);
    let log_row = adw::ActionRow::new();
    log_row.set_child(Some(&text));
    log_row.set_activatable(false);
    row.add_row(&log_row);
    row
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
            // what the open panel shows: one line per job row (title, state)
            let list = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.list.upgrade()));
            let mut out = Vec::new();
            if let Some(list) = list {
                collect_rows(list.upcast_ref(), &mut out);
            }
            format!("rows {}", out.join(" ; "))
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
    if let Some(r) = w.downcast_ref::<adw::ExpanderRow>() {
        out.push(format!("{} / {}", r.title(), r.subtitle()));
        return;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        collect_rows(&child, out);
        c = child.next_sibling();
    }
}
