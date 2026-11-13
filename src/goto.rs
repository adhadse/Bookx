//! The go-to dialog (R24, R42, R12): what the user typed (a page, a printed page label, a
//! percent, a location, a chapter) and where in the book it leads, without GTK.

use crate::i18n::tr;
use bookx_text::pages::PageMap;

/// Characters per "location" (as many e-readers count them).
pub const CHARS_PER_LOCATION: f64 = 128.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A page of the book's numbering, or a printed label when the book has a page-list.
    Page(String),
    Percent(f64),
    Location(usize),
    /// 1-based chapter of the table of contents.
    Chapter(usize),
}

/// What the book can answer with.
pub struct Book<'a> {
    /// Pages of the book's layout.
    pub layout_pages: usize,
    /// Text weight of every spine item (file sizes).
    pub weights: &'a [f64],
    /// Entries in the table of contents.
    pub chapters: usize,
    /// The EPUB page-list, if the book has one.
    pub page_list: Option<&'a PageMap>,
}

/// Where to go.
#[derive(Debug, Clone, PartialEq)]
pub enum Dest {
    /// A fraction of the whole book.
    Fraction(f64),
    /// A place in a spine item.
    Spine(usize, f64),
    /// The n-th (0-based) chapter of the table of contents.
    Chapter(usize),
}

/// Reads the text of the go-to field. "45" is a page; "45%", "loc 1200" / "@1200",
/// "ch 3" / "chapter 3" are the others.
pub fn parse(input: &str) -> Result<Target, String> {
    let t = input.trim();
    if t.is_empty() {
        return Err(tr(
            "Type a page, a percent, a location (loc 500) or a chapter (ch 3).",
        ));
    }
    let lower = t.to_lowercase();
    if let Some(p) = lower.strip_suffix('%') {
        return p
            .trim()
            .replace(',', ".")
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
            .map(Target::Percent)
            .ok_or_else(|| tr("A percent is a number from 0 to 100."));
    }
    for prefix in ["location ", "loc ", "loc. ", "@"] {
        if let Some(n) = lower.strip_prefix(prefix) {
            return n
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Target::Location)
                .ok_or_else(|| tr("A location is a whole number from 1."));
        }
    }
    for prefix in ["chapter ", "chap ", "ch ", "ch. ", "#"] {
        if let Some(n) = lower.strip_prefix(prefix) {
            return n
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|n| *n >= 1)
                .map(Target::Chapter)
                .ok_or_else(|| tr("A chapter is a whole number from 1."));
        }
    }
    for prefix in ["page ", "p. ", "p "] {
        if lower.starts_with(prefix) {
            return Ok(Target::Page(t[prefix.len()..].trim().to_string()));
        }
    }
    Ok(Target::Page(t.to_string()))
}

fn total_weight(book: &Book) -> f64 {
    book.weights.iter().sum()
}

/// How many locations the book has.
pub fn locations(book: &Book) -> usize {
    ((total_weight(book) / CHARS_PER_LOCATION).ceil() as usize).max(1)
}

/// The location of a fraction of the book (1-based).
pub fn location_at(book: &Book, fraction: f64) -> usize {
    ((fraction.clamp(0.0, 1.0) * total_weight(book) / CHARS_PER_LOCATION).floor() as usize + 1)
        .min(locations(book))
}

/// Where a target leads.
pub fn resolve(target: &Target, book: &Book) -> Result<Dest, String> {
    match target {
        Target::Percent(p) => Ok(Dest::Fraction(p / 100.0)),
        Target::Location(n) => {
            let max = locations(book);
            if *n > max {
                return Err(format!("This book has {max} locations."));
            }
            let total = total_weight(book);
            Ok(Dest::Fraction(
                (((*n - 1) as f64) * CHARS_PER_LOCATION / total.max(1.0)).clamp(0.0, 1.0),
            ))
        }
        Target::Chapter(n) => {
            if *n > book.chapters || book.chapters == 0 {
                return Err(format!("This book has {} chapters.", book.chapters));
            }
            Ok(Dest::Chapter(n - 1))
        }
        Target::Page(label) => {
            if let Some(map) = book.page_list {
                if let Some((spine, off)) = map.position_of_label(label) {
                    let w = book.weights.get(spine).copied().unwrap_or(1.0).max(1.0);
                    return Ok(Dest::Spine(spine, (off / w).clamp(0.0, 1.0)));
                }
                return Err(format!("There is no page “{label}” in this book."));
            }
            let n: usize = label
                .parse()
                .map_err(|_| tr("A page is a number, for example 45."))?;
            if n == 0 || n > book.layout_pages.max(1) {
                return Err(format!("This book has {} pages.", book.layout_pages));
            }
            let pages = book.layout_pages.max(1);
            Ok(Dest::Fraction(if pages == 1 {
                0.0
            } else {
                (n - 1) as f64 / pages as f64
            }))
        }
    }
}

