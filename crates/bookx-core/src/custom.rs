//! Custom columns (roadmap L3/L25): user-defined fields of a book that live in the catalogue, not
//! in the book file. A column has a label (`mood`, searched as `#mood:happy`), a display name and
//! a type; the values are stored per book as normalised strings in [`Book::custom`].
//!
//! Stored forms (what [`ColumnDef::normalize`] produces): text as typed, integers `42`, floats
//! `3.5`, ratings in stars `0.5`..`5` (steps of a half), yes/no `true`/`false`, dates `2024`,
//! `2024-05` or `2024-05-17`, tags `a, b` (sorted as given, no repeats), an enumeration value as
//! the defined spelling. An empty string is "no value" and is never stored.
//!
//! A composite column is computed from a template (`{#mood} - {title}`) by the renderer that
//! `bookx-sources` registers ([`register_composite_renderer`]); the result is stored too, so it
//! can be searched and sorted like any other value, and is refreshed whenever a book changes.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::history::OpKind;
use super::library::{Book, Library};

/// The most columns a library can define.
pub const MAX_COLUMNS: usize = 64;
/// The longest stored value, in characters.
pub const MAX_VALUE_CHARS: usize = 10_000;
/// The most values of one enumeration column.
pub const MAX_ENUM_VALUES: usize = 200;

/// The type of a custom column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    #[default]
    Text,
    LongText,
    Integer,
    Float,
    Rating,
    Bool,
    Date,
    Tags,
    Enumeration,
    Composite,
}

impl ColumnKind {
    pub const ALL: [ColumnKind; 10] = [
        Self::Text,
        Self::LongText,
        Self::Integer,
        Self::Float,
        Self::Rating,
        Self::Bool,
        Self::Date,
        Self::Tags,
        Self::Enumeration,
        Self::Composite,
    ];

    /// The name used in files and on the command line.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::LongText => "long_text",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::Rating => "rating",
            Self::Bool => "bool",
            Self::Date => "date",
            Self::Tags => "tags",
            Self::Enumeration => "enumeration",
            Self::Composite => "composite",
        }
    }

    pub fn parse(s: &str) -> Option<ColumnKind> {
        let s = s.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Self::ALL.into_iter().find(|k| k.key() == s)
    }

    /// True for the types whose values sort as numbers.
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer | Self::Float | Self::Rating)
    }
}

/// One custom column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ColumnDef {
    /// Lower-case letters, digits and `_`, starting with a letter: the `#label` of searches.
    pub label: String,
    /// What the user sees.
    pub name: String,
    pub kind: ColumnKind,
    /// The allowed values of an enumeration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// The template of a composite column.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
}

/// Search words and fields that a label must not shadow.
const RESERVED: &[&str] = &[
    "title",
    "author",
    "authors",
    "tag",
    "tags",
    "series",
    "publisher",
    "language",
    "lang",
    "format",
    "rating",
    "identifier",
    "identifiers",
    "isbn",
    "comments",
    "status",
    "progress",
    "size",
    "weak",
    "drm",
    "audiobook",
    "marked",
    "id",
    "date",
    "added",
    "modified",
    "pubdate",
    "published",
    "lastread",
    "search",
    "vl",
    "text",
];

/// True when `s` has the shape of a label (also used by the search parser, case-insensitive).
pub fn is_label_shape(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && s.as_bytes()[0].is_ascii_alphabetic()
}

/// A label made from a display name: `Reading mood` is `reading_mood`.
pub fn label_from_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = if out.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) {
        format!("c_{out}")
    } else {
        out
    };
    out.chars().take(32).collect()
}

fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

