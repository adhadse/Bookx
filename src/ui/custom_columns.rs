//! Custom columns in the app (roadmap L3/L25): the dialog that defines them (Preferences, Library),
//! the rows that edit their values in the metadata form, the bulk editor and the details panel,
//! and the columns of the table view. The decisions are in `library_custom.rs`; the engine
//! (types, storage, search) is `bookx_core::custom` and `bookx-store`.

use crate::i18n::{N_, tr, trf};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk::{self, glib};

use super::SharedRef;
use super::preferences::{PrefChange, PrefCtx, register_group};
use crate::core::custom::{ColumnDef, ColumnKind};
use crate::core::library::{Book, Library};
use crate::library_custom as lc;

/// The title of each column type (English; translated where shown).
fn kind_label(k: ColumnKind) -> &'static str {
    match k {
        ColumnKind::Text => N_("Text"),
        ColumnKind::LongText => N_("Long text"),
        ColumnKind::Integer => N_("Whole number"),
        ColumnKind::Float => N_("Number with decimals"),
        ColumnKind::Rating => N_("Rating"),
        ColumnKind::Bool => N_("Yes or no"),
        ColumnKind::Date => N_("Date"),
        ColumnKind::Tags => N_("Tags (comma separated)"),
        ColumnKind::Enumeration => N_("One of a list"),
        ColumnKind::Composite => N_("Computed from a template"),
    }
}

/// Registers the *Custom Columns* row on the Library page of Preferences.
pub fn register() {
    register_group("library", 70, group);
}

fn group(ctx: &PrefCtx) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&tr("Fields"));
    let row = adw::ActionRow::builder()
        .title(tr("Custom Columns"))
        .subtitle(tr(
            "Your own fields for every book: numbers, dates, yes or no, lists and more",
        ))
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.set_widget_name("custom-columns-row");
    let (shared, parent, on_change) = (
        ctx.shared.clone(),
        ctx.parent.clone(),
        ctx.on_change.clone(),
    );
    row.connect_activated(move |_| {
        let on_change = on_change.clone();
        manage_dialog(
            &parent,
            &shared,
            Rc::new(move || on_change(PrefChange::Layout(tr("Custom columns saved")))),
        );
    });
    g.add(&row);
    g
}

// ---------------------------------------------------------------------------------------------
// the dialog that defines the columns
// ---------------------------------------------------------------------------------------------

/// Lists the columns with *Add*, *Edit* and *Remove*. `changed` runs after every change.
pub fn manage_dialog(parent: &impl IsA<gtk::Widget>, shared: &SharedRef, changed: Rc<dyn Fn()>) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&tr("Custom Columns"));
    dialog.set_content_width(520);
    dialog.set_content_height(560);
    let page = adw::PreferencesPage::new();
    let list = adw::PreferencesGroup::new();
    list.set_description(Some(&tr(
        "A column has a name, a type and a short label to search it: #label:value",
    )));
    page.add(&list);
    let add_group = adw::PreferencesGroup::new();
    let add_row = adw::ActionRow::builder()
        .title(tr("Add Column"))
        .activatable(true)
        .build();
    add_row.add_prefix(&gtk::Image::from_icon_name("list-add-symbolic"));
    add_row.set_widget_name("custom-column-add");
    add_group.add(&add_row);
    page.add(&add_group);
    let overlay = super::dialog_toast::overlay(&page);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&overlay));
    dialog.set_child(Some(&toolbar));

    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let reload: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let fill: Rc<dyn Fn()> = {
        let (shared, list, rows, reload, overlay, changed, dialog) = (
            shared.clone(),
            list.clone(),
            rows.clone(),
            reload.clone(),
            overlay.clone(),
            changed.clone(),
            dialog.clone(),
        );
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                list.remove(&r);
            }
            let defs = shared.lib.borrow().columns().to_vec();
            for d in defs {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&d.name))
                    .subtitle(format!("#{} · {}", d.label, tr(kind_label(d.kind))))
                    .build();
                row.set_widget_name(&format!("custom-column-{}", d.label));
                let edit = gtk::Button::from_icon_name("document-edit-symbolic");
                edit.add_css_class("flat");
                edit.set_valign(gtk::Align::Center);
                super::audit::label(&edit, &trf("Edit {}", &[&d.name]));
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk::Align::Center);
                super::audit::label(&del, &trf("Remove {}", &[&d.name]));
                row.add_suffix(&edit);
                row.add_suffix(&del);
                {
                    let (shared, reload, changed, dialog, d) = (
                        shared.clone(),
                        reload.clone(),
                        changed.clone(),
                        dialog.clone(),
                        d.clone(),
                    );
                    edit.connect_clicked(move |_| {
                        let (reload, changed) = (reload.clone(), changed.clone());
                        column_editor(
                            &dialog,
                            &shared,
                            Some(d.clone()),
                            Rc::new(move || {
                                if let Some(r) = reload.borrow().as_ref() {
                                    r();
                                }
                                changed();
                            }),
                        );
                    });
                }
                {
                    let (shared, reload, changed, dialog, overlay, d) = (
                        shared.clone(),
                        reload.clone(),
                        changed.clone(),
                        dialog.clone(),
                        overlay.clone(),
                        d.clone(),
                    );
                    del.connect_clicked(move |_| {
                        confirm_remove(
                            &dialog,
                            &shared,
                            &d,
                            &overlay,
                            reload.clone(),
                            changed.clone(),
                        );
                    });
                }
                list.add(&row);
                rows.borrow_mut().push(row);
            }
        })
    };
    *reload.borrow_mut() = Some(fill.clone());
    fill();
    {
        let (shared, dialog, reload, changed) = (
            shared.clone(),
            dialog.clone(),
            reload.clone(),
            changed.clone(),
        );
        add_row.connect_activated(move |_| {
            let (reload, changed) = (reload.clone(), changed.clone());
            column_editor(
                &dialog,
                &shared,
                None,
                Rc::new(move || {
                    if let Some(r) = reload.borrow().as_ref() {
                        r();
                    }
                    changed();
                }),
            );
        });
    }
    dialog.present(Some(parent));
}

