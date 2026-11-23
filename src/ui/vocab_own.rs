//! Terms with your own definition, in the reader: the dialog that saves one (a term does not have to be
//! in any dictionary), the "Your definition" block of the look-up popover, and the small popover that
//! a saved term in the text opens. The list lives in `vocab_list.rs`, the matching in `vocab_marks.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use bookx_lookup::vocab::{OwnTerm, Scope, VocabEntry};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gdk, glib};

use super::SharedRef;
pub use super::vocab_list::{BookView, show_book_terms};
use crate::i18n::{tr, trf};

/// What the dialog starts with.
#[derive(Clone)]
pub struct Ctx {
    pub shared: SharedRef,
    pub book_id: Option<u64>,
    pub series: String,
    pub term: String,
    pub definition: String,
    pub note: String,
    pub tags: String,
    pub language: String,
    pub scope: Scope,
    /// The sentence the term was met in (kept with the term and the place in the book).
    pub context: String,
    pub position: Option<f64>,
    /// The term already has your own definition: the dialog edits it.
    pub existing: bool,
}

impl Ctx {
    /// The dialog for `term`, met in `book_id` (none: added from the list), starting with what is saved
    /// for the term already.
    pub fn new(
        shared: &SharedRef,
        book_id: Option<u64>,
        term: &str,
        context: &str,
        position: Option<f64>,
        language: &str,
    ) -> Ctx {
        let series = book_id
            .and_then(|b| shared.lib.borrow().get(b).map(|b| b.meta.series.clone()))
            .unwrap_or_default();
        let mut c = Ctx {
            shared: shared.clone(),
            book_id,
            series,
            term: term.to_string(),
            definition: String::new(),
            note: String::new(),
            tags: String::new(),
            language: language.to_string(),
            scope: book_id.map_or(Scope::All, |b| Scope::Book(b.to_string())),
            context: context.to_string(),
            position,
            existing: false,
        };
        let hub = shared.vocab.borrow();
        if let Some(e) = hub.store.find(term).filter(|e| e.own.is_some()) {
            c.term = e.word.clone();
            c.definition = e.own.clone().unwrap_or_default();
            c.note = e.note.clone();
            c.tags = e.tags.join(" ");
            if !e.language.is_empty() {
                c.language = e.language.clone();
            }
            c.scope = e.scope.clone();
            c.existing = true;
        }
        c
    }
}

#[derive(Clone)]
struct Open {
    dialog: adw::Dialog,
    term: adw::EntryRow,
    definition: gtk::TextView,
    note: adw::EntryRow,
    scope: adw::ComboRow,
    scopes: Vec<Scope>,
    save: gtk::Button,
    remove: gtk::Button,
    status: gtk::Label,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    /// What the Undo toast of the last "Remove Definition" does (the test hook presses it).
    static LAST_REMOVE_UNDO: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

fn scope_label(s: &Scope, ctx: &Ctx) -> String {
    match s {
        Scope::All => tr("All books"),
        Scope::Book(b) if Some(b.as_str()) == ctx.book_id.map(|x| x.to_string()).as_deref() => {
            tr("This book")
        }
        Scope::Book(_) => tr("Another book"),
        Scope::Series(n) if n.trim().to_lowercase() == ctx.series.trim().to_lowercase() => {
            trf("This series ({})", &[n])
        }
        Scope::Series(n) => trf("The series {}", &[n]),
    }
}

/// *Save* is usable when both the term and the definition hold text.
pub fn can_save(term: &str, definition: &str) -> bool {
    !term.trim().is_empty() && !definition.trim().is_empty()
}

/// Opens the dialog over `parent`. `done` runs with the saved term (or `None` when removed).
pub fn open_dialog(
    parent: &impl IsA<gtk::Widget>,
    ctx: Ctx,
    done: impl Fn(Option<String>) + 'static,
) {
    let done: Rc<dyn Fn(Option<String>)> = Rc::new(done);
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&if ctx.existing {
        tr("Your Definition")
    } else {
        tr("Add to Vocabulary")
    });
    dialog.set_content_width(460);
    dialog.set_content_height(560);

    // the places this term can apply to
    let mut scopes: Vec<Scope> = Vec::new();
    if let Some(b) = ctx.book_id {
        scopes.push(Scope::Book(b.to_string()));
    }
    scopes.push(Scope::All);
    if !ctx.series.trim().is_empty() {
        scopes.push(Scope::Series(ctx.series.clone()));
    }
    if !scopes.contains(&ctx.scope) {
        scopes.insert(0, ctx.scope.clone());
    }

