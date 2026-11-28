//! The Device window: the books on a connected e-reader (or the cloud folder) with their covers
//! (grid or list), sending the books selected in the library, removing books, free space, how books
//! are sent (format, save-path template, Kobo shelf, notes), syncing reading progress and notes with
//! the library, and setting up a WebDAV folder. The logic is in `crate::devices`,
//! `crate::device_sync` and `crate::device_covers`; this file is widgets and a thread per slow
//! operation. [`start_sync`] is also what the library's "device connected" banner uses.

use crate::i18n::N_;
use crate::i18n::{ntr, tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use bookx_core::contracts::DeviceInfo;
use bookx_serve::devfs::{self, DeviceBook, human_size};
use bookx_serve::devices::{DeviceKind, OtherDrive};
use bookx_serve::reading::DeviceState;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::SharedRef;
use super::devices_list::{Item, PickList};
use crate::device_sync as ds;
use crate::devices as dev;

const NO_DEVICE_SHORT: &str = N_("No device is connected.");
const SYNC_TOOLTIP: &str =
    N_("Bring the reading progress from the device into the library (Kobo database, KOReader)");
const SEND_DESCRIPTION: &str = N_(
    "The books selected in the library. Untick a book to leave it out; a book with a warning is left out until you tick it.",
);
const OPTIONS_DESCRIPTION: &str =
    N_("Used by Send to Device. If a conversion fails, the next format the device opens is tried.");
const OPTIONS_FIELDS_DESCRIPTION: &str = N_(
    "Used by Send to Device. If a conversion fails, the next format the device opens is tried. Fields: {title} {author} {authors} {author_sort} {series} {series_index} {language}",
);
const WEBDAV_DESCRIPTION: &str = N_(
    "Nextcloud, ownCloud and other WebDAV folders work like a device: send books there, list and remove them.",
);
const OTHER_DRIVES_DESCRIPTION: &str = N_(
    "Removable drives that Bookx does not recognise as e-readers (USB sticks, card readers, disks). If one of them is an e-reader, tell Bookx and it is offered like any other device.",
);
const REMOVE_DESCRIPTION: &str =
    N_("The files are deleted from the device. Copies in your library stay.");

thread_local! {
    /// The books selected in the library, for "Send Library Selection".
    static LIBRARY_SELECTION: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    /// What the open sync dialog does when confirmed (the UI tests confirm through it).
    static PENDING_SYNC: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The library tells the window which books are selected; they are the books to send when the
/// window opens.
pub fn set_library_selection(ids: &[u64]) {
    LIBRARY_SELECTION.with(|s| *s.borrow_mut() = ids.to_vec());
}

fn library_selection() -> Vec<u64> {
    LIBRARY_SELECTION.with(|s| s.borrow().clone())
}

/// The icons of the Books and Settings pages in the view switcher.
const PAGE_ICONS: [&str; 2] = ["view-list-symbolic", "emblem-system-symbolic"];

struct DevWin {
    dialog: adw::Dialog,
    parent: adw::ApplicationWindow,
    root: gtk::Box,
    stack: adw::ViewStack,
    books_page: adw::ViewStackPage,
    settings_page: adw::ViewStackPage,
    no_device_page: adw::StatusPage,
    books_content: gtk::Box,
    view_toggle: gtk::ToggleButton,
    on_dev: Rc<PickList>,
    to_send: Rc<PickList>,
    on_dev_menu: gio::Menu,
    send_menu: gio::Menu,
    on_all: gio::SimpleAction,
    on_none: gio::SimpleAction,
    send_all: gio::SimpleAction,
    send_none: gio::SimpleAction,
    send_ids: RefCell<Vec<u64>>,
    plans: RefCell<Vec<(u64, dev::SendPlan)>>,
    send_group: adw::PreferencesGroup,
    empty_send: adw::ActionRow,
    send_btn: gtk::Button,
    send_status: gtk::Label,
    sending: Cell<bool>,
    notes_row: adw::SwitchRow,
    auto_row: adw::SwitchRow,
    shared: SharedRef,
    on_changed: Rc<dyn Fn()>,
    device_row: adw::ComboRow,
    books_group: adw::PreferencesGroup,
    status: gtk::Label,
    remove_btn: gtk::Button,
    sync_btn: gtk::Button,
    format_row: adw::ComboRow,
    template_row: adw::EntryRow,
    options_group: adw::PreferencesGroup,
    shelf_row: adw::EntryRow,
    dav_url: adw::EntryRow,
    dav_user: adw::EntryRow,
    dav_pass: adw::PasswordEntryRow,
    dav_status: gtk::Label,
    devices: RefCell<Vec<DeviceInfo>>,
    scanner: super::device_scan::Scanner,
    scanning: Cell<bool>,
    others_group: adw::PreferencesGroup,
    other_rows: RefCell<Vec<gtk::Widget>>,
    others: RefCell<Vec<OtherDrive>>,
    chosen: RefCell<Vec<(DeviceInfo, String)>>,
    pending_use: RefCell<Option<(adw::AlertDialog, adw::ComboRow, adw::EntryRow, String)>>,
    books: RefCell<Vec<DeviceBook>>,
    states: RefCell<HashMap<PathBuf, DeviceState>>,
    shelves: RefCell<HashMap<PathBuf, Vec<String>>>,
    generation: Cell<u64>,
    loading: Cell<bool>,
    last_summary: RefCell<String>,
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<DevWin>>> = const { RefCell::new(None) };
}

/// Preferred dialog height, never taller than 85% of the parent (LIB-D8 / X6).
/// "7.5 GB · Not a known e-reader" for the Other drives list.
fn other_subtitle(o: &OtherDrive) -> String {
    let why = dev::not_identified_text(o.why);
    match o.size_bytes {
        Some(b) if b > 0 => format!("{} · {why}", human_size(b)),
        _ => why,
    }
}

fn dialog_content_height(parent_height: i32, preferred: i32) -> i32 {
    if parent_height > 0 {
        preferred
            .min((parent_height as f64 * 0.85).floor() as i32)
            .max(1)
    } else {
        preferred
    }
}

/// The "Books on the device" group description (LIB-D4).
fn selection_description(total: usize, selected: usize) -> String {
    trf("{} of {} selected", &[&selected, &total])
}

/// The dev hook still reports why no device is connected; the status page itself only shows the
/// short connection hint.
fn no_device_hint() -> String {
    format!(
        "{} {}",
        tr(NO_DEVICE_SHORT),
        ds::no_device_status_description()
    )
}

/// The labels of a `gio::Menu`, for the `devwin:layout` hook.
fn menu_labels(menu: &gio::Menu) -> Vec<String> {
    (0..menu.n_items())
        .filter_map(|i| {
            menu.item_attribute_value(i, "label", None)
                .and_then(|v| v.str().map(String::from))
        })
        .collect()
}

/// Count linked toggle buttons. A `linked` box in a header bar is a HIG violation, so this must be
/// zero (LIB-M4 / WIN-L14).
fn linked_toggles(w: &gtk::Widget) -> usize {
    let mut n =
        usize::from(w.has_css_class("linked") && w.downcast_ref::<gtk::ToggleButton>().is_some());
    if let Some(mut child) = w.first_child() {
        loop {
            n += linked_toggles(&child);
            let Some(next) = child.next_sibling() else {
                break;
            };
            child = next;
        }
    }
    n
}

/// One header menu with Select All / Select None for a pick list. The actions live on the dialog's
/// root widget so the menu items have real menu roles and keyboard semantics.
fn pick_menu(
    actions: &gio::SimpleActionGroup,
    prefix: &str,
    list: Rc<PickList>,
) -> (gio::Menu, gtk::MenuButton) {
    let menu = gio::Menu::new();
    for (label, suffix) in [(tr("Select All"), "all"), (tr("Select None"), "none")] {
        let item = gio::MenuItem::new(Some(&label), Some(&format!("devwin.{prefix}-{suffix}")));
        menu.append_item(&item);
        let action = gio::SimpleAction::new(&format!("{prefix}-{suffix}"), None);
        let l = list.clone();
        let select = suffix == "all";
        action.connect_activate(move |_, _| l.set_all(select));
        actions.add_action(&action);
    }
    let button = gtk::MenuButton::new();
    button.set_icon_name("view-more-symbolic");
    button.set_tooltip_text(Some(&tr("Selection options")));
    button.set_menu_model(Some(&menu));
    button.add_css_class("flat");
    super::audit::label(&button, &tr("Selection options"));
    super::audit::name_menu_items(&button);
    (menu, button)
}

/// The single grid/list toggle: the icon shows the current view, the name offers the other one
/// (LIB-M4 / WIN-L14 (b)).
fn update_view_toggle(button: &gtk::ToggleButton) {
    let (icon, _) = ds::view_toggle_state(button.is_active());
    button.set_icon_name(icon);
    let name = if button.is_active() {
        tr("Show as a Grid")
    } else {
        tr("Show as a List")
    };
    button.set_tooltip_text(Some(&name));
    super::audit::label(button, &name);
}

/// Opens the window (a second call brings the open one to the front).
pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef, on_changed: Rc<dyn Fn()>) {
    crate::trace::set_active(crate::trace::Area::Devices);
    if let Some(w) = CURRENT.with(|c| c.borrow().clone()) {
        if w.dialog.is_visible() {
            w.set_send_ids(library_selection());
            w.reload_devices();
            return;
        }
    }
    let w = DevWin::build(parent, shared, on_changed);
    w.dialog.present(Some(parent));
    {
        let weak = Rc::downgrade(&w);
        w.dialog.connect_closed(move |_| {
            if let Some(w) = weak.upgrade() {
                w.generation.set(w.generation.get() + 1);
                w.scanner.invalidate();
            }
            CURRENT.with(|c| *c.borrow_mut() = None);
        });
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(w.clone()));
    w.set_send_ids(library_selection());
    w.reload_devices();
}

impl DevWin {
    fn build(
        parent: &adw::ApplicationWindow,
        shared: &SharedRef,
        on_changed: Rc<dyn Fn()>,
    ) -> Rc<DevWin> {
        let dialog = super::dialog_toast::new_dialog();
        dialog.set_title(&tr("Devices"));
        dialog.set_follows_content_size(false);
        let settings = shared.cfg.borrow().device.clone();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        // the dialog scrolls as a whole: the side margins keep the overlay scrollbar off the header buttons
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_margin_top(12);
        root.set_margin_bottom(12);

        // The device selector lives outside both pages, so it is visible on Books and Settings.
        // The row itself supplies the only visible "Device" label.
        let device_group = adw::PreferencesGroup::new();
        let device_row = adw::ComboRow::builder().title(tr("Device")).build();
        device_row.set_model(Some(&gtk::StringList::new(&[])));
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(&tr("Look for devices again")));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        device_row.add_suffix(&refresh);
        device_group.add(&device_row);
        root.append(&device_group);

        let stack = adw::ViewStack::new();

        // ---- Books page ---------------------------------------------------------------------
        let no_device_page = adw::StatusPage::new();
        no_device_page.set_title(&ds::no_device_status_title());
        no_device_page.set_description(Some(&ds::no_device_status_description()));
        no_device_page.set_icon_name(Some("drive-removable-media-symbolic"));
        no_device_page.set_vexpand(true);

        let books_content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let books_group = adw::PreferencesGroup::new();
        books_group.set_title(&tr("Books on the device"));
        let header_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header_buttons.set_valign(gtk::Align::Center);
        let sync_btn = gtk::Button::with_label(&tr("Sync Reading State"));
        sync_btn.set_tooltip_text(Some(&tr(SYNC_TOOLTIP)));
        let view_toggle = gtk::ToggleButton::new();
        view_toggle.set_active(settings_view_is_list(shared));
        view_toggle.add_css_class("flat");
        update_view_toggle(&view_toggle);
        header_buttons.append(&sync_btn);
        header_buttons.append(&view_toggle);
        books_group.set_header_suffix(Some(&header_buttons));

        let slot: Rc<RefCell<Option<std::rc::Weak<DevWin>>>> = Rc::default();
        let changed = {
            let slot = slot.clone();
            Rc::new(move || {
                if let Some(w) = slot.borrow().as_ref().and_then(|w| w.upgrade()) {
                    w.update_buttons();
                }
            }) as Rc<dyn Fn()>
        };
        let cover = super::book_tile::cover_width(shared.cfg.borrow().cover_size);
        let on_dev = PickList::new(changed.clone(), cover);
        let to_send = PickList::new(changed, cover);
        let grid_view = !settings_view_is_list(shared);
        on_dev.set_grid(grid_view);
        to_send.set_grid(grid_view);

        let actions = gio::SimpleActionGroup::new();
        root.insert_action_group("devwin", Some(&actions));
        let (on_dev_menu, on_menu_button) = pick_menu(&actions, "on", on_dev.clone());
        header_buttons.append(&on_menu_button);
        let on_all = actions
            .lookup_action("on-all")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            .expect("the Select All action was just added");
        let on_none = actions
            .lookup_action("on-none")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            .expect("the Select None action was just added");

        books_group.add(&on_dev.widget);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_selectable(true);
        status.set_margin_top(6);
        books_group.add(&status);
        let remove_btn = gtk::Button::with_label(&ds::remove_button_label(0));
        remove_btn.add_css_class("destructive-action");
        remove_btn.set_halign(gtk::Align::End);
        remove_btn.set_margin_top(6);
        remove_btn.set_sensitive(false);
        books_group.add(&remove_btn);
        books_content.append(&books_group);

        let send_group = adw::PreferencesGroup::new();
        send_group.set_title(&tr("Books to send"));
        send_group.set_description(Some(&tr(SEND_DESCRIPTION)));
        let (send_menu, send_menu_button) = pick_menu(&actions, "send", to_send.clone());
        send_group.set_header_suffix(Some(&send_menu_button));
        let send_all = actions
            .lookup_action("send-all")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            .expect("the Select All action was just added");
        let send_none = actions
            .lookup_action("send-none")
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            .expect("the Select None action was just added");
        let empty_send = adw::ActionRow::builder()
            .title(tr(
                "Select books in the library, then choose Send to Device",
            ))
            .selectable(false)
            .build();
        send_group.add(&empty_send);
        let send_status = gtk::Label::new(None);
        send_status.set_xalign(0.0);
        send_status.set_wrap(true);
        send_status.add_css_class("dim-label");
        send_status.set_margin_top(6);
        let send_btn = gtk::Button::with_label(&ds::send_button_label(0));
        send_btn.add_css_class("suggested-action");
        send_btn.add_css_class("pill");
        send_btn.set_halign(gtk::Align::End);
        send_btn.set_margin_top(6);
        send_btn.set_sensitive(false);
        send_group.add(&to_send.widget);
        send_group.add(&send_status);
        send_group.add(&send_btn);
        books_content.append(&send_group);

        let books = gtk::Box::new(gtk::Orientation::Vertical, 12);
        books.append(&no_device_page);
        books.append(&books_content);
        let books_page = stack.add_titled(&books, Some("books"), &tr("Books"));
        books_page.set_icon_name(Some(PAGE_ICONS[0]));

        // ---- Settings page ------------------------------------------------------------------
        let settings_page_widget = adw::PreferencesPage::new();
        let options_group = adw::PreferencesGroup::new();
        options_group.set_title(&tr("Sending options"));
        options_group.set_description(Some(&tr(OPTIONS_DESCRIPTION)));
        let format_row = adw::ComboRow::builder().title(tr("Format")).build();
        let labels: Vec<String> = std::iter::once(tr("Automatic (best for the device)"))
            .chain(crate::export::TARGETS.iter().map(|t| tr(t.label)))
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        format_row.set_model(Some(&gtk::StringList::new(&refs)));
        let template_row = adw::EntryRow::builder()
            .title(tr("Save path, e.g. {author}/{series}/{title}"))
            .build();
        template_row.set_text(&settings.template);
        let shelf_row = adw::EntryRow::builder()
            .title(tr(
                "Kobo shelf for sent books (empty: leave the Kobo alone)",
            ))
            .build();
        shelf_row.set_text(&settings.kobo_shelf);
        let notes_row = adw::SwitchRow::builder()
            .title(tr(ds::NOTES_TITLE))
            .subtitle(tr(ds::NOTES_SUBTITLE))
            .build();
        notes_row.set_active(shared.cfg.borrow().device_sync.include_notes);
        options_group.add(&format_row);
        options_group.add(&template_row);
        options_group.add(&shelf_row);
        options_group.add(&notes_row);
        settings_page_widget.add(&options_group);

        let sync_group = adw::PreferencesGroup::new();
        sync_group.set_title(&tr("Sync"));
        let auto_row = adw::SwitchRow::builder()
            .title(tr(ds::AUTO_SYNC_TITLE))
            .subtitle(tr(ds::AUTO_SYNC_SUBTITLE))
            .build();
        auto_row.set_active(shared.cfg.borrow().device_sync.auto_sync);
        auto_row.add_suffix(&super::help_button(&tr(ds::AUTO_SYNC_HELP)));
        sync_group.add(&auto_row);
        settings_page_widget.add(&sync_group);

        let dav_group = adw::PreferencesGroup::new();
        dav_group.set_title(&tr("Cloud folder (WebDAV)"));
        dav_group.set_description(Some(&tr(WEBDAV_DESCRIPTION)));
        let dav_url = adw::EntryRow::builder()
            .title(tr(
                "Address, e.g. https://cloud.example/remote.php/dav/files/me/Books",
            ))
            .build();
        dav_url.set_text(&settings.webdav.url);
        let dav_user = adw::EntryRow::builder().title(tr("User name")).build();
        dav_user.set_text(&settings.webdav.user);
        let dav_pass = adw::PasswordEntryRow::new();
        dav_pass.set_title(&tr("Password"));
        dav_pass.set_text(&settings.webdav.password);
        let dav_check = gtk::Button::with_label(&tr("Check Connection"));
        dav_check.set_halign(gtk::Align::Start);
        let dav_status = gtk::Label::new(None);
        dav_status.set_xalign(0.0);
        dav_status.set_text(&crate::secrets::caption_now());
        dav_status.set_wrap(true);
        for r in [
            dav_url.upcast_ref::<gtk::Widget>(),
            dav_user.upcast_ref(),
            dav_pass.upcast_ref(),
            dav_check.upcast_ref(),
            dav_status.upcast_ref(),
        ] {
            dav_group.add(r);
        }
        settings_page_widget.add(&dav_group);
        let others_group = adw::PreferencesGroup::new();
        others_group.set_title(&tr("Other drives"));
        others_group.set_description(Some(&tr(OTHER_DRIVES_DESCRIPTION)));
        others_group.set_visible(false);
        settings_page_widget.add(&others_group);
        settings_page_widget.add(&super::report_dialog::problem_group(
            "Devices|Built-in server|Conversion",
        ));
        let settings_page =
            stack.add_titled(&settings_page_widget, Some("settings"), &tr("Settings"));
        settings_page.set_icon_name(Some(PAGE_ICONS[1]));
        root.append(&stack);

        let view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);
        header.set_title_widget(Some(&switcher));
        view.add_top_bar(&header);
        let outer = gtk::ScrolledWindow::new();
        outer.set_child(Some(&root));
        outer.set_max_content_height(480);
        outer.set_propagate_natural_height(false);
        view.set_content(Some(&outer));
        dialog.set_child(Some(&view));
        // Set the size after the child: libadwaita derives it from the natural size of whatever is inside.
        dialog.set_content_width(640);
        dialog.set_content_height(dialog_content_height(parent.height(), 560));

        let w = Rc::new(DevWin {
            dialog,
            parent: parent.clone(),
            root,
            stack,
            books_page,
            settings_page,
            no_device_page,
            books_content,
            view_toggle,
            on_dev,
            to_send,
            on_dev_menu,
            send_menu,
            on_all,
            on_none,
            send_all,
            send_none,
            send_ids: RefCell::default(),
            plans: RefCell::default(),
            send_group,
            empty_send,
            send_btn,
            send_status,
            sending: Cell::new(false),
            notes_row,
            auto_row,
            shared: shared.clone(),
            on_changed,
            device_row,
            books_group,
            status,
            remove_btn,
            sync_btn,
            format_row,
            template_row,
            options_group,
            shelf_row,
            dav_url,
            dav_user,
            dav_pass,
            dav_status,
            devices: RefCell::default(),
            scanner: super::device_scan::Scanner::new(),
            scanning: Cell::new(false),
            others_group,
            other_rows: RefCell::default(),
            others: RefCell::default(),
            chosen: RefCell::default(),
            pending_use: RefCell::default(),
            books: RefCell::default(),
            states: RefCell::default(),
            shelves: RefCell::default(),
            generation: Cell::new(0),
            loading: Cell::new(false),
            last_summary: RefCell::default(),
        });
        *slot.borrow_mut() = Some(Rc::downgrade(&w));
        w.select_format(&settings.format);

        {
            let w2 = w.clone();
            refresh.connect_clicked(move |_| w2.reload_devices());
            let w2 = w.clone();
            w.device_row
                .connect_selected_notify(move |_| w2.reload_books());
            let w2 = w.clone();
            w.remove_btn
                .connect_clicked(move |_| w2.remove_selected(true));
            let w2 = w.clone();
            w.sync_btn.connect_clicked(move |_| w2.sync_state(true));
            let w2 = w.clone();
            w.view_toggle.connect_toggled(move |b| {
                let list = b.is_active();
                update_view_toggle(b);
                w2.on_dev.set_grid(!list);
                w2.to_send.set_grid(!list);
                let mut c = w2.shared.cfg.borrow_mut();
                c.device_list_view = list;
                c.save();
            });
            let w2 = w.clone();
            w.send_btn.connect_clicked(move |_| w2.send_checked());
            let w2 = w.clone();
            w.notes_row.connect_active_notify(move |r| {
                {
                    let mut c = w2.shared.cfg.borrow_mut();
                    if c.device_sync.include_notes == r.is_active() {
                        return;
                    }
                    c.device_sync.include_notes = r.is_active();
                    c.save();
                }
                w2.refresh_send_list();
                crate::settings_bus::sync().notify();
            });
            let w2 = w.clone();
            w.auto_row.connect_active_notify(move |r| {
                {
                    let mut c = w2.shared.cfg.borrow_mut();
                    if c.device_sync.auto_sync == r.is_active() {
                        return;
                    }
                    c.device_sync.auto_sync = r.is_active();
                    c.save();
                }
                crate::settings_bus::sync().notify();
            });
            {
                // Preferences → Sync shows the same two switches
                let (shared, notes, auto) = (
                    w.shared.clone(),
                    w.notes_row.downgrade(),
                    w.auto_row.downgrade(),
                );
                super::share::follow(&w.auto_row, move || {
                    let d = shared.cfg.borrow().device_sync.clone();
                    if let Some(r) = notes.upgrade() {
                        if r.is_active() != d.include_notes {
                            r.set_active(d.include_notes);
                        }
                    }
                    if let Some(r) = auto.upgrade() {
                        if r.is_active() != d.auto_sync {
                            r.set_active(d.auto_sync);
                        }
                    }
                });
            }
            let w2 = w.clone();
            w.format_row
                .connect_selected_notify(move |_| w2.save_send_settings());
            let w2 = w.clone();
            w.template_row
                .connect_changed(move |_| w2.save_send_settings());
            let w2 = w.clone();
            w.shelf_row
                .connect_changed(move |_| w2.save_send_settings());
            for e in [&w.dav_url, &w.dav_user] {
                let w2 = w.clone();
                e.connect_apply(move |_| w2.save_dav());
                let w2 = w.clone();
                e.connect_entry_activated(move |_| w2.save_dav());
            }
            let w2 = w.clone();
            w.dav_pass.connect_apply(move |_| w2.save_dav());
            let w2 = w.clone();
            w.dav_pass.connect_entry_activated(move |_| w2.save_dav());
            let w2 = w.clone();
            dav_check.connect_clicked(move |_| w2.check_dav());
        }
        w
    }

    fn select_format(&self, id: &str) {
        let idx = crate::export::TARGETS
            .iter()
            .position(|t| t.id == id)
            .map(|i| i as u32 + 1)
            .unwrap_or(0);
        self.format_row.set_selected(idx);
    }

    fn chosen_format(&self) -> String {
        match self.format_row.selected() {
            0 => String::new(),
            i => crate::export::TARGETS
                .get(i as usize - 1)
                .map(|t| t.id.to_string())
                .unwrap_or_default(),
        }
    }

    /// Stores the format, template and shelf. A template that cannot work is shown and not saved.
    fn save_send_settings(self: &Rc<Self>) {
        let template = self.template_row.text().trim().to_string();
        match devfs::template_problem(&template) {
            Some(p) => {
                self.template_row.add_css_class("error");
                self.options_group.set_description(Some(&p));
                return;
            }
            None => {
                self.template_row.remove_css_class("error");
                self.options_group
                    .set_description(Some(&tr(OPTIONS_FIELDS_DESCRIPTION)));
            }
        }
        let mut c = self.shared.cfg.borrow_mut();
        c.device.format = self.chosen_format();
        c.device.template = template;
        c.device.kobo_shelf = self.shelf_row.text().trim().to_string();
        c.save();
        dev::set_settings(c.device.clone());
        drop(c);
        self.refresh_send_list();
    }

    fn save_dav(self: &Rc<Self>) {
        {
            let mut c = self.shared.cfg.borrow_mut();
            c.device.webdav.url = self.dav_url.text().trim().to_string();
            c.device.webdav.user = self.dav_user.text().trim().to_string();
            c.device.webdav.password = self.dav_pass.text().to_string();
            c.save();
            dev::set_settings(c.device.clone());
        }
        self.reload_devices();
    }

    fn check_dav(self: &Rc<Self>) {
        let s = self.shared.cfg.borrow().device.webdav.clone();
        self.dav_status.set_label(&tr("Connecting"));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let r = bookx_serve::webdav::WebDav::new(&s.url, &s.user, &s.password)
                .and_then(|d| d.check())
                .map(|_| tr("Connected."))
                .unwrap_or_else(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
        let label = self.dav_status.clone();
        poll(rx, move |msg| label.set_label(&msg));
    }

    // -- devices and books -----------------------------------------------------------------

    /// Looks for devices on a worker thread; the window stays usable meanwhile and a late answer
    /// (an older look, or the window closed) is dropped.
    fn reload_devices(self: &Rc<Self>) {
        let settings = self.shared.cfg.borrow().device.clone();
        self.scanning.set(true);
        if self.devices.borrow().is_empty() {
            let (_, storage) = ds::device_row_text("", &tr("Looking for devices"));
            self.device_row.set_property("subtitle", storage);
        }
        let me = self.clone();
        self.scanner.spawn(settings, true, move |found| {
            me.scanning.set(false);
            me.apply_scan(found);
        });
    }

    fn apply_scan(self: &Rc<Self>, found: dev::Detected) {
        let devices = dev::order_with_cards(found.devices);
        let previous = self.selected_device().map(|d| d.id);
        let names: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let keep = previous
            .and_then(|id| devices.iter().position(|d| d.id == id))
            .unwrap_or(0);
        *self.devices.borrow_mut() = devices;
        self.fill_others(found.others, found.chosen);
        // changing the model selects row 0, which reloads the books; the notify handler is
        // harmless here because `reload_books` below runs again for the final selection
        self.device_row
            .set_model(Some(&gtk::StringList::new(&refs)));
        self.device_row.set_selected(keep as u32);
        self.device_row.set_sensitive(!refs.is_empty());
        self.reload_books();
    }

    /// The "Other drives" list: drives that are not e-readers, each with "Use as an e-reader", and
    /// the drives the person chose, each with "Stop using as an e-reader".
    fn fill_others(self: &Rc<Self>, others: Vec<OtherDrive>, chosen: Vec<(DeviceInfo, String)>) {
        for r in self.other_rows.borrow_mut().drain(..) {
            self.others_group.remove(&r);
        }
        let mut rows: Vec<gtk::Widget> = Vec::new();
        for o in &others {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&o.name))
                .subtitle(glib::markup_escape_text(&other_subtitle(o)))
                .build();
            let b = gtk::Button::with_label(&tr("Use as an e-reader"));
            b.set_valign(gtk::Align::Center);
            let (me, key, name) = (self.clone(), o.key.clone(), o.name.clone());
            b.connect_clicked(move |_| me.ask_use_as_reader(&key, &name));
            row.add_suffix(&b);
            self.others_group.add(&row);
            rows.push(row.upcast());
        }
        for (d, key) in &chosen {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&d.name))
                .subtitle(tr("Used as an e-reader because you chose it"))
                .build();
            let b = gtk::Button::with_label(&tr("Stop using as an e-reader"));
            b.set_valign(gtk::Align::Center);
            let (me, key) = (self.clone(), key.clone());
            b.connect_clicked(move |_| me.stop_using(&key));
            row.add_suffix(&b);
            self.others_group.add(&row);
            rows.push(row.upcast());
        }
        self.others_group
            .set_visible(!others.is_empty() || !chosen.is_empty());
        *self.other_rows.borrow_mut() = rows;
        *self.others.borrow_mut() = others;
        *self.chosen.borrow_mut() = chosen;
    }

    /// Asks which kind of reader a drive is and where its books go.
    fn ask_use_as_reader(self: &Rc<Self>, key: &str, name: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(trf("Use {} as an e-reader?", &[&name]))
            .body(tr(
                "Bookx will offer this drive like any other e-reader. You can undo this under Other drives.",
            ))
            .build();
        let kinds = [
            DeviceKind::Kobo,
            DeviceKind::Kindle,
            DeviceKind::PocketBook,
            DeviceKind::Generic,
        ];
        let labels: Vec<String> = kinds
            .iter()
            .map(|k| match k {
                DeviceKind::Generic => tr("Other e-reader"),
                k => k.label().to_string(),
            })
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let kind_row = adw::ComboRow::builder()
            .title(tr("Kind of e-reader"))
            .build();
        kind_row.set_model(Some(&gtk::StringList::new(&refs)));
        kind_row.set_selected(3);
        let folder = adw::EntryRow::builder()
            .title(tr("Folder for books on the drive"))
            .build();
        folder.set_text(dev::default_folder(DeviceKind::Generic));
        {
            let folder = folder.clone();
            kind_row.connect_selected_notify(move |r| {
                let kind = kinds[(r.selected() as usize).min(3)];
                folder.set_text(dev::default_folder(kind));
            });
        }
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.append(&kind_row);
        list.append(&folder);
        dialog.set_extra_child(Some(&list));
        dialog.add_response("cancel", &tr("Cancel"));
        dialog.add_response("use", &tr("Use as an e-reader"));
        dialog.set_response_appearance("use", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("use"));
        dialog.set_close_response("cancel");
        {
            let (me, key) = (self.clone(), key.to_string());
            let (kind_row, folder) = (kind_row.clone(), folder.clone());
            dialog.connect_response(None, move |_, id| {
                *me.pending_use.borrow_mut() = None;
                if id == "use" {
                    let kind = kinds[(kind_row.selected() as usize).min(3)];
                    me.use_as_reader(&key, kind, &folder.text());
                }
            });
        }
        *self.pending_use.borrow_mut() = Some((dialog.clone(), kind_row, folder, key.to_string()));
        dialog.present(Some(&self.dialog));
    }

    /// Remembers that the drive is an e-reader of `kind` (the choice is the person's, never an
    /// automatic recognition) and looks again.
    fn use_as_reader(self: &Rc<Self>, key: &str, kind: DeviceKind, folder: &str) {
        self.store_drives(|d| {
            dev::set_user_drive(d, key, Some((kind, dev::folder_choice(kind, folder))))
        });
        self.reload_devices();
    }

    fn stop_using(self: &Rc<Self>, key: &str) {
        self.store_drives(|d| dev::set_user_drive(d, key, None));
        self.reload_devices();
    }

    fn store_drives(
        &self,
        f: impl FnOnce(Vec<bookx_serve::devices::UserDrive>) -> Vec<bookx_serve::devices::UserDrive>,
    ) {
        let mut c = self.shared.cfg.borrow_mut();
        c.device.drives = f(std::mem::take(&mut c.device.drives));
        c.save();
        dev::set_settings(c.device.clone());
    }

    fn selected_device(&self) -> Option<DeviceInfo> {
        self.devices
            .borrow()
            .get(self.device_row.selected() as usize)
            .cloned()
    }

    /// Lists the books of the selected device on a worker thread.
    fn reload_books(self: &Rc<Self>) {
        let r#gen = self.generation.get() + 1;
        self.generation.set(r#gen);
        self.states.borrow_mut().clear();
        let Some(device) = self.selected_device() else {
            let (_, storage) = ds::device_row_text("", &tr(NO_DEVICE_SHORT));
            self.device_row.set_property("subtitle", storage);
            self.no_device_page.set_visible(true);
            self.books_content.set_visible(false);
            self.fill_books(&[]);
            *self.last_summary.borrow_mut() = String::new();
            self.shelf_row.set_visible(false);
            self.remove_btn.set_sensitive(false);
            self.sync_btn.set_sensitive(false);
            self.loading.set(false);
            self.books.borrow_mut().clear();
            self.refresh_send_list();
            return;
        };
        self.no_device_page.set_visible(false);
        self.books_content.set_visible(true);
        self.loading.set(true);
        self.status.set_label(&tr("Reading the device"));
        let device_name = device.name.clone();
        let settings = self.shared.cfg.borrow().device.clone();
        let drivers = dev::drivers(&settings);
        let (_, storage) = ds::device_row_text(&device_name, &tr("Reading the device"));
        self.device_row.set_property("subtitle", storage);
        self.shelf_row
            .set_visible(drivers.kind(&device) == Some(DeviceKind::Kobo));
        self.remove_btn.set_sensitive(true);
        self.sync_btn.set_sensitive(true);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let d = dev::drivers(&settings);
            let summary = d.summary(&device);
            let shelves = d.shelf_map(&device);
            let books = d.books(&device).map_err(|e| format!("{e:#}"));
            let _ = tx.send((summary, shelves, books));
        });
        let me = self.clone();
        poll(rx, move |(summary, shelves, books)| {
            if me.generation.get() != r#gen {
                return;
            }
            let (_, storage) = ds::device_row_text(&device_name, &summary);
            me.device_row.set_property("subtitle", storage);
            *me.last_summary.borrow_mut() = summary;
            *me.shelves.borrow_mut() = shelves;
            me.loading.set(false);
            match books {
                Ok(b) => {
                    me.status.set_label(&match b.len() {
                        0 => tr("There are no books on this device."),
                        n => ntr("{} book", "{} books", n),
                    });
                    me.fill_books(&b);
                    *me.books.borrow_mut() = b;
                    me.refresh_send_list();
                }
                Err(e) => {
                    me.status.set_label(&e);
                    me.fill_books(&[]);
                    me.books.borrow_mut().clear();
                    me.refresh_send_list();
                }
            }
        });
    }

    fn fill_books(self: &Rc<Self>, books: &[DeviceBook]) {
        let items: Vec<Item> = books
            .iter()
            .map(|b| Item {
                title: b.stem(),
                lines: vec![self.subtitle(b)],
                ..Default::default()
            })
            .collect();
        self.on_dev.set_items(&items);
        self.start_covers(books);
        self.update_buttons();
    }

    /// Reads the covers on a worker thread (cached on disk) and shows them as they arrive.
    fn start_covers(self: &Rc<Self>, books: &[DeviceBook]) {
        let r#gen = self.generation.get();
        let books: Vec<DeviceBook> = books.to_vec();
        let (tx, rx) = std::sync::mpsc::channel::<(usize, Option<PathBuf>)>();
        std::thread::spawn(move || {
            let dir = crate::device_covers::cache_dir();
            for (i, b) in books.iter().enumerate() {
                if tx.send((i, crate::device_covers::cached(&dir, b))).is_err() {
                    break;
                }
            }
        });
        let me = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
            if me.generation.get() != r#gen {
                return glib::ControlFlow::Break;
            }
            for _ in 0..12 {
                match rx.try_recv() {
                    Ok((i, Some(file))) => me.on_dev.set_cover(i, &file),
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn subtitle(&self, b: &DeviceBook) -> String {
        let mut s = format!("{} · {}", b.rel, human_size(b.size));
        if let Some(shelves) = self.shelves.borrow().get(&b.path) {
            s.push_str(&format!(" · {}", trf("Shelf: {}", &[&shelves.join(", ")])));
        }
        if let Some(st) = self.states.borrow().get(&b.path) {
            s.push_str(&format!(
                " · {} {}% ({})",
                st.status.label(),
                (st.progress * 100.0).round() as u32,
                st.source
            ));
        }
        s
    }

    fn selected_books(&self) -> Vec<DeviceBook> {
        let books = self.books.borrow();
        self.on_dev
            .checked()
            .iter()
            .zip(books.iter())
            .filter(|(c, _)| **c)
            .map(|(_, b)| b.clone())
            .collect()
    }

    // -- actions ---------------------------------------------------------------------------

    fn remove_selected(self: &Rc<Self>, confirm: bool) {
        let picked = self.selected_books();
        if picked.is_empty() {
            self.status
                .set_label(&tr("Tick the books to remove first."));
            return;
        }
        let Some(device) = self.selected_device() else {
            return;
        };
        let n = picked.len();
        if !confirm {
            self.do_remove(device, picked);
            return;
        }
        let me = self.clone();
        super::alert(
            &self.dialog,
            &ds::remove_confirmation_title(n),
            &tr(REMOVE_DESCRIPTION),
            &tr("Remove from Device"),
            true,
            move || me.do_remove(device.clone(), picked.clone()),
        );
    }

    fn do_remove(self: &Rc<Self>, device: DeviceInfo, picked: Vec<DeviceBook>) {
        self.status.set_label(&tr("Removing"));
        let settings = self.shared.cfg.borrow().device.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let d = dev::drivers(&settings);
            let _ = tx.send(dev::remove_books(&d, &device, &picked));
        });
        let me = self.clone();
        poll(rx, move |(ok, errors)| {
            me.reload_books();
            let msg = if errors.is_empty() {
                trf("Removed {}.", &[&ntr("{} book", "{} books", ok)])
            } else {
                trf(
                    "Removed {}; could not remove: {}",
                    &[&ok, &errors.join("; ")],
                )
            };
            // test hook: the device scenarios watch the result of a removal
            eprintln!("devwin_removed: ok={ok} failed={}", errors.len());
            let status = me.status.clone();
            // `reload_books` set its own text; show ours once it has finished
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                status.set_label(&msg)
            });
        });
    }

    /// Syncs reading progress and notes; `ask` shows the summary first (the button), the test hook
    /// applies straight away.
    fn sync_state(self: &Rc<Self>, ask: bool) {
        let Some(device) = self.selected_device() else {
            return;
        };
        self.status.set_label(&tr("Reading the reading state"));
        let status = self.status.clone();
        let me = self.clone();
        let seen = self.clone();
        start_sync(
            &self.dialog,
            &self.shared,
            device,
            if ask { SyncMode::Ask } else { SyncMode::Auto },
            Rc::new(move |m| status.set_label(&m)),
            Rc::new(move || (me.on_changed)()),
            Rc::new(move |readings: &[crate::device_sync::DeviceReading]| {
                *seen.states.borrow_mut() = readings
                    .iter()
                    .filter_map(|r| r.state.clone().map(|s| (r.path.clone(), s)))
                    .collect();
                let books = seen.books.borrow().clone();
                seen.fill_books(&books);
            }),
        );
    }

    /// The books to send: the ones selected in the library when the window opened.
    fn set_send_ids(self: &Rc<Self>, ids: Vec<u64>) {
        *self.send_ids.borrow_mut() = ids;
        self.refresh_send_list();
    }

    /// Works out, for every book to send, what will happen to it (the same decisions the real send
    /// takes, see `devices::plan_send`) and shows it. Ticks are kept for books that were there.
    fn refresh_send_list(self: &Rc<Self>) {
        let ids = self.send_ids.borrow().clone();
        // what was ticked, and whether the book had a warning then
        let before: HashMap<u64, (bool, bool)> = self
            .plans
            .borrow()
            .iter()
            .zip(self.to_send.checked())
            .map(|((id, plan), c)| (*id, (c, plan.warnings.is_empty())))
            .collect();
        let settings = self.shared.cfg.borrow().device.clone();
        let device = self.selected_device();
        let drivers = dev::drivers(&settings);
        let chain = match &device {
            Some(d) => drivers.chain(d, &self.chosen_format()),
            None => dev::format_chain(None, &self.chosen_format()),
        };
        let free = device
            .as_ref()
            .and_then(|d| drivers.space(d))
            .map(|s| s.free);
        let on_device = self.books.borrow().clone();
        let ctx = dev::SendContext {
            chain: &chain,
            template: &settings.template,
            free,
            on_device: &on_device,
        };
        let include_notes = self.notes_row.is_active();
        let mut items = Vec::new();
        let mut plans = Vec::new();
        {
            let lib = self.shared.lib.borrow();
            for id in ids {
                let Some(b) = lib.get(id) else {
                    continue;
                };
                let fields = dev::fields_of(&b.meta);
                let cand = dev::SendCandidate {
                    fields: &fields,
                    size: b.size,
                    notes: if include_notes {
                        b.highlights.len() + b.bookmarks.len()
                    } else {
                        0
                    },
                    file_missing: !lib.book_path(b).exists(),
                };
                let plan = dev::plan_send(&ctx, &|t| crate::plugins::convert_is_hooked(t), &cand);
                let mut lines = vec![b.meta.authors.join(" & ")];
                lines.push(if device.is_some() {
                    plan.format_line()
                } else {
                    tr("Connect a device to see the format")
                });
                if device.is_some() {
                    lines.push(plan.notes_line());
                }
                lines.push(human_size(b.size));
                items.push(Item {
                    title: b.meta.title.clone(),
                    lines,
                    warning: (!plan.warnings.is_empty()).then(|| {
                        plan.warnings
                            .iter()
                            .map(|w| w.text())
                            .collect::<Vec<_>>()
                            .join(" ")
                    }),
                    // the user's tick is kept unless the book's warnings changed
                    checked: match before.get(&id) {
                        Some((c, clean)) if *clean == plan.warnings.is_empty() => *c,
                        _ => plan.default_included(),
                    },
                    cover: lib.cover_path(b),
                    placeholder_seed: String::new(),
                });
                plans.push((id, plan));
            }
        }
        *self.plans.borrow_mut() = plans;
        self.to_send.set_items(&items);
        let empty = items.is_empty();
        self.empty_send.set_visible(empty);
        self.send_menu_button_visible(!empty);
        self.send_status.set_visible(!empty);
        self.send_btn.set_visible(!empty);
        self.update_buttons();
    }

    /// The group header menu and the Send button are only useful when there is something to send.
    fn send_menu_button_visible(&self, visible: bool) {
        if let Some(suffix) = self.send_group.header_suffix() {
            suffix.set_visible(visible);
        }
    }

    fn update_buttons(&self) {
        let n = self.to_send.selected();
        self.send_btn.set_label(&ds::send_button_label(n));
        let can = n > 0 && self.selected_device().is_some() && !self.sending.get();
        self.send_btn.set_sensitive(can);
        self.send_btn.set_tooltip_text(Some(
            &(if self.selected_device().is_none() {
                tr("Connect a device first")
            } else if n == 0 {
                tr("Tick the books to send")
            } else {
                tr("Send the ticked books to the device")
            }),
        ));
        let m = self.on_dev.selected();
        self.books_group
            .set_description(Some(&selection_description(self.on_dev.len(), m)));
        self.remove_btn.set_label(&ds::remove_button_label(m));
        let (all, clear, remove) = ds::selection_controls_state(self.on_dev.len(), m);
        self.on_all.set_enabled(all);
        self.on_none.set_enabled(clear);
        let (send_all, send_clear, _) = ds::selection_controls_state(self.to_send.len(), n);
        self.send_all.set_enabled(send_all);
        self.send_none.set_enabled(send_clear);
        self.remove_btn
            .set_sensitive(remove && self.selected_device().is_some());
    }

    /// Sends the ticked books of the "Books to send" list.
    fn send_checked(self: &Rc<Self>) {
        let Some(device) = self.selected_device() else {
            return;
        };
        let ids: Vec<u64> = self
            .plans
            .borrow()
            .iter()
            .map(|(id, _)| *id)
            .zip(self.to_send.checked())
            .filter(|(_, c)| *c)
            .map(|(id, _)| id)
            .collect();
        if ids.is_empty() {
            self.send_status
                .set_label(&tr("Tick the books to send first."));
            return;
        }
        // the optional check (Preferences → Library): the window opens only when something was
        // found, and Continue (or a clean result) goes on with the same books
        if self.shared.cfg.borrow().metadata_prefs.preflight {
            let (me, dev) = (self.clone(), device.clone());
            let dest = crate::preflight::Dest::for_target(&self.chosen_format());
            let go_ids = ids.clone();
            let go: Rc<dyn Fn()> = Rc::new(move || me.send_now(dev.clone(), go_ids.clone()));
            let me = self.clone();
            crate::ui::preflight::show(
                self.parent.upcast_ref(),
                &self.shared,
                ids,
                dest,
                Some(go),
                Rc::new(move |msg| me.send_status.set_label(&msg)),
            );
            return;
        }
        self.send_now(device, ids);
    }

    fn send_now(self: &Rc<Self>, device: DeviceInfo, ids: Vec<u64>) {
        self.sending.set(true);
        self.update_buttons();
        self.send_status.set_label(&tr("Sending"));
        let me = self.clone();
        send_books(
            &self.shared,
            device,
            ids,
            self.notes_row.is_active(),
            move |msg| {
                // test hook: the device scenarios watch what was sent
                eprintln!("devwin_sent: {msg}");
                me.sending.set(false);
                me.send_status.set_label(&msg);
                me.reload_books();
            },
        );
    }

    fn dump(&self) {
        let sel = self
            .selected_device()
            .map(|d| d.name)
            .unwrap_or_else(|| "none".into());
        let fmt = self.chosen_format();
        eprintln!(
            "devwin: devices={} selected={} books={} loading={} format={} template={} template_error={} shelf_row={} summary={} status={}",
            self.devices.borrow().len(),
            sel,
            self.books.borrow().len(),
            self.loading.get(),
            if fmt.is_empty() { "auto" } else { &fmt },
            self.template_row.text(),
            self.template_row.has_css_class("error"),
            self.shelf_row.is_visible(),
            self.last_summary.borrow(),
            self.status.label(),
        );
        for d in self.devices.borrow().iter() {
            eprintln!(
                "devwin_device: {} | card={} | user_chosen={}",
                d.name,
                d.parent.is_some(),
                bookx_serve::devices::MassStorageDriver::is_user_chosen(d)
            );
        }
        eprintln!(
            "devwin_others: group_visible={} others={} chosen={} scanning={} names={:?}",
            self.others_group.is_visible(),
            self.others.borrow().len(),
            self.chosen.borrow().len(),
            self.scanning.get(),
            self.others
                .borrow()
                .iter()
                .map(|o| o.name.clone())
                .collect::<Vec<_>>()
        );
        eprintln!(
            "devwin_ui: view={} covers={} include_notes={} auto_sync={} send_button={} send_enabled={}",
            self.on_dev.visible_view(),
            self.on_dev.covers_shown(),
            self.notes_row.is_active(),
            self.auto_row.is_active(),
            self.send_btn.label().unwrap_or_default(),
            self.send_btn.is_sensitive()
        );
        eprintln!(
            "devwin_lists: to_send={} checked={} on_device={} checked={}",
            self.to_send.len(),
            self.to_send.selected(),
            self.on_dev.len(),
            self.on_dev.selected()
        );
        let checked = self.to_send.checked();
        for (i, (id, plan)) in self.plans.borrow().iter().enumerate() {
            let title = self
                .shared
                .lib
                .borrow()
                .get(*id)
                .map(|b| b.meta.title.clone())
                .unwrap_or_default();
            let warns: Vec<String> = plan.warnings.iter().map(|w| w.text()).collect();
            eprintln!(
                "devwin_send_item: {title} | {} | {} | {} | checked={}",
                plan.format_line(),
                plan.notes_line(),
                warns.join(" "),
                checked.get(i).copied().unwrap_or(false)
            );
        }
        for (name, list) in [("to_send", &self.to_send), ("on_device", &self.on_dev)] {
            let l = list.layout();
            let xs: std::collections::BTreeSet<i32> = l.iter().map(|r| r.0).collect();
            let cx: std::collections::BTreeSet<i32> = l.iter().map(|r| r.1).collect();
            let cw: std::collections::BTreeSet<i32> = l.iter().map(|r| r.2).collect();
            let ordered = l.iter().all(|r| r.0 >= 0 && r.0 < r.1);
            eprintln!(
                "devwin_align: {name} rows={} check_x={xs:?} cover_x={cx:?} cover_w={cw:?} check_before_cover={ordered}",
                l.len()
            );
        }
        let summary = self.last_summary.borrow();
        eprintln!(
            "devwin_hint: {}",
            if summary.is_empty() {
                no_device_hint()
            } else {
                summary.clone()
            }
        );
        let books = self.books.borrow().clone();
        for b in &books {
            eprintln!("devwin_book: {}", self.subtitle(b));
        }
        self.dump_layout();
    }

    /// The HIG layout report (`devwin:layout`, also printed by `devwin:dump`).
    fn dump_grid(&self) {
        let rel = |w: &gtk::Widget| {
            w.compute_bounds(&self.books_content)
                .map(|b| (b.y().round() as i32, (b.y() + b.height()).round() as i32))
                .unwrap_or((0, 0))
        };
        let group_gap = if self.send_group.is_visible() {
            rel(self.send_group.upcast_ref()).0 - rel(self.books_group.upcast_ref()).1
        } else {
            0
        };
        let status_overlaps = rel(self.status.upcast_ref()).0
            < rel(self.on_dev.widget.upcast_ref()).1
            && self.on_dev.widget.is_visible();
        let send_status_overlaps = self.to_send.widget.is_visible()
            && rel(self.send_status.upcast_ref()).0 < rel(self.to_send.widget.upcast_ref()).1;
        let remove_gap = rel(self.remove_btn.upcast_ref()).0 - rel(self.status.upcast_ref()).1;
        for (name, list, overlaps) in [
            ("on_device", &self.on_dev, status_overlaps),
            ("to_send", &self.to_send, send_status_overlaps),
        ] {
            let g = list.grid_report();
            eprintln!(
                "devwin_grid: {name} columns={} tile_w={} tile_h={} caption_lines={} clipped={} status_overlaps={overlaps} group_gap={group_gap} remove_gap={remove_gap} cover_w={} card_w={}",
                g.columns,
                g.tile_w,
                g.tile_h,
                g.caption_lines,
                g.clipped,
                list.cover_width(),
                super::book_tile::card_width(list.cover_width()),
            );
        }
    }

    fn dump_layout(&self) {
        eprintln!(
            "devwin_dialog: title={} pages={},{} active={} linked_toggles={} content_height={} dialog_alloc={}x{} parent_alloc={}x{}",
            self.dialog.title(),
            self.books_page.title().unwrap_or_default(),
            self.settings_page.title().unwrap_or_default(),
            self.stack.visible_child_name().unwrap_or_default(),
            linked_toggles(self.root.upcast_ref::<gtk::Widget>()),
            self.dialog.content_height(),
            self.dialog.width(),
            self.dialog.height(),
            self.parent.width(),
            self.parent.height()
        );
        eprintln!(
            "devwin_status: title={} description={} icon=drive-removable-media-symbolic visible={}",
            ds::no_device_status_title(),
            ds::no_device_status_description(),
            self.no_device_page.is_visible()
        );
        eprintln!(
            "devwin_device_row: title={} subtitle={} selected={}",
            self.device_row.title(),
            self.device_row.property::<String>("subtitle"),
            self.device_row
                .selected_item()
                .and_then(|i| i.downcast::<gtk::StringObject>().ok())
                .map(|s| s.string().to_string())
                .unwrap_or_default()
        );
        eprintln!(
            "devwin_buttons: send={} send_enabled={} remove={} remove_enabled={}",
            self.send_btn.label().unwrap_or_default(),
            self.send_btn.is_sensitive(),
            self.remove_btn.label().unwrap_or_default(),
            self.remove_btn.is_sensitive()
        );
        let (shown, hidden) = self.on_dev.checkbox_visibility();
        let (ss, sh) = self.to_send.checkbox_visibility();
        eprintln!(
            "devwin_selection_mode: class={} on_device={} of {}, to_send={} of {}",
            self.on_dev.has_selection_mode_class(),
            shown,
            shown + hidden,
            ss,
            ss + sh
        );
        eprintln!(
            "devwin_menus: on_device=[{}] to_send=[{}]",
            menu_labels(&self.on_dev_menu).join(", "),
            menu_labels(&self.send_menu).join(", ")
        );
        self.dump_grid();
        let (icon, state_name) = ds::view_toggle_state(self.view_toggle.is_active());
        eprintln!(
            "devwin_view_toggle: active={} icon={} state={} action={}",
            self.view_toggle.is_active(),
            icon,
            state_name,
            self.view_toggle.tooltip_text().unwrap_or_default()
        );
    }
}

