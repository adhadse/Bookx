//! Preferences → Dictionaries: the order the installed dictionaries are asked in (Move Up and Move
//! Down), your own web look-up sites, and the history of looked-up words. The dictionary look-up
//! popover of the reader gets a *Web* menu with the same sites and a *History* entry.
//!
//! Logic and tests: `crate::lookup_prefs`. Web addresses are checked when they are saved and again
//! when they are opened (`core::util::safe_external_url`); the selected text is percent-encoded and
//! never chooses the site. The history is a list of words in a file next to the settings; nothing
//! leaves the computer, and it can be switched off and cleared (with Undo).

use crate::i18n::{ntr, tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use super::preferences::{PrefCtx, register_group};
use crate::lookup_prefs as lp;
use bookx_lookup::dictset::WebSource;

struct OrderRow {
    title: String,
    up: gtk::Button,
    down: gtk::Button,
}

struct WebRow {
    name: String,
    edit: Option<gtk::Button>,
    remove: Option<gtk::Button>,
}

#[derive(Default)]
struct Ui {
    shared: Option<SharedRef>,
    order_group: Option<adw::PreferencesGroup>,
    order_rows: Vec<adw::ActionRow>,
    order: Vec<OrderRow>,
    web_group: Option<adw::PreferencesGroup>,
    web_rows: Vec<adw::ActionRow>,
    web: Vec<WebRow>,
    web_add: Option<gtk::Button>,
    history_row: Option<adw::ActionRow>,
    history_show: Option<gtk::Button>,
}

struct WebForm {
    dialog: adw::Dialog,
    name: adw::EntryRow,
    address: adw::EntryRow,
    save: gtk::Button,
    banner: adw::Banner,
}

struct HistoryWin {
    dialog: adw::Dialog,
    toasts: adw::ToastOverlay,
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    clear: gtk::Button,
    removes: RefCell<Vec<(String, gtk::Button)>>,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static SHARED: RefCell<Option<SharedRef>> = const { RefCell::new(None) };
    static TERM: RefCell<String> = const { RefCell::new(String::new()) };
    static FORM: RefCell<Option<Rc<WebForm>>> = const { RefCell::new(None) };
    static WEB_UNDO: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static HISTORY: RefCell<Option<Rc<HistoryWin>>> = const { RefCell::new(None) };
    static HISTORY_UNDO: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static OPENED: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn register() {
    register_group("dictionaries", 56, order_group);
    register_group("dictionaries", 57, web_group);
    register_group("dictionaries", 58, history_group);
}

fn shared() -> Option<SharedRef> {
    SHARED.with(|s| s.borrow().clone())
}

fn root() -> std::path::PathBuf {
    crate::dictionaries::dictionary_dir()
}

fn flat(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tip));
    super::audit::label(&b, tip);
    b
}

// ---- order ------------------------------------------------------------------------------------

fn order_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    SHARED.with(|s| *s.borrow_mut() = Some(ctx.shared.clone()));
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Dictionary order"));
    g.set_description(Some(&tr(
        "Dictionaries are asked from the top, so the first one that knows a word answers first.",
    )));
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.shared = Some(ctx.shared.clone());
        u.order_group = Some(g.clone());
        // the rows of an earlier build belong to another group
        u.order_rows.clear();
    });
    fill_order();
    g
}

