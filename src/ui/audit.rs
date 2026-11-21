//! Accessibility audit of the widget tree (`app:a11y-audit` in a dev script, see `_developer/docs.md`).
//!
//! It lists every interactive control of every window and says which have no accessible name, or are
//! icon-only buttons without a tooltip. `label()` is how code names a control that has no text of its
//! own (a scale, a switch, an entry): it sets the accessible label and remembers it for the audit,
//! because GTK has no getter for the label.

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use crate::a11y::{control_problem, Control};

const KEY: &str = "bookx-a11y-label";

/// Sets the accessible label of `w` (and remembers it for the audit).
pub fn label(w: &impl IsA<gtk::Widget>, name: &str) {
    let w = w.as_ref();
    w.update_property(&[gtk::accessible::Property::Label(name)]);
    // SAFETY: the value is a `String`, only ever read back as a `String` under the same key.
    unsafe { w.set_data(KEY, name.to_string()) };
}

/// Gives every item of the menu a button opens its text as accessible name. GTK 4 leaves the entries of
/// a `gio::Menu` popover unnamed for assistive technology, so a screen reader would only say "menu item".
pub fn name_menu_items(btn: &gtk::MenuButton) {
    if let Some(p) = btn.popover() {
        name_items_below(p.upcast_ref());
        p.connect_map(|p| name_items_below(p.upcast_ref()));
    }
    btn.connect_active_notify(|b| {
        if !b.is_active() {
            return;
        }
        if let Some(p) = b.popover() {
            gtk::glib::idle_add_local_once(move || name_items_below(p.upcast_ref()));
        }
    });
}

fn name_items_below(w: &gtk::Widget) {
    use gtk::AccessibleRole as R;
    if matches!(
        w.accessible_role(),
        R::MenuItem | R::MenuItemCheckbox | R::MenuItemRadio
    ) {
        let t = text_below(w);
        if !t.trim().is_empty() {
            // GTK points the item's labelled-by at its inner label, whose role is "presentation", so the
            // computed name is empty and wins over the label property: drop the relation first
            w.reset_relation(gtk::AccessibleRelation::LabelledBy);
            w.update_property(&[gtk::accessible::Property::Label(&t)]);
        }
        return;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        name_items_below(&child);
        c = child.next_sibling();
    }
}

fn stored(w: &gtk::Widget) -> Option<String> {
    // SAFETY: see `label`; only `String`s are stored under this key.
    unsafe { w.data::<String>(KEY).map(|p| p.as_ref().clone()) }
}

fn text_below(w: &gtk::Widget) -> String {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        return l.text().to_string();
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        let t = text_below(&child);
        if !t.trim().is_empty() {
            return t;
        }
        c = child.next_sibling();
    }
    String::new()
}

fn has_icon_below(w: &gtk::Widget) -> bool {
    if w.is::<gtk::Image>() {
        return true;
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if has_icon_below(&child) {
            return true;
        }
        c = child.next_sibling();
    }
    false
}

