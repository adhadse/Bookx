//! What a report may say about an address, a path or a file name, and what it may not.
//!
//! A server address decides many sync bugs by its *shape* (`http` or `https`, the port, the path the
//! server publishes its API under) and never by *which* machine it is. [`shape`] keeps the shape and
//! replaces the rest: `http://192.168.1.5:30260/api/koreader` becomes
//! `http://<ip:private>:30260/api/koreader`. The user name and password of the address, the query and
//! the fragment are always removed; a path segment that is not a known API word becomes `<segment>`
//! (it could be a user name, a token or a folder). [`clean_text`] does the same for every address, path
//! and file name inside free text (an error message from a library), before the general scrubber runs.
//!
//! No GTK, no I/O.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

use regex::Regex;

/// Path words that are part of a sync, catalogue or file API and name nobody.
const KNOWN_SEGMENTS: &[&str] = &[
    "api", "koreader", "kosync", "syncs", "sync", "progress", "users", "user", "auth", "create", "opds",
    "catalog", "catalogue", "books", "book", "library", "libraries", "webdav", "dav", "remote.php",
    "rest", "v1", "v2", "v3", "kobo", "kepub", "koboapi", "app", "plugin", "plugins", "annotations",
    "bookx", "bookx-sync", "healthcheck", "health", "login", "logout", "files", "file", "download",
    "upload", "static", "index.php", "index.html", "public", "reader", "web", "ui", "calibre",
    "calibre-web", "grimmory", "booklore", "komga", "kavita", "audiobookshelf", "ebooks", "kosync-server",
    "devices", "device", "stats", "statistics", "cwa", "cwa-opds", "opds-catalog", "me", "self",
];

/// The class of a host, never the host itself.
pub fn host_class(host: &str) -> String {
    let h = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    if h.is_empty() {
        return "<host?>".into();
    }
    if let Ok(v4) = h.parse::<Ipv4Addr>() {
        return format!("<ip:{}>", v4_class(&v4));
    }
    if let Ok(v6) = h.parse::<Ipv6Addr>() {
        return format!("<ip6:{}>", v6_class(&v6));
    }
    if h == "localhost" {
        return "localhost".into();
    }
    let local_suffix = [".local", ".lan", ".home", ".internal", ".home.arpa", ".localdomain", ".intranet"]
        .iter()
        .any(|s| h.ends_with(s));
    if local_suffix {
        "<hostname:local>".into()
    } else if h.contains('.') {
        "<domain>".into()
    } else {
        "<hostname>".into()
    }
}

fn v4_class(a: &Ipv4Addr) -> &'static str {
    if a.is_loopback() {
        "loopback"
    } else if a.is_private() || a.is_link_local() {
        "private"
    } else if a.octets()[0] == 100 && (64..128).contains(&a.octets()[1]) {
        "cgnat"
    } else if a.is_unspecified() {
        "any"
    } else {
        "public"
    }
}

fn v6_class(a: &Ipv6Addr) -> &'static str {
    let first = a.segments()[0];
    if a.is_loopback() {
        "loopback"
    } else if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
        "private"
    } else {
        "public"
    }
}

/// Characters a host may have; anything else is not shown even when the user allowed the host.
fn plausible_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
}

fn segment_text(seg: &str) -> String {
    let s = seg.to_ascii_lowercase();
    if KNOWN_SEGMENTS.contains(&s.as_str()) {
        s
    } else {
        "<segment>".into()
    }
}

