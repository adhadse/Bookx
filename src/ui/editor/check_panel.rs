//! The *Problems* panel of the editor (Check Book, CE-160..CE-165): a list grouped by file with severity icons and
//! count badges, a help pane for the selected problem, per-problem *Fix* and *Fix all of this kind* (always after a
//! checkpoint), *Ignore this kind* with a list to restore, next/previous problem and tracking of stale entries.
//! The decisions live in `bookx_editing::problems` (pure, tested there); this file is the widgets and the glue.

use crate::i18n::{tr, trf};
use std::collections::{BTreeSet, HashSet};

use bookx_editing::problems::{self, CheckOptions, Problem, Scope};
use bookx_editing::validate::Severity as VSeverity;

use super::*;

/// What the panel's widgets ask the editor to do.
#[derive(Debug, Clone)]
pub enum Act {
    /// The row with this index in the list box was selected (-1: none).
    Select(i32),
    /// Enter or double-click on a row.
    Open(i32),
    Run,
    Fix,
    FixKind,
    Ignore,
    Restore(String),
    RestoreAll,
    Next,
    Prev,
    Accessibility(bool),
    Close,
}

/// The widgets of the panel.
pub struct Widgets {
    pub root: gtk::Revealer,
    list: gtk::ListBox,
    title: gtk::Label,
    errors: gtk::Label,
    warnings: gtk::Label,
    notes: gtk::Label,
    a11y: gtk::CheckButton,
    body_stack: gtk::Stack,
    head: gtk::Box,
    ignored_btn: gtk::MenuButton,
    ignored_box: gtk::Box,
    help_title: gtk::Label,
    help_meaning: gtk::Label,
    help_why: gtk::Label,
    help_how: gtk::Label,
    help_where: gtk::Label,
    fix_btn: gtk::Button,
    fix_all_btn: gtk::Button,
    ignore_btn: gtk::Button,
}

/// What the editor remembers about the last check.
pub struct State {
    /// The last result, before ignored kinds are removed.
    all: Vec<Problem>,
    /// What the list shows.
    list: Vec<Problem>,
    /// For each row of the list box: the index into `list` (`None` for a file heading).
    rows: Vec<Option<usize>>,
    selected: Option<usize>,
    /// Files edited since the check: their entries may be out of date.
    stale_files: HashSet<String>,
    hidden: usize,
    ignored: BTreeSet<String>,
    accessibility: bool,
    ran: bool,
}

