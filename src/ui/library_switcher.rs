//! The library switcher in the library header: a button with the name of the library in use and a
//! popover listing the others (switch, rename, remove from the list), with *New Library…* and
//! *Open Folder…*. The list is `bookx_store::LibraryRegistry` in `libraries.json`; the pure parts
//! are in `crate::libraries`. Switching goes through the same path as the folder in Preferences,
//! so the library lock and the read-only notice work the same way.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use bookx_store::LibraryRegistry;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use crate::libraries::{self, Row};

type Switch = Rc<dyn Fn(PathBuf)>;
type Toast = Rc<dyn Fn(String)>;

thread_local! {
    static CURRENT: RefCell<Option<glib::WeakRef<gtk::MenuButton>>> = const { RefCell::new(None) };
    static LABEL: RefCell<Option<glib::WeakRef<gtk::Label>>> = const { RefCell::new(None) };
}

/// What the switcher needs from the window.
#[derive(Clone)]
pub struct Ctx {
    pub switch: Switch,
    pub toast: Toast,
}

fn registry() -> Option<LibraryRegistry> {
    crate::config::libraries_path().and_then(|p| LibraryRegistry::open(&p).ok())
}

/// Puts the library in use into the list and makes it the current one (start-up and after a switch).
pub fn note_current(path: &std::path::Path) {
    if let Some(mut r) = registry() {
        let _ = libraries::note_current(&mut r, path);
    }
    update_label();
}

fn current_name() -> String {
    registry()
        .and_then(|r| r.current().map(|c| c.name.clone()))
        .unwrap_or_else(|| "Library".into())
}

/// Shows the name of the library in use on the button.
pub fn update_label() {
    let name = current_name();
    LABEL.with(|l| {
        if let Some(l) = l.borrow().as_ref().and_then(|w| w.upgrade()) {
            l.set_text(&name);
        }
    });
    CURRENT.with(|c| {
        if let Some(b) = c.borrow().as_ref().and_then(|w| w.upgrade()) {
            b.set_tooltip_text(Some(&trf(
                "Library: {}. Click to switch libraries",
                &[&name],
            )));
        }
    });
}

pub fn build(ctx: &Ctx) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.add_css_class("flat");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name("library-symbolic"));
    let label = gtk::Label::new(None);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(18);
    content.append(&label);
    content.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    button.set_child(Some(&content));
    super::audit::label(&button, &tr("Switch library"));
    let pop = gtk::Popover::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(8);
    body.set_margin_bottom(8);
    body.set_margin_start(8);
    body.set_margin_end(8);
    body.set_width_request(300);
    pop.set_child(Some(&body));
    button.set_popover(Some(&pop));
    {
        let (ctx, body, weak) = (ctx.clone(), body.clone(), pop.downgrade());
        pop.connect_show(move |_| {
            if let Some(pop) = weak.upgrade() {
                fill(&ctx, &body, &pop);
            }
        });
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(button.downgrade()));
    LABEL.with(|l| *l.borrow_mut() = Some(label.downgrade()));
    update_label();
    button
}

fn fill(ctx: &Ctx, body: &gtk::Box, pop: &gtk::Popover) {
    while let Some(c) = body.first_child() {
        body.remove(&c);
    }
    let rows = registry().map(|r| libraries::rows(&r)).unwrap_or_default();
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    for r in &rows {
        list.append(&library_row(ctx, r, pop, rows.len()));
    }
    body.append(&list);
    let new = gtk::Button::with_label(&tr("New Library"));
    let open = gtk::Button::with_label(&tr("Open Folder as Library"));
    for b in [&new, &open] {
        b.add_css_class("flat");
        body.append(b);
    }
    {
        let (ctx, pop) = (ctx.clone(), pop.clone());
        new.connect_clicked(move |b| {
            pop.popdown();
            new_library_dialog(&ctx, b.upcast_ref());
        });
    }
    {
        let (ctx, pop) = (ctx.clone(), pop.clone());
        open.connect_clicked(move |b| {
            pop.popdown();
            open_folder(&ctx, b.upcast_ref());
        });
    }
}

