//! The footer statistics editor: which items the reader's footer shows and in which order. One
//! widget, used by Preferences → Reading and by the footer's own "Configure Footer" popover.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, pango};

use super::preferences::{register_group, PrefCtx};
use crate::footer_stats::{Align, FooterConfig, Preset, SEPARATORS};
use crate::i18n::tr;

/// The widgets of the editor, for the dev hooks (the last one built; the Preferences one is kept apart).
#[derive(Clone)]
struct Handles {
    list: gtk::ListBox,
    presets: gtk::DropDown,
    align: gtk::DropDown,
    sep: gtk::DropDown,
    size: gtk::Scale,
}

thread_local! {
    static LAST: RefCell<Option<Handles>> = const { RefCell::new(None) };
    static PREFS: RefCell<Option<Handles>> = const { RefCell::new(None) };
}

/// Dev hook (`prefs:footer-…`): drives the editor in Preferences through its real widgets:
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
            let label = crate::footer_stats::Item::from_key(val).map(|i| tr(i.label()));
            let mut row = h.list.first_child();
            while let Some(r) = row {
                if let Some(c) = r
                    .first_child()
                    .and_then(|b| b.first_child())
                    .and_then(|w| w.downcast::<gtk::CheckButton>().ok())
                {
                    if Some(c.label().unwrap_or_default().to_string()) == label {
                        c.set_active(!c.is_active());
                        eprintln!("prefs_footer: toggled {val}");
                        break;
                    }
                }
                row = r.next_sibling();
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

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Footer Stats"));
    g.set_description(Some(&tr(
        "The statistics under the page: choose what shows, in which order, and how it looks.",
    )));
    let shared = ctx.shared.clone();
    let cfg = shared.cfg.borrow().footer.clone();
    let editor = editor(
        cfg,
        Rc::new(move |c| {
            let mut cfg = shared.cfg.borrow_mut();
            cfg.footer = c;
            cfg.save();
            drop(cfg);
            // every open reader shows the new footer at once
            crate::settings_bus::footer().notify();
        }),
    );
    g.add(&editor);
    LAST.with(|l| PREFS.with(|p| *p.borrow_mut() = l.borrow().clone()));
    g
}

const SEPARATOR_NAMES: [&str; 4] = ["Dot", "Bar", "Bullet", "Space"];

fn fill(list: &gtk::ListBox, cfg: &Rc<RefCell<FooterConfig>>, changed: &Rc<dyn Fn()>) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let items = cfg.borrow().items.clone();
    let n = items.len();
    for (i, e) in items.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_margin_start(6);
        row.set_margin_end(6);
        row.set_margin_top(2);
        row.set_margin_bottom(2);
        let check = gtk::CheckButton::with_label(&tr(e.item.label()));
        check.set_active(e.on);
        check.set_hexpand(true);
        let item = e.item;
        {
            let (cfg, changed) = (cfg.clone(), changed.clone());
            check.connect_toggled(move |c| {
                cfg.borrow_mut().set_on(item, c.is_active());
                changed();
            });
        }
        row.append(&check);
        for (icon, tip, to, enabled) in [
            ("go-up-symbolic", tr("Move up"), i.wrapping_sub(1), i > 0),
            ("go-down-symbolic", tr("Move down"), i + 1, i + 1 < n),
        ] {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_sensitive(enabled);
            b.set_tooltip_text(Some(&format!("{tip}: {}", tr(item.label()))));
            super::audit::label(&b, &format!("{tip}: {}", tr(item.label())));
            let (cfg, changed, list) = (cfg.clone(), changed.clone(), list.clone());
            b.connect_clicked(move |_| {
                cfg.borrow_mut().move_item(i, to);
                fill(&list, &cfg, &changed);
                changed();
            });
            row.append(&b);
        }
        list.append(&row);
    }
}

/// The editor. `on_change` gets the new config after every edit.
pub fn editor(initial: FooterConfig, on_change: Rc<dyn Fn(FooterConfig)>) -> gtk::Box {
    let cfg = Rc::new(RefCell::new(initial.normalized()));
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let busy = Rc::new(Cell::new(false));

    let presets = gtk::DropDown::from_strings(&[
        &tr("Minimal"),
        &tr("Standard"),
        &tr("Everything"),
        &tr("Custom"),
    ]);
    super::audit::label(&presets, &tr("Footer preset"));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    super::audit::label(&list, &tr("Footer items"));
    let align = gtk::DropDown::from_strings(&[
        &tr("Page number at the left, the rest at the right"),
        &tr("One line in the middle of the page"),
    ]);
    super::audit::label(&align, &tr("Footer position"));
    let sep = gtk::DropDown::from_strings(
        &SEPARATOR_NAMES
            .iter()
            .map(|n| tr(n))
            .collect::<Vec<_>>()
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
    );
    super::audit::label(&sep, &tr("Separator"));
    let size = gtk::Scale::with_range(gtk::Orientation::Horizontal, 70.0, 160.0, 5.0);
    size.set_draw_value(true);
    size.set_hexpand(true);
    size.set_value_pos(gtk::PositionType::Right);
    super::audit::label(&size, &tr("Footer text size (percent)"));
    let opacity = gtk::Scale::with_range(gtk::Orientation::Horizontal, 30.0, 100.0, 5.0);
    opacity.set_draw_value(true);
    opacity.set_hexpand(true);
    opacity.set_value_pos(gtk::PositionType::Right);
    super::audit::label(&opacity, &tr("Footer text opacity (percent)"));

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
            presets.set_selected(match c.preset() {
                Some(Preset::Minimal) => 0,
                Some(Preset::Standard) => 1,
                Some(Preset::Everything) => 2,
                None => 3,
            });
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
    fill(&list, &cfg, &changed);

    {
        let (cfg, list, changed, busy) = (cfg.clone(), list.clone(), changed.clone(), busy.clone());
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
            fill(&list, &cfg, &changed);
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
        size.connect_value_changed(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().size_pct = s.value() as u32;
            changed();
        });
    }
    {
        let (cfg, changed, busy) = (cfg.clone(), changed.clone(), busy.clone());
        opacity.connect_value_changed(move |s| {
            if busy.get() {
                return;
            }
            cfg.borrow_mut().opacity_pct = s.value() as u32;
            changed();
        });
    }

    let labelled = |text: &str, w: &gtk::Widget| {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let l = gtk::Label::new(Some(text));
        l.set_xalign(0.0);
        l.set_width_chars(10);
        r.append(&l);
        w.set_hexpand(true);
        r.append(w);
        r
    };
    LAST.with(|l| {
        *l.borrow_mut() = Some(Handles {
            list: list.clone(),
            presets: presets.clone(),
            align: align.clone(),
            sep: sep.clone(),
            size: size.clone(),
        })
    });
    root.append(&labelled(&tr("Preset"), presets.upcast_ref()));
    let hint = gtk::Label::new(Some(&tr("Items are shown in this order.")));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_xalign(0.0);
    root.append(&hint);
    root.append(&list);
    root.append(&labelled(&tr("Position"), align.upcast_ref()));
    root.append(&labelled(&tr("Separator"), sep.upcast_ref()));
    root.append(&labelled(&tr("Text size"), size.upcast_ref()));
    root.append(&labelled(&tr("Opacity"), opacity.upcast_ref()));
    root
}

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
