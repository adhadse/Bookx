//! Preferences → Sync → Advanced → *Library server accounts*: the logins of the library server (OPDS
//! catalogue and web pages) with the collections each may see, and the guests. A row opens a window
//! with the list, *Add Account*, edit and remove (with Undo), and the guest switch.
//!
//! Passwords are hashed with Argon2id (`account_model::hash_password`, the call `bookx-cli serve
//! --account` makes) on a worker thread; only the hash reaches the settings, the entries are emptied
//! as soon as the hash is made, and nothing here prints a password or a hash. A change is saved and
//! the running server is restarted at once (the saved settings are read at start); while the server is
//! off the toast says it takes effect at the next start. Logic and tests: `crate::account_model`.

use crate::i18n::{ntr, tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::SharedRef;
use crate::account_model as model;
use crate::sharing::SharedAccount;

thread_local! {
    static SHARED: RefCell<Option<SharedRef>> = const { RefCell::new(None) };
    static ROW: RefCell<Option<glib::WeakRef<adw::ActionRow>>> = const { RefCell::new(None) };
    static MANAGE: RefCell<Option<Rc<Manage>>> = const { RefCell::new(None) };
    static FORM: RefCell<Option<Rc<Form>>> = const { RefCell::new(None) };
    static UNDO: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static RESTARTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn subtitle(n: usize) -> String {
    ntr("{} account", "{} accounts", n)
}

/// The row of Preferences → Sync → Advanced.
pub fn rows(shared: &SharedRef) -> Vec<gtk::Widget> {
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    let row = adw::ActionRow::new();
    row.set_title(&tr("Library server accounts"));
    row.set_subtitle(&trf(
        "{}. Own logins that see only chosen collections, and guests.",
        &[&subtitle(shared.cfg.borrow().share.accounts.len())],
    ));
    row.set_subtitle_lines(0);
    row.set_title_lines(0);
    let manage = gtk::Button::with_label(&tr("Manage"));
    manage.set_valign(gtk::Align::Center);
    manage.set_tooltip_text(Some(&tr("Manage the accounts of the library server")));
    super::audit::label(&manage, &tr("Manage library server accounts"));
    row.add_suffix(&manage);
    row.set_activatable_widget(Some(&manage));
    {
        let shared = shared.clone();
        manage.connect_clicked(move |b| show(b.upcast_ref(), &shared));
    }
    ROW.with(|r| *r.borrow_mut() = Some(row.downgrade()));
    vec![row.upcast()]
}

fn refresh_row(shared: &SharedRef) {
    ROW.with(|r| {
        if let Some(row) = r.borrow().as_ref().and_then(|w| w.upgrade()) {
            row.set_subtitle(&trf(
                "{}. Own logins that see only chosen collections, and guests.",
                &[&subtitle(shared.cfg.borrow().share.accounts.len())],
            ));
        }
    });
}

/// Saves the settings and makes the running server read them. Returns what to tell the user.
fn commit(shared: &SharedRef) -> Result<String, String> {
    shared.cfg.borrow().save();
    refresh_row(shared);
    match super::share::reapply(shared) {
        Some(Ok(_)) => {
            RESTARTS.with(|r| r.set(r.get() + 1));
            Ok(tr("Saved. The server restarted, so this applies now."))
        }
        Some(Err(e)) => Err(trf("Saved, but the server could not restart: {}", &[&e])),
        None => Ok(tr("Saved. This applies when the server starts.")),
    }
}

fn say(toasts: &adw::ToastOverlay, r: Result<String, String>) {
    LAST.with(|l| *l.borrow_mut() = r.clone().unwrap_or_else(|e| e));
    match r {
        Ok(t) => super::dialog_toast::show_plain(toasts, &t),
        Err(e) => super::dialog_toast::show_error(toasts, &e),
    }
}

// ---- the list ---------------------------------------------------------------------------------

struct Manage {
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    accounts: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    guests: adw::SwitchRow,
    guest_row: adw::ActionRow,
    guest_choose: gtk::Button,
    add: gtk::Button,
    /// The remove and edit buttons by account name, for the tests.
    buttons: RefCell<Vec<(String, gtk::Button, gtk::Button)>>,
    syncing: std::cell::Cell<bool>,
    shared: SharedRef,
}

fn show(parent: &gtk::Widget, shared: &SharedRef) {
    let page = adw::PreferencesPage::new();
    let accounts = adw::PreferencesGroup::new();
    accounts.set_title(&tr("Accounts"));
    accounts.set_description(Some(&tr(
        "Each account has its own password and sees only the collections you choose. The shared library password keeps seeing everything. Changes restart the running server.",
    )));
    let add = gtk::Button::with_label(&tr("Add Account"));
    add.add_css_class("suggested-action");
    add.set_valign(gtk::Align::Center);
    add.set_widget_name("accounts-add");
    accounts.set_header_suffix(Some(&add));
    page.add(&accounts);

    let guest_group = adw::PreferencesGroup::new();
    guest_group.set_title(&tr("Guests"));
    let guests = adw::SwitchRow::new();
    guests.set_title(&tr("Allow guests"));
    guests.set_subtitle(&tr(
        "Visitors without a login can read the collections below, and nothing else",
    ));
    let guest_row = adw::ActionRow::new();
    guest_row.set_title(&tr("Guest collections"));
    guest_row.set_subtitle_lines(0);
    let guest_choose = gtk::Button::with_label(&tr("Choose"));
    guest_choose.set_valign(gtk::Align::Center);
    guest_choose.set_tooltip_text(Some(&tr("Choose the collections guests can read")));
    super::audit::label(&guest_choose, &tr("Choose guest collections"));
    guest_row.add_suffix(&guest_choose);
    guest_group.add(&guests);
    guest_group.add(&guest_row);
    page.add(&guest_group);

    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Library server accounts"));
    dialog.set_content_width(520);
    dialog.set_content_height(560);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let toasts = super::dialog_toast::overlay(&view);
    dialog.set_child(Some(&toasts));

    let m = Rc::new(Manage {
        dialog: dialog.clone(),
        toasts,
        accounts,
        rows: RefCell::new(Vec::new()),
        guests,
        guest_row,
        guest_choose,
        add: add.clone(),
        buttons: RefCell::new(Vec::new()),
        syncing: std::cell::Cell::new(false),
        shared: shared.clone(),
    });
    fill(&m);
    {
        let m = m.clone();
        add.connect_clicked(move |_| edit(&m, None));
    }
    {
        let m = m.clone();
        m.guests.clone().connect_active_notify(move |sw| {
            if m.syncing.get() {
                return;
            }
            if sw.is_active() {
                if m.shared.cfg.borrow().share.guest_collections.is_empty() {
                    choose_guests(&m);
                }
            } else if !m.shared.cfg.borrow().share.guest_collections.is_empty() {
                model::set_guests(&mut m.shared.cfg.borrow_mut().share, &[]);
                say(&m.toasts, commit(&m.shared));
                fill(&m);
            }
        });
    }
    {
        let m = m.clone();
        m.guest_choose
            .clone()
            .connect_clicked(move |_| choose_guests(&m));
    }
    MANAGE.with(|c| *c.borrow_mut() = Some(m.clone()));
    dialog.connect_closed(|_| MANAGE.with(|c| *c.borrow_mut() = None));
    dialog.present(Some(parent));
}

/// Builds the rows again from the settings.
fn fill(m: &Rc<Manage>) {
    for r in m.rows.borrow_mut().drain(..) {
        m.accounts.remove(&r);
    }
    m.buttons.borrow_mut().clear();
    let list = m.shared.cfg.borrow().share.accounts.clone();
    if list.is_empty() {
        let r = super::preferences::info_row(
            &tr("No accounts yet"),
            &tr("Press Add Account to give someone their own login."),
        );
        m.accounts.add(&r);
        m.rows.borrow_mut().push(r);
    }
    for a in list {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&a.name));
        row.set_subtitle(&glib::markup_escape_text(&model::access_text(
            &a.collections,
        )));
        row.set_subtitle_lines(0);
        let edit_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        let rm = gtk::Button::from_icon_name("user-trash-symbolic");
        for (b, text) in [
            (&edit_btn, trf("Edit account {}", &[&a.name])),
            (&rm, trf("Remove account {}", &[&a.name])),
        ] {
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&text));
            super::audit::label(b, &text);
            row.add_suffix(b);
        }
        {
            let (m, a) = (m.clone(), a.clone());
            edit_btn.connect_clicked(move |_| edit(&m, Some(a.clone())));
        }
        {
            let (m, name) = (m.clone(), a.name.clone());
            rm.connect_clicked(move |_| remove(&m, &name));
        }
        m.buttons.borrow_mut().push((a.name.clone(), edit_btn, rm));
        m.accounts.add(&row);
        m.rows.borrow_mut().push(row);
    }
    // the guests
    let guests = m.shared.cfg.borrow().share.guest_collections.clone();
    m.syncing.set(true);
    m.guests.set_active(!guests.is_empty());
    m.syncing.set(false);
    m.guest_row.set_visible(!guests.is_empty());
    m.guest_row
        .set_subtitle(&glib::markup_escape_text(&guests.join(", ")));
}

