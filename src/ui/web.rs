//! Tiny online look-ups for the reader (Wikipedia summaries, translation).
//!
//! Requests go through GIO (`gio::File` on an https URI), so no extra HTTP stack is needed.

use std::path::{Path, PathBuf};

use relm4::gtk::{gdk, gio, glib, prelude::*};
use serde_json::Value;

pub const LANGUAGES: [(&str, &str); 14] = [
    ("en", "English"),
    ("hi", "Hindi"),
    ("es", "Spanish"),
    ("fr", "French"),
    ("de", "German"),
    ("pt", "Portuguese"),
    ("it", "Italian"),
    ("ru", "Russian"),
    ("ja", "Japanese"),
    ("zh-CN", "Chinese"),
    ("ar", "Arabic"),
    ("ko", "Korean"),
    ("tr", "Turkish"),
    ("nl", "Dutch"),
];

pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The folder of canned answers for the UI tests (`$BOOKX_WEB_MOCK`): `wiki-search.json`,
/// `wiki-summary.json` and `wiki-thumb.png` stand in for Wikipedia when they exist. Once
/// `wiki-summary.json` is there the mock is on, and a picture without `wiki-thumb.png` fails like
/// a 404.
fn mock_answer(dir: &Path, url: &str) -> Option<Result<Vec<u8>, String>> {
    if url.contains("upload.wikimedia.org")
        && dir.join("wiki-summary.json").exists()
        && !dir.join("wiki-thumb.png").exists()
    {
        return Some(Err("404 Not Found (mock)".into()));
    }
    let name = if url.contains("/search/title") {
        "wiki-search.json"
    } else if url.contains("/page/summary/") {
        "wiki-summary.json"
    } else if url.contains("upload.wikimedia.org") {
        "wiki-thumb.png"
    } else {
        return None;
    };
    let f = dir.join(name);
    f.exists()
        .then(|| std::fs::read(f).map_err(|e| e.to_string()))
}

fn fetch_bytes(url: &str, cb: impl FnOnce(Result<Vec<u8>, String>) + 'static) {
    if let Some(dir) = std::env::var_os("BOOKX_WEB_MOCK").map(PathBuf::from) {
        if let Some(answer) = mock_answer(&dir, url) {
            glib::idle_add_local_once(move || cb(answer));
            return;
        }
    }
    let file = gio::File::for_uri(url);
    file.load_contents_async(gio::Cancellable::NONE, move |res| {
        cb(res
            .map(|(bytes, _)| bytes.to_vec())
            .map_err(|e| e.to_string()))
    });
}

fn fetch(url: &str, cb: impl FnOnce(Result<String, String>) + 'static) {
    fetch_bytes(url, move |res| {
        cb(res.map(|b| String::from_utf8_lossy(&b).into_owned()))
    });
}

/// No pictures are fetched when the app is offline (`BOOKX_OFFLINE`); cached ones still show.
pub fn offline() -> bool {
    std::env::var_os("BOOKX_OFFLINE").is_some()
}

/// The most the picture cache keeps on disk.
pub const IMAGE_CACHE_BYTES: u64 = 24 * 1024 * 1024;
/// A picture larger than this is not worth a popover.
pub const IMAGE_MAX_BYTES: usize = 4 * 1024 * 1024;

pub fn image_cache_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.cache_dir().join("wiki-images"))
        .unwrap_or_else(|| std::env::temp_dir().join("bookx-wiki-images"))
}

/// The file name of a picture in the cache (FNV-1a of its address).
pub fn image_cache_name(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.img")
}

/// Which cached files to delete to get under `cap` bytes: the oldest first (`files`: path, size,
/// age rank where a bigger number is older).
pub fn prune_plan(mut files: Vec<(PathBuf, u64, u64)>, cap: u64) -> Vec<PathBuf> {
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by(|a, b| b.2.cmp(&a.2));
    let mut out = Vec::new();
    for (p, size, _) in files {
        if total <= cap {
            break;
        }
        total = total.saturating_sub(size);
        out.push(p);
    }
    out
}

fn prune_cache(dir: &Path, cap: u64) {
    let now = std::time::SystemTime::now();
    let files: Vec<(PathBuf, u64, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            let age = now
                .duration_since(m.modified().ok()?)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            Some((e.path(), m.len(), age))
        })
        .collect();
    for p in prune_plan(files, cap) {
        let _ = std::fs::remove_file(p);
    }
}

fn texture_of(bytes: &[u8]) -> Result<gdk::Texture, String> {
    gdk::Texture::from_bytes(&glib::Bytes::from(bytes)).map_err(|e| e.to_string())
}

