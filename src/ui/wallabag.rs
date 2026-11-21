//! Import from Wallabag: the saved articles of a Wallabag server become books (read-later list).
//! Opened from “Add from the Web”. The network and the parsing live in `bookx_sources::wallabag`
//! and `web_import::import_wallabag`; this file is the form and the hand-over of the books to the
//! library. Secrets (client secret, password, token) are saved through the keyring like every other
//! password (`Config::secret_fields`).

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::core::library::Prepared;
use crate::i18n::tr;

type Add = Rc<dyn Fn(Result<Prepared, String>)>;

struct Form {
    url: adw::EntryRow,
    client_id: adw::EntryRow,
    client_secret: adw::PasswordEntryRow,
    user: adw::EntryRow,
    password: adw::PasswordEntryRow,
    token: adw::PasswordEntryRow,
    include_read: adw::SwitchRow,
    tag: adw::EntryRow,
    status: gtk::Label,
    button: gtk::Button,
}

thread_local! {
    static ADD: RefCell<Option<Add>> = const { RefCell::new(None) };
    static FORM: RefCell<Option<Rc<Form>>> = const { RefCell::new(None) };
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The content of the “Add from the Web” dialog: the address box and, under it, a button for
/// Wallabag. `add` takes each book made.
pub fn extra_child(
    entry: &gtk::Entry,
    parent: &gtk::Window,
    shared: &SharedRef,
    add: Add,
) -> gtk::Widget {
    ADD.with(|a| *a.borrow_mut() = Some(add));
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.append(entry);
    let link = gtk::Button::with_label(&tr("Import from Wallabag"));
    link.add_css_class("flat");
    link.set_halign(gtk::Align::Start);
    link.set_widget_name("add-web-wallabag");
    let (parent, shared) = (parent.clone(), shared.clone());
    link.connect_clicked(move |b| {
        if let Some(d) = b
            .ancestor(adw::AlertDialog::static_type())
            .and_downcast::<adw::AlertDialog>()
        {
            d.force_close();
        }
        show(&parent, &shared);
    });
    b.append(&link);
    b.upcast()
}

fn entry_row(title: &str, text: &str) -> adw::EntryRow {
    let r = adw::EntryRow::new();
    r.set_title(title);
    r.set_text(text);
    r
}

fn password_row(title: &str, text: &str) -> adw::PasswordEntryRow {
    let r = adw::PasswordEntryRow::new();
    r.set_title(title);
    r.set_text(text);
    r
}

/// Applies the form to the saved settings (not the imported ids).
fn read_form(f: &Form, shared: &SharedRef) -> crate::web_accounts::WebAccounts {
    let mut c = shared.cfg.borrow_mut();
    let a = &mut c.web_accounts;
    a.wallabag_url = f.url.text().trim().to_string();
    a.wallabag_client_id = f.client_id.text().trim().to_string();
    a.wallabag_client_secret = f.client_secret.text().to_string();
    a.wallabag_user = f.user.text().trim().to_string();
    a.wallabag_password = f.password.text().to_string();
    a.wallabag_token = f.token.text().trim().to_string();
    a.wallabag_include_read = f.include_read.is_active();
    a.wallabag_tag = f.tag.text().trim().to_string();
    let out = a.clone();
    c.save();
    out
}

fn run_import(f: &Rc<Form>, shared: &SharedRef) {
    let acc = read_form(f, shared);
    f.button.set_sensitive(false);
    f.status.set_label(&tr("Importing"));
    f.status.remove_css_class("error");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = crate::web_import::import_wallabag(&acc, &*crate::web_import::fetcher())
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    let (f, shared) = (f.clone(), shared.clone());
    glib::timeout_add_local(std::time::Duration::from_millis(60), move || {
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(_) => f.button.set_sensitive(true),
            Ok(Err(e)) => {
                f.status.set_label(&e);
                f.status.add_css_class("error");
                LAST.with(|l| *l.borrow_mut() = format!("error: {e}"));
                f.button.set_sensitive(true);
            }
            Ok(Ok(run)) => {
                let ids: Vec<u64> = run.books.iter().map(|b| b.0).collect();
                let n = run.books.len();
                if let Some(add) = ADD.with(|a| a.borrow().clone()) {
                    for (_, p) in run.books {
                        add(Ok(p));
                    }
                }
                {
                    let mut c = shared.cfg.borrow_mut();
                    c.web_accounts.remember_wallabag(&ids);
                    c.save();
                }
                let mut line = crate::web_import::wallabag_summary(n, run.problems.len());
                if let Some(p) = run.problems.first() {
                    line.push_str(&format!(" ({p})"));
                }
                LAST.with(|l| {
                    *l.borrow_mut() = format!("ok: {n} added, {} problems", run.problems.len())
                });
                f.status.set_label(&line);
                f.button.set_sensitive(true);
            }
        }
        glib::ControlFlow::Break
    });
}

