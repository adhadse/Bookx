//! A WebDAV folder (Nextcloud, ownCloud, Synology, any WebDAV share) as a place to send books to
//! and to list and clean up, so a cloud folder can be used like an e-reader.
//!
//! **Why no WebDAV crate?** WebDAV needs four requests (`PROPFIND`, `MKCOL`, `PUT`, `DELETE`, plus
//! basic authentication) and the XML of a `PROPFIND` answer, which `roxmltree` already parses here.
//! The WebDAV client crates for Rust pull in `reqwest` and its async stack, and `ureq` (already in
//! the build for `bookx-sources`) refuses non-standard methods like `PROPFIND`. So [`crate::httpc`]
//! is a ~150 line blocking HTTP/1.1 client over `rustls` (+ `webpki-roots`, both already in the
//! dependency tree), tested here.
//!
//! Tested against a small in-process WebDAV server. It has not been run against Nextcloud or
//! another real server.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use bookx_core::contracts::{DeviceDriver, DeviceInfo};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

use crate::devfs::{DeviceBook, Space};
use crate::devices::sanitize_file_name;

/// Everything but the characters that are safe in a path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

const BOOK_EXT: &[&str] = &[
    "epub", "kepub", "mobi", "azw", "azw3", "pdf", "fb2", "txt", "cbz", "djvu",
];

/// A WebDAV collection.
pub struct WebDav {
    base: url::Url,
    auth: Option<String>,
}

/// One entry of a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// Path below the base URL, `/` separated, no leading slash.
    pub rel: String,
    pub is_dir: bool,
    pub size: u64,
}

