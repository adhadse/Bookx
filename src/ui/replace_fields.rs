//! *Search and Replace* for the metadata of the selected books (roadmap L16): choose a field, the
//! text to find and its replacement, see what would change, then apply it as one undo step.
//! The work is `bookx_core::replace`; this file is the dialog.

use crate::i18n::{tr, trf};
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use crate::core::replace::{self, Rule, Target};

/// How many changes the preview lists.
const PREVIEW_ROWS: usize = 12;

fn target_title(t: &Target, shared: &SharedRef) -> String {
    match t {
        Target::Title => tr("Title"),
        Target::Authors => tr("Authors"),
        Target::Series => tr("Series"),
        Target::Publisher => tr("Publisher"),
        Target::Language => tr("Language"),
        Target::Tags => tr("Tags"),
        Target::Comments => tr("Comments"),
        Target::Custom(l) => shared
            .lib
            .borrow()
            .column(l)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("#{l}")),
    }
}

/// The fields a rule can work on: the built-in ones and the custom columns that hold text or numbers.
fn targets(shared: &SharedRef) -> Vec<Target> {
    let mut v: Vec<Target> = Target::BUILTIN.to_vec();
    v.extend(
        shared
            .lib
            .borrow()
            .columns()
            .iter()
            .filter(|c| c.kind != crate::core::custom::ColumnKind::Composite)
            .map(|c| Target::Custom(c.label.clone())),
    );
    v
}

