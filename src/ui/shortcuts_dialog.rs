//! The "Keyboard Shortcuts" page of Preferences: every action with its keys, search, rebinding by
//! pressing the keys, conflict handling (swap or take away), reset, export and import. The model
//! is `crate::shortcuts`; this file shows it and applies it to the application accelerators.

use crate::i18n::{N_, tr, trf};
use std::cell::RefCell;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};

use super::preferences::{PrefCtx, register_group, register_page};
use crate::shortcuts::{self, Action, Group, Settle, Shortcuts};

/// Library actions whose keys are handled by the library's own shortcut controller (so that
/// Delete and Ctrl+A leave text fields alone); every other action is an application accelerator.
pub const LIBRARY_LOCAL: [&str; 7] = [
    "lib.focus-search",
    "lib.favourite",
    "lib.add-books",
    "book.edit-meta",
    "book.delete",
    "lib.select-all",
    "lib.toggle-sidebar",
];

thread_local! {
    static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ROWS: RefCell<Vec<RowRef>> = const { RefCell::new(Vec::new()) };
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

struct RowRef {
    action: String,
    haystack: String,
    row: glib::WeakRef<adw::ActionRow>,
    keys: glib::WeakRef<gtk::Label>,
    reset: glib::WeakRef<gtk::Button>,
    clear: glib::WeakRef<gtk::Button>,
}

struct Pending {
    action: String,
    accel: String,
    dialog: adw::AlertDialog,
}

/// Adds the page; called once from the registry of Preferences.
pub fn register() {
    if REGISTERED.with(|r| r.replace(true)) {
        return;
    }
    register_page(
        "shortcuts",
        &tr("Keyboard Shortcuts"),
        "input-keyboard-symbolic",
        100,
    );
    register_group("shortcuts", 10, tools_group);
    register_group("shortcuts", 15, share_group);
    register_group("shortcuts", 40, preferences_group);
    for (i, g) in [
        Group::Application,
        Group::Library,
        Group::Reader,
        Group::Editor,
    ]
    .into_iter()
    .enumerate()
    {
        register_group("shortcuts", 20 + i as i32, move |ctx| actions_group(ctx, g));
    }
}

/// Loads the saved shortcuts, sets the application accelerators and keeps them current.
pub fn init(app: &impl IsA<gtk::Application>) {
    shortcuts::load_global();
    let app = app.clone().upcast::<gtk::Application>();
    apply_app_accels(&app);
    shortcuts::on_change(move || {
        apply_app_accels(&app);
        refresh_rows();
    });
}

/// Sets the accelerators of every action that is not the library's or the reader's own key handling.
pub fn apply_app_accels(app: &gtk::Application) {
    let cur = shortcuts::current();
    for a in shortcuts::registry() {
        if a.group == Group::Reader || a.group == Group::Editor || is_local_only(&a.action) {
            continue;
        }
        let keys = cur.effective(&a.action);
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        app.set_accels_for_action(&a.action, &refs);
    }
}

fn is_local_only(action: &str) -> bool {
    LIBRARY_LOCAL.contains(&action) && action != "lib.focus-search"
}

/// (accelerator, action) pairs for the library's own shortcut controller.
pub fn library_local_shortcuts() -> Vec<(String, String)> {
    let cur = shortcuts::current();
    let mut out = Vec::new();
    for a in LIBRARY_LOCAL {
        for k in cur.effective(a) {
            out.push((k, a.to_string()));
        }
    }
    out
}

/// Runs a key the user has set for a reader action (or swallows a default key that was given
/// away). `Some(what happened)` when the key was handled, `None` to leave it to the built-in keys.
pub fn handle_reader_key(
    root: &gtk::Widget,
    key: gdk::Key,
    mods: gdk::ModifierType,
) -> Option<String> {
    let cur = shortcuts::current();
    if !cur.any_customised() {
        return None;
    }
    let accel = accel_of(key, mods & gtk::accelerator_get_default_mod_mask())?;
    match cur.resolve(Group::Reader, &accel) {
        shortcuts::Resolve::Action(a) => {
            let _ = root.activate_action(&a, None);
            Some(a)
        }
        shortcuts::Resolve::Disabled => Some("disabled".into()),
        shortcuts::Resolve::Unbound => None,
    }
}

/// A key the reader's built-in handling does not know that is the default of a reader action
/// (see `shortcuts::MORE_DEFAULTS`): runs the action. Returns its name.
pub fn handle_reader_default(
    root: &gtk::Widget,
    key: gdk::Key,
    mods: gdk::ModifierType,
) -> Option<String> {
    let accel = accel_of(key, mods & gtk::accelerator_get_default_mod_mask())?;
    let action = shortcuts::current().default_reader_action(&accel)?;
    let _ = root.activate_action(&action, None);
    Some(action)
}

/// The accelerator text of a key press (`None` for a modifier on its own).
pub fn accel_of(key: gdk::Key, mods: gdk::ModifierType) -> Option<String> {
    let lower = key.to_lower();
    let name = lower.name()?;
    shortcuts::from_press(
        mods.contains(gdk::ModifierType::SHIFT_MASK),
        mods.contains(gdk::ModifierType::CONTROL_MASK),
        mods.contains(gdk::ModifierType::ALT_MASK),
        mods.contains(gdk::ModifierType::SUPER_MASK),
        name.as_str(),
        lower.to_unicode(),
    )
}

// ---- the page ----

fn share_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(tr("Share and Reset"))
        .description(tr(
            "Save your shortcuts to a file, load them from one, or go back to the defaults",
        ))
        .build();
    let export = adw::ButtonRow::new();
    export.set_title(&tr("Export Shortcuts"));
    let import = adw::ButtonRow::new();
    import.set_title(&tr("Import Shortcuts"));
    let reset_all = adw::ButtonRow::new();
    reset_all.set_title(&tr("Reset All Shortcuts"));
    reset_all.add_css_class("destructive-action");
    let parent = ctx.parent.clone();
    {
        let parent = parent.clone();
        export.connect_activated(move |_| export_dialog(&parent));
    }
    import.connect_activated(move |_| import_dialog(&parent));
    reset_all.connect_activated(|_| reset_all_now());
    g.add(&export);
    g.add(&import);
    g.add(&reset_all);
    g
}

