//! The *Saved Searches* dialog: list, add the bar's search, edit, delete, reorder, import/export JSON, load into
//! the bar, and run one or several in sequence (Count or Replace All).

use super::search_bar::SearchAct;
use super::tools_ui::{set_dialog_content, toast_in_dialog};
use super::*;
use crate::editor_search::Scope;
use crate::editor_search_modes::Mode;
use crate::editor_search_saved::{self as saved, Action, SavedSearch};
use crate::i18n::{tr, trf};

struct Manager {
    dialog: adw::Dialog,
    list: gtk::ListBox,
    items: RefCell<Vec<SavedSearch>>,
    /// The bar's search when the dialog opened.
    current: SavedSearch,
    functions: Vec<String>,
    tx: relm4::Sender<Input>,
}

fn window_of(w: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    w.root().and_downcast::<gtk::Window>()
}

impl Editor {
    pub(super) fn open_saved_dialog(&mut self) {
        let m = Rc::new(Manager {
            dialog: crate::ui::dialog_toast::new_dialog(),
            list: gtk::ListBox::new(),
            items: RefCell::new(self.w.sx.state.borrow().saved.clone()),
            current: self.current_saved(""),
            functions: self.w.sx.function_names(),
            tx: self.sender.input_sender().clone(),
        });
        m.dialog.set_title(&tr("Saved Searches"));
        m.dialog.set_content_width(600);
        m.dialog.set_content_height(560);
        m.list.add_css_class("boxed-list");
        m.list.set_selection_mode(gtk::SelectionMode::Multiple);
        crate::ui::audit::label(&m.list, &tr("Saved searches"));
        let hint = gtk::Label::new(Some(
            "Select one or more searches and run them in list order. Replacing takes a checkpoint first. Double-click a search to load it into the find bar.",
        ));
        hint.set_wrap(true);
        hint.set_xalign(0.0);
        hint.add_css_class("dim-label");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_child(Some(&m.list));

        let btn = |label: &str, tip: &str| {
            let b = gtk::Button::with_label(label);
            b.set_tooltip_text(Some(tip));
            b
        };
        let add = btn("Add Current", "Save the search that is in the find bar");
        let edit_b = btn("Edit", "Edit the selected search");
        let del = btn("Delete", "Delete the selected searches");
        del.add_css_class("destructive-action");
        let up = btn("Up", "Move the selected search up");
        let down = btn("Down", "Move the selected search down");
        let load = btn("Load", "Put the selected search into the find bar");
        let count = btn("Count", "Count the matches of the selected searches");
        let run = btn("Replace All", "Run the selected searches in order");
        run.add_css_class("suggested-action");
        let edit_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for b in [&add, &edit_b, &up, &down, &load, &del] {
            edit_row.append(b);
        }
        let run_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        run_row.set_halign(gtk::Align::End);
        run_row.append(&count);
        run_row.append(&run);

        let import = gtk::Button::from_icon_name("document-open-symbolic");
        import.set_tooltip_text(Some(&tr("Import searches from a JSON file")));
        let export = gtk::Button::from_icon_name("document-save-symbolic");
        export.set_tooltip_text(Some(&tr("Export all searches to a JSON file")));
        let hb = adw::HeaderBar::new();
        hb.pack_start(&import);
        hb.pack_start(&export);

        let col = gtk::Box::new(gtk::Orientation::Vertical, 10);
        col.set_margin_all(12);
        col.append(&hint);
        col.append(&scroll);
        col.append(&edit_row);
        col.append(&run_row);
        let tb = adw::ToolbarView::new();
        tb.add_top_bar(&hb);
        tb.set_content(Some(&col));
        set_dialog_content(&m.dialog, &tb);

        let selected = {
            let m = m.clone();
            move || -> Vec<usize> {
                m.list
                    .selected_rows()
                    .iter()
                    .map(|r| r.index() as usize)
                    .collect()
            }
        };
        {
            let m2 = m.clone();
            add.connect_clicked(move |_| {
                let mut s = m2.current.clone();
                if s.settings.find.is_empty() {
                    return;
                }
                let name = {
                    let it = m2.items.borrow();
                    saved::unique_name(&it, &s.settings.find.chars().take(24).collect::<String>())
                };
                s.name = name;
                saved::upsert(&mut m2.items.borrow_mut(), s);
                changed(&m2);
            });
            let (m2, sel) = (m.clone(), selected.clone());
            edit_b.connect_clicked(move |_| {
                if let Some(i) = sel().first() {
                    edit_one(&m2, Some(*i));
                }
            });
            let (m2, sel) = (m.clone(), selected.clone());
            del.connect_clicked(move |_| {
                let mut idx = sel();
                idx.sort_unstable_by(|a, b| b.cmp(a));
                for i in idx {
                    if i < m2.items.borrow().len() {
                        m2.items.borrow_mut().remove(i);
                    }
                }
                changed(&m2);
            });
            for (b, step) in [(&up, -1), (&down, 1)] {
                let (m2, sel) = (m.clone(), selected.clone());
                b.connect_clicked(move |_| {
                    if let Some(i) = sel().first().copied() {
                        let j = saved::move_item(&mut m2.items.borrow_mut(), i, step);
                        changed(&m2);
                        if let Some(r) = m2.list.row_at_index(j as i32) {
                            m2.list.select_row(Some(&r));
                        }
                    }
                });
            }
            let (m2, sel) = (m.clone(), selected.clone());
            load.connect_clicked(move |_| {
                if let Some(i) = sel().first() {
                    load_into_bar(&m2, *i);
                }
            });
            let m2 = m.clone();
            m.list.connect_row_activated(move |_, r| {
                load_into_bar(&m2, r.index() as usize);
            });
            for (b, action) in [(&count, Action::Count), (&run, Action::ReplaceAll)] {
                let (m2, sel) = (m.clone(), selected.clone());
                b.connect_clicked(move |_| {
                    let items = m2.items.borrow();
                    let mut idx = sel();
                    idx.sort_unstable();
                    let list: Vec<SavedSearch> =
                        idx.iter().filter_map(|i| items.get(*i).cloned()).collect();
                    drop(items);
                    m2.tx.emit(Input::Search(SearchAct::RunSaved(list, action)));
                    m2.dialog.close();
                });
            }
            let m2 = m.clone();
            import.connect_clicked(move |b| {
                let fd = gtk::FileDialog::new();
                fd.set_title(&tr("Import searches"));
                let m3 = m2.clone();
                fd.open(window_of(b).as_ref(), gio::Cancellable::NONE, move |res| {
                    let Ok(file) = res else { return };
                    let Some(path) = file.path() else { return };
                    match std::fs::read_to_string(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|t| saved::parse_json(&t))
                    {
                        Ok(list) => {
                            saved::merge_imported(&mut m3.items.borrow_mut(), list);
                            changed(&m3);
                        }
                        Err(e) => toast_in_dialog(
                            &m3.dialog,
                            &trf(
                                "Could not import: that file is not a list of saved searches ({})",
                                &[&e],
                            ),
                        ),
                    }
                });
            });
            let m2 = m.clone();
            export.connect_clicked(move |b| {
                let fd = gtk::FileDialog::new();
                fd.set_title(&tr("Export searches"));
                fd.set_initial_name(Some("saved-searches.json"));
                let m3 = m2.clone();
                fd.save(window_of(b).as_ref(), gio::Cancellable::NONE, move |res| {
                    if let Some(path) = res.ok().and_then(|f| f.path()) {
                        let _ = std::fs::write(path, saved::to_json(&m3.items.borrow()));
                    }
                });
            });
        }
        refresh(&m);
        m.dialog.present(Some(&self.w.win));
    }
}

