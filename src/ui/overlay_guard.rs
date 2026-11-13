//! Keeps the reader's non-modal popover (the selection actions) from drawing over other windows.
//!
//! The selection popover has `autohide` off, so a click on the page does not close it (the click
//! should extend or clear the selection instead). A popover that does not autohide is also not
//! closed by anything else: it is a surface of its own on a real compositor and stays above
//! libadwaita dialogs, other windows and other menus. This guard closes it, in one place, whenever
//! something else comes to the front over the reader:
//!
//! * an `AdwDialog` is shown on the window (`visible-dialog` of the `AdwApplicationWindow`:
//!   Preferences, Properties, Go to, the book map, the shortcuts window, the voice chooser …),
//! * a window is added to the application (`GtkWindow::toplevels()` grows: image viewer, file
//!   chooser, a second window …) or the reader's window stops being the active one,
//! * another popover of the reader is shown (menus, the footer settings, search): every popover
//!   that hangs on a `GtkMenuButton` of the reader page is watched, and others can be added with
//!   [`OverlayGuard::watch_popover`].
//!
//! After that the selection stays, the popover stays closed (it is not brought back behind the
//! user's back when the dialog closes); Enter on the page or a new selection opens it again. The
//! guard also makes any tooltip give way when the window loses the focus, so none stays stuck.

use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

type Cover = Rc<dyn Fn()>;

pub struct OverlayGuard {
    cover: Cover,
    /// The popover has the pointer grab (its note box is open): the window then looks inactive
    /// for a moment and that must not close it.
    grabbing: Rc<dyn Fn() -> bool>,
    /// Everything that was connected, to be undone when the reader goes.
    ids: RefCell<Vec<(glib::Object, glib::SignalHandlerId)>>,
}

impl OverlayGuard {
    /// `cover` closes the selection popover (it must be safe to call at any time).
    pub fn install(
        page: &adw::NavigationPage,
        cover: impl Fn() + 'static,
        grabbing: impl Fn() -> bool + 'static,
    ) -> Rc<OverlayGuard> {
        let g = Rc::new(OverlayGuard {
            cover: Rc::new(cover),
            grabbing: Rc::new(grabbing),
            ids: RefCell::new(Vec::new()),
        });
        // popovers of the page: every menu button and what they hold
        g.watch_tree(page.upcast_ref());
        // the window is only known once the page is on screen
        let done = Rc::new(std::cell::Cell::new(false));
        let weak = Rc::downgrade(&g);
        page.connect_map(move |p| {
            if done.replace(true) {
                return;
            }
            if let (Some(g), Some(win)) = (weak.upgrade(), p.root().and_downcast::<gtk::Window>()) {
                g.watch_window(&win);
            }
        });
        g
    }

    fn remember(&self, obj: &impl IsA<glib::Object>, id: glib::SignalHandlerId) {
        self.ids.borrow_mut().push((obj.clone().upcast(), id));
    }

    fn watch_window(self: &Rc<Self>, win: &gtk::Window) {
        if let Some(w) = win.downcast_ref::<adw::ApplicationWindow>() {
            let c = self.cover.clone();
            let id = w.connect_visible_dialog_notify(move |w| {
                if w.visible_dialog().is_some() {
                    c();
                }
            });
            self.remember(w, id);
        }
        // not the active window any more: another window, or the desktop
        let (c, grabbing) = (self.cover.clone(), self.grabbing.clone());
        let id = win.connect_is_active_notify(move |w| {
            if !w.is_active() && !grabbing() {
                c();
            }
        });
        self.remember(win, id);
        // a window was added
        let list = gtk::Window::toplevels();
        let c = self.cover.clone();
        let id = list.connect_items_changed(move |_, _, _, added| {
            if added > 0 {
                c();
            }
        });
        self.remember(&list, id);
    }

    /// Closes the selection popover when this popover is shown.
    pub fn watch_popover(self: &Rc<Self>, pop: &gtk::Popover) {
        let c = self.cover.clone();
        let id = pop.connect_map(move |_| c());
        self.remember(pop, id);
    }

    fn watch_tree(self: &Rc<Self>, w: &gtk::Widget) {
        if let Some(b) = w.downcast_ref::<gtk::MenuButton>() {
            if let Some(p) = b.popover() {
                self.watch_popover(&p);
            }
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            self.watch_tree(&ch);
            c = ch.next_sibling();
        }
    }

    /// Undoes every connection (the reader is closed).
    pub fn disconnect(&self) {
        for (o, id) in self.ids.borrow_mut().drain(..) {
            o.disconnect(id);
        }
    }
}

/// Makes a tooltip that is showing give way (it can stay behind when the window loses the focus
/// or something covers the widget).
pub fn drop_tooltip(w: &impl IsA<gtk::Widget>) {
    let w = w.clone();
    if w.has_tooltip() {
        w.set_has_tooltip(false);
        glib::idle_add_local_once(move || w.set_has_tooltip(true));
    }
}
