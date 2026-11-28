//! Report a Problem: the dialog that builds the report of `crate::report`, shows exactly what would be
//! shared, and hands it over by clipboard, file or the GitHub issue form. Bookx sends nothing itself.
//!
//! Dev hooks (`report:<command>`): `open`, `open-crash`, `open-from=AREA` (as the Sync window opens it),
//! `titles=on|off`, `host=on|off`, `type=TEXT`, `log=on|off`, `library=on|off`,
//! `preview` (prints the preview on one line), `dump`, `copy`, `save=PATH`, `github`, `folder`, `log-line=TEXT`
//! (writes a warning to the log), `crash-sim` (runs the panic hook's writer on a made-up crash), `notices`,
//! `dismiss-beta`, `dismiss-crash`, `report-from-beta`, `report-from-crash`, `recheck`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::{SharedRef, window_of};
use crate::debug_log::{Scrub, Scrubber};
use crate::i18n::{tr, trf};
use crate::report::{self, Input, Options};

/// How the dialog was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefill {
    None,
    /// After "Bookx closed unexpectedly": the first line of the text is filled in.
    Crash,
}

/// The rules that clean the report: the debug-log rules plus the names of devices and this machine.
pub fn scrub_rules(shared: &SharedRef) -> Scrub {
    // names the user gave to devices (and what a device calls itself, which may be a typed name)
    let mut names = Vec::new();
    if let Some(a) = shared.sharing.borrow().activity() {
        for d in a.devices() {
            names.push(d.alias);
            names.push(d.user);
            if crate::trace::device_label(&d.name) == "<custom name>" {
                names.push(d.name);
            }
        }
    }
    crate::diag_gather::scrub_rules(
        &shared.cfg.borrow(),
        &shared.lib.borrow(),
        names,
        &std::fs::read_to_string("/etc/hostname").unwrap_or_default(),
    )
}

/// What the built-in server knows about the devices that talked to it, as plain data.
fn server_view(shared: &SharedRef) -> crate::diag_gather::ServerView {
    use bookx_serve::activity::What;
    let sh = shared.sharing.borrow();
    let mut v = crate::diag_gather::ServerView {
        running: sh.running(),
        ..Default::default()
    };
    if let Some(a) = sh.activity() {
        let now = chrono::Utc::now().timestamp();
        v.devices = a
            .devices()
            .into_iter()
            .map(|d| crate::diag_gather::SeenDevice {
                reported: d.name,
                client: d.client,
                plugin: d.plugin,
                protocol: d.protocol,
                requests: d.requests,
                last_sync_ago: d.last_sync.map(|t| now - t),
                last_seen_ago: now - d.last_seen,
            })
            .collect();
        let events = a.events(50);
        v.events = events.len();
        let mut tally: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
        for e in &events {
            if let What::Rejected { status, .. } = e.what {
                *tally.entry(status).or_default() += 1;
            }
        }
        v.rejected = tally.into_iter().collect();
    }
    v
}

/// The name of the toolkit's display and renderer, as words.
fn display_words(parent: Option<&gtk::Widget>) -> (String, String) {
    let backend = gtk::gdk::Display::default()
        .map(|d| d.type_().name().to_ascii_lowercase())
        .map(|n| {
            if n.contains("wayland") {
                "wayland"
            } else if n.contains("x11") {
                "x11"
            } else if n.contains("broadway") {
                "broadway"
            } else if n.contains("macos") {
                "macos"
            } else if n.contains("win32") {
                "windows"
            } else {
                "other"
            }
            .to_string()
        })
        .unwrap_or_default();
    let renderer = parent
        .and_then(|w| w.native())
        .and_then(|n| n.renderer())
        .map(|r| {
            let t = r.type_().name().to_ascii_lowercase();
            ["ngl", "vulkan", "gl", "cairo"]
                .iter()
                .find(|k| t.contains(*k))
                .map_or("unknown", |k| *k)
                .to_string()
        })
        .unwrap_or_else(|| "unknown".into());
    (backend, renderer)
}