fn confirm_remove(
    parent: &adw::Dialog,
    shared: &SharedRef,
    def: &ColumnDef,
    overlay: &adw::ToastOverlay,
    reload: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    changed: Rc<dyn Fn()>,
) {
    let have = shared
        .lib
        .borrow()
        .books
        .iter()
        .filter(|b| !b.custom_value(&def.label).is_empty())
        .count();
    let body = if have == 0 {
        tr("No book has a value in this column.")
    } else {
        crate::i18n::ntr(
            "The value of {} book is deleted with the column. This cannot be undone.",
            "The values of {} books are deleted with the column. This cannot be undone.",
            have,
        )
    };
    let d = adw::AlertDialog::new(Some(&trf("Remove “{}”?", &[&def.name])), Some(&body));
    d.add_responses(&[("cancel", &tr("Cancel")), ("remove", &tr("Remove"))]);
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    let (shared, label, overlay) = (shared.clone(), def.label.clone(), overlay.clone());
    d.connect_response(None, move |_, r| {
        if r != "remove" {
            return;
        }
        let res = {
            let mut lib = shared.lib.borrow_mut();
            let defs: Vec<ColumnDef> = lib
                .columns()
                .iter()
                .filter(|c| c.label != label)
                .cloned()
                .collect();
            lib.set_columns(defs)
        };
        match res {
            Ok(()) => {
                // the table must not keep a column that is gone
                let mut c = shared.cfg.borrow_mut();
                c.table_custom.retain(|l| l != &label);
                if c.custom_sort.as_ref().is_some_and(|s| s.label == label) {
                    c.custom_sort = None;
                }
                c.save();
                drop(c);
                if let Some(f) = reload.borrow().as_ref() {
                    f();
                }
                changed();
            }
            Err(e) => super::dialog_toast::show_error(&overlay, &format!("{e:#}")),
        }
    });
    d.present(Some(parent));
}