fn fill_order() {
    let (group, old) = UI.with(|u| {
        let mut u = u.borrow_mut();
        (u.order_group.clone(), std::mem::take(&mut u.order_rows))
    });
    let Some(g) = group else { return };
    for r in old {
        g.remove(&r);
    }
    let entries = lp::entries(&root());
    let n = entries.len();
    let mut rows = Vec::new();
    let mut order = Vec::new();
    if entries.is_empty() {
        let r = super::preferences::info_row(
            &tr("No dictionaries yet"),
            &tr("Dictionaries you install or copy to the dictionary folder are listed here."),
        );
        g.add(&r);
        rows.push(r);
    }
    for (i, e) in entries.into_iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&e.title));
        row.set_subtitle(&glib::markup_escape_text(&trf(
            "Number {} of {}",
            &[&(i + 1), &n],
        )));
        let up = flat("go-up-symbolic", &trf("Move {} up", &[&e.title]));
        let down = flat("go-down-symbolic", &trf("Move {} down", &[&e.title]));
        up.set_sensitive(i > 0);
        down.set_sensitive(i + 1 < n);
        for (b, delta) in [(&up, -1), (&down, 1)] {
            let id = e.id.clone();
            b.connect_clicked(move |_| {
                match lp::move_dictionary(&root(), &id, delta) {
                    Ok(_) => LAST.with(|l| l.borrow_mut().clear()),
                    Err(e) => {
                        super::preferences::toast(&e);
                        LAST.with(|l| *l.borrow_mut() = e);
                    }
                }
                fill_order();
            });
        }
        row.add_suffix(&up);
        row.add_suffix(&down);
        g.add(&row);
        rows.push(row);
        order.push(OrderRow {
            title: e.title,
            up,
            down,
        });
    }
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.order_rows = rows;
        u.order = order;
    });
}

// ---- web sites --------------------------------------------------------------------------------

fn web_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    SHARED.with(|s| *s.borrow_mut() = Some(ctx.shared.clone()));
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Web look-up sites"));
    g.set_description(Some(&tr(
        "Offered next to a look-up in the reader. In the address, {word} stands for the selected text.",
    )));
    let add = gtk::Button::with_label(&tr("Add Site"));
    add.set_valign(gtk::Align::Center);
    g.set_header_suffix(Some(&add));
    {
        let parent = ctx.parent.clone();
        add.connect_clicked(move |_| web_form(&parent, None));
    }
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.web_group = Some(g.clone());
        u.web_add = Some(add);
        u.web_rows.clear();
    });
    fill_web(&ctx.parent);
    g
}

fn fill_web(parent: &gtk::Widget) {
    let (group, old) = UI.with(|u| {
        let mut u = u.borrow_mut();
        (u.web_group.clone(), std::mem::take(&mut u.web_rows))
    });
    let (Some(g), Some(shared)) = (group, shared()) else {
        return;
    };
    for r in old {
        g.remove(&r);
    }
    let own = shared.cfg.borrow().lookup.web_sources.clone();
    let mut rows = Vec::new();
    let mut web = Vec::new();
    for s in WebSource::defaults() {
        let row = super::preferences::info_row(&s.name, &s.template);
        g.add(&row);
        rows.push(row);
        web.push(WebRow {
            name: s.name,
            edit: None,
            remove: None,
        });
    }
    for (i, s) in own.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&s.name));
        row.set_subtitle(&glib::markup_escape_text(&s.template));
        row.set_subtitle_lines(0);
        let edit = flat("document-edit-symbolic", &trf("Edit {}", &[&s.name]));
        let rm = flat("user-trash-symbolic", &trf("Remove {}", &[&s.name]));
        row.add_suffix(&edit);
        row.add_suffix(&rm);
        {
            let (parent, s) = (parent.clone(), s.clone());
            edit.connect_clicked(move |_| web_form(&parent, Some((i, s.clone()))));
        }
        {
            let (parent, shared) = (parent.clone(), shared.clone());
            rm.connect_clicked(move |_| remove_web(&parent, &shared, i));
        }
        g.add(&row);
        rows.push(row);
        web.push(WebRow {
            name: s.name.clone(),
            edit: Some(edit),
            remove: Some(rm),
        });
    }
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.web_rows = rows;
        u.web = web;
    });
}