/// Everything the report is made of, read from the running app. `entry` is the area the user pressed
/// Report in (its section comes first).
pub fn gather(
    shared: &SharedRef,
    entry: Option<&str>,
    parent: Option<&gtk::Widget>,
) -> (Input, Scrubber) {
    let rules = scrub_rules(shared);
    // the log file is cleaned with the same rules from now on
    crate::applog::set_scrub(rules.clone());
    let scrub = Scrubber::new(rules);
    let log = match crate::applog::file() {
        Some(f) => f.tail(report::LOG_LINES * 4),
        None => crate::debug_log::ring()
            .lines()
            .into_iter()
            .filter(|l| crate::applog::keeps(l))
            .collect(),
    };
    let lib = shared.lib.borrow();
    let indexed = shared
        .fulltext
        .borrow()
        .as_ref()
        .map(|ft| (ft.indexed_books(), lib.books.len()));
    let cfg = shared.cfg.borrow();
    let plugins = shared.plugins.borrow();
    let (backend, renderer) = display_words(parent);
    let vars = [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "GSK_RENDERER",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
    .collect();
    let trace = crate::trace::merged(&log);
    let world = crate::diag_gather::World {
        env: report::Environment::detect(),
        cfg: &cfg,
        lib: &lib,
        indexed,
        plugins: plugins.list(),
        patches: std::fs::read_dir(crate::plugins::patches_dir())
            .map(|rd| rd.flatten().count())
            .unwrap_or(0),
        keyring: match crate::secrets::vault().mode() {
            bookx_secrets::Mode::Keyring => "system keyring",
            bookx_secrets::Mode::Vault => "encrypted store",
            bookx_secrets::Mode::Locked => "encrypted store (locked)",
            _ => "not saved (session only)",
        }
        .to_string(),
        server: server_view(shared),
        jobs: report::recent_jobs(),
        log,
        trace,
        previous: crate::applog::file()
            .map(|f| crate::trace::load_tail(&f.dir))
            .unwrap_or_default(),
        crash: crate::applog::file().and_then(|f| f.last_crash_text()),
        flatpak_info: std::fs::read_to_string("/.flatpak-info").ok(),
        backend,
        renderer,
        vars,
        extra: {
            let mut x = crate::trace::collect();
            if entry.is_some_and(|e| e.split('|').any(|a| a == "Devices")) {
                let seen = crate::devices::latest();
                let others = crate::diag_gather::other_drive_rows(&seen.others);
                let list: Vec<_> = seen
                    .devices
                    .into_iter()
                    .map(|d| crate::diag_gather::ConnectedDevice {
                        reported: d.name,
                        id: d.id,
                        mounted: d.mount.exists(),
                        free: crate::diag_system::free_space(&d.mount),
                    })
                    .collect();
                let mut rows = crate::diag_gather::device_rows(&list);
                rows.extend(others);
                x.push((crate::trace::Area::Devices, rows));
            }
            x
        },
        entry: entry.map(str::to_string),
        probe: None,
        helpers: crate::diag_gather::Helpers::detect(),
    };
    let input = crate::diag_gather::input(&world);
    (input, scrub)
}

struct Dlg {
    dialog: adw::Dialog,
    input: Input,
    scrub: Scrubber,
    user: gtk::TextBuffer,
    preview: gtk::TextBuffer,
    log: adw::SwitchRow,
    library: adw::SwitchRow,
    titles: adw::SwitchRow,
    host: adw::SwitchRow,
    hint: gtk::Label,
    toasts: adw::ToastOverlay,
    timer: RefCell<Option<glib::SourceId>>,
    clipboard_len: Cell<usize>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<Dlg>>> = const { RefCell::new(None) };
}

impl Dlg {
    fn options(&self) -> Options {
        Options {
            include_log: self.log.is_active(),
            include_library: self.library.is_active(),
            include_titles: self.titles.is_active(),
            include_host: self.host.is_active(),
        }
    }

    fn user_text(&self) -> String {
        let (a, b) = self.user.bounds();
        self.user.text(&a, &b, false).to_string()
    }

    fn text(&self) -> String {
        report::build(&self.input, self.options(), &self.user_text(), &self.scrub)
    }

    fn refresh(&self) {
        let t = self.text();
        if self
            .preview
            .text(&self.preview.start_iter(), &self.preview.end_iter(), false)
            != t
        {
            self.preview.set_text(&t);
        }
    }

    fn refresh_soon(self: &Rc<Self>) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
        let me = self.clone();
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
            me.timer.borrow_mut().take();
            me.refresh();
        });
        *self.timer.borrow_mut() = Some(id);
    }

    fn toast(&self, text: &str) {
        super::dialog_toast::show_plain(&self.toasts, text);
    }

    fn copy(&self) -> String {
        self.refresh();
        let t = self.text();
        let cb = self.dialog.clipboard();
        cb.set_text(&t);
        self.clipboard_len.set(t.len());
        t
    }

    fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        report::write_report(path, &self.text()).map_err(|e| e.to_string())
    }

    fn github(&self) {
        // the report first: the form takes only a short title, the rest is pasted
        self.copy();
        self.hint.set_visible(true);
        self.toast(&tr(
            "The report is on your clipboard: paste it into the issue.",
        ));
        open_link(report::ISSUE_URL);
    }

    fn open_folder(&self) {
        if let Some(dir) = log_dir() {
            let _ = std::fs::create_dir_all(&dir);
            gtk::FileLauncher::new(Some(&gio::File::for_path(&dir))).launch(
                window_of(&self.dialog.clone().upcast::<gtk::Widget>()).as_ref(),
                gio::Cancellable::NONE,
                |_| {},
            );
        }
    }
}

