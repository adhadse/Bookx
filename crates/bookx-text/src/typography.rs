//! Typography settings (R23), per-book overrides (R19) and reading profiles with auto triggers (R34).

use serde::{Deserialize, Serialize};

pub const FONT_SIZE_RANGE: (f64, f64) = (6.0, 72.0);
pub const LINE_HEIGHT_RANGE: (f64, f64) = (0.8, 3.0);
pub const PARAGRAPH_SPACING_RANGE: (f64, f64) = (0.0, 3.0);
pub const MARGIN_RANGE: (f64, f64) = (0.0, 400.0);
pub const WORD_SPACING_RANGE: (f64, f64) = (-0.25, 1.0);
pub const WEIGHT_RANGE: (u16, u16) = (100, 900);

/// All user-tunable text appearance settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypographySettings {
    pub font_family: String,
    /// Points.
    pub font_size: f64,
    /// 100..=900.
    pub font_weight: u16,
    /// Multiplier of the font size.
    pub line_height: f64,
    /// Extra space between paragraphs, in em.
    pub paragraph_spacing: f64,
    /// Page margins in px.
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub margin_left: f64,
    pub margin_right: f64,
    pub justify: bool,
    pub hyphenate: bool,
    /// Extra word spacing in em (negative tightens).
    pub word_spacing: f64,
    /// Use the book's own CSS (R22). Off ignores publisher styles.
    pub embedded_styles: bool,
    /// Use fonts embedded in the book (R22).
    pub embedded_fonts: bool,
}

impl Default for TypographySettings {
    fn default() -> Self {
        TypographySettings {
            font_family: "Serif".into(),
            font_size: 12.0,
            font_weight: 400,
            line_height: 1.4,
            paragraph_spacing: 0.0,
            margin_top: 48.0,
            margin_bottom: 48.0,
            margin_left: 64.0,
            margin_right: 64.0,
            justify: true,
            hyphenate: true,
            word_spacing: 0.0,
            embedded_styles: true,
            embedded_fonts: true,
        }
    }
}

/// Built-in presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Default,
    Compact,
    Comfortable,
    LargePrint,
    Paperback,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Default,
        Preset::Compact,
        Preset::Comfortable,
        Preset::LargePrint,
        Preset::Paperback,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Default => "default",
            Preset::Compact => "compact",
            Preset::Comfortable => "comfortable",
            Preset::LargePrint => "large_print",
            Preset::Paperback => "paperback",
        }
    }

    pub fn from_name(s: &str) -> Option<Preset> {
        Preset::ALL
            .into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(s.trim()))
    }
}

fn clamp_f(v: f64, (lo, hi): (f64, f64), fallback: f64) -> f64 {
    if v.is_nan() {
        fallback
    } else {
        v.clamp(lo, hi)
    }
}

impl TypographySettings {
    pub fn preset(p: Preset) -> Self {
        let base = TypographySettings::default();
        match p {
            Preset::Default => base,
            Preset::Compact => TypographySettings {
                font_size: 11.0,
                line_height: 1.25,
                margin_top: 24.0,
                margin_bottom: 24.0,
                margin_left: 32.0,
                margin_right: 32.0,
                ..base
            },
            Preset::Comfortable => TypographySettings {
                font_size: 13.0,
                line_height: 1.6,
                paragraph_spacing: 0.4,
                margin_left: 96.0,
                margin_right: 96.0,
                ..base
            },
            Preset::LargePrint => TypographySettings {
                font_size: 18.0,
                line_height: 1.6,
                font_weight: 500,
                justify: false,
                hyphenate: false,
                margin_left: 48.0,
                margin_right: 48.0,
                ..base
            },
            Preset::Paperback => TypographySettings {
                font_size: 11.5,
                line_height: 1.3,
                margin_top: 56.0,
                margin_bottom: 56.0,
                margin_left: 80.0,
                margin_right: 80.0,
                ..base
            },
        }
    }

