//! The list of your vocabulary: every word with its definition, the ones you defined yourself marked
//! "Mine", a search, a filter (all / mine / from a dictionary), adding, editing and removing, a pasted
//! glossary, CSV import, and export (CSV, JSON, a StarDict dictionary, Anki). With a book it is the
//! glossary of that book ("Terms in this book").

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bookx_lookup::own::{
    export_csv, export_stardict, import_glossary, parse_glossary, GlossaryItem,
};
use bookx_lookup::vocab::{export_anki_tsv, AnkiOptions, Scope, Source, VocabEntry};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::vocab_own::{self, Ctx};
use super::SharedRef;
use crate::i18n::{tr, trf};

/// Largest glossary file read.
const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024;

/// The list is the glossary of this book.
#[derive(Clone)]
pub struct BookView {
    pub id: u64,
    pub title: String,
    /// Language of the book, for new terms.
    pub language: String,
    /// Jump to a place (fraction of the book): the terms saved there get a button.
    pub goto: Option<Rc<dyn Fn(f64)>>,
}

pub struct List {
    pub root: gtk::Box,
    pub refresh: Rc<dyn Fn()>,
    search: gtk::SearchEntry,
    filter: adw::ToggleGroup,
    shared: SharedRef,
    book: Option<BookView>,
    status: gtk::Label,
    rows: gtk::ListBox,
}

thread_local! {
    static LAST: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
    static PASTE: RefCell<Option<(gtk::TextView, gtk::Button)>> = const { RefCell::new(None) };
}

fn source_of(name: &str) -> Source {
    match name {
        "mine" => Source::Mine,
        "dictionary" => Source::Dictionary,
        _ => Source::All,
    }
}

/// What to show for the filter, the search and the book.
fn visible_entries(
    shared: &SharedRef,
    source: Source,
    query: &str,
    book: Option<&BookView>,
) -> Vec<VocabEntry> {
    let hub = shared.vocab.borrow();
    let mut v: Vec<VocabEntry> = hub.store.list(source, query).into_iter().cloned().collect();
    if let Some(b) = book {
        let id = b.id.to_string();
        let series = shared
            .lib
            .borrow()
            .get(b.id)
            .map(|x| x.meta.series.clone())
            .unwrap_or_default();
        v.retain(|e| {
            (e.own.is_some() && e.scope.applies(&id, &series))
                || e.contexts.iter().any(|c| c.book_id.as_deref() == Some(&id))
        });
    }
    v
}

