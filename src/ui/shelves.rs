//! Shelves in the library window: the "Shelves" section of the browse panel (create, rename,
//! delete, reorder, counts, drop books on a shelf), the book menu entries and the drag source of
//! books. The model is `crate::shelves`; the books of a shelf are collections in the catalogue.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use bookx_store::Collection;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};

use super::SharedRef;
use crate::shelves::{self, LibraryShelves, ShelfPrefs};

thread_local! {
    /// The books selected in the library, for a drag that starts on one of them.
    static SELECTION: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// The shelf the library is showing (from its search), for "Remove from Shelf".
    static LAST_DROP: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn set_selection(ids: &[u64]) {
    SELECTION.with(|s| *s.borrow_mut() = ids.to_vec());
}

/// What the section needs from the library window.
#[derive(Clone)]
pub struct Ctx {
    pub shared: SharedRef,
    pub search: gtk::SearchEntry,
    /// Shows a search from the sidebar (switches to the Home view first).
    pub go: Rc<dyn Fn(String)>,
    pub parent: gtk::Widget,
    /// Re-draws the library and its browse panel.
    pub refresh: Rc<dyn Fn()>,
    pub toast: Rc<dyn Fn(String)>,
    /// A message with an Undo button.
    pub undo: Rc<dyn Fn(String, crate::app::UndoFn)>,
}

/// The icon of a shelf and of the Shelves group (Jobs and the table view have their own).
pub const SHELVES_ICON: &str = "bookmark-new-symbolic";

fn library_key(shared: &SharedRef) -> String {
    shared.lib.borrow().root.to_string_lossy().to_string()
}

fn load_prefs() -> ShelfPrefs {
    crate::config::shelves_path()
        .map(|p| ShelfPrefs::load(&p))
        .unwrap_or_default()
}

fn edit_order(shared: &SharedRef, f: impl FnOnce(&mut LibraryShelves)) {
    let mut p = load_prefs();
    f(p.of_mut(&library_key(shared)));
    if let Some(path) = crate::config::shelves_path() {
        let _ = p.save(&path);
    }
}

fn user_order(shared: &SharedRef) -> LibraryShelves {
    load_prefs().of(&library_key(shared))
}

/// The shelves in the user's order, with their book counts.
pub fn list(shared: &SharedRef) -> Vec<Collection> {
    let all = shared
        .guard
        .borrow()
        .store
        .list_collections()
        .unwrap_or_default();
    let names: Vec<String> = all.iter().map(|c| c.name.clone()).collect();
    user_order(shared)
        .sort_names(&names)
        .into_iter()
        .filter_map(|n| all.iter().find(|c| c.name == n).cloned())
        .collect()
}

fn find(shared: &SharedRef, name: &str) -> Option<Collection> {
    list(shared)
        .into_iter()
        .find(|c| c.name.eq_ignore_ascii_case(name))
}

/// The books on a shelf in the user's order (`None` when there is no such shelf).
pub fn members(shared: &SharedRef, name: &str) -> Option<Vec<u64>> {
    let c = find(shared, name)?;
    let ids = shared.guard.borrow().store.collection_books(c.id);
    Some(user_order(shared).sort_books(&c.name, ids))
}

/// Applies the `shelf:` terms of a query to the matching books. A single shelf shows its books in
/// the user's order, if the user gave one.
pub fn restrict(shared: &SharedRef, query: &str, ids: Vec<u64>) -> (String, Vec<u64>) {
    let (rest, terms) = shelves::split_terms(query);
    if terms.is_empty() {
        return (rest, ids);
    }
    let kept = shelves::filter_ids(ids, &terms, |n| members(shared, n));
    let positive: Vec<&shelves::ShelfTerm> = terms.iter().filter(|t| !t.negated).collect();
    if let ([one], true) = (
        positive.as_slice(),
        terms.len() == 1 && rest.trim().is_empty(),
    ) {
        if let Some(order) = members(shared, &one.name) {
            let mut sorted: Vec<u64> = order.into_iter().filter(|i| kept.contains(i)).collect();
            let tail: Vec<u64> = kept
                .iter()
                .copied()
                .filter(|i| !sorted.contains(i))
                .collect();
            sorted.extend(tail);
            return (rest, sorted);
        }
    }
    (rest, kept)
}

// ---- changes ----

pub fn create(shared: &SharedRef, name: &str) -> Result<String, String> {
    let existing: Vec<String> = list(shared).into_iter().map(|c| c.name).collect();
    let name = shelves::check_name(name, &existing, None)?;
    shared
        .guard
        .borrow()
        .store
        .create_collection(&name)
        .map_err(|e| e.to_string())?;
    Ok(name)
}

pub fn rename(shared: &SharedRef, old: &str, new: &str) -> Result<String, String> {
    let all = list(shared);
    let c = all
        .iter()
        .find(|c| c.name == old)
        .ok_or("There is no such shelf.")?;
    let existing: Vec<String> = all.iter().map(|c| c.name.clone()).collect();
    let new = shelves::check_name(new, &existing, Some(old))?;
    shared
        .guard
        .borrow()
        .store
        .rename_collection(c.id, &new)
        .map_err(|e| e.to_string())?;
    edit_order(shared, |l| l.rename(old, &new));
    Ok(new)
}

pub fn delete(shared: &SharedRef, name: &str) -> bool {
    let Some(c) = find(shared, name) else {
        return false;
    };
    let ok = shared
        .guard
        .borrow()
        .store
        .delete_collection(c.id)
        .unwrap_or(false);
    if ok {
        edit_order(shared, |l| l.forget(&c.name));
    }
    ok
}

/// Puts books on a shelf; the text for the toast.
pub fn add_books(shared: &SharedRef, name: &str, ids: &[u64]) -> String {
    let Some(c) = find(shared, name) else {
        return trf("There is no shelf called “{}”", &[&name]);
    };
    let store = shared.guard.borrow().store.clone();
    let there = store.collection_books(c.id);
    let (mut added, mut already) = (0, 0);
    for id in ids {
        if there.contains(id) {
            already += 1;
        } else if store.add_to_collection(c.id, *id).is_ok() {
            added += 1;
        }
    }
    shelves::added_text(&c.name, added, already)
}

pub fn remove_books(shared: &SharedRef, name: &str, ids: &[u64]) -> String {
    let Some(c) = find(shared, name) else {
        return trf("There is no shelf called “{}”", &[&name]);
    };
    let store = shared.guard.borrow().store.clone();
    let n = ids
        .iter()
        .filter(|id| store.remove_from_collection(c.id, **id).unwrap_or(false))
        .count();
    trf("Removed {} from “{}”", &[&shelves::count_text(n), &c.name])
}

pub fn move_shelf(shared: &SharedRef, name: &str, delta: i32) -> bool {
    let names: Vec<String> = list(shared).into_iter().map(|c| c.name).collect();
    let mut moved = false;
    edit_order(shared, |l| moved = l.move_shelf(&names, name, delta));
    moved
}

pub fn move_book(shared: &SharedRef, shelf: &str, book: u64, delta: i32) -> bool {
    let Some(current) = members(shared, shelf) else {
        return false;
    };
    let mut moved = false;
    edit_order(shared, |l| moved = l.move_book(shelf, current, book, delta));
    moved
}

/// What dropping a payload (text from a drag of books) on a shelf does.
pub fn handle_drop(ctx: &Ctx, shelf: &str, payload: &str) -> bool {
    let ids = shelves::decode_drag(payload);
    if ids.is_empty() {
        return false;
    }
    (ctx.toast)(add_books(&ctx.shared, shelf, &ids));
    (ctx.refresh)();
    true
}

// ---- drag ----

/// Makes a widget start a drag of books. The widget's name is the id of the book it shows; a
/// drag that starts on a selected book takes the whole selection.
pub fn attach_drag(widget: &impl IsA<gtk::Widget>) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    let w = widget.upcast_ref::<gtk::Widget>().downgrade();
    source.connect_prepare(move |_, _, _| {
        let id: u64 = w.upgrade()?.widget_name().parse().ok()?;
        let ids = SELECTION.with(|s| {
            let s = s.borrow();
            if s.contains(&id) {
                s.clone()
            } else {
                vec![id]
            }
        });
        Some(gdk::ContentProvider::for_value(
            &shelves::encode_drag(&ids).to_value(),
        ))
    });
    widget.add_controller(source);
}

