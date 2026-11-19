//! The *Manage Snippets* dialog: built-in snippets (editing one saves your own version) and the
//! user's, stored by `editor_snippets` in the config folder.

use super::*;
use crate::editor_snippets::{self as snip, Snippet};
use crate::i18n::tr;

const APPLIES: [(&str, &str); 3] = [
    ("any", "All files"),
    ("markup", "XHTML, HTML and XML"),
    ("css", "Stylesheets"),
];

struct Manager {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    templates: gtk::ListBox,
    user: RefCell<Vec<Snippet>>,
    tx: relm4::Sender<Input>,
}

impl Editor {
    pub(super) fn manage_snippets(&mut self) {
        let m = Rc::new(Manager {
            dialog: adw::Dialog::new(),
            list: gtk::ListBox::new(),
            templates: gtk::ListBox::new(),
            user: RefCell::new(snip::load()),
            tx: self.sender.input_sender().clone(),
        });
        m.dialog.set_title(&tr("Snippets"));
        m.dialog.set_content_width(560);
        m.dialog.set_content_height(620);
        m.list.add_css_class("boxed-list");
        m.list.set_selection_mode(gtk::SelectionMode::None);
        m.templates.add_css_class("boxed-list");
        m.templates.set_selection_mode(gtk::SelectionMode::None);
        crate::ui::audit::label(&m.list, &tr("Snippets"));
        crate::ui::audit::label(&m.templates, &tr("Templates for new files"));
        let hint = gtk::Label::new(Some(&tr(
            "Type a trigger word and press Tab to expand it. $1, $2… are tab stops, ${1:text} has a default, ${2*} takes the selected text, $0 is where the caret ends.",
        )));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let add = gtk::Button::with_label(&tr("Add Snippet"));
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);
        {
            let m2 = m.clone();
            add.connect_clicked(move |_| edit(&m2, None));
        }
        let tpl_title = gtk::Label::new(Some(&tr("Templates for new files")));
        tpl_title.add_css_class("heading");
        tpl_title.set_xalign(0.0);
        let tpl_hint = gtk::Label::new(Some(&tr(
            "Offered by Files → New File. {title} in the text is replaced by the name of the new file.",
        )));
        tpl_hint.set_wrap(true);
        tpl_hint.set_xalign(0.0);
        tpl_hint.add_css_class("dim-label");
        let add_tpl = gtk::Button::with_label(&tr("Add Template"));
        add_tpl.set_halign(gtk::Align::End);
        {
            let m2 = m.clone();
            add_tpl.connect_clicked(move |_| edit_template(&m2, None));
        }
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
        inner.append(&m.list);
        inner.append(&add);
        inner.append(&tpl_title);
        inner.append(&tpl_hint);
        inner.append(&m.templates);
        inner.append(&add_tpl);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&inner));
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        col.set_margin_all(12);
        col.append(&hint);
        col.append(&scroll);
        let header = adw::HeaderBar::new();
        let imp = gtk::Button::from_icon_name("document-open-symbolic");
        imp.set_tooltip_text(Some(&tr("Import snippets from a file")));
        crate::ui::audit::label(&imp, &tr("Import snippets"));
        let exp = gtk::Button::from_icon_name("document-save-symbolic");
        exp.set_tooltip_text(Some(&tr("Export your snippets to a file")));
        crate::ui::audit::label(&exp, &tr("Export snippets"));
        header.pack_start(&imp);
        header.pack_start(&exp);
        {
            let (m2, win) = (m.clone(), self.w.win.clone());
            imp.connect_clicked(move |_| {
                let m3 = m2.clone();
                gtk::FileDialog::new().open(Some(&win), gio::Cancellable::NONE, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        import_file(&m3, &p);
                    }
                });
            });
            let (m2, win) = (m.clone(), self.w.win.clone());
            exp.connect_clicked(move |_| {
                let m3 = m2.clone();
                let dlg = gtk::FileDialog::new();
                dlg.set_initial_name(Some("snippets.json"));
                dlg.save(Some(&win), gio::Cancellable::NONE, move |r| {
                    if let Some(p) = r.ok().and_then(|f| f.path()) {
                        export_file(&m3, &p);
                    }
                });
            });
        }
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&header);
        tb.set_content(Some(&col));
        m.dialog.set_child(Some(&tb));
        refresh(&m);
        m.dialog.present(Some(&self.w.win));
        MANAGER.with(|c| *c.borrow_mut() = Some(m));
    }

    /// Dev hooks of the snippets (`editor:snippet-…`, `editor:snippets-…`); `true` when handled.
    pub(super) fn snippets_dev(&mut self, c: &str) -> bool {
        if let Some(arg) = c.strip_prefix("snippet-run=") {
            // BEFORE|SELECTED: put the text in the open file, select the second part, press Tab
            let (before, selected) = arg.split_once('|').unwrap_or((arg, ""));
            let (before, selected) = (before.replace("\\n", "\n"), selected.replace("\\n", "\n"));
            let buf = self.w.view.buffer();
            buf.set_text(&format!("{before}{selected}"));
            self.comp.borrow().region.reset(&buf);
            let end = buf.end_iter();
            if selected.is_empty() {
                buf.place_cursor(&end);
            } else {
                let start = buf.iter_at_offset(before.chars().count() as i32);
                buf.select_range(&start, &end);
            }
            let done = completion::expand_at_caret(&self.w.view, &self.comp);
            let (s, e) = buf.bounds();
            eprintln!(
                "editor_snippet: expanded={done} text={:?}",
                buf.text(&s, &e, false)
            );
            return true;
        }
        if let Some(arg) = c.strip_prefix("snippets-export=") {
            let user = snip::load();
            let _ = std::fs::write(arg, snip::export_json(&user));
            eprintln!("editor_snippets_export: n={}", user.len());
            return true;
        }
        if let Some(arg) = c.strip_prefix("snippets-import=") {
            let text = std::fs::read_to_string(arg).unwrap_or_default();
            match snip::import_json(&text) {
                Ok((ok, skipped)) => {
                    let merged = snip::merge_import(&snip::load(), &ok);
                    let _ = snip::save(&merged);
                    self.comp.borrow_mut().snippets = snip::merged(&merged);
                    eprintln!(
                        "editor_snippets_import: added={} skipped={} total={}",
                        ok.len(),
                        skipped.len(),
                        merged.len()
                    );
                }
                Err(e) => eprintln!("editor_snippets_import: error={e}"),
            }
            return true;
        }
        if let Some(arg) = c.strip_prefix("template-save=") {
            // label|ext|body
            let mut it = arg.splitn(3, '|');
            let (l, e, b) = (
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
                it.next().unwrap_or(""),
            );
            let r = crate::editor_state::templates_dir()
                .map(|d| snip::save_template(&d, l, e, b))
                .unwrap_or(Err("no folder".into()));
            eprintln!("editor_template: saved={}", r.is_ok());
            return true;
        }
        false
    }
}

