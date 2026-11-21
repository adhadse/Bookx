//! Preferences → Library → *Hardcover*: the API token (in the keyring, `Config::secret_fields`), a
//! connection test, and *Import reading status*: the lists of hardcover.app (Want to Read,
//! Currently Reading, Read, Did Not Finish) and the ratings become reading status, rating and the
//! `to-read` tag of the matching books. Read only: nothing is ever sent to Hardcover.
//!
//! The import runs as a background job (the Jobs panel shows it), then a preview says how many books
//! would change; nothing is written before *Apply*, and one *Undo* takes it all back. Logic and tests:
//! `crate::hardcover_sync` and `bookx_sources::hardcover`. The address of the service is fixed in the
//! client, the fetcher is the guarded one, the token only travels in a header.

use crate::i18n::{ntr, tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::preferences::{register_group, PrefChange, PrefCtx};
use super::SharedRef;
use crate::hardcover_sync as sync;
use bookx_sources::hardcover as hc;
use bookx_store::ReadingStatus;

struct Pending {
    updates: Vec<bookx_sources::trackers::TrackerUpdate>,
    counts: sync::Counts,
    alert: Option<adw::AlertDialog>,
}

#[derive(Default)]
struct State {
    status: String,
    last: String,
    pending: Option<Pending>,
    undo: Option<Rc<dyn Fn()>>,
    applied: usize,
    busy: bool,
}

struct Ui {
    token: adw::PasswordEntryRow,
    test_row: adw::ActionRow,
    test_btn: gtk::Button,
    import_row: adw::ActionRow,
    import_btn: gtk::Button,
    overwrite: adw::SwitchRow,
    shared: SharedRef,
    on_change: Rc<dyn Fn(PrefChange)>,
    parent: gtk::Widget,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

pub fn register() {
    register_group("library", 48, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Hardcover"));
    g.set_description(Some(&tr(
        "Reads your lists and ratings from hardcover.app and never changes your account. Copy the API token from your Hardcover account settings.",
    )));
    let token = adw::PasswordEntryRow::new();
    token.set_title(&tr("API token"));
    token.set_show_apply_button(true);
    token.set_text(&ctx.shared.cfg.borrow().web_accounts.hardcover_token);

    let test_row = adw::ActionRow::new();
    test_row.set_title(&tr("Test connection"));
    test_row.set_subtitle(&tr("Checks that Bookx can read your lists"));
    test_row.set_subtitle_lines(0);
    let test_btn = gtk::Button::with_label(&tr("Test Connection"));
    test_btn.set_valign(gtk::Align::Center);
    test_row.add_suffix(&test_btn);
    test_row.set_activatable_widget(Some(&test_btn));

    let overwrite = adw::SwitchRow::new();
    overwrite.set_title(&tr("Replace existing ratings"));
    overwrite.set_subtitle(&tr("Off: a book that has a rating keeps it"));

    let import_row = adw::ActionRow::new();
    import_row.set_title(&tr("Import reading status"));
    import_row.set_subtitle(&tr(
        "Shows what would change first. One Undo takes it all back.",
    ));
    import_row.set_subtitle_lines(0);
    let import_btn = gtk::Button::with_label(&tr("Import"));
    import_btn.set_valign(gtk::Align::Center);
    import_btn.add_css_class("suggested-action");
    import_btn.set_tooltip_text(Some(&tr(
        "Read your Hardcover lists and show what would change",
    )));
    super::audit::label(&import_btn, &tr("Import reading status from Hardcover"));
    import_row.add_suffix(&import_btn);
    import_row.set_activatable_widget(Some(&import_btn));

    g.add(&token);
    g.add(&test_row);
    g.add(&overwrite);
    g.add(&import_row);

    let ui = Rc::new(Ui {
        token: token.clone(),
        test_row,
        test_btn: test_btn.clone(),
        import_row,
        import_btn: import_btn.clone(),
        overwrite,
        shared: ctx.shared.clone(),
        on_change: ctx.on_change.clone(),
        parent: ctx.parent.clone(),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));
    {
        let ui = ui.clone();
        token.connect_apply(move |t| {
            ui.shared.cfg.borrow_mut().web_accounts.hardcover_token = t.text().trim().to_string();
            ui.shared.cfg.borrow().save();
        });
    }
    {
        let ui = ui.clone();
        test_btn.connect_clicked(move |_| test(&ui));
    }
    {
        let ui = ui.clone();
        import_btn.connect_clicked(move |_| import(&ui));
    }
    g
}

/// The token as typed (saved first, so a click on a button without Apply still uses it).
fn token_now(ui: &Ui) -> String {
    let t = ui.token.text().trim().to_string();
    {
        let mut c = ui.shared.cfg.borrow_mut();
        if c.web_accounts.hardcover_token != t {
            c.web_accounts.hardcover_token = t.clone();
            c.save();
        }
    }
    t
}

fn set_status(ui: &Ui, row: &adw::ActionRow, text: &str) {
    let _ = ui;
    row.set_subtitle(&glib::markup_escape_text(text));
    STATE.with(|s| s.borrow_mut().status = text.to_string());
}

fn busy(ui: &Ui, on: bool) {
    ui.test_btn.set_sensitive(!on);
    ui.import_btn.set_sensitive(!on);
    STATE.with(|s| s.borrow_mut().busy = on);
}

/// The reason text for the user (the engine's English sentence goes through the translation).
fn explain(e: &anyhow::Error) -> String {
    let t = format!("{e:#}");
    if t.contains("did not accept the token") {
        tr("Hardcover did not accept the token. Copy a new one from your account settings.")
    } else if t.contains("Paste your Hardcover API token") {
        tr("Paste your Hardcover API token first.")
    } else if t.contains(bookx_sources::fetch::OFFLINE_MESSAGE) {
        tr("Offline mode is on (Preferences → Privacy).")
    } else {
        trf("Could not read your lists from Hardcover: {}", &[&t])
    }
}

fn test(ui: &Rc<Ui>) {
    let token = token_now(ui);
    busy(ui, true);
    set_status(ui, &ui.test_row, &tr("Connecting"));
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(move || {
            hc::test_connection(&*crate::web_import::fetcher(), &token).map_err(|e| explain(&e))
        })
        .await
        .unwrap_or_else(|_| Err(tr("The test stopped unexpectedly.")));
        busy(&ui, false);
        let text = match r {
            Ok(_) => tr("Connected. Your lists can be read."),
            Err(e) => e,
        };
        set_status(&ui, &ui.test_row, &text);
    });
}

fn current_status(shared: &SharedRef) -> impl Fn(u64) -> ReadingStatus + '_ {
    move |id| {
        shared
            .guard
            .borrow()
            .store
            .get_status(id)
            .unwrap_or(ReadingStatus::New)
    }
}