pub fn build(shared: &SharedRef, book: Option<BookView>, focus: Option<&str>) -> Rc<List> {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_margin_top(8);
    root.set_margin_bottom(12);
    root.set_margin_start(12);
    root.set_margin_end(12);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Search words, definitions, notes and tags")));
    super::audit::label(&search, &tr("Search the vocabulary"));
    root.append(&search);

    let filter = adw::ToggleGroup::new();
    filter.set_homogeneous(true);
    for (name, label) in [
        ("all", tr("All")),
        ("mine", tr("Mine")),
        ("dictionary", tr("From dictionary")),
    ] {
        let t = adw::Toggle::new();
        t.set_name(Some(name));
        t.set_label(Some(&label));
        filter.add(t);
    }
    filter.set_active_name(Some("all"));
    super::audit::label(&filter, &tr("Show"));
    root.append(&filter);

    let rows = gtk::ListBox::new();
    rows.add_css_class("boxed-list");
    rows.set_selection_mode(gtk::SelectionMode::None);
    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("accessories-dictionary-symbolic"));
    empty.set_title(&tr("No words here"));
    empty.set_description(Some(&tr(
        "Select a word or phrase while reading and choose Add to Vocabulary to write your own definition, or look a word up in a dictionary.",
    )));
    rows.set_placeholder(Some(&empty));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&rows));
    root.append(&scroll);

    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_wrap(true);
    root.append(&status);

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let add = gtk::Button::with_label(&tr("Add Term"));
    add.add_css_class("suggested-action");
    let more = gtk::MenuButton::new();
    more.set_label(&tr("More"));
    more.set_direction(gtk::ArrowType::Up);
    super::audit::label(&more, &tr("Paste, import and export"));
    bar.append(&add);
    bar.append(&more);
    root.append(&bar);

    let list = Rc::new_cyclic(|weak: &std::rc::Weak<List>| {
        let weak = weak.clone();
        let (rows2, status2, search2, filter2, shared2, book2) = (
            rows.clone(),
            status.clone(),
            search.clone(),
            filter.clone(),
            shared.clone(),
            book.clone(),
        );
        let refresh: Rc<dyn Fn()> = Rc::new(move || {
            let Some(me) = weak.upgrade() else { return };
            let source = source_of(&filter2.active_name().unwrap_or_default());
            let entries = visible_entries(&shared2, source, &search2.text(), book2.as_ref());
            while let Some(r) = rows2.first_child() {
                rows2.remove(&r);
            }
            for e in &entries {
                rows2.append(&row_for(&me, e));
            }
            let all = visible_entries(&shared2, Source::All, "", book2.as_ref());
            let mine = all.iter().filter(|e| e.own.is_some()).count();
            let summary = trf(
                "{} words · {} with your own definition",
                &[&all.len(), &mine],
            );
            // a message about the last action stays until the next refresh of the same text
            if status2.label().is_empty() || status2.label().contains(" · ") {
                status2.set_label(&summary);
            }
        });
        List {
            root: root.clone(),
            refresh,
            search: search.clone(),
            filter: filter.clone(),
            shared: shared.clone(),
            book: book.clone(),
            status: status.clone(),
            rows: rows.clone(),
        }
    });

    {
        let l = list.clone();
        search.connect_search_changed(move |_| (l.refresh)());
        let l = list.clone();
        filter.connect_active_name_notify(move |_| (l.refresh)());
        let l = list.clone();
        add.connect_clicked(move |_| edit_term(&l, ""));
    }
    more.set_popover(Some(&more_menu(&list)));
    if let Some(f) = focus {
        list.search.set_text(f);
    }
    (list.refresh)();
    LAST.with(|l| *l.borrow_mut() = Some(list.clone()));
    list
}

fn language_of(l: &List) -> String {
    l.book
        .as_ref()
        .map(|b| b.language.clone())
        .unwrap_or_default()
}

/// The vocabulary changed through this list: the open readers mark the terms again.
fn changed(l: &List) {
    (l.refresh)();
    super::vocab_settings::changed();
}

fn row_for(l: &Rc<List>, e: &VocabEntry) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(&e.word));
    row.set_title_lines(1);
    let def = e.shown_definition().unwrap_or("");
    row.set_subtitle(&glib::markup_escape_text(def));
    row.set_subtitle_lines(3);
    row.set_activatable(true);
    if e.own.is_some() {
        let badge = gtk::Label::new(Some(&tr("Mine")));
        badge.add_css_class("own-badge");
        badge.set_valign(gtk::Align::Center);
        row.add_suffix(&badge);
    }
    // where it was met in this book
    if let Some((b, goto)) = l.book.as_ref().and_then(|b| b.goto.clone().map(|g| (b, g))) {
        let id = b.id.to_string();
        if let Some(p) = e
            .contexts
            .iter()
            .find(|c| c.book_id.as_deref() == Some(&id))
            .and_then(|c| c.position)
        {
            let go = gtk::Button::from_icon_name("go-jump-symbolic");
            go.add_css_class("flat");
            go.set_valign(gtk::Align::Center);
            let tip = trf("Go to where you met “{}”", &[&e.word]);
            go.set_tooltip_text(Some(&tip));
            super::audit::label(&go, &tip);
            go.connect_clicked(move |_| goto(p));
            row.add_suffix(&go);
        }
    }
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class("flat");
    edit.set_valign(gtk::Align::Center);
    let tip = if e.own.is_some() {
        trf("Edit my definition of {}", &[&e.word])
    } else {
        trf("Add my own definition of {}", &[&e.word])
    };
    edit.set_tooltip_text(Some(&tip));
    super::audit::label(&edit, &tip);
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_valign(gtk::Align::Center);
    let tip = trf("Remove {} from the vocabulary", &[&e.word]);
    del.set_tooltip_text(Some(&tip));
    super::audit::label(&del, &tip);
    row.add_suffix(&edit);
    row.add_suffix(&del);
    let word = e.word.clone();
    {
        let (l1, w1) = (l.clone(), word.clone());
        edit.connect_clicked(move |_| edit_term(&l1, &w1));
        let (l2, w2) = (l.clone(), word.clone());
        row.connect_activated(move |_| edit_term(&l2, &w2));
    }
    {
        let (l, w) = (l.clone(), word);
        del.connect_clicked(move |_| {
            l.shared.vocab.borrow_mut().remove_word(&w);
            changed(&l);
        });
    }
    row
}

