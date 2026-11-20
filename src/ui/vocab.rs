//! The vocabulary window: words collected from dictionary lookups, reviewed as flashcards.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::rc::Rc;

use bookx_lookup::vocab::FlashcardSession;
use relm4::adw::{self, prelude::*};
use relm4::gtk;

use super::SharedRef;

/// Shown when the vocabulary is empty.
pub const EMPTY_HELP: &str =
    "No words yet. Select a word while reading and choose the dictionary button: looked-up words are collected here.";

pub fn show(parent: &adw::ApplicationWindow, shared: &SharedRef) {
    show_with(parent, shared, false, None);
}

/// The vocabulary window. `words` opens on the list of words instead of the flashcards, `focus` is a
/// word to find in it (the "Open in Vocabulary" of a saved term in the text).
pub fn show_with(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    words: bool,
    focus: Option<&str>,
) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&tr("Vocabulary"));
    dialog.set_content_width(520);
    dialog.set_content_height(620);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let summary = gtk::Label::new(None);
    summary.add_css_class("dim-label");
    summary.set_xalign(0.0);
    root.append(&summary);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.set_vexpand(true);
    card.set_valign(gtk::Align::Center);
    let word = gtk::Label::new(None);
    word.add_css_class("title-1");
    word.set_wrap(true);
    let answer = gtk::Label::new(None);
    answer.set_wrap(true);
    answer.set_selectable(true);
    answer.set_xalign(0.0);
    answer.set_visible(false);
    card.append(&word);
    card.append(&answer);
    root.append(&card);

    let show_btn = gtk::Button::with_label(&tr("Show Answer"));
    show_btn.add_css_class("suggested-action");
    show_btn.add_css_class("pill");
    let grades = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    grades.set_homogeneous(true);
    grades.set_visible(false);
    let buttons: Vec<(gtk::Button, u8)> = [("Again", 1u8), ("Hard", 3), ("Good", 4), ("Easy", 5)]
        .into_iter()
        .map(|(l, q)| {
            let b = gtk::Button::with_label(l);
            grades.append(&b);
            (b, q)
        })
        .collect();
    root.append(&show_btn);
    root.append(&grades);

    let session: Rc<RefCell<Option<FlashcardSession>>> = Rc::new(RefCell::new(None));
    let refresh = {
        let (shared, summary, word, answer, show_btn, grades, session) = (
            shared.clone(),
            summary.clone(),
            word.clone(),
            answer.clone(),
            show_btn.clone(),
            grades.clone(),
            session.clone(),
        );
        Rc::new(move || {
            let now = shared.now();
            let hub = shared.vocab.borrow();
            let c = hub.counts(now);
            summary.set_label(&trf(
                "{} words · {} due · {} learning · {} mastered",
                &[&c.total, &c.due, &c.learning, &c.mastered],
            ));
            answer.set_visible(false);
            grades.set_visible(false);
            if c.total == 0 {
                word.set_label(EMPTY_HELP);
                word.remove_css_class("title-1");
                show_btn.set_visible(false);
                return;
            }
            let mut s = session.borrow_mut();
            let sess = s.get_or_insert_with(|| FlashcardSession::new(&hub.store, now, 50));
            match sess.current().map(str::to_string) {
                Some(w) => {
                    word.add_css_class("title-1");
                    word.set_label(&w);
                    let e = hub.store.find(&w);
                    let mut text = e
                        .and_then(|e| e.definition.clone())
                        .unwrap_or_else(|| "(no definition saved)".into());
                    if let Some(ctx) = e.and_then(|e| e.contexts.first()) {
                        text.push_str(&format!("\n\n“{}”", ctx.sentence));
                    }
                    answer.set_label(&text);
                    show_btn.set_visible(true);
                }
                None => {
                    word.add_css_class("title-1");
                    let (n, ok, _) = sess.progress();
                    word.set_label(
                        &(if n == 0 {
                            tr("Nothing due today")
                        } else {
                            tr("All done!")
                        }),
                    );
                    answer.set_label(&trf("Reviewed {} cards, {} remembered.", &[&n, &ok]));
                    answer.set_visible(n > 0);
                    show_btn.set_visible(false);
                }
            }
        })
    };
    {
        let (answer, show_btn, grades) = (answer.clone(), show_btn.clone(), grades.clone());
        show_btn.connect_clicked(move |b| {
            answer.set_visible(true);
            grades.set_visible(true);
            b.set_visible(false);
        });
    }
    for (b, q) in buttons {
        let (shared, session, refresh) = (shared.clone(), session.clone(), refresh.clone());
        b.connect_clicked(move |_| {
            let now = shared.now();
            {
                let mut hub = shared.vocab.borrow_mut();
                if let Some(s) = session.borrow_mut().as_mut() {
                    let _ = s.answer(&mut hub.store, q, now);
                }
                hub.save();
            }
            refresh();
        });
    }
    refresh();

    // two pages: the flashcards, and the list of words (yours marked)
    let list = super::vocab_list::build(shared, None, focus);
    let stack = gtk::Stack::new();
    stack.add_named(&root, Some("review"));
    stack.add_named(&list.root, Some("words"));
    let switch = adw::ToggleGroup::new();
    for (name, label) in [("review", tr("Review")), ("words", tr("Words"))] {
        let t = adw::Toggle::new();
        t.set_name(Some(name));
        t.set_label(Some(&label));
        switch.add(t);
    }
    super::audit::label(&switch, &tr("Vocabulary page"));
    {
        let (stack, list) = (stack.clone(), list.clone());
        switch.connect_active_name_notify(move |g| {
            if let Some(n) = g.active_name() {
                stack.set_visible_child_name(&n);
                if n == "words" {
                    (list.refresh)();
                }
            }
        });
    }
    let start = if words || focus.is_some() {
        "words"
    } else {
        "review"
    };
    stack.set_visible_child_name(start);
    switch.set_active_name(Some(start));
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switch));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&stack));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}
