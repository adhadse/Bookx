//! Preferences → Reading: the settings of the small reading aids (`crate::aids_prefs`): page-turn
//! animation, auto-turn interval, night schedule, citation, page turner keys and the Archive shelf.

use std::rc::Rc;

use bookx_text::aids::{format_hhmm, parse_hhmm, PageAnimation, AUTO_TURN_RANGE};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{register_group, PrefCtx};
use crate::aids_prefs::{remote_lines, remote_text, ReaderAids, ARCHIVE_SHELF};
use crate::i18n::{tr, trf};

pub fn register() {
    register_group("reading", 80, group);
}

const ANIMATIONS: [PageAnimation; 3] = [
    PageAnimation::None,
    PageAnimation::Slide,
    PageAnimation::Fade,
];

fn entry(title: &str, text: &str) -> adw::EntryRow {
    let r = adw::EntryRow::new();
    r.set_title(&tr(title));
    r.set_text(text);
    r
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Reading aids"));
    g.set_description(Some(&tr(
        "Page turning, a schedule for the evening, copying with a citation and the keys of a page turner.",
    )));
    let a = ctx.shared.cfg.borrow().aids.clone();

    let anim = adw::ComboRow::new();
    anim.set_title(&tr("Page-turn animation"));
    anim.set_subtitle(&tr(
        "Off when your system asks for reduced motion, and in e-ink mode",
    ));
    let labels = [tr("None"), tr("Slide"), tr("Fade")];
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    anim.set_model(Some(&gtk::StringList::new(&refs)));
    anim.set_selected(
        ANIMATIONS
            .iter()
            .position(|k| *k == a.page_animation)
            .unwrap_or(0) as u32,
    );

    let auto = adw::SpinRow::with_range(AUTO_TURN_RANGE.0 as f64, 600.0, 1.0);
    auto.set_title(&tr("Auto-turn interval"));
    auto.set_subtitle(&tr(
        "Seconds between pages when Auto-Turn Pages is on (command palette)",
    ));
    auto.set_value(a.auto_turn_secs as f64);

    let night = adw::SwitchRow::new();
    night.set_title(&tr("Evening schedule"));
    night.set_subtitle(&tr("A warm tint and the dark theme between the two times"));
    night.set_active(a.night.enabled);
    let from = entry("Evening starts (hh:mm)", &format_hhmm(a.night.from));
    let to = entry("Evening ends (hh:mm)", &format_hhmm(a.night.to));
    let warmth = adw::SpinRow::with_range(0.0, 1.0, 0.1);
    warmth.set_title(&tr("Warmth"));
    warmth.set_digits(1);
    warmth.set_value(a.night.warmth);
    let dark = adw::SwitchRow::new();
    dark.set_title(&tr("Use the dark theme in the evening"));
    dark.set_subtitle(&tr("Only when the reading theme is Automatic"));
    dark.set_active(a.night.dark);

    let cite_all = adw::SwitchRow::new();
    cite_all.set_title(&tr("Copy with a citation"));
    cite_all.set_subtitle(&tr(
        "Copy always adds the source; Ctrl+Shift+C does it for one copy",
    ));
    cite_all.set_active(a.copy_with_citation);
    let template = entry("Citation template", &a.citation_template);
    template.set_tooltip_text(Some(&tr(
        "Use {text}, {title}, {author}, {chapter}, {page}, {date} and {isbn}. Empty uses the default.",
    )));

    let keys = entry("Page turner keys", &remote_text(&a.remote_keys));
    keys.set_tooltip_text(Some(&tr(
        "Key names with an action, such as F7=prev-chapter. Actions: next-page, prev-page, next-chapter, prev-chapter, toggle-chrome.",
    )));

    let archive = adw::SwitchRow::new();
    archive.set_title(&tr("Archive finished books"));
    archive.set_subtitle(&trf(
        "Adds a book to the shelf “{}” when you reach its end",
        &[&ARCHIVE_SHELF],
    ));
    archive.set_active(a.archive_finished);

    let save = {
        let shared = ctx.shared.clone();
        let (anim, auto, night, from, to, warmth, dark) = (
            anim.clone(),
            auto.clone(),
            night.clone(),
            from.clone(),
            to.clone(),
            warmth.clone(),
            dark.clone(),
        );
        let (cite_all, template, keys, archive) = (
            cite_all.clone(),
            template.clone(),
            keys.clone(),
            archive.clone(),
        );
        Rc::new(move || {
            let mut c = shared.cfg.borrow_mut();
            let mut n = c.aids.night;
            n.enabled = night.is_active();
            if let Some(m) = parse_hhmm(&from.text()) {
                n.from = m;
            }
            if let Some(m) = parse_hhmm(&to.text()) {
                n.to = m;
            }
            n.warmth = warmth.value();
            n.dark = dark.is_active();
            c.aids = ReaderAids {
                night: n,
                page_animation: ANIMATIONS
                    .get(anim.selected() as usize)
                    .copied()
                    .unwrap_or_default(),
                auto_turn_secs: auto.value() as u32,
                citation_template: template.text().to_string(),
                copy_with_citation: cite_all.is_active(),
                remote_keys: remote_lines(&keys.text()),
                archive_finished: archive.is_active(),
            }
            .clamped();
            c.save();
        })
    };
    for r in [&night, &dark, &cite_all, &archive] {
        let s = save.clone();
        r.connect_active_notify(move |_| s());
    }
    {
        let s = save.clone();
        anim.connect_selected_notify(move |_| s());
        for r in [&auto, &warmth] {
            let s = save.clone();
            r.connect_value_notify(move |_| s());
        }
        for r in [&from, &to, &template, &keys] {
            let s = save.clone();
            r.connect_changed(move |_| s());
        }
    }
    for w in [
        anim.upcast_ref::<gtk::Widget>(),
        auto.upcast_ref(),
        night.upcast_ref(),
        from.upcast_ref(),
        to.upcast_ref(),
        warmth.upcast_ref(),
        dark.upcast_ref(),
        cite_all.upcast_ref(),
        template.upcast_ref(),
        keys.upcast_ref(),
        archive.upcast_ref(),
    ] {
        g.add(w);
    }
    g
}
