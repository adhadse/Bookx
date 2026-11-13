//! The reader's colour theme editor (R47): opened from the pencil / plus swatches in the reader menu.
//! The data and the rules are `crate::reader_themes`; the per-book look is `crate::reading_profiles`.
use crate::i18n::{tr, trf};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, gio, glib};

use super::SharedRef;
use crate::config::{parse_hex, ReaderPrefs};

/// What the windows need from the reader.
#[derive(Clone)]
pub struct Ctx {
    pub shared: SharedRef,
    pub book_id: u64,
    /// Shows these reader settings now (and keeps them, like the text menu does).
    pub apply: Rc<dyn Fn(ReaderPrefs)>,
    pub toast: Rc<dyn Fn(String)>,
    /// The list of custom themes changed (the swatches in the menu are rebuilt).
    pub changed: Rc<dyn Fn()>,
}

fn current_prefs(ctx: &Ctx) -> ReaderPrefs {
    ctx.shared.cfg.borrow().reader.clone()
}

/// Puts these settings in place right away (so that what is read next is current) and has the
/// reader show them.
fn put(ctx: &Ctx, p: ReaderPrefs) {
    ctx.shared.cfg.borrow_mut().reader = p.clone();
    (ctx.apply)(p);
}

// ---- themes: changes ----

pub fn save_theme(ctx: &Ctx, name: &str, fg: &str, bg: &str) -> Result<String, String> {
    let n = ctx
        .shared
        .cfg
        .borrow_mut()
        .reader_themes
        .save(name, fg, bg)?;
    ctx.shared.cfg.borrow().save();
    (ctx.changed)();
    Ok(n)
}

pub fn apply_theme(ctx: &Ctx, name: &str) -> bool {
    let t = ctx.shared.cfg.borrow().reader_themes.get(name).cloned();
    match t {
        Some(t) => {
            put(ctx, t.apply(&current_prefs(ctx)));
            true
        }
        None => false,
    }
}

pub fn delete_theme(ctx: &Ctx, name: &str) -> bool {
    let ok = ctx.shared.cfg.borrow_mut().reader_themes.remove(name);
    ctx.shared.cfg.borrow().save();
    (ctx.changed)();
    ok
}

fn swatch(fg: &str, bg: &str) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(56);
    a.set_content_height(32);
    a.set_valign(gtk::Align::Center);
    let (fg, bg) = (
        parse_hex(fg).unwrap_or((0.0, 0.0, 0.0)),
        parse_hex(bg).unwrap_or((1.0, 1.0, 1.0)),
    );
    a.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(bg.0, bg.1, bg.2);
        let _ = cr.paint();
        cr.set_source_rgb(fg.0, fg.1, fg.2);
        cr.select_font_face(
            "Serif",
            gtk::cairo::FontSlant::Normal,
            gtk::cairo::FontWeight::Normal,
        );
        cr.set_font_size(h as f64 * 0.6);
        cr.move_to(w as f64 * 0.2, h as f64 * 0.7);
        let _ = cr.show_text("Aa");
    });
    a.add_css_class("card");
    a
}

fn rgba(hex: &str) -> gdk::RGBA {
    let c = parse_hex(hex).unwrap_or((0.0, 0.0, 0.0));
    gdk::RGBA::new(c.0 as f32, c.1 as f32, c.2 as f32, 1.0)
}

fn hex_of(c: &gdk::RGBA) -> String {
    crate::config::to_hex((c.red() as f64, c.green() as f64, c.blue() as f64))
}

// ---- themes: window ----

