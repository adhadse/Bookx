//! KOReader annotation sync: the switch and the plugin installer in the Share window, the timer
//! that merges what devices sent, and the dev commands the UI tests use (`koannot:*`, with a fake
//! KOReader client that makes the same HTTP calls as the plugin). The rules are in
//! `annotation_sync`; the protocol in `bookx_serve::annotation_sync`.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use bookx_serve::annotation_sync::{AnnotationSync, ORIGIN_BOOKX, Source};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use crate::annotation_sync::{self as sync, Ledger, Report};
use crate::i18n::{tr, trf};
use crate::ui::SharedRef;

/// The files of the plugin, built into the program so the installer needs no other files.
const PLUGIN_FILES: [(&str, &str); 7] = [
    (
        "_meta.lua",
        include_str!("../koreader-plugin/bookx.koplugin/_meta.lua"),
    ),
    (
        "main.lua",
        include_str!("../koreader-plugin/bookx.koplugin/main.lua"),
    ),
    (
        "bookxsync.lua",
        include_str!("../koreader-plugin/bookx.koplugin/bookxsync.lua"),
    ),
    (
        "bookxjson.lua",
        include_str!("../koreader-plugin/bookx.koplugin/bookxjson.lua"),
    ),
    (
        "bookxmd5.lua",
        include_str!("../koreader-plugin/bookx.koplugin/bookxmd5.lua"),
    ),
    (
        "LICENSE",
        include_str!("../koreader-plugin/bookx.koplugin/LICENSE"),
    ),
    ("README.md", include_str!("../koreader-plugin/README.md")),
];

/// Folder name KOReader looks for.
pub const PLUGIN_DIR: &str = "bookx.koplugin";

/// KOReader's plugins folder on a mounted device: `.adds/koreader/plugins` on a Kobo (where
/// KOReader is installed under `.adds`), `koreader/plugins` elsewhere. `None` when KOReader is not
/// installed there.
pub fn plugins_folder(mount: &Path) -> Option<PathBuf> {
    [".adds/koreader", "koreader"]
        .iter()
        .map(|d| mount.join(d))
        .find(|d| d.is_dir())
        .map(|d| d.join("plugins"))
}

/// Writes the plugin into `<plugins>/bookx.koplugin` (the folder is made; files of an earlier
/// version are replaced one by one). Returns the plugin folder. A symbolic link in the way is an
/// error: the installer never writes through one.
pub fn install_plugin(plugins: &Path) -> Result<PathBuf> {
    let dir = plugins.join(PLUGIN_DIR);
    if let Ok(m) = std::fs::symlink_metadata(&dir) {
        if m.file_type().is_symlink() {
            bail!("{} is a link; remove it first", dir.display());
        }
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    for (name, text) in PLUGIN_FILES {
        let target = dir.join(name);
        if let Ok(m) = std::fs::symlink_metadata(&target) {
            if m.file_type().is_symlink() {
                bail!("{} is a link; remove it first", target.display());
            }
        }
        let tmp = dir.join(format!(".{name}.tmp"));
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &target).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(dir)
}

pub(crate) fn data_dir() -> PathBuf {
    crate::config::stats_path()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(std::env::temp_dir)
}

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static STATUS: RefCell<Option<gtk::Label>> = const { RefCell::new(None) };
    static SEEN: Cell<u64> = const { Cell::new(u64::MAX) };
    static LAST_TOAST: RefCell<String> = const { RefCell::new(String::new()) };
    static SOURCES: RefCell<Option<std::collections::HashMap<(u64, String), String>>> = const { RefCell::new(None) };
}

fn set_status(text: &str) {
    LAST.with(|l| *l.borrow_mut() = text.to_string());
    STATUS.with(|s| {
        if let Some(l) = &*s.borrow() {
            l.set_label(text);
        }
    });
}

/// `KOReader (Kobo Libra)` for a highlight that came from a device, for the Notes window.
pub fn source_label(book_id: u64, highlight_id: &str) -> Option<String> {
    SOURCES.with(|s| {
        let mut s = s.borrow_mut();
        let map =
            s.get_or_insert_with(|| sync::sources(&Ledger::load(&sync::ledger_path(&data_dir()))));
        map.get(&(book_id, highlight_id.to_string())).cloned()
    })
}

/// The bookmarks the devices brought, by `(book id, bookmark id)`: where from (`KOReader (Kobo
/// Libra)`, `Kobo`) and whether the place is a rough one. Both syncs (wireless and cable) are read.
pub fn bookmark_infos(root: &Path) -> std::collections::HashMap<(u64, String), (String, bool)> {
    let ledger = Ledger::load(&sync::ledger_path(&data_dir()));
    let mut map = crate::bookmark_sync::infos(&ledger.bookmarks);
    map.extend(crate::bookmark_sync::infos(
        &crate::bookmark_sync::Store::load(root).entries,
    ));
    map
}