fn remove(m: &Rc<Manage>, name: &str) {
    let taken = model::remove(&mut m.shared.cfg.borrow_mut().share, name);
    let Some((index, acc)) = taken else { return };
    say(&m.toasts, commit(&m.shared));
    fill(m);
    let stash = Rc::new(RefCell::new(Some((index, acc))));
    let undo: Rc<dyn Fn()> = {
        let (m, stash) = (m.clone(), stash.clone());
        Rc::new(move || {
            if let Some((i, a)) = stash.borrow_mut().take() {
                model::restore(&mut m.shared.cfg.borrow_mut().share, i, a);
                say(&m.toasts, commit(&m.shared));
                fill(&m);
            }
        })
    };
    UNDO.with(|u| *u.borrow_mut() = Some(undo.clone()));
    super::dialog_toast::show_undo(
        &m.toasts,
        &trf("Account “{}” removed", &[&name]),
        move || undo(),
    );
}

fn choose_guests(m: &Rc<Manage>) {
    let current = m.shared.cfg.borrow().share.guest_collections.clone();
    let m2 = m.clone();
    pick_collections(
        &m.dialog,
        &m.shared,
        &tr("Guest collections"),
        &tr("Guests can read books with any of the chosen tags. Choose at least one, or guests stay off."),
        &current,
        Rc::new(move |chosen| {
            model::set_guests(&mut m2.shared.cfg.borrow_mut().share, &chosen);
            say(&m2.toasts, commit(&m2.shared));
            fill(&m2);
        }),
        Rc::new({
            let m = m.clone();
            move || fill(&m)
        }),
    );
}

