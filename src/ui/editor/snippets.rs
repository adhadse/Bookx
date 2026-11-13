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
    user: RefCell<Vec<Snippet>>,
    tx: relm4::Sender<Input>,
}

impl Editor {
    pub(super) fn manage_snippets(&mut self) {
        let m = Rc::new(Manager {
            dialog: adw::Dialog::new(),
            list: gtk::ListBox::new(),
            user: RefCell::new(snip::load()),
            tx: self.sender.input_sender().clone(),
        });
        m.dialog.set_title(&tr("Snippets"));
        m.dialog.set_content_width(520);
        m.dialog.set_content_height(520);
        m.list.add_css_class("boxed-list");
        m.list.set_selection_mode(gtk::SelectionMode::None);
        let hint = gtk::Label::new(Some(
            "Type a trigger word and press Tab to expand it. $1, $2… are tab stops, ${1:text} has a default, $0 is where the caret ends.",
        ));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&m.list));
        let add = gtk::Button::with_label(&tr("Add Snippet"));
        add.add_css_class("suggested-action");
        add.set_halign(gtk::Align::End);
        {
            let m2 = m.clone();
            add.connect_clicked(move |_| edit(&m2, None));
        }
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        col.set_margin_all(12);
        col.append(&hint);
        col.append(&scroll);
        col.append(&add);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&adw::HeaderBar::new());
        tb.set_content(Some(&col));
        m.dialog.set_child(Some(&tb));
        refresh(&m);
        m.dialog.present(Some(&self.w.win));
    }
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
    let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for w in [
        name.upcast_ref::<gtk::Widget>(),
        trigger.upcast_ref(),
        applies.upcast_ref(),
        bs.upcast_ref(),
        problem.upcast_ref(),
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