/// Where the device had a highlight that a sync added (read from the ledger each time: it is only
/// used when a note is opened in the book).
pub fn hint_for(book_id: u64, highlight_id: &str) -> Option<crate::highlight_goto::Hint> {
    let ledger = Ledger::load(&sync::ledger_path(&data_dir()));
    sync::hints(&ledger).remove(&(book_id, highlight_id.to_string()))
}

/// Test hook: remembers a device's xpointer for a highlight the way a sync would.
pub fn dev_seed_hint(book_id: u64, highlight_id: &str, text: &str, pos0: &str) {
    let path = sync::ledger_path(&data_dir());
    let mut ledger = Ledger::load(&path);
    ledger.incoming.insert(
        format!("dev/{highlight_id}"),
        sync::Incoming {
            book_id,
            highlight: Some(highlight_id.to_string()),
            source: "KOReader (test)".into(),
            note: String::new(),
            device_look: Default::default(),
            updated: 1,
            seq: 1,
            state: sync::State::Linked,
            text: text.to_string(),
            made_here: true,
            hint: crate::highlight_goto::Hint {
                pos0: pos0.to_string(),
                ..Default::default()
            },
        },
    );
    let _ = ledger.save(&path);
}

/// "Last synced from Kobo Libra 3 minutes ago" for the details of a book, empty when no device sent anything for it.
pub fn last_sync_line(book_id: u64) -> String {
    let ledger = Ledger::load(&sync::ledger_path(&data_dir()));
    ledger
        .last_synced
        .get(&book_id)
        .map_or_else(String::new, |s| {
            crate::sync_activity::book_line(&s.device, s.time, chrono::Utc::now().timestamp())
        })
}

/// Merges what the devices sent into the library and publishes the library's highlights for the
/// books the devices know. `None` when the server does not serve annotations. Does nothing and
/// returns `None` if nothing changed since the last merge and `force` is off.
pub fn sync_once(shared: &SharedRef, force: bool) -> Option<Report> {
    let ann = shared.sharing.borrow().annotations()?;
    let seq = ann.seq();
    if !force && SEEN.with(|s| s.get()) == seq {
        return None;
    }
    let mut lib = shared.lib.try_borrow_mut().ok()?;
    let path = sync::ledger_path(&data_dir());
    let mut ledger = Ledger::load(&path);
    if !force && ledger.seen == seq {
        SEEN.with(|s| s.set(seq));
        return None;
    }
    let before = crate::sync_bus::Snapshot::take(&lib.books);
    let report = run(&mut lib, &ann, &mut ledger);
    publish_report(&before, &lib.books, &report);
    ledger.seen = ann.seq();
    SEEN.with(|s| s.set(ledger.seen));
    let _ = ledger.save(&path);
    if report.added
        + report.removed
        + report.notes
        + report.linked
        + report.looks
        + report.bookmarks
        > 0
    {
        let _ = lib.save();
    }
    SOURCES.with(|s| *s.borrow_mut() = Some(sync::sources(&ledger)));
    drop(lib);
    set_status(&report.text());
    Some(report)
}

/// Tells the open readers and the library window what a merge changed: one event per book that is
/// different now, named after the device that brought it.
fn publish_report(
    before: &crate::sync_bus::Snapshot,
    books: &[crate::core::library::Book],
    report: &Report,
) {
    for mut ev in before.events(books, "", crate::sync_bus::Via::Wifi) {
        if let Some(d) = report.books.iter().find(|d| d.book_id == ev.book_id) {
            ev.device = d.device.clone();
        }
        crate::sync_bus::publish(ev);
    }
}