/// Opens the Wallabag form over `parent`.
pub fn show(parent: &gtk::Window, shared: &SharedRef) {
    let acc = shared.cfg.borrow().web_accounts.clone();
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Import from Wallabag"));
    dialog.set_content_width(520);
    dialog.set_content_height(620);
    let page = adw::PreferencesPage::new();

    let server = adw::PreferencesGroup::new();
    server.set_title(&tr("Wallabag server"));
    server.set_description(Some(&tr(
        "The address of your server, and either an access token or the API client (Wallabag → API clients management) with your user name and password.",
    )));
    let url = entry_row(&tr("Address"), &acc.wallabag_url);
    let token = password_row(&tr("Access token"), &acc.wallabag_token);
    let client_id = entry_row(&tr("Client ID"), &acc.wallabag_client_id);
    let client_secret = password_row(&tr("Client secret"), &acc.wallabag_client_secret);
    let user = entry_row(&tr("User name"), &acc.wallabag_user);
    let password = password_row(&tr("Password"), &acc.wallabag_password);
    for r in [&url, &client_id, &user] {
        server.add(r);
    }
    for r in [&client_secret, &password, &token] {
        server.add(r);
    }
    let what = adw::PreferencesGroup::new();
    what.set_title(&tr("Articles"));
    let include_read = adw::SwitchRow::new();
    include_read.set_title(&tr("Include articles already read"));
    include_read.set_subtitle(&tr("Otherwise only the unread ones are added"));
    include_read.set_active(acc.wallabag_include_read);
    let tag = entry_row(&tr("Only this tag (optional)"), &acc.wallabag_tag);
    what.add(&include_read);
    what.add(&tag);
    let go = adw::PreferencesGroup::new();
    let button = gtk::Button::with_label(&tr("Import"));
    button.add_css_class("suggested-action");
    button.add_css_class("pill");
    button.set_halign(gtk::Align::Center);
    button.set_widget_name("wallabag-import");
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    status.set_selectable(true);
    status.set_margin_top(8);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
    inner.append(&button);
    inner.append(&status);
    go.add(&inner);
    page.add(&server);
    page.add(&what);
    page.add(&go);

    let form = Rc::new(Form {
        url,
        client_id,
        client_secret,
        user,
        password,
        token,
        include_read,
        tag,
        status,
        button: button.clone(),
    });
    {
        let (form, shared) = (form.clone(), shared.clone());
        button.connect_clicked(move |_| run_import(&form, &shared));
    }
    FORM.with(|f| *f.borrow_mut() = Some(form));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// `wallabag:<command>` in `BOOKX_DEV_SCRIPT`: `open`, `set=URL;TOKEN;TAG`, `import`, `dump`
/// (prints `wallabag: seen=N last=..`).
pub fn dev_command(shared: &SharedRef, window: &gtk::Window, arg: &str) {
    if arg == "open" {
        show(window, shared);
        return;
    }
    let form = FORM.with(|f| f.borrow().clone());
    let Some(f) = form else {
        eprintln!("wallabag: no form");
        return;
    };
    if let Some(v) = arg.strip_prefix("set=") {
        let mut it = v.splitn(3, ';');
        f.url.set_text(it.next().unwrap_or(""));
        f.token.set_text(it.next().unwrap_or(""));
        f.tag.set_text(it.next().unwrap_or(""));
    } else if arg == "import" {
        run_import(&f, shared);
    } else if arg == "dump" {
        eprintln!(
            "wallabag: seen={} last={} status={}",
            shared.cfg.borrow().web_accounts.wallabag_seen.len(),
            LAST.with(|l| l.borrow().clone()),
            f.status.label()
        );
    }
}
