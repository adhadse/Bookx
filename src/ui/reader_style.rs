//! The style tweaks inside the reader's text appearance popover (roadmap R21, enhancement 22):
//! paragraph spacing, optimal line breaking, hyphenation language and, under "Advanced", the
//! overrides of line height, margins, font and justification and the reader's own style sheet.
//! Every change is the open book's own (it is part of the book's automatic look, see
//! `crate::reading_profiles`); the values live in [`crate::tweaks`].

use crate::i18n::tr;
use std::cell::Cell;
use std::rc::Rc;

use relm4::adw::{self, prelude::*};
use relm4::gtk;

use crate::tweaks::{StyleTweaks, HYPHEN_LANGUAGES};

type Get = Rc<dyn Fn() -> StyleTweaks>;
type Change = Rc<dyn Fn(StyleTweaks)>;

/// The widgets of the dialog, kept so that tests can drive them.
#[derive(Clone)]
pub struct TweaksUi {
    /// "Text" rows: paragraph spacing, line breaking, hyphenation language.
    pub text_group: adw::PreferencesGroup,
    /// "Advanced": overrides and the style sheet, collapsed.
    pub advanced_group: adw::PreferencesGroup,
    line_height: adw::ExpanderRow,
    line_height_v: adw::SpinRow,
    margin: adw::ExpanderRow,
    margin_v: adw::SpinRow,
    spacing: adw::ExpanderRow,
    spacing_v: adw::SpinRow,
    font: adw::ExpanderRow,
    font_v: adw::EntryRow,
    justify: adw::ExpanderRow,
    justify_v: adw::SwitchRow,
    optimal: adw::ExpanderRow,
    optimal_v: adw::SwitchRow,
    hyphen: adw::ComboRow,
    css: gtk::TextView,
    quiet: Rc<Cell<bool>>,
    get: Get,
    change: Change,
}

fn expander(title: &str, subtitle: &str) -> adw::ExpanderRow {
    let r = adw::ExpanderRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.set_show_enable_switch(true);
    r
}

fn spin(title: &str, lo: f64, hi: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(lo, hi, step);
    r.set_title(title);
    r.set_digits(digits);
    r.set_value(value);
    r
}

/// Gives the enable switch in the header of each expander row the name of the row (libadwaita
/// leaves it unnamed); the switches of switch rows are named by their own row.
fn name_enable_switches(w: &gtk::Widget) {
    if let Some(sw) = w.downcast_ref::<gtk::Switch>() {
        if w.ancestor(adw::SwitchRow::static_type()).is_none() {
            if let Some(row) = w
                .ancestor(adw::ExpanderRow::static_type())
                .and_downcast::<adw::ExpanderRow>()
            {
                super::audit::label(sw, &row.title());
            }
        }
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        name_enable_switches(&child);
        c = child.next_sibling();
    }
}

