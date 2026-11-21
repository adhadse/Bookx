//! The extra controls of the find bar (search mode, whole word, ignore markup, function chooser, history,
//! special characters, the menu with saved searches and functions) and what they do. The basics (find,
//! replace, scopes) are in `search.rs`.

use super::*;
use crate::editor_search::Scope;
use crate::editor_search_fn::{self as sfn, Env, Function, Runner};
use crate::editor_search_modes::Mode;
use crate::editor_search_saved::{self as saved, Action, History, SavedSearch};
use crate::i18n::ntr;
use crate::i18n::{tr, trf};

#[derive(Debug)]
pub enum SearchAct {
    Count,
    ModeChanged,
    Insert(String),
    History(bool, String),
    /// Ask for a name and save the bar's search.
    SaveCurrent,
    SaveNamed(String),
    OpenSaved,
    OpenFunctions,
    /// Load a saved search into the bar.
    Load(Box<SavedSearch>),
    /// Run saved searches in order (a checkpoint first when replacing).
    RunSaved(Vec<SavedSearch>, Action),
    SavedChanged(Vec<SavedSearch>),
    FunctionsChanged(Vec<Function>),
    UseFunction(String),
}

#[derive(Default)]
pub(super) struct SearchState {
    pub history: History,
    pub saved: Vec<SavedSearch>,
    pub user_fns: Vec<Function>,
    /// The text of the last function run's result dialog (for tests).
    pub last_report: String,
    /// How many result windows were shown (for tests).
    pub dialogs: usize,
}

/// The widgets and state added to the find bar.
pub(super) struct Extras {
    pub mode: gtk::DropDown,
    pub word_btn: gtk::ToggleButton,
    pub markup_btn: gtk::ToggleButton,
    pub fn_dd: gtk::DropDown,
    pub fn_list: gtk::StringList,
    pub count_btn: gtk::Button,
    pub find_hist: gtk::MenuButton,
    pub repl_hist: gtk::MenuButton,
    pub special: gtk::MenuButton,
    pub more: gtk::MenuButton,
    pub state: Rc<RefCell<SearchState>>,
    pub replace_entry: gtk::Entry,
    pub dotall_btn: gtk::ToggleButton,
}

fn history_button(
    tooltip: &str,
    is_find: bool,
    state: &Rc<RefCell<SearchState>>,
    tx: &relm4::Sender<Input>,
) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("document-open-recent-symbolic");
    b.set_tooltip_text(Some(tooltip));
    b.add_css_class("flat");
    crate::ui::audit::label(&b, tooltip);
    let pop = gtk::Popover::new();
    b.set_popover(Some(&pop));
    let (state, tx) = (state.clone(), tx.clone());
    let p = pop.clone();
    b.set_create_popup_func(move |_| {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let items = {
            let s = state.borrow();
            if is_find {
                s.history.find.clone()
            } else {
                s.history.replace.clone()
            }
        };
        if items.is_empty() {
            let l = gtk::Label::new(Some(&tr("Nothing searched yet")));
            l.add_css_class("dim-label");
            l.set_margin_all(8);
            list.append(&l);
        }
        for it in items {
            let shown: String = it.replace('\n', "⏎").chars().take(48).collect();
            let btn = gtk::Button::with_label(&shown);
            btn.add_css_class("flat");
            if let Some(l) = btn.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (tx, p) = (tx.clone(), p.clone());
            btn.connect_clicked(move |_| {
                tx.emit(Input::Search(SearchAct::History(is_find, it.clone())));
                p.popdown();
            });
            list.append(&btn);
        }
        let sc = gtk::ScrolledWindow::new();
        sc.set_max_content_height(300);
        sc.set_propagate_natural_height(true);
        sc.set_propagate_natural_width(true);
        sc.set_child(Some(&list));
        p.set_child(Some(&sc));
    });
    b
}