    let group = adw::PreferencesGroup::new();
    let term = adw::EntryRow::new();
    term.set_title(&tr("Term"));
    term.set_text(&ctx.term);
    super::audit::label(&term, &tr("Term"));
    group.add(&term);

    let def_label = gtk::Label::new(Some(&tr("Your definition")));
    def_label.add_css_class("heading");
    def_label.set_xalign(0.0);
    def_label.set_margin_top(12);
    let definition = gtk::TextView::new();
    definition.set_wrap_mode(gtk::WrapMode::WordChar);
    definition.set_accepts_tab(false);
    definition.set_top_margin(8);
    definition.set_bottom_margin(8);
    definition.set_left_margin(10);
    definition.set_right_margin(10);
    definition.buffer().set_text(&ctx.definition);
    super::audit::label(&definition, &tr("Your definition"));
    let frame = gtk::ScrolledWindow::new();
    frame.add_css_class("card");
    frame.set_min_content_height(120);
    frame.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    frame.set_child(Some(&definition));

    let details = adw::PreferencesGroup::new();
    details.set_margin_top(12);
    let note = adw::EntryRow::new();
    note.set_title(&tr("Note (optional)"));
    note.set_text(&ctx.note);
    super::audit::label(&note, &tr("Note"));
    let tags = adw::EntryRow::new();
    tags.set_title(&tr("Tags (optional, separated by spaces)"));
    tags.set_text(&ctx.tags);
    super::audit::label(&tags, &tr("Tags"));
    let lang = adw::EntryRow::new();
    lang.set_title(&tr("Language"));
    lang.set_text(&ctx.language);
    super::audit::label(&lang, &tr("Language"));
    let scope = adw::ComboRow::new();
    scope.set_title(&tr("Applies to"));
    let labels: Vec<String> = scopes.iter().map(|s| scope_label(s, &ctx)).collect();
    scope.set_model(Some(&gtk::StringList::new(
        &labels.iter().map(String::as_str).collect::<Vec<_>>(),
    )));
    scope.set_selected(scopes.iter().position(|s| *s == ctx.scope).unwrap_or(0) as u32);
    super::audit::label(&scope, &tr("Applies to"));
    details.add(&note);
    details.add(&tags);
    details.add(&lang);
    details.add(&scope);

