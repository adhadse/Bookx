//! *Conversion Options* (Library → Convert To): the options of EPUB, KEPUB, MOBI, AZW3 and FB2
//! output beyond the built-in presets, and how many books are converted at a time. Every change
//! is saved at once (`~/.config/bookx/convert.json`, see `crate::convert_prefs`) and used by
//! Convert To, Send to Device and e-mail conversions.

use crate::i18n::tr;
use std::cell::RefCell;
use std::rc::Rc;

use bookx_convert::writers::EpubVersion;
use bookx_convert::ConvertOptions;
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use crate::convert_prefs::{self as prefs, FORMATS};
use crate::export;

struct Rows {
    format: adw::ComboRow,
    version: adw::ComboRow,
    smarten: adw::SwitchRow,
    straighten: adw::SwitchRow,
    unused_css: adw::SwitchRow,
    blank: adw::SwitchRow,
    margins: adw::SwitchRow,
    split: adw::SwitchRow,
    toc: adw::SwitchRow,
    parallel: adw::SpinRow,
    reset: adw::ActionRow,
    dialog: adw::Dialog,
    current: Rc<RefCell<ConvertOptions>>,
}

thread_local! {
    static OPEN: RefCell<Option<Rc<Rows>>> = const { RefCell::new(None) };
}

fn switch(title: &str, subtitle: &str, name: &str) -> adw::SwitchRow {
    let r = adw::SwitchRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.set_widget_name(name);
    r
}

fn target_of(i: u32) -> &'static export::Target {
    let id = FORMATS[(i as usize).min(FORMATS.len() - 1)];
    export::target(id).unwrap_or(&export::TARGETS[0])
}

/// Split size used by the switch: files over 260 kB are cut at headings (the limit old readers
/// handle comfortably).
const SPLIT_BYTES: usize = 260 * 1024;

fn show(rows: &Rows) {
    let t = target_of(rows.format.selected());
    let opts = export::options_with(t, prefs::user_options(t.id)).unwrap_or_default();
    *rows.current.borrow_mut() = opts.clone();
    let epubish = matches!(t.id, "epub" | "kepub");
    let mobi = matches!(t.id, "mobi" | "azw3");
    rows.version.set_visible(epubish);
    rows.toc.set_visible(mobi);
    rows.version.set_selected(match opts.epub_version {
        None => 0,
        Some(EpubVersion::V2) => 1,
        Some(EpubVersion::V3) => 2,
    });
    rows.smarten.set_active(opts.smarten_punctuation);
    rows.straighten.set_active(opts.unsmarten_punctuation);
    rows.unused_css.set_active(opts.remove_unused_css);
    rows.blank.set_active(opts.remove_blank_paragraphs);
    rows.margins.set_active(opts.remove_fake_margins);
    rows.split.set_active(opts.split_large_html.is_some());
    rows.toc.set_active(opts.mobi_inline_toc);
    rows.reset
        .set_sensitive(prefs::user_options(t.id).is_some());
}

