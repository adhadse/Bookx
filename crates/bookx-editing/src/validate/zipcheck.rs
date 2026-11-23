//! Zip level and file name rules.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use zip::CompressionMethod;

use super::{Cx, Message, rule};

fn message(id: &'static str, file: &str, text: String) -> Message {
    Message {
        id: id.to_string(),
        severity: rule(id)
            .map(|r| r.severity)
            .unwrap_or(super::Severity::Error),
        file: file.to_string(),
        line: None,
        col: None,
        text,
    }
}

/// Entry names straight from the central directory (the zip crate keeps only one entry per name).
fn central_directory_names(data: &[u8]) -> Vec<String> {
    let le16 = |o: usize| {
        data.get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
    };
    let le32 = |o: usize| {
        data.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let Some(eocd) = (0..data.len().saturating_sub(21))
        .rev()
        .take(70_000)
        .find(|&i| data[i..i + 4] == [0x50, 0x4B, 0x05, 0x06])
    else {
        return Vec::new();
    };
    let (Some(count), Some(mut pos)) = (le16(eocd + 10), le32(eocd + 16)) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for _ in 0..count {
        if data.get(pos..pos + 4) != Some(&[0x50, 0x4B, 0x01, 0x02]) {
            break;
        }
        let (Some(n), Some(e), Some(c)) = (le16(pos + 28), le16(pos + 30), le16(pos + 32)) else {
            break;
        };
        let Some(raw) = data.get(pos + 46..pos + 46 + n) else {
            break;
        };
        names.push(String::from_utf8_lossy(raw).to_string());
        pos += 46 + n + e + c;
    }
    names
}

/// Checks that need the raw zip: entry order, compression, extra field, duplicates, directory entries.
/// Returns the messages and whether the archive could be read at all.
pub(super) fn scan_zip(data: &[u8]) -> (Vec<Message>, bool) {
    let mut out = Vec::new();
    let mut zip = match zip::ZipArchive::new(Cursor::new(data)) {
        Ok(z) => z,
        Err(e) => {
            out.push(message(
                "PKG-004",
                "",
                format!("the file is not a readable zip archive: {e}"),
            ));
            return (out, false);
        }
    };
    let mut dirs: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index_raw(i) else {
            out.push(message(
                "PKG-004",
                "",
                format!("zip entry {i} cannot be read"),
            ));
            continue;
        };
        let name = String::from_utf8_lossy(f.name_raw()).to_string();
        if f.is_dir() {
            dirs.push(name.clone());
            names.push(name);
            continue;
        }
        if name == "mimetype" {
            if i != 0 {
                out.push(message(
                    "PKG-006",
                    &name,
                    format!(
                        "the mimetype entry is entry {} of the archive, it must be the first",
                        i + 1
                    ),
                ));
            }
            if f.compression() != CompressionMethod::Stored {
                out.push(message(
                    "PKG-022",
                    &name,
                    "the mimetype entry is compressed, it must be stored".into(),
                ));
            }
            if f.extra_data_fields().next().is_some()
                || f.extra_data().is_some_and(|e| !e.is_empty())
            {
                out.push(message(
                    "PKG-005",
                    &name,
                    "the mimetype entry has an extra field".into(),
                ));
            }
        }
        names.push(name);
    }
    let mut central: HashSet<String> = HashSet::new();
    for name in central_directory_names(data) {
        if !name.ends_with('/') && !central.insert(name.clone()) {
            out.push(message(
                "OPF-060",
                &name,
                format!("duplicate entry \"{name}\" in the zip archive"),
            ));
        }
    }
    for d in dirs {
        let prefix = d.clone();
        if !names.iter().any(|n| n != &d && n.starts_with(&prefix)) {
            out.push(message(
                "PKG-014",
                &d,
                format!("the directory entry \"{d}\" is empty"),
            ));
        }
    }
    (out, true)
}

/// `mimetype` content.
pub(super) fn check_mimetype_content(cx: &mut Cx) {
    match cx.get("mimetype") {
        None => cx.emit("PKG-006", "mimetype", None, "the mimetype entry is missing"),
        Some(b"application/epub+zip") => {}
        Some(other) => {
            let shown: String = String::from_utf8_lossy(other).chars().take(40).collect();
            cx.emit(
                "PKG-007",
                "mimetype",
                None,
                format!("the mimetype entry contains {shown:?}, expected exactly \"application/epub+zip\""),
            );
        }
    }
}

/// File name rules for every entry.
pub(super) fn check_names(cx: &mut Cx) {
    let Some(archive) = cx.archive else { return };
    let mut lower: HashMap<String, &str> = HashMap::new();
    for name in archive.names() {
        if name.chars().any(|c| {
            c.is_control()
                || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\\')
                || ('\u{E000}'..='\u{F8FF}').contains(&c)
                || c == '\u{FFFD}'
        }) {
            cx.emit(
                "PKG-009",
                name,
                None,
                format!(
                    "the file name \"{}\" contains characters that are not allowed",
                    name.escape_debug()
                ),
            );
        }
        if name
            .split('/')
            .any(|seg| seg.ends_with('.') && seg != "." && seg != "..")
        {
            cx.emit(
                "PKG-011",
                name,
                None,
                format!("a segment of the file name \"{name}\" ends with a period"),
            );
        }
        if name.contains(' ') {
            cx.emit(
                "PKG-010",
                name,
                None,
                format!("the file name \"{name}\" contains spaces"),
            );
        }
        if !name.is_ascii() {
            cx.emit(
                "PKG-012",
                name,
                None,
                format!("the file name \"{name}\" contains non-ASCII characters"),
            );
        }
        if name.len() > 255 {
            cx.emit(
                "PKG-028",
                name,
                None,
                format!("the path is {} bytes long", name.len()),
            );
        }
        if let Some(prev) = lower.insert(name.to_lowercase(), name) {
            if prev != name {
                cx.emit(
                    "PKG-024",
                    name,
                    None,
                    format!("\"{name}\" and \"{prev}\" differ only by case"),
                );
            }
        }
    }
}