fn settings_view_is_list(shared: &SharedRef) -> bool {
    shared.cfg.borrow().device_list_view
}

/// Converts the library books for the device and copies them over, on a worker thread. The
/// highlights and notes of a book are appended when `include_notes` is set. `done` gets the
/// message to show.
pub fn send_books(
    shared: &SharedRef,
    device: DeviceInfo,
    ids: Vec<u64>,
    include_notes: bool,
    done: impl FnOnce(String) + 'static,
) {
    let settings = shared.cfg.borrow().device.clone();
    // every format file of a book: the one that suits the device is picked below
    type Job = (
        Vec<(PathBuf, crate::core::formats::FormatFile)>,
        devfs::BookFields,
        Option<crate::send_notes::Notes>,
    );
    let jobs: Vec<Job> = {
        let lib = shared.lib.borrow();
        ids.iter()
            .filter_map(|id| {
                lib.get(*id).map(|b| {
                    let notes = include_notes
                        .then(|| crate::send_notes::Notes::new(&b.highlights, &b.bookmarks))
                        .filter(|n| !n.is_empty());
                    let files = b
                        .all_formats()
                        .into_iter()
                        .map(|f| (lib.root.join(&f.file), f))
                        .collect();
                    (files, dev::fields_of(&b.meta), notes)
                })
            })
            .collect()
    };
    let name = device.name.clone();
    // the send shows in the Jobs panel like every other long task
    let (job, _) = crate::jobs::start(
        crate::jobs::Kind::Send,
        &trf(
            "Sending {} to {}",
            &[&ntr("{} book", "{} books", ids.len()), &name],
        ),
        false,
    );
    let (tx, rx) = std::sync::mpsc::channel::<(String, bool)>();
    std::thread::spawn(move || {
        let drivers = dev::drivers(&settings);
        let opts = dev::SendOptions::from(&settings);
        let (mut ok, mut failed, mut notes_out) = (0, Vec::new(), Vec::new());
        let target = crate::book_formats::target_for_device(drivers.kind(&device));
        for (files, fields, notes) in &jobs {
            let path = &crate::book_formats::pick_path(files, target);
            match dev::send_full(&drivers, &device, path, fields, &opts, notes.as_ref()) {
                Ok(sent) => {
                    ok += 1;
                    if !sent.fell_back_from.is_empty() {
                        notes_out.push(trf(
                            "{} was sent as {} because {}",
                            &[&fields.title, &sent.format, &sent.fell_back_from.join("; ")],
                        ));
                    }
                    notes_out.extend(sent.note);
                }
                Err(e) => failed.push(format!("{}: {e:#}", fields.title)),
            }
        }
        let mut msg = if failed.is_empty() {
            trf("Sent {} to {}", &[&ntr("{} book", "{} books", ok), &name])
        } else {
            trf("Could not send: {}", &[&failed.join("; ")])
        };
        if !notes_out.is_empty() {
            msg.push_str(&format!(" ({})", notes_out.join("; ")));
        }
        let _ = tx.send((msg, !failed.is_empty()));
    });
    poll(rx, move |(msg, failed): (String, bool)| {
        crate::jobs::finish(job, if failed { Err(msg.clone()) } else { Ok(()) });
        done(msg)
    });
}

