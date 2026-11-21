//! The "Spelling Dictionaries" dialog (CE-275, CE-279): the Hunspell dictionaries that are installed (and importing
//! a `.dic`/`.aff` pair or a LibreOffice `.oxt`), and the named user dictionaries with their words.

use super::spelling::Msg;
use super::*;
use crate::i18n::{tr, trf};
use bookx_editing::spell_import as imp;
use bookx_editing::spell_user::{self as ud, UserDicts};

/// Words listed per user dictionary in the dialog (a long list would make the dialog slow).
const WORDS_SHOWN: usize = 150;

fn user_dir() -> Option<std::path::PathBuf> {
    crate::editor_spell_dicts::user_dictionary_dir()
}

impl Editor {
    pub(super) fn spell_dictionaries_dialog(&mut self) {
        self.ensure_spell();
        let dialog = adw::Dialog::new();
        dialog.set_title(&tr("Spelling Dictionaries"));
        dialog.set_content_width(520);
        dialog.set_content_height(620);
        let page = adw::PreferencesPage::new();

        let installed = adw::PreferencesGroup::new();
        installed.set_title(&tr("Hunspell dictionaries"));
        installed.set_description(Some(
            &tr("Found on this computer and in your own folder. Import a .dic and .aff pair, or a LibreOffice extension (.oxt)."),
        ));
        let imp_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let b_pair = gtk::Button::with_label(&tr("Import .dic"));
        let b_oxt = gtk::Button::with_label(&tr("Import .oxt"));
        imp_box.append(&b_pair);
        imp_box.append(&b_oxt);
        installed.set_header_suffix(Some(&imp_box));
        let installed_list = gtk::ListBox::new();
        installed_list.add_css_class("boxed-list");
        installed_list.set_selection_mode(gtk::SelectionMode::None);
        installed.add(&installed_list);

        let mine = adw::PreferencesGroup::new();
        mine.set_title(&tr("Your dictionaries"));
        mine.set_description(Some(
            &tr("Words you added. Choose which ones are used; the Personal dictionary cannot be deleted."),
        ));
        let mine_list = gtk::ListBox::new();
        mine_list.add_css_class("boxed-list");
        mine_list.set_selection_mode(gtk::SelectionMode::None);
        let new_row = adw::EntryRow::new();
        new_row.set_title(&tr("New dictionary name"));
        new_row.set_show_apply_button(true);
        mine.add(&mine_list);
        mine.add(&new_row);
        page.add(&installed);
        page.add(&mine);

        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));

        let tx = self.sender.input_sender().clone();
        let refill: Rc<RefCell<Option<std::rc::Weak<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let fill = {
            let (il, ml, tx, refill) = (
                installed_list.clone(),
                mine_list.clone(),
                tx.clone(),
                refill.clone(),
            );
            Rc::new(move || {
                let again = {
                    let r = refill.clone();
                    move || {
                        if let Some(f) = r.borrow().as_ref().and_then(|w| w.upgrade()) {
                            f();
                        }
                    }
                };
                fill_installed(&il, &tx, again.clone());
                fill_mine(&ml, &tx, again);
            }) as Rc<dyn Fn()>
        };
        *refill.borrow_mut() = Some(Rc::downgrade(&fill));
        fill();

        {
            let (fill2, tx2, win) = (fill.clone(), tx.clone(), self.w.win.clone());
            b_pair.connect_clicked(move |_| {
                pick_file(
                    &win,
                    "Import a Hunspell Dictionary",
                    "Hunspell dictionary",
                    "*.dic",
                    {
                        let (fill, tx) = (fill2.clone(), tx2.clone());
                        move |p| report(import_pair(&p), &tx, &fill)
                    },
                );
            });
            let (fill2, tx2, win) = (fill.clone(), tx.clone(), self.w.win.clone());
            b_oxt.connect_clicked(move |_| {
                pick_file(
                    &win,
                    "Import a LibreOffice Extension",
                    "LibreOffice extension",
                    "*.oxt",
                    {
                        let (fill, tx) = (fill2.clone(), tx2.clone());
                        move |p| report(import_oxt(&p), &tx, &fill)
                    },
                );
            });
            let (fill2, tx2) = (fill.clone(), tx.clone());
            new_row.connect_apply(move |r| {
                let name = r.text().to_string();
                match user_dicts().map(|d| d.create(&name)) {
                    Some(Ok(())) => {
                        r.set_text("");
                        tx2.emit(Input::SpellPanel(Msg::UserWordsChanged));
                        fill2();
                    }
                    Some(Err(e)) => {
                        r.add_css_class("error");
                        r.set_tooltip_text(Some(&format!("{e:#}")));
                    }
                    None => {}
                }
            });
        }
        dialog.present(Some(&self.w.win));
    }

    /// Dev hooks without a file chooser: `new`, `remove` (a user dictionary), `oxt`, `pair` (import from a path).
    pub(super) fn dev_dict_op(&mut self, op: &str, arg: &str) {
        let res: anyhow::Result<String> = match op {
            "new" => user_dicts()
                .ok_or_else(|| anyhow::anyhow!("no configuration folder"))
                .and_then(|d| d.create(arg))
                .map(|_| format!("created {arg}")),
            "remove" => user_dicts()
                .ok_or_else(|| anyhow::anyhow!("no configuration folder"))
                .and_then(|d| d.remove(arg))
                .map(|_| format!("removed {arg}")),
            "oxt" => import_oxt(std::path::Path::new(arg)),
            "pair" => import_pair(std::path::Path::new(arg)),
            _ => Ok(String::new()),
        };
        match res {
            Ok(m) => {
                eprintln!("editor_spell_dict: ok {m}");
                self.spell_panel(if matches!(op, "new" | "remove") {
                    Msg::UserWordsChanged
                } else {
                    Msg::DictionariesChanged
                });
            }
            Err(e) => eprintln!("editor_spell_dict: error {e:#}"),
        }
    }
}

