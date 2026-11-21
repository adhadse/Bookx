//! The style tweaks inside the reader's text appearance popover (roadmap R21, enhancement 22):
//! paragraph spacing, optimal line breaking, hyphenation language and, under "Advanced", the
//! overrides of line height, margins, font and justification and the reader's own style sheet.
//! Every change is the open book's own (it is part of the book's automatic look, see
//! `crate::reading_profiles`); the values live in [`crate::tweaks`].

use crate::i18n::{tr, N_};
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
    weight: adw::ExpanderRow,
    weight_v: adw::SpinRow,
    word_sp: adw::ExpanderRow,
    word_sp_v: adw::SpinRow,
    contrast: adw::ExpanderRow,
    contrast_v: adw::SpinRow,
    kerning: adw::ExpanderRow,
    kerning_v: adw::SwitchRow,
    widows: adw::ExpanderRow,
    widows_v: adw::SpinRow,
    book_styles: adw::ExpanderRow,
    book_styles_v: adw::SwitchRow,
    book_fonts: adw::ExpanderRow,
    book_fonts_v: adw::SwitchRow,
    css: gtk::TextView,
    quiet: Rc<Cell<bool>>,
    get: Get,
    change: Change,
}

fn expander(title: &str, subtitle: &str) -> adw::ExpanderRow {
    let r = adw::ExpanderRow::new();
    r.set_title(&tr(title));
    r.set_subtitle(&tr(subtitle));
    r.set_show_enable_switch(true);
    r
}