fn remove_web(parent: &gtk::Widget, shared: &SharedRef, i: usize) {
    let removed = {
        let mut c = shared.cfg.borrow_mut();
        (i < c.lookup.web_sources.len()).then(|| c.lookup.web_sources.remove(i))
    };
    let Some(src) = removed else { return };
    shared.cfg.borrow().save();
    fill_web(parent);
    let stash = Rc::new(RefCell::new(Some(src.clone())));
    let undo: Rc<dyn Fn()> = {
        let (shared, parent, stash) = (shared.clone(), parent.clone(), stash.clone());
        Rc::new(move || {
            if let Some(s) = stash.borrow_mut().take() {
                {
                    let mut c = shared.cfg.borrow_mut();
                    let at = i.min(c.lookup.web_sources.len());
                    c.lookup.web_sources.insert(at, s);
                }
                shared.cfg.borrow().save();
                fill_web(&parent);
            }
        })
    };
    WEB_UNDO.with(|u| *u.borrow_mut() = Some(undo.clone()));
    super::preferences::toast_undo(
        &trf("Site “{}” removed", &[&src.name]),
        10,
        move || undo(),
        || {},
    );
}

fn web_form(parent: &gtk::Widget, existing: Option<(usize, WebSource)>) {
    let editing = existing.as_ref().map(|(i, _)| *i);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&if editing.is_some() {
        tr("Edit Web Site")
    } else {
        tr("Add Web Site")
    });
    dialog.set_content_width(480);
    dialog.set_content_height(380);
    let g = adw::PreferencesGroup::new();
    g.set_description(Some(&tr(
        "Put {word} where the selected text goes, for example https://dictionary.example.com/search?q={word}. Only http and https addresses are accepted.",
    )));
    let name = adw::EntryRow::new();
    name.set_title(&tr("Name"));
    name.set_max_length(lp::MAX_NAME as i32);
    let address = adw::EntryRow::new();
    address.set_title(&tr("Address with {word}"));
    address.set_input_purpose(gtk::InputPurpose::Url);
    if let Some((_, s)) = &existing {
        name.set_text(&s.name);
        address.set_text(&s.template);
    }
    g.add(&name);
    g.add(&address);
    let page = adw::PreferencesPage::new();
    page.add(&g);
    let save = gtk::Button::with_label(&if editing.is_some() {
        tr("Save")
    } else {
        tr("Add")
    });
    save.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    let banner = adw::Banner::new("");
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.add_top_bar(&banner);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&save));
    let f = Rc::new(WebForm {
        dialog: dialog.clone(),
        name,
        address,
        save: save.clone(),
        banner,
    });
    FORM.with(|c| *c.borrow_mut() = Some(f.clone()));
    {
        let (f, parent) = (f.clone(), parent.clone());
        save.connect_clicked(move |_| {
            let Some(shared) = shared() else { return };
            let list = shared.cfg.borrow().lookup.web_sources.clone();
            match lp::make_source(
                f.name.text().as_str(),
                f.address.text().as_str(),
                &list,
                editing,
            ) {
                Err(e) => {
                    f.banner.set_title(&e);
                    f.banner.set_revealed(true);
                    LAST.with(|l| *l.borrow_mut() = e);
                }
                Ok(src) => {
                    {
                        let mut c = shared.cfg.borrow_mut();
                        match editing {
                            Some(i) if i < c.lookup.web_sources.len() => {
                                c.lookup.web_sources[i] = src
                            }
                            _ => c.lookup.web_sources.push(src),
                        }
                    }
                    shared.cfg.borrow().save();
                    LAST.with(|l| l.borrow_mut().clear());
                    f.dialog.close();
                    fill_web(&parent);
                }
            }
        });
    }
    dialog.connect_closed(|_| FORM.with(|c| *c.borrow_mut() = None));
    dialog.present(Some(parent));
}

// ---- history ----------------------------------------------------------------------------------