/// The keys of this window itself: fixed, so they are listed but not rebindable.
pub const PREFERENCES_KEYS: [(&str, &str); 2] = [
    (N_("Next Section"), "Ctrl+Page Down"),
    (N_("Previous Section"), "Ctrl+Page Up"),
];

fn preferences_group(_ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(tr("Preferences"))
        .description(tr(
            "These keys work inside Preferences and cannot be changed.",
        ))
        .build();
    for (title, keys) in PREFERENCES_KEYS {
        let row = adw::ActionRow::builder()
            .title(tr(title))
            .activatable(false)
            .focusable(false)
            .build();
        let l = gtk::Label::new(Some(keys));
        l.add_css_class("dim-label");
        l.set_valign(gtk::Align::Center);
        row.add_suffix(&l);
        g.add(&row);
    }
    g
}

fn tools_group(_ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(tr("Keyboard Shortcuts"))
        .description(tr(
            "Click a shortcut to change it, then press the new keys.",
        ))
        .build();
    let search = adw::EntryRow::builder()
        .title(tr("Filter shortcuts"))
        .build();
    search.connect_changed(|e| {
        QUERY.with(|q| *q.borrow_mut() = e.text().to_string());
        filter_rows();
    });
    g.add(&search);

    ROWS.with(|r| r.borrow_mut().clear());
    g
}

fn actions_group(ctx: &PrefCtx, group: Group) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(group.label())
        .build();
    if group == Group::Reader {
        g.set_description(Some(&tr(
            "Ctrl+D (bookmark) and Ctrl+L (Go to) work next to the older Ctrl+B and Ctrl+G. The letters that act on a selection (D, W, T, H, N, V) never act while you type in a field.",
        )));
    }
    let cur = shortcuts::current();
    for a in shortcuts::registry()
        .into_iter()
        .filter(|a| a.group == group)
    {
        g.add(&action_row(ctx, &a, &cur));
    }
    g
}