pub(super) fn build(
    tx: &relm4::Sender<Input>,
    replace_entry: &gtk::Entry,
    dotall_btn: &gtk::ToggleButton,
) -> Extras {
    let state = Rc::new(RefCell::new(SearchState::default()));
    {
        let mut s = state.borrow_mut();
        if let Some(p) = saved::history_path() {
            s.history = saved::load_history(&p);
        }
        if let Some(p) = saved::path() {
            s.saved = saved::load_from(&p);
        }
        if let Some(p) = sfn::path() {
            s.user_fns = sfn::load_from(&p);
        }
    }
    let mode = gtk::DropDown::from_strings(&Mode::ALL.map(|m| m.label()));
    crate::ui::audit::label(&mode, &tr("Search mode"));
    mode.set_tooltip_text(Some(
        &tr("Text: literal · Regex: regular expression · Function: a script decides each replacement · Tag: elements and attributes"),
    ));
    let word_btn = gtk::ToggleButton::with_label("W");
    word_btn.set_tooltip_text(Some(&tr("Whole words only")));
    crate::ui::audit::label(&word_btn, &tr("Whole words only"));
    let markup_btn = gtk::ToggleButton::with_label("<>");
    markup_btn.set_tooltip_text(Some(&tr(
        "Ignore HTML markup: find text across tags such as <i>",
    )));
    crate::ui::audit::label(&markup_btn, &tr("Ignore HTML markup"));
    let fn_list = gtk::StringList::new(&[]);
    let fn_dd = gtk::DropDown::new(Some(fn_list.clone()), gtk::Expression::NONE);
    crate::ui::audit::label(&fn_dd, &tr("Replacement function"));
    fn_dd.set_visible(false);
    let count_btn = gtk::Button::with_label(&tr("Count"));
    count_btn.set_tooltip_text(Some(&tr("Count the matches in the whole scope")));
    let find_hist = history_button("Earlier searches", true, &state, tx);
    let repl_hist = history_button("Earlier replacements", false, &state, tx);

    let special = gtk::MenuButton::new();
    special.set_icon_name("insert-text-symbolic");
    special.set_tooltip_text(Some(&tr("Insert a special character or escape")));
    special.add_css_class("flat");
    crate::ui::audit::label(&special, &tr("Insert a special character"));
    {
        let pop = gtk::Popover::new();
        special.set_popover(Some(&pop));
        let (tx, p, mode) = (tx.clone(), pop.clone(), mode.clone());
        special.set_create_popup_func(move |_| {
            let regex = Mode::from_index(mode.selected()).is_regex();
            let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for (label, ins, regex_only) in saved::SPECIALS {
                if *regex_only && !regex {
                    continue;
                }
                let b = gtk::Button::with_label(label);
                b.add_css_class("flat");
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_xalign(0.0);
                }
                let (tx, p) = (tx.clone(), p.clone());
                let text = saved::special_text(ins, regex);
                b.connect_clicked(move |_| {
                    tx.emit(Input::Search(SearchAct::Insert(text.clone())));
                    p.popdown();
                });
                col.append(&b);
            }
            p.set_child(Some(&col));
        });
    }

    let more = gtk::MenuButton::new();
    more.set_icon_name("view-more-symbolic");
    more.set_tooltip_text(Some(&tr("Saved searches, functions and more")));
    more.add_css_class("flat");
    crate::ui::audit::label(&more, &tr("More search actions"));
    let menu = gio::Menu::new();
    menu.append(Some(&tr("Saved Searches")), Some("search.saved"));
    menu.append(Some(&tr("Save This Search")), Some("search.save"));
    menu.append(Some(&tr("Functions")), Some("search.functions"));
    menu.append(
        Some(&tr("Replace & Find Previous")),
        Some("search.replace-prev"),
    );
    more.set_menu_model(Some(&menu));
    let actions = gio::SimpleActionGroup::new();
    for (name, act) in [
        ("saved", 0),
        ("save", 1),
        ("functions", 2),
        ("replace-prev", 3),
    ] {
        let a = gio::SimpleAction::new(name, None);
        let tx = tx.clone();
        a.connect_activate(move |_, _| match act {
            0 => tx.emit(Input::Search(SearchAct::OpenSaved)),
            1 => tx.emit(Input::Search(SearchAct::SaveCurrent)),
            2 => tx.emit(Input::Search(SearchAct::OpenFunctions)),
            _ => tx.emit(Input::Replace(false)),
        });
        actions.add_action(&a);
    }
    more.insert_action_group("search", Some(&actions));

    {
        let t = tx.clone();
        mode.connect_selected_notify(move |_| t.emit(Input::Search(SearchAct::ModeChanged)));
        for b in [&word_btn, &markup_btn] {
            let t = tx.clone();
            b.connect_toggled(move |_| t.emit(Input::FindChanged));
        }
        let t = tx.clone();
        count_btn.connect_clicked(move |_| t.emit(Input::Search(SearchAct::Count)));
        fn_dd.connect_selected_notify(|_| {});
    }

    dotall_btn.set_visible(false);
    let ex = Extras {
        mode,
        word_btn,
        markup_btn,
        fn_dd,
        fn_list,
        count_btn,
        find_hist,
        repl_hist,
        special,
        more,
        state,
        replace_entry: replace_entry.clone(),
        dotall_btn: dotall_btn.clone(),
    };
    ex.refresh_functions();
    ex
}

