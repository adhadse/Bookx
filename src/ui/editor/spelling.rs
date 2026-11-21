//! Tools → Check Spelling: the spelling side panel (CE-270..CE-278, CE-280). The word list of the whole book
//! (misspelled words, or all words) with counts and languages; selecting a word shows its next occurrence in the
//! editor and suggestions; Change All, Ignore (this session or for this book), Add to a user dictionary, files
//! left out, Refresh, Ctrl+F8 / Ctrl+Shift+F8. The scan runs on a worker thread. The engine is `bookx_editing::spell_session`.

use super::search::{byte_to_char_idx, char_to_byte_idx};
use super::*;
use crate::editor_spell::{self, Spot};
use crate::i18n::ntr;
use crate::i18n::{tr, trf};
use bookx_editing::spell_session::{self as sess, BookWords, WordEntry};

/// Rows shown at most; the filter narrows a longer list.
const MAX_ROWS: usize = 1500;
/// Suggestions listed.
const MAX_SUGGESTIONS: usize = 8;

#[derive(Debug)]
pub enum Msg {
    Toggle,
    Open,
    Close,
    Refresh,
    Scanned(u64, Box<BookWords>),
    /// Row `n` of the list was selected: show its next occurrence, keep the focus where it is.
    Select(usize),
    /// Row `n` was activated: go to its next occurrence and focus the editor.
    Jump(usize),
    Suggestion(String),
    Change,
    /// Ignore the selected word; `true` remembers it for this book.
    Ignore(bool),
    AddTo(String),
    /// Undo of the Ignore toast: the word is reported again (`true`: also forget it for this book).
    UndoIgnore(String, bool),
    /// Undo of the Add to Dictionary toast: the word leaves that dictionary.
    UndoAdd(String, String),
    ShowAll(bool),
    ByCount(bool),
    Filter(String),
    /// Ctrl+F8 (`true`) / Shift+F8: the next misspelling in the editor.
    NextMistake(bool),
    ExcludeCurrent,
    Include(String),
    ManageExcluded,
    ManageDictionaries,
    /// A dictionary was imported, removed, created or changed: reload everything.
    DictionariesChanged,
    /// Words were added to, removed from, or a user dictionary was switched on or off.
    UserWordsChanged,
    Toast(String),
}

struct Ui {
    root: gtk::Revealer,
    list: gtk::ListBox,
    status: gtk::Label,
    spinner: gtk::Spinner,
    word: gtk::Label,
    detail: gtk::Box,
    placeholder: adw::StatusPage,
    entry: gtk::Entry,
    sugg: gtk::ListBox,
    change: gtk::Button,
    ignore: gtk::Button,
    ignore_book: gtk::Button,
    add: gtk::Button,
    dict: gtk::DropDown,
    dict_names: gtk::StringList,
    all: gtk::ToggleButton,
    count: gtk::ToggleButton,
    filter: gtk::SearchEntry,
    /// Set while the list is rebuilt, so the selection handler ignores the churn.
    busy: Rc<Cell<bool>>,
}

#[derive(Default)]
pub struct Panel {
    ui: Option<Ui>,
    words: Option<BookWords>,
    /// Entry indices in display order.
    shown: Vec<usize>,
    selected: Option<usize>,
    show_all: bool,
    by_count: bool,
    filter: String,
    generation: u64,
    scanning: bool,
}

impl Panel {
    pub fn new(tx: relm4::Sender<Input>) -> Panel {
        Panel {
            ui: Some(build_ui(&tx)),
            by_count: true,
            ..Default::default()
        }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.ui
            .as_ref()
            .expect("built in new")
            .root
            .clone()
            .upcast()
    }

    fn ui(&self) -> &Ui {
        self.ui.as_ref().expect("built in new")
    }
}

fn icon_btn(icon: &str, label: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(label));
    b.update_property(&[gtk::accessible::Property::Label(label)]);
    b.add_css_class("flat");
    b
}

fn icon_toggle(icon: &str, label: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.set_icon_name(icon);
    b.set_tooltip_text(Some(label));
    b.update_property(&[gtk::accessible::Property::Label(label)]);
    b.add_css_class("flat");
    b
}