    let context = gtk::Label::new(None);
    context.set_wrap(true);
    context.set_xalign(0.0);
    context.set_selectable(true);
    context.add_css_class("dim-label");
    context.set_margin_top(12);
    if !ctx.context.trim().is_empty() {
        let c: String = ctx.context.chars().take(400).collect();
        context.set_label(&trf("Met in: “{}”", &[&c]));
    } else {
        context.set_visible(false);
    }
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("error");
    status.set_wrap(true);
    status.set_visible(false);
    status.set_margin_top(8);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.set_margin_top(12);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&group);
    body.append(&def_label);
    body.append(&frame);
    body.append(&details);
    body.append(&context);
    body.append(&status);
    let remove = gtk::Button::with_label(&tr("Remove Definition"));
    remove.add_css_class("destructive-action");
    remove.add_css_class("flat");
    remove.set_halign(gtk::Align::Start);
    remove.set_margin_top(8);
    remove.set_visible(ctx.existing);
    body.append(&remove);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_child(Some(&body));
    scroll.set_vexpand(true);

    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    save.set_tooltip_text(Some(&tr("Save (Ctrl+Enter)")));
    header.pack_start(&cancel);
    header.pack_end(&save);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));
    dialog.set_default_widget(Some(&save));
    {
        let refresh = {
            let (save, term, definition) = (save.clone(), term.clone(), definition.clone());
            move || {
                let b = definition.buffer();
                let text = b.text(&b.start_iter(), &b.end_iter(), false);
                save.set_sensitive(can_save(&term.text(), &text));
            }
        };
        refresh();
        let r = refresh.clone();
        term.connect_changed(move |_| r());
        definition.buffer().connect_changed(move |_| refresh());
    }

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    // Ctrl+Enter saves from anywhere in the dialog, Enter in the term moves on to the definition
    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let save = save.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && mods.contains(gdk::ModifierType::CONTROL_MASK)
            {
                save.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(keys);
        let def = definition.clone();
        term.connect_entry_activated(move |_| {
            def.grab_focus();
        });
    }
    {
        let (ctx, term, definition, note, tags, lang, scope, status, d, done) = (
            ctx.clone(),
            term.clone(),
            definition.clone(),
            note.clone(),
            tags.clone(),
            lang.clone(),
            scope.clone(),
            status.clone(),
            dialog.clone(),
            done.clone(),
        );
        let scopes = scopes.clone();
        save.connect_clicked(move |_| {
            let b = definition.buffer();
            let text = b.text(&b.start_iter(), &b.end_iter(), false).to_string();
            if term.text().trim().is_empty() {
                term.add_css_class("error");
                status.set_label(&tr("Type the term first."));
                status.set_visible(true);
                term.grab_focus();
                return;
            }
            if text.trim().is_empty() {
                status.set_label(&tr("Write your definition first."));
                status.set_visible(true);
                definition.grab_focus();
                return;
            }
            let own = OwnTerm {
                term: term.text().to_string(),
                definition: text,
                note: note.text().to_string(),
                tags: tags.text().split([' ', ',']).map(str::to_string).collect(),
                language: lang.text().to_string(),
                scope: scopes
                    .get(scope.selected() as usize)
                    .cloned()
                    .unwrap_or_default(),
                context: Some(ctx.context.clone()),
                book_id: ctx.book_id.map(|b| b.to_string()),
                position: ctx.position,
            };
            let now = ctx.shared.now();
            let mut hub = ctx.shared.vocab.borrow_mut();
            match hub.save_own(&own, now) {
                Ok(_) => {
                    // the term was renamed: the old one goes
                    if ctx.existing && !ctx.term.trim().eq_ignore_ascii_case(own.term.trim()) {
                        hub.remove_own(&ctx.term);
                    }
                    drop(hub);
                    let t = own.term.clone();
                    OPEN.with(|o| *o.borrow_mut() = None);
                    d.close();
                    done(Some(t));
                }
                Err(e) => {
                    status.set_label(&e.to_string());
                    status.set_visible(true);
                }
            }
        });
    }
    {
        let (shared, term_text, d, done, parent) = (
            ctx.shared.clone(),
            ctx.term.clone(),
            dialog.clone(),
            done.clone(),
            parent.clone().upcast::<gtk::Widget>(),
        );
        remove.connect_clicked(move |_| {
            // reversible, so no confirmation: the definition goes and a toast offers it back
            let before = shared.vocab.borrow().store.find(&term_text).cloned();
            shared.vocab.borrow_mut().remove_own(&term_text);
            OPEN.with(|o| *o.borrow_mut() = None);
            d.close();
            done(None);
            if let (Some(entry), Some(overlay)) = (before, super::dialog_toast::overlay_of(&parent))
            {
                let (shared, done) = (shared.clone(), done.clone());
                let stash = Rc::new(RefCell::new(Some(entry)));
                let undo: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(e) = stash.borrow_mut().take() {
                        shared.vocab.borrow_mut().restore_entry(e);
                        done(None);
                    }
                });
                LAST_REMOVE_UNDO.with(|u| *u.borrow_mut() = Some(undo.clone()));
                super::dialog_toast::show_undo(&overlay, &tr("Definition removed"), move || undo());
            }
        });
    }
    dialog.connect_closed(|_| OPEN.with(|o| *o.borrow_mut() = None));
    OPEN.with(|o| {
        *o.borrow_mut() = Some(Open {
            dialog: dialog.clone(),
            term: term.clone(),
            definition: definition.clone(),
            note: note.clone(),
            scope: scope.clone(),
            scopes,
            save: save.clone(),
            remove: remove.clone(),
            status: status.clone(),
        })
    });
    dialog.present(Some(parent));
    if ctx.term.trim().is_empty() {
        term.grab_focus();
    } else {
        definition.grab_focus();
    }
}