fn attach_drop(row: &adw::ActionRow, ctx: &Ctx, shelf: &str) {
    let target = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
    let (ctx, shelf) = (ctx.clone(), shelf.to_string());
    target.connect_drop(move |_, value, _, _| {
        value
            .get::<String>()
            .is_ok_and(|text| handle_drop(&ctx, &shelf, &text))
    });
    {
        let row = row.downgrade();
        target.connect_current_drop_notify(move |t| {
            if let Some(r) = row.upgrade() {
                if t.current_drop().is_some() {
                    r.add_css_class("drop-target");
                } else {
                    r.remove_css_class("drop-target");
                }
            }
        });
    }
    row.add_controller(target);
}

// ---- the sidebar section ----

/// The "Shelves" row of the browse panel: shelves with counts, then the smart shelves (saved searches).
pub fn section(ctx: &Ctx) -> adw::ExpanderRow {
    let shelves_list = list(&ctx.shared);
    let smart = ctx.shared.cfg.borrow().saved_searches.clone();
    let exp = adw::ExpanderRow::builder().title(tr("Shelves")).build();
    exp.add_prefix(&gtk::Image::from_icon_name(SHELVES_ICON));
    exp.set_expanded(true);

    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.add_css_class("flat");
    add.add_css_class("row-actions");
    add.set_valign(gtk::Align::Center);
    add.set_tooltip_text(Some(&tr("New Shelf")));
    super::audit::label(&add, &tr("New Shelf"));
    {
        let ctx = ctx.clone();
        add.connect_clicked(move |_| new_shelf_dialog(&ctx));
    }
    exp.add_suffix(&add);
    exp.add_suffix(&super::library::count_label(
        shelves_list.len() + smart.len(),
    ));

    for c in &shelves_list {
        exp.add_row(&shelf_row(ctx, c));
    }
    for s in smart {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&s.name))
            .tooltip_text(tr("Smart shelf"))
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("system-search-symbolic"));
        let n = {
            let q = crate::saved_searches::reference_for(&s.name);
            let c = ctx.shared.cfg.borrow();
            let resolved = crate::saved_searches::resolve(&q, None, &c.saved_searches)
                .unwrap_or_else(|_| s.query.clone());
            ctx.shared
                .lib
                .borrow()
                .search(&resolved, crate::core::library::SortKey::Title, true)
                .len()
        };
        row.add_suffix(&super::library::count_label(n));
        let (go, q) = (
            ctx.go.clone(),
            crate::saved_searches::reference_for(&s.name),
        );
        row.connect_activated(move |_| go(q.clone()));
        exp.add_row(&row);
    }
    if shelves_list.is_empty() {
        // the empty list is the way to the first shelf
        let hint = adw::ActionRow::builder()
            .title(tr("New Shelf"))
            .subtitle(tr("No shelves yet. Drag books onto a shelf to fill it."))
            .activatable(true)
            .build();
        hint.set_widget_name("new-shelf-row");
        hint.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
        hint.add_css_class("placeholder-row");
        let ctx = ctx.clone();
        hint.connect_activated(move |_| new_shelf_dialog(&ctx));
        exp.add_row(&hint);
    }
    exp
}