/// The form of one column: a new one (`None`) or an existing one, whose label and type stay.
fn column_editor(
    parent: &adw::Dialog,
    shared: &SharedRef,
    existing: Option<ColumnDef>,
    done: Rc<dyn Fn()>,
) {
    let dialog = super::dialog_toast::new_dialog();
    dialog.set_title(&if existing.is_some() {
        tr("Edit Column")
    } else {
        tr("Add Column")
    });
    dialog.set_content_width(480);
    let page = adw::PreferencesPage::new();
    let g = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder().title(tr("Name")).build();
    name.set_widget_name("custom-column-name");
    let label = adw::EntryRow::builder().title(tr("Search label")).build();
    label.set_widget_name("custom-column-label");
    let kinds: Vec<String> = ColumnKind::ALL.iter().map(|k| tr(kind_label(*k))).collect();
    let kind_refs: Vec<&str> = kinds.iter().map(|s| s.as_str()).collect();
    let kind = adw::ComboRow::builder()
        .title(tr("Type"))
        .model(&gtk::StringList::new(&kind_refs))
        .build();
    kind.set_widget_name("custom-column-type");
    let values = adw::EntryRow::builder()
        .title(tr("Values (comma separated)"))
        .build();
    values.set_widget_name("custom-column-values");
    let template = adw::EntryRow::builder().title(tr("Template")).build();
    template.set_widget_name("custom-column-template");
    template.set_tooltip_text(Some(&tr(
        "Fields in braces: {title} {series} {#label}; for example {series} {series_index}",
    )));
    for r in [
        name.upcast_ref::<gtk::Widget>(),
        label.upcast_ref(),
        kind.upcast_ref(),
        values.upcast_ref(),
        template.upcast_ref(),
    ] {
        g.add(r);
    }
    let error = gtk::Label::new(None);
    error.add_css_class("error");
    error.set_wrap(true);
    error.set_xalign(0.0);
    error.set_visible(false);
    g.add(&error);
    page.add(&g);

    let show_for_kind = {
        let (kind, values, template) = (kind.clone(), values.clone(), template.clone());
        Rc::new(move || {
            let k = ColumnKind::ALL[kind.selected() as usize % ColumnKind::ALL.len()];
            values.set_visible(k == ColumnKind::Enumeration);
            template.set_visible(k == ColumnKind::Composite);
        })
    };
    if let Some(d) = &existing {
        name.set_text(&d.name);
        label.set_text(&d.label);
        label.set_editable(false);
        label.set_sensitive(false);
        kind.set_selected(
            ColumnKind::ALL
                .iter()
                .position(|k| *k == d.kind)
                .unwrap_or(0) as u32,
        );
        kind.set_sensitive(false);
        values.set_text(&d.values.join(", "));
        template.set_text(&d.template);
    } else {
        // the label follows the name until it is typed by hand
        let touched = Rc::new(std::cell::Cell::new(false));
        {
            let touched = touched.clone();
            label.connect_changed(move |l| {
                if l.has_focus() || l.focus_child().is_some() {
                    touched.set(true);
                }
            });
        }
        let label2 = label.clone();
        name.connect_changed(move |n| {
            if !touched.get() {
                label2.set_text(&crate::core::custom::label_from_name(&n.text()));
            }
        });
    }
    {
        let f = show_for_kind.clone();
        kind.connect_selected_notify(move |_| f());
    }
    show_for_kind();

    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    let cancel = gtk::Button::with_label(&tr("Cancel"));
    let save = gtk::Button::with_label(&tr("Save"));
    save.add_css_class("suggested-action");
    save.set_widget_name("custom-column-save");
    header.pack_start(&cancel);
    header.pack_end(&save);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));
    // the keyboard starts in the form, not on the Cancel button of the header (typing a space would press it)
    dialog.set_focus(Some(&name));
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let (d, shared) = (dialog.clone(), shared.clone());
        save.connect_clicked(move |_| {
            let k = ColumnKind::ALL[kind.selected() as usize % ColumnKind::ALL.len()];
            let def = ColumnDef {
                label: label.text().trim().to_string(),
                name: name.text().trim().to_string(),
                kind: k,
                values: crate::ui::metadata::split_list(&values.text(), &[',']),
                template: template.text().trim().to_string(),
            };
            let res = {
                let mut lib = shared.lib.borrow_mut();
                let mut defs = lib.columns().to_vec();
                match &existing {
                    Some(old) => {
                        if let Some(slot) = defs.iter_mut().find(|c| c.label == old.label) {
                            *slot = def;
                        }
                    }
                    None => defs.push(def),
                }
                lib.set_columns(defs)
            };
            match res {
                Ok(()) => {
                    done();
                    d.close();
                }
                Err(e) => {
                    error.set_label(&format!("{e:#}"));
                    error.set_visible(true);
                }
            }
        });
    }
    dialog.present(Some(parent));
}