impl WebDav {
    /// `url` must be `http://` or `https://`; credentials belong in `user`/`password`, not in the
    /// URL. The URL is the folder everything happens in.
    pub fn new(url: &str, user: &str, password: &str) -> Result<WebDav> {
        let mut base = url::Url::parse(url.trim()).context("that is not a web address")?;
        if !matches!(base.scheme(), "http" | "https") {
            bail!("a WebDAV address starts with http:// or https://");
        }
        if !base.username().is_empty() || base.password().is_some() {
            bail!("put the user name and password in their own fields, not in the address");
        }
        if base.host_str().is_none() {
            bail!("the address has no host");
        }
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        base.set_query(None);
        base.set_fragment(None);
        let auth = (!user.is_empty()).then(|| {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
            )
        });
        Ok(WebDav { base, auth })
    }

    pub fn base_url(&self) -> &str {
        self.base.as_str()
    }

    fn url_for(&self, parts: &[String], trailing_slash: bool) -> String {
        let mut u = self.base.as_str().to_string();
        for (i, p) in parts.iter().enumerate() {
            u.push_str(&utf8_percent_encode(p, SEGMENT).to_string());
            if i + 1 < parts.len() || trailing_slash {
                u.push('/');
            }
        }
        u
    }

    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<(u16, Vec<u8>)> {
        let u = url::Url::parse(url).context("bad address")?;
        let mut h: Vec<(&str, &str)> = headers.to_vec();
        if let Some(a) = &self.auth {
            h.push(("Authorization", a.as_str()));
        }
        let r =
            crate::httpc::request(method, &u, &h, &body, Duration::from_secs(60)).map_err(|e| {
                anyhow!(
                    "could not reach {}: {e:#}",
                    self.base.host_str().unwrap_or("")
                )
            })?;
        Ok((r.status, r.body))
    }

    fn explain(status: u16, what: &str) -> anyhow::Error {
        match status {
            401 | 403 => anyhow!("{what}: the server refused the user name or password ({status})"),
            404 => anyhow!("{what}: that folder does not exist on the server (404)"),
            507 => anyhow!("{what}: the cloud folder is full (507)"),
            s => anyhow!("{what}: the server answered {s}"),
        }
    }

    /// Checks that the folder can be reached with the credentials.
    pub fn check(&self) -> Result<()> {
        let (st, _) = self.request(
            "PROPFIND",
            self.base.as_str(),
            &[("Depth", "0"), ("Content-Type", "application/xml")],
            PROPFIND_BODY.as_bytes().to_vec(),
        )?;
        if st == 207 || st == 200 {
            Ok(())
        } else {
            Err(Self::explain(st, "Connecting"))
        }
    }

    /// Entries directly inside `parts` (a path below the base); a missing folder is an empty list.
    pub fn list_dir(&self, parts: &[String]) -> Result<Vec<RemoteEntry>> {
        let url = self.url_for(parts, true);
        let (st, body) = self.request(
            "PROPFIND",
            &url,
            &[("Depth", "1"), ("Content-Type", "application/xml")],
            PROPFIND_BODY.as_bytes().to_vec(),
        )?;
        match st {
            404 => Ok(Vec::new()),
            207 => parse_multistatus(&body, &self.base, &url),
            s => Err(Self::explain(s, "Listing")),
        }
    }

    /// Every book below the base folder (three levels deep at most).
    pub fn list_books(&self) -> Result<Vec<RemoteEntry>> {
        let mut out = Vec::new();
        let mut stack: Vec<(Vec<String>, usize)> = vec![(Vec::new(), 0)];
        let mut visited = 0;
        while let Some((dir, depth)) = stack.pop() {
            visited += 1;
            if visited > 200 {
                break;
            }
            for e in self.list_dir(&dir)? {
                if e.is_dir {
                    if depth < 3 {
                        stack.push((e.rel.split('/').map(str::to_string).collect(), depth + 1));
                    }
                } else if is_book(&e.rel) {
                    out.push(e);
                }
            }
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    }

    /// Creates the folders (one `MKCOL` each); folders that exist are fine.
    pub fn ensure_dirs(&self, parts: &[String]) -> Result<()> {
        for i in 1..=parts.len() {
            let url = self.url_for(&parts[..i], true);
            let (st, _) = self.request("MKCOL", &url, &[], Vec::new())?;
            // 405: exists (RFC 4918), 301: some servers redirect to the existing collection
            if !matches!(st, 200 | 201 | 301 | 405) {
                return Err(Self::explain(st, "Creating a folder"));
            }
        }
        Ok(())
    }

    /// Uploads a new file without overwriting: a taken name gets ` (1)`, ` (2)`… Returns the path
    /// below the base.
    pub fn put_new(&self, subdirs: &[String], file_name: &str, bytes: &[u8]) -> Result<String> {
        if file_name.is_empty() || file_name.contains(['/', '\\', '\0']) || file_name == ".." {
            bail!("refusing file name {file_name:?}");
        }
        let dirs: Vec<String> = subdirs
            .iter()
            .map(|d| {
                if d == ".." || d.contains(['/', '\\', '\0']) || d.is_empty() {
                    Err(anyhow!("refusing folder name {d:?}"))
                } else {
                    Ok(sanitize_file_name(d))
                }
            })
            .collect::<Result<_>>()?;
        self.ensure_dirs(&dirs)?;
        let taken: Vec<String> = self
            .list_dir(&dirs)?
            .into_iter()
            .map(|e| e.rel.rsplit('/').next().unwrap_or("").to_lowercase())
            .collect();
        let name = sanitize_file_name(file_name);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut candidate = name.clone();
        let mut n = 0;
        while taken.contains(&candidate.to_lowercase()) {
            n += 1;
            candidate = format!("{stem} ({n}){ext}");
            if n > 1000 {
                bail!("could not find a free file name");
            }
        }
        let mut parts = dirs;
        parts.push(candidate);
        let url = self.url_for(&parts, false);
        let (st, _) = self.request(
            "PUT",
            &url,
            &[
                ("If-None-Match", "*"),
                ("Content-Type", "application/octet-stream"),
            ],
            bytes.to_vec(),
        )?;
        match st {
            200 | 201 | 204 => Ok(parts.join("/")),
            412 => bail!("a file with that name appeared meanwhile; try again"),
            s => Err(Self::explain(s, "Uploading")),
        }
    }

    /// Uploads a file, replacing one of the same name (used for backups, whose names are unique).
    pub fn put_replace(&self, subdirs: &[String], file_name: &str, bytes: &[u8]) -> Result<()> {
        let parts = checked_parts(subdirs, file_name)?;
        self.ensure_dirs(&parts[..parts.len() - 1])?;
        let url = self.url_for(&parts, false);
        let (st, _) = self.request(
            "PUT",
            &url,
            &[("Content-Type", "application/octet-stream")],
            bytes.to_vec(),
        )?;
        match st {
            200 | 201 | 204 => Ok(()),
            s => Err(Self::explain(s, "Uploading")),
        }
    }

    /// Downloads a file below the base, up to `max` bytes.
    pub fn get_file(&self, subdirs: &[String], file_name: &str, max: usize) -> Result<Vec<u8>> {
        let parts = checked_parts(subdirs, file_name)?;
        let u = url::Url::parse(&self.url_for(&parts, false)).context("bad address")?;
        let mut h: Vec<(&str, &str)> = Vec::new();
        if let Some(a) = &self.auth {
            h.push(("Authorization", a.as_str()));
        }
        let r =
            crate::httpc::request_limited("GET", &u, &h, &[], Duration::from_secs(900), None, max)
                .map_err(|e| {
                    anyhow!(
                        "could not reach {}: {e:#}",
                        self.base.host_str().unwrap_or("")
                    )
                })?;
        match r.status {
            200 => Ok(r.body),
            s => Err(Self::explain(s, "Downloading")),
        }
    }

    /// Uploads exactly `len` bytes from `reader` to the file at `parts` (path below the base, the
    /// folders must exist), without holding them in memory. A reader error aborts the request, so
    /// nothing half-sent is kept by well-behaved servers (the caller uploads to a temporary name
    /// and moves it into place).
    pub fn put_stream(
        &self,
        parts: &[String],
        reader: &mut dyn std::io::Read,
        len: u64,
    ) -> Result<()> {
        check_segments(parts)?;
        let u = url::Url::parse(&self.url_for(parts, false)).context("bad address")?;
        let mut h: Vec<(&str, &str)> = vec![("Content-Type", "application/octet-stream")];
        if let Some(a) = &self.auth {
            h.push(("Authorization", a.as_str()));
        }
        let r = crate::httpc::request_stream(
            "PUT",
            &u,
            &h,
            crate::httpc::Body::Stream(reader, len),
            Duration::from_secs(12 * 3600),
            None,
            crate::httpc::MAX_BODY,
            None,
        )?;
        match r.status {
            200 | 201 | 204 => Ok(()),
            s => Err(Self::explain(s, "Uploading")),
        }
    }

    /// Uploads `len` bytes from `reader` as the bytes `from..from + len` of a file that will be
    /// `total` bytes long (`PUT` with `Content-Range`, which only some servers honour; see
    /// `DavStore`'s probe). The first chunk (`from == 0`) creates the file, later ones continue it.
    pub fn put_range(
        &self,
        parts: &[String],
        reader: &mut dyn std::io::Read,
        from: u64,
        len: u64,
        total: u64,
    ) -> Result<()> {
        check_segments(parts)?;
        if len == 0 || from.checked_add(len).is_none_or(|e| e > total) {
            bail!("a bad byte range");
        }
        let u = url::Url::parse(&self.url_for(parts, false)).context("bad address")?;
        let range = format!("bytes {}-{}/{}", from, from + len - 1, total);
        let mut h: Vec<(&str, &str)> = vec![
            ("Content-Type", "application/octet-stream"),
            ("Content-Range", range.as_str()),
        ];
        if let Some(a) = &self.auth {
            h.push(("Authorization", a.as_str()));
        }
        let r = crate::httpc::request_stream(
            "PUT",
            &u,
            &h,
            crate::httpc::Body::Stream(reader, len),
            Duration::from_secs(12 * 3600),
            None,
            crate::httpc::MAX_BODY,
            None,
        )?;
        match r.status {
            200 | 201 | 204 | 206 => Ok(()),
            s => Err(Self::explain(s, "Uploading")),
        }
    }

    /// Downloads the file at `parts` into `out` as it arrives; more than `max` bytes is an error.
    pub fn get_to(&self, parts: &[String], out: &mut dyn std::io::Write, max: u64) -> Result<u64> {
        check_segments(parts)?;
        let u = url::Url::parse(&self.url_for(parts, false)).context("bad address")?;
        let mut h: Vec<(&str, &str)> = Vec::new();
        if let Some(a) = &self.auth {
            h.push(("Authorization", a.as_str()));
        }
        let r = crate::httpc::request_stream(
            "GET",
            &u,
            &h,
            crate::httpc::Body::Bytes(&[]),
            Duration::from_secs(12 * 3600),
            None,
            crate::httpc::MAX_BODY,
            Some(crate::httpc::Sink { out, max }),
        )?;
        match r.status {
            200 => Ok(r.streamed),
            s => Err(Self::explain(s, "Downloading")),
        }
    }

    /// Moves a file to a new path below the base, replacing what is there. `Ok(false)` when the
    /// server does not do `MOVE` (405/501), so the caller can fall back.
    pub fn move_file(&self, from: &[String], to: &[String]) -> Result<bool> {
        check_segments(from)?;
        check_segments(to)?;
        let dest = self.url_for(to, false);
        let (st, _) = self.request(
            "MOVE",
            &self.url_for(from, false),
            &[("Destination", dest.as_str()), ("Overwrite", "T")],
            Vec::new(),
        )?;
        match st {
            200 | 201 | 204 => Ok(true),
            405 | 501 => Ok(false),
            s => Err(Self::explain(s, "Moving")),
        }
    }

    /// Deletes any file below the base (not a folder).
    pub fn delete_file(&self, subdirs: &[String], file_name: &str) -> Result<()> {
        let parts = checked_parts(subdirs, file_name)?;
        let (st, _) = self.request("DELETE", &self.url_for(&parts, false), &[], Vec::new())?;
        match st {
            200 | 202 | 204 | 404 => Ok(()),
            s => Err(Self::explain(s, "Removing")),
        }
    }

    /// Deletes a book (not a folder) by its path below the base.
    pub fn delete_book(&self, rel: &str) -> Result<()> {
        let parts: Vec<String> = rel.split('/').map(str::to_string).collect();
        if parts.iter().any(|p| p.is_empty() || p == ".." || p == ".") || !is_book(rel) {
            bail!("{rel:?} is not a book path");
        }
        let (st, _) = self.request("DELETE", &self.url_for(&parts, false), &[], Vec::new())?;
        match st {
            200 | 202 | 204 => Ok(()),
            s => Err(Self::explain(s, "Removing")),
        }
    }

    /// Free and total space when the server reports a quota (Nextcloud and ownCloud do).
    pub fn quota(&self) -> Option<Space> {
        let (st, body) = self
            .request(
                "PROPFIND",
                self.base.as_str(),
                &[("Depth", "0"), ("Content-Type", "application/xml")],
                PROPFIND_BODY.as_bytes().to_vec(),
            )
            .ok()?;
        if st != 207 {
            return None;
        }
        let text = String::from_utf8_lossy(&body);
        let doc = roxmltree::Document::parse(&text).ok()?;
        let num = |name: &str| -> Option<u64> {
            doc.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .and_then(|t| t.trim().parse::<u64>().ok())
        };
        let free = num("quota-available-bytes")?;
        let used = num("quota-used-bytes").unwrap_or(0);
        Some(Space {
            total: used.saturating_add(free),
            free,
        })
    }
}

