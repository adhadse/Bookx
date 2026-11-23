//! Small string/path helpers shared by the whole core.

use roxmltree::{Document, ParsingOptions};

/// Most attributes one element may carry. Real documents have a few dozen; the XML parser checks
/// each new attribute against the earlier ones, so a hostile element with a million of them would
/// take minutes (30 000 take 7 s in a debug build).
pub const MAX_ATTRS_PER_ELEMENT: usize = crate::limits::ATTRS_PER_ELEMENT;

/// Does some start tag in `text` carry more than [`MAX_ATTRS_PER_ELEMENT`] attributes? One linear
/// pass; comments, CDATA, processing instructions and quoted values are skipped.
pub fn has_attribute_flood(text: &str) -> bool {
    let b = text.as_bytes();
    let (mut i, n) = (0, b.len());
    let skip_to = |from: usize, end: &[u8]| -> usize {
        b[from..]
            .windows(end.len())
            .position(|w| w == end)
            .map_or(n, |p| from + p + end.len())
    };
    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &b[i..];
        if rest.starts_with(b"<!--") {
            i = skip_to(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = skip_to(i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            i = skip_to(i + 2, b">");
        } else {
            i += 1;
            let (mut quote, mut attrs) = (0u8, 0usize);
            while i < n {
                let c = b[i];
                i += 1;
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                    attrs += 1;
                    if attrs > MAX_ATTRS_PER_ELEMENT {
                        return true;
                    }
                } else if c == b'>' {
                    break;
                }
            }
        }
    }
    false
}

/// Deepest element nesting accepted in HTML from a book, feed or web page. html5ever and every
/// recursive walk over its tree overflow the stack long before 100 000 levels.
pub const MAX_HTML_DEPTH: usize = crate::limits::HTML_DEPTH;

/// Does `text` nest elements deeper than [`MAX_HTML_DEPTH`]? One linear pass over the tags that
/// stay open; void elements, self-closed tags and elements whose end tag is optional (`p`, `li`,
/// `td`, ...) are not counted, so ordinary tag soup does not trip it.
pub fn has_excessive_nesting(text: &str) -> bool {
    const SKIP: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr", "p", "li", "dt", "dd", "tr", "td", "th", "option", "optgroup",
        "tbody", "thead", "tfoot", "colgroup", "caption", "html", "head", "body", "rb", "rt", "rp",
        "rtc",
    ];
    let b = text.as_bytes();
    let (mut i, mut depth) = (0usize, 0usize);
    while let Some(off) = b[i..].iter().position(|&c| c == b'<') {
        i += off + 1;
        let Some(&c) = b.get(i) else { break };
        if c == b'/' {
            depth = depth.saturating_sub(1);
            continue;
        }
        if !c.is_ascii_alphabetic() {
            continue;
        }
        let end = b[i..]
            .iter()
            .position(|&c| !(c.is_ascii_alphanumeric() || c == b'-' || c == b':'))
            .map_or(b.len(), |p| i + p);
        let name = text[i..end].to_ascii_lowercase();
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        // find the end of the tag, skipping quoted values; `/>` closes it at once
        let (mut j, mut q, mut selfclosed) = (end, 0u8, false);
        while j < b.len() {
            let ch = b[j];
            if q != 0 {
                if ch == q {
                    q = 0;
                }
            } else if ch == b'"' || ch == b'\'' {
                q = ch;
            } else if ch == b'>' {
                selfclosed = j > 0 && b[j - 1] == b'/';
                break;
            }
            j += 1;
        }
        if !selfclosed {
            depth += 1;
            if depth > MAX_HTML_DEPTH {
                return true;
            }
        }
        i = j.max(end);
    }
    false
}

/// `scraper::Html::parse_document` for text that may be hostile: html5ever compares every new
/// attribute with the earlier ones of its element, so a flood of attributes (see
/// [`has_attribute_flood`]) is parsed as an empty document instead of tying a thread up for minutes.
pub fn parse_html_document(text: &str) -> scraper::Html {
    if has_attribute_flood(text) || has_excessive_nesting(text) {
        return scraper::Html::parse_document("");
    }
    scraper::Html::parse_document(text)
}

/// The fragment version of [`parse_html_document`].
pub fn parse_html_fragment(text: &str) -> scraper::Html {
    if has_attribute_flood(text) || has_excessive_nesting(text) {
        return scraper::Html::parse_fragment("");
    }
    scraper::Html::parse_fragment(text)
}