impl Extras {
    /// The names of the built-in and saved functions, in list order.
    pub fn function_names(&self) -> Vec<String> {
        sfn::merged(&self.state.borrow().user_fns)
            .into_iter()
            .map(|f| f.name)
            .collect()
    }

    pub fn refresh_functions(&self) {
        let keep = self.chosen_function();
        let names = self.function_names();
        self.fn_list.splice(
            0,
            self.fn_list.n_items(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        if let Some(k) = keep {
            if let Some(i) = names.iter().position(|n| *n == k) {
                self.fn_dd.set_selected(i as u32);
            }
        }
    }

    pub fn chosen_function(&self) -> Option<String> {
        self.fn_dd
            .selected_item()
            .and_downcast::<gtk::StringObject>()
            .map(|s| s.string().to_string())
    }

    pub fn function(&self, name: &str) -> Option<Function> {
        sfn::merged(&self.state.borrow().user_fns)
            .into_iter()
            .find(|f| f.name == name)
    }

    pub fn save_all(&self) {
        let s = self.state.borrow();
        if let Some(p) = saved::path() {
            let _ = saved::save_to(&p, &s.saved);
        }
        if let Some(p) = sfn::path() {
            let _ = sfn::save_to(&p, &s.user_fns);
        }
        if let Some(p) = saved::history_path() {
            let _ = saved::save_history(&p, &s.history);
        }
    }
}

impl Editor {
    /// Which controls make sense in the chosen mode.
    pub(super) fn mode_changed(&mut self) {
        let mode = Mode::from_index(self.w.sx.mode.selected());
        let sx = &self.w.sx;
        sx.replace_entry.set_visible(mode != Mode::Function);
        sx.repl_hist.set_visible(mode != Mode::Function);
        sx.fn_dd.set_visible(mode == Mode::Function);
        sx.dotall_btn.set_visible(mode.is_regex());
        sx.word_btn.set_visible(mode != Mode::Tag);
        sx.markup_btn
            .set_visible(matches!(mode, Mode::Normal | Mode::Regex));
        sx.replace_entry.set_placeholder_text(Some(match mode {
            Mode::Tag => "attr=value · -attr · =new text for the matched part",
            _ => "Replace with",
        }));
        self.w.find_entry.set_placeholder_text(Some(match mode {
            Mode::Tag => "Tag query, e.g. img[alt=\"\"]",
            Mode::Function => "Regular expression (the function decides the replacement)",
            Mode::Fuzzy => "Find (small typing mistakes still match)",
            _ => "Find",
        }));
        if mode == Mode::Function {
            sx.refresh_functions();
            self.use_function_find();
        }
        self.find_changed();
    }

    /// Function mode with an empty Find box: fill in the function's suggested expression.
    fn use_function_find(&mut self) {
        if !self.w.find_entry.text().is_empty() {
            return;
        }
        if let Some(f) = self
            .w
            .sx
            .chosen_function()
            .and_then(|n| self.w.sx.function(&n))
        {
            if !f.find.is_empty() {
                self.w.find_entry.set_text(&f.find);
            }
        }
    }

    pub(super) fn remember_search(&mut self) {
        let (f, r) = (
            self.w.find_entry.text().to_string(),
            self.w.replace_entry.text().to_string(),
        );
        {
            let mut s = self.w.sx.state.borrow_mut();
            saved::remember(&mut s.history.find, &f);
            saved::remember(&mut s.history.replace, &r);
        }
        self.w.sx.save_all();
    }

    pub(super) fn search_act(&mut self, act: SearchAct) {
        match act {
            SearchAct::Count => self.count_all(),
            SearchAct::ModeChanged => self.mode_changed(),
            SearchAct::Insert(t) => {
                let into_replace = self.w.replace_entry.has_focus()
                    || self.w.replace_entry.focus_child().is_some();
                let e: gtk::Widget = if into_replace {
                    self.w.replace_entry.clone().upcast()
                } else {
                    self.w.find_entry.clone().upcast()
                };
                if let Some(ed) = e.dynamic_cast_ref::<gtk::Editable>() {
                    let mut pos = ed.position();
                    ed.insert_text(&t, &mut pos);
                    e.grab_focus();
                    ed.select_region(pos, pos);
                }
            }
            SearchAct::History(find, t) => {
                if find {
                    self.w.find_entry.set_text(&t);
                    self.w.find_entry.grab_focus();
                } else {
                    self.w.replace_entry.set_text(&t);
                    self.w.replace_entry.grab_focus();
                }
            }
            SearchAct::SaveCurrent => self.ask_save_name(),
            SearchAct::SaveNamed(name) => self.save_current_as(&name),
            SearchAct::OpenSaved => self.open_saved_dialog(),
            SearchAct::OpenFunctions => self.open_functions_dialog(),
            SearchAct::Load(s) => self.load_saved(&s),
            SearchAct::RunSaved(list, action) => self.run_saved(list, action),
            SearchAct::SavedChanged(list) => {
                self.w.sx.state.borrow_mut().saved = list;
                self.w.sx.save_all();
            }
            SearchAct::FunctionsChanged(list) => {
                self.w.sx.state.borrow_mut().user_fns = list;
                self.w.sx.save_all();
                self.w.sx.refresh_functions();
            }
            SearchAct::UseFunction(name) => {
                let names = self.w.sx.function_names();
                self.w.sx.mode.set_selected(Mode::Function.index());
                self.w.sx.refresh_functions();
                if let Some(i) = names.iter().position(|n| *n == name) {
                    self.w.sx.fn_dd.set_selected(i as u32);
                }
                if let Some(f) = self.w.sx.function(&name) {
                    if !f.find.is_empty() {
                        self.w.find_entry.set_text(&f.find);
                    }
                }
                self.w.find_rev.set_reveal_child(true);
                self.mode_changed();
            }
        }
    }

    // ---- extra highlight for modes GtkSourceView cannot search ---------------------------------

    /// Tag search and "ignore markup" are highlighted by the editor itself.
    pub(super) fn own_highlight(&self) -> bool {
        let st = self.settings();
        !st.find.is_empty() && (matches!(st.mode, Mode::Tag | Mode::Fuzzy) || st.ignore_markup)
    }

    pub(super) fn update_own_highlight(&self) {
        let buf = self.w.view.buffer();
        let table = buf.tag_table();
        let tag = table.lookup("find-extra").unwrap_or_else(|| {
            let t = gtk::TextTag::new(Some("find-extra"));
            t.set_background_rgba(Some(&gdk::RGBA::new(1.0, 0.78, 0.0, 0.35)));
            table.add(&t);
            t
        });
        let (s, e) = buf.bounds();
        buf.remove_tag(&tag, &s, &e);
        if !self.own_highlight() {
            return;
        }
        let Some(re) = self.pattern() else { return };
        let text = buffer_text(&buf);
        for (a, b) in re.find_all(&text, None).into_iter().take(3000) {
            let ia = buf.iter_at_offset(search::byte_to_char_idx(&text, a) as i32);
            let ib = buf.iter_at_offset(search::byte_to_char_idx(&text, b) as i32);
            buf.apply_tag(&tag, &ia, &ib);
        }
    }

    // ---- saved searches ---------------------------------------------------------------------------

    fn ask_save_name(&mut self) {
        let st = self.settings();
        if st.find.is_empty() {
            self.toast(&tr("Type something in Find first"));
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&tr("Save This Search")),
            Some(&tr("Give the search a name.")),
        );
        let entry = gtk::Entry::new();
        entry.set_text(&{
            let t: String = st.find.chars().take(24).collect();
            t
        });
        entry.set_activates_default(true);
        crate::ui::audit::label(&entry, &tr("Name of the saved search"));
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", &tr("Cancel")), ("ok", &tr("Save"))]);
        dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("ok"));
        dialog.set_close_response("cancel");
        let tx = self.sender.input_sender().clone();
        dialog.connect_response(None, move |_, r| {
            if r == "ok" {
                tx.emit(Input::Search(SearchAct::SaveNamed(
                    entry.text().to_string(),
                )));
            }
        });
        dialog.present(Some(&self.w.win));
    }