fn spin(title: &str, lo: f64, hi: f64, step: f64, digits: u32, value: f64) -> adw::SpinRow {
    let r = adw::SpinRow::with_range(lo, hi, step);
    r.set_title(&tr(title));
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
        let line_height = expander(N_("Line height"), N_("Space between lines"));
        let line_height_v = spin(N_("Multiple of the font size"), 1.0, 3.0, 0.05, 2, 1.4);
        line_height.add_row(&line_height_v);
        let margin = expander(N_("Margins"), N_("Space around the page"));
        let margin_v = spin(N_("Pixels"), 0.0, 200.0, 4.0, 0, 56.0);
        margin.add_row(&margin_v);
        let spacing = expander(N_("Paragraph spacing"), N_("Space after each paragraph"));
        let spacing_v = spin(N_("Em"), 0.0, 3.0, 0.1, 1, 0.65);
        spacing.add_row(&spacing_v);
        let font = expander(N_("Font"), N_("Replaces the font chosen above"));
        let font_v = adw::EntryRow::new();
        font_v.set_title(&tr("Font family"));
        font.add_row(&font_v);
        let justify = expander(N_("Justification"), N_("Force the alignment of paragraphs"));
        let justify_v = adw::SwitchRow::new();
        justify_v.set_title(&tr("Justify text"));
        justify.add_row(&justify_v);
        let optimal = expander(
            N_("Optimal line breaking"),
            N_("Knuth-Plass breaks in justified text; off uses the basic breaking"),
        );
        let optimal_v = adw::SwitchRow::new();
        optimal_v.set_title(&tr("Use optimal line breaking"));
        optimal_v.set_active(true);
        optimal.add_row(&optimal_v);
        let hyphen = adw::ComboRow::new();
        hyphen.set_title(&tr("Hyphenation language"));
        let labels: Vec<&str> = HYPHEN_LANGUAGES.iter().map(|(_, l)| *l).collect();
        hyphen.set_model(Some(&gtk::StringList::new(&labels)));

        let weight = expander(N_("Text weight"), N_("Lighter or heavier letters"));
        let weight_v = spin(N_("Weight"), 100.0, 900.0, 100.0, 0, 400.0);
        weight.add_row(&weight_v);
        let word_sp = expander(N_("Word spacing"), N_("Space between words"));
        let word_sp_v = spin(N_("Em"), -0.25, 1.0, 0.05, 2, 0.0);
        word_sp.add_row(&word_sp_v);
        let contrast = expander(N_("Text contrast"), N_("Softer or stronger text colour"));
        let contrast_v = spin(N_("Contrast"), 0.0, 1.0, 0.1, 1, 0.5);
        contrast.add_row(&contrast_v);
        let kerning = expander(N_("Kerning"), N_("Fine spacing of letter pairs"));
        let kerning_v = adw::SwitchRow::new();
        kerning_v.set_title(&tr("Use kerning"));
        kerning_v.set_active(true);
        kerning.add_row(&kerning_v);
        let widows = expander(
            N_("Widows and orphans"),
            N_("Least lines of a paragraph at the top or bottom of a page"),
        );
        let widows_v = spin(N_("Lines"), 1.0, 4.0, 1.0, 0, 2.0);
        widows.add_row(&widows_v);
        let book_styles = expander(
            N_("Book's own styling"),
            N_("Use or ignore the CSS of this book"),
        );
        let book_styles_v = adw::SwitchRow::new();
        book_styles_v.set_title(&tr("Use the book's styling"));
        book_styles_v.set_active(true);
        book_styles.add_row(&book_styles_v);
        let book_fonts = expander(
            N_("Book's fonts and sizes"),
            N_("Use or ignore the fonts and sizes the book asks for"),
        );
        let book_fonts_v = adw::SwitchRow::new();
        book_fonts_v.set_title(&tr("Use the book's fonts"));
        book_fonts_v.set_active(true);
        book_fonts.add_row(&book_fonts_v);

        let text_group = adw::PreferencesGroup::new();
        for r in [
            &spacing,
            &optimal,
            &weight,
            &word_sp,
            &contrast,
            &kerning,
            &widows,
            &book_styles,
            &book_fonts,
        ] {
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
            weight,
            weight_v,
            word_sp,
            word_sp_v,
            contrast,
            contrast_v,
            kerning,
            kerning_v,
            widows,
            widows_v,
            book_styles,
            book_styles_v,
            book_fonts,
            book_fonts_v,
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
            &ui.weight,
            &ui.word_sp,
            &ui.contrast,
            &ui.kerning,
            &ui.widows,
            &ui.book_styles,
            &ui.book_fonts,
        ] {
            let e = emit.clone();
            r.connect_enable_expansion_notify(move |_| e());
        }
        for r in [
            &ui.line_height_v,
            &ui.margin_v,
            &ui.spacing_v,
            &ui.weight_v,
            &ui.word_sp_v,
            &ui.contrast_v,
            &ui.widows_v,
        ] {
            let e = emit.clone();
            r.connect_value_notify(move |_| e());
        }
        for r in [
            &ui.justify_v,
            &ui.optimal_v,
            &ui.kerning_v,
            &ui.book_styles_v,
            &ui.book_fonts_v,
        ] {
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
            weight: self
                .weight
                .enables_expansion()
                .then(|| self.weight_v.value() as i32),
            contrast: self
                .contrast
                .enables_expansion()
                .then(|| self.contrast_v.value()),
            word_spacing: self
                .word_sp
                .enables_expansion()
                .then(|| self.word_sp_v.value()),
            kerning: self
                .kerning
                .enables_expansion()
                .then(|| self.kerning_v.is_active()),
            widows_orphans: self
                .widows
                .enables_expansion()
                .then(|| self.widows_v.value() as u8),
            book_styles: self
                .book_styles
                .enables_expansion()
                .then(|| self.book_styles_v.is_active()),
            book_fonts: self
                .book_fonts
                .enables_expansion()
                .then(|| self.book_fonts_v.is_active()),
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
        self.weight.set_enable_expansion(t.weight.is_some());
        self.weight_v.set_value(t.weight.unwrap_or(400) as f64);
        self.word_sp.set_enable_expansion(t.word_spacing.is_some());
        self.word_sp_v.set_value(t.word_spacing.unwrap_or(0.0));
        self.contrast.set_enable_expansion(t.contrast.is_some());
        self.contrast_v.set_value(t.contrast.unwrap_or(0.5));
        self.kerning.set_enable_expansion(t.kerning.is_some());
        self.kerning_v.set_active(t.kerning.unwrap_or(true));
        self.widows.set_enable_expansion(t.widows_orphans.is_some());
        self.widows_v
            .set_value(t.widows_orphans.unwrap_or(2) as f64);
        self.book_styles
            .set_enable_expansion(t.book_styles.is_some());
        self.book_styles_v.set_active(t.book_styles.unwrap_or(true));
        self.book_fonts.set_enable_expansion(t.book_fonts.is_some());
        self.book_fonts_v.set_active(t.book_fonts.unwrap_or(true));
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
            "weight" => {
                self.weight.set_enable_expansion(true);
                self.weight_v.set_value(value.parse().unwrap_or(400.0));
            }
            "word_spacing" => {
                self.word_sp.set_enable_expansion(true);
                self.word_sp_v.set_value(value.parse().unwrap_or(0.0));
            }
            "contrast" => {
                self.contrast.set_enable_expansion(true);
                self.contrast_v.set_value(value.parse().unwrap_or(0.5));
            }
            "kerning" => {
                self.kerning.set_enable_expansion(true);
                self.kerning_v.set_active(value == "on");
            }
            "widows" => {
                self.widows.set_enable_expansion(true);
                self.widows_v.set_value(value.parse().unwrap_or(2.0));
            }
            "book_styles" => {
                self.book_styles.set_enable_expansion(true);
                self.book_styles_v.set_active(value == "on");
            }
            "book_fonts" => {
                self.book_fonts.set_enable_expansion(true);
                self.book_fonts_v.set_active(value == "on");
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