/// Dev hook (`vocab:dialog=...` and `reader:ui-vocab-*`): drives the open dialog through its widgets.
pub fn dev(arg: &str) -> String {
    if arg == "undo-remove" {
        let undo = LAST_REMOVE_UNDO.with(|u| u.borrow_mut().take());
        return match undo {
            Some(u) => {
                u();
                "vocab_dialog: remove undone".to_string()
            }
            None => "vocab_dialog: nothing to undo".to_string(),
        };
    }
    {
        // the handles are copied out first: pressing Save closes the dialog and clears `OPEN`
        let Some(o) = OPEN.with(|o| o.borrow().clone()) else {
            return "vocab_dialog: closed".to_string();
        };
        let o = &o;
        if let Some(t) = arg.strip_prefix("term=") {
            o.term.set_text(t);
        } else if let Some(t) = arg.strip_prefix("def=") {
            o.definition.buffer().set_text(&t.replace("\\n", "\n"));
        } else if let Some(t) = arg.strip_prefix("note=") {
            o.note.set_text(t);
        } else if let Some(t) = arg.strip_prefix("scope=") {
            let want = match t {
                "all" => Some(Scope::All),
                "book" => o
                    .scopes
                    .iter()
                    .find(|s| matches!(s, Scope::Book(_)))
                    .cloned(),
                "series" => o
                    .scopes
                    .iter()
                    .find(|s| matches!(s, Scope::Series(_)))
                    .cloned(),
                _ => None,
            };
            if let Some(i) = want.and_then(|w| o.scopes.iter().position(|s| *s == w)) {
                o.scope.set_selected(i as u32);
            }
        } else if arg == "remove" {
            o.remove.emit_clicked();
        } else if arg == "save" {
            o.save.emit_clicked();
            if OPEN.with(|o| o.borrow().is_none()) {
                return "vocab_dialog: saved".to_string();
            }
        }
        let b = o.definition.buffer();
        format!(
            "vocab_dialog: open={} title={:?} term={:?} def={:?} note={:?} scope={:?} status={:?} focus_def={} save_enabled={}",
            o.dialog.is_visible() || o.dialog.parent().is_some(),
            o.dialog.title(),
            o.term.text(),
            b.text(&b.start_iter(), &b.end_iter(), false),
            o.note.text(),
            o.scope
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string()),
            if o.status.is_visible() {
                o.status.label().to_string()
            } else {
                String::new()
            },
            o.definition.has_focus(),
            o.save.is_sensitive()
        )
    }
}

// ---- "Your definition" in the look-up popover -----------------------------------------------------------

/// The block above the dictionary text: your definition of the word, or the offer to write one.
#[derive(Clone)]
pub struct YourDef {
    pub root: gtk::Box,
    heading: gtk::Label,
    text: gtk::Label,
    note: gtk::Label,
    pub button: gtk::Button,
    /// The term the button opens the dialog for.
    target: Rc<RefCell<String>>,
}

impl YourDef {
    /// `on_click` gets the term to write the definition of (or to edit).
    pub fn new(on_click: impl Fn(String) + 'static) -> YourDef {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_margin_bottom(8);
        let heading = gtk::Label::new(Some(&tr("Your definition")));
        heading.add_css_class("heading");
        heading.set_xalign(0.0);
        heading.set_visible(false);
        let text = gtk::Label::new(None);
        text.set_wrap(true);
        text.set_xalign(0.0);
        text.set_max_width_chars(44);
        text.set_selectable(true);
        text.set_visible(false);
        let note = gtk::Label::new(None);
        note.set_wrap(true);
        note.set_xalign(0.0);
        note.set_max_width_chars(44);
        note.add_css_class("dim-label");
        note.add_css_class("caption");
        note.set_visible(false);
        let button = gtk::Button::with_label(&tr("Add Your Definition"));
        button.add_css_class("flat");
        button.set_halign(gtk::Align::Start);
        let target: Rc<RefCell<String>> = Rc::default();
        let t2 = target.clone();
        button.connect_clicked(move |_| on_click(t2.borrow().clone()));
        root.append(&heading);
        root.append(&text);
        root.append(&note);
        root.append(&button);
        root.set_visible(false);
        YourDef {
            root,
            heading,
            text,
            note,
            button,
            target,
        }
    }

    /// Shows what is saved for the word (when it is in force in this book), else the offer to add it.
    /// `term` is empty when nothing was selected that can be a term.
    pub fn set(&self, entry: Option<&VocabEntry>, term: &str) {
        self.root.set_visible(!term.trim().is_empty());
        let own = entry.and_then(|e| e.own.as_deref());
        self.heading.set_visible(own.is_some());
        self.text.set_visible(own.is_some());
        self.text.set_label(own.unwrap_or(""));
        let note = entry.map(|e| e.note.as_str()).unwrap_or("");
        self.note.set_visible(own.is_some() && !note.is_empty());
        self.note.set_label(note);
        self.button.set_label(&if own.is_some() {
            tr("Edit Your Definition")
        } else {
            tr("Add Your Definition")
        });
        *self.target.borrow_mut() = entry
            .filter(|e| e.own.is_some())
            .map_or_else(|| term.to_string(), |e| e.word.clone());
    }

    /// What the block says now (tests).
    pub fn dump(&self) -> String {
        format!(
            "visible={} definition={:?} note={:?} button={:?}",
            self.root.is_visible(),
            if self.text.is_visible() {
                self.text.label().to_string()
            } else {
                String::new()
            },
            if self.note.is_visible() {
                self.note.label().to_string()
            } else {
                String::new()
            },
            self.button.label().unwrap_or_default()
        )
    }
}

// ---- the popover of a saved term in the text --------------------------------------------------------------

