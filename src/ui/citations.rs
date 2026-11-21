//! *Export Citations*: a bibliography of the library, or of what the current search shows, as
//! BibTeX, RIS, CSL JSON or APA text, to copy or to save (roadmap PP-G-15, PP-G-17, L28).
//! Logic: `bookx_sources::citation`.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bookx_sources::citation::{self, Style};
use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, gio};

use super::SharedRef;
use crate::core::library::Book;
use crate::core::query::Query;
use crate::i18n::{ntr, tr, trf};

type Notify = Rc<dyn Fn(String)>;

/// The books a bibliography covers: the ones `query` finds, or all when it is empty.
pub fn in_scope<'a>(books: &'a [Book], query: &str) -> Vec<&'a Book> {
    if query.trim().is_empty() {
        return books.iter().collect();
    }
    let q = Query::parse(query);
    books.iter().filter(|b| q.matches(b)).collect()
}

fn render(shared: &SharedRef, query: &str, style: Style) -> (String, usize) {
    let lib = shared.lib.borrow();
    let books = in_scope(&lib.books, query);
    (citation::render_books(&books, style), books.len())
}

/// Shows the first lines only, so a library of thousands stays responsive.
fn preview(text: &str) -> String {
    const LINES: usize = 60;
    let mut it = text.lines();
    let head: Vec<&str> = it.by_ref().take(LINES).collect();
    let mut s = head.join("\n");
    if it.next().is_some() {
        s.push_str("\n…");
    }
    s
}

/// Copy and Save need at least one reference.
pub fn export_enabled(references: usize) -> bool {
    references > 0
}

type Controls = (gtk::Button, gtk::Button, gtk::Label);

thread_local! {
    static WINDOW: RefCell<Option<Controls>> = const { RefCell::new(None) };
}

/// The window. `query` is the search the library shows now (empty: none).
pub fn show(parent: &gtk::Window, shared: &SharedRef, query: &str, notify: Notify) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Export Citations"));
    dialog.set_content_width(640);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();

    let group = adw::PreferencesGroup::new();
    group.set_description(Some(&tr(
        "A bibliography from the title, authors, publisher, date and ISBN of your books, for a reference manager or a paper.",
    )));
    let names: Vec<String> = Style::ALL.iter().map(|s| s.label().to_string()).collect();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let format = adw::ComboRow::new();
    format.set_title(&tr("Format"));
    format.set_model(Some(&gtk::StringList::new(&refs)));
    group.add(&format);

    let have_query = !query.trim().is_empty();
    let scope = adw::ComboRow::new();
    scope.set_title(&tr("Books"));
    let (all_n, search_n) = {
        let lib = shared.lib.borrow();
        (lib.books.len(), in_scope(&lib.books, query).len())
    };
    let all_label = trf("All books ({})", &[&all_n]);
    let search_label = trf("The current search ({})", &[&search_n]);
    let scope_labels: Vec<&str> = if have_query {
        vec![&all_label, &search_label]
    } else {
        vec![&all_label]
    };
    scope.set_model(Some(&gtk::StringList::new(&scope_labels)));
    if have_query {
        scope.set_selected(1);
    }
    group.add(&scope);
    page.add(&group);

    let out = gtk::TextView::new();
    out.set_editable(false);
    out.set_cursor_visible(false);
    out.set_monospace(true);
    out.set_wrap_mode(gtk::WrapMode::WordChar);
    out.set_top_margin(8);
    out.set_bottom_margin(8);
    out.set_left_margin(8);
    out.set_right_margin(8);
    super::audit::label(&out, &tr("Bibliography preview"));
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_child(Some(&out));
    scroll.set_min_content_height(220);
    scroll.set_vexpand(true);
    scroll.add_css_class("card");
    let count = gtk::Label::new(None);
    count.set_xalign(0.0);
    count.add_css_class("dim-label");

    let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
    copy.set_tooltip_text(Some(&tr("Copy to Clipboard")));
    super::audit::label(&copy, &tr("Copy to Clipboard"));
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_bottom(12);
    body.append(&scroll);
    body.append(&count);

    let chosen = Rc::new(RefCell::new((Style::BibTeX, String::new())));
    let refresh = {
        let (shared, format, scope, out, count, chosen, query, copy, save) = (
            shared.clone(),
            format.clone(),
            scope.clone(),
            out.clone(),
            count.clone(),
            chosen.clone(),
            query.to_string(),
            copy.clone(),
            save.clone(),
        );
        Rc::new(move || {
            let style = Style::ALL
                .get(format.selected() as usize)
                .copied()
                .unwrap_or(Style::BibTeX);
            let q = if scope.selected() == 1 { &query } else { "" };
            let (text, n) = render(&shared, q, style);
            out.buffer().set_text(&preview(&text));
            count.set_text(&ntr("{} reference", "{} references", n));
            copy.set_sensitive(export_enabled(n));
            save.set_sensitive(export_enabled(n));
            *chosen.borrow_mut() = (style, text);
        })
    };
    {
        let r = refresh.clone();
        format.connect_selected_notify(move |_| r());
        let r = refresh.clone();
        scope.connect_selected_notify(move |_| r());
    }
    refresh();

    {
        let (chosen, notify, copy_btn) = (chosen.clone(), notify.clone(), copy.clone());
        copy.connect_clicked(move |_| {
            copy_btn.clipboard().set_text(&chosen.borrow().1);
            notify(tr("Copied to Clipboard"));
        });
    }
    {
        let (chosen, notify, parent) = (chosen.clone(), notify.clone(), parent.clone());
        save.connect_clicked(move |_| {
            let (style, text) = chosen.borrow().clone();
            let d = gtk::FileDialog::builder()
                .title(tr("Export Citations"))
                .initial_name(format!("bookx-bibliography.{}", style.extension()))
                .build();
            let notify = notify.clone();
            d.save(Some(&parent), gio::Cancellable::NONE, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    match std::fs::write(&p, &text) {
                        Ok(()) => notify(trf("Saved {}", &[&p.display()])),
                        Err(e) => notify(trf("Could not save: {}", &[&e])),
                    }
                }
            });
        });
    }

    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.pack_end(&save);
    header.pack_end(&copy);
    view.add_top_bar(&header);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.set_vexpand(false);
    content.append(&page);
    content.append(&body);
    let outer = gtk::ScrolledWindow::new();
    outer.set_hscrollbar_policy(gtk::PolicyType::Never);
    outer.set_child(Some(&content));
    view.set_content(Some(&outer));
    WINDOW.with(|w| *w.borrow_mut() = Some((save.clone(), copy.clone(), count.clone())));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));
}