    /// Descriptions of everything outside its allowed range (empty when valid).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |name: &str, v: f64, r: (f64, f64)| {
            if v.is_nan() || v < r.0 || v > r.1 {
                out.push(format!("{name} {v} is outside {}..={}", r.0, r.1));
            }
        };
        check("font_size", self.font_size, FONT_SIZE_RANGE);
        check("line_height", self.line_height, LINE_HEIGHT_RANGE);
        check(
            "paragraph_spacing",
            self.paragraph_spacing,
            PARAGRAPH_SPACING_RANGE,
        );
        check("margin_top", self.margin_top, MARGIN_RANGE);
        check("margin_bottom", self.margin_bottom, MARGIN_RANGE);
        check("margin_left", self.margin_left, MARGIN_RANGE);
        check("margin_right", self.margin_right, MARGIN_RANGE);
        check("word_spacing", self.word_spacing, WORD_SPACING_RANGE);
        if self.font_weight < WEIGHT_RANGE.0 || self.font_weight > WEIGHT_RANGE.1 {
            out.push(format!(
                "font_weight {} is outside {}..={}",
                self.font_weight, WEIGHT_RANGE.0, WEIGHT_RANGE.1
            ));
        }
        if self.font_family.trim().is_empty() {
            out.push("font_family is empty".into());
        }
        out
    }

    pub fn is_valid(&self) -> bool {
        self.problems().is_empty()
    }

    /// Clamp every value into its range (NaN falls back to the default, empty family to the default family,
    /// weight is rounded to the nearest 100).
    pub fn clamped(mut self) -> Self {
        let d = TypographySettings::default();
        self.font_size = clamp_f(self.font_size, FONT_SIZE_RANGE, d.font_size);
        self.line_height = clamp_f(self.line_height, LINE_HEIGHT_RANGE, d.line_height);
        self.paragraph_spacing = clamp_f(
            self.paragraph_spacing,
            PARAGRAPH_SPACING_RANGE,
            d.paragraph_spacing,
        );
        self.margin_top = clamp_f(self.margin_top, MARGIN_RANGE, d.margin_top);
        self.margin_bottom = clamp_f(self.margin_bottom, MARGIN_RANGE, d.margin_bottom);
        self.margin_left = clamp_f(self.margin_left, MARGIN_RANGE, d.margin_left);
        self.margin_right = clamp_f(self.margin_right, MARGIN_RANGE, d.margin_right);
        self.word_spacing = clamp_f(self.word_spacing, WORD_SPACING_RANGE, d.word_spacing);
        self.font_weight = (((self.font_weight.clamp(WEIGHT_RANGE.0, WEIGHT_RANGE.1) as u32 + 50)
            / 100)
            * 100) as u16;
        if self.font_family.trim().is_empty() {
            self.font_family = d.font_family;
        }
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parse JSON; missing fields take defaults, unknown fields are ignored, values are clamped.
    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        let v: TypographySettings = serde_json::from_str(s)?;
        Ok(v.clamped())
    }
}

/// Per-book (or per-profile) overrides: only the fields that are set replace the base.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TypographyOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_bottom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hyphenate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_spacing: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_styles: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_fonts: Option<bool>,
}

macro_rules! override_fields {
    ($mac:ident) => {
        $mac!(
            font_family,
            font_size,
            font_weight,
            line_height,
            paragraph_spacing,
            margin_top,
            margin_bottom,
            margin_left,
            margin_right,
            justify,
            hyphenate,
            word_spacing,
            embedded_styles,
            embedded_fonts
        )
    };
}

impl TypographyOverride {
    pub fn is_empty(&self) -> bool {
        *self == TypographyOverride::default()
    }

    /// Layer the set fields over `base` and clamp the result.
    pub fn apply(&self, base: &TypographySettings) -> TypographySettings {
        let mut out = base.clone();
        macro_rules! apply {
            ($($f:ident),*) => { $( if let Some(v) = &self.$f { out.$f = v.clone(); } )* };
        }
        override_fields!(apply);
        out.clamped()
    }

    /// Combine two overrides; fields set in `top` win.
    pub fn layered(&self, top: &TypographyOverride) -> TypographyOverride {
        let mut out = self.clone();
        macro_rules! layer {
            ($($f:ident),*) => { $( if top.$f.is_some() { out.$f = top.$f.clone(); } )* };
        }
        override_fields!(layer);
        out
    }

    /// The override that turns `base` into `target` (only differing fields are set).
    pub fn diff(base: &TypographySettings, target: &TypographySettings) -> TypographyOverride {
        let mut out = TypographyOverride::default();
        macro_rules! d {
            ($($f:ident),*) => { $( if base.$f != target.$f { out.$f = Some(target.$f.clone()); } )* };
        }
        override_fields!(d);
        out
    }