/// Opens the dialog for `ids`. `done` runs after a replacement was applied, with a message for a toast.
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    shared: &SharedRef,
    ids: Vec<u64>,
    done: Rc<dyn Fn(String)>,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Search and Replace"));
    dialog.set_content_width(560);
    dialog.set_content_height(620);

    let page = adw::PreferencesPage::new();
    let rule_group = adw::PreferencesGroup::builder()
        .description(trf("Applies to {} selected books", &[&ids.len()]))
        .build();
    let list = targets(shared);
    let names: Vec<String> = list.iter().map(|t| target_title(t, shared)).collect();
    let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let field = adw::ComboRow::builder()
        .title(tr("Field"))
        .model(&gtk::StringList::new(&name_refs))
        .build();
    field.set_widget_name("replace-field");
    let find = adw::EntryRow::builder().title(tr("Search for")).build();
    find.set_widget_name("replace-find");
    let with = adw::EntryRow::builder().title(tr("Replace with")).build();
    with.set_widget_name("replace-with");
    let regex = adw::SwitchRow::builder()
        .title(tr("Regular expression"))
        .subtitle(tr("Use $1 in the replacement for the first group"))
        .build();
    regex.set_widget_name("replace-regex");
    let case = adw::SwitchRow::builder().title(tr("Match case")).build();
    case.set_widget_name("replace-case");
    for w in [
        field.upcast_ref::<gtk::Widget>(),
        find.upcast_ref(),
        with.upcast_ref(),
        regex.upcast_ref(),
        case.upcast_ref(),
    ] {
        rule_group.add(w);
    }
    page.add(&rule_group);

    let preview_group = adw::PreferencesGroup::builder()
        .title(tr("Preview"))
        .build();
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    summary.add_css_class("dim-label");
    summary.set_widget_name("replace-summary");
    preview_group.add(&summary);
    let rows: Rc<std::cell::RefCell<Vec<adw::ActionRow>>> = Rc::default();
    page.add(&preview_group);

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let apply = gtk::Button::with_label(&tr("Replace"));
    apply.add_css_class("suggested-action");
    apply.set_widget_name("replace-apply");
    apply.set_sensitive(false);
    header.pack_start(&cancel);
    header.pack_end(&apply);
    toolbar.add_top_bar(&header);
    let overlay = super::dialog_toast::overlay(&page);
    toolbar.set_content(Some(&overlay));
    dialog.set_child(Some(&toolbar));

    let current_rule: Rc<dyn Fn() -> Rule> = {
        let (field, find, with, regex, case, list) = (
            field.clone(),
            find.clone(),
            with.clone(),
            regex.clone(),
            case.clone(),
            list.clone(),
        );
        Rc::new(move || Rule {
            target: list
                .get(field.selected() as usize)
                .cloned()
                .unwrap_or(Target::Title),
            find: find.text().to_string(),
            replace: with.text().to_string(),
            regex: regex.is_active(),
            case_sensitive: case.is_active(),
        })
    };

    let refresh: Rc<dyn Fn()> = {
        let (shared, ids, summary, preview_group, rows, apply, current_rule) = (
            shared.clone(),
            ids.clone(),
            summary.clone(),
            preview_group.clone(),
            rows.clone(),
            apply.clone(),
            current_rule.clone(),
        );
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                preview_group.remove(&r);
            }
            let rule = current_rule();
            if rule.find.is_empty() {
                summary.set_label(&tr("Type the text to search for"));
                apply.set_sensitive(false);
                return;
            }
            match replace::preview(&shared.lib.borrow(), &ids, &rule, PREVIEW_ROWS) {
                Err(e) => {
                    summary.set_label(&format!("{e:#}"));
                    apply.set_sensitive(false);
                }
                Ok(p) => {
                    let ok = p.total - p.problems;
                    let mut text = if p.total == 0 {
                        tr("No book has that text")
                    } else {
                        crate::i18n::ntr("{} book will change", "{} books will change", ok)
                    };
                    if p.problems > 0 {
                        text += &format!(". {}", tr("Changes that are not allowed are skipped"));
                    }
                    summary.set_label(&text);
                    apply.set_sensitive(ok > 0);
                    for c in p.changes {
                        let sub = match &c.problem {
                            Some(m) => m.to_string(),
                            None => format!("{}  →  {}", short(&c.before), short(&c.after)),
                        };
                        let row = adw::ActionRow::builder()
                            .title(glib::markup_escape_text(&c.title))
                            .subtitle(glib::markup_escape_text(&sub))
                            .build();
                        row.set_subtitle_lines(2);
                        if c.problem.is_some() {
                            row.add_css_class("error");
                        }
                        preview_group.add(&row);
                        rows.borrow_mut().push(row);
                    }
                }
            }
        })
    };
    {
        let r = refresh.clone();
        find.connect_changed(move |_| r());
        let r = refresh.clone();
        with.connect_changed(move |_| r());
        let r = refresh.clone();
        field.connect_selected_notify(move |_| r());
        let r = refresh.clone();
        regex.connect_active_notify(move |_| r());
        let r = refresh.clone();
        case.connect_active_notify(move |_| r());
    }
    refresh();

    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let (d, shared, overlay) = (dialog.clone(), shared.clone(), overlay.clone());
        apply.connect_clicked(move |_| {
            let rule = current_rule();
            let res = replace::apply(&mut shared.lib.borrow_mut(), &ids, &rule);
            match res {
                Ok((n, skipped)) => {
                    let mut msg = crate::i18n::ntr("Changed {} book", "Changed {} books", n);
                    if skipped > 0 {
                        msg += &format!(" ({} {})", skipped, tr("skipped"));
                    }
                    done(msg);
                    d.close();
                }
                Err(e) => super::dialog_toast::show_error(&overlay, &format!("{e:#}")),
            }
        });
    }
    dialog.present(Some(parent));
}

/// A value shortened for one line of the preview.
fn short(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        format!("{}…", one.chars().take(59).collect::<String>())
    } else if one.is_empty() {
        "—".to_string()
    } else {
        one
    }
}

/// Test hook (`library:replace=field|find|with|regex`): applies a rule to the whole library without
/// the dialog. Returns the text for the log.
pub fn dev_apply(shared: &SharedRef, spec: &str) -> String {
    let mut p = spec.split('|');
    let (f, find, with, rx) = (
        p.next().unwrap_or(""),
        p.next().unwrap_or(""),
        p.next().unwrap_or(""),
        p.next().unwrap_or("") == "regex",
    );
    let Some(target) = Target::parse(f) else {
        return format!("replace: unknown field {f}");
    };
    let rule = Rule {
        target,
        find: find.to_string(),
        replace: with.to_string(),
        regex: rx,
        case_sensitive: false,
    };
    let ids: Vec<u64> = shared.lib.borrow().books.iter().map(|b| b.id).collect();
    match replace::apply(&mut shared.lib.borrow_mut(), &ids, &rule) {
        Ok((n, s)) => format!("replace: changed={n} skipped={s}"),
        Err(e) => format!("replace: error {e:#}"),
    }
}