impl State {
    pub fn new(prefs: &crate::editor_prefs::EditorPrefs, book_id: u64) -> State {
        State {
            all: Vec::new(),
            list: Vec::new(),
            rows: Vec::new(),
            selected: None,
            stale_files: HashSet::new(),
            hidden: 0,
            ignored: prefs
                .ignored_checks
                .get(&book_id.to_string())
                .cloned()
                .unwrap_or_default(),
            accessibility: prefs.check_accessibility,
            ran: false,
        }
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_selectable(true);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

fn badge(icon: &str, class: &str, name: &str) -> (gtk::Box, gtk::Label) {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class(class);
    let l = gtk::Label::new(Some("0"));
    l.add_css_class("numeric");
    b.append(&img);
    b.append(&l);
    b.update_property(&[gtk::accessible::Property::Label(name)]);
    (b, l)
}

fn severity_look(s: VSeverity) -> (&'static str, &'static str) {
    match s {
        VSeverity::Error => ("dialog-error-symbolic", "issue-error"),
        VSeverity::Warning => ("dialog-warning-symbolic", "issue-warning"),
        _ => ("dialog-information-symbolic", "issue-info"),
    }
}

fn severity_word(s: VSeverity) -> &'static str {
    match s {
        VSeverity::Error => "error",
        VSeverity::Warning => "warning",
        _ => "note",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Builds the panel; its buttons send [`Input::Problems`].
pub fn build(sender: &ComponentSender<Editor>) -> Widgets {
    let tx = sender.input_sender().clone();
    let emit = Rc::new(move |a: Act| tx.emit(Input::Problems(a)));

    let title = gtk::Label::new(Some(&tr("Problems")));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    let (eb, errors) = badge("dialog-error-symbolic", "issue-error", "Errors");
    let (wb, warnings) = badge("dialog-warning-symbolic", "issue-warning", "Warnings");
    let (nb, notes) = badge("dialog-information-symbolic", "issue-info", "Notes");

    let a11y = gtk::CheckButton::with_label(&tr("Also check accessibility"));
    {
        let emit = emit.clone();
        a11y.connect_toggled(move |c| emit(Act::Accessibility(c.is_active())));
    }

    let ignored_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    ignored_box.set_margin_all(8);
    let pop = gtk::Popover::new();
    let pop_scroll = gtk::ScrolledWindow::new();
    pop_scroll.set_propagate_natural_height(true);
    pop_scroll.set_propagate_natural_width(true);
    pop_scroll.set_max_content_height(320);
    pop_scroll.set_child(Some(&ignored_box));
    let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    a11y.set_margin_all(8);
    pop_box.append(&a11y);
    pop_box.append(&pop_scroll);
    pop.set_child(Some(&pop_box));
    let ignored_btn = gtk::MenuButton::new();
    ignored_btn.set_icon_name("view-more-symbolic");
    ignored_btn.set_tooltip_text(Some(&tr("Problem options: accessibility, ignored kinds")));
    ignored_btn.update_property(&[gtk::accessible::Property::Label(&tr("Problem options"))]);
    ignored_btn.set_popover(Some(&pop));
    ignored_btn.add_css_class("flat");

    let run = icon_button("view-refresh-symbolic", &tr("Check the book again (F7)"));
    run.add_css_class("flat");
    run.update_property(&[gtk::accessible::Property::Label(&tr("Check again"))]);
    let prev = icon_button("go-up-symbolic", &tr("Previous problem (Shift+F8)"));
    prev.add_css_class("flat");
    prev.update_property(&[gtk::accessible::Property::Label(&tr("Previous problem"))]);
    let next = icon_button("go-down-symbolic", &tr("Next problem (F8)"));
    next.add_css_class("flat");
    next.update_property(&[gtk::accessible::Property::Label(&tr("Next problem"))]);
    let close = icon_button("window-close-symbolic", &tr("Close"));
    close.add_css_class("flat");
    close.update_property(&[gtk::accessible::Property::Label(&tr("Close the problems"))]);
    {
        let emit1 = emit.clone();
        run.connect_clicked(move |_| emit1(Act::Run));
        let emit2 = emit.clone();
        prev.connect_clicked(move |_| emit2(Act::Prev));
        let emit3 = emit.clone();
        next.connect_clicked(move |_| emit3(Act::Next));
        let emit4 = emit.clone();
        close.connect_clicked(move |_| emit4(Act::Close));
    }

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.set_margin_all(6);
    head.append(&title);
    head.append(&eb);
    head.append(&wb);
    head.append(&nb);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    head.append(&spacer);
    head.append(&ignored_btn);
    head.append(&prev);
    head.append(&next);
    head.append(&run);
    head.append(&close);

    let list = gtk::ListBox::new();
    // a compact list like the problems panel of a code editor: no cards, no gaps between rows
    list.add_css_class("problems-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.update_property(&[gtk::accessible::Property::Label(&tr(
        "Problems found in the book",
    ))]);
    {
        let emit1 = emit.clone();
        list.connect_row_selected(move |_, r| emit1(Act::Select(r.map_or(-1, |r| r.index()))));
        let emit2 = emit.clone();
        list.connect_row_activated(move |_, r| emit2(Act::Open(r.index())));
    }
    let list_scroll = gtk::ScrolledWindow::new();
    list_scroll.set_min_content_height(100);
    list_scroll.set_max_content_height(100);
    list_scroll.set_hexpand(true);
    list_scroll.set_child(Some(&list));

