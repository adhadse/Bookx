//! Preferences → Library → Metadata sources: which source wins for each field when several know it
//! (`crate::source_priority`). “Automatic” keeps the built-in order.

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{PrefCtx, register_group};
use crate::i18n::tr;
use bookx_sources::authors::NameOrder;
use bookx_sources::sources::{Field, SOURCE_IDS};

thread_local! {
    static ROWS: std::cell::RefCell<Vec<(Field, adw::ComboRow)>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    static NAME_ORDER: std::cell::RefCell<Option<adw::ComboRow>> = const { std::cell::RefCell::new(None) };
}

pub fn register() {
    register_group("library", 45, group);
}

fn field_label(f: Field) -> String {
    match f {
        Field::Title => tr("Title"),
        Field::Authors => tr("Authors"),
        Field::Publisher => tr("Publisher"),
        Field::Published => tr("Publication date"),
        Field::Description => tr("Description"),
        Field::Tags => tr("Tags"),
        Field::Language => tr("Language"),
        Field::Series => tr("Series"),
        Field::Cover => tr("Cover"),
    }
}

fn source_label(id: &str) -> String {
    match id {
        "openlibrary" => tr("Open Library"),
        "google" => tr("Google Books"),
        "wikidata" => tr("Wikidata"),
        other => other.to_string(),
    }
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Metadata sources"));
    g.set_description(Some(&tr(
        "When more than one source knows a field, the one chosen here wins. Automatic keeps the built-in order.",
    )));
    let mut names = vec![tr("Automatic")];
    names.extend(SOURCE_IDS.iter().map(|s| source_label(s)));
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let order = adw::ComboRow::new();
    order.set_title(&tr("Author name order"));
    order.set_subtitle(&tr(
        "How the sort name of an author is made when metadata is fetched. Automatic leaves Chinese, Japanese and Korean names as written.",
    ));
    order.set_model(Some(&gtk::StringList::new(&[
        &tr("Automatic"),
        &tr("Last name, first name"),
        &tr("As written"),
    ])));
    order.set_selected(match ctx.shared.cfg.borrow().source_priority.name_order() {
        NameOrder::Auto => 0,
        NameOrder::LastFirst => 1,
        NameOrder::AsWritten => 2,
    });
    {
        let shared = ctx.shared.clone();
        order.connect_selected_notify(move |r| {
            let o = [NameOrder::Auto, NameOrder::LastFirst, NameOrder::AsWritten]
                [(r.selected() as usize).min(2)];
            let mut c = shared.cfg.borrow_mut();
            c.source_priority.name_order = o.key().to_string();
            crate::source_priority::install(&c.source_priority);
            c.save();
        });
    }
    g.add(&order);
    NAME_ORDER.with(|n| *n.borrow_mut() = Some(order));
    ROWS.with(|r| r.borrow_mut().clear());
    for field in Field::ALL {
        let row = adw::ComboRow::new();
        row.set_title(&field_label(field));
        row.set_model(Some(&gtk::StringList::new(&refs)));
        let current = ctx
            .shared
            .cfg
            .borrow()
            .source_priority
            .preferred(field)
            .map(String::from);
        row.set_selected(
            current
                .and_then(|c| SOURCE_IDS.iter().position(|s| *s == c))
                .map_or(0, |p| p as u32 + 1),
        );
        let shared = ctx.shared.clone();
        row.connect_selected_notify(move |r| {
            let sel = r.selected() as usize;
            let src = sel.checked_sub(1).and_then(|i| SOURCE_IDS.get(i)).copied();
            let mut c = shared.cfg.borrow_mut();
            if c.source_priority.set(field, src) {
                crate::source_priority::install(&c.source_priority);
                c.save();
            }
        });
        ROWS.with(|r| r.borrow_mut().push((field, row.clone())));
        g.add(&row);
    }
    g
}

/// Dev hook `prefs:sources-set=FIELD;SOURCE` (SOURCE `auto` clears) and `prefs:sources-dump`
/// (`prefs_sources: description=google ...`): drives the rows of the open Preferences.
pub fn dev(arg: &str) {
    if let Some(v) = arg.strip_prefix("order=") {
        let idx = match v {
            "last-first" => 1,
            "as-written" => 2,
            _ => 0,
        };
        NAME_ORDER.with(|n| {
            if let Some(r) = n.borrow().as_ref() {
                r.set_selected(idx);
            }
        });
        return;
    }
    if let Some(v) = arg.strip_prefix("set=") {
        if let Some((f, s)) = v.split_once(';') {
            let idx = SOURCE_IDS
                .iter()
                .position(|x| *x == s)
                .map_or(0, |p| p as u32 + 1);
            ROWS.with(|r| {
                for (field, row) in r.borrow().iter() {
                    if field.key() == f {
                        row.set_selected(idx);
                    }
                }
            });
        }
        return;
    }
    let parts: Vec<String> = ROWS.with(|r| {
        r.borrow()
            .iter()
            .map(|(f, row)| {
                let sel = row.selected() as usize;
                let name = sel
                    .checked_sub(1)
                    .and_then(|i| SOURCE_IDS.get(i))
                    .unwrap_or(&"auto");
                format!("{}={name}", f.key())
            })
            .collect()
    });
    let order = NAME_ORDER.with(|n| n.borrow().as_ref().map_or(0, |r| r.selected()));
    eprintln!("prefs_sources: {} name_order={order}", parts.join(" "));
}