thread_local! {
    static MANAGER: RefCell<Option<Rc<Manager>>> = const { RefCell::new(None) };
}

fn import_file(m: &Rc<Manager>, path: &std::path::Path) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    match snip::import_json(&text) {
        Ok((ok, skipped)) => {
            let merged = snip::merge_import(&m.user.borrow(), &ok);
            *m.user.borrow_mut() = merged;
            commit(m);
            let msg = if skipped.is_empty() {
                trf("Imported {} snippets", &[&ok.len()])
            } else {
                trf(
                    "Imported {} snippets, skipped {}: {}",
                    &[&ok.len(), &skipped.len(), &skipped.join("; ")],
                )
            };
            tell(m, &msg);
        }
        Err(e) => tell(m, &e),
    }
}

fn export_file(m: &Rc<Manager>, path: &std::path::Path) {
    let user = m.user.borrow().clone();
    match std::fs::write(path, snip::export_json(&user)) {
        Ok(()) => tell(m, &trf("Exported {} snippets", &[&user.len()])),
        Err(e) => tell(m, &e.to_string()),
    }
}

fn tell(m: &Rc<Manager>, msg: &str) {
    let d = adw::AlertDialog::new(Some(msg), None);
    d.add_response("ok", &tr("OK"));
    d.present(Some(&m.dialog));
}

fn applies_label(a: &str) -> &'static str {
    APPLIES
        .iter()
        .find(|(k, _)| *k == a)
        .map(|(_, l)| *l)
        .unwrap_or("All files")
}