impl TweaksUi {
    pub fn build(get: Get, change: Change) -> TweaksUi {
        let line_height = expander("Line height", "Space between lines");
        let line_height_v = spin("Multiple of the font size", 1.0, 3.0, 0.05, 2, 1.4);
        line_height.add_row(&line_height_v);
        let margin = expander("Margins", "Space around the page");
        let margin_v = spin("Pixels", 0.0, 200.0, 4.0, 0, 56.0);
        margin.add_row(&margin_v);
        let spacing = expander("Paragraph spacing", "Space after each paragraph");
        let spacing_v = spin("Em", 0.0, 3.0, 0.1, 1, 0.65);
        spacing.add_row(&spacing_v);
        let font = expander("Font", "Replaces the font chosen above");
        let font_v = adw::EntryRow::new();
        font_v.set_title(&tr("Font family"));
        font.add_row(&font_v);
        let justify = expander("Justification", "Force the alignment of paragraphs");
        let justify_v = adw::SwitchRow::new();
        justify_v.set_title(&tr("Justify text"));
        justify.add_row(&justify_v);
        let optimal = expander(
            "Optimal line breaking",
            "Knuth-Plass breaks in justified text; off uses the basic breaking",
        );
        let optimal_v = adw::SwitchRow::new();
        optimal_v.set_title(&tr("Use optimal line breaking"));
        optimal_v.set_active(true);
        optimal.add_row(&optimal_v);
        let hyphen = adw::ComboRow::new();
        hyphen.set_title(&tr("Hyphenation language"));
        let labels: Vec<&str> = HYPHEN_LANGUAGES.iter().map(|(_, l)| *l).collect();
        hyphen.set_model(Some(&gtk::StringList::new(&labels)));

        let text_group = adw::PreferencesGroup::new();
        for r in [&spacing, &optimal] {
            text_group.add(r);
        }
        text_group.add(&hyphen);

        // the overrides and the style sheet sit in one collapsed row
        let advanced_group = adw::PreferencesGroup::new();
        let advanced = adw::ExpanderRow::new();
        advanced.set_title(&tr("Advanced and custom CSS"));
        advanced.set_subtitle(&tr("Overrides and your own style sheet"));
        for r in [&line_height, &margin, &font, &justify] {
            advanced.add_row(r);
        }
        let css = gtk::TextView::new();
        css.set_monospace(true);
        css.set_wrap_mode(gtk::WrapMode::WordChar);
        css.set_top_margin(8);
        css.set_bottom_margin(8);
        css.set_left_margin(8);
        css.set_right_margin(8);
        super::audit::label(&css, &tr("Custom style sheet"));
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(110);
        scroll.set_child(Some(&css));
        scroll.add_css_class("card");
        let css_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        css_box.set_margin_top(8);
        css_box.set_margin_bottom(8);
        css_box.set_margin_start(8);
        css_box.set_margin_end(8);
        let css_hint = gtk::Label::new(Some(&tr(
            "CSS that is applied after the styling of the book.",
        )));
        css_hint.set_xalign(0.0);
        css_hint.set_wrap(true);
        css_hint.add_css_class("caption");
        css_box.append(&css_hint);
        css_box.append(&scroll);
        let apply = gtk::Button::with_label(&tr("Apply Style Sheet"));
        apply.add_css_class("suggested-action");
        apply.set_halign(gtk::Align::End);
        css_box.append(&apply);
        let reset = gtk::Button::with_label(&tr("Reset These Tweaks"));
        reset.set_halign(gtk::Align::End);
        css_box.append(&reset);
        let css_row = adw::PreferencesRow::new();
        css_row.set_activatable(false);
        css_row.set_child(Some(&css_box));
        advanced.add_row(&css_row);
        advanced_group.add(&advanced);
        name_enable_switches(text_group.upcast_ref());
        name_enable_switches(advanced_group.upcast_ref());

        let ui = TweaksUi {
            text_group,
            advanced_group,
            line_height,
            line_height_v,
            margin,
            margin_v,
            spacing,
            spacing_v,
            font,
            font_v,
            justify,
            justify_v,
            optimal,
            optimal_v,
            hyphen,
            css,
            quiet: Rc::new(Cell::new(false)),
            get,
            change,
        };
        ui.fill(&(ui.get)());

        // every change of a control is applied right away
        let emit = {
            let ui = ui.clone();
            move || ui.emit()
        };
        for r in [
            &ui.line_height,
            &ui.margin,
            &ui.spacing,
            &ui.font,
            &ui.justify,
            &ui.optimal,
        ] {
            let e = emit.clone();
            r.connect_enable_expansion_notify(move |_| e());
        }
        for r in [&ui.line_height_v, &ui.margin_v, &ui.spacing_v] {
            let e = emit.clone();
            r.connect_value_notify(move |_| e());
        }
        for r in [&ui.justify_v, &ui.optimal_v] {
            let e = emit.clone();
            r.connect_active_notify(move |_| e());
        }
        {
            let e = emit.clone();
            ui.font_v.connect_changed(move |_| e());
            let e = emit.clone();
            ui.hyphen.connect_selected_notify(move |_| e());
            let e = emit.clone();
            apply.connect_clicked(move |_| e());
        }
        {
            let u = ui.clone();
            reset.connect_clicked(move |_| {
                u.fill(&StyleTweaks::default());
                u.emit();
            });
        }
        ui
    }