fn action_row(ctx: &PrefCtx, a: &Action, cur: &Shortcuts) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&a.title))
        .activatable(true)
        .build();
    let keys = gtk::Label::new(None);
    keys.add_css_class("dim-label");
    keys.set_valign(gtk::Align::Center);
    let clear = gtk::Button::from_icon_name("edit-clear-symbolic");
    clear.add_css_class("flat");
    clear.set_valign(gtk::Align::Center);
    clear.set_tooltip_text(Some(&tr("Remove the shortcut")));
    super::audit::label(&clear, &trf("Remove the shortcut of {}", &[&a.title]));
    let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
    reset.add_css_class("flat");
    reset.set_valign(gtk::Align::Center);
    reset.set_tooltip_text(Some(&tr("Reset to the default")));
    super::audit::label(&reset, &trf("Reset the shortcut of {}", &[&a.title]));
    row.add_suffix(&keys);
    row.add_suffix(&clear);
    row.add_suffix(&reset);
    {
        let (action, parent) = (a.action.clone(), ctx.parent.clone());
        row.connect_activated(move |_| capture_dialog(&parent, &action));
    }
    {
        let action = a.action.clone();
        clear.connect_clicked(move |_| shortcuts::update(|s| s.clear(&action)));
    }
    {
        let action = a.action.clone();
        reset.connect_clicked(move |_| shortcuts::update(|s| s.reset(&action)));
    }
    let haystack = format!(
        "{} {} {} {}",
        a.title,
        a.keywords,
        a.group.label(),
        cur.display(&a.action)
    )
    .to_lowercase();
    ROWS.with(|r| {
        r.borrow_mut().push(RowRef {
            action: a.action.clone(),
            haystack,
            row: row.downgrade(),
            keys: keys.downgrade(),
            reset: reset.downgrade(),
            clear: clear.downgrade(),
        })
    });
    paint_row(a, cur, &row, &keys, &reset, &clear);
    row
}

fn paint_row(
    a: &Action,
    cur: &Shortcuts,
    row: &adw::ActionRow,
    keys: &gtk::Label,
    reset: &gtk::Button,
    clear: &gtk::Button,
) {
    let effective = cur.effective(&a.action);
    keys.set_text(&if effective.is_empty() {
        tr("Not set")
    } else {
        cur.display(&a.action)
    });
    let custom = cur.is_customised(&a.action);
    reset.set_visible(custom);
    clear.set_visible(!effective.is_empty());
    let default = a
        .defaults
        .iter()
        .map(|d| shortcuts::label(d))
        .collect::<Vec<_>>()
        .join(", ");
    row.set_subtitle(&if !custom {
        String::new()
    } else if default.is_empty() {
        tr("Changed (no default)")
    } else {
        trf("Changed (default {})", &[&default])
    });
}

fn refresh_rows() {
    let cur = shortcuts::current();
    let reg = shortcuts::registry();
    ROWS.with(|r| {
        r.borrow_mut().retain(|x| x.row.upgrade().is_some());
        for x in r.borrow().iter() {
            let (Some(row), Some(keys), Some(reset), Some(clear)) = (
                x.row.upgrade(),
                x.keys.upgrade(),
                x.reset.upgrade(),
                x.clear.upgrade(),
            ) else {
                continue;
            };
            if let Some(a) = reg.iter().find(|a| a.action == x.action) {
                paint_row(a, &cur, &row, &keys, &reset, &clear);
            }
        }
    });
}

fn filter_rows() {
    let q = QUERY.with(|q| q.borrow().to_lowercase());
    ROWS.with(|r| {
        for x in r.borrow().iter() {
            if let Some(row) = x.row.upgrade() {
                row.set_visible(row_matches(&x.haystack, &q));
            }
        }
    });
}

