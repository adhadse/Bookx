//! Shared helpers for secondary dialogs: a toast overlay with an Undo button (XC-1, X5) and the
//! standard dialog constructor with a Ctrl+W close shortcut (X9).
//!
//! Rule for the whole app: a reversible action shows an undo toast and no confirmation;
//! an irreversible one asks first and names the count.

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio};

use crate::i18n::tr;

/// How long an undo toast stays, in seconds (long enough to notice and click).
pub const UNDO_TIMEOUT_SECS: u32 = 10;

/// Plain toast timeout, in seconds.
pub const PLAIN_TIMEOUT_SECS: u32 = 4;

/// What a toast looks like, decided without GTK so it can be unit tested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToastSpec {
    pub text: String,
    pub undo: bool,
    pub high_priority: bool,
    pub timeout: u32,
}

/// `undo` = the toast carries an Undo button; `error` = shown at once and kept as long as an
/// undo toast.
pub fn toast_spec(text: &str, undo: bool, error: bool) -> ToastSpec {
    ToastSpec {
        text: text.to_string(),
        undo,
        high_priority: undo || error,
        timeout: if undo || error {
            UNDO_TIMEOUT_SECS
        } else {
            PLAIN_TIMEOUT_SECS
        },
    }
}

fn build_toast(spec: &ToastSpec) -> adw::Toast {
    let t = adw::Toast::new(&spec.text);
    t.set_use_markup(false);
    t.set_timeout(spec.timeout);
    if spec.high_priority {
        t.set_priority(adw::ToastPriority::High);
    }
    if spec.undo {
        t.set_button_label(Some(&tr("Undo")));
    }
    t
}

/// Wraps a dialog's content in a `ToastOverlay` and returns it (set it as the dialog child).
pub fn overlay(content: &impl IsA<gtk::Widget>) -> adw::ToastOverlay {
    let o = adw::ToastOverlay::new();
    o.set_child(Some(content));
    o
}

/// A plain, short toast (feedback of an action that needs no undo).
pub fn show_plain(overlay: &adw::ToastOverlay, text: &str) {
    overlay.add_toast(build_toast(&toast_spec(text, false, false)));
}

/// An error toast (high priority, stays longer).
pub fn show_error(overlay: &adw::ToastOverlay, text: &str) {
    overlay.add_toast(build_toast(&toast_spec(text, false, true)));
}

/// A toast with an Undo button. `undo` runs once when the button is pressed.
pub fn show_undo(overlay: &adw::ToastOverlay, text: &str, undo: impl Fn() + 'static) {
    let toast = build_toast(&toast_spec(text, true, false));
    toast.connect_button_clicked(move |_| undo());
    overlay.add_toast(toast);
}

/// A toast with a button of its own (for example "Open Folder"); `action` runs once when it is
/// pressed.
pub fn show_action(
    overlay: &adw::ToastOverlay,
    text: &str,
    button: &str,
    action: impl Fn() + 'static,
) {
    let toast = build_toast(&toast_spec(text, false, false));
    toast.set_button_label(Some(button));
    toast.set_timeout(UNDO_TIMEOUT_SECS);
    toast.connect_button_clicked(move |_| action());
    overlay.add_toast(toast);
}

/// The overlay of the application window (for a toast with a button from code that only has the
/// plain toast callback).
pub fn app_overlay() -> Option<adw::ToastOverlay> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter(|w| w.is::<adw::ApplicationWindow>())
        .find_map(|w| find_overlay(&w))
}

/// The `ToastOverlay` a widget sits in (a dialog's own, or the app window's when it has none).
pub fn overlay_of(widget: &impl IsA<gtk::Widget>) -> Option<adw::ToastOverlay> {
    let w = widget.upcast_ref::<gtk::Widget>();
    w.ancestor(adw::ToastOverlay::static_type())
        .and_downcast::<adw::ToastOverlay>()
        .or_else(|| w.root().and_then(|r| find_overlay(&r)))
}

/// The first `ToastOverlay` below `root`: how a dialog that only has the app's plain toast
/// callback reaches the main window's overlay to add a toast with a button.
pub fn find_overlay(root: &impl IsA<gtk::Widget>) -> Option<adw::ToastOverlay> {
    let w = root.upcast_ref::<gtk::Widget>();
    if let Some(o) = w.downcast_ref::<adw::ToastOverlay>() {
        return Some(o.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(o) = find_overlay(&c) {
            return Some(o);
        }
        child = c.next_sibling();
    }
    None
}

/// `adw::Dialog::new()` plus Ctrl+W to close (the libadwaita close button and Esc already work).
/// Registers the action `dlg.close` too, which the test hook `dialog:ctrlw` activates.
pub fn new_dialog() -> adw::Dialog {
    let dialog = adw::Dialog::new();
    install_close_shortcut(&dialog);
    dialog
}

/// The same for dialogs made with a builder.
pub fn install_close_shortcut(dialog: &adw::Dialog) {
    let group = gio::SimpleActionGroup::new();
    let close = gio::SimpleAction::new("close", None);
    {
        let dialog = dialog.downgrade();
        close.connect_activate(move |_, _| {
            if let Some(d) = dialog.upgrade() {
                d.close();
            }
        });
    }
    group.add_action(&close);
    dialog.insert_action_group("dlg", Some(&group));
    let controller = gtk::ShortcutController::new();
    controller.set_scope(gtk::ShortcutScope::Local);
    controller.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>w"),
        Some(gtk::NamedAction::new("dlg.close")),
    ));
    dialog.add_controller(controller);
}

/// Closes the visible dialog the way Ctrl+W does (test hook).
pub fn dev_ctrl_w(window: &adw::ApplicationWindow) {
    if let Some(d) = window.visible_dialog() {
        let _ = d.activate_action("dlg.close", None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_toasts_are_high_priority_and_stay_longer() {
        let t = toast_spec("Deleted “x”", true, false);
        assert!(t.undo && t.high_priority);
        assert_eq!(t.timeout, UNDO_TIMEOUT_SECS);
    }

    #[test]
    fn plain_toasts_are_short_and_normal_priority() {
        let t = toast_spec("Saved", false, false);
        assert!(!t.undo && !t.high_priority);
        assert_eq!(t.timeout, PLAIN_TIMEOUT_SECS);
    }

    #[test]
    fn errors_are_high_priority_without_a_button() {
        let t = toast_spec("Could not save", false, true);
        assert!(!t.undo && t.high_priority);
        assert!(t.timeout >= PLAIN_TIMEOUT_SECS);
    }
}