fn history_group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    SHARED.with(|s| *s.borrow_mut() = Some(ctx.shared.clone()));
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Look-up history"));
    let keep = adw::SwitchRow::new();
    keep.set_title(&tr("Keep a history of look-ups"));
    keep.set_subtitle(&tr(
        "The words you look up, newest first. They stay on this computer.",
    ));
    keep.set_active(ctx.shared.cfg.borrow().lookup.keep_history);
    {
        let shared = ctx.shared.clone();
        keep.connect_active_notify(move |s| {
            shared.cfg.borrow_mut().lookup.keep_history = s.is_active();
            shared.cfg.borrow().save();
        });
    }
    let row = adw::ActionRow::new();
    row.set_title(&tr("History"));
    let show = gtk::Button::with_label(&tr("Show"));
    show.set_valign(gtk::Align::Center);
    show.set_tooltip_text(Some(&tr("Show the words you looked up")));
    super::audit::label(&show, &tr("Show the look-up history"));
    row.add_suffix(&show);
    row.set_activatable_widget(Some(&show));
    {
        let parent = ctx.parent.clone();
        show.connect_clicked(move |_| show_history(&parent));
    }
    g.add(&keep);
    g.add(&row);
    UI.with(|u| {
        let mut u = u.borrow_mut();
        u.history_row = Some(row);
        u.history_show = Some(show);
    });
    refresh_history_row();
    g
}

fn history() -> bookx_lookup::dictset::LookupHistory {
    lp::history_path()
        .map(|p| lp::load_history(&p))
        .unwrap_or_else(|| bookx_lookup::dictset::LookupHistory::new(lp::HISTORY_CAPACITY))
}

fn save_history(h: &bookx_lookup::dictset::LookupHistory) {
    if let Some(p) = lp::history_path() {
        let _ = lp::save_history(&p, h);
    }
}

fn refresh_history_row() {
    let n = history().len();
    UI.with(|u| {
        if let Some(r) = &u.borrow().history_row {
            r.set_subtitle(&ntr("{} word", "{} words", n));
        }
    });
}

/// The window with the words, newest first.
pub fn show_history(parent: &gtk::Widget) {
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Words you looked up"));
    let clear = gtk::Button::with_label(&tr("Clear"));
    clear.add_css_class("destructive-action");
    clear.set_valign(gtk::Align::Center);
    clear.set_tooltip_text(Some(&tr("Remove every word from the history")));
    group.set_header_suffix(Some(&clear));
    page.add(&group);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Look-up history"));
    dialog.set_content_width(420);
    dialog.set_content_height(520);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let toasts = super::dialog_toast::overlay(&view);
    dialog.set_child(Some(&toasts));
    let w = Rc::new(HistoryWin {
        dialog: dialog.clone(),
        toasts,
        group,
        rows: RefCell::new(Vec::new()),
        clear: clear.clone(),
        removes: RefCell::new(Vec::new()),
    });
    fill_history(&w);
    {
        let w = w.clone();
        clear.connect_clicked(move |_| clear_history(&w));
    }
    HISTORY.with(|h| *h.borrow_mut() = Some(w));
    dialog.connect_closed(|_| HISTORY.with(|h| *h.borrow_mut() = None));
    dialog.present(Some(parent));
}

fn fill_history(w: &Rc<HistoryWin>) {
    for r in w.rows.borrow_mut().drain(..) {
        w.group.remove(&r);
    }
    w.removes.borrow_mut().clear();
    let words = history().entries();
    w.clear.set_sensitive(!words.is_empty());
    if words.is_empty() {
        let r = super::preferences::info_row(
            &tr("No words yet"),
            &tr("Words you look up in a book appear here."),
        );
        w.group.add(&r);
        w.rows.borrow_mut().push(r);
    }
    for word in words {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&word));
        let rm = flat(
            "edit-delete-symbolic",
            &trf("Remove {} from the history", &[&word]),
        );
        row.add_suffix(&rm);
        {
            let (w, word) = (w.clone(), word.clone());
            rm.connect_clicked(move |_| {
                let mut h = history();
                if h.remove(&word) {
                    save_history(&h);
                }
                fill_history(&w);
                refresh_history_row();
            });
        }
        w.group.add(&row);
        w.rows.borrow_mut().push(row);
        w.removes.borrow_mut().push((word, rm));
    }
}