    /// The search bar as a saved search.
    pub(super) fn current_saved(&self, name: &str) -> SavedSearch {
        let mut s = SavedSearch::new(name, self.settings(), self.scope());
        if s.settings.mode == Mode::Function {
            s.function = self.w.sx.chosen_function().unwrap_or_default();
        }
        s
    }

    fn save_current_as(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let s = self.current_saved(name);
        {
            let mut st = self.w.sx.state.borrow_mut();
            saved::upsert(&mut st.saved, s);
        }
        self.w.sx.save_all();
        self.toast(&trf("Saved search “{}”", &[&name]));
    }

    fn load_saved(&mut self, s: &SavedSearch) {
        let sx = &self.w.sx;
        sx.mode.set_selected(s.settings.mode.index());
        self.w.case_btn.set_active(s.settings.case_sensitive);
        self.w.dotall_btn.set_active(s.settings.dot_all);
        sx.word_btn.set_active(s.settings.whole_word);
        sx.markup_btn.set_active(s.settings.ignore_markup);
        let idx = Scope::ALL.iter().position(|x| *x == s.scope).unwrap_or(0);
        self.w.scope.set_selected(idx as u32);
        self.w.find_rev.set_reveal_child(true);
        self.w.find_entry.set_text(&s.settings.find);
        self.w.replace_entry.set_text(&s.settings.replace);
        if s.settings.mode == Mode::Function {
            sx.refresh_functions();
            if let Some(i) = sx.function_names().iter().position(|n| *n == s.function) {
                sx.fn_dd.set_selected(i as u32);
            }
        }
        self.mode_changed();
    }