/// Opens the address through the hardened gate. Under a dev script it only says what it would open
/// (the UI tests have no browser, and must not start one).
fn open_link(url: &str) {
    if std::env::var_os("BOOKX_DEV_SCRIPT").is_some() {
        eprintln!("report_open: {url}");
    } else {
        super::open_external_link(url);
    }
}

fn log_dir() -> Option<std::path::PathBuf> {
    crate::applog::file().map(|f| f.dir.clone())
}

/// The folder as shown: the home folder is `~`.
fn shown_path(p: &std::path::Path) -> String {
    let home = crate::applog::home_dir_text();
    let t = p.display().to_string();
    if home.len() > 1 && t.starts_with(&home) {
        format!("~{}", &t[home.len()..])
    } else {
        t
    }
}

fn intro_text() -> String {
    if crate::beta::channel().is_prerelease() {
        tr(
            "Bookx is in beta. Reports help us fix bugs. This report contains no book titles, passwords or personal paths.",
        )
    } else {
        tr(
            "Reports help us fix bugs. This report contains no book titles, passwords or personal paths.",
        )
    }
}

fn text_card(view: &gtk::TextView, min_height: i32) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(view));
    sw.set_min_content_height(min_height);
    sw.set_max_content_height(min_height * 3);
    sw.set_propagate_natural_height(false);
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    sw.add_css_class("card");
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(10);
    view.set_right_margin(10);
    sw
}

fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.add_css_class("heading");
    l
}

fn button(label: &str) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.set_valign(gtk::Align::Center);
    super::audit::label(&b, label);
    b
}

/// Opens the dialog over `parent`.
pub fn show(shared: &SharedRef, parent: &impl IsA<gtk::Widget>, prefill: Prefill) {
    show_from(shared, parent, prefill, None)
}

