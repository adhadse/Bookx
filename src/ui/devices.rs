//! The Device window: the books on a connected e-reader (or the cloud folder) with their covers
//! (grid or list), sending the books selected in the library, removing books, free space, how books
//! are sent (format, save-path template, Kobo shelf, notes), syncing reading progress and notes with
//! the library, and setting up a WebDAV folder. The logic is in `crate::devices`,
//! `crate::device_sync` and `crate::device_covers`; this file is widgets and a thread per slow
//! operation. [`start_sync`] is also what the library's "device connected" banner uses.

use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use bookx_core::contracts::DeviceInfo;
use bookx_serve::devfs::{self, human_size, DeviceBook};
use bookx_serve::devices::DeviceKind;
use bookx_serve::reading::DeviceState;
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::devices_list::{Item, PickList};
use super::SharedRef;
use crate::devices as dev;

const NO_DEVICE_HINT: &str = "No device is connected. Plug in an e-reader with a USB cable (it must appear in Files), or mount a phone or tablet in Files (MTP). A cloud folder can be set up below.";

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

struct DevWin {
    dialog: adw::Dialog,
    view_grid: gtk::ToggleButton,
    on_dev: Rc<PickList>,
    to_send: Rc<PickList>,
    send_ids: RefCell<Vec<u64>>,
    plans: RefCell<Vec<(u64, dev::SendPlan)>>,
    send_group: adw::PreferencesGroup,
    send_btn: gtk::Button,
    send_status: gtk::Label,
    sending: Cell<bool>,
    notes_row: adw::SwitchRow,
    auto_row: adw::SwitchRow,
    shared: SharedRef,
    on_changed: Rc<dyn Fn()>,
    device_row: adw::ComboRow,
    device_group: adw::PreferencesGroup,
    summary: gtk::Label,
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

/// Opens the window (a second call brings the open one to the front).
pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef, on_changed: Rc<dyn Fn()>) {
    if let Some(w) = CURRENT.with(|c| c.borrow().clone()) {
        if w.dialog.is_visible() {
            w.set_send_ids(library_selection());
            w.reload_devices();
            return;
        }
    }
    let w = DevWin::build(shared, on_changed);
    w.dialog.present(Some(parent));
    {
        let weak = Rc::downgrade(&w);
        w.dialog.connect_closed(move |_| {
            if let Some(w) = weak.upgrade() {
                w.generation.set(w.generation.get() + 1);
            }
            CURRENT.with(|c| *c.borrow_mut() = None);
        });
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(w.clone()));
    w.set_send_ids(library_selection());
    w.reload_devices();
}

impl DevWin {
    fn build(shared: &SharedRef, on_changed: Rc<dyn Fn()>) -> Rc<DevWin> {
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Device"));
        dialog.set_content_width(640);
        dialog.set_content_height(720);
        let page = adw::PreferencesPage::new();
        let settings = shared.cfg.borrow().device.clone();

        let device_group = adw::PreferencesGroup::new();
        device_group.set_title(&tr("Device"));
        device_group.set_description(Some(NO_DEVICE_HINT));
        let device_row = adw::ComboRow::builder().title(tr("Device")).build();
        device_row.set_model(Some(&gtk::StringList::new(&[])));
        let summary = gtk::Label::new(None);
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.add_css_class("dim-label");
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some(&tr("Look for devices again")));
        refresh.add_css_class("flat");
        refresh.set_valign(gtk::Align::Center);
        device_row.add_suffix(&refresh);
        device_group.add(&device_row);
        device_group.add(&summary);
        page.add(&device_group);