    let help_title = gtk::Label::new(Some(&tr("Select a problem to read what it means")));
    help_title.add_css_class("heading");
    help_title.set_xalign(0.0);
    help_title.set_wrap(true);
    let help_meaning = label("", "");
    let help_why = label("", "dim-label");
    let help_how = label("", "");
    let help_where = label("", "caption");
    help_where.add_css_class("dim-label");
    let fix_btn = gtk::Button::with_label(&tr("Fix"));
    fix_btn.add_css_class("suggested-action");
    let fix_all_btn = gtk::Button::with_label(&tr("Fix all of this kind"));
    let ignore_btn = gtk::Button::with_label(&tr("Ignore this kind"));
    ignore_btn.add_css_class("flat");
    {
        let emit1 = emit.clone();
        fix_btn.connect_clicked(move |_| emit1(Act::Fix));
        let emit2 = emit.clone();
        fix_all_btn.connect_clicked(move |_| emit2(Act::FixKind));
        let emit3 = emit.clone();
        ignore_btn.connect_clicked(move |_| emit3(Act::Ignore));
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.append(&fix_btn);
    buttons.append(&fix_all_btn);
    buttons.append(&ignore_btn);
    let help = gtk::Box::new(gtk::Orientation::Vertical, 6);
    help.set_margin_all(10);
    help.append(&help_title);
    help.append(&help_where);
    help.append(&help_meaning);
    help.append(&help_why);
    help.append(&help_how);
    let help_scroll = gtk::ScrolledWindow::new();
    help_scroll.set_min_content_height(50);
    help_scroll.set_vexpand(true);
    help_scroll.set_child(Some(&help));
    buttons.set_margin_all(6);
    let help_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    help_card.append(&help_scroll);
    help_card.append(&buttons);
    help_card.set_size_request(340, 100);
    help_card.add_css_class("card");
    help_card.set_margin_all(4);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    body.append(&list_scroll);
    body.append(&help_card);
    let empty = adw::StatusPage::builder()
        .icon_name("object-select-symbolic")
        .title(tr("No Problems Found"))
        .description(tr("The book passed every check."))
        .build();
    empty.add_css_class("compact");
    let again = gtk::Button::with_label(&tr("Check Again"));
    again.add_css_class("pill");
    again.set_halign(gtk::Align::Center);
    {
        let emit = emit.clone();
        again.connect_clicked(move |_| emit(Act::Run));
    }
    empty.set_child(Some(&again));
    let body_stack = gtk::Stack::new();
    body_stack.set_hhomogeneous(false);
    body_stack.set_vhomogeneous(false);
    body_stack.add_named(&body, Some("list"));
    body_stack.add_named(&empty, Some("empty"));
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    outer.append(&head);
    outer.append(&body_stack);
    let root = gtk::Revealer::new();
    root.set_child(Some(&outer));
    Widgets {
        root,
        list,
        title,
        errors,
        warnings,
        notes,
        a11y,
        body_stack,
        head,
        ignored_btn,
        ignored_box,
        help_title,
        help_meaning,
        help_why,
        help_how,
        help_where,
        fix_btn,
        fix_all_btn,
        ignore_btn,
    }
}

fn place(p: &Problem) -> String {
    match (p.line, p.col) {
        (Some(l), Some(c)) => format!("{l}:{c}"),
        (Some(l), None) => l.to_string(),
        _ => String::new(),
    }
}

impl Editor {
    pub(super) fn problems_act(&mut self, act: Act) {
        match act {
            Act::Select(row) => self.problems_select_row(row),
            Act::Open(row) => {
                self.problems_select_row(row);
                self.problems_open_selected();
            }
            Act::Run => self.problems_run(),
            Act::Fix => self.problems_fix(false),
            Act::FixKind => self.problems_fix(true),
            Act::Ignore => self.problems_ignore(),
            Act::Restore(id) => self.problems_restore(Some(&id)),
            Act::RestoreAll => self.problems_restore(None),
            Act::Next => self.problems_step(true),
            Act::Prev => self.problems_step(false),
            Act::Accessibility(on) => {
                if self.problems.accessibility != on {
                    self.problems.accessibility = on;
                    self.prefs.check_accessibility = on;
                    self.prefs.save();
                    if self.problems.ran {
                        self.problems_run();
                    }
                }
            }
            Act::Close => self.w.problems.root.set_reveal_child(false),
        }
    }