/// What the buttons of the popover do (each gets the term).
#[derive(Clone)]
pub struct MarkActions {
    pub edit: Rc<dyn Fn(String)>,
    pub remove: Rc<dyn Fn(String)>,
    pub open: Rc<dyn Fn(String)>,
}

#[derive(Clone)]
struct MarkPop {
    pop: gtk::Popover,
    text: String,
    edit: gtk::Button,
    remove: gtk::Button,
    open: gtk::Button,
}

thread_local! {
    static MARK: RefCell<Option<MarkPop>> = const { RefCell::new(None) };
}

/// Shows your definition of a saved term in a small popover at `rect` (widget coordinates of `parent`).
pub fn show_mark_popover(
    parent: &impl IsA<gtk::Widget>,
    rect: (i32, i32, i32, i32),
    entry: &VocabEntry,
    actions: MarkActions,
) {
    close_mark_popover();
    let pop = gtk::Popover::new();
    pop.set_parent(parent);
    super::card_drag::avoid(&pop);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(
        rect.0,
        rect.1,
        rect.2.max(1),
        rect.3.max(1),
    )));
    pop.add_css_class("vocab-popover");
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
    bx.set_margin_top(8);
    bx.set_margin_bottom(8);
    bx.set_margin_start(10);
    bx.set_margin_end(10);
    let term = gtk::Label::new(Some(&entry.word));
    term.add_css_class("heading");
    term.set_xalign(0.0);
    term.set_wrap(true);
    term.set_max_width_chars(40);
    let tag = gtk::Label::new(Some(&tr("Your definition")));
    tag.add_css_class("caption");
    tag.add_css_class("dim-label");
    tag.set_xalign(0.0);
    let text = gtk::Label::new(entry.own.as_deref());
    text.set_wrap(true);
    text.set_xalign(0.0);
    text.set_max_width_chars(40);
    text.set_selectable(false);
    bx.append(&term);
    bx.append(&tag);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(220);
    scroll.set_child(Some(&text));
    bx.append(&scroll);
    if !entry.note.is_empty() {
        let n = gtk::Label::new(Some(&entry.note));
        n.set_wrap(true);
        n.set_xalign(0.0);
        n.set_max_width_chars(40);
        n.add_css_class("dim-label");
        n.add_css_class("caption");
        bx.append(&n);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let edit = gtk::Button::with_label(&tr("Edit"));
    let remove = gtk::Button::with_label(&tr("Remove"));
    let open = gtk::Button::with_label(&tr("Open in Vocabulary"));
    for b in [&edit, &remove, &open] {
        b.add_css_class("flat");
        row.append(b);
    }
    remove.add_css_class("destructive-action");
    bx.append(&row);
    pop.set_child(Some(&bx));
    let word = entry.word.clone();
    for (b, f) in [
        (&edit, actions.edit.clone()),
        (&remove, actions.remove.clone()),
        (&open, actions.open.clone()),
    ] {
        let (pop, word) = (pop.clone(), word.clone());
        b.connect_clicked(move |_| {
            pop.popdown();
            f(word.clone());
        });
    }
    pop.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || {
            if p.parent().is_some() {
                p.unparent();
            }
        });
    });
    MARK.with(|m| {
        *m.borrow_mut() = Some(MarkPop {
            pop: pop.clone(),
            text: entry.own.clone().unwrap_or_default(),
            edit,
            remove,
            open,
        })
    });
    pop.popup();
    // the first button takes the focus, so the keyboard works at once
    if let Some(b) = MARK.with(|m| m.borrow().as_ref().map(|m| m.edit.clone())) {
        b.grab_focus();
    }
}

pub fn close_mark_popover() {
    MARK.with(|m| {
        if let Some(old) = m.borrow_mut().take() {
            old.pop.popdown();
            glib::idle_add_local_once(move || {
                if old.pop.parent().is_some() {
                    old.pop.unparent();
                }
            });
        }
    });
}

/// Dev hook: what the popover of a saved term shows, and a press of its buttons.
pub fn dev_mark(arg: &str) -> String {
    {
        let Some(m) = MARK.with(|m| m.borrow().clone()) else {
            return "vocab_mark: closed".to_string();
        };
        match arg {
            "edit" => m.edit.emit_clicked(),
            "remove" => m.remove.emit_clicked(),
            "open" => m.open.emit_clicked(),
            _ => {}
        }
        format!("vocab_mark: open={} text={:?}", m.pop.is_visible(), m.text)
    }
}

#[cfg(test)]
mod tests {
    use super::can_save;

    #[test]
    fn save_needs_a_term_and_a_definition() {
        assert!(!can_save("", "x"));
        assert!(!can_save("word", "  \n"));
        assert!(can_save(" word ", "a meaning"));
    }
}