const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:quota-available-bytes/><d:quota-used-bytes/></d:prop></d:propfind>"#;

/// `subdirs` + `file_name` as path segments, refusing anything that could leave the folder.
fn checked_parts(subdirs: &[String], file_name: &str) -> Result<Vec<String>> {
    let mut parts: Vec<String> = subdirs.to_vec();
    parts.push(file_name.to_string());
    if parts
        .iter()
        .any(|p| p.is_empty() || p == ".." || p == "." || p.contains(['/', '\\', '\0']))
    {
        bail!("refusing the path {parts:?}");
    }
    Ok(parts)
}

fn check_segments(parts: &[String]) -> Result<()> {
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || p == ".." || p == "." || p.contains(['/', '\\', '\0']))
    {
        bail!("refusing the path {parts:?}");
    }
    Ok(())
}

fn is_book(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, e)| BOOK_EXT.contains(&e.to_ascii_lowercase().as_str()))
}

/// Entries of a `207 Multi-Status` answer for the collection at `request_url`, without the
/// collection itself.
fn parse_multistatus(body: &[u8], base: &url::Url, request_url: &str) -> Result<Vec<RemoteEntry>> {
    let text = std::str::from_utf8(body).context("the server's answer is not text")?;
    let doc = roxmltree::Document::parse(text).context("the server's answer is not XML")?;
    let base_path = base.path().to_string();
    let self_path = url::Url::parse(request_url)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let mut out = Vec::new();
    for resp in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "response")
    {
        let child = |name: &str| {
            resp.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == name)
        };
        let Some(href) = child("href").and_then(|n| n.text()) else {
            continue;
        };
        let path = if href.starts_with("http://") || href.starts_with("https://") {
            url::Url::parse(href.trim())
                .map(|u| u.path().to_string())
                .unwrap_or_default()
        } else {
            href.trim().to_string()
        };
        let path = percent_decode_str(&path).decode_utf8_lossy().into_owned();
        let decoded_self = percent_decode_str(&self_path)
            .decode_utf8_lossy()
            .into_owned();
        if path.trim_end_matches('/') == decoded_self.trim_end_matches('/') {
            continue;
        }
        let decoded_base = percent_decode_str(&base_path)
            .decode_utf8_lossy()
            .into_owned();
        let Some(rel) = path.strip_prefix(&decoded_base) else {
            continue;
        };
        let is_dir = child("collection").is_some();
        let rel = rel.trim_matches('/').to_string();
        if rel.is_empty() || rel.split('/').any(|p| p == ".." || p == ".") {
            continue;
        }
        let size = child("getcontentlength")
            .and_then(|n| n.text())
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0);
        out.push(RemoteEntry { rel, is_dir, size });
    }
    Ok(out)
}

