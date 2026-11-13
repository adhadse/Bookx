//! Several formats in one book: the *Formats* part of the details panel, the merge dialog with its
//! preview, the offer to add a file as another format, and the `formats:` developer commands.
//! Logic: `crate::book_formats`, file handling `core::merge`.

use crate::i18n::ntr;
use crate::i18n::tr;
use crate::i18n::trf;
use std::path::PathBuf;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::book_formats::{self as bf, Offer};
use crate::core::library::{Book, Library, Prepared};

/// What the rows of the details panel can ask for.
#[derive(Clone)]
pub struct Callbacks {
    /// Open the book in the reader.
    pub read: Rc<dyn Fn(u64)>,
    /// Convert the selected book to an export target (`crate::export::TARGETS` id).
    pub convert: Rc<dyn Fn(String)>,
    /// The library changed: save, redraw, toast.
    pub changed: Rc<dyn Fn(String)>,
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// The *Protected* banner of a book with copy protection; `None` for every other book.
pub fn protected_banner(b: &Book) -> Option<gtk::Widget> {
    if b.drm.is_empty() {
        return None;
    }
    let l = gtk::Label::new(Some(&bf::protected_message(&b.drm)));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.add_css_class("error");
    l.add_css_class("caption-heading");
    l.set_margin_top(6);
    Some(l.upcast())
}

/// The list of format files of a book with Open, Convert and Remove; `None` when the book has just
/// one file (the plain *Format* row shows it).
pub fn details_group(
    shared: &SharedRef,
    b: &Book,
    cb: &Callbacks,
) -> Option<adw::PreferencesGroup> {
    if b.formats.is_empty() {
        return None;
    }
    let rows = bf::rows(&shared.lib.borrow(), b);
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Formats"));
    group.set_margin_top(10);
    for r in rows {
        let row = adw::ActionRow::new();
        row.set_title(&esc(&format!(
            "{}{}",
            r.format.to_uppercase(),
            if r.main { " · main" } else { "" }
        )));
        row.set_subtitle(&esc(&format!(
            "{}{}",
            crate::library_health::size_text(r.size),
            if r.exists { "" } else { " · file is missing" }
        )));
        let open = gtk::Button::with_label(&tr("Open"));
        open.add_css_class("flat");
        open.set_valign(gtk::Align::Center);
        open.set_sensitive(r.exists);
        {
            let (read, id, path, in_bookx) = (
                cb.read.clone(),
                b.id,
                shared.lib.borrow().root.join(&r.file),
                r.opens_in_bookx(),
            );
            open.connect_clicked(move |_| {
                if in_bookx {
                    read(id);
                } else {
                    super::open_uri(&gtk::gio::File::for_path(&path).uri());
                }
            });
        }
        row.add_suffix(&open);
        if r.main {
            let menu = gtk::MenuButton::new();
            menu.set_label(&tr("Convert"));
            menu.add_css_class("flat");
            menu.set_valign(gtk::Align::Center);
            let pop = gtk::Popover::new();
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for t in crate::export::TARGETS {
                let item = gtk::Button::with_label(t.label);
                item.add_css_class("flat");
                let (convert, id, pop) = (cb.convert.clone(), t.id.to_string(), pop.clone());
                item.connect_clicked(move |_| {
                    pop.popdown();
                    convert(id.clone());
                });
                list.append(&item);
            }
            pop.set_child(Some(&list));
            menu.set_popover(Some(&pop));
            row.add_suffix(&menu);
        } else {
            let remove = gtk::Button::with_label(&tr("Remove"));
            remove.add_css_class("flat");
            remove.set_valign(gtk::Align::Center);
            let (shared, changed, id, file, fmt) = (
                shared.clone(),
                cb.changed.clone(),
                b.id,
                r.file.clone(),
                r.format.to_uppercase(),
            );
            remove.connect_clicked(move |btn| {
                let parent = btn.root();
                let (shared, changed, file, fmt) =
                    (shared.clone(), changed.clone(), file.clone(), fmt.clone());
                let do_remove = move || {
                    let res = shared.lib.borrow_mut().remove_format(id, &file);
                    match res {
                        Ok(()) => changed(trf("Removed the {} file (it waits in the trash folder; Undo brings it back)", &[&fmt])),
                        Err(e) => changed(trf("Could not remove: {}", &[&format!("{:#}", e)])),
                    }
                };
                match parent {
                    Some(p) => super::alert(
                        &p,
                        "Remove this format?",
                        "The book keeps its other formats. The file is moved to the library's trash folder and can be put back with Undo.",
                        "Remove",
                        true,
                        do_remove,
                    ),
                    None => do_remove(),
                }
            });
            row.add_suffix(&remove);
        }
        group.add(&row);
    }
    Some(group)
}

/// Shows what *Merge as formats of one book* would do and merges when confirmed.
/// `ids[0]` is the book that stays; the others are folded into it.
pub fn show_merge(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    ids: Vec<u64>,
    changed: Rc<dyn Fn(String)>,
) {
    let Some((&primary, others)) = ids.split_first() else {
        return;
    };
    let plan = match shared.lib.borrow().plan_merge(primary, others) {
        Ok(p) => p,
        Err(e) => {
            changed(trf("Cannot merge: {}", &[&format!("{:#}", e)]));
            return;
        }
    };
    let lines = bf::preview_lines(&plan);
    eprintln!("formats: preview {}", lines.join(" | "));
    let can = !plan.mergeable().is_empty();
    let d = adw::AlertDialog::new(
        Some(&tr("Merge as formats of one book?")),
        Some(&lines.join("\n")),
    );
    d.add_response("cancel", &tr("Cancel"));
    if can {
        d.add_response("merge", &tr("Merge"));
        d.set_response_appearance("merge", adw::ResponseAppearance::Suggested);
    }
    let shared = shared.clone();
    d.connect_response(None, move |_, r| {
        if r != "merge" {
            return;
        }
        let res = {
            let guard = shared.guard.borrow();
            bf::merge(
                &mut shared.lib.borrow_mut(),
                &guard.store,
                primary,
                &others_of(&ids),
            )
        };
        match res {
            Ok(n) => changed(trf(
                "Merged {} as formats of one book. Undo takes it back.",
                &[&ntr("{} book", "{} books", n)],
            )),
            Err(e) => changed(trf("Could not merge: {}", &[&format!("{:#}", e)])),
        }
    });
    d.present(Some(parent));
}

fn others_of(ids: &[u64]) -> Vec<u64> {
    ids.iter().skip(1).copied().collect()
}

thread_local! {
    /// Developer script: answer the offer without asking (`add`, `separate` or `skip`).
    static AUTO_ANSWER: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// An import found files that are other formats of books the library has: ask once for all of them.
/// `import_separately` gets the books the user wants as separate books after all.
pub fn offer_formats(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    offers: Vec<(Offer, Prepared)>,
    changed: Rc<dyn Fn(String)>,
) {
    if offers.is_empty() {
        return;
    }
    let body = offers
        .iter()
        .take(8)
        .map(|(o, _)| {
            format!(
                "{} is another format ({}) of \"{}\"",
                o.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                o.format.to_uppercase(),
                o.existing_title
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let more = offers.len().saturating_sub(8);
    let d = adw::AlertDialog::new(
        Some(&tr("Add as another format?")),
        Some(&if more > 0 {
            format!("{body}\n… and {more} more")
        } else {
            body
        }),
    );
    d.add_response("skip", &tr("Skip"));
    d.add_response("separate", &tr("Import as Separate Books"));
    d.add_response("add", &tr("Add as Another Format"));
    d.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("add"));
    let shared = shared.clone();
    let offers = std::cell::RefCell::new(Some(offers));
    let respond = Rc::new(move |r: &str| {
        let Some(offers) = offers.borrow_mut().take() else {
            return;
        };
        let (mut added, mut separate, mut failed) = (0, 0, Vec::new());
        for (o, p) in offers {
            match r {
                "add" => match shared.lib.borrow_mut().add_format(o.existing, &o.path) {
                    Ok(_) => added += 1,
                    Err(e) => failed.push(format!("{}: {e:#}", o.path.display())),
                },
                "separate" => match shared.lib.borrow_mut().commit_import(p, true) {
                    Ok(_) => separate += 1,
                    Err(e) => failed.push(format!("{e:#}")),
                },
                _ => {}
            }
        }
        let _ = shared.lib.borrow().save();
        let mut msg = match (added, separate) {
            (0, 0) => String::new(),
            (a, 0) => trf(
                "Added {} as another format",
                &[&ntr("{} file", "{} files", a)],
            ),
            (0, s) => trf("Imported {} separately", &[&ntr("{} book", "{} books", s)]),
            (a, s) => trf("Added {} formats, imported {} separately", &[&a, &s]),
        };
        if !failed.is_empty() {
            msg = format!("{msg} {} failed: {}", failed.len(), failed[0]);
        }
        if !msg.is_empty() {
            changed(msg.trim().to_string());
        }
    });
    let auto = AUTO_ANSWER.with(|a| a.borrow().clone());
    if let Some(a) = auto {
        eprintln!("formats: offer answered {a}");
        respond(&a);
        return;
    }
    {
        let respond = respond.clone();
        d.connect_response(None, move |_, r| respond(r));
    }
    d.present(Some(parent));
}

fn dump(lib: &Library) {
    for b in &lib.books {
        eprintln!(
            "formats: book={} title={:?} files={}",
            b.id,
            b.meta.title,
            b.all_formats()
                .iter()
                .map(|f| f.format.clone())
                .collect::<Vec<_>>()
                .join(",")
        );
    }
}

/// `formats:<command>` of the developer script.
pub fn dev(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    changed: Rc<dyn Fn(String)>,
    cmd: &str,
) {
    let ids =
        |s: &str| -> Vec<u64> { s.split('|').filter_map(|x| x.trim().parse().ok()).collect() };
    match cmd {
        "dump" => dump(&shared.lib.borrow()),
        c if c.starts_with("offers=") => {
            AUTO_ANSWER.with(|a| *a.borrow_mut() = Some(c[7..].to_string()));
        }
        // the protected books and what the reader says about them
        "drm" => {
            let lib = shared.lib.borrow();
            for b in lib.books.iter().filter(|b| !b.drm.is_empty()) {
                eprintln!(
                    "formats: protected book={} title={:?} drm={:?} message={:?}",
                    b.id,
                    b.meta.title,
                    b.drm,
                    bf::protected_message(&b.drm)
                );
            }
            let q = crate::core::query::Query::parse("drm:");
            eprintln!(
                "formats: drm-search matches {}",
                lib.books.iter().filter(|b| q.matches(b)).count()
            );
        }
        // the rows of the Formats list of a book: `rows=ID`
        c if c.starts_with("rows=") => {
            let lib = shared.lib.borrow();
            if let Some(b) = c[5..].parse::<u64>().ok().and_then(|id| lib.get(id)) {
                for r in bf::rows(&lib, b) {
                    eprintln!(
                        "formats: row book={} format={} main={} exists={} in_bookx={}",
                        b.id,
                        r.format,
                        r.main,
                        r.exists,
                        r.opens_in_bookx()
                    );
                }
            }
        }
        // `retype=ID:EXT` renames a book's file (a book another tool filed as AZW3, for example)
        c if c.starts_with("retype=") => {
            let Some((id, ext)) = c[7..].split_once(':') else {
                return;
            };
            let Ok(id) = id.parse::<u64>() else { return };
            let mut lib = shared.lib.borrow_mut();
            let Some(old) = lib.get(id).map(|b| b.file.clone()) else {
                return;
            };
            let new = format!("{}.{ext}", old.trim_end_matches(".epub"));
            if std::fs::rename(lib.root.join(&old), lib.root.join(&new)).is_ok() {
                if let Some(b) = lib.get_mut(id) {
                    b.file = new;
                }
                let _ = lib.save();
            }
            drop(lib);
            changed(String::new());
        }
        // `merge-preview=1,2` shows the dialog, `merge=1,2` merges without asking
        c if c.starts_with("merge-preview=") => show_merge(parent, shared, ids(&c[14..]), changed),
        c if c.starts_with("merge=") => {
            let v = ids(&c[6..]);
            if let Some((p, o)) = v.split_first() {
                let res = {
                    let guard = shared.guard.borrow();
                    bf::merge(&mut shared.lib.borrow_mut(), &guard.store, *p, o)
                };
                match res {
                    Ok(n) => eprintln!("formats: merged {n}"),
                    Err(e) => eprintln!("formats: merge failed: {e:#}"),
                }
                changed("Merged".into());
            }
        }
        // `add=ID:/path/file.azw3`
        c if c.starts_with("add=") => {
            if let Some((id, path)) = c[4..].split_once(':') {
                let res = id
                    .parse::<u64>()
                    .map_err(anyhow::Error::from)
                    .and_then(|id| shared.lib.borrow_mut().add_format(id, &PathBuf::from(path)));
                match res {
                    Ok(f) => eprintln!("formats: added {}", f.format),
                    Err(e) => eprintln!("formats: add failed: {e:#}"),
                }
                changed("Added".into());
            }
        }
        c if c.starts_with("remove=") => {
            if let Some((id, fmt)) = c[7..].split_once(':') {
                let file = id.parse::<u64>().ok().and_then(|id| {
                    shared.lib.borrow().get(id).and_then(|b| {
                        b.formats
                            .iter()
                            .find(|f| f.format == fmt)
                            .map(|f| (id, f.file.clone()))
                    })
                });
                if let Some((id, file)) = file {
                    match shared.lib.borrow_mut().remove_format(id, &file) {
                        Ok(()) => eprintln!("formats: removed {fmt}"),
                        Err(e) => eprintln!("formats: remove failed: {e:#}"),
                    }
                    changed("Removed".into());
                }
            }
        }
        _ => {}
    }
}
