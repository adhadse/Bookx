//! The Markup menu of the editor header: four short entries (Format, Insert, Structure, View) that open
//! submenus, so no page of it is taller than a 900x600 window, plus the accelerator labels of the Markup and Tools
//! menus. A label is shown where the editor registry (`editor_keymap`) has a binding for the item and follows
//! the bindings when they are changed in Preferences.

use super::*;
use crate::i18n::{tr, trf};
use crate::shortcuts;
use bookx_editing::code_format::Format;
use bookx_editing::code_style::{Align, ColourKind};

/// The root menu and the Structure submenu (the preview adds its split entries to that one).
pub(super) struct Built {
    pub root: gio::Menu,
    pub structure: gio::Menu,
}

fn target_item(action: &str, label: &str, id: &str) -> gio::MenuItem {
    let it = gio::MenuItem::new(Some(label), None);
    it.set_action_and_target_value(Some(action), Some(&id.to_variant()));
    it
}

pub(super) fn build() -> Built {
    let tag = |label: &str, a: TagAction| target_item("edit.tag", label, &a.id());
    let code = |label: &str, id: &str| target_item("edit.code", label, id);

    // ---- Format ----
    let format = gio::Menu::new();
    let styles = gio::Menu::new();
    for f in Format::ALL {
        styles.append_item(&code(&format_actions::format_label(f), f.id()));
    }
    format.append_section(None, &styles);
    let colour = gio::Menu::new();
    for k in [ColourKind::Text, ColourKind::Background] {
        colour.append_item(&code(
            &style_actions::colour_label(k),
            &format!("colour-{}", k.id()),
        ));
    }
    let off = gio::Menu::new();
    for k in [ColourKind::Text, ColourKind::Background] {
        off.append_item(&code(
            &style_actions::colour_off_label(k),
            &format!("colour-{}-off", k.id()),
        ));
    }
    colour.append_section(None, &off);
    let align = gio::Menu::new();
    for a in Align::ALL {
        align.append_item(&code(
            &style_actions::align_label(Some(a)),
            &format!("align-{}", a.css()),
        ));
    }
    let none = gio::Menu::new();
    none.append_item(&code(&style_actions::align_label(None), "align-off"));
    align.append_section(None, &none);
    let more = gio::Menu::new();
    more.append_submenu(Some(&tr("Colour")), &colour);
    more.append_submenu(Some(&tr("Alignment")), &align);
    let case = gio::Menu::new();
    case.append_item(&tag(&tr("UPPERCASE"), TagAction::Case(CaseChange::Upper)));
    case.append_item(&tag("lowercase", TagAction::Case(CaseChange::Lower)));
    case.append_item(&tag(&tr("Title Case"), TagAction::Case(CaseChange::Title)));
    case.append_item(&tag(&tr("sWAP cASE"), TagAction::Case(CaseChange::Swap)));
    more.append_submenu(Some(&tr("Change Case")), &case);
    let block = gio::Menu::new();
    block.append_item(&tag(&tr("Paragraph"), TagAction::Heading(0)));
    for n in 1..=6u8 {
        block.append_item(&tag(&trf("Heading {}", &[&n]), TagAction::Heading(n)));
    }
    more.append_submenu(Some(&tr("Change Block to")), &block);
    format.append_section(None, &more);

    // ---- Insert ----
    let insert = gio::Menu::new();
    insert.append_item(&code(&tr("Insert Hyperlink"), "link"));
    insert.append_item(&code(&tr("Insert Image"), "image"));
    insert.append(Some(&tr("Insert Special Character")), Some("view.special"));
    insert.append(Some(&tr("Manage Snippets")), Some("view.snippets"));

    // ---- Structure ----
    let structure = gio::Menu::new();
    let t = gio::Menu::new();
    t.append_item(&tag(
        &tr("Select to Matching Tag"),
        TagAction::SelectMatching,
    ));
    t.append_item(&tag(&tr("Jump to Matching Tag"), TagAction::JumpMatching));
    t.append_item(&tag(&tr("Close the Open Tag"), TagAction::CloseTag));
    structure.append_section(None, &t);
    let w = gio::Menu::new();
    w.append_item(&tag(&tr("Wrap Selection in Tag"), TagAction::Wrap));
    w.append_item(&tag(&tr("Remove Surrounding Tag"), TagAction::Unwrap));
    w.append_item(&tag(&tr("Rename Tag"), TagAction::Rename));
    w.append_item(&tag(&tr("Toggle Comment"), TagAction::Comment));
    structure.append_section(None, &w);

    // ---- View ----
    let view = gio::Menu::new();
    view.append(Some(&tr("Show Minimap")), Some("view.minimap"));
    view.append(Some(&tr("Show Hidden Characters")), Some("view.whitespace"));
    view.append(Some(&tr("Convert Typed Entities")), Some("view.entities"));
    markup_hints::append_view_menu(&view);

    let root = gio::Menu::new();
    root.append_submenu(Some(&tr("Format")), &format);
    root.append_submenu(Some(&tr("Insert")), &insert);
    root.append_submenu(Some(&tr("Structure")), &structure);
    let go = gio::Menu::new();
    go.append_item(&code(&tr("Split Tag at Caret"), "split"));
    go.append_item(&code(&tr("Go to Definition"), "goto"));
    go.append_item(&code(&tr("Sort CSS Rules"), "sort-css"));
    root.append_section(None, &go);
    root.append_submenu(Some(&tr("View")), &view);
    Built { root, structure }
}