fn shelf_row(ctx: &Ctx, c: &Collection) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&c.name))
        .activatable(true)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(SHELVES_ICON));
    row.add_suffix(&super::library::count_label(c.books));

    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.add_css_class("row-actions");
    menu.set_valign(gtk::Align::Center);
    menu.set_tooltip_text(Some(&tr("Shelf actions")));
    super::audit::label(&menu, &trf("Actions of shelf {}", &[&c.name]));
    let pop = gtk::Popover::new();
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let entry = |label: &str, f: Box<dyn Fn()>| {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        let pop = pop.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            f();
        });
        col.append(&b);
    };
    let name = c.name.clone();
    {
        let (ctx, name) = (ctx.clone(), name.clone());
        entry(&tr("Rename"), Box::new(move || rename_dialog(&ctx, &name)));
    }
    {
        let (ctx, name) = (ctx.clone(), name.clone());
        entry(
            &tr("Send to Device"),
            Box::new(move || send_shelf(&ctx, &name)),
        );
    }
    {
        let (ctx, name) = (ctx.clone(), name.clone());
        entry(
            &tr("Move Up"),
            Box::new(move || {
                move_shelf(&ctx.shared, &name, -1);
                (ctx.refresh)();
            }),
        );
    }
    {
        let (ctx, name) = (ctx.clone(), name.clone());
        entry(
            &tr("Move Down"),
            Box::new(move || {
                move_shelf(&ctx.shared, &name, 1);
                (ctx.refresh)();
            }),
        );
    }
    {
        let (ctx, name) = (ctx.clone(), name.clone());
        entry(
            &tr("Delete"),
            Box::new(move || delete_with_undo(&ctx, &name)),
        );
    }
    pop.set_child(Some(&col));
    menu.set_popover(Some(&pop));
    row.add_suffix(&menu);

    {
        let (go, q) = (ctx.go.clone(), shelves::reference_for(&c.name));
        row.connect_activated(move |_| go(q.clone()));
    }
    attach_drop(&row, ctx, &c.name);
    row
}