// ---------------------------------------------------------------------------------------------
// rows that edit the values
// ---------------------------------------------------------------------------------------------

/// One input for the value of a column.
struct Field {
    def: ColumnDef,
    widget: gtk::Widget,
    read: Rc<dyn Fn() -> String>,
}

/// What a choice shows: `empty` for no value, Yes and No, stars, or the value itself.
fn choice_text(def: &ColumnDef, value: &str, empty: &str) -> String {
    if value.is_empty() {
        return empty.to_string();
    }
    lc::cell_text(def, value, &tr("Yes"), &tr("No"))
}

/// Builds the input for `def` showing `current`. `commit` runs with the new text when the user
/// applies an entry (Enter or the apply button) or picks from a choice.
fn field(def: &ColumnDef, current: &str, empty: &str, commit: Option<Rc<dyn Fn(String)>>) -> Field {
    if def.kind == ColumnKind::Composite {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&def.name))
            .subtitle(glib::markup_escape_text(current))
            .subtitle_selectable(true)
            .build();
        row.add_css_class("property");
        return Field {
            def: def.clone(),
            widget: row.upcast(),
            read: Rc::new(String::new),
        };
    }
    if lc::is_choice(def) {
        let values = lc::choices(def);
        let labels: Vec<String> = values.iter().map(|v| choice_text(def, v, empty)).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let combo = adw::ComboRow::builder()
            .title(glib::markup_escape_text(&def.name))
            .model(&gtk::StringList::new(&refs))
            .build();
        combo.set_widget_name(&format!("custom-field-{}", def.label));
        let sel = values
            .iter()
            .position(|v| v.eq_ignore_ascii_case(current))
            .unwrap_or(0);
        combo.set_selected(sel as u32);
        if let Some(c) = commit {
            let values = values.clone();
            combo.connect_selected_notify(move |r| {
                c(values
                    .get(r.selected() as usize)
                    .cloned()
                    .unwrap_or_default());
            });
        }
        let c2 = combo.clone();
        return Field {
            def: def.clone(),
            widget: combo.upcast(),
            read: Rc::new(move || {
                values
                    .get(c2.selected() as usize)
                    .cloned()
                    .unwrap_or_default()
            }),
        };
    }
    let row = adw::EntryRow::builder()
        .title(glib::markup_escape_text(&def.name))
        .build();
    row.set_widget_name(&format!("custom-field-{}", def.label));
    row.set_text(&match def.kind {
        ColumnKind::LongText => current.lines().next().unwrap_or("").to_string(),
        _ => current.to_string(),
    });
    {
        // a value that cannot be stored is marked while it is typed
        let d = def.clone();
        row.connect_changed(move |r| {
            if d.normalize(&r.text()).is_err() {
                r.add_css_class("error");
            } else {
                r.remove_css_class("error");
            }
        });
    }
    if let Some(c) = commit {
        row.set_show_apply_button(true);
        row.connect_apply(move |r| c(r.text().to_string()));
    }
    let r2 = row.clone();
    Field {
        def: def.clone(),
        widget: row.upcast(),
        read: Rc::new(move || r2.text().to_string()),
    }
}

struct Ctx {
    defs: Vec<ColumnDef>,
    values: BTreeMap<String, String>,
}