// ---- choosing collections ---------------------------------------------------------------------

struct Choices {
    group: adw::PreferencesGroup,
    rows: Vec<(String, adw::SwitchRow)>,
}

impl Choices {
    fn new(shared: &SharedRef, selected: &[String]) -> Choices {
        let group = adw::PreferencesGroup::new();
        let books = shared.lib.borrow().books.clone();
        let wanted = model::clean_collections(selected);
        let mut rows = Vec::new();
        let choices = model::choices_with(selected, &books);
        if choices.is_empty() {
            group.add(&super::preferences::info_row(
                &tr("No tags yet"),
                &tr(
                    "Collections are the tags of your books. Add tags to books to offer them here.",
                ),
            ));
        }
        for (tag, n) in choices {
            let row = adw::SwitchRow::new();
            row.set_title(&glib::markup_escape_text(&tag));
            row.set_subtitle(&ntr("{} book", "{} books", n));
            row.set_active(wanted.contains(&tag));
            group.add(&row);
            rows.push((tag, row));
        }
        Choices { group, rows }
    }

    fn chosen(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|(_, r)| r.is_active())
            .map(|(t, _)| t.clone())
            .collect()
    }

    fn set(&self, tags: &[String]) {
        let tags = model::clean_collections(tags);
        for (t, r) in &self.rows {
            r.set_active(tags.contains(t));
        }
    }
}

/// A small window to pick tags. `done` gets the choice when *Done* is pressed with at least one
/// tag; `cancelled` runs otherwise (so the guest switch can fall back).
fn pick_collections(
    parent: &adw::Dialog,
    shared: &SharedRef,
    title: &str,
    help: &str,
    selected: &[String],
    done: Rc<dyn Fn(Vec<String>)>,
    cancelled: Rc<dyn Fn()>,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(title);
    dialog.set_content_width(440);
    dialog.set_content_height(480);
    let choices = Rc::new(Choices::new(shared, selected));
    choices.group.set_description(Some(help));
    let page = adw::PreferencesPage::new();
    page.add(&choices.group);
    let ok = gtk::Button::with_label(&tr("Done"));
    ok.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&ok);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    let accepted = Rc::new(std::cell::Cell::new(false));
    {
        let (choices, dialog, accepted) = (choices.clone(), dialog.clone(), accepted.clone());
        ok.connect_clicked(move |_| {
            let c = choices.chosen();
            if c.is_empty() {
                return;
            }
            accepted.set(true);
            done(c);
            dialog.close();
        });
    }
    {
        let ok2 = ok.clone();
        for (_, r) in &choices.rows {
            let (choices, ok2) = (choices.clone(), ok2.clone());
            r.connect_active_notify(move |_| ok2.set_sensitive(!choices.chosen().is_empty()));
        }
        ok.set_sensitive(!choices.chosen().is_empty());
    }
    dialog.connect_closed(move |_| {
        if !accepted.get() {
            cancelled();
        }
    });
    dialog.present(Some(parent));
}

