//! Preferences → Reading → Comics: the defaults of the comic reader. What you change inside a
//! comic (layout, fit, direction) is remembered for that book; these apply to comics you have not
//! changed yet. See `ui/reader_comic.rs`.

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use bookx_comic::settings::{Fit, Mode, Store};
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::preferences::{register_group, PrefCtx};

pub fn register() {
    register_group("reading", 75, group);
}

fn combo(title: &str, subtitle: &str, items: &[String], selected: usize) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    row.set_model(Some(&gtk::StringList::new(&refs)));
    row.set_selected(selected as u32);
    row
}

fn switch(title: &str, subtitle: &str, on: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_active(on);
    row
}

struct Rows {
    mode: adw::ComboRow,
    fit: adw::ComboRow,
    cover: adw::SwitchRow,
    wide: adw::SwitchRow,
    guided: adw::SwitchRow,
    grey: adw::SwitchRow,
    animate: adw::SwitchRow,
}

thread_local! {
    static EINK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static ROWS: RefCell<Option<Rows>> = const { RefCell::new(None) };
}

/// E-ink mode forces grey pages and no animation: the two rows show on and cannot be changed (PRF-D8).
pub fn set_eink(on: bool) {
    EINK.with(|e| e.set(on));
    ROWS.with(|r| {
        if let Some(rows) = r.borrow().as_ref() {
            apply_eink(rows, on);
        }
    });
}

fn apply_eink(rows: &Rows, on: bool) {
    let s = crate::comic::load().global;
    for (row, stored) in [(&rows.grey, s.grey), (&rows.animate, s.animate)] {
        row.set_sensitive(!on);
        row.set_active(on || stored);
    }
}

/// Dev hook `prefs:comic-layout=single|spread|webtoon`, `comic-fit=page|width|height|original`,
/// `comic-cover|wide|guided|grey|animate=on|off`, `comic-dump`.
pub fn dev(arg: &str) {
    ROWS.with(|r| {
        let r = r.borrow();
        let Some(rows) = r.as_ref() else {
            eprintln!("comic_prefs: not built");
            return;
        };
        let (k, v) = arg.split_once('=').unwrap_or((arg, ""));
        let on = v == "on";
        match k {
            "layout" => {
                if let Some(i) = MODES.iter().position(|m| m.name() == v) {
                    rows.mode.set_selected(i as u32);
                }
            }
            "fit" => {
                if let Some(i) = FITS.iter().position(|f| f.name() == v) {
                    rows.fit.set_selected(i as u32);
                }
            }
            "cover" => rows.cover.set_active(on),
            "wide" => rows.wide.set_active(on),
            "guided" => rows.guided.set_active(on),
            "grey" => rows.grey.set_active(on),
            "animate" => rows.animate.set_active(on),
            _ => {}
        }
        eprintln!(
            "comic_prefs: layout={} fit={} cover={} wide={} guided={} grey={} animate={} grey_sensitive={} animate_sensitive={}",
            MODES[(rows.mode.selected() as usize).min(2)].name(),
            FITS[(rows.fit.selected() as usize).min(3)].name(),
            rows.cover.is_active(),
            rows.wide.is_active(),
            rows.guided.is_active(),
            rows.grey.is_active(),
            rows.animate.is_active(),
            rows.grey.is_sensitive(),
            rows.animate.is_sensitive()
        );
    });
}

const MODES: [Mode; 3] = [Mode::Single, Mode::Spread, Mode::Webtoon];
const FITS: [Fit; 4] = [Fit::Page, Fit::Width, Fit::Height, Fit::Original];

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    EINK.with(|e| e.set(ctx.shared.cfg.borrow().eink));
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Comics"));
    g.set_description(Some(&tr(
        "Defaults for comic books (CBZ, CBT). Layout, fit and reading direction you choose inside a comic are remembered for that book.",
    )));
    let s = crate::comic::load().global;
    let mode = combo(
        &tr("Layout"),
        &tr("How pages are shown in a comic you have not changed yet"),
        &[
            tr("Single page"),
            tr("Two pages"),
            tr("Vertical scroll (webtoon)"),
        ],
        MODES.iter().position(|m| *m == s.mode).unwrap_or(0),
    );
    mode.set_widget_name("comic-layout");
    let fit = combo(
        &tr("Fit"),
        &tr("Page, width, height or original size"),
        &[
            tr("Fit page"),
            tr("Fit width"),
            tr("Fit height"),
            tr("Original size"),
        ],
        FITS.iter().position(|f| *f == s.fit).unwrap_or(0),
    );
    let cover = switch(
        &tr("Cover on its own page"),
        &tr("In two-page layout the first page stands alone"),
        s.cover_alone,
    );
    let wide = switch(
        &tr("Wide pages on their own"),
        &tr("In two-page layout a double-page picture takes the whole screen"),
        s.wide_alone,
    );
    let guided = switch(
        &tr("Guided view"),
        &tr("Space steps through the panels of a page instead of scrolling"),
        s.guided,
    );
    let grey = switch(
        &tr("Grey pages"),
        &tr("Show pages in grey. Always on in e-ink mode."),
        s.grey,
    );
    let animate = switch(
        &tr("Animate moves between panels"),
        &tr("Off when your system asks for reduced motion, and in e-ink mode"),
        s.animate,
    );
    let save: Rc<dyn Fn()> = {
        let (mode, fit, cover, wide, guided, grey, animate) = (
            mode.clone(),
            fit.clone(),
            cover.clone(),
            wide.clone(),
            guided.clone(),
            grey.clone(),
            animate.clone(),
        );
        Rc::new(move || {
            let mut store: Store = crate::comic::load();
            store.global.mode = MODES[(mode.selected() as usize).min(2)];
            store.global.fit = FITS[(fit.selected() as usize).min(3)];
            store.global.cover_alone = cover.is_active();
            store.global.wide_alone = wide.is_active();
            store.global.guided = guided.is_active();
            if !EINK.with(|e| e.get()) {
                store.global.grey = grey.is_active();
                store.global.animate = animate.is_active();
            }
            crate::comic::save(&store);
        })
    };
    {
        let s = save.clone();
        mode.connect_selected_notify(move |_| s());
        let s = save.clone();
        fit.connect_selected_notify(move |_| s());
        for row in [&cover, &wide, &guided, &grey, &animate] {
            let s = save.clone();
            row.connect_active_notify(move |_| s());
        }
    }
    g.add(&mode);
    g.add(&fit);
    g.add(&cover);
    g.add(&wide);
    g.add(&guided);
    g.add(&grey);
    g.add(&animate);
    let rows = Rows {
        mode,
        fit,
        cover,
        wide,
        guided,
        grey,
        animate,
    };
    apply_eink(&rows, EINK.with(|e| e.get()));
    ROWS.with(|r| *r.borrow_mut() = Some(rows));
    g
}
