//! Tools → Structure and Accessibility → Accessibility Metadata (CE-427): a form for the schema.org accessibility
//! metadata of an EPUB 3 (access modes, sufficient modes, features, hazards, summary, conformance, certifier) with
//! presets, suggestions taken from the book, live validation and a check of the claims against the book.
//! The vocabulary, the checks and the writer are `bookx_editing::a11y_meta`.

use std::collections::BTreeSet;

use crate::editor_structure as es;
use crate::i18n::tr;
use bookx_editing::a11y_meta::{self, A11yForm, Severity};
use bookx_editing::book_tools::{AccessMeta, ACCESS_FEATURES, ACCESS_HAZARDS, ACCESS_MODES};

use super::tools_ui::*;
use super::*;

type Checks = Vec<(String, gtk::CheckButton)>;

fn checks(
    title: &str,
    all: &[&str],
    on: &BTreeSet<String>,
    changed: &Rc<dyn Fn()>,
) -> (adw::ExpanderRow, Checks) {
    let ex = adw::ExpanderRow::new();
    ex.set_title(title);
    let mut cs = Vec::new();
    for v in all {
        let r = adw::ActionRow::new();
        r.set_title(v);
        let c = gtk::CheckButton::new();
        c.set_active(on.contains(*v));
        crate::ui::audit::label(&c, &format!("{title}: {v}"));
        r.add_prefix(&c);
        r.set_activatable_widget(Some(&c));
        let ch = changed.clone();
        c.connect_toggled(move |_| ch());
        ex.add_row(&r);
        cs.push((v.to_string(), c));
    }
    (ex, cs)
}

struct Widgets {
    summary: adw::EntryRow,
    conforms: adw::ComboRow,
    conforms_items: Rc<RefCell<Vec<String>>>,
    sufficient: adw::EntryRow,
    modes: Checks,
    features: Checks,
    hazards: Checks,
    by: adw::EntryRow,
    credential: adw::EntryRow,
    report: adw::EntryRow,
}

impl Widgets {
    fn collect(&self) -> A11yForm {
        let on = |c: &Checks| -> BTreeSet<String> {
            c.iter()
                .filter(|(_, b)| b.is_active())
                .map(|(v, _)| v.clone())
                .collect()
        };
        let i = self.conforms.selected() as usize;
        let conformance = if i == 0 {
            String::new()
        } else {
            self.conforms_items
                .borrow()
                .get(i - 1)
                .cloned()
                .unwrap_or_default()
        };
        A11yForm {
            meta: AccessMeta {
                summary: self.summary.text().trim().to_string(),
                access_modes: on(&self.modes),
                sufficient_modes: self
                    .sufficient
                    .text()
                    .split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                features: on(&self.features),
                hazards: on(&self.hazards),
                conformance,
            },
            certified_by: self.by.text().trim().to_string(),
            certifier_credential: self.credential.text().trim().to_string(),
            certifier_report: self.report.text().trim().to_string(),
        }
    }

    fn fill(&self, f: &A11yForm) {
        self.summary.set_text(&f.meta.summary);
        self.sufficient
            .set_text(&f.meta.sufficient_modes.join("; "));
        for (checks, set) in [
            (&self.modes, &f.meta.access_modes),
            (&self.features, &f.meta.features),
            (&self.hazards, &f.meta.hazards),
        ] {
            for (v, c) in checks {
                c.set_active(set.contains(v));
            }
        }
        let sel = {
            let items = self.conforms_items.borrow();
            items
                .iter()
                .position(|c| *c == f.meta.conformance)
                .map(|i| i as u32 + 1)
                .unwrap_or(0)
        };
        self.conforms.set_selected(sel);
        self.by.set_text(&f.certified_by);
        self.credential.set_text(&f.certifier_credential);
        self.report.set_text(&f.certifier_report);
    }
}

/// The text under the form: errors first, then warnings, then what the book contradicts.
fn problems_text(book: &OpenBook, f: &A11yForm) -> (String, bool) {
    let mut lines = Vec::new();
    let mut errors = false;
    for p in a11y_meta::validate(f) {
        if p.severity == Severity::Error {
            errors = true;
            lines.push(format!("✗ {}", tr(&p.message)));
        }
    }
    for p in a11y_meta::validate(f) {
        if p.severity == Severity::Warning {
            lines.push(format!("• {}", tr(&p.message)));
        }
    }
    for c in a11y_meta::check_claims(book, f) {
        lines.push(format!("! {}", tr(&c)));
    }
    if lines.is_empty() {
        lines.push(tr("The metadata is complete and matches the book."));
    }
    (lines.join("\n"), errors)
}