/// What a screen reader would say for a control, and whether it is just an icon.
fn describe(w: &gtk::Widget) -> Option<(&'static str, String, bool)> {
    let tip = w
        .tooltip_text()
        .map(|t| t.to_string())
        .filter(|t| !t.is_empty());
    let own = stored(w);
    let (kind, text, icon_only): (&'static str, String, bool) =
        if let Some(b) = w.downcast_ref::<gtk::MenuButton>() {
            let l = b.label().map(|l| l.to_string()).unwrap_or_default();
            let child = b.child().map(|c| text_below(&c)).unwrap_or_default();
            let t = if !l.is_empty() { l } else { child };
            ("menu-button", t.clone(), t.is_empty())
        } else if let Some(b) = w.downcast_ref::<gtk::Button>() {
            let l = b.label().map(|l| l.to_string()).unwrap_or_default();
            let t = if !l.is_empty() {
                l
            } else {
                b.child().map(|c| text_below(&c)).unwrap_or_default()
            };
            let icon = t.is_empty()
                && (b.icon_name().is_some() || b.child().is_some_and(|c| has_icon_below(&c)));
            (
                if w.is::<gtk::ToggleButton>() {
                    "toggle-button"
                } else {
                    "button"
                },
                t,
                icon,
            )
        } else if let Some(b) = w.downcast_ref::<gtk::CheckButton>() {
            (
                "check-button",
                b.label().map(|l| l.to_string()).unwrap_or_default(),
                false,
            )
        } else if w.is::<gtk::Switch>() {
            ("switch", String::new(), false)
        } else if w.is::<gtk::Scale>() {
            ("scale", String::new(), false)
        } else if w.is::<gtk::SpinButton>() {
            ("spin-button", String::new(), false)
        } else if w.is::<gtk::DropDown>() {
            ("dropdown", String::new(), false)
        } else if let Some(e) = w.downcast_ref::<gtk::SearchEntry>() {
            (
                "search-entry",
                e.placeholder_text()
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                false,
            )
        } else if let Some(e) = w.downcast_ref::<gtk::Entry>() {
            (
                "entry",
                e.placeholder_text()
                    .map(|t| t.to_string())
                    .unwrap_or_default(),
                false,
            )
        } else if w.is::<gtk::TextView>() {
            ("text-view", String::new(), false)
        } else if w.is::<gtk::ColorDialogButton>() {
            ("color-button", String::new(), false)
        } else {
            return None;
        };
    let name = own.or(if text.trim().is_empty() {
        None
    } else {
        Some(text)
    });
    let name = name.or_else(|| tip.clone()).unwrap_or_default();
    Some((kind, name, icon_only))
}

/// Widgets whose insides are named by the widget itself (rows know their title, window controls
/// their own labels): the audit does not look into them.
fn is_opaque(w: &gtk::Widget) -> bool {
    matches!(
        w.type_().name(),
        "AdwBackButton" | "AdwSheetControls" | "AdwToastWidget" | "AdwViewSwitcher" | "AdwTabBar"
    ) || w.is::<adw::SwitchRow>()
        || w.is::<adw::ComboRow>()
        || w.is::<adw::SpinRow>()
        || w.is::<adw::EntryRow>()
        || w.is::<adw::PasswordEntryRow>()
        || w.is::<gtk::SearchEntry>()
        || w.is::<gtk::Entry>()
        || w.is::<gtk::SpinButton>()
        || w.is::<gtk::DropDown>()
        || w.is::<gtk::Scale>()
        || w.is::<gtk::Switch>()
        || w.is::<gtk::ColorDialogButton>()
        || w.is::<gtk::WindowControls>()
        || w.is::<adw::WindowTitle>()
        || w.is::<gtk::Scrollbar>()
        || w.is::<gtk::PopoverMenu>()
        || w.is::<gtk::TextView>()
}

fn path_of(w: &gtk::Widget) -> String {
    let mut parts = Vec::new();
    let mut cur = Some(w.clone());
    while let Some(c) = cur {
        let mut s = c.type_().name().to_string();
        let n = c.widget_name();
        if n.as_str() != s.as_str() && !n.is_empty() {
            s = format!("{s}#{n}");
        }
        if let Some(css) = c.css_classes().first() {
            s = format!("{s}.{css}");
        }
        parts.push(s);
        if parts.len() >= 5 {
            break;
        }
        cur = c.parent();
    }
    parts.reverse();
    let mut near = String::new();
    let mut up = w.parent();
    for _ in 0..4 {
        let Some(p) = up else { break };
        near = text_below(&p);
        if !near.trim().is_empty() {
            break;
        }
        up = p.parent();
    }
    let near: String = near.chars().take(30).collect();
    format!("{} near {:?}", parts.join(" > "), near)
}

fn walk(w: &gtk::Widget, out: &mut Vec<Control>, in_popover: bool) {
    let in_popover = in_popover || w.is::<gtk::Popover>();
    if !w.is_visible() && !in_popover {
        return;
    }
    if let Some((kind, name, icon_only)) = describe(w) {
        out.push(Control {
            kind,
            path: path_of(w),
            name,
            icon_only,
            tooltip: w.tooltip_text().is_some_and(|t| !t.is_empty()),
        });
    }
    // the popover of a menu button is a child too, but is built from a menu model: its items are named by the menu
    if is_opaque(w) {
        return;
    }
    if w.is::<gtk::MenuButton>() {
        if let Some(pop) = w
            .downcast_ref::<gtk::MenuButton>()
            .and_then(|m| m.popover())
        {
            if !pop.is::<gtk::PopoverMenu>() {
                walk(pop.upcast_ref(), out, in_popover);
            }
        }
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        // the inner toggle of a MenuButton is part of it; the close button of a search bar names itself
        let internal = (w.is::<gtk::MenuButton>() && child.is::<gtk::ToggleButton>())
            || (w.is::<gtk::CenterBox>()
                && child.is::<gtk::Button>()
                && w.parent().is_some_and(|p| p.is::<gtk::Revealer>())
                && w.parent()
                    .and_then(|p| p.parent())
                    .is_some_and(|p| p.is::<gtk::SearchBar>()));
        if !internal {
            walk(&child, out, in_popover);
        }
        c = child.next_sibling();
    }
}

/// Every interactive control of every window, with its name.
pub fn controls() -> Vec<Control> {
    let mut out = Vec::new();
    for t in gtk::Window::list_toplevels() {
        walk(&t, &mut out, false);
    }
    out
}

/// What fits in what: a dialog (and its default button) inside its window (X6, XC-7).
pub fn fits_in(dialog: (i32, i32), window: (i32, i32)) -> bool {
    dialog.0 <= window.0 && dialog.1 <= window.1
}

/// Whether a box `(x, y, w, h)` lies inside an area of `size`.
pub fn box_within(b: (f32, f32, f32, f32), size: (i32, i32)) -> bool {
    b.0 >= -0.5
        && b.1 >= -0.5
        && b.0 + b.2 <= size.0 as f32 + 0.5
        && b.1 + b.3 <= size.1 as f32 + 0.5
}

/// The labels of the mapped buttons that cut their text (Pango had to ellipsize it).
fn clipped_buttons(w: &gtk::Widget, out: &mut Vec<String>) {
    if let Some(b) = w.downcast_ref::<gtk::Button>() {
        if b.is_mapped() {
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                if l.layout().is_ellipsized() {
                    out.push(l.text().to_string());
                }
            }
        }
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        clipped_buttons(&x, out);
        c = x.next_sibling();
    }
}