fn refresh(m: &Rc<Manager>) {
    while let Some(c) = m.list.first_child() {
        m.list.remove(&c);
    }
    let user = m.user.borrow().clone();
    for s in snip::merged(&user) {
        let is_user = user.contains(&s);
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&s.name))
            .subtitle(glib::markup_escape_text(&format!(
                "{} · {} · {} tab stop{}{}",
                s.trigger,
                applies_label(&s.applies_to),
                snip::tab_stops(&s.body).len(),
                if snip::tab_stops(&s.body).len() == 1 {
                    ""
                } else {
                    "s"
                },
                if is_user { "" } else { " · built in" }
            )))
            .build();
        let edit_b = icon_button("document-edit-symbolic", &tr("Edit"));
        edit_b.add_css_class("flat");
        edit_b.set_valign(gtk::Align::Center);
        {
            let (m2, s2) = (m.clone(), s.clone());
            edit_b.connect_clicked(move |_| edit(&m2, Some(s2.clone())));
        }
        row.add_suffix(&edit_b);
        if is_user {
            let del = icon_button("user-trash-symbolic", &tr("Delete"));
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            let (m2, s2) = (m.clone(), s.clone());
            del.connect_clicked(move |_| {
                m2.user.borrow_mut().retain(|u| *u != s2);
                commit(&m2);
            });
            row.add_suffix(&del);
        }
        m.list.append(&row);
    }
    while let Some(c) = m.templates.first_child() {
        m.templates.remove(&c);
    }
    let templates = crate::editor_state::user_templates();
    if templates.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No templates yet"))
            .build();
        m.templates.append(&row);
    }
    for t in templates {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&t.label))
            .subtitle(glib::markup_escape_text(&t.extension))
            .build();
        let edit_b = icon_button("document-edit-symbolic", &tr("Edit"));
        edit_b.add_css_class("flat");
        edit_b.set_valign(gtk::Align::Center);
        let del = icon_button("user-trash-symbolic", &tr("Delete"));
        del.add_css_class("flat");
        del.set_valign(gtk::Align::Center);
        {
            let (m2, t2) = (m.clone(), t.clone());
            edit_b.connect_clicked(move |_| edit_template(&m2, Some(t2.clone())));
            let (m2, t2) = (m.clone(), t.clone());
            del.connect_clicked(move |_| {
                if let Some(d) = crate::editor_state::templates_dir() {
                    snip::delete_template(&d, &t2.label, &t2.extension);
                }
                refresh(&m2);
            });
        }
        row.add_suffix(&edit_b);
        row.add_suffix(&del);
        m.templates.append(&row);
    }
}

/// The add/edit form of a template for new files.
fn edit_template(m: &Rc<Manager>, existing: Option<crate::editor_state::UserTemplate>) {
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some(&tr("Name, for example Chapter")));
    crate::ui::audit::label(&name, &tr("Template name"));
    let kind = gtk::DropDown::from_strings(&["XHTML", "CSS"]);
    crate::ui::audit::label(&kind, &tr("Template file type"));
    let body = gtk::TextView::new();
    body.set_monospace(true);
    body.set_top_margin(6);
    body.set_left_margin(6);
    crate::ui::audit::label(&body, &tr("Template text"));
    let bs = gtk::ScrolledWindow::new();
    bs.set_min_content_height(160);
    bs.set_has_frame(true);
    bs.set_child(Some(&body));
    let problem = gtk::Label::new(None);
    problem.add_css_class("issue-error");
    problem.set_xalign(0.0);
    if let Some(t) = &existing {
        name.set_text(&t.label);
        kind.set_selected(u32::from(t.extension == "css"));
        body.buffer().set_text(&t.body);
    }
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    col.append(&name);
    col.append(&kind);
    col.append(&bs);
    col.append(&problem);
    let d = adw::AlertDialog::new(
        Some(
            &(if existing.is_some() {
                tr("Edit Template")
            } else {
                tr("New Template")
            }),
        ),
        None,
    );
    d.set_extra_child(Some(&col));
    d.add_responses(&[("cancel", &tr("Cancel")), ("save", &tr("Save"))]);
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let m2 = m.clone();
    let old = existing.clone();
    d.connect_response(None, move |_, r| {
        if r != "save" {
            return;
        }
        let ext = if kind.selected() == 1 { "css" } else { "xhtml" };
        let b = body.buffer();
        let (s, e) = b.bounds();
        let text = b.text(&s, &e, false).to_string();
        let Some(dir) = crate::editor_state::templates_dir() else {
            return;
        };
        if let Some(o) = &old {
            if o.label != name.text().trim() {
                snip::delete_template(&dir, &o.label, &o.extension);
            }
        }
        match snip::save_template(&dir, name.text().trim(), ext, &text) {
            Ok(()) => refresh(&m2),
            Err(e) => {
                problem.set_label(&e);
                tell(&m2, &e);
            }
        }
    });
    d.present(Some(&m.dialog));
}

fn commit(m: &Rc<Manager>) {
    let user = m.user.borrow().clone();
    let _ = snip::save(&user);
    m.tx.emit(Input::SnippetsChanged(user));
    refresh(m);
}