/// Every word of the query must occur in the row's text.
pub fn row_matches(haystack: &str, query: &str) -> bool {
    query
        .split_whitespace()
        .all(|w| haystack.contains(&w.to_lowercase()))
}

// ---- rebinding ----

fn title_of(action: &str) -> String {
    shortcuts::registry()
        .into_iter()
        .find(|a| a.action == action)
        .map(|a| a.title)
        .unwrap_or_else(|| action.to_string())
}

/// Opens the "press the keys" dialog for an action.
pub fn capture_dialog(parent: &gtk::Widget, action: &str) {
    let title = title_of(action);
    let d = adw::AlertDialog::new(
        Some(&trf("Shortcut for “{}”", &[&title])),
        Some(&tr(
            "Press the new keys. Esc cancels, Backspace or Clear Shortcut removes the shortcut.",
        )),
    );
    d.add_response("cancel", &tr("Cancel"));
    d.add_response("clear", &tr("Clear Shortcut"));
    d.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
    d.set_close_response("cancel");
    {
        let action = action.to_string();
        d.connect_response(Some("clear"), move |_, _| {
            shortcuts::update(|s| s.clear(&action));
        });
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (d, action, parent) = (d.clone(), action.to_string(), parent.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            let mods = mods & gtk::accelerator_get_default_mod_mask();
            if mods.is_empty() {
                if key == gdk::Key::Escape {
                    d.close();
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::BackSpace {
                    shortcuts::update(|s| s.clear(&action));
                    d.close();
                    return glib::Propagation::Stop;
                }
            }
            let Some(accel) = accel_of(key, mods) else {
                return glib::Propagation::Stop; // a modifier on its own: wait for the key
            };
            if shortcuts::canonical(&accel).is_some_and(|c| needs_modifier(&action, &c)) {
                warn_modifier();
                return glib::Propagation::Stop; // the dialog stays open for another try
            }
            d.close();
            request(&parent, &action, &accel);
            glib::Propagation::Stop
        });
    }
    d.add_controller(keys);
    d.present(Some(parent));
}

/// A plain letter or digit would stop you typing in search fields; only the reader may use one.
fn needs_modifier(action: &str, accel: &str) -> bool {
    let group = shortcuts::registry()
        .into_iter()
        .find(|a| a.action == action)
        .map(|a| a.group);
    group != Some(Group::Reader) && shortcuts::is_bare_text_key(accel)
}

fn warn_modifier() {
    super::preferences::toast(&tr(
        "Add Ctrl or Alt: a plain letter would stop you typing in search fields",
    ));
}

/// Gives an action a key: binds it, or explains and offers to settle a conflict. This is what the
/// dialog does after the key press, and what the dev hook `keys:press=` runs.
pub fn request(parent: &gtk::Widget, action: &str, accel: &str) {
    let Some(accel) = shortcuts::canonical(accel) else {
        say("invalid");
        return;
    };
    if needs_modifier(action, &accel) {
        say("needs-modifier");
        warn_modifier();
        return;
    }
    let result = shortcuts::update(|s| s.bind(action, &accel));
    match result {
        Ok(()) => say("bound"),
        Err(others) => {
            say("conflict");
            let names: Vec<String> = others
                .iter()
                .map(|o| format!("“{}” ({})", o.title, o.group.label()))
                .collect();
            let d = adw::AlertDialog::new(
                Some(&tr("Shortcut already in use")),
                Some(&trf(
                    "{} is used by {}. Swap the two shortcuts, or take it away from {}?",
                    &[
                        &shortcuts::label(&accel),
                        &names.join(" and "),
                        &(if others.len() == 1 { "it" } else { "them" }),
                    ],
                )),
            );
            d.add_response("cancel", &tr("Cancel"));
            d.add_response("take", &tr("Take It Away"));
            d.add_response("swap", &tr("Swap"));
            d.set_response_appearance("swap", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("swap"));
            d.set_close_response("cancel");
            PENDING.with(|p| {
                *p.borrow_mut() = Some(Pending {
                    action: action.to_string(),
                    accel,
                    dialog: d.clone(),
                })
            });
            d.connect_response(None, |_, response| match response {
                "swap" => settle(Some(Settle::Swap)),
                "take" => settle(Some(Settle::TakeAway)),
                _ => settle(None),
            });
            d.present(Some(parent));
        }
    }
}