fn edit_term(l: &Rc<List>, word: &str) {
    let ctx = Ctx::new(
        &l.shared,
        l.book.as_ref().map(|b| b.id),
        word,
        "",
        None,
        &language_of(l),
    );
    let l2 = l.clone();
    vocab_own::open_dialog(&l.root, ctx, move |_| changed(&l2));
}

fn say(l: &List, text: String) {
    l.status.set_label(&text);
}

fn more_menu(l: &Rc<List>) -> gtk::Popover {
    let pop = gtk::Popover::new();
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let item = |label: &str, f: Box<dyn Fn()>| {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        if let Some(c) = b.child().and_downcast::<gtk::Label>() {
            c.set_xalign(0.0);
        }
        let pop = pop.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            f();
        });
        bx.append(&b);
    };
    let l1 = l.clone();
    item(&tr("Paste a Glossary"), Box::new(move || paste_dialog(&l1)));
    let l1 = l.clone();
    item(&tr("Import a CSV File"), Box::new(move || import_file(&l1)));
    bx.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    for (label, kind, name) in export_kinds() {
        let l1 = l.clone();
        item(
            &label,
            Box::new(move || {
                let l2 = l1.clone();
                export_choose(
                    &l1.shared,
                    l1.root.upcast_ref(),
                    kind,
                    name,
                    Rc::new(move |m| say(&l2, m)),
                );
            }),
        );
    }
    pop.set_child(Some(&bx));
    pop
}

// ---- glossary: paste and import --------------------------------------------------------------------------

/// Adds parsed items; the message to show. In a book list new terms apply to that book.
fn add_items(l: &List, items: &[GlossaryItem], all_books: bool) -> String {
    let scope = match (&l.book, all_books) {
        (Some(b), false) => Scope::Book(b.id.to_string()),
        _ => Scope::All,
    };
    let now = l.shared.now();
    let (added, updated) = {
        let mut hub = l.shared.vocab.borrow_mut();
        let r = import_glossary(&mut hub.store, items, &scope, &language_of(l), now);
        hub.save();
        r
    };
    changed(l);
    trf("{} terms added, {} updated.", &[&added, &updated])
}