        let send_group = adw::PreferencesGroup::new();
        send_group.set_title(&tr("Books to send"));
        send_group.set_description(Some(
            "The books selected in the library. Untick a book to leave it out; a book with a warning is left out until you tick it.",
        ));
        let send_status = gtk::Label::new(None);
        send_status.set_xalign(0.0);
        send_status.set_wrap(true);
        send_status.add_css_class("dim-label");
        let send_btn = gtk::Button::with_label(&tr("Send 0 books"));
        send_btn.add_css_class("suggested-action");
        send_btn.add_css_class("pill");
        send_btn.set_halign(gtk::Align::End);
        send_btn.set_sensitive(false);
        let books_group = adw::PreferencesGroup::new();
        books_group.set_title(&tr("Books on the device"));
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let sync_btn = gtk::Button::with_label(&tr("Sync Reading State"));
        sync_btn.set_tooltip_text(Some(
            "Bring the reading progress from the device into the library (Kobo database, KOReader)",
        ));
        let remove_btn = gtk::Button::with_label(&tr("Remove Selected"));
        remove_btn.add_css_class("destructive-action");
        buttons.append(&sync_btn);
        buttons.append(&remove_btn);
        books_group.set_header_suffix(Some(&buttons));
        let slot: Rc<RefCell<Option<std::rc::Weak<DevWin>>>> = Rc::default();
        let changed = {
            let slot = slot.clone();
            Rc::new(move || {
                if let Some(w) = slot.borrow().as_ref().and_then(|w| w.upgrade()) {
                    w.update_buttons();
                }
            }) as Rc<dyn Fn()>
        };
        let on_dev = PickList::new(changed.clone());
        let to_send = PickList::new(changed);
        let grid_view = !settings_view_is_list(shared);
        on_dev.set_grid(grid_view);
        to_send.set_grid(grid_view);
        let dev_controls = on_dev.controls();
        buttons.prepend(&dev_controls);
        books_group.add(&on_dev.widget);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_selectable(true);
        books_group.add(&status);
        send_group.set_header_suffix(Some(&to_send.controls()));
        send_group.add(&to_send.widget);
        send_group.add(&send_status);
        send_group.add(&send_btn);
        page.add(&send_group);
        page.add(&books_group);

        let options_group = adw::PreferencesGroup::new();
        options_group.set_title(&tr("Sending options"));
        options_group.set_description(Some(
            "Used by Send to Device. If a conversion fails, the next format the device opens is tried.",
        ));
        let format_row = adw::ComboRow::builder().title(tr("Format")).build();
        let labels: Vec<&str> = std::iter::once("Automatic (best for the device)")
            .chain(crate::export::TARGETS.iter().map(|t| t.label))
            .collect();
        format_row.set_model(Some(&gtk::StringList::new(&labels)));
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
            .title(crate::device_sync::NOTES_TITLE)
            .subtitle(crate::device_sync::NOTES_SUBTITLE)
            .build();
        notes_row.set_active(shared.cfg.borrow().device_sync.include_notes);
        let auto_row = adw::SwitchRow::builder()
            .title(crate::device_sync::AUTO_SYNC_TITLE)
            .subtitle(crate::device_sync::AUTO_SYNC_SUBTITLE)
            .build();
        auto_row.set_active(shared.cfg.borrow().device_sync.auto_sync);
        auto_row.add_suffix(&super::help_button(crate::device_sync::AUTO_SYNC_HELP));
        options_group.add(&format_row);
        options_group.add(&template_row);
        options_group.add(&shelf_row);
        options_group.add(&notes_row);
        options_group.add(&auto_row);
        page.add(&options_group);

        let dav_group = adw::PreferencesGroup::new();
        dav_group.set_title(&tr("Cloud folder (WebDAV)"));
        dav_group.set_description(Some(
            "Nextcloud, ownCloud and other WebDAV folders work like a device: send books there, list and remove them.",
        ));
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
        dav_status.set_text(crate::secrets::caption_now());
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
        page.add(&dav_group);