/// The merge itself, on a library and a store (no GTK; the tests of the pieces are in
/// `annotation_sync`).
pub fn run(
    lib: &mut crate::core::library::Library,
    ann: &AnnotationSync,
    ledger: &mut Ledger,
) -> Report {
    let docs = ann.documents();
    let root = lib.root.clone();
    sync::resolve_docs(&root, &lib.books, &docs, ledger, &sync::digest_of_file);
    let changes = ann.changes_since(0);
    let locator = crate::device_sync::book_locator(lib);
    let mut report = sync::merge_incoming(
        &mut lib.books,
        &changes,
        ledger,
        &locator,
        chrono::Utc::now(),
    );
    if let Some(log) = ann.activity() {
        sync::record_merge(
            &log,
            ledger,
            &report,
            &docs,
            &lib.books,
            chrono::Utc::now().timestamp(),
        );
    }
    // The server's change number may not have moved: an older sync linked a coloured annotation
    // with Bookx's default yellow. Repair it now, before publishing anything back.
    report.looks += sync::repair_looks(&mut lib.books, &changes, ledger);
    let now = chrono::Utc::now().timestamp();
    for (doc, list, entries) in sync::outgoing(&lib.books, ledger, now) {
        let mut ok = true;
        for (user, d, _, _) in docs.iter().filter(|d| d.1 == doc) {
            let _ = d;
            let src = Source {
                origin: ORIGIN_BOOKX,
                device: "Bookx",
                device_id: "",
                title: "",
                authors: "",
            };
            if ann.put(user, &doc, &src, list.clone()).is_err() {
                ok = false;
            }
        }
        if ok {
            report.sent += list.len();
            sync::commit_outgoing(ledger, entries);
        }
    }
    report
}

/// Starts the timer that merges new annotations (every few seconds, only when the server's change
/// counter moved). Call once at start-up.
pub fn install(shared: &SharedRef) {
    install_live(shared);
    let weak = Rc::downgrade(shared);
    relm4::gtk::glib::timeout_add_seconds_local(4, move || match weak.upgrade() {
        Some(s) => {
            if let Some(report) = sync_once(&s, false) {
                announce(&s, &report);
            }
            relm4::gtk::glib::ControlFlow::Continue
        }
        None => relm4::gtk::glib::ControlFlow::Break,
    });
}