/// [`show`] from a window of a feature: `entry` is the title of that feature's section (for example
/// `Sync (KOSync client)`), which then comes first in the report.
pub fn show_from(
    shared: &SharedRef,
    parent: &impl IsA<gtk::Widget>,
    prefill: Prefill,
    entry: Option<&str>,
) {
    let (input, scrub) = gather(shared, entry, Some(parent.upcast_ref()));
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Report a Problem"));
    dialog.set_content_width(620);
    dialog.set_content_height(640);

    let intro = gtk::Label::new(Some(&intro_text()));
    intro.set_wrap(true);
    intro.set_xalign(0.0);
    let privacy = gtk::Label::new(Some(&tr(
        "Bookx sends nothing by itself: you decide where the report goes.",
    )));
    privacy.set_wrap(true);
    privacy.set_xalign(0.0);
    privacy.add_css_class("dim-label");

    // what happened
    let what_label = caption(&tr("What happened?"));
    let what = gtk::TextView::new();
    what.set_wrap_mode(gtk::WrapMode::WordChar);
    what.set_accepts_tab(false);
    super::audit::label(&what, &tr("What happened?"));
    let what_card = text_card(&what, 96);
    let what_hint = gtk::Label::new(Some(&tr(
        "Your own words are shared exactly as you type them: leave out anything private.",
    )));
    what_hint.set_wrap(true);
    what_hint.set_xalign(0.0);
    what_hint.add_css_class("dim-label");
    what_hint.add_css_class("caption");
    if prefill == Prefill::Crash {
        what.buffer()
            .set_text(&tr("Bookx closed unexpectedly.\n\n"));
    }

    // what is included
    let include = adw::PreferencesGroup::new();
    include.set_title(&tr("Included in the Report"));
    let log = adw::SwitchRow::new();
    log.set_title(&tr("Include recent log"));
    log.set_subtitle(&tr("Warnings, errors and a few start-up notes."));
    log.set_active(true);
    let library = adw::SwitchRow::new();
    library.set_title(&tr("Include library statistics"));
    library.set_subtitle(&tr("How many books and which formats, never titles."));
    library.set_active(true);
    let titles = adw::SwitchRow::new();
    titles.set_title(&tr("Include book titles and authors of the books involved"));
    titles.set_subtitle(&tr(
        "Off by default. Only the books the failing action was about, never your whole library.",
    ));
    titles.set_active(false);
    let host = adw::SwitchRow::new();
    host.set_title(&tr("Include the server address host"));
    // a subtitle is Pango markup: the address example holds `<` and `>`
    host.set_subtitle(&glib::markup_escape_text(&tr(
        "Off by default. Without it an address shows only its kind, port and path, such as http://<ip:private>:30260/api/koreader.",
    )));
    host.set_active(false);
    include.add(&log);
    include.add(&library);
    include.add(&titles);
    include.add(&host);

    // what is in it, in plain words
    let contents = gtk::Label::new(Some(
        &report::CONTENTS
            .iter()
            .map(|l| format!("• {}", tr(l)))
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    contents.set_wrap(true);
    contents.set_xalign(0.0);
    contents.add_css_class("dim-label");
    contents.add_css_class("caption");
    contents.set_selectable(true);
    super::audit::label(&contents, &tr("What the report contains"));

    // the preview
    let preview_label = caption(&tr("Preview"));
    let preview_hint = gtk::Label::new(Some(&tr("This is exactly what will be shared.")));
    preview_hint.set_xalign(0.0);
    preview_hint.add_css_class("dim-label");
    let preview = gtk::TextView::new();
    preview.set_editable(false);
    preview.set_cursor_visible(false);
    preview.set_monospace(true);
    preview.set_wrap_mode(gtk::WrapMode::WordChar);
    super::audit::label(&preview, &tr("Report preview"));
    let preview_card = text_card(&preview, 200);

    // the log folder
    let folder = adw::PreferencesGroup::new();
    let folder_row = adw::ActionRow::new();
    folder_row.set_title(&tr("Log Folder"));
    folder_row.set_subtitle(&glib::markup_escape_text(
        &log_dir().map(|p| shown_path(&p)).unwrap_or_default(),
    ));
    folder_row.set_subtitle_selectable(true);
    folder_row.set_subtitle_lines(0);
    let copy_path = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy_path.set_valign(gtk::Align::Center);
    copy_path.add_css_class("flat");
    copy_path.set_tooltip_text(Some(&tr("Copy Log Folder Path")));
    super::audit::label(&copy_path, &tr("Copy Log Folder Path"));
    let open_folder = button(&tr("Open Log Folder"));
    folder_row.add_suffix(&copy_path);
    folder_row.add_suffix(&open_folder);
    folder.add(&folder_row);

    let hint = gtk::Label::new(Some(&tr(
        "The report is on your clipboard: paste it into the issue.",
    )));
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    hint.add_css_class("accent");
    hint.set_visible(false);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    for w in [
        intro.upcast_ref::<gtk::Widget>(),
        privacy.upcast_ref(),
        what_label.upcast_ref(),
        what_card.upcast_ref(),
        what_hint.upcast_ref(),
        include.upcast_ref(),
        contents.upcast_ref(),
        preview_label.upcast_ref(),
        preview_hint.upcast_ref(),
        preview_card.upcast_ref(),
        folder.upcast_ref(),
        hint.upcast_ref(),
    ] {
        body.append(w);
    }
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(640);
    clamp.set_child(Some(&body));
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&clamp));
    scroller.set_vexpand(true);
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);

    let copy = button(&tr("Copy Report"));
    let save = button(&tr("Save Report"));
    let github = button(&tr("Open GitHub Issue"));
    github.add_css_class("suggested-action");
    let bar = adw::WrapBox::new();
    bar.set_child_spacing(8);
    bar.set_line_spacing(8);
    bar.set_margin_top(8);
    bar.set_margin_bottom(8);
    bar.set_margin_start(12);
    bar.set_margin_end(12);
    bar.set_justify(adw::JustifyMode::Fill);
    bar.set_natural_line_length(480);
    for b in [&copy, &save, &github] {
        bar.append(b);
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroller));
    view.add_bottom_bar(&bar);
    let toasts = super::dialog_toast::overlay(&view);
    dialog.set_child(Some(&toasts));

    let d = Rc::new(Dlg {
        dialog: dialog.clone(),
        input,
        scrub,
        user: what.buffer(),
        preview: preview.buffer(),
        log,
        library,
        titles,
        host,
        hint,
        toasts,
        timer: RefCell::new(None),
        clipboard_len: Cell::new(0),
    });
    d.refresh();
    {
        let dd = d.clone();
        d.user.connect_changed(move |_| dd.refresh_soon());
        let dd = d.clone();
        d.log.connect_active_notify(move |_| dd.refresh());
        let dd = d.clone();
        d.library.connect_active_notify(move |_| dd.refresh());
        let dd = d.clone();
        d.titles.connect_active_notify(move |_| dd.refresh());
        let dd = d.clone();
        d.host.connect_active_notify(move |_| dd.refresh());
    }
    {
        let dd = d.clone();
        copy.connect_clicked(move |_| {
            dd.copy();
            dd.toast(&tr("Report Copied"));
        });
        let dd = d.clone();
        github.connect_clicked(move |_| dd.github());
        let dd = d.clone();
        open_folder.connect_clicked(move |_| dd.open_folder());
        let dd = d.clone();
        copy_path.connect_clicked(move |_| {
            let p = log_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            dd.dialog.clipboard().set_text(&p);
            dd.toast(&tr("Path Copied"));
        });
        let dd = d.clone();
        save.connect_clicked(move |_| {
            let dlg = gtk::FileDialog::builder()
                .title(tr("Save Report"))
                .initial_name(format!(
                    "bookx-report-{}.md",
                    chrono::Local::now().format("%Y-%m-%d")
                ))
                .build();
            let dd2 = dd.clone();
            dlg.save(
                window_of(&dd.dialog.clone().upcast::<gtk::Widget>()).as_ref(),
                gio::Cancellable::NONE,
                move |res| {
                    let Some(path) = res.ok().and_then(|f| f.path()) else {
                        return;
                    };
                    match dd2.save_to(&path) {
                        Ok(()) => dd2.toast(&tr("Report Saved")),
                        Err(e) => dd2.toast(&trf("Could not save the report: {}", &[&e])),
                    }
                },
            );
        });
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(d.clone()));
    {
        let dd = d.clone();
        dialog.connect_closed(move |_| {
            if let Some(id) = dd.timer.borrow_mut().take() {
                id.remove();
            }
            CURRENT.with(|c| {
                let mut c = c.borrow_mut();
                if c.as_ref().is_some_and(|x| Rc::ptr_eq(x, &dd)) {
                    *c = None;
                }
            });
        });
    }
    dialog.set_focus(Some(&what));
    dialog.present(Some(parent));
    if prefill == Prefill::Crash {
        // put the caret after the first line, ready to type
        let end = what.buffer().end_iter();
        what.buffer().place_cursor(&end);
    }
}