    /// Runs saved searches in order on the texts of the whole book. Replacing takes a checkpoint first.
    fn run_saved(&mut self, list: Vec<SavedSearch>, action: Action) {
        if list.is_empty() {
            self.toast(&tr("Choose at least one search"));
            return;
        }
        self.flush();
        let all = self.book.borrow().file_list();
        let mut texts: Vec<(String, String)> = Vec::new();
        for f in all
            .iter()
            .filter(|f| is_text_path(f) && f.as_str() != "mimetype")
        {
            let t = self.text_of(f);
            texts.push((f.clone(), t));
        }
        let before = texts.clone();
        let selected = self.selected_paths();
        let open = self.tab_paths();
        let spine = self.book.borrow().spine_paths();
        let functions = sfn::merged(&self.w.sx.state.borrow().user_fns);
        let env = self.fn_env();
        let current = self.current.clone();
        let reports = {
            let mut book = saved::Book {
                texts: &mut texts,
                current: current.as_deref(),
                selected: &selected,
                open: &open,
                spine: &spine,
            };
            saved::run_sequence(&list, action, &mut book, &functions, &env)
        };
        let mut changed = 0;
        if action == Action::ReplaceAll {
            let edits: Vec<(String, String)> = texts
                .iter()
                .zip(before.iter())
                .filter(|(a, b)| a.1 != b.1)
                .map(|(a, _)| a.clone())
                .collect();
            if !edits.is_empty() {
                self.create_checkpoint("Before: Saved searches");
                for (f, new) in &edits {
                    let buf = self.buffer_for(f);
                    buf.begin_user_action();
                    let (mut s, mut e) = buf.bounds();
                    buf.delete(&mut s, &mut e);
                    let mut s = buf.start_iter();
                    buf.insert(&mut s, new);
                    buf.end_user_action();
                }
                changed = edits.len();
            }
        }
        let verb = if action == Action::Count {
            "matches"
        } else {
            "replaced"
        };
        let mut text = String::new();
        for r in &reports {
            let n = if action == Action::Count {
                r.matches
            } else {
                r.replaced
            };
            text.push_str(&format!(
                "{}: {n} {verb} in {} file(s)",
                r.name,
                if action == Action::Count {
                    r.files_with_matches
                } else {
                    r.files_changed
                }
            ));
            if let Some(e) = &r.error {
                text.push_str(&format!(" · error: {e}"));
            }
            text.push('\n');
            for o in &r.output {
                text.push_str(&format!("    {o}\n"));
            }
            if let Some(f) = &r.final_output {
                text.push_str(&format!("    result: {f}\n"));
            }
        }
        self.w.sx.state.borrow_mut().last_report = text.clone();
        if action == Action::ReplaceAll {
            self.toast(&trf(
                "Ran {}: {} changed",
                &[
                    &ntr("{} search", "{} searches", list.len()),
                    &ntr("{} file", "{} files", changed),
                ],
            ));
        }
        self.show_text_dialog("Saved searches", &text);
    }

