//! The footer statistics editor: which items the reader's footer shows and in which order. One
//! group in Preferences → Reading, and the Footer Statistics dialog opened from the reader.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, pango};

use super::preferences::{PrefCtx, register_group};
use crate::footer_stats::{Align, FooterConfig, Preset, SEPARATORS};
use crate::i18n::tr;

/// The widgets of the Preferences group, for the dev hooks.
#[derive(Clone)]
struct Handles {
    items: Rc<RefCell<Vec<adw::SwitchRow>>>,
    presets: adw::ComboRow,
    align: adw::ComboRow,
    sep: adw::ComboRow,
    size: adw::SpinRow,
}

thread_local! {
    /// The open Footer Statistics dialog (dev hooks): its items list and the config it edits.
    static DIALOG: RefCell<Option<(gtk::ListBox, Rc<RefCell<FooterConfig>>, adw::Dialog)>> = const { RefCell::new(None) };
    static PREFS: RefCell<Option<Handles>> = const { RefCell::new(None) };
}

/// Dev hook (`prefs:footer-…`): drives the group in Preferences through its real widgets:
/// `toggle=KEY`, `align=spread|centered`, `preset=NAME`, `sep=N`, `size=PCT`.
pub fn dev(arg: &str) {
    let Some(h) = PREFS.with(|p| p.borrow().clone()) else {
        eprintln!("prefs_footer: no editor");
        return;
    };
    let (cmd, val) = arg.split_once('=').unwrap_or((arg, ""));
    eprintln!("prefs_footer: {cmd}={val}");
    match cmd {
        "toggle" => {
            let row = h
                .items
                .borrow()
                .iter()
                .find(|r| r.widget_name() == val)
                .cloned();
            if let Some(r) = row {
                r.set_active(!r.is_active());
                eprintln!("prefs_footer: toggled {val}");
            }
        }
        "align" => h.align.set_selected(u32::from(val != "spread")),
        "preset" => {
            if let Some(i) = Preset::ALL
                .iter()
                .position(|p| p.label().eq_ignore_ascii_case(val))
            {
                h.presets.set_selected(i as u32);
            }
        }
        "sep" => h.sep.set_selected(val.parse().unwrap_or(0)),
        "size" => h.size.set_value(val.parse().unwrap_or(100.0)),
        _ => {}
    }
}

pub fn register() {
    register_group("reading", 60, group);
}