/// Opens the dialog over `parent`; a second call presents the open one.
pub fn present(parent: &impl IsA<gtk::Widget>) {
    if let Some(r) = OPEN.with(|o| o.borrow().clone()) {
        r.dialog.present(Some(parent));
        return;
    }
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Conversion Options"));
    dialog.set_content_width(480);
    dialog.set_content_height(640);

    let format = adw::ComboRow::new();
    format.set_title(&tr("Format"));
    format.set_subtitle(&tr("The options below apply to this output format"));
    format.set_widget_name("convert-format");
    let labels: Vec<&str> = FORMATS.iter().map(|id| target_of_id(id).label).collect();
    // the labels of the menu are translated where they are shown; here the names are format names
    format.set_model(Some(&gtk::StringList::new(&labels)));

    let version = adw::ComboRow::new();
    version.set_title(&tr("EPUB version"));
    version.set_widget_name("convert-epub-version");
    version.set_model(Some(&gtk::StringList::new(&[
        &tr("Automatic"),
        &tr("EPUB 2"),
        &tr("EPUB 3"),
    ])));
    let smarten = switch(
        &tr("Smarten punctuation"),
        &tr("Curly quotes, dashes and ellipses"),
        "convert-smarten",
    );
    let straighten = switch(
        &tr("Straighten punctuation"),
        &tr("Plain quotes, dashes and ellipses"),
        "convert-straighten",
    );
    let unused_css = switch(
        &tr("Remove unused styles"),
        &tr("Drops style rules no page uses"),
        "convert-unused-css",
    );
    let blank = switch(
        &tr("Remove blank paragraphs"),
        &tr("Empty paragraphs that only make space"),
        "convert-blank",
    );
    let margins = switch(
        &tr("Remove fake margins"),
        &tr("Indents made with spaces or empty lines"),
        "convert-margins",
    );
    let split = switch(
        &tr("Split large files"),
        &tr("Cuts very large chapters into smaller files for slow readers"),
        "convert-split",
    );
    let toc = switch(
        &tr("Contents page in the text"),
        &tr("Adds a page with the table of contents to the book"),
        "convert-inline-toc",
    );
    let parallel = adw::SpinRow::with_range(1.0, bookx_convert::queue::MAX_PARALLEL as f64, 1.0);
    parallel.set_title(&tr("Convert at a time"));
    parallel.set_subtitle(&tr(
        "How many books are converted together. More is faster and uses more memory.",
    ));
    parallel.set_widget_name("convert-parallel");
    parallel.set_value(prefs::parallel_now() as f64);
    let reset = adw::ActionRow::builder()
        .title(tr("Use the Built-in Options"))
        .subtitle(tr("Forget what you changed for this format"))
        .activatable(true)
        .build();
    reset.set_widget_name("convert-reset");

    let options = adw::PreferencesGroup::new();
    options.set_title(&tr("Options"));
    for r in [&format, &version] {
        options.add(r);
    }
    let cleanup = adw::PreferencesGroup::new();
    for r in [
        &smarten,
        &straighten,
        &unused_css,
        &blank,
        &margins,
        &split,
        &toc,
    ] {
        cleanup.add(r);
    }
    let speed = adw::PreferencesGroup::new();
    speed.set_title(&tr("Speed"));
    speed.add(&parallel);
    let again = adw::PreferencesGroup::new();
    again.add(&reset);

    let page = adw::PreferencesPage::new();
    page.add(&options);
    page.add(&cleanup);
    page.add(&speed);
    page.add(&again);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let rows = Rc::new(Rows {
        format,
        version,
        smarten,
        straighten,
        unused_css,
        blank,
        margins,
        split,
        toc,
        parallel,
        reset,
        dialog: dialog.clone(),
        current: Rc::new(RefCell::new(ConvertOptions::default())),
    });
    show(&rows);

    let updating = Rc::new(std::cell::Cell::new(false));
    let save: Rc<dyn Fn()> = {
        let (rows, updating) = (rows.clone(), updating.clone());
        Rc::new(move || {
            if updating.get() {
                return;
            }
            let t = target_of(rows.format.selected());
            let mut o = rows.current.borrow().clone();
            o.epub_version = match rows.version.selected() {
                1 => Some(EpubVersion::V2),
                2 => Some(EpubVersion::V3),
                _ => None,
            };
            o.smarten_punctuation = rows.smarten.is_active();
            o.unsmarten_punctuation = rows.straighten.is_active();
            o.remove_unused_css = rows.unused_css.is_active();
            o.remove_blank_paragraphs = rows.blank.is_active();
            o.remove_fake_margins = rows.margins.is_active();
            o.split_large_html = rows.split.is_active().then_some(SPLIT_BYTES);
            o.mobi_inline_toc = rows.toc.is_active();
            let builtin = export::builtin_options(t).unwrap_or_default();
            let mut p = prefs::load();
            p.set_options(t.id, o.clone(), &builtin);
            prefs::save(&p);
            *rows.current.borrow_mut() = o;
            rows.reset.set_sensitive(p.options_for(t.id).is_some());
        })
    };
    for r in [
        &rows.smarten,
        &rows.straighten,
        &rows.unused_css,
        &rows.blank,
        &rows.margins,
        &rows.split,
        &rows.toc,
    ] {
        let s = save.clone();
        r.connect_active_notify(move |_| s());
    }
    {
        let s = save.clone();
        rows.version.connect_selected_notify(move |_| s());
    }
    {
        let (rows2, u) = (rows.clone(), updating.clone());
        rows.format.connect_selected_notify(move |_| {
            u.set(true);
            show(&rows2);
            u.set(false);
        });
    }
    rows.parallel.connect_value_notify(|r| {
        prefs::set_parallel(r.value() as usize);
    });
    {
        let (rows2, u) = (rows.clone(), updating.clone());
        rows.reset.connect_activated(move |_| {
            let t = target_of(rows2.format.selected());
            let mut p = prefs::load();
            p.formats.remove(t.id);
            prefs::save(&p);
            u.set(true);
            show(&rows2);
            u.set(false);
        });
    }
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    OPEN.with(|o| *o.borrow_mut() = Some(rows.clone()));
    dialog.present(Some(parent));
}

fn target_of_id(id: &str) -> &'static export::Target {
    export::target(id).unwrap_or(&export::TARGETS[0])
}

/// Dev hook `convert:open`, `convert:format=epub|kepub|mobi|azw3|fb2`, `convert:smarten|straighten|
/// unused-css|blank|margins|split|inline-toc=on|off`, `convert:version=auto|2|3`, `convert:parallel=N`,
/// `convert:reset`, `convert:dump`, `convert:close`.
pub fn dev(arg: &str, parent: &impl IsA<gtk::Widget>) {
    if arg == "open" {
        present(parent);
        return;
    }
    let Some(r) = OPEN.with(|o| o.borrow().clone()) else {
        eprintln!("convert_options: not open");
        return;
    };
    let (k, v) = arg.split_once('=').unwrap_or((arg, ""));
    let on = v == "on";
    match k {
        "format" => {
            if let Some(i) = FORMATS.iter().position(|f| *f == v) {
                r.format.set_selected(i as u32);
            }
        }
        "version" => r.version.set_selected(match v {
            "2" => 1,
            "3" => 2,
            _ => 0,
        }),
        "smarten" => r.smarten.set_active(on),
        "straighten" => r.straighten.set_active(on),
        "unused-css" => r.unused_css.set_active(on),
        "blank" => r.blank.set_active(on),
        "margins" => r.margins.set_active(on),
        "split" => r.split.set_active(on),
        "inline-toc" => r.toc.set_active(on),
        "parallel" => r.parallel.set_value(v.parse().unwrap_or(1.0)),
        "reset" => r.reset.emit_by_name::<()>("activated", &[]),
        "close" => {
            r.dialog.close();
            return;
        }
        _ => {}
    }
    let t = target_of(r.format.selected());
    let saved = prefs::user_options(t.id).is_some();
    eprintln!(
        "convert_options: format={} version={} smarten={} straighten={} unused_css={} blank={} margins={} split={} inline_toc={} parallel={} saved={} version_visible={} toc_visible={}",
        t.id,
        r.version.selected(),
        r.smarten.is_active(),
        r.straighten.is_active(),
        r.unused_css.is_active(),
        r.blank.is_active(),
        r.margins.is_active(),
        r.split.is_active(),
        r.toc.is_active(),
        prefs::parallel_now(),
        saved,
        r.version.is_visible(),
        r.toc.is_visible()
    );
}