/// The accelerator the registry gives a menu target (`edit.tag::wrap`, `tools.split`), in the GTK spelling.
fn accel_of(action: &str, param: Option<&str>) -> Option<String> {
    let target = match param {
        Some(p) => format!("{action}::{p}"),
        None => action.to_string(),
    };
    keymap::accel_for_target(&target)
}

/// Sets the `accel` attribute of every item of `menu` (and of the menus below it) to the current binding of its
/// target; items are only replaced where the label changes.
pub(super) fn refresh_accels(menu: &gio::Menu) {
    for i in 0..menu.n_items() {
        if let Some(sub) = menu
            .item_link(i, "submenu")
            .or_else(|| menu.item_link(i, "section"))
        {
            if let Some(m) = sub.downcast_ref::<gio::Menu>() {
                refresh_accels(m);
            }
            continue;
        }
        let Some(action) = menu
            .item_attribute_value(i, "action", None)
            .and_then(|v| v.str().map(str::to_string))
        else {
            continue;
        };
        let param = menu
            .item_attribute_value(i, "target", None)
            .and_then(|v| v.str().map(str::to_string));
        let want = accel_of(&action, param.as_deref());
        let have = menu
            .item_attribute_value(i, "accel", None)
            .and_then(|v| v.str().map(str::to_string));
        if want == have {
            continue;
        }
        let item = gio::MenuItem::from_model(menu, i);
        item.set_attribute_value("accel", want.as_ref().map(|a| a.to_variant()).as_ref());
        menu.remove(i);
        menu.insert_item(i, &item);
    }
}

/// Labels the menus now and again whenever the keys are changed.
pub(super) fn keep_accels(menus: &[gio::Menu]) {
    for m in menus {
        refresh_accels(m);
    }
    let weak: Vec<_> = menus.iter().map(|m| m.downgrade()).collect();
    shortcuts::on_change(move || {
        for m in weak.iter().filter_map(|w| w.upgrade()) {
            refresh_accels(&m);
        }
    });
}

/// The accelerator label a menu item shows, for tests and the dev hook (`Ctrl+B`).
pub(super) fn labels(menu: &gio::Menu) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..menu.n_items() {
        if let Some(sub) = menu
            .item_link(i, "submenu")
            .or_else(|| menu.item_link(i, "section"))
        {
            if let Some(m) = sub.downcast_ref::<gio::Menu>() {
                out.extend(labels(m));
            }
            continue;
        }
        let label = menu
            .item_attribute_value(i, "label", None)
            .and_then(|v| v.str().map(str::to_string))
            .unwrap_or_default();
        let accel = menu
            .item_attribute_value(i, "accel", None)
            .and_then(|v| v.str().map(str::to_string))
            .unwrap_or_default();
        out.push((label, accel));
    }
    out
}

/// Rows one page of the menu shows: its items and submenu entries, sections flattened (separators count as one).
pub(super) fn page_rows(menu: &gio::MenuModel) -> usize {
    let mut rows = 0;
    for i in 0..menu.n_items() {
        match menu.item_link(i, "section") {
            Some(sec) => rows += page_rows(&sec) + usize::from(i > 0),
            None => rows += 1,
        }
    }
    rows
}