/// The names of every item, so the settings search finds "battery" or "clock" in this group.
fn items_description() -> String {
    let names: Vec<String> = crate::footer_stats::Item::ALL
        .iter()
        .map(|i| tr(i.label()))
        .collect();
    format!(
        "{} {}",
        tr("The statistics under the page: choose what shows, in which order, and how it looks."),
        names.join(", ")
    )
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Footer Statistics"));
    g.set_description(Some(&items_description()));
    let shared = ctx.shared.clone();
    let cfg = Rc::new(RefCell::new(
        shared.cfg.borrow().footer.clone().normalized(),
    ));
    let on_change: Rc<dyn Fn(FooterConfig)> = Rc::new(move |c| {
        let mut cfg = shared.cfg.borrow_mut();
        cfg.footer = c;
        cfg.save();
        drop(cfg);
        // every open reader shows the new footer at once
        crate::settings_bus::footer().notify();
    });
    let busy = Rc::new(Cell::new(false));

    let presets = adw::ComboRow::new();
    presets.set_title(&tr("Preset"));
    presets.set_model(Some(&gtk::StringList::new(&[
        &tr("Minimal"),
        &tr("Standard"),
        &tr("Everything"),
        &tr("Custom"),
    ])));
    let align = adw::ComboRow::new();
    align.set_title(&tr("Position"));
    align.set_model(Some(&gtk::StringList::new(&[
        &tr("Page number at the left, the rest at the right"),
        &tr("One line in the middle of the page"),
    ])));
    let sep = adw::ComboRow::new();
    sep.set_title(&tr("Separator"));
    let sep_names: Vec<String> = SEPARATOR_NAMES.iter().map(|n| tr(n)).collect();
    sep.set_model(Some(&gtk::StringList::new(
        &sep_names.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    let size = adw::SpinRow::with_range(70.0, 160.0, 5.0);
    size.set_title(&tr("Text size"));
    size.set_subtitle(&tr("Percent"));
    let opacity = adw::SpinRow::with_range(30.0, 100.0, 5.0);
    opacity.set_title(&tr("Opacity"));
    opacity.set_subtitle(&tr("Percent"));

    let item_rows: Rc<RefCell<Vec<adw::SwitchRow>>> = Rc::new(RefCell::new(Vec::new()));
    // The widgets of the group in order, rebuilt when the items move.
    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let sync = {
        let (cfg, presets, align, sep, size, opacity, busy) = (
            cfg.clone(),
            presets.clone(),
            align.clone(),
            sep.clone(),
            size.clone(),
            opacity.clone(),
            busy.clone(),
        );
        Rc::new(move || {
            busy.set(true);
            let c = cfg.borrow();
            presets.set_selected(preset_index(&c));
            align.set_selected(u32::from(c.align() == Align::Centered));
            sep.set_selected(
                SEPARATORS
                    .iter()
                    .position(|s| *s == c.separator)
                    .unwrap_or(0) as u32,
            );
            size.set_value(c.size_pct as f64);
            opacity.set_value(c.opacity_pct as f64);
            busy.set(false);
        })
    };
    sync();

    let changed: Rc<dyn Fn()> = {
        let (cfg, sync, busy) = (cfg.clone(), sync.clone(), busy.clone());
        Rc::new(move || {
            if busy.get() {
                return;
            }
            sync();
            on_change(cfg.borrow().clone());
        })
    };

    let build_items = {
        let (g, cfg, changed, item_rows, rebuild) = (
            g.clone(),
            cfg.clone(),
            changed.clone(),
            item_rows.clone(),
            rebuild.clone(),
        );
        let (presets, align, sep, size, opacity) = (
            presets.clone(),
            align.clone(),
            sep.clone(),
            size.clone(),
            opacity.clone(),
        );
        let first = Rc::new(Cell::new(true));
        Rc::new(move || {
            if !first.replace(false) {
                g.remove(&presets);
                for r in item_rows.borrow().iter() {
                    g.remove(r);
                }
                for w in [&align, &sep] {
                    g.remove(w);
                }
                g.remove(&size);
                g.remove(&opacity);
            }
            item_rows.borrow_mut().clear();
            g.add(&presets);
            let items = cfg.borrow().items.clone();
            let n = items.len();
            for (i, e) in items.iter().enumerate() {
                let row = adw::SwitchRow::new();
                row.set_title(&tr(e.item.label()));
                row.set_widget_name(e.item.key());
                row.set_active(e.on);
                let item = e.item;
                {
                    let (cfg, changed) = (cfg.clone(), changed.clone());
                    row.connect_active_notify(move |r| {
                        cfg.borrow_mut().set_on(item, r.is_active());
                        changed();
                    });
                }
                let menu = gtk::MenuButton::new();
                menu.set_icon_name("view-more-symbolic");
                menu.set_valign(gtk::Align::Center);
                menu.add_css_class("flat");
                let name = format!("{}: {}", tr("Move"), tr(item.label()));
                menu.set_tooltip_text(Some(&name));
                super::audit::label(&menu, &name);
                let pop = gtk::Popover::new();
                let bx = gtk::Box::new(gtk::Orientation::Vertical, 2);
                for (label, to, enabled, id) in [
                    (tr("Move Up"), i.wrapping_sub(1), i > 0, "up"),
                    (tr("Move Down"), i + 1, i + 1 < n, "down"),
                ] {
                    let b = gtk::Button::with_label(&label);
                    b.add_css_class("flat");
                    b.set_widget_name(id);
                    b.set_sensitive(enabled);
                    let (cfg, changed, pop, rebuild) =
                        (cfg.clone(), changed.clone(), pop.clone(), rebuild.clone());
                    b.connect_clicked(move |_| {
                        pop.popdown();
                        cfg.borrow_mut().move_item(i, to);
                        if let Some(rb) = rebuild.borrow().clone() {
                            rb();
                        }
                        changed();
                    });
                    bx.append(&b);
                }
                pop.set_child(Some(&bx));
                menu.set_popover(Some(&pop));
                row.add_suffix(&menu);
                g.add(&row);
                item_rows.borrow_mut().push(row);
            }
            for w in [
                align.upcast_ref::<gtk::Widget>(),
                sep.upcast_ref(),
                size.upcast_ref(),
                opacity.upcast_ref(),
            ] {
                if let Some(r) = w.downcast_ref::<adw::PreferencesRow>() {
                    g.add(r);
                }
            }
        })
    };
    *rebuild.borrow_mut() = Some(build_items.clone() as Rc<dyn Fn()>);
    build_items();

    {
        let (cfg, changed, busy, rebuild) =
            (cfg.clone(), changed.clone(), busy.clone(), rebuild.clone());
        presets.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            let p = match d.selected() {
                0 => Preset::Minimal,
                1 => Preset::Standard,
                2 => Preset::Everything,
                _ => return,
            };
            *cfg.borrow_mut() = p.config();
            if let Some(rb) = rebuild.borrow().clone() {
                rb();
            }
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        align.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().align = Some(if d.selected() == 1 {
                Align::Centered
            } else {
                Align::Spread
            });
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        sep.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            if let Some(s) = SEPARATORS.get(d.selected() as usize) {
                cfg.borrow_mut().separator = s.to_string();
                changed();
            }
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        size.connect_value_notify(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().size_pct = s.value() as u32;
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        opacity.connect_value_notify(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().opacity_pct = s.value() as u32;
            changed();
        });
    }
    PREFS.with(|p| {
        *p.borrow_mut() = Some(Handles {
            items: item_rows,
            presets,
            align,
            sep,
            size,
        })
    });
    g
}

const SEPARATOR_NAMES: [&str; 4] = ["Dot", "Bar", "Bullet", "Space"];

/// Draws the footer labels at the size and opacity of the config.
pub fn style_labels(labels: &[&gtk::Label], cfg: &FooterConfig) {
    let al = pango::AttrList::new();
    al.insert(pango::AttrFloat::new_scale(cfg.size_pct as f64 / 100.0));
    for l in labels {
        l.set_attributes(Some(&al));
        l.set_opacity(cfg.opacity_pct as f64 / 100.0);
    }
}

/// The names of the items, for the dev dump.
pub fn names(cfg: &FooterConfig) -> String {
    cfg.items
        .iter()
        .filter(|e| e.on)
        .map(|e| e.item.key())
        .collect::<Vec<_>>()
        .join(",")
}

// ---- the Footer Statistics dialog (RDR-R23) ------------------------------------------------------

fn preset_index(c: &FooterConfig) -> u32 {
    match c.preset() {
        Some(Preset::Minimal) => 0,
        Some(Preset::Standard) => 1,
        Some(Preset::Everything) => 2,
        None => 3,
    }
}

/// Fills the items list: a row per item with its check box and up and down buttons.
fn fill_items(list: &gtk::ListBox, cfg: &Rc<RefCell<FooterConfig>>, changed: &Rc<dyn Fn()>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let items = cfg.borrow().items.clone();
    let n = items.len();
    for (i, e) in items.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&tr(e.item.label()));
        row.set_widget_name(e.item.key());
        let check = gtk::CheckButton::new();
        check.set_active(e.on);
        check.set_valign(gtk::Align::Center);
        super::audit::label(&check, &tr(e.item.label()));
        row.add_prefix(&check);
        row.set_activatable_widget(Some(&check));
        let item = e.item;
        {
            let (cfg, changed) = (cfg.clone(), changed.clone());
            check.connect_toggled(move |c| {
                cfg.borrow_mut().set_on(item, c.is_active());
                changed();
            });
        }
        for (icon, tip, to, enabled) in [
            ("go-up-symbolic", tr("Move up"), i.wrapping_sub(1), i > 0),
            ("go-down-symbolic", tr("Move down"), i + 1, i + 1 < n),
        ] {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_sensitive(enabled);
            let name = format!("{tip}: {}", tr(item.label()));
            b.set_tooltip_text(Some(&name));
            super::audit::label(&b, &name);
            b.set_widget_name(if icon == "go-up-symbolic" {
                "up"
            } else {
                "down"
            });
            let (cfg, changed, list) = (cfg.clone(), changed.clone(), list.clone());
            b.connect_clicked(move |_| {
                cfg.borrow_mut().move_item(i, to);
                fill_items(&list, &cfg, &changed);
                changed();
            });
            row.add_suffix(&b);
        }
        list.append(&row);
    }
}

/// "Footer Statistics": boxed lists for the preset, the items (with their order), how it looks, and
/// "Only for this book". `on_change` gets the new config after every edit, `on_scope` the switch.
pub fn dialog(
    initial: FooterConfig,
    own: bool,
    on_change: Rc<dyn Fn(FooterConfig)>,
    on_scope: Rc<dyn Fn(bool)>,
) -> adw::Dialog {
    let cfg = Rc::new(RefCell::new(initial.normalized()));
    let busy = Rc::new(Cell::new(false));
    let dlg = super::dialog_toast::new_dialog();
    dlg.set_title(&tr("Footer Statistics"));
    dlg.set_content_width(440);
    dlg.set_content_height(560);

    let page = adw::PreferencesPage::new();
    let g_preset = adw::PreferencesGroup::new();
    let preset = adw::ComboRow::new();
    preset.set_title(&tr("Preset"));
    preset.set_model(Some(&gtk::StringList::new(&[
        &tr("Minimal"),
        &tr("Standard"),
        &tr("Everything"),
        &tr("Custom"),
    ])));
    g_preset.add(&preset);
    page.add(&g_preset);

    let g_items = adw::PreferencesGroup::new();
    g_items.set_title(&tr("Items"));
    g_items.set_description(Some(&tr("Items are shown in this order.")));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    super::audit::label(&list, &tr("Footer items"));
    g_items.add(&list);
    page.add(&g_items);

    let g_look = adw::PreferencesGroup::new();
    g_look.set_title(&tr("Look"));
    let align = adw::ComboRow::new();
    align.set_title(&tr("Position"));
    align.set_model(Some(&gtk::StringList::new(&[
        &tr("Page number at the left, the rest at the right"),
        &tr("One line in the middle of the page"),
    ])));
    let sep = adw::ComboRow::new();
    sep.set_title(&tr("Separator"));
    let sep_names: Vec<String> = SEPARATOR_NAMES.iter().map(|n| tr(n)).collect();
    sep.set_model(Some(&gtk::StringList::new(
        &sep_names.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    let size = adw::SpinRow::with_range(70.0, 160.0, 5.0);
    size.set_title(&tr("Text size"));
    size.set_subtitle(&tr("Percent"));
    let opacity = adw::SpinRow::with_range(30.0, 100.0, 5.0);
    opacity.set_title(&tr("Opacity"));
    opacity.set_subtitle(&tr("Percent"));
    for r in [
        align.upcast_ref::<gtk::Widget>(),
        sep.upcast_ref(),
        size.upcast_ref(),
        opacity.upcast_ref(),
    ] {
        g_look.add(r);
    }
    page.add(&g_look);

    let g_scope = adw::PreferencesGroup::new();
    let scope = adw::SwitchRow::new();
    scope.set_title(&tr("Only for this book"));
    scope.set_subtitle(&tr("Other books keep the footer you chose in Preferences."));
    scope.set_active(own);
    scope.connect_active_notify(move |r| on_scope(r.is_active()));
    g_scope.add(&scope);
    page.add(&g_scope);

    let sync = {
        let (cfg, preset, align, sep, size, opacity, busy) = (
            cfg.clone(),
            preset.clone(),
            align.clone(),
            sep.clone(),
            size.clone(),
            opacity.clone(),
            busy.clone(),
        );
        Rc::new(move || {
            busy.set(true);
            let c = cfg.borrow();
            preset.set_selected(preset_index(&c));
            align.set_selected(u32::from(c.align() == Align::Centered));
            sep.set_selected(
                SEPARATORS
                    .iter()
                    .position(|s| *s == c.separator)
                    .unwrap_or(0) as u32,
            );
            size.set_value(c.size_pct as f64);
            opacity.set_value(c.opacity_pct as f64);
            busy.set(false);
        })
    };
    sync();
    let changed: Rc<dyn Fn()> = {
        let (cfg, sync, busy) = (cfg.clone(), sync.clone(), busy.clone());
        Rc::new(move || {
            if busy.get() {
                return;
            }
            sync();
            on_change(cfg.borrow().clone());
        })
    };
    fill_items(&list, &cfg, &changed);
    {
        let (cfg, list, changed, busy) = (cfg.clone(), list.clone(), changed.clone(), busy.clone());
        preset.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            let p = match d.selected() {
                0 => Preset::Minimal,
                1 => Preset::Standard,
                2 => Preset::Everything,
                _ => return,
            };
            *cfg.borrow_mut() = p.config();
            fill_items(&list, &cfg, &changed);
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        align.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().align = Some(if d.selected() == 1 {
                Align::Centered
            } else {
                Align::Spread
            });
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        sep.connect_selected_notify(move |d| {
            if busy.get() {
                return;
            }
            if let Some(s) = SEPARATORS.get(d.selected() as usize) {
                cfg.borrow_mut().separator = s.to_string();
                changed();
            }
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        size.connect_value_notify(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().size_pct = s.value() as u32;
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        opacity.connect_value_notify(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().opacity_pct = s.value() as u32;
            changed();
        });
    }
    let tb = adw::ToolbarView::new();
    tb.add_top_bar(&adw::HeaderBar::new());
    tb.set_content(Some(&page));
    dlg.set_child(Some(&tb));
    DIALOG.with(|d| *d.borrow_mut() = Some((list, cfg, dlg.clone())));
    dlg
}

/// Dev hook (`reader:footer-dialog=…`): `dump`, `toggle=KEY`, `up=KEY`, `down=KEY` (the real buttons of
/// the open dialog); prints `footer_dialog: open=B items=a,b,c order=<all keys in order>`.
pub fn dev_dialog(arg: &str) -> String {
    let Some((list, cfg, dlg)) = DIALOG.with(|d| d.borrow().clone()) else {
        return "footer_dialog: none".into();
    };
    let (cmd, key) = arg.split_once('=').unwrap_or((arg, ""));
    let mut row = list.first_child();
    while let Some(r) = row {
        if r.widget_name() == key {
            let find = |name: &str| {
                let mut found = None;
                fn walk(w: &gtk::Widget, name: &str, out: &mut Option<gtk::Widget>) {
                    if w.widget_name() == name {
                        *out = Some(w.clone());
                        return;
                    }
                    let mut c = w.first_child();
                    while let Some(x) = c {
                        walk(&x, name, out);
                        c = x.next_sibling();
                    }
                }
                walk(&r, name, &mut found);
                found
            };
            match cmd {
                "toggle" => {
                    let mut c = None;
                    fn check(w: &gtk::Widget, out: &mut Option<gtk::CheckButton>) {
                        if let Some(cb) = w.downcast_ref::<gtk::CheckButton>() {
                            *out = Some(cb.clone());
                            return;
                        }
                        let mut x = w.first_child();
                        while let Some(y) = x {
                            check(&y, out);
                            x = y.next_sibling();
                        }
                    }
                    check(&r, &mut c);
                    if let Some(c) = c {
                        c.set_active(!c.is_active());
                    }
                }
                "up" | "down" => {
                    if let Some(b) = find(cmd).and_downcast::<gtk::Button>() {
                        b.emit_clicked();
                    }
                }
                _ => {}
            }
            break;
        }
        row = r.next_sibling();
    }
    let c = cfg.borrow();
    format!(
        "footer_dialog: open={} items={} order={}",
        dlg.is_mapped(),
        names(&c),
        c.items
            .iter()
            .map(|e| e.item.key())
            .collect::<Vec<_>>()
            .join(",")
    )
}