/// How a sync starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncMode {
    /// Show what will happen and ask.
    Ask,
    /// The user switched automatic sync on: do it and report.
    Auto,
}

/// Reads the device, works out what a sync would do (nothing is changed yet), then either shows
/// the summary and waits for the user, or (automatic) applies it. `toast` gets the messages,
/// `changed` is called when the library changed, `seen` gets what the device holds.
pub fn start_sync(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    device: DeviceInfo,
    mode: SyncMode,
    toast: Rc<dyn Fn(String)>,
    changed: Rc<dyn Fn()>,
    seen: Rc<dyn Fn(&[crate::device_sync::DeviceReading])>,
) {
    let settings = shared.cfg.borrow().device.clone();
    let (books, locator) = {
        let lib = shared.lib.borrow();
        let mut books = ds::library_books(&lib.books);
        ds::attach_looks(&mut books, &lib.root);
        (books, ds::book_locator(&lib))
    };
    let name = device.name.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let d = dev::drivers(&settings);
        let r = ds::read_device(&d, &device)
            .map(|readings| {
                let plan = ds::plan(&readings, &books, &locator);
                (readings, plan)
            })
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
    });
    let parent = parent.clone().upcast::<gtk::Widget>();
    let shared = shared.clone();
    poll(rx, move |r| {
        let (readings, plan) = match r {
            Ok(v) => v,
            Err(e) => {
                toast(trf("Could not read {}: {}", &[&name, &e]));
                return;
            }
        };
        seen(&readings);
        let summary = plan.summary();
        for (title, from, to) in &summary.progress {
            let source = plan
                .books
                .iter()
                .find(|b| &b.title == title)
                .and_then(|b| b.progress.as_ref().map(|p| p.source))
                .unwrap_or("");
            // test hook: the sync scenarios check which readings moved
            eprintln!(
                "devwin_sync_item: {title} {:.0}% -> {:.0}% ({source})",
                from * 100.0,
                to * 100.0
            );
        }
        // test hook: the sync scenarios check the counts
        eprintln!(
            "devwin_sync: read={} updated={}",
            readings.iter().filter(|r| r.state.is_some()).count(),
            summary.progress.len()
        );
        for l in summary.lines() {
            eprintln!("sync_summary: {l}");
        }
        if plan.is_empty() && summary.conflicts.is_empty() {
            let mut m = trf("{}: already up to date", &[&name]);
            if summary.unmatched > 0 {
                m.push_str(&trf(
                    " ({} on the device not in your library)",
                    &[&summary.unmatched],
                ));
            }
            toast(m);
            return;
        }
        let plan = Rc::new(plan);
        let apply: Rc<dyn Fn()> = {
            let (shared, toast, changed, plan, name) = (
                shared.clone(),
                toast.clone(),
                changed.clone(),
                plan.clone(),
                name.clone(),
            );
            Rc::new(move || {
                let done = {
                    let mut lib = shared.lib.borrow_mut();
                    let snap = crate::sync_bus::Snapshot::take(&lib.books);
                    let a = ds::apply_to_library(&mut lib, &plan);
                    let _ = lib.save();
                    crate::sync_bus::publish_changes(
                        &snap,
                        &lib.books,
                        &name,
                        crate::sync_bus::Via::Cable,
                    );
                    a
                };
                if done.progress + done.highlights + done.notes > 0 {
                    changed();
                }
                // the device side on a worker thread: new notes files only, none is overwritten
                let plan2: ds::Plan = (*plan).clone();
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(ds::apply_to_device(&plan2, &mut ds::SidecarSink));
                });
                let (toast, name) = (toast.clone(), name.clone());
                poll(rx, move |(written, errors)| {
                    let mut parts = Vec::new();
                    if done.progress > 0 {
                        parts.push(trf(
                            "progress of {}",
                            &[&ntr("{} book", "{} books", done.progress)],
                        ));
                    }
                    if done.highlights + done.notes > 0 {
                        parts.push(trf(
                            "{} (highlights and notes) from the device",
                            &[&ntr("{} item", "{} items", done.highlights + done.notes)],
                        ));
                    }
                    if written > 0 {
                        parts.push(trf("{} to the device", &[&written]));
                    }
                    let mut m = if parts.is_empty() {
                        trf("{}: nothing needed to change", &[&name])
                    } else {
                        trf("{} synced: {}", &[&name, &parts.join(", ")])
                    };
                    if !errors.is_empty() {
                        m.push_str(&trf(" (problems: {})", &[&errors.join("; ")]));
                    }
                    // test hook: the sync scenarios check that the second plan is empty
                    eprintln!("sync_done: {m}");
                    toast(m);
                });
            })
        };
        match mode {
            SyncMode::Auto => apply(),
            SyncMode::Ask => {
                PENDING_SYNC.with(|p| *p.borrow_mut() = Some(apply.clone()));
                super::alert(
                    &parent,
                    &trf("Sync with {}?", &[&name]),
                    &summary.text(),
                    &tr("Sync"),
                    false,
                    move || {
                        PENDING_SYNC.with(|p| p.borrow_mut().take());
                        apply()
                    },
                );
            }
        }
    });
}