        let view = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        let view_grid = gtk::ToggleButton::new();
        view_grid.set_icon_name("view-grid-symbolic");
        view_grid.set_tooltip_text(Some(&tr("Covers")));
        let view_list = gtk::ToggleButton::new();
        view_list.set_icon_name("view-list-symbolic");
        view_list.set_tooltip_text(Some(&tr("List")));
        view_list.set_group(Some(&view_grid));
        view_grid.set_active(!settings_view_is_list(shared));
        view_list.set_active(settings_view_is_list(shared));
        let view_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        view_box.add_css_class("linked");
        view_box.append(&view_grid);
        view_box.append(&view_list);
        header.pack_start(&view_box);
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));

        let w = Rc::new(DevWin {
            dialog,
            view_grid,
            on_dev,
            to_send,
            send_ids: RefCell::default(),
            plans: RefCell::default(),
            send_group,
            send_btn,
            send_status,
            sending: Cell::new(false),
            notes_row,
            auto_row,
            shared: shared.clone(),
            on_changed,
            device_row,
            device_group,
            summary,
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
            w.view_grid.connect_toggled(move |b| {
                let grid = b.is_active();
                w2.on_dev.set_grid(grid);
                w2.to_send.set_grid(grid);
                let mut c = w2.shared.cfg.borrow_mut();
                c.device_list_view = !grid;
                c.save();
            });
            let w2 = w.clone();
            w.send_btn.connect_clicked(move |_| w2.send_checked());
            let w2 = w.clone();
            w.notes_row.connect_active_notify(move |r| {
                {
                    let mut c = w2.shared.cfg.borrow_mut();
                    c.device_sync.include_notes = r.is_active();
                    c.save();
                }
                w2.refresh_send_list();
            });
            let w2 = w.clone();
            w.auto_row.connect_active_notify(move |r| {
                let mut c = w2.shared.cfg.borrow_mut();
                c.device_sync.auto_sync = r.is_active();
                c.save();
            });
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
                self.options_group.set_description(Some(
                    "Used by Send to Device. If a conversion fails, the next format the device opens is tried. Fields: {title} {author} {authors} {author_sort} {series} {series_index} {language}",
                ));
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
        self.dav_status.set_label(&tr("Connecting…"));
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let r = bookx_serve::webdav::WebDav::new(&s.url, &s.user, &s.password)
                .and_then(|d| d.check())
                .map(|_| "Connected.".to_string())
                .unwrap_or_else(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
        let label = self.dav_status.clone();
        poll(rx, move |msg| label.set_label(&msg));
    }

    // -- devices and books -----------------------------------------------------------------

    fn reload_devices(self: &Rc<Self>) {
        let settings = self.shared.cfg.borrow().device.clone();
        let drivers = dev::drivers(&settings);
        let devices = drivers.detect();
        let previous = self.selected_device().map(|d| d.id);
        let names: Vec<String> = devices
            .iter()
            .map(|d| {
                if drivers.backend(d) == dev::Backend::Usb {
                    format!("{} · {}", d.name, drivers.storage(d).label())
                } else {
                    d.name.clone()
                }
            })
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let keep = previous
            .and_then(|id| devices.iter().position(|d| d.id == id))
            .unwrap_or(0);
        *self.devices.borrow_mut() = devices;
        // changing the model selects row 0, which reloads the books; the notify handler is
        // harmless here because `reload_books` below runs again for the final selection
        self.device_row
            .set_model(Some(&gtk::StringList::new(&refs)));
        self.device_row.set_selected(keep as u32);
        self.device_group.set_description(if refs.is_empty() {
            Some(NO_DEVICE_HINT)
        } else {
            None
        });
        self.device_row.set_sensitive(!refs.is_empty());
        self.reload_books();
    }

    fn selected_device(&self) -> Option<DeviceInfo> {
        self.devices
            .borrow()
            .get(self.device_row.selected() as usize)
            .cloned()
    }

    /// Lists the books of the selected device on a worker thread.
    fn reload_books(self: &Rc<Self>) {
        let gen = self.generation.get() + 1;
        self.generation.set(gen);
        self.states.borrow_mut().clear();
        let Some(device) = self.selected_device() else {
            self.fill_books(&[]);
            self.summary.set_label("");
            self.books_group.set_description(Some(NO_DEVICE_HINT));
            self.shelf_row.set_visible(false);
            self.remove_btn.set_sensitive(false);
            self.sync_btn.set_sensitive(false);
            self.loading.set(false);
            self.books.borrow_mut().clear();
            self.refresh_send_list();
            return;
        };
        self.loading.set(true);
        self.books_group.set_description(None);
        self.status.set_label(&tr("Reading the device…"));
        let settings = self.shared.cfg.borrow().device.clone();
        let drivers = dev::drivers(&settings);
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
            if me.generation.get() != gen {
                return;
            }
            *me.shelves.borrow_mut() = shelves;
            me.loading.set(false);
            *me.last_summary.borrow_mut() = summary.clone();
            me.summary.set_label(&summary);
            match books {
                Ok(b) => {
                    me.status.set_label(&match b.len() {
                        0 => "There are no books on this device.".to_string(),
                        1 => "1 book".to_string(),
                        n => format!("{n} books"),
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
        let gen = self.generation.get();
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
            if me.generation.get() != gen {
                return glib::ControlFlow::Break;
            }
            for _ in 0..12 {
                match rx.try_recv() {
                    Ok((i, Some(file))) => me.on_dev.set_cover(i, &file),
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return glib::ControlFlow::Break
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn subtitle(&self, b: &DeviceBook) -> String {
        let mut s = format!("{} · {}", b.rel, human_size(b.size));
        if let Some(shelves) = self.shelves.borrow().get(&b.path) {
            s.push_str(&format!(" · Shelf: {}", shelves.join(", ")));
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
            &trf(
                "Remove {} from the device?",
                &[&ntr("{} book", "{} books", n)],
            ),
            "The files are deleted from the device. Copies in your library stay.",
            "Remove",
            true,
            move || me.do_remove(device.clone(), picked.clone()),
        );
    }

    fn do_remove(self: &Rc<Self>, device: DeviceInfo, picked: Vec<DeviceBook>) {
        self.status.set_label(&tr("Removing…"));
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
        self.status.set_label(&tr("Reading the reading state…"));
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
                    "Connect a device to see the format".to_string()
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
        self.send_group.set_description(Some(&(if items.is_empty() { tr("Nothing to send yet. Select books in the library, then use Send to Device… in the right-click menu (or the Device button).") } else { tr("The books selected in the library. Untick a book to leave it out; a book with a warning is left out until you tick it.") })));
        *self.plans.borrow_mut() = plans;
        self.to_send.set_items(&items);
        self.update_buttons();
    }

    fn update_buttons(&self) {
        let n = self.to_send.selected();
        self.send_btn.set_label(&match n {
            1 => "Send 1 book".to_string(),
            n => trf("Send {} books", &[&n]),
        });
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
        self.remove_btn.set_label(&if m == 0 {
            "Remove Selected".to_string()
        } else {
            trf("Remove {} Selected", &[&m])
        });
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
        self.sending.set(true);
        self.update_buttons();
        self.send_status.set_label(&tr("Sending…"));
        let me = self.clone();
        send_books(
            &self.shared,
            device,
            ids,
            self.notes_row.is_active(),
            move |msg| {
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
        eprintln!(
            "devwin_hint: {}",
            self.device_group.description().unwrap_or_default()
        );
        let books = self.books.borrow().clone();
        for b in &books {
            eprintln!("devwin_book: {}", self.subtitle(b));
        }
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
    let (tx, rx) = std::sync::mpsc::channel::<String>();
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
                        notes_out.push(format!(
                            "{} was sent as {} because {}",
                            fields.title,
                            sent.format,
                            sent.fell_back_from.join("; ")
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
        let _ = tx.send(msg);
    });
    poll(rx, done);
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
    use crate::device_sync as ds;
    let settings = shared.cfg.borrow().device.clone();
    let (books, locator) = {
        let lib = shared.lib.borrow();
        (ds::library_books(&lib.books), ds::book_locator(&lib))
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
            eprintln!(
                "devwin_sync_item: {title} {:.0}% -> {:.0}% ({source})",
                from * 100.0,
                to * 100.0
            );
        }
        eprintln!(
            "devwin_sync: read={} updated={}",
            readings.iter().filter(|r| r.state.is_some()).count(),
            summary.progress.len()
        );
        for l in summary.lines() {
            eprintln!("sync_summary: {l}");
        }
        if plan.is_empty() && summary.conflicts.is_empty() {
            let mut m = format!("{name}: already up to date");
            if summary.unmatched > 0 {
                m.push_str(&format!(
                    " ({} on the device not in your library)",
                    summary.unmatched
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
                    let a = ds::apply_to_library(&mut lib, &plan);
                    let _ = lib.save();
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
                        m.push_str(&format!(" (problems: {})", errors.join("; ")));
                    }
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
                    "Sync",
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

/// Test hooks (`devwin:<command>` in `BOOKX_DEV_SCRIPT`): `show`, `dump`, `device=<n>`,
/// `select=<i,j>`, `remove`, `sync`, `format=<id>`, `template=<text>`, `shelf=<name>`, `reload`,
/// `dav=<url>`, `view=grid|list`, `notes=on|off`, `autosync=on|off`, `syncask` (summary dialog),
/// `syncconfirm`, `send` (the library's selection), `close`.
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
        "reload" => w.reload_devices(),
        "remove" => w.remove_selected(false),
        "sync" => w.sync_state(false),
        "syncask" => w.sync_state(true),
        "syncconfirm" => {
            if let Some(f) = PENDING_SYNC.with(|p| p.borrow_mut().take()) {
                f();
            } else {
                eprintln!("sync_summary: no dialog");
            }
        }
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
                // `click=<send|device>:<index>[:shift]` is a click on a row or tile
                let mut it = arg.split(':');
                let list = if it.next() == Some("send") {
                    &w.to_send
                } else {
                    &w.on_dev
                };
                let i = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
                list.click(i, it.next() == Some("shift"));
            } else if let Some(v) = other.strip_prefix("view=") {
                w.view_grid.set_active(v == "grid");
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