fn import(ui: &Rc<Ui>) {
    let token = token_now(ui);
    busy(ui, true);
    set_status(ui, &ui.import_row, &tr("Reading your lists"));
    let (job, _) = crate::jobs::start(
        crate::jobs::Kind::Import,
        &tr("Reading status from Hardcover"),
        false,
    );
    crate::jobs::progress(job, None, &tr("Reading your lists"));
    let ui = ui.clone();
    glib::spawn_future_local(async move {
        let r = gio::spawn_blocking(move || {
            hc::fetch_items(&*crate::web_import::fetcher(), &token).map_err(|e| explain(&e))
        })
        .await
        .unwrap_or_else(|_| Err(tr("The import stopped unexpectedly.")));
        busy(&ui, false);
        match r {
            Err(e) => {
                crate::jobs::finish(job, Err(e.clone()));
                set_status(&ui, &ui.import_row, &e);
            }
            Ok(items) => {
                crate::jobs::log(
                    job,
                    &ntr(
                        "{} book on your lists",
                        "{} books on your lists",
                        items.len(),
                    ),
                );
                crate::jobs::finish(job, Ok(()));
                preview(&ui, &items);
            }
        }
    });
}

fn preview(ui: &Rc<Ui>, items: &[hc::Item]) {
    let (updates, counts) = {
        let lib = ui.shared.lib.borrow();
        let u = sync::plan(
            items,
            &lib.books,
            &current_status(&ui.shared),
            ui.overwrite.is_active(),
        );
        let c = sync::counts(items, &lib.books, &u);
        (u, c)
    };
    let heading = if counts.changes == 0 {
        tr("Nothing to change")
    } else {
        ntr(
            "{} book would change",
            "{} books would change",
            counts.changes,
        )
    };
    let body = trf(
        "{} on your Hardcover lists, {} found in your library. Reading status for {}, ratings for {}.",
        &[
            &ntr("{} book", "{} books", counts.items),
            &counts.matched,
            &ntr("{} book", "{} books", counts.statuses),
            &ntr("{} book", "{} books", counts.ratings),
        ],
    );
    set_status(ui, &ui.import_row, &heading);
    let alert = adw::AlertDialog::new(Some(&heading), Some(&body));
    alert.add_response("cancel", &tr("Cancel"));
    if counts.changes > 0 {
        alert.add_response("apply", &tr("Apply"));
        alert.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
        alert.set_default_response(Some("apply"));
    }
    alert.set_close_response("cancel");
    STATE.with(|s| {
        s.borrow_mut().pending = Some(Pending {
            updates,
            counts,
            alert: Some(alert.clone()),
        })
    });
    {
        let ui = ui.clone();
        alert.connect_response(None, move |_, id| {
            if id == "apply" {
                apply(&ui);
            } else {
                STATE.with(|s| s.borrow_mut().pending = None);
            }
        });
    }
    alert.present(Some(&ui.parent));
}