fn load_into_bar(m: &Rc<Manager>, i: usize) {
    if let Some(s) = m.items.borrow().get(i).cloned() {
        m.tx.emit(Input::Search(SearchAct::Load(Box::new(s))));
        m.dialog.close();
    }
}

fn changed(m: &Rc<Manager>) {
    m.tx.emit(Input::Search(SearchAct::SavedChanged(
        m.items.borrow().clone(),
    )));
    refresh(m);
}

fn refresh(m: &Rc<Manager>) {
    while let Some(c) = m.list.first_child() {
        m.list.remove(&c);
    }
    let items = m.items.borrow();
    if items.is_empty() {
        let l = gtk::Label::new(Some(
            "No saved searches yet. Type a search in the find bar, then choose Add Current.",
        ));
        l.set_wrap(true);
        l.add_css_class("dim-label");
        l.set_margin_all(16);
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_child(Some(&l));
        m.list.append(&row);
        return;
    }
    for s in items.iter() {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&s.name))
            .subtitle(glib::markup_escape_text(&s.summary()))
            .activatable(true)
            .build();
        row.set_subtitle_lines(1);
        m.list.append(&row);
    }
}

/// Add (`None`) or edit one search.
fn edit_one(m: &Rc<Manager>, index: Option<usize>) {
    let cur = index
        .and_then(|i| m.items.borrow().get(i).cloned())
        .unwrap_or_else(|| m.current.clone());
    let dialog = crate::ui::dialog_toast::new_dialog();
    dialog.set_title(&tr("Edit Saved Search"));
    dialog.set_content_width(520);
    let group = adw::PreferencesGroup::new();
    let entry = |title: &str, text: &str| {
        let r = adw::EntryRow::new();
        r.set_title(title);
        r.set_text(text);
        r
    };
    let name = entry(&tr("Name"), &cur.name);
    let find = entry(&tr("Find"), &cur.settings.find);
    let replace = entry(&tr("Replace with"), &cur.settings.replace);
    let combo = |title: &str, items: &[&str], sel: usize| {
        let r = adw::ComboRow::new();
        r.set_title(title);
        r.set_model(Some(&gtk::StringList::new(items)));
        r.set_selected(sel as u32);
        r
    };
    let mode = combo(
        "Mode",
        &Mode::ALL.map(|x| x.label()),
        cur.settings.mode.index() as usize,
    );
    let scope = combo(
        "Search in",
        &Scope::ALL.map(|x| x.label()),
        Scope::ALL.iter().position(|x| *x == cur.scope).unwrap_or(0),
    );
    let fnames: Vec<&str> = m.functions.iter().map(String::as_str).collect();
    let func = combo(
        "Function",
        &fnames,
        m.functions
            .iter()
            .position(|n| *n == cur.function)
            .unwrap_or(0),
    );
    let sw = |title: &str, on: bool| {
        let r = adw::SwitchRow::new();
        r.set_title(title);
        r.set_active(on);
        r
    };
    let case = sw("Match case", cur.settings.case_sensitive);
    let dotall = sw("Dot matches line breaks", cur.settings.dot_all);
    let word = sw("Whole words", cur.settings.whole_word);
    let markup = sw("Ignore HTML markup", cur.settings.ignore_markup);
    for r in [&name, &find, &replace] {
        group.add(r);
    }
    group.add(&mode);
    group.add(&scope);
    group.add(&func);
    for r in [&case, &dotall, &word, &markup] {
        group.add(r);
    }
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let hb = adw::HeaderBar::new();
    hb.set_show_end_title_buttons(false);
    hb.set_show_start_title_buttons(false);
    hb.pack_start(&cancel);
    hb.pack_end(&save);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    let tb = adw::ToolbarView::new();
    tb.add_top_bar(&hb);
    tb.set_content(Some(&page));
    dialog.set_child(Some(&tb));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });
    let (m2, d) = (m.clone(), dialog.clone());
    save.connect_clicked(move |_| {
        let mode_v = Mode::from_index(mode.selected());
        let s = SavedSearch {
            name: name.text().trim().to_string(),
            settings: crate::editor_search_modes::Settings {
                find: find.text().to_string(),
                replace: replace.text().to_string(),
                mode: mode_v,
                case_sensitive: case.is_active(),
                dot_all: dotall.is_active(),
                whole_word: word.is_active(),
                ignore_markup: markup.is_active(),
            },
            scope: Scope::ALL
                .get(scope.selected() as usize)
                .copied()
                .unwrap_or(Scope::ThisFile),
            function: if mode_v == Mode::Function {
                m2.functions
                    .get(func.selected() as usize)
                    .cloned()
                    .unwrap_or_default()
            } else {
                String::new()
            },
        };
        if s.name.is_empty() || s.settings.find.is_empty() {
            return;
        }
        {
            let mut items = m2.items.borrow_mut();
            match index.filter(|i| *i < items.len()) {
                Some(i) => items[i] = s,
                None => {
                    let mut s = s;
                    s.name = saved::unique_name(&items, &s.name);
                    items.push(s);
                }
            }
        }
        changed(&m2);
        d.close();
    });
    dialog.present(Some(&m.dialog));
}