/// Preferences → General → Troubleshooting.
pub fn preferences_group(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Troubleshooting"));
    g.set_description(Some(&tr(
        "Bookx is in beta and can have bugs. Reports help us fix them.",
    )));
    let row = adw::ActionRow::new();
    row.set_title(&tr("Report a Problem"));
    row.set_subtitle(&tr(
        "See exactly what a report contains before you share it.",
    ));
    row.set_activatable(true);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let (parent, shared) = (parent.clone().upcast::<gtk::Widget>(), shared.clone());
    row.connect_activated(move |_| {
        let w = window_of(&parent)
            .map(|w| w.upcast::<gtk::Widget>())
            .unwrap_or_else(|| parent.clone());
        show(&shared, &w, Prefill::None)
    });
    g.add(&row);
    g
}

/// A group with one row, "Report a Problem", for a window of a feature (Sync, Devices): it opens the
/// dialog with `area` (the titles of the sections to put first, separated by `|`). The row is
/// activated through the application action, so the window needs no handle on the app.
pub fn problem_group(area: &'static str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let row = adw::ActionRow::new();
    row.set_title(&tr("Report a Problem"));
    row.set_subtitle(&tr(
        "Something not working? See what a report contains before you share it.",
    ));
    row.set_subtitle_lines(0);
    row.set_activatable(true);
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.connect_activated(move |r| {
        let _ = r.activate_action("app.report-problem-from", Some(&area.to_variant()));
    });
    g.add(&row);
    g
}