thread_local! {
    /// The columns and the values of the book the next edit dialog is for.
    static CONTEXT: RefCell<Option<Ctx>> = const { RefCell::new(None) };
    /// What the last saved edit form changed (read by `take_stashed`).
    static STASH: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    /// The dynamic columns of the table, with the definition they were made from.
    static COLS: RefCell<Vec<(ColumnDef, gtk::ColumnViewColumn)>> = const { RefCell::new(Vec::new()) };
    static CHOOSER: RefCell<Option<(gtk::Box, SharedRef, gtk::ColumnView)>> = const { RefCell::new(None) };
    static SORT_HOOKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The columns the chooser was built for, so it is only rebuilt when they change.
    static CHOOSER_SIG: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Tells the next edit form (one book) or bulk editor (several) which columns exist and, for one
/// book, its values. Call before opening the dialog.
pub fn set_context(lib: &Library, ids: &[u64]) {
    let defs = lib.columns().to_vec();
    let values = match ids {
        [one] => lib.get(*one).map(|b| b.custom.clone()).unwrap_or_default(),
        _ => BTreeMap::new(),
    };
    CONTEXT.with(|c| {
        *c.borrow_mut() = if defs.is_empty() {
            None
        } else {
            Some(Ctx { defs, values })
        }
    });
}

/// The rows of the custom columns inside the form of an edit dialog (or the bulk editor).
#[derive(Clone)]
pub struct CustomRows {
    pub group: adw::PreferencesGroup,
    fields: Rc<Vec<Field>>,
    defs: Rc<Vec<ColumnDef>>,
    original: Rc<BTreeMap<String, String>>,
    bulk: bool,
}

fn build_rows(bulk: bool) -> Option<CustomRows> {
    let ctx = CONTEXT.with(|c| c.borrow_mut().take())?;
    let group = adw::PreferencesGroup::builder()
        .title(tr("Custom Columns"))
        .build();
    let empty = if bulk {
        tr("Leave as is")
    } else {
        "—".to_string()
    };
    let mut fields = Vec::new();
    for d in &ctx.defs {
        let cur = if bulk {
            ""
        } else {
            ctx.values.get(&d.label).map(String::as_str).unwrap_or("")
        };
        let f = field(d, cur, &empty, None);
        group.add(&f.widget);
        fields.push(f);
    }
    Some(CustomRows {
        group,
        fields: Rc::new(fields),
        defs: Rc::new(ctx.defs),
        original: Rc::new(ctx.values),
        bulk,
    })
}

/// The rows for the edit form of one book; `None` when the library has no custom columns.
pub fn form_rows() -> Option<CustomRows> {
    build_rows(false)
}

/// The rows for the bulk editor.
pub fn bulk_rows() -> Option<CustomRows> {
    build_rows(true)
}

impl CustomRows {
    /// The values to write: for a book what changed, for the bulk editor what was filled in.
    /// An error names the column and what is wrong.
    pub fn collect(&self) -> Result<Vec<(String, String)>, String> {
        let entered: Vec<(String, String)> = self
            .fields
            .iter()
            .map(|f| (f.def.label.clone(), (f.read)()))
            .collect();
        if self.bulk {
            lc::bulk_changes(&self.defs, &entered)
        } else {
            lc::changed(&self.defs, &self.original, &entered)
        }
    }
}

/// Keeps what an edit form changed until the library applies it (`apply_stashed`).
pub fn stash(v: Vec<(String, String)>) {
    STASH.with(|s| *s.borrow_mut() = v);
}

/// Writes what the last saved form changed into the book, as one undo step with the metadata.
pub fn apply_stashed(lib: &mut Library, id: u64) -> anyhow::Result<()> {
    let v = STASH.with(|s| std::mem::take(&mut *s.borrow_mut()));
    if v.is_empty() {
        return Ok(());
    }
    lib.edit_custom(trf("Edited custom columns of {}", &[&id]), &[(id, v)])?;
    Ok(())
}

/// A small error dialog over a form (a value that does not fit its column).
pub fn alert(parent: &impl IsA<gtk::Widget>, message: &str) {
    let d = adw::AlertDialog::new(Some(&tr("A value does not fit")), Some(message));
    d.add_response("ok", &tr("OK"));
    d.present(Some(parent));
}

// ---------------------------------------------------------------------------------------------
// the details panel and Properties
// ---------------------------------------------------------------------------------------------

/// The custom columns of one book as editable rows for the details panel. `result` runs with the
/// outcome after a value is applied (the caller refreshes the view or shows the error).
pub fn details_group(
    lib: &Library,
    b: &Book,
    shared: &SharedRef,
    result: Rc<dyn Fn(Result<(), String>)>,
) -> Option<adw::PreferencesGroup> {
    if lib.columns().is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::new();
    group.set_title(&tr("Custom Columns"));
    group.set_margin_top(10);
    for d in lib.columns() {
        let cur = b.custom_value(&d.label).to_string();
        let commit: Option<Rc<dyn Fn(String)>> = if d.kind == ColumnKind::Composite {
            None
        } else {
            let (shared, label, id, result, before) = (
                shared.clone(),
                d.label.clone(),
                b.id,
                result.clone(),
                cur.clone(),
            );
            let def = d.clone();
            Some(Rc::new(move |text: String| {
                // a combo reports its initial selection change too: nothing to do then
                if def.normalize(&text).is_ok_and(|n| n == before) {
                    return;
                }
                let r = shared
                    .lib
                    .borrow_mut()
                    .set_custom(id, &label, &text)
                    .map_err(|e| format!("{e:#}"));
                result(r);
            }))
        };
        group.add(&field(d, &cur, "—", commit).widget);
    }
    Some(group)
}

/// Label and text of the custom columns that have a value, for the Properties dialog.
pub fn property_rows(lib: &Library, b: &Book) -> Vec<(String, String)> {
    lib.columns()
        .iter()
        .filter_map(|d| {
            let v = b.custom_value(&d.label);
            (!v.is_empty()).then(|| (d.name.clone(), lc::cell_text(d, v, &tr("Yes"), &tr("No"))))
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// the table
// ---------------------------------------------------------------------------------------------

fn row_of(obj: &glib::Object) -> Option<std::cell::Ref<'_, super::library::BookRow>> {
    obj.downcast_ref::<glib::BoxedAnyObject>()
        .map(|b| b.borrow::<super::library::BookRow>())
}

fn make_column(def: &ColumnDef) -> gtk::ColumnViewColumn {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_margin_start(6);
        l.set_margin_end(6);
        l.set_margin_top(6);
        l.set_margin_bottom(6);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_width_chars(10);
        item.set_child(Some(&l));
    });
    {
        let d = def.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let (Some(l), Some(obj)) = (item.child().and_downcast::<gtk::Label>(), item.item())
            else {
                return;
            };
            let text = row_of(&obj)
                .map(|r| {
                    lc::cell_text(
                        &d,
                        lc::row_value(&r.custom, &d.label).unwrap_or(""),
                        &tr("Yes"),
                        &tr("No"),
                    )
                })
                .unwrap_or_default();
            l.set_label(&text);
        });
    }
    let col = gtk::ColumnViewColumn::new(Some(&def.name), Some(factory));
    col.set_resizable(true);
    let d = def.clone();
    col.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
        match (row_of(a), row_of(b)) {
            (Some(x), Some(y)) => d
                .compare(
                    lc::row_value(&x.custom, &d.label),
                    lc::row_value(&y.custom, &d.label),
                )
                .into(),
            _ => gtk::Ordering::Equal,
        }
    })));
    col
}