    /// How many problems the list shows (for the dev dump).
    pub(super) fn problem_count(&self) -> usize {
        self.problems.list.len()
    }

    fn problems_options(&self) -> CheckOptions {
        CheckOptions {
            accessibility: self.problems.accessibility,
        }
    }

    /// F7: checks the whole book and shows the list. The selected problem stays selected when it is still there.
    pub(super) fn problems_run(&mut self) {
        self.flush();
        let previous = self
            .problems
            .selected
            .and_then(|i| self.problems.list.get(i))
            .cloned();
        let all = problems::collect(&self.book.borrow().archive, &self.problems_options());
        self.problems.all = all;
        self.problems.stale_files.clear();
        self.problems.ran = true;
        self.problems_refilter(previous.as_ref());
        self.w.results_rev.set_reveal_child(false);
        self.w.problems.root.set_reveal_child(true);
        if self.problems.list.is_empty() && self.problems.hidden == 0 {
            self.toast(&tr("Book check passed — no problems found"));
        }
    }

    /// Applies the ignored kinds to the last result and rebuilds the list.
    fn problems_refilter(&mut self, keep: Option<&Problem>) {
        let (list, hidden) =
            problems::without_ignored(self.problems.all.clone(), &self.problems.ignored);
        self.problems.list = list;
        self.problems.hidden = hidden;
        self.problems.selected = keep.and_then(|k| problems::reselect(k, &self.problems.list));
        self.problems_rebuild();
        let issues = self.problems_as_issues();
        self.set_issue_marks(&issues);
    }

    fn problems_as_issues(&self) -> Vec<crate::core::check::Issue> {
        self.problems
            .list
            .iter()
            .map(|p| crate::core::check::Issue {
                severity: match p.severity {
                    VSeverity::Error => Severity::Error,
                    VSeverity::Warning => Severity::Warning,
                    _ => Severity::Info,
                },
                file: p.file.clone(),
                line: p.line,
                message: format!("[{}] {}", p.id, p.text),
            })
            .collect()
    }