/// The `dialog_fit:` line for the dialog that is open over `window`, if any: its size against the
/// window's, whether its default button is on screen, and how many buttons cut their label.
pub fn dialog_fit_line(window: &adw::ApplicationWindow) -> Option<String> {
    let d = window.visible_dialog()?;
    let content = d.child()?;
    let size = (content.width(), content.height());
    let win = (window.width(), window.height());
    let default_box = d
        .default_widget()
        .and_then(|w| w.compute_bounds(&content))
        .map(|r| (r.x(), r.y(), r.width(), r.height()));
    let default = d
        .default_widget()
        .map(|w| w.is_mapped() && default_box.is_some_and(|b| box_within(b, size)));
    let mut clipped = Vec::new();
    clipped_buttons(&content, &mut clipped);
    Some(format!(
        "dialog_fit: title={:?} dialog={}x{} window={}x{} fits={} default_visible={} default_box={:?} clipped_buttons={} clipped={:?}",
        d.title(),
        size.0,
        size.1,
        win.0,
        win.1,
        fits_in(size, win),
        default.map_or("none".to_string(), |v| v.to_string()),
        default_box,
        clipped.len(),
        clipped
    ))
}

/// Prints the report the UI scenarios read.
pub fn report() {
    for t in gtk::Window::list_toplevels() {
        if let Some(w) = t.downcast_ref::<adw::ApplicationWindow>() {
            if let Some(line) = dialog_fit_line(w) {
                eprintln!("{line}");
            }
        }
    }
    let all = controls();
    let bad: Vec<&Control> = all
        .iter()
        .filter(|c| control_problem(c).is_some())
        .collect();
    eprintln!("a11y: controls={} problems={}", all.len(), bad.len());
    eprintln!(
        "a11y-env: animations={} high_contrast={} dark={}",
        gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations()),
        adw::StyleManager::default().is_high_contrast(),
        adw::StyleManager::default().is_dark(),
    );
    for c in bad {
        eprintln!(
            "a11y-problem: {} | {} | {}",
            c.kind,
            control_problem(c).unwrap_or(""),
            c.path
        );
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;

    #[test]
    fn a_dialog_fits_when_it_is_not_bigger_than_its_window() {
        assert!(fits_in((640, 560), (900, 600)));
        assert!(!fits_in((880, 640), (900, 600)));
    }

    #[test]
    fn a_button_is_on_screen_only_when_all_of_it_is_inside_the_dialog() {
        assert!(box_within((500.0, 500.0, 100.0, 40.0), (640, 560)));
        assert!(!box_within((500.0, 540.0, 100.0, 40.0), (640, 560)));
        assert!(!box_within((-30.0, 10.0, 100.0, 40.0), (640, 560)));
    }
}
