//! The list of your vocabulary: every word with its definition, the ones you defined yourself marked
//! "Mine", a search, a filter (all / mine / from a dictionary), adding, editing and removing, a pasted
//! glossary, CSV import, and export (CSV, JSON, a StarDict dictionary, Anki). With a book it is the
//! glossary of that book ("Terms in this book").

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bookx_lookup::own::{
    GlossaryItem, export_csv, export_stardict, import_glossary, parse_glossary,
};
use bookx_lookup::vocab::{AnkiOptions, Scope, Source, VocabEntry, export_anki_tsv};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio, glib};

use super::SharedRef;
use super::vocab_own::{self, Ctx};
use crate::i18n::{ntr, tr, trf};

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
    /// The text of the last action; it is shown as a toast and read back by the test hook.
    message: RefCell<String>,
    /// Header buttons: the host dialog puts them in its header bar (`pack_into`).
    add_btn: gtk::Button,
    more_btn: gtk::MenuButton,
    rows: gtk::ListBox,
    empty: adw::StatusPage,
    toasts: adw::ToastOverlay,
    /// What Undo of the last removal does (the test hook presses it).
    last_undo: RefCell<Option<Rc<dyn Fn()>>>,
}

thread_local! {
    static LAST: RefCell<Option<Rc<List>>> = const { RefCell::new(None) };
    static PASTE: RefCell<Option<(gtk::TextView, gtk::Button)>> = const { RefCell::new(None) };
}

/// Which message the empty list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placeholder {
    /// The vocabulary has no words at all.
    NoWords,
    /// There are words, but the search or the filter hides all of them.
    NoResults,
}

fn placeholder_for(all: usize, shown: usize) -> Placeholder {
    if all > 0 && shown == 0 {
        Placeholder::NoResults
    } else {
        Placeholder::NoWords
    }
}

fn set_placeholder(page: &adw::StatusPage, kind: Placeholder) {
    match kind {
        Placeholder::NoWords => {
            page.set_title(&tr("No Words Yet"));
            page.set_description(Some(&tr(
                "Select a word or phrase while reading and choose Add to Vocabulary to write your own definition, or look a word up in a dictionary.",
            )));
        }
        Placeholder::NoResults => {
            page.set_title(&tr("No Results"));
            page.set_description(Some(&tr("Try a different search or filter")));
        }
    }
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
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    inner.set_margin_top(8);
    inner.set_margin_bottom(12);
    inner.set_margin_start(12);
    inner.set_margin_end(12);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some(&tr("Search words, definitions, notes and tags")));
    super::audit::label(&search, &tr("Search the vocabulary"));
    inner.append(&search);

    let filter = adw::ToggleGroup::new();
    filter.set_homogeneous(false);
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
    inner.append(&filter);

    let rows = gtk::ListBox::new();
    rows.add_css_class("boxed-list");
    rows.set_selection_mode(gtk::SelectionMode::None);
    let empty = adw::StatusPage::new();
    empty.set_icon_name(Some("accessories-dictionary-symbolic"));
    set_placeholder(&empty, Placeholder::NoWords);
    rows.set_placeholder(Some(&empty));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&rows));
    inner.append(&scroll);

    let status = gtk::Label::new(None);
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_wrap(true);
    inner.append(&status);

    // Add Term and the paste/import/export menu live in the header of the host dialog; an empty
    // vocabulary also offers Add Term in its placeholder
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some(&tr("Add Term")));
    super::audit::label(&add, &tr("Add Term"));
    let more = gtk::MenuButton::new();
    more.set_icon_name("view-more-symbolic");
    more.set_tooltip_text(Some(&tr("Paste, Import and Export")));
    super::audit::label(&more, &tr("Paste, import and export"));
    let empty_add = gtk::Button::with_label(&tr("Add Term"));
    empty_add.add_css_class("suggested-action");
    empty_add.add_css_class("pill");
    empty_add.set_halign(gtk::Align::Center);
    empty.set_child(Some(&empty_add));

    // the toasts of this list (a removed term can be brought back) show over it, in a dialog too
    let toasts = super::dialog_toast::overlay(&inner);
    toasts.set_vexpand(true);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&toasts);

    let list = Rc::new_cyclic(|weak: &std::rc::Weak<List>| {
        let weak = weak.clone();
        let empty_add2 = empty_add.clone();
        let (rows2, empty2, status2, search2, filter2, shared2, book2) = (
            rows.clone(),
            empty.clone(),
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
            let kind = placeholder_for(all.len(), entries.len());
            set_placeholder(&empty2, kind);
            empty_add2.set_visible(kind == Placeholder::NoWords);
            let mine = all.iter().filter(|e| e.own.is_some()).count();
            let summary = trf(
                "{} words · {} with your own definition",
                &[&all.len(), &mine],
            );
            status2.set_label(&summary);
        });
        List {
            root: root.clone(),
            refresh,
            search: search.clone(),
            filter: filter.clone(),
            shared: shared.clone(),
            book: book.clone(),
            status: status.clone(),
            message: RefCell::new(String::new()),
            add_btn: add.clone(),
            more_btn: more.clone(),
            rows: rows.clone(),
            empty: empty.clone(),
            toasts: toasts.clone(),
            last_undo: RefCell::new(None),
        }
    });

    {
        let l = list.clone();
        search.connect_search_changed(move |_| (l.refresh)());
        let l = list.clone();
        filter.connect_active_name_notify(move |_| (l.refresh)());
        let l = list.clone();
        add.connect_clicked(move |_| edit_term(&l, ""));
        let l = list.clone();
        empty_add.connect_clicked(move |_| edit_term(&l, ""));
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
    // the row itself edits (or adds your definition); the buttons are jump and remove, two at most
    let tip = if e.own.is_some() {
        trf("Edit Your Definition of {}", &[&e.word])
    } else {
        trf("Add Your Definition of {}", &[&e.word])
    };
    row.set_tooltip_text(Some(&tip));
    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_valign(gtk::Align::Center);
    let tip = trf("Remove {} from the vocabulary", &[&e.word]);
    del.set_tooltip_text(Some(&tip));
    super::audit::label(&del, &tip);
    row.add_suffix(&del);
    let word = e.word.clone();
    {
        let (l2, w2) = (l.clone(), word.clone());
        row.connect_activated(move |_| edit_term(&l2, &w2));
    }
    {
        let (l, w) = (l.clone(), word);
        del.connect_clicked(move |_| remove_term(&l, &w));
    }
    row
}