    pub(super) fn show_text_dialog(&self, title: &str, text: &str) {
        let dialog = adw::Dialog::new();
        dialog.set_title(title);
        dialog.set_content_width(520);
        dialog.set_content_height(360);
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_left_margin(10);
        view.set_top_margin(10);
        view.buffer().set_text(text);
        crate::ui::audit::label(&view, &tr("Results"));
        let sc = gtk::ScrolledWindow::new();
        sc.set_child(Some(&view));
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&sc));
        dialog.set_child(Some(&tb));
        dialog.present(Some(&self.w.win));
    }

    // ---- function mode ---------------------------------------------------------------------------

    /// What a function can read about the book, with the spelling dictionary behind `dict_recognized`.
    pub(super) fn fn_env(&self) -> Env {
        self.ensure_spell();
        let mut env = Env::from_metadata(self.book.borrow().metadata());
        let spell = self.spell.clone();
        env.dict = Some(Rc::new(move |w: &str| {
            spell
                .borrow()
                .state
                .as_ref()
                .is_some_and(|s| s.recognized(w))
        }));
        env.functions = sfn::merged(&self.w.sx.state.borrow().user_fns);
        env
    }

    /// Where the `data` of a persistent function is kept for this library.
    fn fn_data_file(&self, name: &str) -> Option<std::path::PathBuf> {
        let dir = sfn::data_dir()?;
        let lib = self
            .shared
            .cfg
            .borrow()
            .library_dir
            .to_string_lossy()
            .to_string();
        Some(sfn::data_file(&dir, &lib, name))
    }

    fn save_fn_data(&self, r: &Runner) {
        if r.func.persistent_data {
            if let Some(p) = self.fn_data_file(&r.func.name) {
                let _ = sfn::save_data(&p, &r.data_map());
            }
        }
    }

    /// A runner for the function chosen in the bar.
    pub(super) fn make_runner(&mut self) -> Result<Runner, String> {
        let name = self
            .w
            .sx
            .chosen_function()
            .ok_or("Choose a function in the Function list")?;
        let f = self
            .w
            .sx
            .function(&name)
            .ok_or_else(|| trf("The function “{}” does not exist", &[&name]))?;
        let mut r = Runner::new(&f, &self.fn_env()).map_err(|e| e.to_string())?;
        if f.persistent_data {
            if let Some(p) = self.fn_data_file(&f.name) {
                r.set_data(sfn::load_data(&p));
            }
        }
        Ok(r)
    }

    /// The result dialog after a function run: what it printed, what the last call returned.
    pub(super) fn finish_fn_run(
        &mut self,
        r: &Runner,
        final_output: Option<String>,
        total: usize,
        files: usize,
    ) {
        let mut text = trf(
            "{} in {}\n",
            &[
                &ntr("{} replacement", "{} replacements", total),
                &ntr("{} file", "{} files", files),
            ],
        );
        if let Some(f) = &final_output {
            text.push_str(&format!("\nFinal output:\n{f}\n"));
        }
        let out = r.output();
        if !out.is_empty() {
            text.push_str("\nDebug output:\n");
            for l in &out {
                text.push_str(l);
                text.push('\n');
            }
        }
        self.w.sx.state.borrow_mut().last_report = text.clone();
        self.save_fn_data(r);
        if r.func.append_final_output_to_marked {
            if let Some(f) = &final_output {
                self.append_final_to_marked(f);
            }
        }
        if r.func.suppress_result_dialog {
            return;
        }
        if final_output.is_some() || !out.is_empty() {
            self.show_fn_dialog(&text, final_output.as_deref());
        }
    }

    /// Shows a failed run: the error with its line number, and the output until then.
    pub(super) fn show_fn_result(&mut self, r: Option<&Runner>, err: Option<String>) {
        let mut text = String::new();
        if let Some(e) = &err {
            text.push_str(&format!("Error: {e}\n"));
        }
        if let Some(r) = r {
            let out = r.output();
            if !out.is_empty() {
                text.push_str("\nDebug output:\n");
                for l in &out {
                    text.push_str(l);
                    text.push('\n');
                }
            }
        }
        self.w.sx.state.borrow_mut().last_report = text.clone();
        self.show_fn_dialog(&text, None);
    }

    fn show_fn_dialog(&self, text: &str, copy: Option<&str>) {
        self.w.sx.state.borrow_mut().dialogs += 1;
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Function results"));
        dialog.set_content_width(520);
        dialog.set_content_height(380);
        let view = gtk::TextView::new();
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_left_margin(10);
        view.set_top_margin(10);
        view.buffer().set_text(text);
        crate::ui::audit::label(&view, &tr("Function results and debug output"));
        let sc = gtk::ScrolledWindow::new();
        sc.set_child(Some(&view));
        let tb = adw::ToolbarView::new();
        let hb = adw::HeaderBar::new();
        if let Some(c) = copy {
            let b = gtk::Button::with_label(&tr("Copy Final Output"));
            let c = c.to_string();
            b.connect_clicked(move |b| b.clipboard().set_text(&c));
            hb.pack_start(&b);
        }
        tb.add_top_bar(&hb);
        tb.set_content(Some(&sc));
        dialog.set_child(Some(&tb));
        dialog.present(Some(&self.w.win));
    }

    // ---- dev hooks ----------------------------------------------------------------------------

    pub(super) fn dev_search(&mut self, c: &str) {
        let (cmd, arg) = c.split_once('=').unwrap_or((c, ""));
        let sx = self.w.sx.state.clone();
        match cmd {
            "mode" => {
                let m = match arg {
                    "regex" => Mode::Regex,
                    "function" => Mode::Function,
                    "tag" => Mode::Tag,
                    "fuzzy" => Mode::Fuzzy,
                    _ => Mode::Normal,
                };
                self.w.find_rev.set_reveal_child(true);
                self.w.sx.mode.set_selected(m.index());
                self.mode_changed();
            }
            "opt" => {
                let (name, on) = match arg.strip_prefix("no-") {
                    Some(n) => (n, false),
                    None => (arg, true),
                };
                match name {
                    "word" => self.w.sx.word_btn.set_active(on),
                    "markup" => self.w.sx.markup_btn.set_active(on),
                    "case" => self.w.case_btn.set_active(on),
                    "dotall" => self.w.dotall_btn.set_active(on),
                    _ => {}
                }
            }
            "scope" => {
                let s = match arg {
                    "text" => Scope::TextFiles,
                    "style" => Scope::StyleFiles,
                    "all" => Scope::AllFiles,
                    "selected" => Scope::SelectedFiles,
                    "open" => Scope::OpenFiles,
                    "marked" => Scope::MarkedText,
                    _ => Scope::ThisFile,
                };
                let i = Scope::ALL.iter().position(|x| *x == s).unwrap_or(0);
                self.w.scope.set_selected(i as u32);
            }
            "find" => {
                self.w.find_rev.set_reveal_child(true);
                self.w.find_entry.set_text(&arg.replace("\\n", "\n"));
            }
            "replace" => self.w.replace_entry.set_text(&arg.replace("\\n", "\n")),
            "next" => self.find_next(true),
            "prev" => self.find_next(false),
            "count" => self.count_all(),
            "replace-all" => self.replace_all(),
            "replace-find" => self.replace_one(true),
            "replace-find-prev" => self.replace_one(false),
            "special" => {
                if let Some((label, ins, _)) = saved::SPECIALS.get(arg.parse::<usize>().unwrap_or(0)) {
                    let regex = Mode::from_index(self.w.sx.mode.selected()).is_regex();
                    let _ = label;
                    self.search_act(SearchAct::Insert(saved::special_text(ins, regex)));
                }
            }
            "status" => eprintln!(
                "editor_search: status={:?} mode={} find={:?} replace={:?} fn_visible={} replace_visible={} dotall_visible={} markup_visible={} scope={} hits={}",
                self.w.find_status.label(),
                Mode::from_index(self.w.sx.mode.selected()).label(),
                self.w.find_entry.text(),
                self.w.replace_entry.text(),
                self.w.sx.fn_dd.get_visible(),
                WidgetExt::is_visible(&self.w.replace_entry),
                self.w.dotall_btn.get_visible(),
                self.w.sx.markup_btn.get_visible(),
                self.scope().label(),
                self.own_hits()
            ),
            "has" => {
                // `path|needle`: how often the needle occurs in that file's current text
                let (path, needle) = arg.split_once('|').unwrap_or((arg, ""));
                let t = self.text_of(path);
                eprintln!("editor_search_has: {path} {needle:?} = {}", t.matches(needle).count());
            }
            "sel" => {
                let buf = self.w.view.buffer();
                let t = buf
                    .selection_bounds()
                    .map(|(a, b)| buf.text(&a, &b, false).to_string())
                    .unwrap_or_default();
                eprintln!("editor_search_sel: {t:?}");
            }
            "history" => {
                let s = sx.borrow();
                eprintln!(
                    "editor_history: find={:?} replace={:?}",
                    s.history.find, s.history.replace
                );
            }
            "history-use" => {
                let t = sx.borrow().history.find.get(arg.parse().unwrap_or(0)).cloned();
                if let Some(t) = t {
                    self.search_act(SearchAct::History(true, t));
                }
            }
            "save" => self.save_current_as(arg),
            "saved" => {
                let s = sx.borrow();
                for (i, x) in s.saved.iter().enumerate() {
                    eprintln!("editor_saved: {i} | {} | {}", x.name, x.summary());
                }
                eprintln!("editor_saved: count={}", s.saved.len());
            }
            "saved-open" => self.open_saved_dialog(),
            "saved-load" => {
                let s = sx.borrow().saved.get(arg.parse().unwrap_or(0)).cloned();
                if let Some(s) = s {
                    self.load_saved(&s);
                }
            }
            "saved-run" => {
                // `0,1|count` or `0|replace`
                let (idx, act) = arg.split_once('|').unwrap_or((arg, "count"));
                let list: Vec<SavedSearch> = {
                    let s = sx.borrow();
                    idx.split([' ', '+'])
                        .filter_map(|i| i.parse::<usize>().ok().and_then(|i| s.saved.get(i).cloned()))
                        .collect()
                };
                let action = if act == "replace" { Action::ReplaceAll } else { Action::Count };
                self.run_saved(list, action);
            }
            "saved-delete" => {
                let mut s = sx.borrow_mut();
                let i = arg.parse::<usize>().unwrap_or(usize::MAX);
                if i < s.saved.len() {
                    s.saved.remove(i);
                }
                drop(s);
                self.w.sx.save_all();
            }
            "saved-move" => {
                let (i, d) = arg.split_once('|').unwrap_or((arg, "1"));
                let mut s = sx.borrow_mut();
                saved::move_item(&mut s.saved, i.parse().unwrap_or(0), d.parse().unwrap_or(1));
                drop(s);
                self.w.sx.save_all();
            }
            "saved-export" => {
                let json = saved::to_json(&sx.borrow().saved);
                let _ = std::fs::write(arg, json);
            }
            "saved-import" => {
                let imported = std::fs::read_to_string(arg)
                    .map_err(|e| e.to_string())
                    .and_then(|t| saved::parse_json(&t));
                match imported {
                    Ok(list) => {
                        let n = saved::merge_imported(&mut sx.borrow_mut().saved, list);
                        self.w.sx.save_all();
                        eprintln!("editor_saved: imported={n}");
                    }
                    Err(e) => eprintln!("editor_saved: import error={e}"),
                }
            }
            "fn-add" => {
                // `Name|find|source` (`\n` is a line break)
                let p: Vec<&str> = arg.splitn(3, '|').collect();
                if let [name, find, src] = p[..] {
                    let mut f = Function::new(name, find, &src.replace("\\n", "\n").replace('§', ","));
                    f.description = "Test function".into();
                    let mut s = sx.borrow_mut();
                    s.user_fns.retain(|x| x.name != name);
                    s.user_fns.push(f);
                    drop(s);
                    self.w.sx.save_all();
                    self.w.sx.refresh_functions();
                }
            }
            "fn-use" => self.search_act(SearchAct::UseFunction(arg.to_string())),
            "fn-list" => eprintln!("editor_fns: {:?}", self.w.sx.function_names()),
            "fn-report" => eprintln!(
                "editor_fn_report: dialogs={} {}",
                sx.borrow().dialogs,
                sx.borrow().last_report.replace('\n', "⏎")
            ),
            "fn-open" => self.open_functions_dialog(),
            "fn-edit" => self.edit_function_dialog(arg),
            "fn-export" => {
                let json = sfn::to_json(&sx.borrow().user_fns);
                let _ = std::fs::write(arg, json);
            }
            _ => {}
        }
    }

    fn own_hits(&self) -> usize {
        let buf = self.w.view.buffer();
        let Some(tag) = buf.tag_table().lookup("find-extra") else {
            return 0;
        };
        let (mut it, end) = buf.bounds();
        let mut n = 0;
        while it < end {
            if it.starts_tag(Some(&tag)) {
                n += 1;
            }
            if !it.forward_to_tag_toggle(Some(&tag)) {
                break;
            }
        }
        n
    }
}