/// The picture at `url` as a texture: from the disk cache, else downloaded (unless offline) and
/// cached. The callback runs later on the main loop, never inside this call.
pub fn image(url: &str, cb: impl FnOnce(Result<gdk::Texture, String>) + 'static) {
    let dir = image_cache_dir();
    let file = dir.join(image_cache_name(url));
    if let Ok(bytes) = std::fs::read(&file) {
        if let Ok(t) = texture_of(&bytes) {
            // a hit counts as use: the oldest files go first
            let _ = std::fs::File::options()
                .append(true)
                .open(&file)
                .and_then(|f| f.set_modified(std::time::SystemTime::now()));
            glib::idle_add_local_once(move || cb(Ok(t)));
            return;
        }
        let _ = std::fs::remove_file(&file);
    }
    if offline() {
        glib::idle_add_local_once(move || cb(Err("offline".into())));
        return;
    }
    fetch_bytes(url, move |res| {
        let bytes = match res {
            Ok(b) if b.len() <= IMAGE_MAX_BYTES => b,
            Ok(_) => return cb(Err("the picture is too large".into())),
            Err(e) => return cb(Err(e)),
        };
        match texture_of(&bytes) {
            Ok(t) => {
                if std::fs::create_dir_all(&dir).is_ok() {
                    let _ = std::fs::write(&file, &bytes);
                    prune_cache(&dir, IMAGE_CACHE_BYTES);
                }
                cb(Ok(t));
            }
            Err(e) => cb(Err(e)),
        }
    });
}

/// First result key of a Wikipedia title search response.
pub fn parse_wiki_search(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["pages"][0]["key"].as_str().map(String::from))
}

pub fn parse_wiki_summary(body: &str, lang: &str, key: &str) -> Result<WikiResult, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let url = v["content_urls"]["desktop"]["page"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("https://{lang}.wikipedia.org/wiki/{}", encode(key)));
    // the lead picture: the thumbnail (small), else the original
    let thumb = v["thumbnail"]["source"]
        .as_str()
        .or_else(|| v["originalimage"]["source"].as_str())
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(String::from);
    Ok(WikiResult {
        title: v["title"].as_str().unwrap_or(key).to_string(),
        extract: v["extract"].as_str().unwrap_or("").to_string(),
        url,
        thumb,
    })
}

/// Joins the translated segments of a `translate_a/single` response.
pub fn parse_translation(body: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let out: String = v[0]
        .as_array()
        .map(|parts| parts.iter().filter_map(|p| p[0].as_str()).collect())
        .unwrap_or_default();
    if out.is_empty() {
        Err("No translation returned.".into())
    } else {
        Ok(out)
    }
}

/// Translating into the book's own language is pointless, so pick another target.
pub fn pick_translate_target(current: &str, book_lang: &str) -> String {
    let book = book_lang.split('-').next().unwrap_or("").to_lowercase();
    if current.split('-').next().unwrap_or("").to_lowercase() != book {
        return current.to_string();
    }
    ["hi", "es", "fr"]
        .iter()
        .find(|c| **c != book)
        .unwrap()
        .to_string()
}

pub struct WikiResult {
    pub title: String,
    pub extract: String,
    pub url: String,
    /// The lead picture of the article, when it has one.
    pub thumb: Option<String>,
}

/// Finds the best matching article for `term` and returns its summary.
pub fn wikipedia(lang: &str, term: &str, cb: impl FnOnce(Result<WikiResult, String>) + 'static) {
    let lang = lang.to_string();
    let search = format!(
        "https://{lang}.wikipedia.org/w/rest.php/v1/search/title?q={}&limit=1",
        encode(term)
    );
    fetch(&search, move |res| {
        let body = match res {
            Ok(b) => b,
            Err(e) => return cb(Err(e)),
        };
        let Some(key) = parse_wiki_search(&body) else {
            return cb(Err("No Wikipedia article found for this text.".into()));
        };
        let url = format!(
            "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{}",
            encode(&key)
        );
        fetch(&url, move |res| {
            cb(res.and_then(|b| parse_wiki_summary(&b, &lang, &key)));
        });
    });
}