fn paste_dialog(l: &Rc<List>) {
    let d = adw::Dialog::new();
    d.set_title(&tr("Paste a Glossary"));
    d.set_content_width(460);
    d.set_content_height(460);
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 8);
    bx.set_margin_top(12);
    bx.set_margin_bottom(12);
    bx.set_margin_start(12);
    bx.set_margin_end(12);
    let help = gtk::Label::new(Some(&tr(
        "One term per line: term; definition (or a tab, or “term — definition”).",
    )));
    help.set_wrap(true);
    help.set_xalign(0.0);
    help.add_css_class("dim-label");
    let tv = gtk::TextView::new();
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    tv.set_left_margin(8);
    tv.set_right_margin(8);
    tv.set_top_margin(8);
    tv.set_bottom_margin(8);
    super::audit::label(&tv, &tr("Glossary"));
    let sc = gtk::ScrolledWindow::new();
    sc.add_css_class("card");
    sc.set_vexpand(true);
    sc.set_child(Some(&tv));
    let count = gtk::Label::new(Some(&tr("No terms found yet.")));
    count.set_xalign(0.0);
    let all = gtk::CheckButton::with_label(&tr("Apply to all books"));
    all.set_visible(l.book.is_some());
    all.set_active(l.book.is_none());
    bx.append(&help);
    bx.append(&sc);
    bx.append(&count);
    bx.append(&all);
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let add = gtk::Button::with_label(&tr("Add"));
    add.add_css_class("suggested-action");
    add.set_sensitive(false);
    header.pack_start(&cancel);
    header.pack_end(&add);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&bx));
    d.set_child(Some(&view));
    {
        let d = d.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    let items: Rc<RefCell<Vec<GlossaryItem>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let (items, count, add) = (items.clone(), count.clone(), add.clone());
        tv.buffer().connect_changed(move |b| {
            let text = b.text(&b.start_iter(), &b.end_iter(), false);
            let parsed = parse_glossary(&text);
            count.set_label(&if parsed.is_empty() {
                tr("No terms found yet.")
            } else {
                trf("{} terms found.", &[&parsed.len()])
            });
            add.set_sensitive(!parsed.is_empty());
            *items.borrow_mut() = parsed;
        });
    }
    {
        let (l, items, all, d) = (l.clone(), items.clone(), all.clone(), d.clone());
        add.connect_clicked(move |_| {
            let msg = add_items(&l, &items.borrow(), all.is_active());
            say(&l, msg);
            d.close();
        });
    }
    PASTE.with(|p| *p.borrow_mut() = Some((tv.clone(), add.clone())));
    d.connect_closed(|_| PASTE.with(|p| *p.borrow_mut() = None));
    d.present(Some(&l.root));
    tv.grab_focus();
}

fn read_limited(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_IMPORT_BYTES {
        return Err(tr("That file is too big for a glossary."));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn import_path(l: &List, path: &Path) -> String {
    match read_limited(path) {
        Ok(text) => {
            let items = parse_glossary(&text);
            if items.is_empty() {
                tr("No terms with a definition found in that file.")
            } else {
                add_items(l, &items, false)
            }
        }
        Err(e) => trf("Could not read the file: {}", &[&e]),
    }
}

fn import_file(l: &Rc<List>) {
    let l = l.clone();
    let parent = super::window_of(&l.root);
    gtk::FileDialog::builder()
        .title(tr("Import a CSV File"))
        .build()
        .open(parent.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                let m = import_path(&l, &p);
                say(&l, m);
            }
        });
}

// ---- export --------------------------------------------------------------------------------------------------

/// Writes an export; the message to show.
pub fn export_to(shared: &SharedRef, kind: &str, path: &Path) -> String {
    let hub = shared.vocab.borrow();
    let res: Result<(), String> = match kind {
        "csv" => std::fs::write(path, export_csv(&hub.store, true)).map_err(|e| e.to_string()),
        "json" => hub
            .store
            .only_own()
            .to_json()
            .map_err(|e| e.to_string())
            .and_then(|j| std::fs::write(path, j).map_err(|e| e.to_string())),
        "anki" => std::fs::write(path, export_anki_tsv(&hub.store, &AnkiOptions::default()))
            .map_err(|e| e.to_string()),
        "stardict" => export_stardict(&hub.store, path, &tr("My Vocabulary"))
            .map(|_| ())
            .map_err(|e| format!("{e:#}")),
        _ => Err("unknown export".into()),
    };
    match res {
        Ok(()) => trf("Saved to {}", &[&path.display().to_string()]),
        Err(e) => trf("Could not export: {}", &[&e]),
    }
}

/// Asks where to write an export of the vocabulary and writes it; `say` gets the message.
pub fn export_choose(
    shared: &SharedRef,
    parent: &gtk::Widget,
    kind: &'static str,
    name: &str,
    say: Rc<dyn Fn(String)>,
) {
    let shared = shared.clone();
    let window = super::window_of(parent);
    let dlg = gtk::FileDialog::builder().title(tr("Export")).build();
    if kind == "stardict" {
        dlg.select_folder(window.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                say(export_to(
                    &shared,
                    kind,
                    &p.join("my-vocabulary-dictionary"),
                ));
            }
        });
    } else {
        dlg.set_initial_name(Some(name));
        dlg.save(window.as_ref(), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                say(export_to(&shared, kind, &p));
            }
        });
    }
}