/// Makes the table show the custom columns the user ticked: adds the missing ones, removes the
/// ones that are gone or changed, and refreshes the chooser in the view options.
pub fn sync_table(view: &gtk::ColumnView, shared: &SharedRef) {
    let (defs, ticked) = (
        shared.lib.borrow().columns().to_vec(),
        shared.cfg.borrow().table_custom.clone(),
    );
    let wanted: Vec<ColumnDef> = lc::shown_columns(&defs, &ticked)
        .into_iter()
        .cloned()
        .collect();
    COLS.with(|cols| {
        let mut cols = cols.borrow_mut();
        // a column whose definition changed is made again
        let keep: Vec<(ColumnDef, gtk::ColumnViewColumn)> = cols
            .drain(..)
            .filter_map(|(d, c)| {
                if wanted.contains(&d) {
                    Some((d, c))
                } else {
                    view.remove_column(&c);
                    None
                }
            })
            .collect();
        *cols = keep;
        for d in &wanted {
            if !cols.iter().any(|(c, _)| c == d) {
                let col = make_column(d);
                view.append_column(&col);
                cols.push((d.clone(), col));
            }
        }
    });
    hook_sort(view, shared);
    rebuild_chooser(view, shared);
}

/// A click on the header of a custom column is remembered as the sort of the table; a click on
/// another header ends it.
fn hook_sort(view: &gtk::ColumnView, shared: &SharedRef) {
    if SORT_HOOKED.with(|h| h.replace(true)) {
        return;
    }
    let Some(cs) = view.sorter().and_downcast::<gtk::ColumnViewSorter>() else {
        return;
    };
    let shared = shared.clone();
    cs.connect_changed(move |s, _| {
        let Some(col) = s.primary_sort_column() else {
            return;
        };
        let label = COLS.with(|c| {
            c.borrow()
                .iter()
                .find(|(_, x)| *x == col)
                .map(|(d, _)| d.label.clone())
        });
        let mut cfg = shared.cfg.borrow_mut();
        let new = label.map(|label| lc::CustomSort {
            label,
            ascending: s.primary_sort_order() == gtk::SortType::Ascending,
            base_key: cfg.sort_key,
            base_ascending: cfg.sort_ascending,
        });
        if cfg.custom_sort != new {
            cfg.custom_sort = new;
            cfg.save();
        }
    });
}