// -- as a device ------------------------------------------------------------------------------

/// A WebDAV folder behaving like a device for sending.
pub struct WebDavDriver {
    dav: WebDav,
}

impl WebDavDriver {
    pub fn new(dav: WebDav) -> WebDavDriver {
        WebDavDriver { dav }
    }

    pub fn dav(&self) -> &WebDav {
        &self.dav
    }

    /// The one "device" this driver offers; its `mount` is the web address.
    pub fn device(&self) -> DeviceInfo {
        DeviceInfo {
            id: format!("webdav:{}", self.dav.base_url()),
            name: format!(
                "Cloud folder ({})",
                self.dav.base.host_str().unwrap_or("WebDAV")
            ),
            mount: PathBuf::from(self.dav.base_url()),
        }
    }

    pub fn send_in(&self, subdirs: &[String], file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.dav
            .put_new(subdirs, file_name, bytes)
            .map(PathBuf::from)
    }

    pub fn books(&self) -> Result<Vec<DeviceBook>> {
        Ok(self
            .dav
            .list_books()?
            .into_iter()
            .map(|e| DeviceBook {
                path: PathBuf::from(&e.rel),
                rel: e.rel,
                size: e.size,
                modified: None,
            })
            .collect())
    }
}

impl DeviceDriver for WebDavDriver {
    fn id(&self) -> &'static str {
        "webdav"
    }

    fn detect(&self) -> Vec<DeviceInfo> {
        vec![self.device()]
    }

    fn preferred_formats(&self) -> &'static [&'static str] {
        &["epub"]
    }

    fn send(&self, _device: &DeviceInfo, file_name: &str, bytes: &[u8]) -> Result<PathBuf> {
        self.send_in(&[], file_name, bytes)
    }

    fn list(&self, _device: &DeviceInfo) -> Result<Vec<PathBuf>> {
        Ok(self.books()?.into_iter().map(|b| b.path).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    /// A tiny WebDAV server: files and folders in memory, basic auth, one request per connection.
    struct Dav {
        port: u16,
        files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        dirs: Arc<Mutex<Vec<String>>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    fn start(prefix: &str, auth: &str, quota: Option<(u64, u64)>) -> Dav {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let files = Arc::new(Mutex::new(BTreeMap::new()));
        let dirs = Arc::new(Mutex::new(vec![prefix.trim_end_matches('/').to_string()]));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (f2, d2, l2) = (files.clone(), dirs.clone(), log.clone());
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(auth)
        );
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (f, d, l, a) = (f2.clone(), d2.clone(), l2.clone(), auth.clone());
                std::thread::spawn(move || serve_one(s, &f, &d, &l, &a, quota));
            }
        });
        Dav {
            port,
            files,
            dirs,
            log,
        }
    }

    fn serve_one(
        mut s: TcpStream,
        files: &Mutex<BTreeMap<String, Vec<u8>>>,
        dirs: &Mutex<Vec<String>>,
        log: &Mutex<Vec<String>>,
        auth: &str,
        quota: Option<(u64, u64)>,
    ) {
        s.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            match s.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        let mut lines = head.lines();
        let first = lines.next().unwrap_or("");
        let mut it = first.split_whitespace();
        let (method, target) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
        let header = |n: &str| {
            head.lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case(n).then(|| v.trim().to_string())
                })
                .unwrap_or_default()
        };
        let len: usize = header("content-length").parse().unwrap_or(0);
        while buf.len() < head_end + len {
            match s.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let body = buf[head_end..].to_vec();
        let path = percent_decode_str(target).decode_utf8_lossy().into_owned();
        let key = path.trim_end_matches('/').to_string();
        log.lock().unwrap().push(format!("{method} {path}"));
        let reply = |s: &mut TcpStream, code: u16, ctype: &str, body: &str| {
            let _ = write!(
                s,
                "HTTP/1.1 {code} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        };
        if header("authorization") != auth {
            reply(&mut s, 401, "text/plain", "no");
            return;
        }
        let parent = key
            .rsplit_once('/')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default();
        let mut files = files.lock().unwrap();
        let mut dirs = dirs.lock().unwrap();
        match method {
            "MKCOL" => {
                if dirs.contains(&key) {
                    reply(&mut s, 405, "text/plain", "");
                } else if !dirs.contains(&parent) {
                    reply(&mut s, 409, "text/plain", "");
                } else {
                    dirs.push(key);
                    reply(&mut s, 201, "text/plain", "");
                }
            }
            "PUT" => {
                if !dirs.contains(&parent) {
                    reply(&mut s, 409, "text/plain", "");
                } else if files.contains_key(&key) && header("if-none-match") == "*" {
                    reply(&mut s, 412, "text/plain", "");
                } else {
                    files.insert(key, body);
                    reply(&mut s, 201, "text/plain", "");
                }
            }
            "GET" => match files.get(&key) {
                Some(b) => {
                    let _ = write!(
                        s,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        b.len()
                    );
                    let _ = s.write_all(b);
                }
                None => reply(&mut s, 404, "text/plain", ""),
            },
            "DELETE" => {
                if files.remove(&key).is_some() {
                    reply(&mut s, 204, "text/plain", "");
                } else {
                    reply(&mut s, 404, "text/plain", "");
                }
            }
            "PROPFIND" => {
                let mut x = String::from(r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">"#);
                let enc = |p: &str| {
                    p.split('/')
                        .map(|s| utf8_percent_encode(s, SEGMENT).to_string())
                        .collect::<Vec<_>>()
                        .join("/")
                };
                if dirs.contains(&key) {
                    let q = quota
                        .map(|(f, u)| {
                            format!("<d:quota-available-bytes>{f}</d:quota-available-bytes><d:quota-used-bytes>{u}</d:quota-used-bytes>")
                        })
                        .unwrap_or_default();
                    x.push_str(&format!(
                        "<d:response><d:href>{}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>{q}</d:prop></d:propstat></d:response>",
                        enc(&key)
                    ));
                    if header("depth") != "0" {
                        for d in dirs
                            .iter()
                            .filter(|d| d.rsplit_once('/').map(|(p, _)| p) == Some(&key))
                        {
                            x.push_str(&format!("<d:response><d:href>{}/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat></d:response>", enc(d)));
                        }
                        for (f, b) in files
                            .iter()
                            .filter(|(f, _)| f.rsplit_once('/').map(|(p, _)| p) == Some(&key))
                        {
                            x.push_str(&format!("<d:response><d:href>{}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength></d:prop></d:propstat></d:response>", enc(f), b.len()));
                        }
                    }
                    x.push_str("</d:multistatus>");
                    reply(&mut s, 207, "application/xml", &x);
                } else {
                    reply(&mut s, 404, "text/plain", "");
                }
            }
            _ => reply(&mut s, 405, "text/plain", ""),
        }
    }

    fn client(d: &Dav, prefix: &str) -> WebDav {
        WebDav::new(
            &format!("http://127.0.0.1:{}{prefix}", d.port),
            "ann",
            "secret",
        )
        .unwrap()
    }

    #[test]
    fn addresses_are_validated() {
        assert!(WebDav::new("ftp://x/y", "", "").is_err());
        assert!(WebDav::new("not a url", "", "").is_err());
        assert!(WebDav::new("https://u:p@host/dav", "", "").is_err());
        let w = WebDav::new("https://host/dav/books", "u", "p").unwrap();
        assert_eq!(w.base_url(), "https://host/dav/books/");
    }

    #[test]
    fn connecting_checks_the_password() {
        let d = start("/dav", "ann:secret", None);
        client(&d, "/dav/").check().unwrap();
        let bad =
            WebDav::new(&format!("http://127.0.0.1:{}/dav/", d.port), "ann", "wrong").unwrap();
        let e = bad.check().unwrap_err().to_string();
        assert!(e.contains("user name or password"), "{e}");
        let missing = client(&d, "/nowhere/").check().unwrap_err().to_string();
        assert!(missing.contains("does not exist"), "{missing}");
    }

    #[test]
    fn a_dead_server_is_an_error_not_a_hang() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let w = WebDav::new(&format!("http://127.0.0.1:{port}/"), "", "").unwrap();
        assert!(w
            .check()
            .unwrap_err()
            .to_string()
            .contains("could not reach"));
    }

    #[test]
    fn books_are_uploaded_into_folders_and_never_overwritten() {
        let d = start("/dav", "ann:secret", None);
        let w = client(&d, "/dav/");
        let sub = vec!["Ann Author".to_string(), "My Series".to_string()];
        let a = w.put_new(&sub, "One & Two.epub", b"first").unwrap();
        let b = w.put_new(&sub, "One & Two.epub", b"second").unwrap();
        assert_eq!(a, "Ann Author/My Series/One & Two.epub");
        assert_eq!(b, "Ann Author/My Series/One & Two (1).epub");
        let files = d.files.lock().unwrap();
        assert_eq!(files["/dav/Ann Author/My Series/One & Two.epub"], b"first");
        assert_eq!(
            files["/dav/Ann Author/My Series/One & Two (1).epub"],
            b"second"
        );
        drop(files);
        let log = d.log.lock().unwrap().join("\n");
        assert!(log.contains("MKCOL /dav/Ann Author/"), "{log}");
    }

    #[test]
    fn the_listing_finds_books_in_subfolders_and_ignores_the_rest() {
        let d = start("/dav", "ann:secret", None);
        let w = client(&d, "/dav/");
        w.put_new(&[], "top.epub", b"1234").unwrap();
        w.put_new(&["Sub".into()], "deep.azw3", b"12").unwrap();
        w.put_new(&[], "notes.docx", b"1").unwrap();
        let books = w.list_books().unwrap();
        let rels: Vec<(&str, u64)> = books.iter().map(|b| (b.rel.as_str(), b.size)).collect();
        assert_eq!(rels, vec![("Sub/deep.azw3", 2), ("top.epub", 4)]);
    }

    #[test]
    fn removing_deletes_only_books() {
        let d = start("/dav", "ann:secret", None);
        let w = client(&d, "/dav/");
        w.put_new(&[], "a.epub", b"1").unwrap();
        w.delete_book("a.epub").unwrap();
        assert!(d.files.lock().unwrap().is_empty());
        assert!(w.delete_book("a.epub").is_err(), "already gone");
        assert!(w.delete_book("../x.epub").is_err());
        assert!(w.delete_book("folder/").is_err());
        assert!(w.delete_book("notes.docx").is_err());
    }

    #[test]
    fn hostile_names_never_reach_the_server() {
        let d = start("/dav", "ann:secret", None);
        let w = client(&d, "/dav/");
        assert!(w.put_new(&[], "../evil.epub", b"1").is_err());
        assert!(w.put_new(&["..".into()], "x.epub", b"1").is_err());
        assert!(w.put_new(&["a/b".into()], "x.epub", b"1").is_err());
        assert!(d.log.lock().unwrap().is_empty());
    }

    #[test]
    fn the_quota_is_reported_when_the_server_has_one() {
        let d = start("/dav", "ann:secret", Some((900, 100)));
        let q = client(&d, "/dav/").quota().unwrap();
        assert_eq!((q.free, q.total), (900, 1000));
        let d = start("/dav", "ann:secret", None);
        assert_eq!(client(&d, "/dav/").quota(), None);
    }

    #[test]
    fn it_works_as_a_device_driver() {
        let d = start("/dav", "ann:secret", None);
        let drv = WebDavDriver::new(client(&d, "/dav/"));
        let dev = drv.detect().remove(0);
        assert!(dev.id.starts_with("webdav:http://127.0.0.1"));
        assert!(dev.name.starts_with("Cloud folder"));
        let p = drv.send(&dev, "x.epub", b"abc").unwrap();
        assert_eq!(p, PathBuf::from("x.epub"));
        assert_eq!(drv.list(&dev).unwrap(), vec![PathBuf::from("x.epub")]);
        let _ = d.dirs.lock().unwrap().len();
    }

    #[test]
    fn backups_can_be_written_replaced_read_back_and_deleted() {
        let dav = start("/remote.php/webdav", "u:p", None);
        let w = WebDav::new(
            &format!("http://127.0.0.1:{}/remote.php/webdav", dav.port),
            "u",
            "p",
        )
        .unwrap();
        let dir = vec!["Bookx backups".to_string()];
        // bigger than the usual 8 MB answer limit of the client
        let big: Vec<u8> = (0..9 * 1024 * 1024).map(|i| (i % 253) as u8).collect();
        w.put_replace(&dir, "bookx-1.db", &big).unwrap();
        w.put_replace(&dir, "bookx-1.db", b"second").unwrap();
        assert_eq!(w.get_file(&dir, "bookx-1.db", 1 << 20).unwrap(), b"second");
        w.put_replace(&dir, "bookx-2.db", &big).unwrap();
        assert_eq!(w.get_file(&dir, "bookx-2.db", 64 << 20).unwrap(), big);
        assert!(
            w.get_file(&dir, "bookx-2.db", 1 << 20).is_err(),
            "limit is honoured"
        );
        let names: Vec<String> = w
            .list_dir(&dir)
            .unwrap()
            .into_iter()
            .map(|e| e.rel)
            .collect();
        assert_eq!(names.len(), 2);
        w.delete_file(&dir, "bookx-1.db").unwrap();
        w.delete_file(&dir, "bookx-1.db").unwrap();
        assert!(w.get_file(&dir, "bookx-1.db", 1 << 20).is_err());
        assert!(w.get_file(&dir, "../x", 10).is_err());
        assert!(w.put_replace(&["..".to_string()], "x", b"").is_err());
        let wrong = WebDav::new(
            &format!("http://127.0.0.1:{}/remote.php/webdav", dav.port),
            "u",
            "bad",
        )
        .unwrap();
        assert!(wrong
            .put_replace(&dir, "x.db", b"y")
            .unwrap_err()
            .to_string()
            .contains("refused"));
    }
}