fn library_row(ctx: &Ctx, r: &Row, pop: &gtk::Popover, total: usize) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&r.name))
        .subtitle(glib::markup_escape_text(&libraries::row_detail(r)))
        .activatable(!r.current && r.available)
        .build();
    row.set_subtitle_lines(1);
    if r.current {
        row.add_prefix(&gtk::Image::from_icon_name("object-select-symbolic"));
    }
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.set_valign(gtk::Align::Center);
    menu.set_tooltip_text(Some(&tr("Library actions")));
    super::audit::label(&menu, &trf("Actions of library {}", &[&r.name]));
    let inner = gtk::Popover::new();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let rename = gtk::Button::with_label(&tr("Rename"));
    let remove = gtk::Button::with_label(&tr("Remove from List"));
    remove.set_tooltip_text(Some(&tr("The books and files stay where they are")));
    remove.set_sensitive(!r.current && total > 1);
    for b in [&rename, &remove] {
        b.add_css_class("flat");
        col.append(b);
    }
    inner.set_child(Some(&col));
    menu.set_popover(Some(&inner));
    row.add_suffix(&menu);
    {
        let (ctx, name, pop, inner) = (ctx.clone(), r.name.clone(), pop.clone(), inner.clone());
        rename.connect_clicked(move |b| {
            inner.popdown();
            pop.popdown();
            rename_dialog(&ctx, b.upcast_ref(), &name);
        });
    }
    {
        let (ctx, name, pop, inner) = (ctx.clone(), r.name.clone(), pop.clone(), inner.clone());
        remove.connect_clicked(move |_| {
            inner.popdown();
            pop.popdown();
            let said = remove_entry(&name);
            (ctx.toast)(said);
        });
    }
    {
        let (ctx, r, pop) = (ctx.clone(), r.clone(), pop.clone());
        row.connect_activated(move |_| {
            pop.popdown();
            switch_to(&ctx, &r.name);
        });
    }
    row
}

// ---- actions ----

/// Opens the library of this name (a click on its row).
pub fn switch_to(ctx: &Ctx, name: &str) -> String {
    let Some(entry) = registry().and_then(|r| r.get(name).cloned()) else {
        return format!("no library named {name}");
    };
    if !entry.path.is_dir() {
        let t = trf(
            "The folder of “{}” is not available. Is the drive connected?",
            &[&name],
        );
        (ctx.toast)(t.clone());
        return t;
    }
    (ctx.switch)(entry.path);
    format!("switching to {name}")
}

pub fn remove_entry(name: &str) -> String {
    match registry().map(|mut r| {
        if r.current().is_some_and(|c| c.name == name) {
            return Err("The library in use stays in the list.".to_string());
        }
        r.remove(name).map_err(|e| e.to_string())
    }) {
        Some(Ok(true)) => trf(
            "“{}” removed from the list; its files were not touched",
            &[&name],
        ),
        Some(Ok(false)) => format!("no library named {name}"),
        Some(Err(e)) => e,
        None => "the list of libraries cannot be read".into(),
    }
}

pub fn rename_entry(old: &str, new: &str) -> Result<String, String> {
    let mut r = registry().ok_or("The list of libraries cannot be read.")?;
    let new = libraries::check_name(&r, new, Some(old))?;
    r.rename(old, &new).map_err(|e| e.to_string())?;
    update_label();
    Ok(new)
}

/// Adds a folder to the list (named after it) and switches to it.
pub fn add_folder(ctx: &Ctx, path: &std::path::Path) -> String {
    let Some(mut r) = registry() else {
        return "the list of libraries cannot be read".into();
    };
    let name = match r.list().iter().find(|l| l.path == path) {
        Some(l) => l.name.clone(),
        None => {
            let name = libraries::unique_name(&r, &libraries::name_for_folder(path));
            if let Err(e) = r.add(&name, path) {
                return e.to_string();
            }
            name
        }
    };
    (ctx.switch)(path.to_path_buf());
    format!("added {name}")
}

/// Makes a folder for a new library and switches to it.
pub fn create_library(ctx: &Ctx, name: &str, parent: &std::path::Path) -> Result<String, String> {
    let r = registry().ok_or("The list of libraries cannot be read.")?;
    let name = libraries::check_name(&r, name, None)?;
    let folder = libraries::new_library_folder(parent, &name)
        .ok_or("That name cannot be used for a folder.")?;
    if folder.exists() && std::fs::read_dir(&folder).is_ok_and(|mut d| d.next().is_some()) {
        return Err(trf(
            "The folder {} already has files in it. Use “Open Folder as Library” for that.",
            &[&folder.display()],
        ));
    }
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    drop(r);
    let mut r = registry().ok_or("The list of libraries cannot be read.")?;
    r.add(&name, &folder).map_err(|e| e.to_string())?;
    (ctx.switch)(folder);
    Ok(name)
}