// ---- add and edit -----------------------------------------------------------------------------

struct Form {
    dialog: adw::Dialog,
    name: adw::EntryRow,
    password: adw::PasswordEntryRow,
    confirm: adw::PasswordEntryRow,
    choices: Choices,
    save: gtk::Button,
    banner: adw::Banner,
}

fn edit(m: &Rc<Manage>, existing: Option<SharedAccount>) {
    let editing = existing.is_some();
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&if editing {
        tr("Edit Account")
    } else {
        tr("Add Account")
    });
    dialog.set_content_width(480);
    dialog.set_content_height(560);

    let basics = adw::PreferencesGroup::new();
    let name = adw::EntryRow::new();
    name.set_title(&tr("Name"));
    name.set_max_length(model::MAX_NAME as i32);
    let password = adw::PasswordEntryRow::new();
    password.set_title(&if editing {
        tr("New password, empty keeps the old one")
    } else {
        tr("Password")
    });
    let confirm = adw::PasswordEntryRow::new();
    confirm.set_title(&tr("Repeat the password"));
    basics.add(&name);
    basics.add(&password);
    basics.add(&confirm);
    if let Some(a) = &existing {
        name.set_text(&a.name);
    }
    let selected = existing
        .as_ref()
        .map(|a| a.collections.clone())
        .unwrap_or_default();
    let choices = Choices::new(&m.shared, &selected);
    choices.group.set_title(&tr("Allowed collections"));
    choices.group.set_description(Some(&tr(
        "Collections are tags. This account sees books with any chosen tag. Choose none for all books.",
    )));
    let page = adw::PreferencesPage::new();
    page.add(&basics);
    page.add(&choices.group);

    let save = gtk::Button::with_label(&if editing { tr("Save") } else { tr("Add") });
    save.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let banner = adw::Banner::new("");
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&banner);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&save));

    let form = Rc::new(Form {
        dialog: dialog.clone(),
        name,
        password,
        confirm,
        choices,
        save: save.clone(),
        banner,
    });
    FORM.with(|f| *f.borrow_mut() = Some(form.clone()));
    {
        let (m, form) = (m.clone(), form.clone());
        let old = existing.map(|a| a.name);
        save.connect_clicked(move |_| submit(&m, &form, old.clone()));
    }
    dialog.connect_closed(|_| FORM.with(|f| *f.borrow_mut() = None));
    dialog.present(Some(&m.dialog));
}

fn problem(form: &Form, text: &str) {
    form.banner.set_title(text);
    form.banner.set_revealed(true);
}

/// Checks the form, hashes the password off the main thread and stores the account.
fn submit(m: &Rc<Manage>, form: &Rc<Form>, old: Option<String>) {
    let list = m.shared.cfg.borrow().share.accounts.clone();
    let name = match model::check_name(form.name.text().as_str(), &list, old.as_deref()) {
        Ok(n) => n,
        Err(e) => return problem(form, &e),
    };
    let pw = form.password.text().to_string();
    let confirm = form.confirm.text().to_string();
    let need_password = old.is_none() || !pw.is_empty() || !confirm.is_empty();
    if need_password {
        if let Err(e) = model::check_password(&pw, &confirm) {
            return problem(form, &e);
        }
    }
    let collections = form.choices.chosen();
    form.banner.set_revealed(false);
    form.save.set_sensitive(false);
    // the entries are emptied now: the password lives only in the closure that hashes it
    form.password.set_text("");
    form.confirm.set_text("");
    let (m, form) = (m.clone(), form.clone());
    glib::spawn_future_local(async move {
        let hash = if need_password {
            gio::spawn_blocking(move || model::hash_password(&pw))
                .await
                .ok()
        } else {
            None
        };
        let result = if need_password && hash.is_none() {
            Err(tr("The password could not be prepared. Try again."))
        } else {
            let mut cfg = m.shared.cfg.borrow_mut();
            match &old {
                None => model::add(
                    &mut cfg.share,
                    &name,
                    hash.as_deref().unwrap_or_default(),
                    &collections,
                ),
                Some(o) => model::edit(&mut cfg.share, o, &name, hash.as_deref(), &collections),
            }
        };
        form.save.set_sensitive(true);
        match result {
            Ok(()) => {
                let said = commit(&m.shared);
                form.dialog.close();
                say(&m.toasts, said);
                fill(&m);
            }
            Err(e) => problem(&form, &e),
        }
    });
}