/// `report:<command>` of a dev script.
pub fn dev_command(shared: &SharedRef, window: &gtk::Window, arg: &str) {
    let (cmd, val) = arg.split_once('=').unwrap_or((arg, ""));
    let cur = CURRENT.with(|c| c.borrow().clone());
    match cmd {
        "open" => show(shared, window, Prefill::None),
        "open-crash" => show(shared, window, Prefill::Crash),
        "open-from" => show_from(shared, window, Prefill::None, Some(val)),
        "log-line" => tracing::warn!("{val}"),
        "crash-sim" => {
            let info = crate::applog::CrashInfo {
                message: val.to_string(),
                location: Some("src/dev.rs:1:1".into()),
                thread: "main".into(),
                backtrace: std::backtrace::Backtrace::force_capture().to_string(),
            };
            let (_, scrub) = gather(shared, None, None);
            let r = crate::applog::file().map(|f| {
                crate::applog::write_crash(f, &info, &scrub, chrono::Utc::now())
                    .map(|p| p.display().to_string())
            });
            eprintln!("report_crash: {r:?}");
        }
        _ => {
            let Some(d) = cur else {
                eprintln!("report: no dialog");
                return;
            };
            match cmd {
                "type" => {
                    d.user.set_text(&val.replace("\\n", "\n"));
                    d.refresh();
                }
                "log" => d.log.set_active(val != "off"),
                "library" => d.library.set_active(val != "off"),
                "titles" => d.titles.set_active(val == "on"),
                "host" => d.host.set_active(val == "on"),
                "preview" => eprintln!("report_preview: {}", d.text().replace('\n', "⏎")),
                "dump" => {
                    let t = d.text();
                    eprintln!(
                        "report_dump: len={} lines={} log={} library={} titles={} host={} focus_on_entry={} hint={}",
                        t.len(),
                        t.lines().count(),
                        d.log.is_active(),
                        d.library.is_active(),
                        d.titles.is_active(),
                        d.host.is_active(),
                        d.dialog
                            .focus()
                            .and_then(|w| w.downcast::<gtk::TextView>().ok())
                            .is_some_and(|t| t.is_editable()),
                        d.hint.is_visible()
                    );
                }
                "copy" => {
                    let t = d.copy();
                    let first = t.lines().next().unwrap_or("").to_string();
                    d.dialog
                        .clipboard()
                        .read_text_async(gio::Cancellable::NONE, move |r| {
                            let got = r.ok().flatten().map(|s| s.to_string()).unwrap_or_default();
                            eprintln!(
                                "report_clipboard: len={} same={} first={}",
                                got.len(),
                                got == t,
                                first
                            );
                        });
                }
                "save" => match d.save_to(std::path::Path::new(val)) {
                    Ok(()) => eprintln!("report_saved: {val}"),
                    Err(e) => eprintln!("report_save_error: {e}"),
                },
                "github" => {
                    d.github();
                    eprintln!(
                        "report_github: clipboard_len={} hint={}",
                        d.clipboard_len.get(),
                        d.hint.is_visible()
                    );
                }
                "folder" => eprintln!(
                    "report_folder: {}",
                    log_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
                _ => eprintln!("report: unknown command {cmd}"),
            }
        }
    }
}
