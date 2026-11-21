//! Volumes of a series: which book comes after this one, and which series a file belongs to when
//! its folder says so. Pure functions, no files and no window.

use std::cmp::Ordering;

/// What the neighbour search needs to know about a comic of the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub id: u64,
    pub series: String,
    pub index: f64,
    pub title: String,
    /// Folder of the file the book was imported from or is linked to; empty when unknown.
    pub folder: String,
    /// File name (or the title when there is none), for ordering inside a folder.
    pub file: String,
}

fn natural(a: &str, b: &str) -> Ordering {
    // digits compare by value, the rest case-insensitively
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut xs = String::new();
                while let Some(c) = ai.next_if(|c| c.is_ascii_digit()) {
                    xs.push(c);
                }
                let mut ys = String::new();
                while let Some(c) = bi.next_if(|c| c.is_ascii_digit()) {
                    ys.push(c);
                }
                let (xt, yt) = (xs.trim_start_matches('0'), ys.trim_start_matches('0'));
                let ord = xt.len().cmp(&yt.len()).then_with(|| xt.cmp(yt));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// The volume that comes before (`forward == false`) or after the book `current`.
///
/// The series wins: the other books of the same series, by number and then title. A book with no
/// series, or alone in it, falls back to the files of its folder in natural order.
pub fn neighbour(all: &[Volume], current: u64, forward: bool) -> Option<u64> {
    let me = all.iter().find(|v| v.id == current)?;
    let pick = |group: Vec<&Volume>| -> Option<u64> {
        let at = group.iter().position(|v| v.id == current)?;
        if group.len() < 2 {
            return None;
        }
        let i = if forward {
            at.checked_add(1).filter(|i| *i < group.len())?
        } else {
            at.checked_sub(1)?
        };
        Some(group[i].id)
    };
    if !me.series.is_empty() {
        let mut group: Vec<&Volume> = all
            .iter()
            .filter(|v| v.series.eq_ignore_ascii_case(&me.series))
            .collect();
        group.sort_by(|a, b| {
            a.index
                .partial_cmp(&b.index)
                .unwrap_or(Ordering::Equal)
                .then_with(|| natural(&a.title, &b.title))
                .then(a.id.cmp(&b.id))
        });
        if group.len() > 1 {
            return pick(group);
        }
    }
    if me.folder.is_empty() {
        return None;
    }
    let mut group: Vec<&Volume> = all.iter().filter(|v| v.folder == me.folder).collect();
    group.sort_by(|a, b| natural(&a.file, &b.file).then(a.id.cmp(&b.id)));
    pick(group)
}

/// Names of folders that say where a file lives, not which series it is in.
const GENERIC: &[&str] = &[
    "comics",
    "comic",
    "manga",
    "books",
    "ebooks",
    "downloads",
    "download",
    "documents",
    "desktop",
    "home",
    "tmp",
    "temp",
    "media",
    "mnt",
    "data",
    "library",
    "cbz",
    "cbr",
    "scans",
    "new",
    "inbox",
    "import",
    "imports",
    "incoming",
    "unsorted",
    "misc",
];

/// A volume number in a file name: `v03`, `vol. 3`, `volume 3`, `#12`, `ch 12`, `issue 7`, or a
/// number at the end of the name (`One Piece 104`).
pub fn volume_number(file_stem: &str) -> Option<f64> {
    let s = file_stem.to_lowercase();
    let b = s.as_bytes();
    let number_at = |mut i: usize| -> Option<f64> {
        while i < b.len() && matches!(b[i], b' ' | b'.' | b'_' | b'-' | b'#') {
            i += 1;
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || (b[i] == b'.' && i > start)) {
            i += 1;
        }
        s[start..i].trim_end_matches('.').parse().ok()
    };
    for key in ["volume", "vol", "issue", "chapter", "ch", "no", "v", "#"] {
        let mut from = 0;
        while let Some(p) = s[from..].find(key) {
            let at = from + p;
            let before_ok = at == 0 || !b[at - 1].is_ascii_alphanumeric() || key == "#";
            let after = at + key.len();
            let next_ok = after >= b.len() || !b[after].is_ascii_alphabetic();
            if before_ok && next_ok {
                if let Some(n) = number_at(after) {
                    return Some(n);
                }
            }
            from = at + key.len();
        }
    }
    let digits: String = s
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let rest = &s[..s.len() - digits.len()];
    (!digits.is_empty() && !rest.trim().is_empty() && digits.len() <= 4)
        .then(|| digits.parse().ok())
        .flatten()
}

/// The series a file belongs to when its folder says so: the name of the folder that holds it
/// (`Berserk/Volume 03.cbz` → `Berserk`), unless that is a generic place such as `Downloads`.
/// The number comes from the file name.
pub fn series_from_path(path: &str) -> Option<(String, Option<f64>)> {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let (file, dirs) = parts.split_last()?;
    let folder = dirs.last()?;
    let lower = folder.to_lowercase();
    let name = folder.trim();
    if name.chars().count() < 2
        || GENERIC.contains(&lower.as_str())
        || name.starts_with('.')
        || (name.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    let stem = file.rsplit_once('.').map_or(*file, |(s, _)| s);
    Some((name.to_string(), volume_number(stem)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(id: u64, series: &str, index: f64, folder: &str, file: &str) -> Volume {
        Volume {
            id,
            series: series.into(),
            index,
            title: file.into(),
            folder: folder.into(),
            file: file.into(),
        }
    }

    #[test]
    fn the_next_volume_of_a_series_follows_the_number_not_the_import_order() {
        let all = vec![
            vol(1, "Berserk", 3.0, "", "c"),
            vol(2, "Berserk", 1.0, "", "a"),
            vol(3, "Berserk", 2.0, "", "b"),
            vol(4, "Other", 1.0, "", "z"),
        ];
        assert_eq!(neighbour(&all, 2, true), Some(3));
        assert_eq!(neighbour(&all, 3, true), Some(1));
        assert_eq!(
            neighbour(&all, 1, true),
            None,
            "the last volume has no next"
        );
        assert_eq!(neighbour(&all, 1, false), Some(3));
        assert_eq!(neighbour(&all, 2, false), None);
        assert_eq!(neighbour(&all, 4, true), None);
        assert_eq!(neighbour(&all, 99, true), None);
    }

    #[test]
    fn series_names_match_without_regard_to_case() {
        let all = vec![
            vol(1, "berserk", 1.0, "", "a"),
            vol(2, "BERSERK", 2.0, "", "b"),
        ];
        assert_eq!(neighbour(&all, 1, true), Some(2));
    }

    #[test]
    fn without_a_series_the_folder_decides_in_natural_order() {
        let all = vec![
            vol(1, "", 0.0, "/c/x", "Issue 10.cbz"),
            vol(2, "", 0.0, "/c/x", "Issue 9.cbz"),
            vol(3, "", 0.0, "/c/y", "Issue 11.cbz"),
            vol(4, "", 0.0, "", "Issue 12.cbz"),
        ];
        assert_eq!(neighbour(&all, 2, true), Some(1), "9 before 10");
        assert_eq!(neighbour(&all, 1, true), None);
        assert_eq!(neighbour(&all, 3, true), None, "another folder");
        assert_eq!(neighbour(&all, 4, true), None, "no folder known");
    }

    #[test]
    fn a_lone_book_of_a_series_falls_back_to_its_folder() {
        let all = vec![
            vol(1, "Solo", 1.0, "/c/x", "a.cbz"),
            vol(2, "", 0.0, "/c/x", "b.cbz"),
        ];
        assert_eq!(neighbour(&all, 1, true), Some(2));
    }

    #[test]
    fn numbers_are_found_in_common_file_names() {
        assert_eq!(volume_number("Berserk v03"), Some(3.0));
        assert_eq!(volume_number("Berserk Vol. 12"), Some(12.0));
        assert_eq!(volume_number("Berserk - Volume 7 (Digital)"), Some(7.0));
        assert_eq!(volume_number("Saga #045"), Some(45.0));
        assert_eq!(volume_number("One Piece 104"), Some(104.0));
        assert_eq!(volume_number("Issue 7.5"), Some(7.5));
        assert_eq!(volume_number("Chapter_12"), Some(12.0));
        assert_eq!(volume_number("Berserk"), None);
        assert_eq!(volume_number("2001"), None, "a bare number is a title");
        assert_eq!(volume_number("Dive vol"), None);
    }

    #[test]
    fn the_folder_names_the_series_unless_it_is_a_generic_place() {
        assert_eq!(
            series_from_path("/home/me/Comics/Berserk/Berserk v03.cbz"),
            Some(("Berserk".into(), Some(3.0)))
        );
        assert_eq!(
            series_from_path("Saga/Saga 12.cbr"),
            Some(("Saga".into(), Some(12.0)))
        );
        assert_eq!(series_from_path("/home/me/Downloads/thing.cbz"), None);
        assert_eq!(series_from_path("/Manga/x.cbz"), None);
        assert_eq!(series_from_path("thing.cbz"), None);
        assert_eq!(series_from_path("/2020/thing.cbz"), None);
        assert_eq!(series_from_path(""), None);
        assert_eq!(
            series_from_path("C:\\Comics\\Saga\\Saga 2.cbz"),
            Some(("Saga".into(), Some(2.0)))
        );
    }
}