/// `citations:<command>` of the developer script: `open`, or `write=STYLE|QUERY|FILE` (writes the
/// bibliography exactly as the window would and prints `citations: wrote N STYLE`).
pub fn dev(parent: &gtk::Window, shared: &SharedRef, notify: Notify, cmd: &str) {
    if cmd == "open" {
        show(parent, shared, "", notify);
        return;
    }
    // `state`: `citations_window: save_mapped=BOOL save_sensitive=BOOL copy_sensitive=BOOL refs=TEXT`
    if cmd == "state" {
        WINDOW.with(|w| match w.borrow().as_ref() {
            Some((save, copy, count)) => {
                eprintln!(
                    "citations_window: save_mapped={} save_sensitive={} copy_sensitive={} refs={}",
                    save.is_mapped(),
                    save.is_sensitive(),
                    copy.is_sensitive(),
                    count.text()
                )
            }
            None => eprintln!("citations_window: closed"),
        });
        return;
    }
    if let Some(arg) = cmd.strip_prefix("write=") {
        let mut it = arg.splitn(3, '|');
        let (style, query, file) = (
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
            it.next().unwrap_or(""),
        );
        let Some(style) = Style::parse(style) else {
            eprintln!("citations: unknown style {style}");
            return;
        };
        let (text, n) = render(shared, query, style);
        match std::fs::write(Path::new(file), &text) {
            Ok(()) => eprintln!("citations: wrote {n} {}", style.label()),
            Err(e) => eprintln!("citations: write failed {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_needs_references() {
        assert!(!export_enabled(0));
        assert!(export_enabled(1));
    }

    #[test]
    fn preview_cuts_long_text() {
        let long: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let p = preview(&long);
        assert_eq!(p.lines().count(), 61);
        assert!(p.ends_with('…'));
        assert_eq!(preview("a\nb"), "a\nb");
    }
}