// ---- developer hooks --------------------------------------------------------------------------

/// `prefs:accounts-…` of the developer script. Nothing here prints a password or a hash.
///
/// * `open` the window, `close` it; `new` presses *Add Account*; `edit=NAME`, `remove=NAME`, `undo`
/// * `fill=NAME;PASSWORD;REPEAT;tag+tag` fills the open form (tags are switched on), `save` presses it
/// * `guests=tag+tag` sets the guests (empty turns them off)
/// * `verify=NAME;PASSWORD` prints whether that login would be accepted
/// * `dump` prints `prefs_accounts: count=N accounts=name[tags]hashed=B,… guests=… restarts=N window=B form=B rows=N last="…"`
pub fn dev(arg: &str) {
    let shared = SHARED.with(|s| s.borrow().clone());
    let manage = MANAGE.with(|m| m.borrow().clone());
    let form = FORM.with(|f| f.borrow().clone());
    match arg.split_once('=').unwrap_or((arg, "")) {
        ("open", _) => {
            if let Some(s) = shared {
                if let Some(w) = ROW.with(|r| r.borrow().as_ref().and_then(|w| w.upgrade())) {
                    show(w.upcast_ref(), &s);
                }
            }
        }
        ("close", _) => {
            if let Some(m) = manage {
                m.dialog.close();
            }
        }
        ("new", _) => {
            if let Some(m) = manage {
                m.add.emit_clicked();
            }
        }
        ("edit", n) | ("remove", n) => {
            if let Some(m) = manage {
                let btn = m
                    .buttons
                    .borrow()
                    .iter()
                    .find(|(name, _, _)| name == n)
                    .map(|(_, e, r)| {
                        if arg.starts_with("edit") {
                            e.clone()
                        } else {
                            r.clone()
                        }
                    });
                if let Some(b) = btn {
                    b.emit_clicked();
                }
            }
        }
        ("undo", _) => {
            if let Some(u) = UNDO.with(|u| u.borrow_mut().take()) {
                u();
            }
        }
        ("fill", v) => {
            if let Some(f) = form {
                let mut it = v.splitn(4, ';');
                f.name.set_text(it.next().unwrap_or(""));
                f.password.set_text(it.next().unwrap_or(""));
                f.confirm.set_text(it.next().unwrap_or(""));
                let tags: Vec<String> = it
                    .next()
                    .unwrap_or("")
                    .split('+')
                    .map(str::to_string)
                    .collect();
                f.choices.set(&tags);
            }
        }
        ("save", _) => {
            if let Some(f) = form {
                f.save.emit_clicked();
            }
        }
        ("guests", v) => {
            if let Some(m) = manage {
                let tags: Vec<String> = v.split('+').map(str::to_string).collect();
                model::set_guests(&mut m.shared.cfg.borrow_mut().share, &tags);
                say(&m.toasts, commit(&m.shared));
                fill(&m);
            }
        }
        ("verify", v) => {
            let (n, pw) = v.split_once(';').unwrap_or((v, ""));
            let ok = shared.is_some_and(|s| {
                s.cfg
                    .borrow()
                    .share
                    .accounts
                    .iter()
                    .any(|a| a.name == n && bookx_serve::keyhash::verify(&a.name, &a.hash, pw))
            });
            eprintln!("prefs_accounts_verify: name={n} ok={ok}");
        }
        ("dump", _) => {
            let (summary, rows) = match (&shared, &manage) {
                (Some(s), m) => (
                    model::dump(&s.cfg.borrow().share),
                    m.as_ref().map_or(0, |m| m.rows.borrow().len()),
                ),
                _ => ("count=0 accounts= guests=".to_string(), 0),
            };
            eprintln!(
                "prefs_accounts: {summary} restarts={} window={} form={} rows={rows} last={:?}",
                RESTARTS.with(|r| r.get()),
                manage.is_some(),
                form.is_some(),
                LAST.with(|l| l.borrow().clone())
            );
        }
        _ => {}
    }
}
