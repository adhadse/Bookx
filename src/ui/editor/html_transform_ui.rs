//! Tools → Styles → Transform HTML (CE-421): a rule builder for element and attribute changes ("rename b to
//! strong", "remove the div.ad elements", "wrap images in a div") with saved rule sets and the usual Preview of the
//! changed pages. The engine is `bookx_editing::transform_html`.

use crate::editor_structure as es;
use crate::i18n::{tr, trf};
use bookx_editing::transform_html::{HtmlRule, HtmlRuleKind, HtmlRuleSet};

use super::tools_ui::*;
use super::*;

type Rules = Rc<RefCell<Vec<HtmlRule>>>;

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
    pub(super) fn tool_transform_html(&self) {
        let rules: Rules = Rc::new(RefCell::new(Vec::new()));
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");
        rebuild_list(&list, &rules);

        let labels: Vec<String> = HtmlRuleKind::ALL
            .iter()
            .map(|k| tr(k.short_label()))
            .collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let kind = combo_row(&tr("New rule"), &refs, 0);
        let selector = entry_row(&tr("Elements (for example p.note, //div/p, #id)"), "");
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
        form.add(&selector);
        for f in &fields {
            form.add(f);
        }
        let show_fields = {
            let (kind, fields, form2) = (kind.clone(), fields.clone(), form.clone());
            Rc::new(move || {
                let k = HtmlRuleKind::ALL[kind.selected() as usize % HtmlRuleKind::ALL.len()];
                form2.set_description(Some(&glib::markup_escape_text(&tr(k.label()))));
                let fs = k.fields();
                for (i, row) in fields.iter().enumerate() {
                    match fs.get(i) {
                        Some((label, hint)) => {
                            row.set_visible(true);
                            row.set_title(&format!(
                                "{} ({} {})",
                                tr(label),
                                tr("for example"),
                                hint
                            ));
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
            let (rules, list, kind, fields, msg, selector) = (
                rules.clone(),
                list.clone(),
                kind.clone(),
                fields.clone(),
                msg.clone(),
                selector.clone(),
            );
            let s = show_fields.clone();
            add.connect_clicked(move |_| {
                let k = HtmlRuleKind::ALL[kind.selected() as usize % HtmlRuleKind::ALL.len()];
                let vals: Vec<String> = fields.iter().map(|f| f.text().to_string()).collect();
                match k.build(&selector.text(), &vals) {
                    Ok(r) => {
                        msg.set_text("");
                        rules.borrow_mut().push(r);
                        rebuild_list(&list, &rules);
                        selector.set_text("");
                        s();
                    }
                    Err(e) => msg.set_text(&format!("{e:#}")),
                }
            });
        }

        // saved rule sets
        let dir = es::html_rules_dir();
        let saved: Rc<RefCell<Vec<HtmlRuleSet>>> = Rc::new(RefCell::new(
            dir.as_deref()
                .map(es::list_html_rule_sets_in)
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
                        es::delete_html_rule_set_in(d, &n);
                        *saved.borrow_mut() = es::list_html_rule_sets_in(d);
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
                    let set = HtmlRuleSet {
                        name,
                        rules: rules.borrow().clone(),
                    };
                    match es::save_html_rule_set_in(d, &set) {
                        Ok(()) => {
                            msg.set_text("");
                            *saved.borrow_mut() = es::list_html_rule_sets_in(d);
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

        let scope = combo_row(
            &tr("Pages"),
            &[&tr("All pages"), &tr("The open page only")],
            0,
        );
        let scope_g = group("");
        scope_g.add(&scope);
        let rules_group = group(&tr("Rules, applied in this order"));
        rules_group.add(&scrolled(&list, 120));
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bx.append(&scope_g);
        bx.append(&rules_group);
        bx.append(&form);
        bx.append(&add);
        bx.append(&msg);
        bx.append(&sets);

        let cur = self
            .current
            .clone()
            .filter(|p| bookx_editing::markup::is_markup_path(p));
        let make_rules = rules.clone();
        let make: MakeOp = Rc::new(move || {
            let rs = make_rules.borrow().clone();
            if rs.is_empty() {
                return Err("Add at least one rule".to_string());
            }
            let paths: Vec<String> = if scope.selected() == 1 {
                vec![cur.clone().ok_or("Open a page of the book first")?]
            } else {
                Vec::new()
            };
            Ok((
                "Transform HTML".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| es::html_transform(b, &rs, &paths))
                    as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 780,
                ..Spec::new(
                    crate::i18n::N_("Transform HTML"),
                    crate::i18n::N_("Change elements and attributes in all pages at once. Preview lists the pages a rule set would change; nothing outside the tags you name is touched."),
                    crate::i18n::N_("Apply"),
                )
            },
            Some(bx.upcast()),
            make,
            None,
        );
    }

    /// `x:html-rules=<kind>|<selector>|<field>|<field>...;;<next rule>;;...` then optional `~~<scope>`
    /// (`~~current`, a page path or nothing for all pages); `~~preview` only lists the changes.
    pub(super) fn dev_html_rules(&mut self, arg: &str) {
        let (spec, scope) = arg.rsplit_once("~~").unwrap_or((arg, ""));
        let mut rules = Vec::new();
        for r in spec.split(";;").filter(|r| !r.is_empty()) {
            let mut it = r.split('|');
            let kind = it.next().and_then(HtmlRuleKind::from_id);
            let sel = it.next().unwrap_or("");
            let vals: Vec<String> = it.map(str::to_string).collect();
            match kind.map(|k| k.build(sel, &vals)) {
                Some(Ok(r)) => rules.push(r),
                Some(Err(e)) => {
                    self.toast(&format!("{e:#}"));
                    return;
                }
                None => {
                    self.toast(&tr("Unknown rule"));
                    return;
                }
            }
        }
        if rules.is_empty() {
            self.toast(&tr("Add at least one rule"));
            return;
        }
        let paths: Vec<String> = match scope {
            "" | "all" => Vec::new(),
            "current" => self.current.clone().into_iter().collect(),
            "preview" => {
                let rep = bookx_editing::transform_html::transform_book(
                    &mut OpenBook::from_archive(self.book.borrow().archive.clone()).unwrap(),
                    &rules,
                    &[],
                    true,
                );
                eprintln!(
                    "editor_x: html-preview: changed={} skipped={} total={}",
                    rep.changed.len(),
                    rep.skipped.len(),
                    rep.total()
                );
                return;
            }
            p => vec![p.to_string()],
        };
        self.run_op("Transform HTML", move |b, _| {
            es::html_transform(b, &rules, &paths)
        });
    }

    /// `x:html-ruleset=save|<name>|<same spec as html-rules>`, `list`, `load|<name>`, `delete|<name>`.
    pub(super) fn dev_html_ruleset(&mut self, arg: &str) {
        let Some(dir) = es::html_rules_dir() else {
            return;
        };
        let (cmd, rest) = arg.split_once('|').unwrap_or((arg, ""));
        match cmd {
            "save" => {
                let (name, spec) = rest.split_once('|').unwrap_or((rest, ""));
                let rules: Vec<HtmlRule> = spec
                    .split(";;")
                    .filter(|r| !r.is_empty())
                    .filter_map(|r| {
                        let mut it = r.split('|');
                        let k = it.next().and_then(HtmlRuleKind::from_id)?;
                        let sel = it.next().unwrap_or("");
                        let vals: Vec<String> = it.map(str::to_string).collect();
                        k.build(sel, &vals).ok()
                    })
                    .collect();
                let set = HtmlRuleSet {
                    name: name.into(),
                    rules,
                };
                if let Err(e) = es::save_html_rule_set_in(&dir, &set) {
                    self.toast(&format!("{e}"));
                }
            }
            "list" => eprintln!(
                "editor_x: html-rulesets: {}",
                es::list_html_rule_sets_in(&dir)
                    .iter()
                    .map(|s| format!("{}({})", s.name, s.rules.len()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            "load" => {
                if let Some(s) = es::list_html_rule_sets_in(&dir)
                    .into_iter()
                    .find(|s| s.name == rest)
                {
                    self.run_op("Transform HTML", move |b, _| {
                        es::html_transform(b, &s.rules, &[])
                    });
                }
            }
            "delete" => es::delete_html_rule_set_in(&dir, rest),
            _ => {}
        }
    }
}