fn build_ui(tx0: &relm4::Sender<Input>) -> Ui {
    let emit = |tx: &relm4::Sender<Input>, m: Msg| tx.emit(Input::SpellPanel(m));

    let title = gtk::Label::new(Some(&tr("Spelling")));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let spinner = gtk::Spinner::new();
    let refresh = icon_btn("view-refresh-symbolic", &tr("Check the spelling again"));
    let all = icon_toggle(
        "view-list-symbolic",
        &tr("Show all words, not only misspelled ones"),
    );
    let count = icon_toggle(
        "view-sort-descending-symbolic",
        &tr("Sort by number of occurrences"),
    );
    count.set_active(true);
    let menu = gio::Menu::new();
    menu.append(
        Some(&tr("Exclude This File from Spell Check")),
        Some("spellpanel.exclude"),
    );
    menu.append(Some(&tr("Excluded Files")), Some("spellpanel.excluded"));
    menu.append(
        Some(&tr("Spelling Dictionaries")),
        Some("spellpanel.dictionaries"),
    );
    let menu_btn = gtk::MenuButton::new();
    menu_btn.set_icon_name("view-more-symbolic");
    menu_btn.set_menu_model(Some(&menu));
    menu_btn.set_tooltip_text(Some(&tr("More spelling options")));
    menu_btn.update_property(&[gtk::accessible::Property::Label(&tr(
        "More spelling options",
    ))]);
    menu_btn.add_css_class("flat");
    let close = icon_btn("window-close-symbolic", &tr("Close the spelling panel"));
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for w in [
        title.upcast_ref::<gtk::Widget>(),
        spinner.upcast_ref(),
        count.upcast_ref(),
        all.upcast_ref(),
        refresh.upcast_ref(),
        menu_btn.upcast_ref(),
        close.upcast_ref(),
    ] {
        head.append(w);
    }
    head.set_margin_start(8);
    head.set_margin_end(4);
    head.set_margin_top(4);

    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    status.add_css_class("caption");
    status.set_margin_start(8);
    status.set_margin_end(8);

    let filter = gtk::SearchEntry::new();
    filter.set_placeholder_text(Some(&tr("Filter words")));
    filter.update_property(&[gtk::accessible::Property::Label(&tr(
        "Filter the word list",
    ))]);
    filter.set_margin_start(8);
    filter.set_margin_end(8);

    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.update_property(&[gtk::accessible::Property::Label(&tr("Words"))]);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&list));
    scroll.set_vexpand(true);
    scroll.set_min_content_height(120);

    let word = gtk::Label::new(Some(&tr("Select a word")));
    word.set_xalign(0.0);
    word.set_wrap(true);
    word.add_css_class("heading");
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(&tr("Change to")));
    entry.update_property(&[gtk::accessible::Property::Label(&tr(
        "Replacement for the selected word",
    ))]);
    let sugg = gtk::ListBox::new();
    sugg.add_css_class("boxed-list");
    sugg.update_property(&[gtk::accessible::Property::Label(&tr("Suggestions"))]);
    let sugg_scroll = gtk::ScrolledWindow::new();
    sugg_scroll.set_child(Some(&sugg));
    sugg_scroll.set_min_content_height(90);
    sugg_scroll.set_max_content_height(160);
    sugg_scroll.set_propagate_natural_height(true);

    let change = gtk::Button::with_label(&tr("Change All"));
    change.add_css_class("suggested-action");
    change.set_tooltip_text(Some(&tr("Change every occurrence of the word in the book")));
    let ignore = gtk::Button::with_label(&tr("Ignore"));
    ignore.set_tooltip_text(Some(&tr("Ignore this word until the editor is closed")));
    let ignore_book = gtk::Button::with_label(&tr("Ignore in Book"));
    ignore_book.set_tooltip_text(Some(&tr("Ignore this word in this book from now on")));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.append(&change);
    actions.append(&ignore);
    actions.append(&ignore_book);
    let dict_names = gtk::StringList::new(&[]);
    let dict = gtk::DropDown::new(Some(dict_names.clone()), gtk::Expression::NONE);
    dict.set_tooltip_text(Some(&tr("The dictionary to add the word to")));
    dict.update_property(&[gtk::accessible::Property::Label(&tr(
        "Dictionary to add the word to",
    ))]);
    let add = gtk::Button::with_label(&tr("Add to Dictionary"));
    let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    add_row.append(&add);
    add_row.append(&dict);

    let detail = gtk::Box::new(gtk::Orientation::Vertical, 6);
    detail.set_margin_start(8);
    detail.set_margin_end(8);
    detail.set_margin_bottom(8);
    detail.set_margin_top(6);
    detail.append(&word);
    detail.append(&sugg_scroll);
    detail.append(&entry);
    detail.append(&actions);
    detail.append(&add_row);

    let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
    col.append(&head);
    col.append(&status);
    col.append(&filter);
    col.append(&scroll);
    col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let placeholder = adw::StatusPage::builder()
        .icon_name("tools-check-spelling-symbolic")
        .title(tr("Select a Word"))
        .description(tr("Pick a word in the list for suggestions"))
        .build();
    placeholder.add_css_class("compact");
    col.append(&placeholder);
    detail.set_visible(false);
    col.append(&detail);
    col.set_width_request(330);
    col.add_css_class("background");
    let sep = gtk::Separator::new(gtk::Orientation::Vertical);
    let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    wrap.append(&sep);
    wrap.append(&col);
    let root = gtk::Revealer::new();
    root.set_transition_type(gtk::RevealerTransitionType::SlideLeft);
    root.set_child(Some(&wrap));
    root.set_reveal_child(false);

    let busy = Rc::new(Cell::new(false));
    {
        let tx = tx0.clone();
        let busy = busy.clone();
        list.connect_row_selected(move |_, row| {
            if busy.get() {
                return;
            }
            if let Some(r) = row {
                emit(&tx, Msg::Select(r.index() as usize));
            }
        });
        let tx = tx0.clone();
        list.connect_row_activated(move |_, row| emit(&tx, Msg::Jump(row.index() as usize)));
        let tx = tx0.clone();
        sugg.connect_row_activated(move |_, row| {
            if let Some(l) = row.child().and_downcast::<gtk::Label>() {
                emit(&tx, Msg::Suggestion(l.text().to_string()));
            }
        });
        let tx = tx0.clone();
        sugg.connect_row_selected(move |_, row| {
            if let Some(l) = row.and_then(|r| r.child()).and_downcast::<gtk::Label>() {
                emit(&tx, Msg::Suggestion(l.text().to_string()));
            }
        });
        let tx = tx0.clone();
        entry.connect_activate(move |_| emit(&tx, Msg::Change));
        let tx = tx0.clone();
        change.connect_clicked(move |_| emit(&tx, Msg::Change));
        let tx = tx0.clone();
        ignore.connect_clicked(move |_| emit(&tx, Msg::Ignore(false)));
        let tx = tx0.clone();
        ignore_book.connect_clicked(move |_| emit(&tx, Msg::Ignore(true)));
        let (tx2, d) = (tx0.clone(), dict.clone());
        add.connect_clicked(move |_| {
            if let Some(n) = d
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().to_string())
            {
                emit(&tx2, Msg::AddTo(n));
            }
        });
        let tx = tx0.clone();
        refresh.connect_clicked(move |_| emit(&tx, Msg::Refresh));
        let tx = tx0.clone();
        close.connect_clicked(move |_| emit(&tx, Msg::Close));
        let tx = tx0.clone();
        all.connect_toggled(move |b| emit(&tx, Msg::ShowAll(b.is_active())));
        let tx = tx0.clone();
        count.connect_toggled(move |b| emit(&tx, Msg::ByCount(b.is_active())));
        let tx = tx0.clone();
        filter.connect_search_changed(move |e| emit(&tx, Msg::Filter(e.text().to_string())));
        let group = gio::SimpleActionGroup::new();
        for (name, msg) in [
            ("exclude", (|| Msg::ExcludeCurrent) as fn() -> Msg),
            ("excluded", || Msg::ManageExcluded),
            ("dictionaries", || Msg::ManageDictionaries),
        ] {
            let a = gio::SimpleAction::new(name, None);
            let tx = tx0.clone();
            a.connect_activate(move |_, _| emit(&tx, msg()));
            group.add_action(&a);
        }
        root.insert_action_group("spellpanel", Some(&group));
    }
    Ui {
        root,
        list,
        status,
        spinner,
        word,
        detail,
        placeholder,
        entry,
        sugg,
        change,
        ignore,
        ignore_book,
        add,
        dict,
        dict_names,
        all,
        count,
        filter,
        busy,
    }
}