fn new_library_dialog(ctx: &Ctx, anchor: &gtk::Widget) {
    let d = adw::AlertDialog::new(
        Some(&tr("New Library")),
        Some(&tr("A library is a folder with its own books and catalogue. Choose a name; you will pick where it is kept next.")),
    );
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(&tr("Library name")));
    entry.set_activates_default(true);
    super::audit::label(&entry, &tr("Library name"));
    d.set_extra_child(Some(&entry));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("ok", &tr("Choose Folder"));
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let (ctx, anchor2) = (ctx.clone(), anchor.clone());
    d.connect_response(None, move |_, r| {
        if r != "ok" {
            return;
        }
        let name = entry.text().to_string();
        let dlg = gtk::FileDialog::builder()
            .title(tr("Where should the library folder be made?"))
            .build();
        let win = anchor2.root().and_downcast::<gtk::Window>();
        let ctx = ctx.clone();
        dlg.select_folder(win.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(parent) = res.ok().and_then(|f| f.path()) {
                if let Err(e) = create_library(&ctx, &name, &parent) {
                    (ctx.toast)(e);
                }
            }
        });
    });
    d.present(Some(anchor));
}

fn open_folder(ctx: &Ctx, anchor: &gtk::Widget) {
    let dlg = gtk::FileDialog::builder()
        .title(tr("Choose a Library Folder"))
        .accept_label(tr("Open"))
        .build();
    let win = anchor.root().and_downcast::<gtk::Window>();
    let ctx = ctx.clone();
    dlg.select_folder(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(p) = res.ok().and_then(|f| f.path()) {
            add_folder(&ctx, &p);
        }
    });
}

fn rename_dialog(ctx: &Ctx, anchor: &gtk::Widget, old: &str) {
    let d = adw::AlertDialog::new(Some(&tr("Rename Library")), None);
    let entry = gtk::Entry::new();
    entry.set_text(old);
    entry.set_activates_default(true);
    super::audit::label(&entry, &tr("Library name"));
    d.set_extra_child(Some(&entry));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("ok", &tr("Rename"));
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let (ctx, old) = (ctx.clone(), old.to_string());
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            if let Err(e) = rename_entry(&old, entry.text().as_str()) {
                (ctx.toast)(e);
            }
        }
    });
    d.present(Some(anchor));
}

// ---- test hooks ----

/// `library:libs=…` (see `_developer/docs.md`); printed as `libs: …`.
pub fn dev(ctx: &Ctx, cmd: &str) -> String {
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    let pair = || arg.split_once('|').unwrap_or((arg, ""));
    match name {
        "add" => {
            let (n, p) = pair();
            match registry().map(|mut r| r.add(n, std::path::Path::new(p))) {
                Some(Ok(())) => format!("added {n}"),
                Some(Err(e)) => format!("error {e}"),
                None => "error no registry".into(),
            }
        }
        "folder" => add_folder(ctx, std::path::Path::new(arg)),
        "switch" => switch_to(ctx, arg),
        "rename" => {
            let (a, b) = pair();
            match rename_entry(a, b) {
                Ok(n) => format!("renamed {n}"),
                Err(e) => format!("error {e}"),
            }
        }
        "remove" => remove_entry(arg),
        "new" => {
            let (n, parent) = pair();
            match create_library(ctx, n, std::path::Path::new(parent)) {
                Ok(n) => format!("created {n}"),
                Err(e) => format!("error {e}"),
            }
        }
        "dump" => {
            let rows = registry().map(|r| libraries::rows(&r)).unwrap_or_default();
            let text: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "{}{}{}",
                        r.name,
                        if r.current { "*" } else { "" },
                        if r.available { "" } else { "(missing)" }
                    )
                })
                .collect();
            let label = LABEL.with(|l| {
                l.borrow()
                    .as_ref()
                    .and_then(|w| w.upgrade())
                    .map(|l| l.text().to_string())
                    .unwrap_or_default()
            });
            format!("list {} label={label}", text.join(" ; "))
        }
        _ => format!("unknown {cmd}"),
    }
}