    /// Remove one setting by its field name. Returns whether the name is known.
    pub fn reset(&mut self, field: &str) -> bool {
        macro_rules! r {
            ($($f:ident),*) => { match field { $( stringify!($f) => { self.$f = None; true } )* _ => false } };
        }
        override_fields!(r)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

/// A named bundle of overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingProfile {
    pub name: String,
    #[serde(default)]
    pub settings: TypographyOverride,
}

impl ReadingProfile {
    pub fn new(name: impl Into<String>, settings: TypographyOverride) -> Self {
        ReadingProfile {
            name: name.into(),
            settings,
        }
    }
}

/// One condition of an auto-trigger rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// The book is in a collection with this name (case-insensitive).
    Collection { name: String },
    /// The book has this id.
    Book { id: String },
    /// The book language matches the primary subtag (`en` matches `en-GB`).
    Language { code: String },
    /// Local time of day window `[start, end)` in minutes since midnight; wraps over midnight when
    /// `start > end` (22:00-06:00 is `1320, 360`).
    TimeWindow { start_minute: u16, end_minute: u16 },
}

/// A rule: all triggers must match for the profile to be picked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileRule {
    pub profile: String,
    pub triggers: Vec<Trigger>,
}

/// What is known when a book is opened.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProfileContext {
    pub book_id: Option<String>,
    pub collections: Vec<String>,
    pub language: Option<String>,
    pub minute_of_day: u16,
}

impl Trigger {
    pub fn matches(&self, ctx: &ProfileContext) -> bool {
        match self {
            Trigger::Collection { name } => ctx
                .collections
                .iter()
                .any(|c| c.trim().eq_ignore_ascii_case(name.trim())),
            Trigger::Book { id } => ctx.book_id.as_deref() == Some(id.as_str()),
            Trigger::Language { code } => {
                let want = crate::hyphenate::primary_subtag(code);
                let have = ctx
                    .language
                    .as_deref()
                    .and_then(crate::hyphenate::primary_subtag);
                want.is_some() && want == have
            }
            Trigger::TimeWindow {
                start_minute,
                end_minute,
            } => {
                let m = ctx.minute_of_day % 1440;
                let (s, e) = (*start_minute % 1440, *end_minute % 1440);
                if s == e {
                    false
                } else if s < e {
                    m >= s && m < e
                } else {
                    m >= s || m < e
                }
            }
        }
    }
}

/// The profile name of the first rule whose triggers all match. A rule without triggers never matches.
pub fn pick_profile<'a>(rules: &'a [ProfileRule], ctx: &ProfileContext) -> Option<&'a str> {
    rules
        .iter()
        .find(|r| !r.triggers.is_empty() && r.triggers.iter().all(|t| t.matches(ctx)))
        .map(|r| r.profile.as_str())
}

/// Profiles and their rules.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileSet {
    pub profiles: Vec<ReadingProfile>,
    pub rules: Vec<ProfileRule>,
}

impl ProfileSet {
    pub fn profile(&self, name: &str) -> Option<&ReadingProfile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    /// Add or replace a profile by name.
    pub fn upsert(&mut self, p: ReadingProfile) {
        match self.profiles.iter_mut().find(|x| x.name == p.name) {
            Some(x) => *x = p,
            None => self.profiles.push(p),
        }
    }