/// Calls `done` on the main loop with the first message of `rx`.
fn poll<T: 'static>(rx: std::sync::mpsc::Receiver<T>, done: impl FnOnce(T) + 'static) {
    let done = RefCell::new(Some(done));
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(v) => {
                if let Some(f) = done.borrow_mut().take() {
                    f(v);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Test hooks (`devwin:<command>` in `BOOKX_DEV_SCRIPT`): `show`, `dump`, `layout`,
/// `page=books|settings`, `device=<n>`, `select=<i,j>`, `remove`, `sync`, `format=<id>`,
/// `template=<text>`, `shelf=<name>`, `reload`, `dav=<url>`, `view=grid|list`, `notes=on|off`,
/// `autosync=on|off`, `syncask` (summary dialog), `syncconfirm`, `send` (the library's
/// selection), `use=<drive>:<kind>[:<folder>]`, `stop=<device>`, `usedialog=<drive>`,
/// `usedialog-answer=<kind>:<folder>`, `usedialog-cancel`, `reload-settings`, `close`.
pub fn dev_command(
    parent: &adw::ApplicationWindow,
    shared: &SharedRef,
    on_changed: Rc<dyn Fn()>,
    cmd: &str,
) {
    if cmd == "show" {
        show(parent, shared, on_changed);
        return;
    }
    let Some(w) = CURRENT.with(|c| c.borrow().clone()) else {
        eprintln!("devwin: not open");
        return;
    };
    match cmd {
        "dump" => w.dump(),
        "layout" => w.dump_layout(),
        "icons" => {
            let mut seen: Vec<String> = Vec::new();
            if let Some(c) = w.dialog.child() {
                super::library::collect_icons(&c, &mut seen);
            }
            for page in [&w.books_page, &w.settings_page] {
                if let Some(n) = page.icon_name() {
                    if !seen.contains(&n.to_string()) {
                        seen.push(n.to_string());
                    }
                }
            }
            let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap());
            let missing: Vec<&String> = seen.iter().filter(|n| !theme.has_icon(n)).collect();
            let pages: Vec<String> = [&w.books_page, &w.settings_page]
                .iter()
                .map(|p| p.icon_name().map(|n| n.to_string()).unwrap_or_default())
                .collect();
            eprintln!(
                "devwin_icons: checked={} pages={} missing={:?}",
                seen.len(),
                pages.join(","),
                missing
            );
        }
        "reload" => w.reload_devices(),
        // the settings as they are on disk, as after a restart of the program
        "reload-settings" => {
            let saved = crate::config::Config::load().device;
            // what is on disk: the person's choices, by hashed key
            eprintln!(
                "devwin_settings_file: {}",
                serde_json::to_string(&saved.drives).unwrap_or_default()
            );
            w.shared.cfg.borrow_mut().device.drives = saved.drives;
            dev::set_settings(w.shared.cfg.borrow().device.clone());
            w.reload_devices();
        }
        "usedialog-cancel" => {
            if let Some((d, ..)) = w.pending_use.borrow().as_ref() {
                d.close();
            }
        }
        "remove" => w.remove_selected(false),
        "sync" => w.sync_state(false),
        "syncask" => w.sync_state(true),
        "syncconfirm" => match PENDING_SYNC.with(|p| p.borrow_mut().take()) {
            Some(f) => {
                f();
            }
            _ => {
                eprintln!("sync_summary: no dialog");
            }
        },
        "send" => w.send_checked(),
        "selectall" => w.on_dev.set_all(true),
        "none" => w.on_dev.set_all(false),
        "close" => {
            w.dialog.close();
        }
        other => {
            if let Some(n) = other.strip_prefix("device=") {
                // an index, or a part of the device's name
                let idx = n.parse().ok().or_else(|| {
                    w.devices
                        .borrow()
                        .iter()
                        .position(|d| d.name.contains(n) || d.mount.to_string_lossy().contains(n))
                });
                w.device_row.set_selected(idx.unwrap_or(0) as u32);
            } else if let Some(arg) = other.strip_prefix("use=") {
                // `use=<part of the drive's name>:<kind>[:<folder>]` is "Use as an e-reader"
                let mut it = arg.splitn(3, ':');
                let (part, kind, folder) = (
                    it.next().unwrap_or(""),
                    it.next().unwrap_or("generic"),
                    it.next().unwrap_or(""),
                );
                let key = w
                    .others
                    .borrow()
                    .iter()
                    .find(|o| o.name.contains(part))
                    .map(|o| o.key.clone());
                match (key, DeviceKind::from_slug(kind)) {
                    (Some(k), Some(kind)) => w.use_as_reader(&k, kind, folder),
                    _ => eprintln!("devwin_use: no such drive {part}"),
                }
            } else if let Some(part) = other.strip_prefix("stop=") {
                let key = w
                    .chosen
                    .borrow()
                    .iter()
                    .find(|(d, _)| {
                        d.name.contains(part) || d.mount.to_string_lossy().contains(part)
                    })
                    .map(|(_, k)| k.clone());
                match key {
                    Some(k) => w.stop_using(&k),
                    None => eprintln!("devwin_stop: no such drive {part}"),
                }
            } else if let Some(part) = other.strip_prefix("usedialog=") {
                // opens the real dialog for a drive of Other drives
                let found = w
                    .others
                    .borrow()
                    .iter()
                    .find(|o| o.name.contains(part))
                    .map(|o| (o.key.clone(), o.name.clone()));
                match found {
                    Some((k, n)) => {
                        w.ask_use_as_reader(&k, &n);
                        if let Some((d, ..)) = w.pending_use.borrow().as_ref() {
                            eprintln!("devwin_usedialog: {}", d.heading().unwrap_or_default());
                        }
                    }
                    None => eprintln!("devwin_usedialog: no such drive {part}"),
                }
            } else if let Some(arg) = other.strip_prefix("usedialog-answer=") {
                // `usedialog-answer=<kind>:<folder>` fills the open dialog and presses the button
                let (kind, folder) = arg.split_once(':').unwrap_or((arg, ""));
                let pending = w.pending_use.borrow().clone();
                if let Some((d, kind_row, folder_row, _)) = pending {
                    let idx = ["kobo", "kindle", "pocketbook", "generic"]
                        .iter()
                        .position(|k| *k == kind)
                        .unwrap_or(3);
                    kind_row.set_selected(idx as u32);
                    folder_row.set_text(folder);
                    d.emit_by_name::<()>("response", &[&"use"]);
                    d.force_close();
                } else {
                    eprintln!("devwin_usedialog: not open");
                }
            } else if let Some(list) = other.strip_prefix("select=") {
                let idx: Vec<usize> = list.split(',').filter_map(|s| s.parse().ok()).collect();
                for i in 0..w.on_dev.len() {
                    w.on_dev.set_checked(i, idx.contains(&i));
                }
            } else if let Some(list) = other.strip_prefix("sendselect=") {
                let idx: Vec<usize> = list.split(',').filter_map(|s| s.parse().ok()).collect();
                for i in 0..w.to_send.len() {
                    w.to_send.set_checked(i, idx.contains(&i));
                }
            } else if let Some(arg) = other.strip_prefix("click=") {
                // `click=<send|device>:<index>[:shift]` is a click on a row or a tile
                let mut it = arg.split(':');
                let list = if it.next() == Some("send") {
                    &w.to_send
                } else {
                    &w.on_dev
                };
                let i = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                list.click(i, it.next() == Some("shift"));
            } else if let Some(name) = other.strip_prefix("page=") {
                if name == "books" || name == "settings" {
                    w.stack.set_visible_child_name(name);
                    eprintln!("devwin_page: {name}");
                } else {
                    eprintln!("devwin_page: unknown={name}");
                }
            } else if let Some(v) = other.strip_prefix("view=") {
                w.view_toggle.set_active(v == "list");
            } else if let Some(v) = other.strip_prefix("notes=") {
                w.notes_row.set_active(v == "on");
            } else if let Some(v) = other.strip_prefix("autosync=") {
                w.auto_row.set_active(v == "on");
            } else if let Some(f) = other.strip_prefix("format=") {
                w.select_format(f);
            } else if let Some(t) = other.strip_prefix("template=") {
                w.template_row.set_text(t);
            } else if let Some(s) = other.strip_prefix("shelf=") {
                w.shelf_row.set_text(s);
            } else if let Some(u) = other.strip_prefix("dav=") {
                w.dav_url.set_text(u);
                w.save_dav();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_height_fits_a_600px_parent() {
        assert_eq!(dialog_content_height(600, 560), 510);
        assert_eq!(dialog_content_height(800, 560), 560);
        assert_eq!(dialog_content_height(0, 560), 560);
        assert_eq!(dialog_content_height(300, 560), 255);
    }

    #[test]
    fn buttons_follow_the_device_window_wording() {
        assert_eq!(ds::send_button_label(0), "Send 0 Books");
        assert_eq!(ds::send_button_label(1), "Send 1 Book");
        assert_eq!(ds::send_button_label(3), "Send 3 Books");
        assert_eq!(ds::remove_button_label(0), "Remove from Device");
        assert_eq!(
            ds::remove_confirmation_title(3),
            "Remove 3 Books from the Device?"
        );
    }

    #[test]
    fn selection_description_counts_the_books() {
        assert_eq!(selection_description(4, 0), "0 of 4 selected");
        assert_eq!(selection_description(4, 3), "3 of 4 selected");
    }

    #[test]
    fn status_page_words_come_from_the_shared_helper() {
        assert_eq!(ds::no_device_status_title(), "No Device Connected");
        assert_eq!(
            ds::no_device_status_description(),
            "Connect your e-reader with USB, or add a WebDAV folder below."
        );
        assert!(no_device_hint().starts_with("No device is connected."));
    }
}