pub fn show_themes(parent: &gtk::Widget, ctx: &Ctx, edit_theme: Option<&str>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Reading Themes"));
    dialog.set_content_width(480);
    dialog.set_content_height(620);
    let page = adw::PreferencesPage::new();
    let list_group = adw::PreferencesGroup::builder()
        .title(tr("Your themes"))
        .description(tr("Colour themes for the page, shared by all your books."))
        .build();
    page.add(&list_group);

    let edit = adw::PreferencesGroup::builder()
        .title(tr("Make a theme"))
        .build();
    let name = adw::EntryRow::builder().title(tr("Theme name")).build();
    let (start_fg, start_bg) = {
        let c = ctx.shared.cfg.borrow();
        (c.reader.custom_fg.clone(), c.reader.custom_bg.clone())
    };
    let mk_picker = |title: &str, hex: &str| {
        let d = gtk::ColorDialog::new();
        d.set_with_alpha(false);
        d.set_title(title);
        let b = gtk::ColorDialogButton::new(Some(d));
        b.set_rgba(&rgba(hex));
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(title));
        super::audit::label(&b, title);
        b
    };
    let fg_btn = mk_picker("Text colour", &start_fg);
    let bg_btn = mk_picker("Page colour", &start_bg);
    let existing = edit_theme.and_then(|n| ctx.shared.cfg.borrow().reader_themes.get(n).cloned());
    if let Some(t) = existing {
        name.set_text(&t.name);
        fg_btn.set_rgba(&rgba(&t.fg));
        bg_btn.set_rgba(&rgba(&t.bg));
    }
    let fg_row = adw::ActionRow::builder().title(tr("Text colour")).build();
    fg_row.add_suffix(&fg_btn);
    let bg_row = adw::ActionRow::builder().title(tr("Page colour")).build();
    bg_row.add_suffix(&bg_btn);
    let preview = gtk::Label::new(Some(&tr("The quick brown fox jumps over the lazy dog.")));
    preview.set_wrap(true);
    preview.set_margin_top(10);
    preview.set_margin_bottom(10);
    let prev_row = adw::ActionRow::builder().title(tr("Preview")).build();
    prev_row.set_child(Some(&preview));
    let warn = gtk::Label::new(None);
    warn.set_xalign(0.0);
    warn.set_wrap(true);
    warn.add_css_class("caption");
    let save = gtk::Button::with_label(&tr("Save Theme"));
    save.add_css_class("suggested-action");
    save.set_halign(gtk::Align::End);
    edit.add(&name);
    edit.add(&fg_row);
    edit.add(&bg_row);
    edit.add(&prev_row);
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 6);
    box_.append(&warn);
    box_.append(&save);
    edit.add(&box_);
    page.add(&edit);

    let files = adw::PreferencesGroup::new();
    let io = adw::ActionRow::builder().title(tr("Share themes")).build();
    let export = gtk::Button::with_label(&tr("Export"));
    let import = gtk::Button::with_label(&tr("Import"));
    for b in [&export, &import] {
        b.set_valign(gtk::Align::Center);
        io.add_suffix(b);
    }
    files.add(&io);
    page.add(&files);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    // the preview follows the pickers
    let style = gtk::CssProvider::new();
    #[allow(deprecated)]
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &style,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    preview.add_css_class("theme-preview");
    let update = {
        let (fg_btn, bg_btn, warn, style) =
            (fg_btn.clone(), bg_btn.clone(), warn.clone(), style.clone());
        move || {
            let (fg, bg) = (hex_of(&fg_btn.rgba()), hex_of(&bg_btn.rgba()));
            style.load_from_string(&format!(
                ".theme-preview {{ color: {fg}; background: {bg}; padding: 8px; border-radius: 6px; font-family: serif; font-size: 120%; }}"
            ));
            match crate::reader_themes::warning(&fg, &bg) {
                Some(w) => {
                    warn.set_text(&w);
                    warn.add_css_class("warning");
                }
                None => {
                    warn.set_text("");
                }
            }
        }
    };
    update();
    {
        let u = update.clone();
        fg_btn.connect_rgba_notify(move |_| u());
        let u = update.clone();
        bg_btn.connect_rgba_notify(move |_| u());
    }

    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Default::default();
    let rebuild: Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>> = Default::default();
    {
        let (ctx, group, rows, rb) = (
            ctx.clone(),
            list_group.clone(),
            rows.clone(),
            rebuild.clone(),
        );
        let (name, fg_btn, bg_btn) = (name.clone(), fg_btn.clone(), bg_btn.clone());
        let f: Rc<dyn Fn()> = Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let (themes, prefs) = {
                let c = ctx.shared.cfg.borrow();
                (c.reader_themes.list.clone(), c.reader.clone())
            };
            if themes.is_empty() {
                let r = adw::ActionRow::builder()
                    .title(tr("No themes yet"))
                    .subtitle(tr("Pick two colours below and save them"))
                    .build();
                group.add(&r);
                rows.borrow_mut().push(r);
            }
            for t in themes {
                let r = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&t.name))
                    .subtitle(trf(
                        "{} on {} · {}",
                        &[&t.fg, &t.bg, &(if t.is_dark() { "dark" } else { "light" })],
                    ))
                    .activatable(true)
                    .build();
                r.add_prefix(&swatch(&t.fg, &t.bg));
                let in_use = prefs.colors == crate::config::ColorMode::Custom
                    && prefs.custom_fg == t.fg
                    && prefs.custom_bg == t.bg;
                if in_use {
                    r.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
                }
                let edit_btn = gtk::Button::from_icon_name("document-edit-symbolic");
                edit_btn.add_css_class("flat");
                edit_btn.set_valign(gtk::Align::Center);
                edit_btn.set_tooltip_text(Some(&tr("Edit the theme")));
                super::audit::label(&edit_btn, &trf("Edit theme {}", &[&t.name]));
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk::Align::Center);
                del.set_tooltip_text(Some(&tr("Delete the theme")));
                super::audit::label(&del, &trf("Delete theme {}", &[&t.name]));
                r.add_suffix(&edit_btn);
                r.add_suffix(&del);
                {
                    let (ctx, n, rb) = (ctx.clone(), t.name.clone(), rb.clone());
                    r.connect_activated(move |_| {
                        apply_theme(&ctx, &n);
                        if let Some(f) = rb.borrow().as_ref() {
                            f();
                        }
                    });
                }
                {
                    let (t, name, fg_btn, bg_btn) =
                        (t.clone(), name.clone(), fg_btn.clone(), bg_btn.clone());
                    edit_btn.connect_clicked(move |_| {
                        name.set_text(&t.name);
                        fg_btn.set_rgba(&rgba(&t.fg));
                        bg_btn.set_rgba(&rgba(&t.bg));
                    });
                }
                {
                    let (ctx, n, rb) = (ctx.clone(), t.name.clone(), rb.clone());
                    del.connect_clicked(move |_| {
                        delete_theme(&ctx, &n);
                        if let Some(f) = rb.borrow().as_ref() {
                            f();
                        }
                    });
                }
                group.add(&r);
                rows.borrow_mut().push(r);
            }
        });
        *rebuild.borrow_mut() = Some(f);
    }
    let refresh = {
        let rb = rebuild.clone();
        move || {
            if let Some(f) = rb.borrow().as_ref() {
                f();
            }
        }
    };
    refresh();
    {
        let (ctx, name, fg_btn, bg_btn, refresh) = (
            ctx.clone(),
            name.clone(),
            fg_btn.clone(),
            bg_btn.clone(),
            refresh.clone(),
        );
        save.connect_clicked(move |_| {
            match save_theme(
                &ctx,
                name.text().as_str(),
                &hex_of(&fg_btn.rgba()),
                &hex_of(&bg_btn.rgba()),
            ) {
                Ok(n) => {
                    (ctx.toast)(format!("Saved the theme “{n}”"));
                    refresh();
                }
                Err(e) => (ctx.toast)(e),
            }
        });
    }
    {
        let (ctx, parent) = (ctx.clone(), parent.clone());
        export.connect_clicked(move |_| {
            let text = ctx.shared.cfg.borrow().reader_themes.to_json();
            save_text(&parent, "Export Themes", "bookx-themes.json", text);
        });
    }
    {
        let (ctx, parent, refresh) = (ctx.clone(), parent.clone(), refresh.clone());
        import.connect_clicked(move |_| {
            let (ctx, refresh) = (ctx.clone(), refresh.clone());
            open_text(&parent, "Import Themes", move |text| {
                let r = ctx.shared.cfg.borrow_mut().reader_themes.import(&text);
                ctx.shared.cfg.borrow().save();
                (ctx.toast)(match r {
                    Ok(n) => format!("Imported {n} theme(s)"),
                    Err(e) => format!("Not a themes file: {e}"),
                });
                (ctx.changed)();
                refresh();
            });
        });
    }
    dialog.present(Some(parent));
}