fn clear_history(w: &Rc<HistoryWin>) {
    let before = history();
    if before.is_empty() {
        return;
    }
    let n = before.len();
    save_history(&bookx_lookup::dictset::LookupHistory::new(
        lp::HISTORY_CAPACITY,
    ));
    fill_history(w);
    refresh_history_row();
    let stash = Rc::new(RefCell::new(Some(before)));
    let undo: Rc<dyn Fn()> = {
        let (w, stash) = (w.clone(), stash.clone());
        Rc::new(move || {
            if let Some(h) = stash.borrow_mut().take() {
                save_history(&h);
                fill_history(&w);
                refresh_history_row();
            }
        })
    };
    HISTORY_UNDO.with(|u| *u.borrow_mut() = Some(undo.clone()));
    super::dialog_toast::show_undo(
        &w.toasts,
        &ntr("History cleared ({} word)", "History cleared ({} words)", n),
        move || undo(),
    );
}

// ---- the reader's look-up popover -------------------------------------------------------------

/// Called when the dictionary page shows `term`: remembers it for the *Web* menu and notes it in the
/// history (when that is on).
pub fn note_term(shared: &SharedRef, term: &str) {
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    TERM.with(|t| *t.borrow_mut() = term.to_string());
    let keep = shared.cfg.borrow().lookup.keep_history;
    if let Some(p) = lp::history_path() {
        lp::record(&p, keep, term);
    }
}

/// Opens the site for the current term. Returns the address (or why not). `launch` false only for
/// the tests.
fn open_source(src: &WebSource, launch: bool) -> Result<String, String> {
    let term = TERM.with(|t| t.borrow().clone());
    let url = lp::expand(src, &term)?;
    OPENED.with(|o| *o.borrow_mut() = url.clone());
    if launch && !super::open_external_link(&url) {
        return Err(tr("This address cannot be used."));
    }
    Ok(url)
}

/// The *Web* menu of the dictionary page: the sites and the history.
pub fn web_button() -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_label(&tr("Web"));
    b.add_css_class("flat");
    b.add_css_class("caption");
    b.set_tooltip_text(Some(&tr("Look this up on a web site, or see the history")));
    super::audit::label(&b, &tr("Look up on the web"));
    b.set_create_popup_func(|b| {
        let pop = gtk::Popover::new();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_margin_top(6);
        col.set_margin_bottom(6);
        col.set_margin_start(6);
        col.set_margin_end(6);
        let prefs = shared()
            .map(|s| s.cfg.borrow().lookup.clone())
            .unwrap_or_default();
        let have_term = !TERM.with(|t| t.borrow().trim().is_empty());
        for src in lp::all_sources(&prefs) {
            let item = gtk::Button::with_label(&src.name);
            item.add_css_class("flat");
            item.set_sensitive(have_term);
            let pop2 = pop.clone();
            item.connect_clicked(move |_| {
                pop2.popdown();
                let _ = open_source(&src, true);
            });
            col.append(&item);
        }
        let hist = gtk::Button::with_label(&tr("History"));
        hist.add_css_class("flat");
        let (pop2, anchor) = (pop.clone(), b.clone());
        hist.connect_clicked(move |_| {
            pop2.popdown();
            show_history(anchor.upcast_ref());
        });
        col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        col.append(&hist);
        pop.set_child(Some(&col));
        b.set_popover(Some(&pop));
    });
    b
}

// ---- developer hooks --------------------------------------------------------------------------