/// The export entries: label, kind, suggested file name.
pub fn export_kinds() -> Vec<(String, &'static str, &'static str)> {
    vec![
        (
            tr("Export My Definitions as CSV"),
            "csv",
            "my-vocabulary.csv",
        ),
        (
            tr("Export My Definitions as JSON"),
            "json",
            "my-vocabulary.json",
        ),
        (
            tr("Export Everything as Anki Cards"),
            "anki",
            "bookx-vocabulary.txt",
        ),
        (tr("Export as a StarDict Dictionary"), "stardict", ""),
    ]
}

/// "Terms in this book": the glossary of one book in a window of its own.
pub fn show_book_terms(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, book: BookView) {
    let d = adw::Dialog::new();
    d.set_title(&trf("Terms in {}", &[&book.title]));
    d.set_content_width(520);
    d.set_content_height(620);
    let list = build(shared, Some(book), None);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&list.root));
    d.set_child(Some(&view));
    d.present(Some(parent));
}

// ---- dev hooks -------------------------------------------------------------------------------------------

fn has_badge(w: &gtk::Widget) -> bool {
    if let Some(lb) = w.downcast_ref::<gtk::Label>() {
        if lb.has_css_class("own-badge") {
            return true;
        }
    }
    let mut c = w.first_child();
    while let Some(x) = c {
        if has_badge(&x) {
            return true;
        }
        c = x.next_sibling();
    }
    false
}

/// `vocab:list=...` in a dev script: drives the open list through its widgets and reports its rows.
pub fn dev(arg: &str) -> String {
    let Some(l) = LAST.with(|l| l.borrow().clone()) else {
        return "vocab_list: none".to_string();
    };
    if let Some(v) = arg.strip_prefix("filter=") {
        l.filter.set_active_name(Some(v));
    } else if let Some(v) = arg.strip_prefix("search=") {
        l.search.set_text(v);
        // the entry reports a change a moment later; the test does not wait for it
        (l.refresh)();
    } else if let Some(v) = arg.strip_prefix("add=") {
        // term|definition through the real dialog
        let (t, d) = v.split_once('|').unwrap_or((v, ""));
        edit_term(&l, "");
        vocab_own::dev(&format!("term={t}"));
        vocab_own::dev(&format!("def={d}"));
        vocab_own::dev("save");
    } else if arg == "paste-open" {
        paste_dialog(&l);
    } else if let Some(v) = arg.strip_prefix("paste-text=") {
        PASTE.with(|p| {
            if let Some((tv, _)) = p.borrow().as_ref() {
                tv.buffer().set_text(&v.replace("\\n", "\n"));
            }
        });
    } else if arg == "paste-add" {
        let b = PASTE.with(|p| p.borrow().as_ref().map(|(_, b)| b.clone()));
        if let Some(b) = b {
            b.emit_clicked();
        }
    } else if let Some(v) = arg.strip_prefix("import=") {
        let m = import_path(&l, &PathBuf::from(v));
        say(&l, m);
    } else if let Some(v) = arg.strip_prefix("export=") {
        if let Some((kind, path)) = v.split_once(':') {
            let m = export_to(&l.shared, kind, &PathBuf::from(path));
            say(&l, m);
        }
    } else if let Some(v) = arg.strip_prefix("edit=") {
        edit_term(&l, v);
    } else if let Some(v) = arg.strip_prefix("delete=") {
        l.shared.vocab.borrow_mut().remove_word(v);
        changed(&l);
    }
    let mut rows = Vec::new();
    let mut c = l.rows.first_child();
    while let Some(w) = c {
        if let Some(r) = w.downcast_ref::<adw::ActionRow>() {
            rows.push(format!(
                "{}{}={}",
                r.title(),
                if has_badge(&w) { "[mine]" } else { "" },
                r.subtitle().unwrap_or_default().replace('\n', " ")
            ));
        }
        c = w.next_sibling();
    }
    format!(
        "vocab_list: rows={} [{}] status={:?}",
        rows.len(),
        rows.join(" | "),
        l.status.label()
    )
}