// ---- files ----

fn save_text(parent: &gtk::Widget, title: &str, name: &str, text: String) {
    let dlg = gtk::FileDialog::builder()
        .title(title)
        .initial_name(name)
        .build();
    let win = parent.root().and_downcast::<gtk::Window>();
    dlg.save(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            let _ = std::fs::write(path, text);
        }
    });
}

fn open_text(parent: &gtk::Widget, title: &str, done: impl Fn(String) + 'static) {
    let dlg = gtk::FileDialog::builder().title(title).build();
    let win = parent.root().and_downcast::<gtk::Window>();
    dlg.open(win.as_ref(), gio::Cancellable::NONE, move |res| {
        if let Some(text) = res
            .ok()
            .and_then(|f| f.path())
            .and_then(|p| std::fs::read_to_string(p).ok())
        {
            done(text);
        }
    });
}

// ---- test hooks ----

/// `reader:profile=…` (settings and themes); the text is printed as `reader_profile: …`.
pub fn dev(ctx: &Ctx, cmd: &str) -> String {
    let (name, arg) = cmd.split_once('=').unwrap_or((cmd, ""));
    let (a, b) = arg.split_once('|').unwrap_or((arg, ""));
    match name {
        "set" => {
            // set=field|value on the reader's settings, for building a profile
            let mut p = current_prefs(ctx);
            match a {
                "font_size" => p.font_size = b.parse().unwrap_or(p.font_size),
                "margin" => p.margin = b.parse().unwrap_or(p.margin),
                "line_spacing" => p.line_spacing = b.parse().unwrap_or(p.line_spacing),
                "font" => p.font_family = b.to_string(),
                _ => return format!("error unknown field {a}"),
            }
            put(ctx, p);
            format!("set {a}={b}")
        }
        "theme-save" => {
            let (fg, bg) = b.split_once('/').unwrap_or((b, ""));
            match save_theme(ctx, a, fg, bg) {
                Ok(n) => format!("theme saved {n}"),
                Err(e) => format!("error {e}"),
            }
        }
        "theme-apply" => format!("theme applied {}", apply_theme(ctx, a)),
        "theme-delete" => format!("theme deleted {}", delete_theme(ctx, a)),
        "theme-export" => {
            let text = ctx.shared.cfg.borrow().reader_themes.to_json();
            format!("theme exported {}", std::fs::write(a, text).is_ok())
        }
        "theme-import" => {
            let text = std::fs::read_to_string(a).unwrap_or_default();
            let r = ctx.shared.cfg.borrow_mut().reader_themes.import(&text);
            ctx.shared.cfg.borrow().save();
            (ctx.changed)();
            format!("theme imported {r:?}")
        }
        "dump" => {
            let c = ctx.shared.cfg.borrow();
            let themes: Vec<String> = c
                .reader_themes
                .list
                .iter()
                .map(|t| format!("{}({}/{})", t.name, t.fg, t.bg))
                .collect();
            format!(
                "own={} defaults={} themes={} now=size{:.0},margin{:.0},colors{:?},fg{},bg{}",
                c.book_looks.get(ctx.book_id).is_some(),
                c.reader_defaults
                    .as_ref()
                    .map(|d| format!("size{:.0},margin{:.0}", d.font_size, d.margin))
                    .unwrap_or_default(),
                themes.join(" ; "),
                c.reader.font_size,
                c.reader.margin,
                c.reader.colors,
                c.reader.custom_fg,
                c.reader.custom_bg
            )
        }
        _ => format!("error unknown {cmd}"),
    }
}