thread_local! {
    static MERGE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Live updates: a device talked to the built-in server. The merge runs a moment later (a burst
/// of requests is one merge) instead of at the next tick of the timer, and the library views, the
/// open readers and the banner for a device that is ahead hear of it through `sync_bus`.
fn install_live(shared: &SharedRef) {
    let weak = Rc::downgrade(shared);
    crate::sync_bus::install(move |notes| {
        let Some(shared) = weak.upgrade() else { return };
        let plan = crate::sync_bus::plan(&notes);
        if plan.merge && !MERGE_PENDING.with(|p| p.replace(true)) {
            let weak = Rc::downgrade(&shared);
            relm4::gtk::glib::timeout_add_local_once(
                std::time::Duration::from_millis(crate::sync_bus::DEBOUNCE_MS),
                move || {
                    MERGE_PENDING.with(|p| p.set(false));
                    if let Some(s) = weak.upgrade() {
                        if let Some(report) = sync_once(&s, false) {
                            announce(&s, &report);
                        }
                    }
                },
            );
        }
        for p in &plan.positions {
            publish_position(&shared, p);
        }
    });
}

/// A device reported where it is in a book: if the book is in the library, readers of it may offer
/// to jump there. The library's own position is never touched here.
fn publish_position(shared: &SharedRef, p: &crate::sync_bus::PositionNote) {
    let own = shared.cfg.borrow().kosync.device_id.clone();
    if !own.is_empty() && p.device_id == own {
        return;
    }
    let book = {
        let Ok(lib) = shared.lib.try_borrow() else {
            return;
        };
        let path = sync::ledger_path(&data_dir());
        let mut ledger = Ledger::load(&path);
        let docs = vec![(
            String::new(),
            p.document.clone(),
            String::new(),
            String::new(),
        )];
        sync::resolve_docs(
            &lib.root,
            &lib.books,
            &docs,
            &mut ledger,
            &sync::digest_of_file,
        );
        let _ = ledger.save(&path);
        ledger.docs.get(&p.document).copied()
    };
    let Some(book_id) = book else { return };
    let mut ev = crate::sync_bus::SyncEvent::new(
        book_id,
        crate::sync_bus::Kinds::PROGRESS,
        &p.device,
        crate::sync_bus::Via::Wifi,
    );
    ev.ahead = Some(crate::sync_bus::Ahead {
        percentage: p.percentage,
        device_id: p.device_id.clone(),
        progress: p.progress.clone(),
    });
    crate::sync_bus::publish(ev);
}

/// What arrived in a merge, as the toast says it.
pub fn arrivals(
    lib: &crate::core::library::Library,
    report: &Report,
) -> Vec<crate::sync_activity::Arrival> {
    report
        .books
        .iter()
        .filter(|d| d.arrived() + d.bookmarks > 0)
        .map(|d| crate::sync_activity::Arrival {
            book_id: d.book_id,
            title: lib
                .get(d.book_id)
                .map(|b| b.meta.title.clone())
                .unwrap_or_default(),
            device: d.device.clone(),
            highlights: d.arrived(),
            bookmarks: d.bookmarks,
        })
        .collect()
}

/// A sync arrived while Bookx is open: a toast with a button that opens Notes & Highlights for the book,
/// and the library views (and the counts in the sidebar) read the highlights again.
fn announce(shared: &SharedRef, report: &Report) {
    let arrived = arrivals(&shared.lib.borrow(), report);
    if arrived.is_empty() && report.looks + report.notes + report.removed == 0 {
        return;
    }
    crate::settings_bus::sync().notify();
    let Some(text) = crate::sync_activity::toast_text(&arrived) else {
        return;
    };
    let title = arrived.first().map(|a| a.title.clone()).unwrap_or_default();
    LAST_TOAST.with(|t| *t.borrow_mut() = text.clone());
    let book = arrived.first().map(|a| a.book_id);
    let open_here = book.is_some_and(crate::sync_bus::has_reader_for);
    if let Some(overlay) = crate::ui::dialog_toast::app_overlay() {
        match book {
            // a book that is not open: the message can open it
            Some(id) if !open_here => crate::ui::dialog_toast::show_action_and(
                &overlay,
                &text,
                &tr("Show"),
                move || crate::ui::sync_activity::show_notes(&title),
                (&tr("Open Book"), move || {
                    crate::ui::sync_activity::open_book(id)
                }),
            ),
            _ => crate::ui::dialog_toast::show_action(&overlay, &text, &tr("Show"), move || {
                crate::ui::sync_activity::show_notes(&title)
            }),
        }
    }
}

/// The rows of step 3 (copy the Bookx plugin to the e-reader or a folder), with the result inline.
pub fn install_rows(parent: &gtk::Widget, shared: &SharedRef) -> Vec<gtk::Widget> {
    let install_row = adw::ActionRow::builder()
        .title(tr("Bookx plugin for KOReader"))
        .build();
    install_row.set_title_lines(0);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.set_selectable(true);
    status.add_css_class("dim-label");
    let copy = gtk::Button::with_label(&tr("Copy to the device"));
    copy.set_valign(gtk::Align::Center);
    crate::ui::audit::label(
        &copy,
        &tr("Copy the Bookx plugin to the connected e-reader"),
    );
    let folder = gtk::Button::with_label(&tr("Save to a folder"));
    folder.set_valign(gtk::Align::Center);
    crate::ui::audit::label(&folder, &tr("Save the Bookx plugin to a folder"));
    install_row.add_suffix(&copy);
    install_row.add_suffix(&folder);
    {
        let (status, shared) = (status.clone(), shared.clone());
        copy.connect_clicked(move |_| {
            let found = crate::devices::detect_physical()
                .into_iter()
                .find_map(|d| plugins_folder(&d.mount).map(|p| (d.name, p)));
            match found {
                None => status.set_label(&tr(
                    "No e-reader with KOReader found. Connect it with the USB cable, or save the plugin to a folder.",
                )),
                Some((name, plugins)) => match install_plugin(&plugins) {
                    Ok(dir) => {
                        mark_plugin_copied(&shared);
                        status.set_label(&trf(
                            "Copied to {} ({}). Restart KOReader, then open Bookx Sync in its top menu.",
                            &[&name, &dir.display().to_string()],
                        ))
                    }
                    Err(e) => status.set_label(&trf("Could not copy the plugin: {}", &[&format!("{e:#}")])),
                },
            }
        });
    }
    {
        let (status, parent, shared) = (status.clone(), parent.clone(), shared.clone());
        folder.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title(tr("Choose the folder for the plugin"))
                .build();
            let (status, shared) = (status.clone(), shared.clone());
            let win = parent.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            dialog.select_folder(win.as_ref(), gtk::gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    match install_plugin(&p) {
                        Ok(dir) => {
                            mark_plugin_copied(&shared);
                            status.set_label(&trf("Saved to {}", &[&dir.display().to_string()]))
                        }
                        Err(e) => status
                            .set_label(&trf("Could not save the plugin: {}", &[&format!("{e:#}")])),
                    }
                }
            });
        });
    }

    install_row.set_tooltip_text(Some(&tr("Kobo: .adds/koreader/plugins/bookx.koplugin · other readers: koreader/plugins/bookx.koplugin")));
    vec![install_row.upcast(), status.upcast()]
}

/// Remembers that the plugin was copied (step 3 of the setup is done) and tells the Sync views.
fn mark_plugin_copied(shared: &SharedRef) {
    {
        let mut c = shared.cfg.borrow_mut();
        if c.share.plugin_copied {
            return;
        }
        c.share.plugin_copied = true;
        c.save();
    }
    crate::settings_bus::sync().notify();
}