fn user_dicts() -> Option<UserDicts> {
    crate::editor_spell_dicts::user_dicts()
}

fn import_pair(p: &std::path::Path) -> anyhow::Result<String> {
    let dest = user_dir().ok_or_else(|| anyhow::anyhow!("no folder for dictionaries"))?;
    let i = imp::import_pair(p, &dest)?;
    Ok(format!("{} ({} words)", i.lang, i.words))
}

fn import_oxt(p: &std::path::Path) -> anyhow::Result<String> {
    let dest = user_dir().ok_or_else(|| anyhow::anyhow!("no folder for dictionaries"))?;
    let v = imp::import_oxt(p, &dest)?;
    Ok(v.iter()
        .map(|i| i.lang.clone())
        .collect::<Vec<_>>()
        .join(", "))
}

fn report(res: anyhow::Result<String>, tx: &relm4::Sender<Input>, fill: &Rc<dyn Fn()>) {
    match res {
        Ok(m) => {
            tx.emit(Input::SpellPanel(Msg::DictionariesChanged));
            tx.emit(Input::SpellPanel(Msg::Toast(trf("Imported {}", &[&m]))));
        }
        Err(e) => tx.emit(Input::SpellPanel(Msg::Toast(trf(
            "Could not import: {}",
            &[&format!("{:#}", e)],
        )))),
    }
    fill();
}

fn pick_file(
    win: &adw::Window,
    title: &str,
    filter_name: &str,
    pattern: &str,
    done: impl Fn(std::path::PathBuf) + 'static,
) {
    let f = gtk::FileFilter::new();
    f.set_name(Some(filter_name));
    f.add_pattern(pattern);
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&f);
    let dlg = gtk::FileDialog::builder()
        .title(title)
        .filters(&filters)
        .build();
    dlg.open(Some(win), gio::Cancellable::NONE, move |r| {
        if let Some(p) = r.ok().and_then(|f| f.path()) {
            done(p);
        }
    });
}

fn clear(list: &gtk::ListBox) {
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
}

