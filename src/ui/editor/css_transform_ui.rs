//! Tools → Styles → Transform Styles: a small rule builder ("if color is red, change it to green", "multiply
//! every font size by 1.2", "make Georgia italic") with saved rule sets (CE-133; the roadmap's CSS transforms UI).

use crate::i18n::{tr, trf};
use bookx_editing::transform_css::{CssTransform, RuleSet};

use super::tools_ui::*;
use super::*;
use crate::editor_state;
use crate::editor_tools::rules::RuleKind;

type Rules = Rc<RefCell<Vec<CssTransform>>>;

fn rebuild_list(list: &gtk::ListBox, rules: &Rules) {
    list.remove_all();
    let n = rules.borrow().len();
    if n == 0 {
        let row = empty_rules_row(list);
        list.append(&row);
        return;
    }
    for i in 0..n {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&rules.borrow()[i].describe()));
        row.set_activatable(false);
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.set_valign(gtk::Align::Center);
        del.add_css_class("flat");
        del.set_tooltip_text(Some(&tr("Remove this rule")));
        crate::ui::audit::label(&del, &tr("Remove this rule"));
        let (rules, list2) = (rules.clone(), list.clone());
        del.connect_clicked(move |_| {
            if i < rules.borrow().len() {
                rules.borrow_mut().remove(i);
            }
            rebuild_list(&list2, &rules);
        });
        row.add_suffix(&del);
        list.append(&row);
    }
}