/// True when a KOReader device has sent something to this computer (the last step of the setup).
pub fn device_seen(shared: &SharedRef) -> bool {
    shared
        .sharing
        .borrow()
        .annotations()
        .is_some_and(|a| !a.documents().is_empty())
}

/// The group “What Bookx sync does”: the switch for highlights, notes and bookmarks and the status.
/// `changed` is called after the switch moved (the window applies the settings).
pub fn group(
    _parent: &gtk::Widget,
    shared: &SharedRef,
    changed: Rc<dyn Fn()>,
) -> (adw::PreferencesGroup, adw::SwitchRow) {
    let g = adw::PreferencesGroup::new();
    g.set_title(&crate::sync_model::title("what"));
    g.set_description(Some(&relm4::gtk::glib::markup_escape_text(&format!(
        "{}\n\n{}",
        tr(crate::sync_model::BRAND_EXPLAINED),
        tr(crate::sync_model::WHAT_SYNCS)
    ))));
    let on = adw::SwitchRow::builder()
        .title(tr("Sync highlights, notes and bookmarks"))
        .subtitle(tr(
            "Uses the same account as the reading position. Highlights are placed by their text; one that is not found in the book is kept as a note without a place.",
        ))
        .build();
    on.set_active(shared.cfg.borrow().share.annotations);
    {
        let (shared, changed) = (shared.clone(), changed.clone());
        on.connect_active_notify(move |s| {
            {
                let mut c = shared.cfg.borrow_mut();
                if c.share.annotations == s.is_active() {
                    return;
                }
                c.share.annotations = s.is_active();
                c.save();
            }
            changed();
        });
    }
    g.add(&on);

    let merged = gtk::Label::new(None);
    merged.set_wrap(true);
    merged.set_xalign(0.0);
    merged.set_selectable(true);
    merged.add_css_class("dim-label");
    LAST.with(|l| {
        let t = l.borrow();
        merged.set_label(&if t.is_empty() {
            tr("Nothing received yet.")
        } else {
            trf("Last sync: {}", &[&*t])
        })
    });
    STATUS.with(|s| *s.borrow_mut() = Some(merged.clone()));
    g.add(&merged);
    let now = gtk::Button::with_label(&tr("Sync now"));
    now.add_css_class("flat");
    now.set_halign(gtk::Align::Start);
    crate::ui::audit::label(&now, &tr("Merge the highlights received from KOReader now"));
    {
        let (shared, merged) = (shared.clone(), merged.clone());
        now.connect_clicked(move |_| match sync_once(&shared, true) {
            Some(r) => merged.set_label(&trf("Last sync: {}", &[&r.text()])),
            None => merged.set_label(&tr(
                "Turn on the Bookx sync server (step 1) and this switch first.",
            )),
        });
    }
    g.add(&now);
    (g, on)
}

// -- dev commands -----------------------------------------------------------------------------

fn fake_device(shared: &SharedRef, spec: &str) -> Result<String> {
    let url = shared
        .sharing
        .borrow()
        .urls()
        .first()
        .cloned()
        .context("sharing is not running")?;
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(spec)?)?;
    let doc = match v["document"].as_str() {
        Some(d) if d.starts_with("auto:") => {
            let id: u64 = d[5..].parse()?;
            let lib = shared.lib.borrow();
            let b = lib.get(id).context("no such book")?;
            sync::digest_of_file(&lib.root.join(&b.file)).context("cannot read the book")?
        }
        Some(d) => d.to_string(),
        None => bail!("no document"),
    };
    let base = format!("{}/bookx", url.trim_end_matches('/'));
    let key = bookx_serve::kosync::md5_hex("pw");
    let t = std::time::Duration::from_secs(10);
    let post = |path: &str, body: &str| -> Result<bookx_serve::httpc::Reply> {
        bookx_serve::httpc::request(
            "POST",
            &format!("{base}{path}").parse()?,
            &[("Content-Type", "application/json")],
            body.as_bytes(),
            t,
        )
    };
    let _ = post(
        "/users/create",
        &serde_json::json!({"username": "ko", "password": key}).to_string(),
    )?;
    let body = serde_json::json!({
        "origin": "koreader", "device": v["device"],
        "device_id": format!("fake-{}", v["device"].as_str().unwrap_or("")),
        "title": v["title"], "authors": v["authors"], "annotations": v["annotations"],
    });
    let r = bookx_serve::httpc::request(
        "PUT",
        &format!("{base}/annotations/{doc}").parse()?,
        &[
            ("Content-Type", "application/json"),
            ("x-auth-user", "ko"),
            ("x-auth-key", &key),
            ("X-Bookx-Client", "KOReader/2026.07.2 bookx-plugin/0.3.1"),
            ("X-Bookx-Protocol", "1"),
        ],
        body.to_string().as_bytes(),
        t,
    )?;
    let a: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or_default();
    Ok(format!(
        "status={} doc={doc} accepted={} unchanged={} stale={} records={}",
        r.status,
        a["result"]["accepted"],
        a["result"]["unchanged"],
        a["result"]["stale"],
        a["annotations"].as_array().map_or(0, |l| l.len()),
    ))
}