/// `lookup:settings=…` of the developer script (arguments separated by `;`; open Preferences →
/// Dictionaries first with `prefs:pick=Dictionaries`).
///
/// * `up;TITLE`, `down;TITLE` press Move Up / Move Down of a dictionary
/// * `web-new`, `web-fill;NAME;ADDRESS`, `web-save`, `web-edit;NAME`, `web-remove;NAME`, `web-undo`
/// * `term;WORD` is a look-up (history, Web menu); `web-open;NAME` prints the address the menu would open
/// * `history-show`, `history-clear`, `history-remove;WORD`, `history-undo`, `history-close`, `keep;on|off`
/// * `dump` prints `lookup_settings: order=a,b web=name,… history=w1,w2 keep=B error="…" opened=…`
pub fn dev(arg: &str) {
    let Some(arg) = arg.strip_prefix("settings=") else {
        return;
    };
    let parts: Vec<&str> = arg.split(';').collect();
    let p = |i: usize| parts.get(i).copied().unwrap_or("");
    match parts[0] {
        "up" | "down" => {
            let b = UI.with(|u| {
                u.borrow().order.iter().find(|r| r.title == p(1)).map(|r| {
                    if parts[0] == "up" {
                        r.up.clone()
                    } else {
                        r.down.clone()
                    }
                })
            });
            if let Some(b) = b {
                b.emit_clicked();
            }
        }
        "web-new" => {
            if let Some(b) = UI.with(|u| u.borrow().web_add.clone()) {
                b.emit_clicked();
            }
        }
        "web-fill" => {
            if let Some(f) = FORM.with(|c| c.borrow().clone()) {
                f.name.set_text(p(1));
                f.address.set_text(p(2));
            }
        }
        "web-save" => {
            if let Some(f) = FORM.with(|c| c.borrow().clone()) {
                f.save.emit_clicked();
            }
        }
        "web-edit" | "web-remove" => {
            let b = UI.with(|u| {
                u.borrow()
                    .web
                    .iter()
                    .find(|r| r.name == p(1))
                    .and_then(|r| {
                        if parts[0] == "web-edit" {
                            r.edit.clone()
                        } else {
                            r.remove.clone()
                        }
                    })
            });
            if let Some(b) = b {
                b.emit_clicked();
            }
        }
        "web-undo" => {
            if let Some(u) = WEB_UNDO.with(|u| u.borrow_mut().take()) {
                u();
            }
        }
        "term" => {
            if let Some(s) = shared() {
                note_term(&s, p(1));
                refresh_history_row();
            }
        }
        "web-open" => {
            let prefs = shared()
                .map(|s| s.cfg.borrow().lookup.clone())
                .unwrap_or_default();
            match lp::all_sources(&prefs).into_iter().find(|s| s.name == p(1)) {
                Some(src) => match open_source(&src, false) {
                    Ok(u) => eprintln!("lookup_web: name={} url={u}", p(1)),
                    Err(e) => eprintln!("lookup_web: name={} error={e:?}", p(1)),
                },
                None => eprintln!("lookup_web: name={} missing", p(1)),
            }
        }
        "history-show" => {
            if let Some(b) = UI.with(|u| u.borrow().history_show.clone()) {
                b.emit_clicked();
            }
        }
        "history-clear" => {
            if let Some(w) = HISTORY.with(|h| h.borrow().clone()) {
                w.clear.emit_clicked();
            }
        }
        "history-remove" => {
            if let Some(w) = HISTORY.with(|h| h.borrow().clone()) {
                let b = w
                    .removes
                    .borrow()
                    .iter()
                    .find(|(word, _)| word == p(1))
                    .map(|(_, b)| b.clone());
                if let Some(b) = b {
                    b.emit_clicked();
                }
            }
        }
        "history-undo" => {
            if let Some(u) = HISTORY_UNDO.with(|u| u.borrow_mut().take()) {
                u();
            }
        }
        "history-close" => {
            if let Some(w) = HISTORY.with(|h| h.borrow().clone()) {
                w.dialog.close();
            }
        }
        "keep" => {
            if let Some(s) = shared() {
                s.cfg.borrow_mut().lookup.keep_history = p(1) != "off";
                s.cfg.borrow().save();
            }
        }
        _ => {
            let own = shared()
                .map(|s| {
                    s.cfg
                        .borrow()
                        .lookup
                        .web_sources
                        .iter()
                        .map(|w| w.name.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            eprintln!(
                "lookup_settings: order={} web={} history={} keep={} window={} error={:?} opened={}",
                UI.with(|u| u
                    .borrow()
                    .order
                    .iter()
                    .map(|r| r.title.clone())
                    .collect::<Vec<_>>()
                    .join(",")),
                own.join(","),
                history().entries().join(","),
                shared().is_some_and(|s| s.cfg.borrow().lookup.keep_history),
                HISTORY.with(|h| h.borrow().is_some()),
                LAST.with(|l| l.borrow().clone()),
                OPENED.with(|o| o.borrow().clone()),
            );
        }
    }
}