/// Removes a word at once and offers to bring it back (no confirmation: it is reversible).
fn remove_term(l: &Rc<List>, word: &str) {
    let entry = l.shared.vocab.borrow().store.find(word).cloned();
    l.shared.vocab.borrow_mut().remove_word(word);
    changed(l);
    let Some(entry) = entry else { return };
    let (l2, stash) = (l.clone(), Rc::new(RefCell::new(Some(entry))));
    let undo: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(e) = stash.borrow_mut().take() {
            l2.shared.vocab.borrow_mut().restore_entry(e);
            changed(&l2);
        }
    });
    *l.last_undo.borrow_mut() = Some(undo.clone());
    super::dialog_toast::show_undo(&l.toasts, &tr("Removed from vocabulary"), move || undo());
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

/// Tells the user what an import, paste or export did: a toast over the list, not a label that
/// stays.
fn say(l: &List, text: String) {
    super::dialog_toast::show_plain(&l.toasts, &text);
    *l.message.borrow_mut() = text;
}

impl List {
    /// Puts Add Term at the start of the header and the paste/import/export menu at its end; the
    /// buttons show only while `show` says so (the Words page of the Vocabulary dialog).
    pub fn pack_into(&self, header: &adw::HeaderBar) {
        header.pack_start(&self.add_btn);
        header.pack_end(&self.more_btn);
    }

    /// Show or hide the header buttons with the page that owns them.
    pub fn set_header_buttons_visible(&self, on: bool) {
        self.add_btn.set_visible(on);
        self.more_btn.set_visible(on);
    }
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
    trf("{} terms added, {} updated", &[&added, &updated])
}

fn paste_dialog(l: &Rc<List>) {
    let d = super::dialog_toast::new_dialog();
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
    let count = gtk::Label::new(None);
    count.set_xalign(0.0);
    count.set_visible(false);
    // while nothing is parsed yet, a page says what to type
    let hint = adw::StatusPage::new();
    hint.add_css_class("compact");
    hint.set_icon_name(Some("edit-paste-symbolic"));
    hint.set_title(&tr("No Terms Yet"));
    hint.set_description(Some(&tr(
        "Paste one term per line, for example: word; its definition.",
    )));
    hint.set_vexpand(false);
    let all = gtk::CheckButton::with_label(&tr("Apply to all books"));
    all.set_visible(l.book.is_some());
    all.set_active(l.book.is_none());
    bx.append(&help);
    bx.append(&sc);
    bx.append(&hint);
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
        let (items, count, add, hint) = (items.clone(), count.clone(), add.clone(), hint.clone());
        tv.buffer().connect_changed(move |b| {
            let text = b.text(&b.start_iter(), &b.end_iter(), false);
            let parsed = parse_glossary(&text);
            hint.set_visible(parsed.is_empty());
            count.set_visible(!parsed.is_empty());
            count.set_label(&ntr("{} term found", "{} terms found", parsed.len()));
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
            tr("Export Your Definitions as CSV"),
            "csv",
            "my-vocabulary.csv",
        ),
        (
            tr("Export Your Definitions as JSON"),
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
    let d = super::dialog_toast::new_dialog();
    d.set_title(&tr("Terms in This Book"));
    let book_title = book.title.clone();
    d.set_content_width(520);
    d.set_content_height(560);
    let list = build(shared, Some(book), None);
    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    // the book is the subtitle, so a long title cannot push the window name out
    header.set_title_widget(Some(&adw::WindowTitle::new(
        &tr("Terms in This Book"),
        &book_title,
    )));
    list.pack_into(&header);
    view.add_top_bar(&header);
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

fn count_buttons(w: &gtk::Widget) -> usize {
    let mut n = usize::from(w.is::<gtk::Button>());
    let mut c = w.first_child();
    while let Some(x) = c {
        n += count_buttons(&x);
        c = x.next_sibling();
    }
    n
}

/// The most buttons any row of the list has.
fn most_buttons_in_a_row(rows: &gtk::ListBox) -> usize {
    let mut most = 0;
    let mut c = rows.first_child();
    while let Some(w) = c {
        most = most.max(count_buttons(&w));
        c = w.next_sibling();
    }
    most
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
        remove_term(&l, v);
    } else if arg == "undo" {
        let undo = l.last_undo.borrow_mut().take();
        if let Some(u) = undo {
            u();
        }
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
    let buttons = most_buttons_in_a_row(&l.rows);
    format!(
        "vocab_list: rows={} [{}] max_buttons={buttons} status={:?} placeholder={:?}",
        rows.len(),
        rows.join(" | "),
        if l.message.borrow().is_empty() {
            l.status.label().to_string()
        } else {
            l.message.borrow().clone()
        },
        l.empty.title()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_result_is_told_apart_from_an_empty_vocabulary() {
        assert_eq!(placeholder_for(0, 0), Placeholder::NoWords);
        assert_eq!(placeholder_for(5, 0), Placeholder::NoResults);
        assert_eq!(placeholder_for(5, 2), Placeholder::NoWords);
    }
}