fn apply(ui: &Rc<Ui>) {
    let Some(p) = STATE.with(|s| s.borrow_mut().pending.take()) else {
        return;
    };
    let shared = &ui.shared;
    let res = {
        let guard = shared.guard.borrow();
        sync::apply(&mut shared.lib.borrow_mut(), &guard.store, &p.updates)
    };
    match res {
        Err(e) => {
            let t = trf("Could not apply: {}", &[&format!("{e:#}")]);
            set_status(ui, &ui.import_row, &t);
            super::preferences::toast(&t);
        }
        Ok((a, token)) => {
            let n = a.books;
            let msg = ntr(
                "Updated {} book from Hardcover",
                "Updated {} books from Hardcover",
                n,
            );
            set_status(ui, &ui.import_row, &msg);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.applied = n;
                s.last = msg.clone();
            });
            (ui.on_change)(PrefChange::Layout(msg.clone()));
            let undo: Rc<dyn Fn()> = {
                let (ui, token) = (ui.clone(), Rc::new(RefCell::new(Some(token))));
                Rc::new(move || {
                    let Some(t) = token.borrow_mut().take() else {
                        return;
                    };
                    let r = {
                        let guard = ui.shared.guard.borrow();
                        sync::undo(&mut ui.shared.lib.borrow_mut(), &guard.store, &t)
                    };
                    let text = match r {
                        Ok(_) => tr("Hardcover import undone"),
                        Err(e) => trf("Could not undo: {}", &[&format!("{e:#}")]),
                    };
                    set_status(&ui, &ui.import_row, &text);
                    STATE.with(|s| s.borrow_mut().last = text.clone());
                    (ui.on_change)(PrefChange::Layout(text));
                })
            };
            STATE.with(|s| s.borrow_mut().undo = Some(undo.clone()));
            let undo2 = undo.clone();
            super::preferences::toast_undo(&msg, 10, move || undo2(), || {});
        }
    }
}

/// `prefs:hardcover-…` of the developer script. The token is never printed.
///
/// * `set=TOKEN` types the token and applies it, `test`, `import` (reads the lists, then the preview opens)
/// * `overwrite=on|off`, `apply` (Apply of the preview), `cancel`, `undo`
/// * `dump` prints `prefs_hardcover: token_set=B busy=B pending=N statuses=N ratings=N matched=N items=N applied=N status="…" last="…" alert=B`
pub fn dev(arg: &str) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else {
        eprintln!("prefs_hardcover: no page");
        return;
    };
    match arg.split_once('=').unwrap_or((arg, "")) {
        ("set", t) => {
            ui.token.set_text(t);
            ui.token.emit_by_name::<()>("apply", &[]);
        }
        ("test", _) => ui.test_btn.emit_clicked(),
        ("import", _) => ui.import_btn.emit_clicked(),
        ("overwrite", v) => ui.overwrite.set_active(v == "on"),
        ("apply", _) => {
            let alert = STATE.with(|s| s.borrow().pending.as_ref().and_then(|p| p.alert.clone()));
            if let Some(a) = alert {
                a.emit_by_name::<()>("response", &[&"apply"]);
                a.force_close();
            }
        }
        ("cancel", _) => {
            let alert = STATE.with(|s| s.borrow().pending.as_ref().and_then(|p| p.alert.clone()));
            if let Some(a) = alert {
                a.force_close();
            }
            STATE.with(|s| s.borrow_mut().pending = None);
        }
        ("undo", _) => {
            if let Some(u) = STATE.with(|s| s.borrow_mut().undo.take()) {
                u();
            }
        }
        _ => STATE.with(|s| {
            let s = s.borrow();
            let (p, c) = s.pending.as_ref().map_or((0, sync::Counts::default()), |p| {
                (p.updates.len(), p.counts.clone())
            });
            eprintln!(
                "prefs_hardcover: token_set={} busy={} pending={p} statuses={} ratings={} matched={} items={} applied={} status={:?} last={:?} alert={}",
                !ui.shared.cfg.borrow().web_accounts.hardcover_token.is_empty(),
                s.busy,
                c.statuses,
                c.ratings,
                c.matched,
                c.items,
                s.applied,
                s.status,
                s.last,
                s.pending.as_ref().is_some_and(|p| p.alert.is_some()),
            );
        }),
    }
}