impl ColumnDef {
    /// Checks the definition on its own (not against other columns).
    pub fn validate(&self) -> Result<()> {
        if !is_label_shape(&self.label) || self.label != self.label.to_ascii_lowercase() {
            bail!(
                "`{}` is not a valid label: use lower-case letters, digits and _, starting with a letter (32 at most)",
                self.label
            );
        }
        if RESERVED.contains(&self.label.as_str()) {
            bail!("`{}` is the name of a built-in search field", self.label);
        }
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(|c| c.is_control()) {
            bail!("a column needs a name of 1 to 64 characters");
        }
        match self.kind {
            ColumnKind::Enumeration => {
                if self.values.is_empty() {
                    bail!("an enumeration column needs at least one value");
                }
                if self.values.len() > MAX_ENUM_VALUES {
                    bail!("an enumeration can have {MAX_ENUM_VALUES} values at most");
                }
                let mut seen = std::collections::HashSet::new();
                for v in &self.values {
                    let t = v.trim();
                    if t.is_empty() || t.chars().count() > 64 || t.contains(',') {
                        bail!("enumeration values are 1 to 64 characters without commas");
                    }
                    if !seen.insert(t.to_lowercase()) {
                        bail!("the value `{t}` is listed twice");
                    }
                }
            }
            ColumnKind::Composite => {
                if self.template.trim().is_empty() {
                    bail!("a composite column needs a template");
                }
                if self.template.chars().count() > 2000 {
                    bail!("a template can have 2000 characters at most");
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The stored form of what the user typed. `""` means "no value". Errors say what is wrong.
    pub fn normalize(&self, input: &str) -> Result<String> {
        let t = input.trim();
        if t.is_empty() {
            return Ok(String::new());
        }
        if t.chars().count() > MAX_VALUE_CHARS {
            bail!("a value can have {MAX_VALUE_CHARS} characters at most");
        }
        Ok(match self.kind {
            ColumnKind::Text => t.lines().next().unwrap_or("").trim().to_string(),
            ColumnKind::LongText => t.to_string(),
            ColumnKind::Integer => t
                .replace([',', '_'], "")
                .parse::<i64>()
                .with_context(|| format!("`{t}` is not a whole number"))?
                .to_string(),
            ColumnKind::Float => {
                let v: f64 = t
                    .replace(',', ".")
                    .parse()
                    .with_context(|| format!("`{t}` is not a number"))?;
                if !v.is_finite() {
                    bail!("`{t}` is not a number");
                }
                format_number(v)
            }
            ColumnKind::Rating => {
                let stars = t.chars().filter(|c| *c == '★').count();
                let v: f64 = if stars > 0 && t.chars().all(|c| c == '★') {
                    stars as f64
                } else {
                    t.replace(',', ".")
                        .parse()
                        .with_context(|| format!("`{t}` is not a rating from 0 to 5"))?
                };
                if !(0.0..=5.0).contains(&v) {
                    bail!("a rating is from 0 to 5 stars");
                }
                let v = (v * 2.0).round() / 2.0;
                if v == 0.0 {
                    return Ok(String::new());
                }
                format_number(v)
            }
            ColumnKind::Bool => match t.to_lowercase().as_str() {
                "true" | "yes" | "y" | "1" | "on" => "true".into(),
                "false" | "no" | "n" | "0" | "off" => "false".into(),
                _ => bail!("`{t}` is not yes or no"),
            },
            ColumnKind::Date => normalize_date(t)?,
            ColumnKind::Tags => {
                let mut out: Vec<String> = Vec::new();
                for p in t.split(',') {
                    let p = p.trim();
                    if !p.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(p)) {
                        out.push(p.to_string());
                    }
                }
                out.join(", ")
            }
            ColumnKind::Enumeration => match self
                .values
                .iter()
                .find(|v| v.trim().eq_ignore_ascii_case(t))
            {
                Some(v) => v.trim().to_string(),
                None => bail!("`{t}` is not one of: {}", self.values.join(", ")),
            },
            ColumnKind::Composite => bail!("a composite column is computed; it cannot be edited"),
        })
    }

    /// How a stored value reads: ratings as stars (`★★★½`), yes/no values as `true`/`false` (the
    /// caller translates), everything else as stored.
    pub fn display(&self, stored: &str) -> String {
        match self.kind {
            ColumnKind::Rating => {
                let v: f64 = stored.parse().unwrap_or(0.0);
                let full = v.floor() as usize;
                let mut s = "★".repeat(full.min(5));
                if v.fract() >= 0.5 {
                    s.push('½');
                }
                s
            }
            ColumnKind::LongText => stored.lines().next().unwrap_or("").to_string(),
            _ => stored.to_string(),
        }
    }

    /// Orders two stored values (a missing one is `None` and sorts last when ascending).
    pub fn compare(&self, a: Option<&str>, b: Option<&str>) -> Ordering {
        let a = a.filter(|s| !s.is_empty());
        let b = b.filter(|s| !s.is_empty());
        match (a, b) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(x), Some(y)) => {
                if self.kind.is_numeric() {
                    let (p, q) = (
                        x.parse::<f64>().unwrap_or(0.0),
                        y.parse::<f64>().unwrap_or(0.0),
                    );
                    p.partial_cmp(&q).unwrap_or(Ordering::Equal)
                } else if self.kind == ColumnKind::Bool {
                    // yes before no
                    (y == "true").cmp(&(x == "true"))
                } else if self.kind == ColumnKind::Enumeration {
                    // in the order the values are defined
                    let pos = |s: &str| self.values.iter().position(|v| v.trim() == s);
                    pos(x).cmp(&pos(y))
                } else {
                    x.to_lowercase().cmp(&y.to_lowercase())
                }
            }
        }
    }
}

fn normalize_date(t: &str) -> Result<String> {
    let t: String = if t.len() > 10 && t.as_bytes().get(10) == Some(&b'T') {
        t[..10].to_string()
    } else {
        t.to_string()
    };
    let parts: Vec<&str> = t.split('-').collect();
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let bad = || anyhow::anyhow!("`{t}` is not a date: write 2024, 2024-05 or 2024-05-17");
    match parts.as_slice() {
        [y] if digits(y, 4) => Ok(t),
        [y, m] if digits(y, 4) && digits(m, 2) => {
            if (1..=12).contains(&m.parse::<u32>()?) {
                Ok(t)
            } else {
                Err(bad())
            }
        }
        [y, m, d] if digits(y, 4) && digits(m, 2) && digits(d, 2) => {
            chrono::NaiveDate::parse_from_str(&t, "%Y-%m-%d").map_err(|_| bad())?;
            Ok(t)
        }
        _ => Err(bad()),
    }
}

/// Checks a whole set of columns: each one valid, labels unique, not too many.
pub fn validate_columns(defs: &[ColumnDef]) -> Result<()> {
    if defs.len() > MAX_COLUMNS {
        bail!("a library can have {MAX_COLUMNS} custom columns at most");
    }
    let mut labels = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for d in defs {
        d.validate()?;
        if !labels.insert(d.label.clone()) {
            bail!("the label `{}` is used twice", d.label);
        }
        if !names.insert(d.name.trim().to_lowercase()) {
            bail!("the name `{}` is used twice", d.name.trim());
        }
    }
    Ok(())
}

/// Renders a composite template for a book. Registered once by the program (the template language
/// lives in `bookx-sources`, which this crate cannot depend on).
pub type CompositeFn = fn(&str, &Book) -> String;

static COMPOSITE: OnceLock<CompositeFn> = OnceLock::new();

pub fn register_composite_renderer(f: CompositeFn) {
    let _ = COMPOSITE.set(f);
}

pub fn composite_renderer() -> Option<CompositeFn> {
    COMPOSITE.get().copied()
}

impl Book {
    /// The stored value of custom column `label` (empty when none).
    pub fn custom_value(&self, label: &str) -> &str {
        self.custom.get(label).map(String::as_str).unwrap_or("")
    }
}

impl Library {
    /// The custom columns of this library, in the order the user arranged them.
    pub fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    /// The column with this label.
    pub fn column(&self, label: &str) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.label == label)
    }

    /// Replaces the column definitions. Values of columns that are gone are removed from the books;
    /// a value that no longer fits its column (an enumeration value that was taken out) is kept
    /// as it is, so nothing is lost silently, but is not offered again. Composites are recomputed.
    pub fn set_columns(&mut self, defs: Vec<ColumnDef>) -> Result<()> {
        self.writable()?;
        let defs: Vec<ColumnDef> = defs
            .into_iter()
            .map(|mut d| {
                d.name = d.name.trim().to_string();
                d
            })
            .collect();
        validate_columns(&defs)?;
        if defs == self.columns {
            return Ok(());
        }
        // a label cannot change its type: the values would not mean the same
        for d in &defs {
            if let Some(old) = self.column(&d.label) {
                if old.kind != d.kind {
                    bail!(
                        "the type of `{}` cannot change; remove the column and add it again",
                        d.label
                    );
                }
            }
        }
        self.backend_save_columns(&defs)?;
        let gone: Vec<String> = self
            .columns
            .iter()
            .filter(|c| !defs.iter().any(|d| d.label == c.label))
            .map(|c| c.label.clone())
            .collect();
        for b in &mut self.books {
            for g in &gone {
                b.custom.remove(g);
            }
        }
        self.columns = defs;
        self.refresh_composites();
        self.save()
    }

    /// Sets custom values: `edits` are `(book id, [(label, new value)])`; an empty value removes
    /// the value. One undo step. Returns the number of books that changed. Nothing is changed when
    /// any value is invalid.
    pub fn edit_custom(
        &mut self,
        label: String,
        edits: &[(u64, Vec<(String, String)>)],
    ) -> Result<usize> {
        self.writable()?;
        // check everything first so a bad value changes nothing
        let mut plan: Vec<(u64, BTreeMap<String, String>)> = Vec::new();
        for (id, vals) in edits {
            let Some(b) = self.get(*id) else { continue };
            let mut next = b.custom.clone();
            for (l, v) in vals {
                let Some(def) = self.column(l) else {
                    bail!("there is no custom column `#{l}`");
                };
                let n = def
                    .normalize(v)
                    .with_context(|| format!("column `{}`", def.name))?;
                if n.is_empty() {
                    next.remove(l);
                } else {
                    next.insert(l.clone(), n);
                }
            }
            if next != b.custom {
                plan.push((*id, next));
            }
        }
        let mut before: Vec<(u64, BTreeMap<String, String>)> = Vec::new();
        let ids: Vec<u64> = plan.iter().map(|(i, _)| *i).collect();
        for (id, next) in plan {
            if let Some(b) = self.get_mut(id) {
                before.push((id, std::mem::replace(&mut b.custom, next)));
            }
        }
        if before.is_empty() {
            return Ok(0);
        }
        self.refresh_composites_of(&ids);
        let n = before.len();
        self.log(label, OpKind::SetCustom { before });
        self.save()?;
        Ok(n)
    }

    /// Sets one value of one book (`""` removes it).
    pub fn set_custom(&mut self, id: u64, label: &str, value: &str) -> Result<()> {
        let title = self
            .get(id)
            .map(|b| b.meta.title.clone())
            .context("no such book")?;
        self.edit_custom(
            format!("Edited \"{title}\""),
            &[(id, vec![(label.to_string(), value.to_string())])],
        )?;
        Ok(())
    }

    pub(crate) fn undo_custom(&mut self, before: &[(u64, BTreeMap<String, String>)]) -> Result<()> {
        for (id, map) in before {
            if let Some(b) = self.get_mut(*id) {
                b.custom = map.clone();
            }
        }
        let ids: Vec<u64> = before.iter().map(|(i, _)| *i).collect();
        self.refresh_composites_of(&ids);
        self.save()
    }

    /// Recomputes the composite columns of every book. Returns how many books changed.
    pub fn refresh_composites(&mut self) -> usize {
        let ids: Vec<u64> = self.books.iter().map(|b| b.id).collect();
        self.refresh_composites_of(&ids)
    }

    /// Recomputes the composite columns of the given books. Returns how many books changed.
    pub fn refresh_composites_of(&mut self, ids: &[u64]) -> usize {
        let comps: Vec<(String, String)> = self
            .columns
            .iter()
            .filter(|c| c.kind == ColumnKind::Composite)
            .map(|c| (c.label.clone(), c.template.clone()))
            .collect();
        let Some(render) = composite_renderer() else {
            return 0;
        };
        if comps.is_empty() {
            return 0;
        }
        let want: std::collections::HashSet<u64> = ids.iter().copied().collect();
        let mut changed = 0;
        // composites may refer to each other: two passes settle chains one level deep
        for _ in 0..2 {
            for b in self.books.iter_mut().filter(|b| want.contains(&b.id)) {
                let mut any = false;
                for (label, tpl) in &comps {
                    let v: String = render(tpl, b)
                        .trim()
                        .chars()
                        .take(MAX_VALUE_CHARS)
                        .collect();
                    let old = b.custom.get(label).cloned().unwrap_or_default();
                    if v != old {
                        if v.is_empty() {
                            b.custom.remove(label);
                        } else {
                            b.custom.insert(label.clone(), v);
                        }
                        any = true;
                    }
                }
                if any {
                    changed += 1;
                }
            }
        }
        changed
    }

    /// Sorts `ids` by the values of a custom column. Books without a value come last (first when
    /// descending is requested the whole order is reversed, as the other sorts do).
    pub fn sort_by_custom(&self, ids: &mut [u64], label: &str, ascending: bool) {
        let Some(def) = self.column(label) else {
            return;
        };
        let by_id: std::collections::HashMap<u64, &Book> =
            self.books.iter().map(|b| (b.id, b)).collect();
        ids.sort_by(|a, b| {
            let (x, y) = (
                by_id.get(a).map(|b| b.custom_value(label)),
                by_id.get(b).map(|b| b.custom_value(label)),
            );
            let o = def.compare(x, y);
            if ascending { o } else { o.reverse() }.then_with(|| a.cmp(b))
        });
    }

    /// The values a custom column has in the library with how many books carry each (tags-like
    /// columns count every item), most used first.
    pub fn custom_values(&self, label: &str) -> Vec<(String, usize)> {
        let Some(def) = self.column(label) else {
            return Vec::new();
        };
        let mut map: BTreeMap<String, usize> = BTreeMap::new();
        for b in &self.books {
            let v = b.custom_value(label);
            if v.is_empty() {
                continue;
            }
            if def.kind == ColumnKind::Tags {
                for t in v.split(',') {
                    *map.entry(t.trim().to_string()).or_default() += 1;
                }
            } else {
                *map.entry(v.to_string()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, usize)> = map.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(kind: ColumnKind) -> ColumnDef {
        ColumnDef {
            label: "x".into(),
            name: "X".into(),
            kind,
            ..Default::default()
        }
    }

    #[test]
    fn labels_have_a_fixed_shape_and_do_not_shadow_fields() {
        assert!(is_label_shape("mood_2"));
        assert!(!is_label_shape("2mood"));
        assert!(!is_label_shape("a b"));
        assert!(!is_label_shape(""));
        let mut d = def(ColumnKind::Text);
        assert!(d.validate().is_ok());
        d.label = "title".into();
        assert!(d.validate().is_err());
        d.label = "Mood".into();
        assert!(d.validate().is_err(), "labels are lower case");
        assert_eq!(label_from_name("Reading Mood!"), "reading_mood");
        assert_eq!(label_from_name("2nd read"), "c_2nd_read");
    }

    #[test]
    fn values_are_normalised_by_type() {
        let n = |k, s: &str| def(k).normalize(s);
        assert_eq!(n(ColumnKind::Integer, " 1,200 ").unwrap(), "1200");
        assert!(n(ColumnKind::Integer, "1.5").is_err());
        assert_eq!(n(ColumnKind::Float, "3,50").unwrap(), "3.5");
        assert_eq!(n(ColumnKind::Float, "4").unwrap(), "4");
        assert!(n(ColumnKind::Float, "nan").is_err());
        assert_eq!(n(ColumnKind::Rating, "3.6").unwrap(), "3.5");
        assert_eq!(n(ColumnKind::Rating, "★★★★").unwrap(), "4");
        assert_eq!(n(ColumnKind::Rating, "0").unwrap(), "");
        assert!(n(ColumnKind::Rating, "6").is_err());
        assert_eq!(n(ColumnKind::Bool, "Yes").unwrap(), "true");
        assert_eq!(n(ColumnKind::Bool, "0").unwrap(), "false");
        assert!(n(ColumnKind::Bool, "maybe").is_err());
        assert_eq!(n(ColumnKind::Date, "2024-05").unwrap(), "2024-05");
        assert_eq!(
            n(ColumnKind::Date, "2024-05-17T10:00:00Z").unwrap(),
            "2024-05-17"
        );
        assert!(n(ColumnKind::Date, "2024-13").is_err());
        assert!(n(ColumnKind::Date, "2024-02-30").is_err());
        assert_eq!(n(ColumnKind::Tags, "b, a ,B,,c").unwrap(), "b, a, c");
        assert_eq!(n(ColumnKind::Text, "").unwrap(), "");
        assert!(n(ColumnKind::Composite, "x").is_err());
        let long = "x".repeat(MAX_VALUE_CHARS + 1);
        assert!(n(ColumnKind::LongText, &long).is_err());
    }

    #[test]
    fn enumerations_accept_only_their_values() {
        let mut d = def(ColumnKind::Enumeration);
        assert!(d.validate().is_err(), "needs values");
        d.values = vec!["Low".into(), "High".into()];
        assert!(d.validate().is_ok());
        assert_eq!(d.normalize("high").unwrap(), "High");
        assert!(d.normalize("mid").is_err());
        d.values.push("low".into());
        assert!(d.validate().is_err(), "repeats differ only by case");
    }

    #[test]
    fn columns_are_checked_together() {
        let a = def(ColumnKind::Text);
        let mut b = def(ColumnKind::Integer);
        assert!(
            validate_columns(&[a.clone(), b.clone()]).is_err(),
            "same label"
        );
        b.label = "y".into();
        assert!(
            validate_columns(&[a.clone(), b.clone()]).is_err(),
            "same name"
        );
        b.name = "Y".into();
        assert!(validate_columns(&[a, b]).is_ok());
    }

    #[test]
    fn values_sort_by_type_with_empty_last() {
        let n = def(ColumnKind::Integer);
        assert_eq!(n.compare(Some("9"), Some("10")), Ordering::Less);
        assert_eq!(n.compare(None, Some("1")), Ordering::Greater);
        assert_eq!(n.compare(Some(""), None), Ordering::Equal);
        let t = def(ColumnKind::Text);
        assert_eq!(t.compare(Some("apple"), Some("Banana")), Ordering::Less);
        let b = def(ColumnKind::Bool);
        assert_eq!(b.compare(Some("true"), Some("false")), Ordering::Less);
        let mut e = def(ColumnKind::Enumeration);
        e.values = vec!["Low".into(), "Mid".into(), "High".into()];
        assert_eq!(e.compare(Some("High"), Some("Low")), Ordering::Greater);
    }

    #[test]
    fn display_shows_stars() {
        assert_eq!(def(ColumnKind::Rating).display("3.5"), "★★★½");
        assert_eq!(def(ColumnKind::Rating).display("5"), "★★★★★");
        assert_eq!(def(ColumnKind::LongText).display("a\nb"), "a");
    }
}