pub fn parse_xml(text: &str) -> anyhow::Result<Document<'_>> {
    let text = text.trim_start_matches('\u{feff}');
    if has_attribute_flood(text) {
        anyhow::bail!(
            "an element has more than {MAX_ATTRS_PER_ELEMENT} attributes (hostile or corrupt file)"
        );
    }
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

/// Does this href point outside the book (a URL, not an archive path)? This is a *classification*
/// for converters and checkers (`data:` images and `ftp:` links are not archive files); it is not
/// permission to open anything. To open a link from a book use [`safe_external_url`].
pub fn is_external(href: &str) -> bool {
    let h = href.trim().to_ascii_lowercase();
    h.starts_with("http://")
        || h.starts_with("https://")
        || h.starts_with("mailto:")
        || h.starts_with("ftp://")
        || h.starts_with("data:")
}

/// The longest link Bookx hands to the desktop.
pub const MAX_LAUNCH_URL: usize = crate::limits::LAUNCH_URL_BYTES;

/// The only gate between a link in a book (or feed, or metadata) and the desktop's URI handler.
/// Returns the URL to open, or `None` when it must not be opened.
///
/// Allowed: `http://host...`, `https://host...` and `mailto:` (without Thunderbird-style `attach`
/// parameters). Everything else is refused: `file:`, `data:`, `javascript:`, `ftp:`, `smb:`, `ssh:`,
/// `x-scheme-handler`, custom `bookx:`-style schemes, relative and protocol-relative links, and any
/// URL with control characters, spaces or backslashes (browsers read `http:\\evil` and
/// `java\tscript:` differently from a parser, so they are refused, not repaired), or with
/// characters that hide the real text (bidi overrides, zero-width, line separators).
/// Leading and trailing ASCII whitespace is trimmed. The scheme must be ASCII.
pub fn safe_external_url(href: &str) -> Option<String> {
    let u = href.trim_matches(|c: char| c.is_ascii_whitespace());
    if u.is_empty() || u.len() > MAX_LAUNCH_URL {
        return None;
    }
    if u.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || c == '\\'
            || matches!(
                c,
                '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}'
                    | '\u{FFF9}'..='\u{FFFB}'
            )
    }) {
        return None;
    }
    let (scheme, rest) = u.split_once(':')?;
    if !scheme.is_ascii() {
        return None;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" => {
            let after = rest.strip_prefix("//")?;
            let authority = after.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            let host = match host.strip_prefix('[') {
                Some(h) => h.split(']').next().unwrap_or(""),
                None => host.split(':').next().unwrap_or(""),
            };
            if host.is_empty() || host.starts_with('.') {
                return None;
            }
            Some(u.to_string())
        }
        "mailto" => {
            let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
            if addr.is_empty() && query.is_empty() {
                return None;
            }
            let q = percent_decode(query).to_ascii_lowercase();
            // `attach=` makes some mail clients attach a local file to the message
            if q.split('&').any(|kv| {
                let k = kv.split('=').next().unwrap_or("").trim();
                matches!(k, "attach" | "attachment" | "x-attach")
            }) {
                return None;
            }
            Some(u.to_string())
        }
        _ => None,
    }
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
    fn an_attribute_flood_is_refused_in_linear_time() {
        let flood = format!(
            "<r {}/>",
            (0..MAX_ATTRS_PER_ELEMENT + 1)
                .map(|i| format!("a{i}='1'"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let t = std::time::Instant::now();
        assert!(parse_xml(&flood).is_err());
        // a million attributes (about 10 MB) is still answered at once
        let huge = format!("<r {}/>", "a='1' ".repeat(1_000_000));
        assert!(parse_xml(&huge).is_err());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        // html5ever has the same quadratic check; the guarded entry points answer at once
        let html = format!("<p {}>x</p>", "a='1' ".repeat(400_000));
        let t = std::time::Instant::now();
        let doc = parse_html_document(&html);
        let frag = parse_html_fragment(&html);
        assert!(doc.root_element().text().collect::<String>().is_empty());
        assert!(frag.root_element().text().collect::<String>().is_empty());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        assert!(
            parse_html_document("<p class='a'>hi</p>")
                .root_element()
                .text()
                .any(|t| t == "hi")
        );
        // normal documents, odd quoting and `=` in text are fine
        assert!(
            parse_xml("<r a='1' b=\"x>y=z\"><!-- <a b='1' c='2'> --><![CDATA[<x y='1'>]]>k=v</r>")
                .is_ok()
        );
        let many_ok = format!(
            "<r {}/>",
            (0..200)
                .map(|i| format!("a{i}='1'"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(parse_xml(&many_ok).is_ok());
        // the flood is per element, not per document
        let many_elements = format!("<r>{}</r>", "<e a='1' b='2'/>".repeat(100_000));
        assert!(parse_xml(&many_elements).is_ok());
    }

    #[test]
    fn absurd_html_nesting_is_refused_before_the_parser_can_overflow() {
        let deep = format!("{}x{}", "<div>".repeat(100_000), "</div>".repeat(100_000));
        assert!(has_excessive_nesting(&deep));
        assert!(
            parse_html_document(&deep)
                .root_element()
                .text()
                .next()
                .is_none()
        );
        assert!(has_excessive_nesting(&"<div><p><span><b>".repeat(50_000)));
        assert!(has_excessive_nesting(&format!(
            "{}x",
            "<a href='>'>".repeat(5000)
        )));
        // ordinary documents, tag soup and long flat lists are fine
        assert!(!has_excessive_nesting(
            &"<p>text<br>more<img src=a.png>".repeat(100_000)
        ));
        assert!(!has_excessive_nesting(
            &"<ul><li>a<li>b<li>c</ul>".repeat(50_000)
        ));
        assert!(!has_excessive_nesting(&"<div>x</div>".repeat(100_000)));
        assert!(!has_excessive_nesting(&"<div/>".repeat(100_000)));
        assert!(!has_excessive_nesting(&nested_ok(1500)));
        assert!(
            parse_html_document("<p>hi</p>")
                .root_element()
                .text()
                .any(|t| t == "hi")
        );
    }

    fn nested_ok(n: usize) -> String {
        format!("{}x{}", "<div>".repeat(n), "</div>".repeat(n))
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
    fn only_http_https_and_mailto_are_ever_opened() {
        for ok in [
            "http://example.com",
            "https://example.com/a?b=c#d",
            "  https://example.com/x \n",
            "HTTPS://EXAMPLE.COM/",
            "https://[::1]:8080/x",
            "https://example.com/\u{65e5}\u{672c}",
            "mailto:me@example.com",
            "MAILTO:me@example.com?subject=hi",
        ] {
            assert!(safe_external_url(ok).is_some(), "{ok:?} should open");
        }
        assert_eq!(
            safe_external_url("  https://example.com/x \n").as_deref(),
            Some("https://example.com/x")
        );
        for bad in [
            "",
            "   ",
            "ftp://example.com/x",
            "data:text/html;base64,PHNjcmlwdD4=",
            "DATA:image/png;base64,AAAA",
            "file:///etc/passwd",
            "FILE:///etc/passwd",
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "\u{0}javascript:alert(1)",
            "\u{1}https://example.com",
            "https://exa\u{0}mple.com",
            "https://example.com/a\u{7f}",
            "https://example.com/a b",
            "https://example.com/\u{a0}x",
            "https:\\\\example.com",
            "http:/example.com",
            "http:example.com",
            "https://",
            "https:///x",
            "https://@/x",
            "//example.com/x",
            "/etc/passwd",
            "chapter1.xhtml#a",
            "smb://host/share",
            "ssh://host",
            "sftp://host",
            "vbscript:msgbox(1)",
            "x-scheme-handler/https://a",
            "bookx://open?id=1",
            "tel:+123456",
            "intent://x",
            "\u{ff48}ttps://example.com",
            "https\u{ff1a}//example.com",
            "https://example.com/\u{202e}gpj.exe",
            "https://example.com/\u{200b}",
            "https://example.com/\u{2028}",
            "https://example.com/\u{feff}",
            "mailto:",
            "mailto:?attach=/etc/passwd",
            "mailto:me@example.com?subject=x&Attach=/home/u/.ssh/id_rsa",
            "mailto:me@example.com?%61ttach=/etc/passwd",
            "mailto:me@example.com?attachment=/etc/passwd",
        ] {
            assert!(safe_external_url(bad).is_none(), "{bad:?} must not open");
        }
        assert!(safe_external_url(&format!("https://e.com/{}", "a".repeat(9000))).is_none());
    }

    #[test]
    fn is_external_stays_a_classification_not_permission() {
        // converters rely on this to skip inline images and ftp links
        assert!(is_external("data:image/png;base64,AAAA"));
        assert!(safe_external_url("data:image/png;base64,AAAA").is_none());
        assert!(is_external("ftp://example.com/x"));
        assert!(safe_external_url("ftp://example.com/x").is_none());
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