/// The add/edit form. A built-in snippet is edited as a copy that overrides it.
fn edit(m: &Rc<Manager>, existing: Option<Snippet>) {
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some(&tr("Name, for example Footnote")));
    let trigger = gtk::Entry::new();
    trigger.set_placeholder_text(Some(&tr("Trigger word, for example fn")));
    let applies = gtk::DropDown::from_strings(&APPLIES.map(|(_, l)| l));
    let body = gtk::TextView::new();
    body.set_monospace(true);
    body.set_top_margin(6);
    body.set_left_margin(6);
    let bs = gtk::ScrolledWindow::new();
    bs.set_min_content_height(140);
    bs.set_has_frame(true);
    bs.set_child(Some(&body));
    let problem = gtk::Label::new(None);
    problem.add_css_class("issue-error");
    problem.set_xalign(0.0);
    let old = existing.clone();
    if let Some(s) = &existing {
        name.set_text(&s.name);
        trigger.set_text(&s.trigger);
        applies.set_selected(
            APPLIES
                .iter()
                .position(|(k, _)| *k == s.applies_to)
                .unwrap_or(0) as u32,
        );
        body.buffer().set_text(&s.body);
    }
    let sample = gtk::Entry::new();
    sample.set_placeholder_text(Some(&tr(
        "Selected text to try, for ${2*} stops (optional)",
    )));
    crate::ui::audit::label(&sample, &tr("Sample selected text"));
    let scratch = sv::View::new();
    scratch.set_monospace(true);
    scratch.set_enable_snippets(true);
    crate::ui::audit::label(&scratch, &tr("Snippet test result"));
    let scratch_scroll = gtk::ScrolledWindow::new();
    scratch_scroll.set_min_content_height(70);
    scratch_scroll.set_has_frame(true);
    scratch_scroll.set_child(Some(&scratch));
    let test = gtk::Button::with_label(&tr("Test"));
    test.set_tooltip_text(Some(&tr("Expand the snippet in the box below")));
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for w in [
        name.upcast_ref::<gtk::Widget>(),
        trigger.upcast_ref(),
        applies.upcast_ref(),
        bs.upcast_ref(),
        problem.upcast_ref(),
        sample.upcast_ref(),
        test.upcast_ref(),
        scratch_scroll.upcast_ref(),
    ] {
        col.append(w);
    }
    let d = adw::AlertDialog::new(
        Some(
            &(if old.is_some() {
                tr("Edit Snippet")
            } else {
                tr("New Snippet")
            }),
        ),
        None,
    );
    d.set_extra_child(Some(&col));
    d.add_responses(&[("cancel", &tr("Cancel")), ("save", &tr("Save"))]);
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let build = {
        let (name, trigger, applies, body) =
            (name.clone(), trigger.clone(), applies.clone(), body.clone());
        move || {
            let b = body.buffer();
            let (s, e) = b.bounds();
            Snippet {
                name: name.text().trim().to_string(),
                trigger: trigger.text().trim().to_string(),
                body: b.text(&s, &e, false).to_string(),
                applies_to: APPLIES[applies.selected() as usize % APPLIES.len()]
                    .0
                    .to_string(),
            }
        }
    };
    let build = Rc::new(build);
    {
        let (build, sample, scratch) = (build.clone(), sample.clone(), scratch.clone());
        test.connect_clicked(move |_| {
            let s = build();
            if snip::problem(&s).is_some() {
                return;
            }
            let b = scratch.buffer();
            b.set_text(&sample.text());
            b.select_range(&b.start_iter(), &b.end_iter());
            let region = completion::SnippetRegion::default();
            completion::expand(&scratch, &region, &s, 0);
        });
    }
    let validate = {
        let (build, problem, d) = (build.clone(), problem.clone(), d.clone());
        Rc::new(move || {
            let p = snip::problem(&build());
            problem.set_label(p.as_deref().unwrap_or(""));
            d.set_response_enabled("save", p.is_none());
        })
    };
    validate();
    for e in [&name, &trigger] {
        let v = validate.clone();
        e.connect_changed(move |_| v());
    }
    {
        let v = validate.clone();
        body.buffer().connect_changed(move |_| v());
    }
    let m2 = m.clone();
    d.connect_response(None, move |_, r| {
        if r != "save" {
            return;
        }
        let new = build();
        {
            let mut u = m2.user.borrow_mut();
            // replace the edited one (or the user override of a built-in) by the new version
            if let Some(o) = &old {
                u.retain(|x| x != o);
            }
            u.retain(|x| !(x.trigger == new.trigger && x.applies_to == new.applies_to));
            u.push(new);
        }
        commit(&m2);
    });
    d.present(Some(&m.dialog));
}