    fn problems_rebuild(&mut self) {
        let w = &self.w.problems;
        while let Some(c) = w.list.first_child() {
            w.list.remove(&c);
        }
        self.problems.rows.clear();
        let list = &self.problems.list;
        for (file, idx) in problems::group_by_file(list) {
            let head = gtk::ListBoxRow::new();
            head.set_selectable(false);
            head.set_activatable(false);
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            b.set_margin_top(2);
            b.set_margin_bottom(2);
            b.set_margin_start(8);
            b.set_margin_end(8);
            let name = gtk::Label::new(Some(if file.is_empty() { "The book" } else { &file }));
            name.add_css_class("caption-heading");
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(pango::EllipsizeMode::Start);
            let count = gtk::Label::new(Some(&idx.len().to_string()));
            count.add_css_class("dim-label");
            count.add_css_class("numeric");
            b.append(&name);
            b.append(&count);
            head.set_child(Some(&b));
            head.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} — {}",
                if file.is_empty() { "The book" } else { &file },
                plural(idx.len(), "problem", "problems")
            ))]);
            w.list.append(&head);
            self.problems.rows.push(None);
            for i in idx {
                let p = &list[i];
                let row = gtk::ListBoxRow::new();
                let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                b.set_margin_top(3);
                b.set_margin_bottom(3);
                b.set_margin_end(8);
                b.set_margin_start(20);
                let (icon, class) = severity_look(p.severity);
                let img = gtk::Image::from_icon_name(icon);
                img.add_css_class(class);
                let msg = gtk::Label::new(Some(&p.text));
                msg.set_xalign(0.0);
                msg.set_hexpand(true);
                msg.set_ellipsize(pango::EllipsizeMode::End);
                let at = gtk::Label::new(Some(&place(p)));
                at.add_css_class("dim-label");
                at.add_css_class("numeric");
                let id = gtk::Label::new(Some(&p.id));
                id.add_css_class("dim-label");
                id.add_css_class("caption");
                b.append(&img);
                b.append(&msg);
                b.append(&at);
                b.append(&id);
                row.set_child(Some(&b));
                row.set_tooltip_text(Some(&p.text));
                row.update_property(&[gtk::accessible::Property::Label(&format!(
                    "{}: {}{}",
                    severity_word(p.severity),
                    p.text,
                    if p.line.is_some() {
                        format!(", line {}", p.line.unwrap_or(0))
                    } else {
                        String::new()
                    }
                ))]);
                w.list.append(&row);
                self.problems.rows.push(Some(i));
            }
        }
        let (e, wn, n) = problems::counts(&self.problems.list);
        w.errors.set_label(&e.to_string());
        w.warnings.set_label(&wn.to_string());
        w.notes.set_label(&n.to_string());
        let total = self.problems.list.len();
        w.title.set_label(&if total == 0 {
            "No problems found".to_string()
        } else {
            plural(total, "problem", "problems")
        });
        w.a11y.set_active(self.problems.accessibility);
        w.body_stack.set_visible_child_name(
            if self.problems.ran && total == 0 && self.problems.hidden == 0 {
                "empty"
            } else {
                "list"
            },
        );
        self.problems_rebuild_ignored();
        self.problems_style_stale();
        match self.problems.selected {
            Some(i) => {
                let row = self.problems.rows.iter().position(|r| *r == Some(i));
                if let Some(r) = row.and_then(|r| w.list.row_at_index(r as i32)) {
                    w.list.select_row(Some(&r));
                }
            }
            None => w.list.unselect_all(),
        }
        self.problems_show_help();
    }

    fn problems_rebuild_ignored(&self) {
        let w = &self.w.problems;
        while let Some(c) = w.ignored_box.first_child() {
            w.ignored_box.remove(&c);
        }
        let n = self.problems.ignored.len();
        w.ignored_btn.set_tooltip_text(Some(&if n > 0 {
            trf(
                "{} ignored ({} hidden problems)",
                &[&plural(n, "kind", "kinds"), &self.problems.hidden],
            )
        } else {
            tr("Problem options: accessibility, ignored kinds")
        }));
        if n == 0 {
            return;
        }
        let head = gtk::Label::new(Some(&tr("Ignored kinds")));
        head.add_css_class("heading");
        head.set_xalign(0.0);
        w.ignored_box.append(&head);
        for id in &self.problems.ignored {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(&format!("{} — {}", id, problems::title(id))));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_wrap(true);
            l.set_max_width_chars(44);
            let b = gtk::Button::with_label(&tr("Show again"));
            b.add_css_class("flat");
            b.update_property(&[gtk::accessible::Property::Label(&trf(
                "Show {} again",
                &[&id],
            ))]);
            let tx = self.sender.input_sender().clone();
            let id2 = id.clone();
            b.connect_clicked(move |_| tx.emit(Input::Problems(Act::Restore(id2.clone()))));
            row.append(&l);
            row.append(&b);
            w.ignored_box.append(&row);
        }
        let all = gtk::Button::with_label(&tr("Show all again"));
        let tx = self.sender.input_sender().clone();
        all.connect_clicked(move |_| tx.emit(Input::Problems(Act::RestoreAll)));
        w.ignored_box.append(&all);
    }

    fn problems_selected(&self) -> Option<&Problem> {
        self.problems
            .selected
            .and_then(|i| self.problems.list.get(i))
    }

    fn problems_select_row(&mut self, row: i32) {
        let idx = usize::try_from(row)
            .ok()
            .and_then(|r| self.problems.rows.get(r).copied().flatten());
        if self.problems.selected != idx {
            self.problems.selected = idx;
            self.problems_show_help();
        }
    }

    fn problems_show_help(&self) {
        let w = &self.w.problems;
        let Some(p) = self.problems_selected() else {
            w.help_title
                .set_label(&tr("Select a problem to read what it means"));
            for l in [&w.help_meaning, &w.help_why, &w.help_how, &w.help_where] {
                l.set_label("");
            }
            for b in [&w.fix_btn, &w.fix_all_btn, &w.ignore_btn] {
                b.set_visible(false);
            }
            return;
        };
        w.help_title
            .set_label(&format!("{} · {}", p.id, problems::title(&p.id)));
        let stale = self.problems.stale_files.contains(&p.file);
        w.help_where.set_label(&format!(
            "{}{}{}",
            if p.file.is_empty() {
                "The book"
            } else {
                &p.file
            },
            if p.line.is_some() {
                format!(", line {}", place(p))
            } else {
                String::new()
            },
            if stale {
                " — edited since the check, check again to be sure"
            } else {
                ""
            }
        ));
        match problems::help(&p.id) {
            Some(h) => {
                w.help_meaning.set_markup(&format!(
                    "<b>What it means.</b> {}",
                    glib::markup_escape_text(h.meaning)
                ));
                w.help_why.set_markup(&format!(
                    "<b>Why it matters.</b> {}",
                    glib::markup_escape_text(h.why)
                ));
                w.help_how.set_markup(&format!(
                    "<b>How to fix it.</b> {}",
                    glib::markup_escape_text(h.how)
                ));
            }
            None => {
                w.help_meaning.set_label(&p.text);
                w.help_why.set_label("");
                w.help_how.set_label("");
            }
        }
        match problems::fix_label(&p.id) {
            Some(l) => {
                w.fix_btn.set_label(&tr("Fix"));
                w.fix_btn.set_tooltip_text(Some(l));
                w.fix_btn.set_visible(true);
                let same = self.problems.list.iter().filter(|q| q.id == p.id).count();
                w.fix_all_btn.set_visible(same > 1);
                w.fix_all_btn
                    .set_label(&trf("Fix all {} of this kind", &[&same]));
            }
            None => {
                w.fix_btn.set_visible(false);
                w.fix_all_btn.set_visible(false);
            }
        }
        w.ignore_btn.set_visible(problems::can_ignore(&p.id));
    }

    /// Dims the entries of files edited since the check (CE-165).
    fn problems_style_stale(&self) {
        for (r, idx) in self.problems.rows.iter().enumerate() {
            let Some(row) = self.w.problems.list.row_at_index(r as i32) else {
                continue;
            };
            let stale = idx
                .and_then(|i| self.problems.list.get(i))
                .is_some_and(|p| self.problems.stale_files.contains(&p.file));
            row.set_opacity(if stale { 0.55 } else { 1.0 });
            if stale {
                row.set_tooltip_text(Some(&tr(
                    "This file was edited after the check. Check again.",
                )));
            }
        }
    }

    /// Called when a file's text changed: its entries may no longer be true.
    pub(super) fn problems_mark_stale(&mut self, file: &str) {
        if !self.problems.ran
            || self.problems.stale_files.contains(file)
            || !self.problems.list.iter().any(|p| p.file == file)
        {
            return;
        }
        self.problems.stale_files.insert(file.to_string());
        self.problems_style_stale();
        self.problems_show_help();
    }

    fn problems_open_selected(&mut self) {
        let Some(p) = self.problems_selected().cloned() else {
            return;
        };
        if p.file.is_empty() || !self.book.borrow().archive.contains(&p.file) {
            self.toast(&tr(
                "This problem is about the book as a whole, not one file",
            ));
            return;
        }
        self.select_file(&p.file);
        if let Some(l) = p.line.filter(|_| is_text_path(&p.file)) {
            let buf = self.w.view.buffer();
            if let Some(mut it) = buf.iter_at_line(l as i32 - 1) {
                if let Some(c) = p.col {
                    it.forward_chars(c as i32 - 1);
                }
                buf.place_cursor(&it);
                self.w.view.scroll_to_iter(&mut it, 0.1, true, 0.5, 0.3);
            }
        }
        self.w.view.grab_focus();
    }

    /// F8 / Shift+F8: the next or previous problem, shown in the list and opened in the editor.
    pub(super) fn problems_step(&mut self, forward: bool) {
        if !self.problems.ran {
            self.problems_run();
        }
        let Some(i) = problems::step(self.problems.list.len(), self.problems.selected, forward)
        else {
            self.toast(&tr("There are no problems to go to"));
            return;
        };
        self.problems.selected = Some(i);
        self.w.problems.root.set_reveal_child(true);
        if let Some(r) = self.problems.rows.iter().position(|r| *r == Some(i)) {
            if let Some(row) = self.w.problems.list.row_at_index(r as i32) {
                self.w.problems.list.select_row(Some(&row));
            }
        }
        self.problems_show_help();
        self.problems_open_selected();
    }

    fn problems_ignore(&mut self) {
        let Some(p) = self.problems_selected().cloned() else {
            return;
        };
        if !problems::can_ignore(&p.id) {
            self.toast(&tr(
                "Problems with the markup or the package cannot be skipped",
            ));
            return;
        }
        self.problems.ignored.insert(p.id.clone());
        self.problems_save_ignored();
        self.toast(&trf(
            "Ignoring “{}” — it stays hidden until you show it again",
            &[&problems::title(&p.id)],
        ));
        self.problems_refilter(None);
    }

    fn problems_restore(&mut self, id: Option<&str>) {
        match id {
            Some(id) => {
                self.problems.ignored.remove(id);
            }
            None => self.problems.ignored.clear(),
        }
        self.problems_save_ignored();
        self.problems_refilter(None);
    }

    fn problems_save_ignored(&mut self) {
        let key = self.book_id.to_string();
        if self.problems.ignored.is_empty() {
            self.prefs.ignored_checks.remove(&key);
        } else {
            self.prefs
                .ignored_checks
                .insert(key, self.problems.ignored.clone());
        }
        self.prefs.save();
    }

    /// Fix (this file) or Fix all of this kind, after a checkpoint; the list is checked again afterwards.
    fn problems_fix(&mut self, whole_kind: bool) {
        let Some(p) = self.problems_selected().cloned() else {
            return;
        };
        let Some(label) = problems::fix_label(&p.id) else {
            self.toast(&tr("This problem has to be fixed by hand"));
            return;
        };
        let scope = if whole_kind || p.file.is_empty() {
            Scope::Kind
        } else {
            Scope::File(p.file.clone())
        };
        self.flush();
        self.commit_toc();
        let snapshot = self.create_checkpoint(&format!("Before: Fix {} ({label})", p.id));
        let opts = self.problems_options();
        let res = problems::apply_fix(&mut self.book.borrow_mut(), &p.id, &scope, &opts);
        match res {
            Ok(out) if out.changed => {
                self.after_archive_changed();
                self.problems_run();
                self.toast(&trf(
                    "{}: {} — undo with Tools → Restore Checkpoint",
                    &[&label, &(out.notes.first().cloned().unwrap_or_default())],
                ));
            }
            Ok(_) => {
                let _ = self.checkpoints.delete(snapshot);
                self.refresh_restore_menu();
                self.toast(&tr("Nothing could be fixed automatically here"));
            }
            Err(e) => {
                let _ = self
                    .checkpoints
                    .restore_into(snapshot, &mut self.book.borrow_mut().archive);
                self.after_archive_changed();
                self.toast(&trf("Could not fix: {}", &[&format!("{:#}", e)]));
            }
        }
    }

    // ---- dev hooks --------------------------------------------------------------------------

    /// `editor:check:<command>`: test infrastructure for build-aux/ui-tests.sh.
    pub(super) fn problems_dev(&mut self, c: &str) {
        let (cmd, arg) = c.split_once('=').unwrap_or((c, ""));
        match cmd {
            "run" => self.problems_run(),
            "select" => {
                let i: usize = arg.parse().unwrap_or(0);
                if let Some(r) = self.problems.rows.iter().position(|r| *r == Some(i)) {
                    if let Some(row) = self.w.problems.list.row_at_index(r as i32) {
                        self.w.problems.list.select_row(Some(&row));
                    }
                    self.problems_select_row(r as i32);
                }
            }
            "select-id" => {
                if let Some(i) = self.problems.list.iter().position(|p| p.id == arg) {
                    self.problems_dev(&format!("select={i}"));
                }
            }
            "open" => self.problems_open_selected(),
            "fix" => self.problems_fix(false),
            "fixkind" => self.problems_fix(true),
            "ignore" => self.problems_ignore(),
            "restore" => self.problems_restore(if arg.is_empty() { None } else { Some(arg) }),
            "next" => self.problems_step(true),
            "prev" => self.problems_step(false),
            "a11y" => self.problems_act(Act::Accessibility(arg == "on")),
            "edit" => {
                // pretends the file was edited since the check
                self.problems_mark_stale(arg);
            }
            "prefs" => {
                let p = crate::editor_prefs::EditorPrefs::load();
                eprintln!(
                    "check_prefs: {:?} a11y={}",
                    p.ignored_checks, p.check_accessibility
                );
            }
            "panel" => {
                let w = &self.w.problems;
                let mut controls = 0;
                let mut c = w.head.first_child();
                while let Some(x) = c {
                    if x.is::<gtk::Button>()
                        || x.is::<gtk::MenuButton>()
                        || x.is::<gtk::CheckButton>()
                    {
                        controls += 1;
                    }
                    c = x.next_sibling();
                }
                eprintln!(
                    "check_panel: problems={} results={} empty={} controls={controls}",
                    w.root.reveals_child(),
                    self.w.results_rev.reveals_child(),
                    w.body_stack.visible_child_name().as_deref() == Some("empty"),
                );
            }
            "dump" => {
                let (e, w, n) = problems::counts(&self.problems.list);
                eprintln!(
                    "check: problems={} errors={e} warnings={w} notes={n} hidden={} selected={} ignored={} a11y={} stale={} open={} rows={}",
                    self.problems.list.len(),
                    self.problems.hidden,
                    self.problems.selected.map_or(-1, |i| i as i32),
                    self.problems.ignored.iter().cloned().collect::<Vec<_>>().join(","),
                    self.problems.accessibility,
                    self.problems.stale_files.len(),
                    self.w.problems.root.reveals_child(),
                    self.problems.rows.len()
                );
                for (i, p) in self.problems.list.iter().enumerate() {
                    eprintln!(
                        "check_item: {i}|{}|{}|{}|{}|fix={}|stale={}",
                        p.id,
                        severity_word(p.severity),
                        p.file,
                        p.line.unwrap_or(0),
                        problems::fix_label(&p.id).is_some(),
                        self.problems.stale_files.contains(&p.file)
                    );
                }
            }
            "rows" => {
                // `editor_check_rows: rows=N max_row_h=H headers=K max_header_h=H gap=G`: the real sizes of the list
                let (mut rows, mut headers, mut max_row, mut max_head, mut gap) = (0, 0, 0, 0, 0);
                let mut prev_end: Option<i32> = None;
                for (r, idx) in self.problems.rows.iter().enumerate() {
                    let Some(row) = self.w.problems.list.row_at_index(r as i32) else {
                        continue;
                    };
                    let b = row.compute_bounds(&self.w.problems.list);
                    let (y, h) = b.map_or((0, row.height()), |b| {
                        (b.y().round() as i32, b.height().round() as i32)
                    });
                    if idx.is_some() {
                        rows += 1;
                        max_row = max_row.max(h);
                    } else {
                        headers += 1;
                        max_head = max_head.max(h);
                    }
                    if let Some(e) = prev_end {
                        gap = gap.max(y - e);
                    }
                    prev_end = Some(y + h);
                }
                eprintln!(
                    "editor_check_rows: rows={rows} max_row_h={max_row} headers={headers} max_header_h={max_head} gap={gap}"
                );
            }
            "help" => {
                let w = &self.w.problems;
                eprintln!(
                    "check_help: title={:?} meaning={:?} why={:?} how={:?} where={:?} fix={} fixall={} fixall_label={:?} ignore={}",
                    w.help_title.label(),
                    w.help_meaning.text(),
                    w.help_why.text(),
                    w.help_how.text(),
                    w.help_where.text(),
                    w.fix_btn.is_visible(),
                    w.fix_all_btn.is_visible(),
                    w.fix_all_btn.label().unwrap_or_default(),
                    w.ignore_btn.is_visible()
                );
            }
            _ => {}
        }
    }
}