/// Applies the remembered sort by a custom column (when it still holds). Returns true when it did,
/// so the caller leaves the sort alone.
pub fn restore_sort(view: &gtk::ColumnView, shared: &SharedRef) -> bool {
    let (sort, key, asc) = {
        let c = shared.cfg.borrow();
        (c.custom_sort.clone(), c.sort_key, c.sort_ascending)
    };
    let Some(s) = sort else { return false };
    if !lc::sort_applies(&s, shared.lib.borrow().columns(), key, asc) {
        return false;
    }
    let col = COLS.with(|c| {
        c.borrow()
            .iter()
            .find(|(d, _)| d.label == s.label)
            .map(|(_, c)| c.clone())
    });
    let Some(col) = col else { return false };
    view.sort_by_column(
        Some(&col),
        if s.ascending {
            gtk::SortType::Ascending
        } else {
            gtk::SortType::Descending
        },
    );
    true
}

/// The check boxes of the custom columns for the view options menu.
pub fn chooser(shared: &SharedRef, view: &gtk::ColumnView) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    CHOOSER.with(|c| *c.borrow_mut() = Some((b.clone(), shared.clone(), view.clone())));
    rebuild_chooser(view, shared);
    b
}

fn rebuild_chooser(view: &gtk::ColumnView, shared: &SharedRef) {
    let Some((b, _, _)) = CHOOSER.with(|c| c.borrow().clone()) else {
        return;
    };
    let defs = shared.lib.borrow().columns().to_vec();
    let sig: String = defs
        .iter()
        .map(|d| format!("{}={};", d.label, d.name))
        .collect();
    if CHOOSER_SIG.with(|s| *s.borrow() == sig) && b.first_child().is_some() != defs.is_empty() {
        return;
    }
    CHOOSER_SIG.with(|s| *s.borrow_mut() = sig);
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    b.set_visible(!defs.is_empty());
    if defs.is_empty() {
        return;
    }
    let head = gtk::Label::new(Some(&tr("Custom columns")));
    head.set_xalign(0.0);
    head.add_css_class("heading");
    b.append(&head);
    let ticked = shared.cfg.borrow().table_custom.clone();
    for d in defs {
        let c = gtk::CheckButton::with_label(&d.name);
        c.set_active(ticked.contains(&d.label));
        c.set_widget_name(&format!("custom-col-check-{}", d.label));
        let (shared, view, label) = (shared.clone(), view.clone(), d.label.clone());
        c.connect_toggled(move |c| {
            {
                let mut cfg = shared.cfg.borrow_mut();
                let defs = shared.lib.borrow().columns().to_vec();
                cfg.table_custom = lc::toggle(&cfg.table_custom, &defs, &label, c.is_active());
                cfg.save();
            }
            sync_table(&view, &shared);
        });
        b.append(&c);
    }
}

/// The table column of a custom column (test hook).
pub fn column_of(label: &str) -> Option<gtk::ColumnViewColumn> {
    COLS.with(|c| {
        c.borrow()
            .iter()
            .find(|(d, _)| d.label == label)
            .map(|(_, c)| c.clone())
    })
}

/// Test hook: the custom columns of the table and what the first rows show.
pub fn dev_dump(shared: &SharedRef) -> String {
    let defs = shared.lib.borrow().columns().len();
    let shown: Vec<String> = COLS.with(|c| {
        c.borrow()
            .iter()
            .map(|(d, col)| format!("{}:{}", d.label, col.is_visible()))
            .collect()
    });
    format!(
        "customcols: defined={defs} table=[{}] sort={:?}",
        shown.join(","),
        shared.cfg.borrow().custom_sort.as_ref().map(|s| format!(
            "{}:{}",
            s.label,
            if s.ascending { "asc" } else { "desc" }
        ))
    )
}