impl Editor {
    pub(super) fn tool_transform_styles(&self) {
        let rules: Rules = Rc::new(RefCell::new(Vec::new()));
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        rebuild_list(&list, &rules);

        // the form for one new rule
        let kinds_owned: Vec<String> = RuleKind::ALL.iter().map(|k| tr(k.short_label())).collect();
        let kinds: Vec<&str> = kinds_owned.iter().map(String::as_str).collect();
        let kind = combo_row(&tr("New rule"), &kinds, 0);
        let fields: Vec<adw::EntryRow> = (0..3).map(|_| entry_row("", "")).collect();
        let add = gtk::Button::with_label(&tr("Add Rule"));
        add.set_halign(gtk::Align::End);
        add.add_css_class("pill");
        let msg = gtk::Label::new(None);
        msg.set_xalign(0.0);
        msg.add_css_class("error");
        msg.set_wrap(true);
        let form = group(&tr("Add a rule"));
        form.add(&kind);
        for f in &fields {
            form.add(f);
        }
        let show_fields = {
            let (kind, fields, form2) = (kind.clone(), fields.clone(), form.clone());
            Rc::new(move || {
                let k = RuleKind::ALL[kind.selected() as usize % RuleKind::ALL.len()];
                form2.set_description(Some(&glib::markup_escape_text(&tr(k.label()))));
                let fs = k.fields();
                for (i, row) in fields.iter().enumerate() {
                    match fs.get(i) {
                        Some(f) => {
                            row.set_visible(true);
                            row.set_title(&format!("{} (e.g. {})", f.label, f.hint));
                        }
                        None => row.set_visible(false),
                    }
                    row.set_text("");
                }
            })
        };
        show_fields();
        {
            let s = show_fields.clone();
            kind.connect_selected_notify(move |_| s());
        }
        {
            let (rules, list, kind, fields, msg) = (
                rules.clone(),
                list.clone(),
                kind.clone(),
                fields.clone(),
                msg.clone(),
            );
            let s = show_fields.clone();
            add.connect_clicked(move |_| {
                let k = RuleKind::ALL[kind.selected() as usize % RuleKind::ALL.len()];
                let vals: Vec<String> = fields.iter().map(|f| f.text().to_string()).collect();
                match k.build(&vals) {
                    Ok(r) => {
                        msg.set_text("");
                        rules.borrow_mut().push(r);
                        rebuild_list(&list, &rules);
                        s();
                    }
                    Err(e) => msg.set_text(&e),
                }
            });
        }

        // saved rule sets
        let dir = editor_state::rules_dir();
        let saved: Rc<RefCell<Vec<RuleSet>>> = Rc::new(RefCell::new(
            dir.as_deref()
                .map(editor_state::list_rule_sets_in)
                .unwrap_or_default(),
        ));
        let sets_row = adw::ComboRow::new();
        sets_row.set_title(&tr("Saved rule sets"));
        let refill = {
            let (sets_row, saved) = (sets_row.clone(), saved.clone());
            Rc::new(move || {
                let n = std::iter::once(tr("None chosen"))
                    .chain(saved.borrow().iter().map(|x| x.name.clone()))
                    .collect::<Vec<_>>();
                let refs: Vec<&str> = n.iter().map(String::as_str).collect();
                sets_row.set_model(Some(&gtk::StringList::new(&refs)));
            })
        };
        refill();
        let load = gtk::Button::with_label(&tr("Load"));
        load.set_valign(gtk::Align::Center);
        let delete = gtk::Button::with_label(&tr("Delete"));
        delete.set_valign(gtk::Align::Center);
        sets_row.add_suffix(&load);
        sets_row.add_suffix(&delete);
        let save_name = entry_row(&tr("Save the rules above as"), "");
        let save = gtk::Button::with_label(&tr("Save"));
        save.set_valign(gtk::Align::Center);
        save_name.add_suffix(&save);
        {
            let (rules, list, saved, sets_row) =
                (rules.clone(), list.clone(), saved.clone(), sets_row.clone());
            load.connect_clicked(move |_| {
                let i = sets_row.selected() as usize;
                if i > 0 {
                    if let Some(s) = saved.borrow().get(i - 1) {
                        *rules.borrow_mut() = s.rules.clone();
                    }
                    rebuild_list(&list, &rules);
                }
            });
        }
        {
            let (saved, sets_row, refill, dir) =
                (saved.clone(), sets_row.clone(), refill.clone(), dir.clone());
            delete.connect_clicked(move |_| {
                let i = sets_row.selected() as usize;
                if i > 0 {
                    let name = saved.borrow().get(i - 1).map(|s| s.name.clone());
                    if let (Some(n), Some(d)) = (name, &dir) {
                        editor_state::delete_rule_set_in(d, &n);
                        *saved.borrow_mut() = editor_state::list_rule_sets_in(d);
                        refill();
                    }
                }
            });
        }
        {
            let (rules, saved, refill, dir, save_name, msg) = (
                rules.clone(),
                saved.clone(),
                refill.clone(),
                dir.clone(),
                save_name.clone(),
                msg.clone(),
            );
            save.connect_clicked(move |_| {
                let name = save_name.text().trim().to_string();
                if name.is_empty() || rules.borrow().is_empty() {
                    msg.set_text(&tr("Add some rules and give the set a name first"));
                    return;
                }
                if let Some(d) = &dir {
                    let set = RuleSet {
                        name,
                        rules: rules.borrow().clone(),
                    };
                    match editor_state::save_rule_set_in(d, &set) {
                        Ok(()) => {
                            msg.set_text("");
                            *saved.borrow_mut() = editor_state::list_rule_sets_in(d);
                            refill();
                        }
                        Err(e) => msg.set_text(&trf("Could not save: {}", &[&e])),
                    }
                }
            });
        }
        let sets = group("");
        sets.add(&sets_row);
        sets.add(&save_name);

        let rules_group = group(&tr("Rules, applied in this order"));
        rules_group.add(&scrolled(&list, 130));
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bx.append(&rules_group);
        bx.append(&form);
        bx.append(&add);
        bx.append(&msg);
        bx.append(&sets);

        let make_rules = rules.clone();
        let make: MakeOp = Rc::new(move || {
            let rs = make_rules.borrow().clone();
            if rs.is_empty() {
                return Err("Add at least one rule".to_string());
            }
            Ok((
                "Transform Styles".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| {
                    editor_tools::transform_styles(b, &rs)
                }) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 760,
                ..Spec::new(
                    crate::i18n::N_("Transform Styles"),
                    crate::i18n::N_("Change how the book looks without editing every stylesheet by hand. The rules apply to stylesheets, <style> blocks and style attributes."),
                    crate::i18n::N_("Apply"),
                )
            },
            Some(bx.upcast()),
            make,
            None,
        );
    }
}