impl Editor {
    pub(super) fn tool_a11y_form(&self) {
        let (form0, epub3) = {
            let b = self.book.borrow();
            (a11y_meta::read_form(&b), b.package.is_epub3())
        };
        if !epub3 {
            let s = adw::StatusPage::new();
            s.set_title(&tr("Needs EPUB 3"));
            s.set_description(Some(&tr(
                "Accessibility metadata is part of EPUB 3. Use Tools → Upgrade to EPUB 3 first.",
            )));
            s.set_icon_name(Some("dialog-information-symbolic"));
            let make: MakeOp = Rc::new(|| Err("Upgrade the book to EPUB 3 first".to_string()));
            self.tool_dialog(
                Spec::new("Accessibility Metadata", "", "Apply"),
                Some(s.upcast()),
                make,
                None,
            );
            return;
        }
        let check_label = gtk::Label::new(None);
        check_label.set_wrap(true);
        check_label.set_xalign(0.0);
        check_label.set_selectable(true);
        check_label.add_css_class("caption");
        crate::ui::audit::label(&check_label, &tr("Checks of the accessibility metadata"));
        let widgets: Rc<RefCell<Option<Rc<Widgets>>>> = Rc::new(RefCell::new(None));
        let book = self.book.clone();
        let changed: Rc<dyn Fn()> = {
            let (widgets, label, book) = (widgets.clone(), check_label.clone(), book.clone());
            Rc::new(move || {
                if let Some(w) = widgets.borrow().as_ref() {
                    let (t, _) = problems_text(&book.borrow(), &w.collect());
                    label.set_text(&t);
                }
            })
        };

        let summary = entry_row(&tr("Summary"), &form0.meta.summary);
        let mut items: Vec<String> = a11y_meta::CONFORMANCE
            .iter()
            .map(|s| s.to_string())
            .collect();
        if !form0.meta.conformance.is_empty() && !items.contains(&form0.meta.conformance) {
            items.push(form0.meta.conformance.clone());
        }
        let mut labels = vec![tr("(not stated)")];
        labels.extend(items.iter().cloned());
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let sel = items
            .iter()
            .position(|c| *c == form0.meta.conformance)
            .map(|i| i as u32 + 1)
            .unwrap_or(0);
        let conforms = combo_row(&tr("Conforms to"), &refs, sel);
        let sufficient = entry_row(
            &tr("Sufficient modes, sets separated by ; (for example textual; textual,visual)"),
            &form0.meta.sufficient_modes.join("; "),
        );
        let (modes_ex, modes) = checks(
            &tr("Access modes"),
            ACCESS_MODES,
            &form0.meta.access_modes,
            &changed,
        );
        let (feat_ex, feats) = checks(
            &tr("Features"),
            ACCESS_FEATURES,
            &form0.meta.features,
            &changed,
        );
        let (haz_ex, hazs) = checks(
            &tr("Hazards"),
            ACCESS_HAZARDS,
            &form0.meta.hazards,
            &changed,
        );
        let by = entry_row(&tr("Certified by"), &form0.certified_by);
        let credential = entry_row(&tr("Certifier credential"), &form0.certifier_credential);
        let report = entry_row(&tr("Certifier report (address)"), &form0.certifier_report);
        for r in [&summary, &sufficient, &by, &credential, &report] {
            let ch = changed.clone();
            r.connect_changed(move |_| ch());
        }
        {
            let ch = changed.clone();
            conforms.connect_selected_notify(move |_| ch());
        }
        let w = Rc::new(Widgets {
            summary: summary.clone(),
            conforms: conforms.clone(),
            conforms_items: Rc::new(RefCell::new(items)),
            sufficient: sufficient.clone(),
            modes,
            features: feats,
            hazards: hazs,
            by: by.clone(),
            credential: credential.clone(),
            report: report.clone(),
        });
        *widgets.borrow_mut() = Some(w.clone());

        // presets and suggestions
        let presets = a11y_meta::presets();
        let mut pl = vec![tr("(choose a preset)")];
        pl.extend(presets.iter().map(|p| tr(p.label)));
        let pr: Vec<&str> = pl.iter().map(String::as_str).collect();
        let preset_row = combo_row(&tr("Start from a preset"), &pr, 0);
        {
            let (w, changed) = (w.clone(), changed.clone());
            let presets = presets.clone();
            preset_row.connect_selected_notify(move |r| {
                let i = r.selected() as usize;
                if i > 0 {
                    if let Some(p) = presets.get(i - 1) {
                        w.fill(&p.form);
                        changed();
                    }
                }
            });
        }
        let suggest_note = gtk::Label::new(None);
        suggest_note.set_wrap(true);
        suggest_note.set_xalign(0.0);
        suggest_note.add_css_class("dim-label");
        suggest_note.add_css_class("caption");
        let suggest = gtk::Button::with_label(&tr("Suggest from the Book"));
        suggest.set_valign(gtk::Align::Center);
        let suggest_row = adw::ActionRow::new();
        suggest_row.set_title(&tr("Look at the book"));
        suggest_row.set_subtitle(&tr(
            "Fills the form from the images, audio, MathML, headings and contents it finds",
        ));
        suggest_row.add_suffix(&suggest);
        {
            let (w, changed, book, note) = (
                w.clone(),
                changed.clone(),
                book.clone(),
                suggest_note.clone(),
            );
            suggest.connect_clicked(move |_| {
                let s = a11y_meta::suggest(&book.borrow());
                let mut f = s.form;
                let cur = w.collect();
                f.meta.conformance = cur.meta.conformance;
                f.certified_by = cur.certified_by;
                f.certifier_credential = cur.certifier_credential;
                f.certifier_report = cur.certifier_report;
                w.fill(&f);
                note.set_text(&s.notes.iter().map(|n| tr(n)).collect::<Vec<_>>().join("\n"));
                changed();
            });
        }

        let g1 = group("");
        g1.add(&preset_row);
        g1.add(&suggest_row);
        let g2 = group("");
        for r in [
            summary.upcast_ref::<gtk::Widget>(),
            sufficient.upcast_ref(),
            conforms.upcast_ref(),
            modes_ex.upcast_ref(),
            feat_ex.upcast_ref(),
            haz_ex.upcast_ref(),
        ] {
            g2.add(r);
        }
        let g3 = group(&tr("Certifier"));
        for r in [&by, &credential, &report] {
            g3.add(r);
        }
        let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bx.append(&g1);
        bx.append(&suggest_note);
        bx.append(&g2);
        bx.append(&g3);
        let cg = group(&tr("Checks"));
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("card");
        check_label.set_margin_all(10);
        card.append(&check_label);
        cg.add(&card);
        bx.append(&cg);
        changed();

        let (wm, bm) = (w.clone(), book.clone());
        let make: MakeOp = Rc::new(move || {
            let f = wm.collect();
            let (text, errors) = problems_text(&bm.borrow(), &f);
            if errors {
                return Err(text);
            }
            Ok((
                "Accessibility Metadata".to_string(),
                Rc::new(move |b: &mut OpenBook, _: Option<&str>| es::a11y_apply(b, &f)) as Op,
            ))
        });
        self.tool_dialog(
            Spec {
                height: 760,
                ..Spec::new(
                    "Accessibility Metadata",
                    tr("What stores, libraries and screen readers are told about the book: how it can be read, which features it has and which hazards. Values are checked against the EPUB accessibility vocabulary and against the book."),
                    "Apply",
                )
            },
            Some(bx.upcast()),
            make,
            None,
        );
    }