/// The shape of a server address. With `show_host` the host is kept as typed (the user allowed it in
/// the report dialog); the credentials, query and fragment never are.
pub fn shape(raw: &str, show_host: bool) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "(none)".into();
    }
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) => {
            (s.to_ascii_lowercase(), r)
        }
        _ => ("(no scheme)".to_string(), raw),
    };
    let (authority_path, has_query) = {
        let cut = rest.find(['?', '#']);
        (&rest[..cut.unwrap_or(rest.len())], cut.is_some())
    };
    let (authority, path) = match authority_path.find('/') {
        Some(i) => (&authority_path[..i], &authority_path[i..]),
        None => (authority_path, ""),
    };
    let (creds, hostport) = match authority.rfind('@') {
        Some(i) => (true, &authority[i + 1..]),
        None => (false, authority),
    };
    let (host, port) = split_host_port(hostport);
    let mut out = String::new();
    if scheme != "(no scheme)" {
        out.push_str(&scheme);
        out.push_str("://");
    } else {
        out.push_str("(no scheme) ");
    }
    if creds {
        out.push_str("<credentials>@");
    }
    if show_host && plausible_host(host) {
        out.push_str(host);
    } else {
        out.push_str(&host_class(host));
    }
    match port {
        Some(p) if !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()) => {
            out.push(':');
            out.push_str(p);
        }
        Some(_) => out.push_str(":<port?>"),
        None => {}
    }
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        out.push_str(&segment_text(seg));
    }
    if path.ends_with('/') && path.len() > 1 {
        out.push('/');
    }
    if has_query {
        out.push_str("?<query>");
    }
    out
}

fn split_host_port(hp: &str) -> (&str, Option<&str>) {
    if let Some(rest) = hp.strip_prefix('[') {
        // [v6]:port
        if let Some(end) = rest.find(']') {
            let host = &hp[..end + 2];
            let port = rest[end + 1..].strip_prefix(':');
            return (host, port);
        }
        return (hp, None);
    }
    match hp.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, Some(p)),
        _ => (hp, None),
    }
}

fn rx(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static pattern"))
}

/// File extensions of books and their companions: a name that ends in one is the user's file name.
const BOOK_EXT: &str = "epub|kepub|pdf|mobi|azw3?|azw|fb2|fbz|cbz|cbr|cbt|cb7|djvu?|txt|md|docx?|odt|rtf|htmlz?|txtz|lit|lrf|pdb|prc|mp3|m4b|m4a|ogg|opus|flac|wav|zip|json|db|sqlite|xml|opf|html?|css|png|jpe?g|gif|webp|svg";

/// Paths that start here are about the program or the system, not about the user.
const SYSTEM_ROOTS: &[&str] = &[
    "usr", "app", "proc", "sys", "etc", "lib", "lib64", "bin", "sbin", "org", "com", "net", "dev", "rustc",
    "opt", "var/lib/flatpak", "run/user", "run/host", "run/flatpak", "cargo", "rust", "build", "builddir",
];

/// Cleans the addresses, paths and file names inside free text (see [`clean_urls`], [`clean_paths`]).
/// Where a scrubber that knows the library folder and the titles is available, run it *between* the two
/// (see `Redactor::text`): a title with spaces inside a path is whole until the path is cut.
pub fn clean_text(text: &str) -> String {
    clean_paths(&clean_urls(text))
}