thread_local! {
    static PUSH_CLOCK: Cell<i64> = const { Cell::new(0) };
}

/// A fake device's connection to the running server: the address of `/bookx`, the key, the document id of a book.
fn fake_link(shared: &SharedRef, book: u64) -> Result<(String, String, String)> {
    let url = shared
        .sharing
        .borrow()
        .urls()
        .first()
        .cloned()
        .context("sharing is not running")?;
    let doc = {
        let lib = shared.lib.borrow();
        let b = lib.get(book).context("no such book")?;
        sync::digest_of_file(&lib.root.join(&b.file)).context("cannot read the book")?
    };
    let base = format!("{}/bookx", url.trim_end_matches('/'));
    let key = bookx_serve::kosync::md5_hex("pw");
    let _ = bookx_serve::httpc::request(
        "POST",
        &format!("{base}/users/create").parse()?,
        &[("Content-Type", "application/json")],
        serde_json::json!({"username": "ko", "password": key})
            .to_string()
            .as_bytes(),
        std::time::Duration::from_secs(10),
    )?;
    Ok((base, key, doc))
}

/// `koannot:push=<book>|<text>|<colour>[|<note>[|<device>]]` and `koannot:pushdel=<book>|<text>`: a
/// device sends (or deletes) one highlight through the real HTTP server, exactly as the plugin
/// does, and nothing else: the merge, the events and the readers' repaint happen on their own.
fn push(shared: &SharedRef, rest: &str, delete: bool) -> Result<String> {
    use std::hash::{Hash, Hasher};
    let p: Vec<&str> = rest.split('|').collect();
    let book: u64 = p.first().and_then(|b| b.parse().ok()).context("book id")?;
    let text = p.get(1).copied().unwrap_or("");
    let colour = p.get(2).copied().unwrap_or("yellow");
    let note = p.get(3).copied().unwrap_or("");
    let device = p.get(4).copied().unwrap_or("Kobo Libra");
    let (base, key, doc) = fake_link(shared, book)?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    let id = format!("live-{:x}", h.finish());
    // each push is newer than the one before, even inside one second
    let at = PUSH_CLOCK.with(|c| {
        let t = chrono::Utc::now().timestamp().max(c.get() + 1);
        c.set(t);
        t
    });
    let mut a = serde_json::json!({
        "id": id, "kind": "highlight", "text": text, "note": note, "color": colour,
        "drawer": "lighten", "chapter": "One", "page": 3, "created": at - 5, "updated": at,
    });
    if delete {
        a["deleted"] = true.into();
    }
    let body = serde_json::json!({
        "origin": "koreader", "device": device, "device_id": format!("fake-{device}"),
        "title": "UI Test Book", "authors": "Test Author", "annotations": [a],
    });
    let r = bookx_serve::httpc::request(
        "PUT",
        &format!("{base}/annotations/{doc}").parse()?,
        &[
            ("Content-Type", "application/json"),
            ("x-auth-user", "ko"),
            ("x-auth-key", &key),
            ("X-Bookx-Client", "KOReader/2026.07.2 bookx-plugin/0.3.1"),
            ("X-Bookx-Protocol", "1"),
        ],
        body.to_string().as_bytes(),
        std::time::Duration::from_secs(10),
    )?;
    Ok(format!("status={} id={id}", r.status))
}

/// `koannot:progress=<book>|<percentage 0..1>|<device>`: a device stores its reading position, as
/// KOReader's progress sync does.
fn push_progress(shared: &SharedRef, rest: &str) -> Result<String> {
    let p: Vec<&str> = rest.split('|').collect();
    let book: u64 = p.first().and_then(|b| b.parse().ok()).context("book id")?;
    let pct: f64 = p
        .get(1)
        .and_then(|b| b.parse().ok())
        .context("percentage")?;
    let device = p.get(2).copied().unwrap_or("Kobo Libra");
    let (base, key, doc) = fake_link(shared, book)?;
    let body = serde_json::json!({
        "document": doc, "progress": "/body/DocFragment[3]/body", "percentage": pct,
        "device": device, "device_id": format!("fake-{device}"),
    });
    let r = bookx_serve::httpc::request(
        "PUT",
        &format!("{base}/syncs/progress").parse()?,
        &[
            ("Content-Type", "application/json"),
            ("x-auth-user", "ko"),
            ("x-auth-key", &key),
        ],
        body.to_string().as_bytes(),
        std::time::Duration::from_secs(10),
    )?;
    Ok(format!("status={}", r.status))
}