/// The ids of the books on the shelf, in the user's order for the shelf.
pub fn shelf_book_ids(shared: &SharedRef, name: &str) -> Vec<u64> {
    find(shared, name)
        .map(|c| shared.guard.borrow().store.collection_books(c.id))
        .unwrap_or_default()
}

/// Opens the Devices window with the books of the shelf as the books to send.
fn send_shelf(ctx: &Ctx, name: &str) {
    let ids = shelf_book_ids(&ctx.shared, name);
    if ids.is_empty() {
        (ctx.toast)(trf("The shelf “{}” has no books to send", &[&name]));
        return;
    }
    super::devices::set_library_selection(&ids);
    if let Some(app) = gio::Application::default() {
        gio::prelude::ActionGroupExt::activate_action(&app, "devices", None);
    }
}

// ---- dialogs ----

fn name_dialog(
    ctx: &Ctx,
    heading: &str,
    ok_label: &str,
    initial: &str,
    error: Option<&str>,
    done: Rc<dyn Fn(&str) -> Result<(), String>>,
) {
    let d = adw::AlertDialog::new(
        Some(heading),
        Some(error.unwrap_or("Shelves group books the way you like. A book can be on several.")),
    );
    let entry = gtk::Entry::new();
    entry.set_text(initial);
    entry.set_activates_default(true);
    entry.set_placeholder_text(Some(&tr("Shelf name")));
    super::audit::label(&entry, &tr("Shelf name"));
    d.set_extra_child(Some(&entry));
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("ok", ok_label);
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    {
        let (ctx, heading, ok_label) = (ctx.clone(), heading.to_string(), ok_label.to_string());
        let entry = entry.clone();
        d.connect_response(None, move |_, r| {
            if r != "ok" {
                return;
            }
            let text = entry.text().to_string();
            if let Err(e) = done(&text) {
                let ctx = ctx.clone();
                let (heading, ok_label, done) = (heading.clone(), ok_label.clone(), done.clone());
                glib::idle_add_local_once(move || {
                    name_dialog(&ctx, &heading, &ok_label, &text, Some(&e), done)
                });
            }
        });
    }
    d.present(Some(&ctx.parent));
}

pub fn new_shelf_dialog(ctx: &Ctx) {
    let c = ctx.clone();
    name_dialog(
        ctx,
        "New Shelf",
        "Create",
        "",
        None,
        Rc::new(move |name| {
            let made = create(&c.shared, name)?;
            (c.toast)(trf("Created shelf “{}”", &[&made]));
            (c.refresh)();
            Ok(())
        }),
    );
}

/// New shelf, then put these books on it.
pub fn new_shelf_with(ctx: &Ctx, ids: Vec<u64>) {
    let c = ctx.clone();
    name_dialog(
        ctx,
        "New Shelf",
        "Create",
        "",
        None,
        Rc::new(move |name| {
            let made = create(&c.shared, name)?;
            (c.toast)(add_books(&c.shared, &made, &ids));
            (c.refresh)();
            Ok(())
        }),
    );
}