/// Every address in the text as its shape (host class, port, known path words).
pub fn clean_urls(text: &str) -> String {
    static URL: OnceLock<Regex> = OnceLock::new();
    rx(&URL, r#"[A-Za-z][A-Za-z0-9+.\-]*://[^\s'"<>)\]}“”]+"#)
        .replace_all(text, |c: &regex::Captures| {
            let m = &c[0];
            // a trailing sentence mark is not part of the address
            let trimmed = m.trim_end_matches(['.', ',', ';', ':', '!']);
            let tail = &m[trimmed.len()..];
            format!("{}{}", shape(trimmed, false), tail)
        })
        .into_owned()
}

/// Words that end a file name inside a sentence (`cannot open /a/b.epub for alice`).
const PATH_STOPS: &[&str] = &[
    " for ", " at ", " on ", " from ", " with ", " as ", " and ", " is ", " was ", " not ", " failed", " could",
    " cannot", " because", " while", " when", " to ", " in ", " by ", ", ", "; ", " (",
];

/// Paths and file names in the text: a path that is not the program's or the system's becomes
/// `<path>` (plus the extension of its last part), a quoted or bare name with a book extension becomes
/// `<file>`.
pub fn clean_paths(text: &str) -> String {
    static QUOTED: OnceLock<Regex> = OnceLock::new();
    static NAMED: OnceLock<Regex> = OnceLock::new();
    static ABS: OnceLock<Regex> = OnceLock::new();
    static HOME_REL: OnceLock<Regex> = OnceLock::new();
    let ext = BOOK_EXT;
    // a quoted name with a book or document extension: 'Secret Title.pdf', "My Book.epub", `x.fb2`
    let mut s = rx(
        &QUOTED,
        &format!(r#"(?i)(['"`“‘])[^'"`”’\n]{{0,200}}\.(?:{ext})(['"`”’])"#),
    )
    .replace_all(text, "${1}<file>${2}")
    .into_owned();
    // an absolute path (parts may hold spaces) that is not the program's or the system's
    s = rx(&ABS, r#"(?:^|[\s(=:'"`])((?:/[^/\n'"`:()]+)+/?)"#)
        .replace_all(&s, |c: &regex::Captures| {
            let whole = &c[0];
            let path = &c[1];
            if is_system_path(path) {
                return whole.to_string();
            }
            let lead = &whole[..whole.len() - path.len()];
            let (p, rest) = split_sentence(path);
            format!("{lead}{}{rest}", path_shape(p))
        })
        .into_owned();
    s = rx(&HOME_REL, r#"~/[^\n'"`:()]+"#)
        .replace_all(&s, |c: &regex::Captures| {
            let (p, rest) = split_sentence(&c[0]);
            format!("{}{rest}", path_shape(p))
        })
        .into_owned();
    s = rx(
        &NAMED,
        &format!(r#"(?i)[^\s/'"`:()<>]*[^\s/'"`:()<>.]\.(?:{ext})\b"#),
    )
    .replace_all(&s, |c: &regex::Captures| {
        let m = &c[0];
        if m.starts_with('<') {
            return m.to_string();
        }
        let ext = m.rsplit('.').next().unwrap_or("");
        format!("<file>.{}", ext.to_ascii_lowercase())
    })
    .into_owned();
    // serial numbers and ids: eight or more capitals and digits with both kinds in them
    static SERIAL: OnceLock<Regex> = OnceLock::new();
    rx(&SERIAL, r"\b[A-Z0-9]{8,}\b")
        .replace_all(&s, |c: &regex::Captures| {
            let m = &c[0];
            let digit = m.bytes().any(|b| b.is_ascii_digit());
            let upper = m.bytes().any(|b| b.is_ascii_uppercase());
            if digit && upper { "<id>".to_string() } else { m.to_string() }
        })
        .into_owned()
}

/// Cuts a path that runs into the sentence around it: `("/a/b.epub", " for alice")`.
fn split_sentence(path: &str) -> (&str, &str) {
    let last = path.rfind('/').unwrap_or(0);
    let tail = &path[last..];
    let cut = PATH_STOPS
        .iter()
        .filter_map(|w| tail.find(w))
        .min()
        .map(|i| last + i);
    // a known extension ends the name too
    static EXT_END: OnceLock<Regex> = OnceLock::new();
    let ext_end = rx(&EXT_END, &format!(r"(?i)\.(?:{BOOK_EXT})\b"))
        .find(tail)
        .map(|m| last + m.end());
    let end = match (cut, ext_end) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => path.len(),
    };
    path.split_at(end.min(path.len()))
}

fn is_system_path(p: &str) -> bool {
    let t = p.trim_start_matches('/');
    SYSTEM_ROOTS
        .iter()
        .any(|r| t == *r || t.starts_with(&format!("{r}/")))
}

/// `/mnt/NAS-Alice/Books/x.epub` becomes `<path>` plus the extension of the last part.
fn path_shape(p: &str) -> String {
    let last = p.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, e)) if (1..=5).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            format!("<path>/<file>.{}", e.to_ascii_lowercase())
        }
        _ => "<path>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_of_the_address_in_the_bug_report_keeps_scheme_port_and_path() {
        assert_eq!(
            shape("http://192.168.1.5:30260/api/koreader", false),
            "http://<ip:private>:30260/api/koreader"
        );
    }

    #[test]
    fn hosts_are_told_by_class() {
        for (a, want) in [
            ("http://10.0.0.7/api", "http://<ip:private>/api"),
            ("http://127.0.0.1:8080", "http://<ip:loopback>:8080"),
            ("http://203.0.113.9", "http://<ip:public>"),
            ("http://100.100.1.1", "http://<ip:cgnat>"),
            ("http://[::1]:80/", "http://<ip6:loopback>:80"),
            ("http://[fd00::5]/", "http://<ip6:private>"),
            ("http://[2001:db8::1]:443/", "http://<ip6:public>:443"),
            ("https://nas/api", "https://<hostname>/api"),
            ("https://nas.local/api", "https://<hostname:local>/api"),
            ("https://books.example.org", "https://<domain>"),
            ("http://localhost:3000", "http://localhost:3000"),
        ] {
            assert_eq!(shape(a, false), want, "{a}");
        }
    }

    #[test]
    fn credentials_query_and_fragment_never_survive() {
        let s = shape("https://alice:hunter2@books.example.org:8443/api/koreader?token=abc#frag", false);
        assert_eq!(s, "https://<credentials>@<domain>:8443/api/koreader?<query>");
        let s = shape("https://alice:hunter2@books.example.org/x", true);
        assert!(!s.contains("alice") && !s.contains("hunter2"), "{s}");
        assert!(s.contains("books.example.org"));
    }

    #[test]
    fn path_segments_that_could_name_someone_are_replaced() {
        assert_eq!(
            shape("http://h.example.org/users/alice/9f8a7b6c5d4e3f2a1b0c/syncs/progress", false),
            "http://<domain>/users/<segment>/<segment>/syncs/progress"
        );
        assert_eq!(shape("http://h.example.org/Alice-Books", false), "http://<domain>/<segment>");
        assert_eq!(shape("http://h.example.org/API/KOReader/", false), "http://<domain>/api/koreader/");
    }

    #[test]
    fn broken_addresses_are_still_cleaned() {
        assert_eq!(shape("", false), "(none)");
        assert_eq!(shape("192.168.1.5:30260/api", false), "(no scheme) <ip:private>:30260/api");
        assert_eq!(shape("http://h.example.org:notaport/x", false), "http://<domain>:<port?>/<segment>");
        assert!(!shape("http://bad host with spaces/x", true).contains("bad host"));
    }

    #[test]
    fn free_text_loses_addresses_paths_and_file_names() {
        for (inp, forbidden) in [
            ("cannot open /home/alice/Books/Foo/Foo.epub", "Foo"),
            ("cannot open /mnt/NAS-Alice/Books/Foo.epub", "NAS-Alice"),
            ("talking to http://10.0.0.7:30260/api/koreader", "10.0.0.7"),
            ("no such file: 'Secret Title.pdf'", "Secret"),
            ("failed: \u{201c}Der geheime Garten.epub\u{201d}", "geheime"),
            ("copy Secret_Title_Final.mobi failed", "Secret"),
            ("see ~/Documents/Private Stuff/My Diary.docx now", "Diary"),
            ("get https://alice:pw@nas.example.org/dav/Alice/ failed.", "alice"),
            ("open /run/media/alice/KOBOeReader/Secret.kepub.epub", "KOBOeReader"),
        ] {
            let c = clean_text(inp);
            assert!(!c.to_lowercase().contains(&forbidden.to_lowercase()), "{inp} -> {c}");
        }
        let c = clean_text("talking to http://10.0.0.7:30260/api/koreader");
        assert_eq!(c, "talking to http://<ip:private>:30260/api/koreader");
        let c = clean_text("no such file: 'Secret Title.pdf'");
        assert_eq!(c, "no such file: '<file>'");
    }

    #[test]
    fn program_and_system_paths_stay_readable() {
        let t = "at /usr/lib/x86_64-linux-gnu/libfoo.so and /app/bin/bookx and /org/freedesktop/portal/desktop";
        assert_eq!(clean_text(t), t);
        assert!(clean_text("/rustc/abcdef/library/core/src/x.rs:1:2").contains("/rustc/"));
    }

    #[test]
    fn cleaning_is_idempotent_and_never_panics_on_odd_text() {
        for t in ["", "://", "http://", "///", "a//b", "'.pdf'", "~/", "\u{0}\u{1f600}/x/y.epub", "http://[::1"] {
            let once = clean_text(t);
            assert_eq!(clean_text(&once), once, "{t:?}");
            let _ = shape(t, false);
            let _ = shape(t, true);
        }
    }
}