/// `koannot:<command>`: `start`, `device=<json file>`, `merge`, `dump`, `pull=<book id>`,
/// `addhl=<book id>;<text>;<note>`, `delhl=<book id>;<text>`, `editnote=<book id>;<text>;<note>`,
/// `plugin=<folder>`.
pub fn dev_command(shared: &SharedRef, arg: &str) {
    let (cmd, rest) = arg.split_once('=').unwrap_or((arg, ""));
    match cmd {
        "start" => {
            let mut s = shared.cfg.borrow().share.clone();
            s.port = 0;
            s.kosync = true;
            s.annotations = true;
            shared.cfg.borrow_mut().share = s.clone();
            match crate::ui::share::apply(shared, &s, true) {
                Ok(u) => eprintln!("koannot_started: {}", u.join(" ")),
                Err(e) => eprintln!("koannot_error: {e}"),
            }
        }
        "device" => match fake_device(shared, rest) {
            Ok(m) => eprintln!("koannot_put: {m}"),
            Err(e) => eprintln!("koannot_error: {e:#}"),
        },
        "push" | "pushdel" => match push(shared, rest, cmd == "pushdel") {
            Ok(m) => eprintln!("koannot_push: {m}"),
            Err(e) => eprintln!("koannot_error: {e:#}"),
        },
        "progress" => match push_progress(shared, rest) {
            Ok(m) => eprintln!("koannot_progress: {m}"),
            Err(e) => eprintln!("koannot_error: {e:#}"),
        },
        // the events that reached the readers and the library: `koannot:events` prints and clears
        "events" => {
            for l in crate::sync_bus::dev_log_take() {
                eprintln!("koannot_event: {l}");
            }
        }
        "toast" => {
            eprintln!(
                "koannot_toast: {:?}",
                LAST_TOAST.with(|t| t.borrow().clone())
            );
            eprintln!("koannot_lastsync: {:?}", last_sync_line(1));
        }
        "merge" => match sync_once(shared, true) {
            Some(r) => {
                announce(shared, &r);
                eprintln!(
                    "koannot_merge: added={} linked={} notes={} removed={} kept={} conflicts={} unplaced={} bookmarks={} unmatched={} sent={} looks={} bookmarks_skipped={}",
                    r.added,
                    r.linked,
                    r.notes,
                    r.removed,
                    r.kept,
                    r.conflicts,
                    r.unplaced,
                    r.bookmarks,
                    r.unmatched,
                    r.sent,
                    r.looks,
                    r.bookmarks_skipped
                )
            }
            None => eprintln!("koannot_merge: not running"),
        },
        "dump" => {
            let ledger = Ledger::load(&sync::ledger_path(&data_dir()));
            let lib = shared.lib.borrow();
            for b in &lib.books {
                for h in &b.highlights {
                    eprintln!(
                        "koannot_hl: book={} id={} anchored={} spine={} start={} note={:?} source={:?} text={:?} color={:?} style={:?}",
                        b.id,
                        h.id,
                        !crate::core::anchor::is_unanchored(h),
                        h.spine,
                        h.start,
                        h.note,
                        ledger.source_of(b.id, &h.id).unwrap_or(""),
                        h.text,
                        h.color,
                        h.style
                    );
                }
            }
            let infos = bookmark_infos(&lib.root);
            for b in &lib.books {
                for m in &b.bookmarks {
                    let (src, approx) = infos
                        .get(&(b.id, m.id.clone()))
                        .cloned()
                        .unwrap_or_default();
                    eprintln!(
                        "koannot_bm: book={} id={} spine={} frac={:.2} label={:?} source={:?} approx={}",
                        b.id, m.id, m.spine, m.frac, m.label, src, approx
                    );
                }
            }
            eprintln!(
                "koannot_ledger: incoming={} outgoing={} docs={}",
                ledger.incoming.len(),
                ledger.outgoing.len(),
                ledger.docs.len()
            );
        }
        // what the device would receive: the records of the book's document
        "pull" => {
            let ann = shared.sharing.borrow().annotations();
            let Some(ann) = ann else {
                eprintln!("koannot_pull: not running");
                return;
            };
            let id: u64 = rest.parse().unwrap_or(0);
            let doc = {
                let lib = shared.lib.borrow();
                lib.get(id)
                    .and_then(|b| sync::digest_of_file(&lib.root.join(&b.file)))
            };
            let list = doc.map(|d| ann.records("ko", &d, 0)).unwrap_or_default();
            for r in list.iter().filter(|r| r.origin == ORIGIN_BOOKX) {
                eprintln!(
                    "koannot_pull: origin=bookx deleted={} note={:?} text={:?}",
                    r.annotation.deleted, r.annotation.note, r.annotation.text
                );
            }
            eprintln!("koannot_pull: total={}", list.len());
        }
        "addhl" | "editnote" | "delhl" => {
            let mut p = rest.splitn(3, ';');
            let id: u64 = p.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let text = p.next().unwrap_or("").to_string();
            let note = p.next().unwrap_or("").to_string();
            let mut lib = shared.lib.borrow_mut();
            let loc = crate::device_sync::book_locator(&lib);
            use crate::device_sync::Locator;
            let at = loc.locate(id, &text);
            let Some(book) = lib.get_mut(id) else {
                eprintln!("koannot_error: no book {id}");
                return;
            };
            match cmd {
                "addhl" => match at {
                    Some((spine, start, end)) => {
                        let hid = format!("hl-{}", uuid::Uuid::new_v4());
                        book.highlights.push(crate::core::library::Highlight {
                            id: hid,
                            spine,
                            start,
                            end,
                            color: "pink".into(),
                            style: String::new(),
                            note,
                            text,
                            created: chrono::Utc::now(),
                        });
                        eprintln!("koannot_added: ok");
                    }
                    None => eprintln!("koannot_error: text not in the book"),
                },
                "editnote" => {
                    if let Some(h) = book.highlights.iter_mut().find(|h| h.text.contains(&text)) {
                        h.note = note;
                    }
                }
                _ => book.highlights.retain(|h| !h.text.contains(&text)),
            }
            let _ = lib.save();
        }
        "plugin" => match install_plugin(Path::new(rest)) {
            Ok(d) => {
                let mut files: Vec<String> = std::fs::read_dir(&d)
                    .map(|r| {
                        r.flatten()
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                eprintln!("koannot_plugin: {} files={}", d.display(), files.join(","));
            }
            Err(e) => eprintln!("koannot_plugin_error: {e:#}"),
        },
        "plugins-folder" => {
            eprintln!(
                "koannot_plugins_folder: {:?}",
                plugins_folder(Path::new(rest)).map(|p| p.display().to_string())
            )
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-koplugin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_plugin_goes_into_the_plugins_folder_of_kobo_and_others() {
        let d = tmp();
        assert_eq!(plugins_folder(&d), None);
        std::fs::create_dir_all(d.join(".adds/koreader")).unwrap();
        assert_eq!(plugins_folder(&d), Some(d.join(".adds/koreader/plugins")));
        let e = tmp();
        std::fs::create_dir_all(e.join("koreader")).unwrap();
        assert_eq!(plugins_folder(&e), Some(e.join("koreader/plugins")));
    }

    #[test]
    fn installing_writes_every_file_and_can_be_repeated() {
        let d = tmp();
        let plugins = d.join("plugins");
        let dir = install_plugin(&plugins).unwrap();
        assert_eq!(dir, plugins.join(PLUGIN_DIR));
        for (n, _) in PLUGIN_FILES {
            assert!(dir.join(n).is_file(), "{n}");
        }
        assert!(
            std::fs::read_to_string(dir.join("main.lua"))
                .unwrap()
                .contains("Bookx Sync")
        );
        // again over the old version, no temporary files left
        install_plugin(&plugins).unwrap();
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn a_link_in_the_way_is_refused() {
        let d = tmp();
        let plugins = d.join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        let elsewhere = d.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, plugins.join(PLUGIN_DIR)).unwrap();
        assert!(install_plugin(&plugins).is_err());
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
        // a link in place of one file
        let plugins = d.join("plugins2");
        let dir = install_plugin(&plugins).unwrap();
        std::fs::remove_file(dir.join("main.lua")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("x"), dir.join("main.lua")).unwrap();
        assert!(install_plugin(&plugins).is_err());
        assert!(!elsewhere.join("x").exists());
    }

    #[test]
    fn the_built_in_copy_is_the_plugin_on_disk() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("koreader-plugin/bookx.koplugin");
        for (name, text) in PLUGIN_FILES {
            if name == "README.md" {
                continue;
            }
            assert_eq!(
                std::fs::read_to_string(src.join(name)).unwrap(),
                text,
                "{name}"
            );
        }
    }
}