fn fill_installed(
    list: &gtk::ListBox,
    tx: &relm4::Sender<Input>,
    again: impl Fn() + Clone + 'static,
) {
    clear(list);
    let dirs = crate::editor_spell_dicts::search_dirs();
    let own = user_dir();
    let all = imp::list(&dirs);
    if all.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No dictionaries installed"))
            .subtitle(tr(
                "Install hunspell dictionaries with your system, or import one here",
            ))
            .build();
        list.append(&row);
    }
    for i in all {
        let is_own = own.as_deref().is_some_and(|d| i.aff.parent() == Some(d));
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&i.lang))
            .subtitle(trf(
                "{} words · {}",
                &[
                    &i.words,
                    &(if is_own { "imported by you" } else { "system" }),
                ],
            ))
            .build();
        if is_own {
            let b = gtk::Button::from_icon_name("user-trash-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&trf("Remove the dictionary {}", &[&i.lang])));
            b.update_property(&[gtk::accessible::Property::Label(&trf(
                "Remove the dictionary {}",
                &[&i.lang],
            ))]);
            let (tx, again, lang, dir) = (tx.clone(), again.clone(), i.lang.clone(), own.clone());
            b.connect_clicked(move |_| {
                if let Some(d) = &dir {
                    let _ = imp::remove(d, &lang);
                }
                tx.emit(Input::SpellPanel(Msg::DictionariesChanged));
                again();
            });
            row.add_suffix(&b);
        }
        list.append(&row);
    }
}

fn fill_mine(list: &gtk::ListBox, tx: &relm4::Sender<Input>, again: impl Fn() + Clone + 'static) {
    clear(list);
    let Some(dicts) = user_dicts() else { return };
    for d in dicts.all() {
        let row = adw::ExpanderRow::new();
        row.set_title(&glib::markup_escape_text(&d.name));
        row.set_subtitle(&trf("{} words", &[&d.words.len()]));
        let sw = gtk::Switch::new();
        sw.set_active(d.enabled);
        sw.set_valign(gtk::Align::Center);
        sw.set_tooltip_text(Some(&trf("Use the dictionary {}", &[&d.name])));
        sw.update_property(&[gtk::accessible::Property::Label(&trf(
            "Use the dictionary {}",
            &[&d.name],
        ))]);
        {
            let (tx, name) = (tx.clone(), d.name.clone());
            sw.connect_active_notify(move |s| {
                if let Some(u) = user_dicts() {
                    let _ = u.set_enabled(&name, s.is_active());
                }
                tx.emit(Input::SpellPanel(Msg::UserWordsChanged));
            });
        }
        row.add_suffix(&sw);
        if d.name != ud::PERSONAL {
            let b = gtk::Button::from_icon_name("user-trash-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&trf("Delete the dictionary {}", &[&d.name])));
            b.update_property(&[gtk::accessible::Property::Label(&trf(
                "Delete the dictionary {}",
                &[&d.name],
            ))]);
            let (tx, again, name) = (tx.clone(), again.clone(), d.name.clone());
            b.connect_clicked(move |_| {
                if let Some(u) = user_dicts() {
                    let _ = u.remove(&name);
                }
                tx.emit(Input::SpellPanel(Msg::UserWordsChanged));
                again();
            });
            row.add_suffix(&b);
        }
        let add = adw::EntryRow::new();
        add.set_title(&tr("Add a word"));
        add.set_show_apply_button(true);
        {
            let (tx, again, name) = (tx.clone(), again.clone(), d.name.clone());
            add.connect_apply(move |r| {
                if let Some(u) = user_dicts() {
                    let _ = u.add_word(&name, r.text().trim());
                }
                tx.emit(Input::SpellPanel(Msg::UserWordsChanged));
                again();
            });
        }
        row.add_row(&add);
        for w in d.words.iter().take(WORDS_SHOWN) {
            let wr = adw::ActionRow::builder()
                .title(glib::markup_escape_text(w))
                .build();
            let b = gtk::Button::from_icon_name("list-remove-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&trf("Remove {} from {}", &[&w, &d.name])));
            b.update_property(&[gtk::accessible::Property::Label(&trf(
                "Remove {} from {}",
                &[&w, &d.name],
            ))]);
            let (tx, again, name, word) = (tx.clone(), again.clone(), d.name.clone(), w.clone());
            b.connect_clicked(move |_| {
                if let Some(u) = user_dicts() {
                    let _ = u.remove_word(&name, &word);
                }
                tx.emit(Input::SpellPanel(Msg::UserWordsChanged));
                again();
            });
            wr.add_suffix(&b);
            row.add_row(&wr);
        }
        list.append(&row);
    }
}