/// Entry indices to show, in order.
fn arrange(words: &BookWords, show_all: bool, by_count: bool, filter: &str) -> Vec<usize> {
    let f = filter.trim().to_lowercase();
    let mut idx: Vec<usize> = (0..words.entries.len())
        .filter(|i| {
            let e = &words.entries[*i];
            (show_all || e.misspelled) && (f.is_empty() || e.word.to_lowercase().contains(&f))
        })
        .collect();
    idx.sort_by(|a, b| sess::compare_entries(&words.entries[*a], &words.entries[*b], by_count));
    idx
}

impl Editor {
    pub(super) fn spell_panel(&mut self, msg: Msg) {
        match msg {
            Msg::Toggle => {
                if self.spelling.ui().root.reveals_child() {
                    self.spell_panel(Msg::Close)
                } else {
                    self.spell_panel(Msg::Open)
                }
            }
            Msg::Open => {
                // in a narrow window the spelling pane replaces the preview instead of squeezing both
                if self.w.split.is_collapsed() && self.w.preview_btn.is_active() {
                    self.ext.spell_hid_preview.set(true);
                    self.w.preview_btn.set_active(false);
                }
                self.spelling.ui().root.set_reveal_child(true);
                self.spell_panel(Msg::Refresh);
            }
            Msg::Close => {
                if self.ext.spell_hid_preview.replace(false) {
                    self.w.preview_btn.set_active(true);
                }
                self.spelling.ui().root.set_reveal_child(false);
                self.w.view.grab_focus();
            }
            Msg::Refresh => self.spell_scan(),
            Msg::Scanned(generation, words) => {
                if generation != self.spelling.generation {
                    return;
                }
                self.spelling.scanning = false;
                self.spelling.ui().spinner.stop();
                self.spelling.words = Some(*words);
                self.spell_fill();
            }
            Msg::Select(n) => self.spell_select(n, false),
            Msg::Jump(n) => self.spell_select(n, true),
            Msg::Suggestion(s) => {
                let ui = self.spelling.ui();
                ui.entry.set_text(&s);
                ui.entry.set_position(-1);
            }
            Msg::Change => self.spell_change(),
            Msg::Ignore(book) => self.spell_ignore(book),
            Msg::AddTo(name) => self.spell_add(&name),
            Msg::UndoIgnore(word, book) => {
                {
                    let mut sh = self.spell.borrow_mut();
                    if let Some(st) = sh.state.as_mut() {
                        st.unignore(&word);
                    }
                    if book {
                        sh.prefs.ignored.remove(&word);
                    }
                }
                if book {
                    self.save_spell_prefs();
                }
                self.spell_changed_scope();
                self.toast(&tr("Undone"));
            }
            Msg::UndoAdd(dict, word) => {
                let res = self
                    .spell
                    .borrow_mut()
                    .state
                    .as_mut()
                    .map(|s| s.remove_from_named(&dict, &word));
                if let Some(Err(e)) = res {
                    self.toast(&trf(
                        "Could not remove the word: {}",
                        &[&format!("{:#}", e)],
                    ));
                } else {
                    self.spell_changed_scope();
                    self.toast(&tr("Undone"));
                }
            }
            Msg::ShowAll(on) => {
                self.spelling.show_all = on;
                self.spell_fill();
            }
            Msg::ByCount(on) => {
                self.spelling.by_count = on;
                self.spell_fill();
            }
            Msg::Filter(t) => {
                self.spelling.filter = t;
                self.spell_fill();
            }
            Msg::NextMistake(fwd) => self.spell_next_mistake(fwd),
            Msg::ExcludeCurrent => match self.current.clone() {
                Some(p) if live_is_markup(&p) => {
                    self.spell.borrow_mut().prefs.excluded.insert(p.clone());
                    self.save_spell_prefs();
                    self.toast(&trf("{} is left out of the spell check", &[&file_name(&p)]));
                    self.spell_changed_scope();
                }
                _ => self.toast(&tr("Open a text file first")),
            },
            Msg::Include(p) => {
                self.spell.borrow_mut().prefs.excluded.remove(&p);
                self.save_spell_prefs();
                self.spell_changed_scope();
            }
            Msg::ManageExcluded => self.spell_excluded_dialog(),
            Msg::ManageDictionaries => self.spell_dictionaries_dialog(),
            Msg::Toast(t) => self.toast(&t),
            Msg::UserWordsChanged => {
                if let Some(st) = self.spell.borrow_mut().state.as_mut() {
                    st.reload_user_words();
                }
                self.spell_changed_scope();
            }
            Msg::DictionariesChanged => {
                self.spell.borrow_mut().state = None;
                self.ensure_spell();
                self.spell_changed_scope();
            }
        }
    }