/// The line under the field: what the book has.
pub fn hint(book: &Book) -> String {
    let pages = match book.page_list {
        Some(m) => format!("{} printed pages", m.total_pages()),
        None => format!("{} pages", book.layout_pages),
    };
    format!(
        "{pages} · {} locations · {} chapters",
        locations(book),
        book.chapters
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_text::pages::{PageMap, PageSource, PageTarget, SpineItem};

    fn book<'a>(w: &'a [f64], map: Option<&'a PageMap>) -> Book<'a> {
        Book {
            layout_pages: 100,
            weights: w,
            chapters: 12,
            page_list: map,
        }
    }

    #[test]
    fn typed_text_is_understood() {
        assert_eq!(parse("45"), Ok(Target::Page("45".into())));
        assert_eq!(parse(" xiv "), Ok(Target::Page("xiv".into())));
        assert_eq!(parse("page 7"), Ok(Target::Page("7".into())));
        assert_eq!(parse("p. 7"), Ok(Target::Page("7".into())));
        assert_eq!(parse("37%"), Ok(Target::Percent(37.0)));
        assert_eq!(parse("12,5 %"), Ok(Target::Percent(12.5)));
        assert_eq!(parse("loc 1200"), Ok(Target::Location(1200)));
        assert_eq!(parse("@50"), Ok(Target::Location(50)));
        assert_eq!(parse("Location 3"), Ok(Target::Location(3)));
        assert_eq!(parse("ch 3"), Ok(Target::Chapter(3)));
        assert_eq!(parse("Chapter 10"), Ok(Target::Chapter(10)));
        assert!(parse("").is_err());
        assert!(parse("101%").is_err());
        assert!(parse("abc%").is_err());
        assert!(parse("loc 0").is_err());
        assert!(parse("ch x").is_err());
    }

    #[test]
    fn percent_and_chapter_resolve() {
        let w = [1000.0; 4];
        let b = book(&w, None);
        assert_eq!(
            resolve(&Target::Percent(25.0), &b),
            Ok(Dest::Fraction(0.25))
        );
        assert_eq!(resolve(&Target::Chapter(3), &b), Ok(Dest::Chapter(2)));
        assert!(resolve(&Target::Chapter(13), &b)
            .unwrap_err()
            .contains("12 chapters"));
    }

    #[test]
    fn locations_use_the_text_size() {
        let w = [12_800.0];
        let b = book(&w, None);
        assert_eq!(locations(&b), 100);
        assert_eq!(resolve(&Target::Location(1), &b), Ok(Dest::Fraction(0.0)));
        assert_eq!(resolve(&Target::Location(51), &b), Ok(Dest::Fraction(0.5)));
        assert!(resolve(&Target::Location(101), &b).is_err());
        assert_eq!(location_at(&b, 0.0), 1);
        assert_eq!(location_at(&b, 0.5), 51);
        assert_eq!(location_at(&b, 1.0), 100);
    }

    #[test]
    fn layout_pages_become_fractions() {
        let w = [1000.0];
        let b = book(&w, None);
        assert_eq!(
            resolve(&Target::Page("1".into()), &b),
            Ok(Dest::Fraction(0.0))
        );
        assert_eq!(
            resolve(&Target::Page("51".into()), &b),
            Ok(Dest::Fraction(0.5))
        );
        assert!(resolve(&Target::Page("101".into()), &b)
            .unwrap_err()
            .contains("100 pages"));
        assert!(resolve(&Target::Page("0".into()), &b).is_err());
        assert!(resolve(&Target::Page("iv".into()), &b).is_err());
    }

    #[test]
    fn printed_page_labels_use_the_page_list() {
        let items = vec![
            SpineItem::new(1000.0).with_targets(vec![
                PageTarget::new("i", 0.0),
                PageTarget::new("ii", 500.0),
            ]),
            SpineItem::new(2000.0).with_targets(vec![
                PageTarget::new("1", 0.0),
                PageTarget::new("2", 1000.0),
            ]),
        ];
        let map = PageMap::new(&items, 0.0, PageSource::PageList);
        let w = [1000.0, 2000.0];
        let b = book(&w, Some(&map));
        assert_eq!(
            resolve(&Target::Page("ii".into()), &b),
            Ok(Dest::Spine(0, 0.5))
        );
        assert_eq!(
            resolve(&Target::Page("2".into()), &b),
            Ok(Dest::Spine(1, 0.5))
        );
        assert!(resolve(&Target::Page("99".into()), &b)
            .unwrap_err()
            .contains("no page"));
        assert!(hint(&b).starts_with("4 printed pages"));
    }

    #[test]
    fn the_hint_names_what_the_book_has() {
        let w = [12_800.0];
        assert_eq!(
            hint(&book(&w, None)),
            "100 pages · 100 locations · 12 chapters"
        );
    }
}