fn rename_dialog(ctx: &Ctx, old: &str) {
    let (c, old_name) = (ctx.clone(), old.to_string());
    name_dialog(
        ctx,
        "Rename Shelf",
        "Rename",
        old,
        None,
        Rc::new(move |name| {
            let new = rename(&c.shared, &old_name, name)?;
            let q = shelves::reference_for(&old_name);
            if c.search.text().as_str() == q {
                c.search.set_text(&shelves::reference_for(&new));
            }
            (c.refresh)();
            Ok(())
        }),
    );
}

/// What a deleted shelf needs to come back: its name, books and place in the user's order.
#[derive(Clone)]
struct Snapshot {
    name: String,
    books: Vec<u64>,
    position: Option<usize>,
    book_order: Option<Vec<u64>>,
}

fn snapshot(shared: &SharedRef, name: &str) -> Option<Snapshot> {
    let c = find(shared, name)?;
    let order = user_order(shared);
    Some(Snapshot {
        books: members(shared, &c.name).unwrap_or_default(),
        position: order.position(&c.name),
        book_order: order.books.get(&c.name).cloned(),
        name: c.name,
    })
}

fn restore(shared: &SharedRef, snap: &Snapshot) -> bool {
    if create(shared, &snap.name).is_err() {
        return false;
    }
    add_books(shared, &snap.name, &snap.books);
    edit_order(shared, |l| {
        l.restore(&snap.name, snap.position, snap.book_order.clone())
    });
    true
}

/// Deletes a shelf at once; the toast brings it back (the books were never touched).
pub fn delete_with_undo(ctx: &Ctx, name: &str) {
    let Some(snap) = snapshot(&ctx.shared, name) else {
        return;
    };
    if !delete(&ctx.shared, name) {
        return;
    }
    if ctx.search.text().as_str() == shelves::reference_for(name) {
        ctx.search.set_text("");
    }
    (ctx.refresh)();
    let c = ctx.clone();
    (ctx.undo)(
        trf("Shelf “{}” deleted", &[&snap.name]),
        crate::app::UndoFn::new(move || {
            restore(&c.shared, &snap);
            (c.refresh)();
        }),
    );
}

// ---- book menu ----

/// The "Add to Shelf" submenu of the book menu (actions `book.shelf-add` and `book.shelf-new`).
pub fn add_to_shelf_menu(shared: &SharedRef) -> gio::Menu {
    let m = gio::Menu::new();
    let shelves_menu = gio::Menu::new();
    for c in list(shared) {
        let item = gio::MenuItem::new(Some(&c.name), None);
        item.set_action_and_target_value(Some("book.shelf-add"), Some(&c.name.to_variant()));
        shelves_menu.append_item(&item);
    }
    m.append_section(None, &shelves_menu);
    let new = gio::Menu::new();
    new.append(Some(&tr("New Shelf")), Some("book.shelf-new"));
    m.append_section(None, &new);
    m
}

/// "Remove from Shelf" for the shelf the search shows, if it shows one.
pub fn current_shelf(query: &str) -> Option<String> {
    let (_, terms) = shelves::split_terms(query);
    match terms.as_slice() {
        [t] if !t.negated => Some(t.name.clone()),
        _ => None,
    }
}

// ---- test hooks ----