    /// Shows what is stored now (after a reset or a new default) without reporting a change.
    pub fn refresh(&self) {
        self.fill(&(self.get)());
    }

    /// The tweaks the controls show.
    pub fn collect(&self) -> StyleTweaks {
        let b = self.css.buffer();
        let (s, e) = b.bounds();
        let font = self.font_v.text().to_string();
        let hyphen = HYPHEN_LANGUAGES
            .get(self.hyphen.selected() as usize)
            .map(|(v, _)| v.to_string())
            .filter(|v| !v.is_empty());
        StyleTweaks {
            line_height: self
                .line_height
                .enables_expansion()
                .then(|| self.line_height_v.value()),
            margin: self
                .margin
                .enables_expansion()
                .then(|| self.margin_v.value()),
            paragraph_spacing: self
                .spacing
                .enables_expansion()
                .then(|| self.spacing_v.value()),
            font: self.font.enables_expansion().then_some(font),
            justify: self
                .justify
                .enables_expansion()
                .then(|| self.justify_v.is_active()),
            optimal_breaks: self
                .optimal
                .enables_expansion()
                .then(|| self.optimal_v.is_active()),
            hyphenation: hyphen,
            user_css: b.text(&s, &e, false).to_string(),
        }
    }

    fn emit(&self) {
        if self.quiet.get() {
            return;
        }
        (self.change)(self.collect().clamped());
    }

    /// Shows `t` in the controls without reporting a change.
    fn fill(&self, t: &StyleTweaks) {
        self.quiet.set(true);
        self.line_height
            .set_enable_expansion(t.line_height.is_some());
        if let Some(v) = t.line_height {
            self.line_height_v.set_value(v);
        }
        self.margin.set_enable_expansion(t.margin.is_some());
        if let Some(v) = t.margin {
            self.margin_v.set_value(v);
        }
        self.spacing
            .set_enable_expansion(t.paragraph_spacing.is_some());
        if let Some(v) = t.paragraph_spacing {
            self.spacing_v.set_value(v);
        }
        self.font.set_enable_expansion(t.font.is_some());
        self.font_v.set_text(t.font.as_deref().unwrap_or(""));
        self.justify.set_enable_expansion(t.justify.is_some());
        self.justify_v.set_active(t.justify.unwrap_or(true));
        self.optimal
            .set_enable_expansion(t.optimal_breaks.is_some());
        self.optimal_v.set_active(t.optimal_breaks.unwrap_or(true));
        let at = HYPHEN_LANGUAGES
            .iter()
            .position(|(v, _)| Some(*v) == t.hyphenation.as_deref())
            .unwrap_or(0);
        self.hyphen.set_selected(at as u32);
        self.css.buffer().set_text(&t.user_css);
        self.quiet.set(false);
    }

    /// Test helper: sets one control the way a user would (`key=value`).
    pub fn dev_set(&self, key: &str, value: &str) {
        match key {
            "line_height" => {
                self.line_height.set_enable_expansion(true);
                self.line_height_v.set_value(value.parse().unwrap_or(1.4));
            }
            "margin" => {
                self.margin.set_enable_expansion(true);
                self.margin_v.set_value(value.parse().unwrap_or(56.0));
            }
            "spacing" => {
                self.spacing.set_enable_expansion(true);
                self.spacing_v.set_value(value.parse().unwrap_or(0.65));
            }
            "font" => {
                self.font.set_enable_expansion(true);
                self.font_v.set_text(value);
            }
            "justify" => {
                self.justify.set_enable_expansion(true);
                self.justify_v.set_active(value == "on");
            }
            "optimal" => {
                self.optimal.set_enable_expansion(true);
                self.optimal_v.set_active(value == "on");
            }
            "hyphen" => {
                let at = HYPHEN_LANGUAGES
                    .iter()
                    .position(|(v, _)| *v == value)
                    .unwrap_or(0);
                self.hyphen.set_selected(at as u32);
            }
            "css" => {
                self.css.buffer().set_text(value);
                self.emit();
            }
            "reset" => {
                self.fill(&StyleTweaks::default());
                self.emit();
            }
            _ => {}
        }
    }
}