pub fn translate(to: &str, text: &str, cb: impl FnOnce(Result<String, String>) + 'static) {
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl=auto&tl={to}&dt=t&q={}",
        encode(text)
    );
    fetch(&url, move |res| cb(res.and_then(|b| parse_translation(&b))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_components() {
        assert_eq!(encode("a b/c&é"), "a%20b%2Fc%26%C3%A9");
        assert_eq!(encode("safe-_.~09"), "safe-_.~09");
    }

    #[test]
    fn parses_translation_segments() {
        let body = r#"[[["दुनिया","world",null,null,10],[" नमस्ते","hello",null,null,1]],null,"en"]"#;
        assert_eq!(parse_translation(body).unwrap(), "दुनिया नमस्ते");
        assert!(parse_translation("[[],null]").is_err());
        assert!(parse_translation("not json").is_err());
    }

    #[test]
    fn parses_wikipedia() {
        assert_eq!(
            parse_wiki_search(r#"{"pages":[{"key":"World","title":"World"}]}"#).as_deref(),
            Some("World")
        );
        assert_eq!(parse_wiki_search(r#"{"pages":[]}"#), None);
        let r = parse_wiki_summary(
            r#"{"title":"World","extract":"The world is…","content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/World"}}}"#,
            "en",
            "World",
        )
        .unwrap();
        assert_eq!(r.title, "World");
        assert_eq!(r.url, "https://en.wikipedia.org/wiki/World");
        let r = parse_wiki_summary(r#"{"extract":"x"}"#, "de", "Alter Fritz").unwrap();
        assert_eq!(r.url, "https://de.wikipedia.org/wiki/Alter%20Fritz");
        assert_eq!(r.thumb, None);
    }

    #[test]
    fn the_lead_picture_is_the_thumbnail_else_the_original() {
        let both = r#"{"extract":"x","thumbnail":{"source":"https://upload.wikimedia.org/t.jpg","width":320},"originalimage":{"source":"https://upload.wikimedia.org/o.jpg"}}"#;
        let r = parse_wiki_summary(both, "en", "A").unwrap();
        assert_eq!(
            r.thumb.as_deref(),
            Some("https://upload.wikimedia.org/t.jpg")
        );
        let orig =
            r#"{"extract":"x","originalimage":{"source":"https://upload.wikimedia.org/o.jpg"}}"#;
        let r = parse_wiki_summary(orig, "en", "A").unwrap();
        assert_eq!(
            r.thumb.as_deref(),
            Some("https://upload.wikimedia.org/o.jpg")
        );
        // only web addresses count
        let odd = r#"{"extract":"x","thumbnail":{"source":"file:///etc/passwd"}}"#;
        assert_eq!(parse_wiki_summary(odd, "en", "A").unwrap().thumb, None);
    }

    #[test]
    fn cache_names_are_stable_and_distinct() {
        let a = image_cache_name("https://upload.wikimedia.org/a.jpg");
        assert_eq!(a, image_cache_name("https://upload.wikimedia.org/a.jpg"));
        assert_ne!(a, image_cache_name("https://upload.wikimedia.org/b.jpg"));
        assert_eq!(a.len(), 16 + 4);
        assert!(a.ends_with(".img"));
    }

    #[test]
    fn the_cache_drops_the_oldest_pictures_first() {
        let f = |n: &str, size: u64, age: u64| (PathBuf::from(n), size, age);
        let files = vec![f("new", 10, 1), f("old", 10, 900), f("mid", 10, 50)];
        assert!(prune_plan(files.clone(), 30).is_empty(), "within the cap");
        assert_eq!(prune_plan(files.clone(), 25), vec![PathBuf::from("old")]);
        assert_eq!(
            prune_plan(files.clone(), 10),
            vec![PathBuf::from("old"), PathBuf::from("mid")]
        );
        assert_eq!(prune_plan(files, 0).len(), 3);
        assert!(prune_plan(Vec::new(), 0).is_empty());
    }

    #[test]
    fn pruning_the_cache_folder_removes_files_over_the_cap() {
        let d = std::env::temp_dir().join(format!("bookx-imgcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let now = std::time::SystemTime::now();
        for (name, age) in [("a", 300u64), ("b", 200), ("c", 100)] {
            let f = d.join(name);
            std::fs::write(&f, vec![0u8; 100]).unwrap();
            let t = now - std::time::Duration::from_secs(age);
            std::fs::File::options()
                .append(true)
                .open(&f)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        prune_cache(&d, 250);
        let mut left: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["b", "c"], "the oldest file went");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn canned_answers_stand_in_for_wikipedia() {
        let d = std::env::temp_dir().join(format!("bookx-webmock-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("wiki-summary.json"), b"{}").unwrap();
        let s = "https://en.wikipedia.org/api/rest_v1/page/summary/X";
        assert_eq!(mock_answer(&d, s), Some(Ok(b"{}".to_vec())));
        assert_eq!(
            mock_answer(
                &d,
                "https://en.wikipedia.org/w/rest.php/v1/search/title?q=x"
            ),
            None,
            "no such file"
        );
        assert_eq!(
            mock_answer(&d, "https://example.com/"),
            None,
            "other hosts are real"
        );
        assert!(
            matches!(
                mock_answer(&d, "https://upload.wikimedia.org/a/b.png"),
                Some(Err(_))
            ),
            "a picture that is not canned fails"
        );
        std::fs::write(d.join("wiki-thumb.png"), b"png").unwrap();
        assert_eq!(
            mock_answer(&d, "https://upload.wikimedia.org/a/b.png"),
            Some(Ok(b"png".to_vec()))
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn translate_target_avoids_book_language() {
        assert_eq!(pick_translate_target("en", "en-US"), "hi");
        assert_eq!(pick_translate_target("EN", "en"), "hi");
        assert_eq!(pick_translate_target("hi", "hi"), "es");
        assert_eq!(pick_translate_target("fr", "en"), "fr");
        assert_eq!(pick_translate_target("en", ""), "en");
    }
}