/// `library:shelf=…` (see `_developer/docs.md`); the text is printed as `shelves: …`.
pub fn dev(ctx: &Ctx, selected: &[u64], cmd: &str) -> String {
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    let ids = |s: &str| -> Vec<u64> {
        s.split(['+', ','])
            .filter_map(|x| x.trim().parse().ok())
            .collect()
    };
    match name {
        "create" => match create(&ctx.shared, arg) {
            Ok(n) => {
                (ctx.refresh)();
                format!("created {n}")
            }
            Err(e) => format!("error {e}"),
        },
        "rename" => {
            let (old, new) = arg.split_once('|').unwrap_or((arg, ""));
            match rename(&ctx.shared, old, new) {
                Ok(n) => {
                    (ctx.refresh)();
                    format!("renamed {n}")
                }
                Err(e) => format!("error {e}"),
            }
        }
        // the Send to Device of a shelf row (opens the Devices window with the shelf's books)
        "send" => {
            let ids = shelf_book_ids(&ctx.shared, arg);
            send_shelf(ctx, arg);
            format!("send {arg} ids={ids:?}")
        }
        "delete" => {
            let ok = delete(&ctx.shared, arg);
            (ctx.refresh)();
            format!("deleted {ok}")
        }
        // the Delete of a shelf row: no dialog, a toast with Undo (`library:undo-last` presses it)
        "delete-undo" => {
            let before = list(&ctx.shared).len();
            delete_with_undo(ctx, arg);
            format!(
                "delete-undo shelves {before} -> {}",
                list(&ctx.shared).len()
            )
        }
        "add" => {
            let (n, l) = arg.split_once('|').unwrap_or((arg, ""));
            let t = add_books(&ctx.shared, n, &ids(l));
            (ctx.refresh)();
            t
        }
        "addsel" => {
            let t = add_books(&ctx.shared, arg, selected);
            (ctx.refresh)();
            t
        }
        "remove" => {
            let (n, l) = arg.split_once('|').unwrap_or((arg, ""));
            let t = remove_books(&ctx.shared, n, &ids(l));
            (ctx.refresh)();
            t
        }
        // the drop handler of a shelf row, with the text a drag of these books carries
        "drop" => {
            let (n, l) = arg.split_once('|').unwrap_or((arg, ""));
            let payload = if l.starts_with("bookx-books:") {
                l.to_string()
            } else {
                shelves::encode_drag(&ids(l))
            };
            let said = Rc::new(RefCell::new(String::new()));
            let mut probe = ctx.clone();
            let sink = said.clone();
            probe.toast = Rc::new(move |m| *sink.borrow_mut() = m);
            let handled = handle_drop(&probe, n, &payload);
            format!("drop handled={handled} said={}", said.borrow())
        }
        "up" | "down" => {
            let moved = move_shelf(&ctx.shared, arg, if name == "up" { -1 } else { 1 });
            (ctx.refresh)();
            format!("moved {moved}")
        }
        "bookup" | "bookdown" => {
            let (n, b) = arg.split_once('|').unwrap_or((arg, ""));
            let moved = move_book(
                &ctx.shared,
                n,
                b.parse().unwrap_or(0),
                if name == "bookup" { -1 } else { 1 },
            );
            (ctx.refresh)();
            format!("moved {moved}")
        }
        "open" => {
            ctx.search.set_text(&shelves::reference_for(arg));
            format!("opened {arg}")
        }
        "menu" => {
            let m = add_to_shelf_menu(&ctx.shared);
            let mut out = Vec::new();
            labels(m.upcast_ref(), &mut out);
            format!("menu {}", out.join("|"))
        }
        "dump" => {
            let l = list(&ctx.shared);
            let rows: Vec<String> = l
                .iter()
                .map(|c| {
                    let m = members(&ctx.shared, &c.name).unwrap_or_default();
                    format!(
                        "{}={}:{}",
                        c.name,
                        c.books,
                        m.iter().map(u64::to_string).collect::<Vec<_>>().join("+")
                    )
                })
                .collect();
            format!("list {}", rows.join(" ; "))
        }
        _ => format!("unknown {cmd}"),
    }
}

fn labels(m: &gio::MenuModel, out: &mut Vec<String>) {
    for i in 0..m.n_items() {
        if let Some(l) = m
            .item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(String::from))
        {
            out.push(l);
        }
        let links = m.iterate_item_links(i);
        while let Some((_, sub)) = links.next() {
            labels(&sub, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_single_positive_shelf_term_is_the_current_shelf() {
        assert_eq!(
            current_shelf("shelf:\"To read\"").as_deref(),
            Some("To read")
        );
        assert_eq!(current_shelf("shelf:A tag:x"), Some("A".into()));
        assert_eq!(current_shelf("shelf:A shelf:B"), None);
        assert_eq!(current_shelf("-shelf:A"), None);
        assert_eq!(current_shelf("tag:x"), None);
    }
}
