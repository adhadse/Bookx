//! Small string/path helpers shared by the whole core.

use roxmltree::{Document, ParsingOptions};

pub fn parse_xml(text: &str) -> anyhow::Result<Document<'_>> {
    let text = text.trim_start_matches('\u{feff}');
    let opts = ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    Ok(Document::parse_with_options(text, opts)?)
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Normalises `a/b/../c` style paths (no leading slash in the result).
pub fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn extension(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        normalize_path(name)
    } else {
        normalize_path(&format!("{dir}/{name}"))
    }
}

/// Resolves `href` (possibly with `#fragment`) relative to the directory `base_dir`
/// inside the archive. Returns the archive path and the fragment, if any.
pub fn resolve_href(base_dir: &str, href: &str) -> (String, Option<String>) {
    let (path, frag) = match href.split_once('#') {
        Some((p, f)) => (p, Some(percent_decode(f))),
        None => (href, None),
    };
    let path = path.split('?').next().unwrap_or("");
    let path = percent_decode(path);
    let full = if path.is_empty() {
        String::new()
    } else if let Some(stripped) = path.strip_prefix('/') {
        normalize_path(stripped)
    } else {
        join_path(base_dir, &path)
    };
    (full, frag.filter(|f| !f.is_empty()))
}

pub fn is_external(href: &str) -> bool {
    let h = href.trim().to_ascii_lowercase();
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("mailto:")
        || h.starts_with("ftp://")
        || h.starts_with("data:")
}

/// Builds the href that, when written inside the file `from_file`, points at `to`.
pub fn relative_href(from_file: &str, to: &str) -> String {
    let (to_path, frag) = match to.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (to, None),
    };
    let from_dir: Vec<&str> = parent_dir(from_file)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let to_parts: Vec<&str> = to_path.split('/').filter(|s| !s.is_empty()).collect();
    let mut common = 0;
    while common < from_dir.len()
        && common + 1 < to_parts.len()
        && from_dir[common] == to_parts[common]
    {
        common += 1;
    }
    let mut out = String::new();
    for _ in common..from_dir.len() {
        out.push_str("../");
    }
    out.push_str(&to_parts[common..].join("/"));
    let mut out = percent_encode_path(&out);
    if let Some(f) = frag {
        out.push('#');
        out.push_str(f);
    }
    out
}

pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

pub fn media_type_for(path: &str) -> &'static str {
    match extension(path).as_str() {
        "xhtml" | "xht" => "application/xhtml+xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ncx" => "application/x-dtbncx+xml",
        "opf" => "application/oebps-package+xml",
        "js" => "text/javascript",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "xml" => "application/xml",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

pub fn is_image_path(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "svg" | "webp" | "bmp"
    )
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// "The Rust Book" -> "Rust Book, The"
pub fn title_sort(title: &str) -> String {
    let t = title.trim();
    for art in ["The ", "A ", "An "] {
        if t.len() > art.len()
            && t.get(..art.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(art))
        {
            return format!("{}, {}", &t[art.len()..], &t[..art.len() - 1]);
        }
    }
    t.to_string()
}

/// "Ada Lovelace" -> "Lovelace, Ada"
pub fn author_sort(author: &str) -> String {
    let a = author.trim();
    if a.contains(',') {
        return a.to_string();
    }
    let parts: Vec<&str> = a.split_whitespace().collect();
    match parts.len() {
        0 | 1 => a.to_string(),
        _ => format!(
            "{}, {}",
            parts[parts.len() - 1],
            parts[..parts.len() - 1].join(" ")
        ),
    }
}

pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let mut s: String = cleaned.chars().take(80).collect();
    if s.is_empty() {
        s = "Unknown".into();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves() {
        assert_eq!(
            resolve_href("OEBPS/text", "../images/a%20b.png#x"),
            ("OEBPS/images/a b.png".into(), Some("x".into()))
        );
        assert_eq!(
            resolve_href("OEBPS", "#top"),
            ("".into(), Some("top".into()))
        );
    }

    #[test]
    fn relative() {
        assert_eq!(
            relative_href("OEBPS/toc.ncx", "OEBPS/text/ch1.xhtml#a"),
            "text/ch1.xhtml#a"
        );
        assert_eq!(
            relative_href("OEBPS/text/nav.xhtml", "OEBPS/images/a b.png"),
            "../images/a%20b.png"
        );
    }

    #[test]
    fn sorts() {
        assert_eq!(title_sort("The Rust Book"), "Rust Book, The");
        // regression: a multi-byte character straddling the article length used to panic
        assert_eq!(title_sort("Война и мир"), "Война и мир");
        assert_eq!(title_sort("日本語の本"), "日本語の本");
        assert_eq!(title_sort("Ää"), "Ää");
        assert_eq!(title_sort("The Ünïcode"), "Ünïcode, The");
        assert_eq!(author_sort("Ada Lovelace"), "Lovelace, Ada");
        assert_eq!(percent_decode("a%20b%"), "a b%");
    }
}