    /// Remove a profile and its rules. Returns whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.profiles.len();
        self.profiles.retain(|p| p.name != name);
        self.rules.retain(|r| r.profile != name);
        self.profiles.len() != before
    }

    /// The profile picked automatically for a context (rules that point to a missing profile are skipped).
    pub fn pick(&self, ctx: &ProfileContext) -> Option<&ReadingProfile> {
        self.rules
            .iter()
            .filter(|r| self.profile(&r.profile).is_some())
            .find(|r| !r.triggers.is_empty() && r.triggers.iter().all(|t| t.matches(ctx)))
            .and_then(|r| self.profile(&r.profile))
    }

    /// Final settings: defaults, then the manually chosen profile (or the auto-picked one), then the per-book override.
    pub fn effective(
        &self,
        defaults: &TypographySettings,
        manual_profile: Option<&str>,
        book: &TypographyOverride,
        ctx: &ProfileContext,
    ) -> TypographySettings {
        let profile = manual_profile
            .and_then(|n| self.profile(n))
            .or_else(|| self.pick(ctx));
        let layered = match profile {
            Some(p) => p.settings.layered(book),
            None => book.clone(),
        };
        layered.apply(defaults)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ProfileContext {
        ProfileContext {
            book_id: Some("b1".into()),
            collections: vec!["Sci-Fi".into(), "To Read".into()],
            language: Some("en-GB".into()),
            minute_of_day: 12 * 60,
        }
    }

    #[test]
    fn defaults_are_valid_and_sane() {
        let d = TypographySettings::default();
        assert!(d.is_valid(), "{:?}", d.problems());
        assert!(d.embedded_styles && d.embedded_fonts && d.justify && d.hyphenate);
        assert_eq!(d.font_weight, 400);
    }

    #[test]
    fn presets_valid_distinct_and_named() {
        let mut seen = Vec::new();
        for p in Preset::ALL {
            let s = TypographySettings::preset(p);
            assert!(s.is_valid(), "{:?}: {:?}", p, s.problems());
            assert!(!seen.contains(&s) || p == Preset::Default);
            seen.push(s);
            assert_eq!(Preset::from_name(p.name()), Some(p));
            let j = serde_json::to_string(&p).unwrap();
            assert_eq!(j, format!("\"{}\"", p.name()));
        }
        assert_eq!(
            TypographySettings::preset(Preset::Default),
            TypographySettings::default()
        );
        assert!(
            TypographySettings::preset(Preset::LargePrint).font_size
                > TypographySettings::preset(Preset::Compact).font_size
        );
        assert!(!TypographySettings::preset(Preset::LargePrint).justify);
        assert_eq!(Preset::from_name(" LARGE_print "), Some(Preset::LargePrint));
        assert_eq!(Preset::from_name("huge"), None);
    }

    #[test]
    fn problems_lists_every_violation() {
        let bad = TypographySettings {
            font_size: 2.0,
            line_height: 9.0,
            paragraph_spacing: -1.0,
            margin_top: -5.0,
            margin_bottom: 1000.0,
            margin_left: f64::NAN,
            margin_right: 500.0,
            word_spacing: 5.0,
            font_weight: 50,
            font_family: "  ".into(),
            ..TypographySettings::default()
        };
        let p = bad.problems();
        assert_eq!(p.len(), 10, "{p:?}");
        assert!(p[0].starts_with("font_size 2 is outside 6..=72"));
        assert!(p.iter().any(|m| m.contains("font_family is empty")));
        assert!(p
            .iter()
            .any(|m| m.starts_with("font_weight 50 is outside 100..=900")));
        assert!(!bad.is_valid());
    }

    #[test]
    fn clamped_fixes_everything() {
        let bad = TypographySettings {
            font_size: 2.0,
            line_height: f64::NAN,
            paragraph_spacing: 99.0,
            margin_top: -5.0,
            margin_bottom: 1000.0,
            word_spacing: -4.0,
            font_weight: 449,
            font_family: "".into(),
            ..TypographySettings::default()
        };
        let c = bad.clamped();
        assert!(c.is_valid(), "{:?}", c.problems());
        assert_eq!(c.font_size, 6.0);
        assert_eq!(c.line_height, TypographySettings::default().line_height);
        assert_eq!(c.paragraph_spacing, 3.0);
        assert_eq!(c.margin_top, 0.0);
        assert_eq!(c.margin_bottom, 400.0);
        assert_eq!(c.word_spacing, -0.25);
        assert_eq!(c.font_weight, 400);
        assert_eq!(c.font_family, "Serif");
        let w = |v: u16| {
            TypographySettings {
                font_weight: v,
                ..Default::default()
            }
            .clamped()
            .font_weight
        };
        assert_eq!(
            (w(0), w(450), w(449), w(1000), w(900)),
            (100, 500, 400, 900, 900)
        );
    }

    #[test]
    fn clamped_is_idempotent_and_keeps_valid_values() {
        for p in Preset::ALL {
            let s = TypographySettings::preset(p);
            assert_eq!(s.clone().clamped(), s);
        }
    }

    #[test]
    fn json_roundtrip_all_fields() {
        let s = TypographySettings {
            font_family: "Literata".into(),
            font_size: 15.5,
            font_weight: 600,
            line_height: 1.75,
            paragraph_spacing: 0.5,
            margin_top: 10.0,
            margin_bottom: 20.0,
            margin_left: 30.0,
            margin_right: 40.0,
            justify: false,
            hyphenate: false,
            word_spacing: 0.1,
            embedded_styles: false,
            embedded_fonts: false,
        };
        let back = TypographySettings::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn json_partial_unknown_and_invalid() {
        let s =
            TypographySettings::from_json(r#"{"font_size": 20, "future_field": true}"#).unwrap();
        assert_eq!(s.font_size, 20.0);
        assert_eq!(s.line_height, TypographySettings::default().line_height);
        let s = TypographySettings::from_json(r#"{"font_size": 500}"#).unwrap();
        assert_eq!(s.font_size, 72.0);
        assert_eq!(
            TypographySettings::from_json("{}").unwrap(),
            TypographySettings::default()
        );
        assert!(TypographySettings::from_json("[1]").is_err());
        assert!(TypographySettings::from_json(r#"{"font_size":"big"}"#).is_err());
    }

    #[test]
    fn override_apply_layers_only_set_fields() {
        let base = TypographySettings::default();
        let o = TypographyOverride {
            font_size: Some(16.0),
            justify: Some(false),
            font_family: Some("Sans".into()),
            ..Default::default()
        };
        let out = o.apply(&base);
        assert_eq!(out.font_size, 16.0);
        assert!(!out.justify);
        assert_eq!(out.font_family, "Sans");
        assert_eq!(out.line_height, base.line_height);
        assert_eq!(out.margin_left, base.margin_left);
        assert!(TypographyOverride::default().apply(&base) == base);
        assert!(TypographyOverride::default().is_empty());
        assert!(!o.is_empty());
    }

    #[test]
    fn override_apply_clamps() {
        let o = TypographyOverride {
            font_size: Some(1000.0),
            line_height: Some(0.0),
            ..Default::default()
        };
        let out = o.apply(&TypographySettings::default());
        assert_eq!(out.font_size, 72.0);
        assert_eq!(out.line_height, 0.8);
    }

    #[test]
    fn override_every_field_applies() {
        let o = TypographyOverride {
            font_family: Some("F".into()),
            font_size: Some(20.0),
            font_weight: Some(700),
            line_height: Some(2.0),
            paragraph_spacing: Some(1.0),
            margin_top: Some(1.0),
            margin_bottom: Some(2.0),
            margin_left: Some(3.0),
            margin_right: Some(4.0),
            justify: Some(false),
            hyphenate: Some(false),
            word_spacing: Some(0.2),
            embedded_styles: Some(false),
            embedded_fonts: Some(false),
        };
        let out = o.apply(&TypographySettings::default());
        assert_eq!(
            out,
            TypographySettings {
                font_family: "F".into(),
                font_size: 20.0,
                font_weight: 700,
                line_height: 2.0,
                paragraph_spacing: 1.0,
                margin_top: 1.0,
                margin_bottom: 2.0,
                margin_left: 3.0,
                margin_right: 4.0,
                justify: false,
                hyphenate: false,
                word_spacing: 0.2,
                embedded_styles: false,
                embedded_fonts: false,
            }
        );
        assert_eq!(
            TypographyOverride::diff(&TypographySettings::default(), &out),
            o
        );
    }

    #[test]
    fn override_layered_top_wins() {
        let a = TypographyOverride {
            font_size: Some(14.0),
            justify: Some(true),
            ..Default::default()
        };
        let b = TypographyOverride {
            font_size: Some(18.0),
            hyphenate: Some(false),
            ..Default::default()
        };
        let l = a.layered(&b);
        assert_eq!(l.font_size, Some(18.0));
        assert_eq!(l.justify, Some(true));
        assert_eq!(l.hyphenate, Some(false));
        assert_eq!(a.layered(&TypographyOverride::default()), a);
        assert_eq!(TypographyOverride::default().layered(&b), b);
    }

    #[test]
    fn override_diff_and_reset() {
        let base = TypographySettings::default();
        let target = TypographySettings {
            font_size: 14.0,
            ..base.clone()
        };
        let mut d = TypographyOverride::diff(&base, &target);
        assert_eq!(
            d,
            TypographyOverride {
                font_size: Some(14.0),
                ..Default::default()
            }
        );
        assert_eq!(d.apply(&base), target);
        assert!(TypographyOverride::diff(&base, &base).is_empty());
        assert!(d.reset("font_size"));
        assert!(d.is_empty());
        assert!(!d.reset("nonsense"));
        let mut all =
            TypographyOverride::diff(&base, &TypographySettings::preset(Preset::LargePrint));
        for f in [
            "font_size",
            "line_height",
            "font_weight",
            "justify",
            "hyphenate",
            "margin_left",
            "margin_right",
        ] {
            assert!(all.reset(f));
        }
        assert!(all.is_empty());
    }

    #[test]
    fn override_json_skips_unset_and_roundtrips() {
        let o = TypographyOverride {
            font_size: Some(14.0),
            hyphenate: Some(false),
            ..Default::default()
        };
        let j = o.to_json();
        assert_eq!(j, r#"{"font_size":14.0,"hyphenate":false}"#);
        assert_eq!(TypographyOverride::from_json(&j).unwrap(), o);
        assert_eq!(
            TypographyOverride::from_json("{}").unwrap(),
            TypographyOverride::default()
        );
        assert!(TypographyOverride::from_json("nope").is_err());
        assert!(TypographyOverride::from_json(r#"{"font_size":"x"}"#).is_err());
    }

    #[test]
    fn trigger_collection_book_language() {
        let c = ctx();
        assert!(Trigger::Collection {
            name: "sci-fi".into()
        }
        .matches(&c));
        assert!(Trigger::Collection {
            name: " To Read ".into()
        }
        .matches(&c));
        assert!(!Trigger::Collection {
            name: "Horror".into()
        }
        .matches(&c));
        assert!(Trigger::Book { id: "b1".into() }.matches(&c));
        assert!(!Trigger::Book { id: "b2".into() }.matches(&c));
        assert!(!Trigger::Book { id: "b1".into() }.matches(&ProfileContext::default()));
        assert!(Trigger::Language { code: "en".into() }.matches(&c));
        assert!(Trigger::Language {
            code: "EN-us".into()
        }
        .matches(&c));
        assert!(!Trigger::Language { code: "de".into() }.matches(&c));
        assert!(!Trigger::Language { code: "en".into() }.matches(&ProfileContext::default()));
        assert!(
            !Trigger::Language { code: "".into() }.matches(&ProfileContext {
                language: Some("".into()),
                ..c
            })
        );
    }

    #[test]
    fn trigger_time_window() {
        let at = |m: u16| ProfileContext {
            minute_of_day: m,
            ..Default::default()
        };
        let day = Trigger::TimeWindow {
            start_minute: 9 * 60,
            end_minute: 17 * 60,
        };
        assert!(day.matches(&at(9 * 60)));
        assert!(day.matches(&at(16 * 60 + 59)));
        assert!(!day.matches(&at(17 * 60)));
        assert!(!day.matches(&at(8 * 60 + 59)));
        let night = Trigger::TimeWindow {
            start_minute: 22 * 60,
            end_minute: 6 * 60,
        };
        assert!(night.matches(&at(23 * 60)));
        assert!(night.matches(&at(0)));
        assert!(night.matches(&at(5 * 60 + 59)));
        assert!(!night.matches(&at(6 * 60)));
        assert!(!night.matches(&at(12 * 60)));
        assert!(night.matches(&at(22 * 60)));
        let empty = Trigger::TimeWindow {
            start_minute: 100,
            end_minute: 100,
        };
        assert!(!empty.matches(&at(100)));
        // minute values past a day wrap
        assert!(day.matches(&at(1440 + 10 * 60)));
    }

    #[test]
    fn pick_profile_first_match_all_triggers() {
        let rules = vec![
            ProfileRule {
                profile: "night".into(),
                triggers: vec![Trigger::TimeWindow {
                    start_minute: 1200,
                    end_minute: 360,
                }],
            },
            ProfileRule {
                profile: "scifi-large".into(),
                triggers: vec![
                    Trigger::Collection {
                        name: "Sci-Fi".into(),
                    },
                    Trigger::Language { code: "en".into() },
                ],
            },
            ProfileRule {
                profile: "english".into(),
                triggers: vec![Trigger::Language { code: "en".into() }],
            },
            ProfileRule {
                profile: "nothing".into(),
                triggers: vec![],
            },
        ];
        let mut c = ctx();
        assert_eq!(pick_profile(&rules, &c), Some("scifi-large"));
        c.minute_of_day = 23 * 60;
        assert_eq!(pick_profile(&rules, &c), Some("night"));
        c.minute_of_day = 12 * 60;
        c.collections.clear();
        assert_eq!(pick_profile(&rules, &c), Some("english"));
        c.language = Some("fr".into());
        assert_eq!(
            pick_profile(&rules, &c),
            None,
            "an empty trigger list never matches"
        );
        assert_eq!(pick_profile(&[], &c), None);
    }

    #[test]
    fn profile_set_pick_effective_and_manage() {
        let mut set = ProfileSet::default();
        set.upsert(ReadingProfile::new(
            "big",
            TypographyOverride {
                font_size: Some(20.0),
                line_height: Some(1.8),
                ..Default::default()
            },
        ));
        set.upsert(ReadingProfile::new(
            "dark",
            TypographyOverride {
                font_weight: Some(500),
                ..Default::default()
            },
        ));
        set.rules.push(ProfileRule {
            profile: "ghost".into(),
            triggers: vec![Trigger::Language { code: "en".into() }],
        });
        set.rules.push(ProfileRule {
            profile: "big".into(),
            triggers: vec![Trigger::Collection {
                name: "sci-fi".into(),
            }],
        });
        let d = TypographySettings::default();
        let c = ctx();
        // rule pointing to an unknown profile is skipped
        assert_eq!(set.pick(&c).unwrap().name, "big");
        let eff = set.effective(&d, None, &TypographyOverride::default(), &c);
        assert_eq!((eff.font_size, eff.line_height), (20.0, 1.8));
        // manual choice beats auto
        let eff = set.effective(&d, Some("dark"), &TypographyOverride::default(), &c);
        assert_eq!((eff.font_size, eff.font_weight), (12.0, 500));
        // unknown manual falls through to auto
        let eff = set.effective(&d, Some("missing"), &TypographyOverride::default(), &c);
        assert_eq!(eff.font_size, 20.0);
        // book override beats profile
        let book = TypographyOverride {
            font_size: Some(9.0),
            ..Default::default()
        };
        let eff = set.effective(&d, None, &book, &c);
        assert_eq!((eff.font_size, eff.line_height), (9.0, 1.8));
        // no profile at all
        let eff = set.effective(&d, None, &book, &ProfileContext::default());
        assert_eq!((eff.font_size, eff.line_height), (9.0, d.line_height));
        // replace
        set.upsert(ReadingProfile::new(
            "big",
            TypographyOverride {
                font_size: Some(22.0),
                ..Default::default()
            },
        ));
        assert_eq!(set.profiles.len(), 2);
        assert_eq!(set.profile("big").unwrap().settings.font_size, Some(22.0));
        // remove drops rules too
        assert!(set.remove("big"));
        assert!(!set.remove("big"));
        assert_eq!(set.rules.len(), 1);
        assert!(set.pick(&c).is_none());
    }

    #[test]
    fn profile_set_json_roundtrip_with_golden() {
        let golden = r#"{
          "profiles": [{"name": "night", "settings": {"font_size": 14.0, "justify": false}}],
          "rules": [{"profile": "night", "triggers": [
              {"type": "time_window", "start_minute": 1320, "end_minute": 360},
              {"type": "collection", "name": "Bedtime"},
              {"type": "language", "code": "en"},
              {"type": "book", "id": "x"}]}]
        }"#;
        let set = ProfileSet::from_json(golden).unwrap();
        assert_eq!(set.profiles[0].settings.font_size, Some(14.0));
        assert_eq!(set.rules[0].triggers.len(), 4);
        assert_eq!(
            set.rules[0].triggers[0],
            Trigger::TimeWindow {
                start_minute: 1320,
                end_minute: 360
            }
        );
        let back = ProfileSet::from_json(&set.to_json()).unwrap();
        assert_eq!(back, set);
        assert!(ProfileSet::from_json(
            r#"{"rules":[{"profile":"a","triggers":[{"type":"moon"}]}]}"#
        )
        .is_err());
        assert!(ProfileSet::from_json("").is_err());
        assert_eq!(ProfileSet::from_json("{}").unwrap(), ProfileSet::default());
    }

    #[test]
    fn property_apply_always_valid() {
        let mut seed = 42u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let f = move |s: u64| ((s % 20000) as f64 - 5000.0) / 10.0;
        for _ in 0..500 {
            let o = TypographyOverride {
                font_size: Some(f(rnd())),
                line_height: Some(f(rnd()) / 50.0),
                margin_left: Some(f(rnd())),
                word_spacing: Some(f(rnd()) / 100.0),
                font_weight: Some((rnd() % 2000) as u16),
                ..Default::default()
            };
            assert!(o.apply(&TypographySettings::default()).is_valid());
        }
    }
}