    /// `x:a11y-form=read` | `preset|<id>` | `suggest` | `set|summary=..|modes=a:b|features=..|hazards=..|suff=a;b,c|by=..|cred=..|report=..` |
    /// `validate`.
    pub(super) fn dev_a11y(&mut self, arg: &str) {
        let mut it = arg.split('|');
        let cmd = it.next().unwrap_or("");
        let show = |f: &A11yForm| {
            eprintln!(
                "editor_x: a11y-form: summary={} modes={} suff={} features={} hazards={} conf={} by={} report={}",
                f.meta.summary,
                f.meta.access_modes.iter().cloned().collect::<Vec<_>>().join(","),
                f.meta.sufficient_modes.join(";"),
                f.meta.features.iter().cloned().collect::<Vec<_>>().join(","),
                f.meta.hazards.iter().cloned().collect::<Vec<_>>().join(","),
                f.meta.conformance,
                f.certified_by,
                f.certifier_report
            );
        };
        match cmd {
            "read" => show(&a11y_meta::read_form(&self.book.borrow())),
            "suggest" => {
                let s = a11y_meta::suggest(&self.book.borrow());
                show(&s.form);
                eprintln!("editor_x: a11y-notes: {}", s.notes.join(" / "));
            }
            "validate" => {
                let f = a11y_meta::read_form(&self.book.borrow());
                let (t, errors) = problems_text(&self.book.borrow(), &f);
                eprintln!(
                    "editor_x: a11y-validate: errors={errors} {}",
                    t.replace('\n', " / ")
                );
            }
            "preset" => {
                if let Some(p) = a11y_meta::preset(it.next().unwrap_or("")) {
                    self.run_op("Accessibility Metadata", move |b, _| {
                        es::a11y_apply(b, &p.form)
                    });
                }
            }
            "set" => {
                let mut f = a11y_meta::read_form(&self.book.borrow());
                let list = |v: &str| -> BTreeSet<String> {
                    v.split(':')
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                };
                for kv in it {
                    let Some((k, v)) = kv.split_once('=') else {
                        continue;
                    };
                    match k {
                        "summary" => f.meta.summary = v.into(),
                        "modes" => f.meta.access_modes = list(v),
                        "features" => f.meta.features = list(v),
                        "hazards" => f.meta.hazards = list(v),
                        "suff" => {
                            f.meta.sufficient_modes = v
                                .split(';')
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                                .collect()
                        }
                        "conf" => f.meta.conformance = v.into(),
                        "by" => f.certified_by = v.into(),
                        "cred" => f.certifier_credential = v.into(),
                        "report" => f.certifier_report = v.into(),
                        _ => {}
                    }
                }
                self.run_op("Accessibility Metadata", move |b, _| es::a11y_apply(b, &f));
            }
            _ => self.toast(&format!("Unknown a11y-form command {cmd}")),
        }
    }
}