/// Finishes a conflict: `None` cancels.
pub fn settle(how: Option<Settle>) {
    let Some(p) = PENDING.with(|p| p.borrow_mut().take()) else {
        return;
    };
    match how {
        Some(h) => {
            shortcuts::update(|s| s.bind_settled(&p.action, &p.accel, h));
            say(if h == Settle::Swap {
                "swapped"
            } else {
                "taken"
            });
        }
        None => say("cancelled"),
    }
    p.dialog.close();
}

fn say(what: &str) {
    LAST.with(|l| *l.borrow_mut() = what.to_string());
}

fn reset_all_now() {
    let before = shortcuts::current();
    shortcuts::update(|s| s.reset_all());
    say("reset-all");
    undo_toast(&tr("Shortcuts reset"), before);
}

/// A toast that puts the shortcuts back the way they were.
fn undo_toast(text: &str, before: Shortcuts) {
    super::preferences::toast_undo(text, 8, move || shortcuts::replace(before.clone()), || {});
}

// ---- export and import ----

pub fn export_to(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(path, shortcuts::current().to_json())
}

/// Replaces the shortcuts with a file's; the text says what happened.
pub fn import_from(path: &std::path::Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (s, report) = Shortcuts::from_json(&text)?;
    let conflicts = s.all_conflicts();
    shortcuts::replace(s);
    let mut msg = trf("Imported {} shortcut(s).", &[&report.applied]);
    if !report.skipped.is_empty() {
        msg.push_str(&format!(
            " Skipped {}: {}.",
            report.skipped.len(),
            report.skipped.join("; ")
        ));
    }
    if !conflicts.is_empty() {
        msg.push_str(&format!(
            " {} key(s) are used twice; fix them in the list.",
            conflicts.len()
        ));
    }
    Ok(msg)
}

fn export_dialog(parent: &gtk::Widget) {
    let dlg = gtk::FileDialog::builder()
        .title(tr("Export Shortcuts"))
        .initial_name("bookx-shortcuts.json")
        .build();
    let win = parent.root().and_downcast::<gtk::Window>();
    dlg.save(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            let msg = match export_to(&path) {
                Ok(()) => trf("Saved to {}.", &[&path.display()]),
                Err(e) => trf("Could not save: {}", &[&e]),
            };
            super::preferences::toast(&msg);
        }
    });
}

fn import_dialog(parent: &gtk::Widget) {
    let dlg = gtk::FileDialog::builder()
        .title(tr("Import Shortcuts"))
        .build();
    let win = parent.root().and_downcast::<gtk::Window>();
    let parent = parent.clone();
    dlg.open(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            let before = shortcuts::current();
            match import_from(&path) {
                Ok(msg) => undo_toast(&msg, before),
                Err(e) => info(
                    &parent,
                    &tr("Import Shortcuts"),
                    &trf("Not a shortcuts file: {}", &[&e]),
                ),
            }
        }
    });
}

fn info(parent: &gtk::Widget, title: &str, msg: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(msg));
    d.add_response("ok", &tr("OK"));
    d.present(Some(parent));
}

// ---- test hooks ----

fn active_parent() -> Option<gtk::Widget> {
    relm4::main_adw_application()
        .active_window()
        .map(|w| w.upcast())
}