    fn save_spell_prefs(&self) {
        self.spell.borrow().prefs.save(self.book_id);
    }

    /// The files or words that count changed: rescan (when the panel is open) and underline again.
    fn spell_changed_scope(&mut self) {
        if self.spelling.ui().root.reveals_child() {
            self.spell_scan();
        }
        self.run_live();
    }

    fn spell_scan(&mut self) {
        self.flush();
        self.ensure_spell();
        let excluded: HashSet<String> =
            self.spell.borrow().prefs.excluded.iter().cloned().collect();
        // what is on screen counts, whether or not the editor has noted the edit yet
        let (lang, sources, langs) = {
            let b = self.book.borrow();
            let (lang, mut sources) = sess::collect_sources(&b);
            for (p, t) in sources.iter_mut() {
                if let Some(buf) = self.buffers.get(p) {
                    *t = buffer_text(buf);
                }
            }
            let mut langs = std::collections::BTreeSet::new();
            let pkg = bookx_editing::spell::lang_key(&lang);
            if !pkg.is_empty() {
                langs.insert(pkg);
            }
            for (p, t) in &sources {
                if !excluded.contains(p) {
                    langs.extend(sess::languages_in_source(t));
                }
            }
            (lang, sources, langs)
        };
        let checker = {
            let mut sh = self.spell.borrow_mut();
            let Some(st) = sh.state.as_mut() else { return };
            st.load_languages(&langs, &crate::editor_spell_dicts::search_dirs());
            st.snapshot()
        };
        self.spelling.generation += 1;
        self.spelling.scanning = true;
        let generation = self.spelling.generation;
        let ui = self.spelling.ui();
        ui.spinner.start();
        ui.status.set_label(&tr("Checking the book…"));
        self.refresh_dict_choices();
        let tx = self.sender.input_sender().clone();
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || {
                sess::scan_sources(&lang, &sources, &checker, &excluded)
            })
            .await;
            if let Ok(words) = res {
                tx.emit(Input::SpellPanel(Msg::Scanned(generation, Box::new(words))));
            }
        });
    }

    fn refresh_dict_choices(&self) {
        let ui = self.spelling.ui();
        let names: Vec<String> = self
            .spell
            .borrow()
            .state
            .as_ref()
            .and_then(|s| s.user_dicts().map(|d| d.names()))
            .unwrap_or_else(|| vec![bookx_editing::spell_user::PERSONAL.to_string()]);
        let cur = ui
            .dict
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|s| s.string().to_string());
        ui.dict_names.splice(
            0,
            ui.dict_names.n_items(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        if let Some(i) = cur.and_then(|c| names.iter().position(|n| *n == c)) {
            ui.dict.set_selected(i as u32);
        }
    }

    /// Rebuilds the list from the last scan and the display options.
    fn spell_fill(&mut self) {
        let Some(words) = self.spelling.words.as_ref() else {
            return;
        };
        let shown = arrange(
            words,
            self.spelling.show_all,
            self.spelling.by_count,
            &self.spelling.filter,
        );
        let keep = self
            .spelling
            .selected
            .and_then(|e| shown.iter().position(|i| *i == e));
        let ui = self.spelling.ui();
        ui.busy.set(true);
        while let Some(c) = ui.list.first_child() {
            ui.list.remove(&c);
        }
        for i in shown.iter().take(MAX_ROWS) {
            let e = &words.entries[*i];
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&e.word))
                .subtitle(glib::markup_escape_text(&e.lang))
                .activatable(true)
                .build();
            row.set_title_lines(1);
            let c = gtk::Label::new(Some(&e.count.to_string()));
            c.add_css_class("dim-label");
            c.add_css_class("numeric");
            row.add_suffix(&c);
            if !e.checked {
                row.add_css_class("dim-label");
            }
            ui.list.append(&row);
        }
        if let Some(r) = keep.and_then(|k| ui.list.row_at_index(k as i32)) {
            ui.list.select_row(Some(&r));
        }
        ui.busy.set(false);
        let bad = words.misspelled().count();
        let mut s = format!(
            "{} misspelled · {} different words · {} files checked",
            bad,
            words.entries.len(),
            words.files.len()
        );
        if shown.len() > MAX_ROWS {
            s.push_str(&format!(" · showing the first {MAX_ROWS}"));
        }
        if !words.skipped.is_empty() {
            s.push_str(&format!(" · {} left out", words.skipped.len()));
        }
        if !words.unavailable.is_empty() {
            let l: Vec<&str> = words.unavailable.iter().map(String::as_str).collect();
            s.push_str(&format!(" · no dictionary for {}", l.join(", ")));
        }
        ui.status.set_label(&s);
        let why: Vec<String> = self
            .spell
            .borrow()
            .state
            .as_ref()
            .map(|st| {
                st.missing_languages()
                    .iter()
                    .map(|(l, r)| format!("{l}: {r}"))
                    .collect()
            })
            .unwrap_or_default();
        ui.status
            .set_tooltip_text((!why.is_empty()).then(|| why.join("\n")).as_deref());
        self.spelling.shown = shown;
        if keep.is_none() {
            self.spelling.selected = None;
            self.spell_detail(None);
        }
    }

    fn spell_detail(&self, entry: Option<usize>) {
        let ui = self.spelling.ui();
        while let Some(c) = ui.sugg.first_child() {
            ui.sugg.remove(&c);
        }
        let Some(e) = entry.and_then(|i| self.spelling.words.as_ref().map(|w| &w.entries[i]))
        else {
            ui.word.set_label(&tr("Select a word"));
            ui.detail.set_visible(false);
            ui.placeholder.set_visible(true);
            ui.entry.set_text("");
            for b in [&ui.change, &ui.ignore, &ui.ignore_book, &ui.add] {
                b.set_sensitive(false);
            }
            return;
        };
        ui.detail.set_visible(true);
        ui.placeholder.set_visible(false);
        ui.word.set_label(&format!("{} ({})", e.word, e.lang));
        ui.entry.set_text(&e.word);
        ui.entry.set_position(-1);
        ui.change.set_sensitive(true);
        for b in [&ui.ignore, &ui.ignore_book, &ui.add] {
            b.set_sensitive(e.misspelled);
        }
        if e.misspelled {
            let spot = Spot {
                start: 0,
                end: 0,
                word: e.word.clone(),
                lang: e.lang.clone(),
            };
            let sugg = self
                .spell
                .borrow()
                .state
                .as_ref()
                .map(|s| s.suggest(&spot))
                .unwrap_or_default();
            for s in sugg.into_iter().take(MAX_SUGGESTIONS) {
                let l = gtk::Label::new(Some(&s));
                l.set_xalign(0.0);
                l.set_margin_start(6);
                l.set_margin_end(6);
                l.set_margin_top(4);
                l.set_margin_bottom(4);
                ui.sugg.append(&l);
            }
        }
    }

    /// Where the caret is: the open file and the byte offset in it.
    fn caret_byte(&self) -> Option<(String, usize)> {
        let p = self.current.clone()?;
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let off = buf.cursor_position() as usize;
        Some((p, char_to_byte_idx(&text, off)))
    }

    fn spell_select(&mut self, n: usize, focus: bool) {
        let Some(&ei) = self.spelling.shown.get(n) else {
            return;
        };
        self.spelling.selected = Some(ei);
        self.spell_detail(Some(ei));
        let caret = self.caret_byte();
        let occ = {
            let Some(words) = self.spelling.words.as_ref() else {
                return;
            };
            let from = caret.as_ref().map(|(p, b)| (p.as_str(), *b));
            // selecting again moves on: from the caret, which sits on the previous hit
            words
                .next_occurrence(&words.entries[ei], from)
                .map(|o| (words.file_of(&o).to_string(), o.start, o.end))
        };
        if let Some((file, a, b)) = occ {
            self.spell_show(&file, a, b, focus);
        }
    }

    /// Selects the byte range in the editor. Without `focus` the keyboard focus stays where it is, so the list
    /// can be walked with the arrow keys.
    fn spell_show(&mut self, path: &str, start: usize, end: usize, focus: bool) {
        if self.current.as_deref() != Some(path) {
            self.select_file(path);
        }
        let buf = self.w.view.buffer();
        let text = buffer_text(&buf);
        let fresh = text.get(start..end).is_some();
        if !fresh {
            self.toast(&tr("The text changed since it was checked: refreshing"));
            self.spell_scan();
            return;
        }
        let a = byte_to_char_idx(&text, start) as i32;
        let b = byte_to_char_idx(&text, end) as i32;
        let (ia, ib) = (buf.iter_at_offset(a), buf.iter_at_offset(b));
        buf.select_range(&ib, &ia);
        let mut ia = buf.iter_at_offset(a);
        self.w.view.scroll_to_iter(&mut ia, 0.1, true, 0.5, 0.4);
        if focus {
            self.w.view.grab_focus();
        }
    }

    fn selected_entry(&self) -> Option<WordEntry> {
        let i = self.spelling.selected?;
        self.spelling.words.as_ref().map(|w| w.entries[i].clone())
    }

    fn spell_change(&mut self) {
        let Some(e) = self.selected_entry() else {
            self.toast(&tr("Select a word first"));
            return;
        };
        let with = self.spelling.ui().entry.text().trim().to_string();
        if with.is_empty() || with == e.word {
            self.toast(&tr("Type or choose the word to change it to"));
            return;
        }
        let excluded: HashSet<String> =
            self.spell.borrow().prefs.excluded.iter().cloned().collect();
        let (word, to) = (e.word.clone(), with.clone());
        self.run_op(
            &trf("Change “{}” to “{}”", &[&word, &to]),
            move |b, _| {
                let (files, n) = sess::change_everywhere(b, &word, &to, &excluded)?;
                Ok(editor_tools::Outcome {
                    changed: n > 0,
                    summary: if n == 0 {
                        trf("“{}” was not found", &[&word])
                    } else {
                        trf(
                            "Changed {} of “{}” in {}",
                            &[
                                &ntr("{} occurrence", "{} occurrences", n),
                                &word,
                                &ntr("{} file", "{} files", files),
                            ],
                        )
                    },
                    details: Vec::new(),
                })
            },
        );
        self.spelling.selected = None;
        self.spell_changed_scope();
    }

    fn spell_ignore(&mut self, book: bool) {
        let Some(e) = self.selected_entry() else {
            return;
        };
        {
            let mut sh = self.spell.borrow_mut();
            if let Some(st) = sh.state.as_mut() {
                st.ignore(&e.word);
            }
            if book {
                sh.prefs.ignored.insert(e.word.clone());
            }
        }
        if book {
            self.save_spell_prefs();
        }
        let word = e.word.clone();
        let _ = self.toast_button(
            &if book {
                trf("“{}” is ignored in this book", &[&e.word])
            } else {
                trf("“{}” is ignored until the editor closes", &[&e.word])
            },
            &tr("Undo"),
            move || Input::SpellPanel(Msg::UndoIgnore(word.clone(), book)),
        );
        self.spelling.selected = None;
        self.spell_changed_scope();
    }

    fn spell_add(&mut self, dict: &str) {
        let Some(e) = self.selected_entry() else {
            return;
        };
        let res = self
            .spell
            .borrow_mut()
            .state
            .as_mut()
            .map(|s| s.add_to_named(dict, &e.word));
        match res {
            Some(Err(err)) => self.toast(&trf(
                "Could not save the word: {}",
                &[&format!("{:#}", err)],
            )),
            _ => {
                let (d, w) = (dict.to_string(), e.word.clone());
                let _ = self.toast_button(
                    &trf("“{}” added to {}", &[&e.word, &dict]),
                    &tr("Undo"),
                    move || Input::SpellPanel(Msg::UndoAdd(d.clone(), w.clone())),
                );
            }
        }
        self.spelling.selected = None;
        self.spell_changed_scope();
    }

    /// Ctrl+F8 / Ctrl+Shift+F8: the next misspelling in the open file; at the end of the file, the next file that has one
    /// (when the panel has scanned the book).
    fn spell_next_mistake(&mut self, forward: bool) {
        self.run_live();
        let spots = self.spell.borrow().spots.clone();
        let Some(path) = self.current.clone() else {
            return;
        };
        // moving back starts from the start of the selected word, moving on from its end
        let caret = {
            let buf = self.w.view.buffer();
            let text = buffer_text(&buf);
            let at = match buf.selection_bounds() {
                Some((a, b)) => {
                    if forward {
                        b.offset()
                    } else {
                        a.offset()
                    }
                }
                None => buf.cursor_position(),
            };
            char_to_byte_idx(&text, at as usize)
        };
        let at_edge = match (forward, spots.last(), spots.first()) {
            (true, Some(l), _) => caret > l.start,
            (false, _, Some(f)) => caret <= f.start,
            _ => true,
        };
        if at_edge {
            if let Some((file, a, b)) = self.adjacent_misspelling(&path, forward) {
                self.spell_show(&file, a, b, true);
                return;
            }
        }
        match editor_spell::pick_spot(&spots, caret, forward) {
            Some(i) => {
                let s = spots[i].clone();
                self.spell_show(&path, s.start, s.end, true);
            }
            None => self.toast(&tr("No misspelled words in this file")),
        }
    }

    /// The first (or last) misspelling of the next (or previous) file with one, from the last scan.
    fn adjacent_misspelling(&self, current: &str, forward: bool) -> Option<(String, usize, usize)> {
        let words = self.spelling.words.as_ref()?;
        let cur = words.files.iter().position(|f| f == current)?;
        let mut per_file: Vec<Option<(usize, usize)>> = vec![None; words.files.len()];
        for e in words.misspelled() {
            for o in words.occurrences(e) {
                let slot = &mut per_file[o.file];
                let better = match (*slot, forward) {
                    (None, _) => true,
                    (Some((s, _)), true) => o.start < s,
                    (Some((s, _)), false) => o.start > s,
                };
                if better {
                    *slot = Some((o.start, o.end));
                }
            }
        }
        let n = words.files.len();
        (1..n).find_map(|k| {
            let i = if forward {
                (cur + k) % n
            } else {
                (cur + n - k) % n
            };
            per_file[i].map(|(a, b)| (words.files[i].clone(), a, b))
        })
    }

    fn spell_excluded_dialog(&mut self) {
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Files Left Out of the Spell Check"));
        dialog.set_content_width(420);
        dialog.set_content_height(360);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let excluded: Vec<String> = self.spell.borrow().prefs.excluded.iter().cloned().collect();
        for p in &excluded {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(p))
                .build();
            let b = icon_btn(
                "list-remove-symbolic",
                &trf("Check {} again", &[&file_name(p)]),
            );
            b.set_valign(gtk::Align::Center);
            let (tx, p2, d) = (
                self.sender.input_sender().clone(),
                p.clone(),
                dialog.clone(),
            );
            b.connect_clicked(move |_| {
                tx.emit(Input::SpellPanel(Msg::Include(p2.clone())));
                d.close();
            });
            row.add_suffix(&b);
            list.append(&row);
        }
        let body: gtk::Widget = if excluded.is_empty() {
            let s = adw::StatusPage::new();
            s.set_icon_name(Some("tools-check-spelling-symbolic"));
            s.set_title(&tr("No Files Left Out"));
            s.set_description(Some(
                &tr("Add the comment <!-- bookx-no-spell-check --> to a file, or use “Exclude This File” in the panel menu."),
            ));
            s.upcast()
        } else {
            let sc = gtk::ScrolledWindow::new();
            sc.set_child(Some(&list));
            sc.set_margin_start(12);
            sc.set_margin_end(12);
            sc.set_margin_bottom(12);
            sc.upcast()
        };
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&body));
        dialog.set_child(Some(&view));
        dialog.present(Some(&self.w.win));
    }

    /// Dev hooks of the spelling panel (`spell:…`), see build-aux/ui-tests.sh.
    pub(super) fn dev_spelling(&mut self, c: &str) -> bool {
        let Some(c) = c.strip_prefix("spell:") else {
            return false;
        };
        if c == "open" {
            self.spell_panel(Msg::Open);
        } else if c == "close" {
            self.spell_panel(Msg::Close);
        } else if c == "refresh" {
            self.spell_panel(Msg::Refresh);
        } else if c == "f8" || c == "shift-f8" {
            self.spell_panel(Msg::NextMistake(c == "f8"));
        } else if let Some(w) = c.strip_prefix("select=") {
            // `spell:select=word` selects the row of that word (the first language) and jumps like Enter
            let pos = self.spelling.shown.iter().position(|i| {
                self.spelling
                    .words
                    .as_ref()
                    .is_some_and(|ws| ws.entries[*i].word == w)
            });
            match pos {
                Some(n) => {
                    if let Some(r) = self.spelling.ui().list.row_at_index(n as i32) {
                        self.spelling.ui().list.select_row(Some(&r));
                    }
                }
                None => eprintln!("editor_spell_panel: no row for {w}"),
            }
        } else if let Some(w) = c.strip_prefix("jump=") {
            if let Some(n) = self.spelling.shown.iter().position(|i| {
                self.spelling
                    .words
                    .as_ref()
                    .is_some_and(|ws| ws.entries[*i].word == w)
            }) {
                self.spell_panel(Msg::Jump(n));
            }
        } else if let Some(t) = c.strip_prefix("change=") {
            self.spelling.ui().entry.set_text(t);
            self.spell_panel(Msg::Change);
        } else if c == "suggest-first" {
            let first = self
                .spelling
                .ui()
                .sugg
                .row_at_index(0)
                .and_then(|r| r.child())
                .and_downcast::<gtk::Label>()
                .map(|l| l.text().to_string());
            if let Some(s) = first {
                self.spell_panel(Msg::Suggestion(s));
            }
        } else if c == "ignore" {
            self.spell_panel(Msg::Ignore(false));
        } else if c == "ignore-book" {
            self.spell_panel(Msg::Ignore(true));
        } else if let Some(d) = c.strip_prefix("add=") {
            self.spell_panel(Msg::AddTo(d.to_string()));
        } else if let Some(v) = c.strip_prefix("all=") {
            self.spelling.ui().all.set_active(v == "on");
        } else if let Some(v) = c.strip_prefix("bycount=") {
            self.spelling.ui().count.set_active(v == "on");
        } else if let Some(t) = c.strip_prefix("filter=") {
            self.spelling.ui().filter.set_text(t);
        } else if c == "exclude" {
            self.spell_panel(Msg::ExcludeCurrent);
        } else if let Some(p) = c.strip_prefix("include=") {
            self.spell_panel(Msg::Include(p.to_string()));
        } else if c == "excluded-dialog" {
            self.spell_panel(Msg::ManageExcluded);
        } else if c == "manage" {
            self.spell_panel(Msg::ManageDictionaries);
        } else if let Some(r) = c.strip_prefix("new-dict=") {
            self.dev_dict_op("new", r);
        } else if let Some(r) = c.strip_prefix("remove-dict=") {
            self.dev_dict_op("remove", r);
        } else if let Some(r) = c.strip_prefix("import-oxt=") {
            self.dev_dict_op("oxt", r);
        } else if let Some(r) = c.strip_prefix("import-pair=") {
            self.dev_dict_op("pair", r);
        } else if c == "sel" {
            let b = self.w.view.buffer();
            let t = b
                .selection_bounds()
                .map(|(s, e)| b.text(&s, &e, false).to_string())
                .unwrap_or_default();
            eprintln!(
                "editor_spell_sel: file={} text={t}",
                self.current.clone().unwrap_or_default()
            );
        } else if c == "info" {
            let ui = self.spelling.ui();
            eprintln!(
                "editor_spell_ui: placeholder={} detail={}",
                ui.placeholder.is_visible(),
                ui.detail.is_visible()
            );
        } else if c == "dump" {
            self.dev_spell_dump();
        }
        true
    }

    fn dev_spell_dump(&self) {
        let ui = self.spelling.ui();
        let names: Vec<String> = (0..ui.dict_names.n_items())
            .filter_map(|i| ui.dict_names.string(i))
            .map(|s| s.to_string())
            .collect();
        let rows: Vec<String> = self
            .spelling
            .shown
            .iter()
            .filter_map(|i| self.spelling.words.as_ref().map(|w| &w.entries[*i]))
            .map(|e| format!("{}/{}/{}", e.word, e.lang, e.count))
            .collect();
        let sugg: Vec<String> = {
            let mut v = Vec::new();
            let mut i = 0;
            while let Some(l) = ui
                .sugg
                .row_at_index(i)
                .and_then(|r| r.child())
                .and_downcast::<gtk::Label>()
            {
                v.push(l.text().to_string());
                i += 1;
            }
            v
        };
        let sh = self.spell.borrow();
        let excluded: Vec<&String> = sh.prefs.excluded.iter().collect();
        eprintln!(
            "editor_spell_panel: open={} scanning={} rows={} words={} sugg={} entry={} dicts={} excluded={} status={}",
            ui.root.reveals_child(),
            self.spelling.scanning,
            rows.len(),
            rows.join("|"),
            sugg.join("|"),
            ui.entry.text(),
            names.join("|"),
            excluded.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("|"),
            ui.status.label()
        );
    }
}

fn live_is_markup(p: &str) -> bool {
    matches!(extension(p).as_str(), "xhtml" | "html" | "htm" | "xht")
}