/// `keys:` dev commands (see `_developer/docs.md`).
pub fn dev(cmd: &str) {
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    match name {
        "open" => super::preferences::dev("pick=Keyboard Shortcuts"),
        "filter" => {
            QUERY.with(|q| *q.borrow_mut() = arg.to_string());
            filter_rows();
            let (shown, total) = ROWS.with(|r| {
                let r = r.borrow();
                (
                    r.iter()
                        .filter(|x| x.row.upgrade().is_some_and(|w| w.is_visible()))
                        .count(),
                    r.len(),
                )
            });
            eprintln!("keys: filter={arg:?} shown={shown} of={total}");
        }
        "press" => {
            let (action, accel) = arg.split_once('|').unwrap_or((arg, ""));
            if let Some(p) = active_parent() {
                request(&p, action, accel);
            }
            eprintln!(
                "keys: press {action} {accel} -> {}",
                LAST.with(|l| l.borrow().clone())
            );
        }
        "settle" => {
            settle(match arg {
                "swap" => Some(Settle::Swap),
                "take" => Some(Settle::TakeAway),
                _ => None,
            });
            eprintln!(
                "keys: settle {arg} -> {}",
                LAST.with(|l| l.borrow().clone())
            );
        }
        "clear" => shortcuts::update(|s| s.clear(arg)),
        "reset" => shortcuts::update(|s| s.reset(arg)),
        "resetall" => {
            reset_all_now();
            eprintln!(
                "keys: resetall -> {} customised={}",
                LAST.with(|l| l.borrow().clone()),
                shortcuts::current().any_customised()
            );
        }
        "undo" => {
            let pressed = super::preferences::press_undo();
            eprintln!(
                "keys: undo pressed={pressed} customised={}",
                shortcuts::current().any_customised()
            );
        }
        "export" => eprintln!(
            "keys: export {:?}",
            export_to(std::path::Path::new(arg)).is_ok()
        ),
        "import" => eprintln!(
            "keys: import {}",
            import_from(std::path::Path::new(arg)).unwrap_or_else(|e| format!("error {e}"))
        ),
        "dump" => {
            let cur = shortcuts::current();
            let a = if arg.is_empty() { "app.palette" } else { arg };
            let app = relm4::main_adw_application();
            eprintln!(
                "keys: {a} effective={:?} display={:?} gtk={:?} customised={} conflicts={}",
                cur.effective(a),
                cur.display(a),
                app.accels_for_action(a),
                cur.is_customised(a),
                cur.all_conflicts().len()
            );
        }
        "local" => eprintln!("keys: local={:?}", library_local_shortcuts()),
        "dialog" => {
            if let Some(p) = active_parent() {
                capture_dialog(&p, arg);
            }
        }
        _ => eprintln!("keys: unknown command {cmd}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preferences_section_keys_are_listed_and_do_not_clash_with_a_binding() {
        assert_eq!(PREFERENCES_KEYS.len(), 2);
        assert!(PREFERENCES_KEYS.iter().any(|(_, k)| *k == "Ctrl+Page Down"));
        assert!(PREFERENCES_KEYS.iter().any(|(_, k)| *k == "Ctrl+Page Up"));
        for a in shortcuts::registry() {
            for k in shortcuts::current().effective(&a.action) {
                assert!(
                    !(k.contains("<Control>")
                        && (k.contains("Page_Down") || k.contains("Page_Up"))),
                    "{} uses {k}",
                    a.action
                );
            }
        }
    }

    #[test]
    fn filter_needs_every_word() {
        assert!(row_matches("next page turn forward reader", "next page"));
        assert!(row_matches("next page", ""));
        assert!(!row_matches("next page", "next chapter"));
        assert!(row_matches("toggle bookmark ctrl+b", "CTRL+B"));
    }

    #[test]
    fn a_snapshot_restores_the_shortcuts_after_reset_all() {
        let mut s = Shortcuts::default();
        assert!(s.bind("reader.reload", "<Control><Alt>j").is_ok());
        let before = s.clone();
        s.reset_all();
        assert_ne!(s, before);
        s = before.clone();
        assert_eq!(s, before);
        assert!(s.is_customised("reader.reload"));
    }

    #[test]
    fn library_local_keys_exist_in_the_registry() {
        let reg = shortcuts::registry();
        for a in LIBRARY_LOCAL {
            assert!(reg.iter().any(|r| r.action == a), "{a}");
        }
    }
}
