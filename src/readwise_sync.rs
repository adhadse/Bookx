//! Sending highlights to Readwise (`POST /api/v2/highlights/`).
//!
//! * The token comes from the settings (kept in the system keyring through `bookx-secrets`, see
//!   `Config::secret_fields`). It is only ever put into the `Authorization` header: no error
//!   text, log line or `Debug` output contains it.
//! * **Incremental**: a [`Ledger`] remembers what was sent (highlight id → fingerprint of quote and
//!   note, saved next to the settings). Only new or edited highlights are sent again; the ledger is
//!   saved after every accepted batch, so an interrupted run continues where it stopped.
//! * Batches of 100 highlights per request (the API has no stated limit; this keeps requests
//!   small); progress and cancel go through callbacks.
//! * The HTTP is `bookx_serve::httpc` (the client already used for WebDAV); the [`Poster`] trait
//!   lets the tests talk to a local fake server or a closure.

use crate::i18n::ntr;
use crate::i18n::tr;
use crate::i18n::trf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use bookx_lookup::export::{self as ex, AnnotationItem, ExportOptions, HighlightStyle};
use bookx_lookup::ids;
use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str = "https://readwise.io/api/v2/highlights/";
pub const BATCH: usize = 100;

/// What has been sent: highlight id → fingerprint of what was sent.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    sent: HashMap<String, String>,
}

pub fn fingerprint(i: &AnnotationItem) -> String {
    ids::content_id(
        "fp",
        i.note.as_deref(),
        &format!("{}\u{1}{}", i.text.trim(), i.created.timestamp()),
    )
}

impl Ledger {
    pub fn load(path: &Path) -> Ledger {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sent.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }
}

/// Where the ledger lives.
pub fn ledger_path() -> Option<PathBuf> {
    std::env::var_os("BOOKX_READWISE_LEDGER")
        .map(PathBuf::from)
        .or_else(|| {
            directories::ProjectDirs::from("com", "adhadse", "Bookx")
                .map(|d| d.config_dir().join("readwise-sent.json"))
        })
}

/// The address to post to (`$BOOKX_READWISE_URL` for tests, else Readwise).
pub fn endpoint() -> String {
    std::env::var("BOOKX_READWISE_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string())
}

/// The items that still have to be sent, with their ids: highlights with text that are not in the
/// ledger or changed since.
pub fn pending(items: &[AnnotationItem], ledger: &Ledger) -> Vec<(String, AnnotationItem)> {
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .filter(|i| i.style != HighlightStyle::Bookmark && !i.text.trim().is_empty())
        .filter_map(|i| {
            let id = ids::item_id(i);
            if !seen.insert(id.clone()) {
                return None;
            }
            (ledger.sent.get(&id) != Some(&fingerprint(i))).then(|| (id, i.clone()))
        })
        .collect()
}

pub struct Reply {
    pub status: u16,
    pub body: String,
}

pub trait Poster {
    fn post(&self, url: &str, token: &str, body: &str) -> Result<Reply>;
}

/// The real client.
pub struct Http;

impl Poster for Http {
    fn post(&self, url: &str, token: &str, body: &str) -> Result<Reply> {
        let u = url::Url::parse(url).map_err(|_| anyhow!("the Readwise address is not valid"))?;
        let auth = format!("Token {token}");
        let r = bookx_serve::httpc::request(
            "POST",
            &u,
            &[
                ("Authorization", auth.as_str()),
                ("Content-Type", "application/json"),
            ],
            body.as_bytes(),
            Duration::from_secs(30),
        )
        .map_err(|e| {
            anyhow!(
                "could not reach Readwise: {}",
                scrub(&format!("{e:#}"), token)
            )
        })?;
        Ok(Reply {
            status: r.status,
            body: String::from_utf8_lossy(&r.body).into_owned(),
        })
    }
}

/// `text` without `secret`.
pub fn scrub(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "…")
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub sent: usize,
    pub batches: usize,
    pub already: usize,
}

/// Sends what is pending. `progress` gets (done, total) after each accepted batch; `cancelled` is
/// asked before each batch. The ledger is saved to `ledger_file` after each batch.
pub fn send(
    poster: &dyn Poster,
    token: &str,
    url: &str,
    items: &[AnnotationItem],
    ledger: &mut Ledger,
    ledger_file: Option<&Path>,
    progress: &mut dyn FnMut(usize, usize),
    cancelled: &dyn Fn() -> bool,
) -> Result<Outcome> {
    if token.trim().is_empty() {
        bail!("There is no Readwise access token yet");
    }
    let todo = pending(items, ledger);
    let total_hl = items
        .iter()
        .filter(|i| i.style != HighlightStyle::Bookmark && !i.text.trim().is_empty())
        .count();
    let mut out = Outcome {
        already: total_hl.saturating_sub(todo.len()),
        ..Default::default()
    };
    let total = todo.len();
    for chunk in todo.chunks(BATCH) {
        if cancelled() {
            bail!("Cancelled after {} of {total}", out.sent);
        }
        let batch: Vec<AnnotationItem> = chunk.iter().map(|(_, i)| i.clone()).collect();
        let body = ex::readwise_payload(&batch, &ExportOptions::default()).to_string();
        let r = poster.post(url, token.trim(), &body)?;
        match r.status {
            200..=299 => {}
            401 | 403 => {
                bail!("Readwise did not accept the access token (check it in Preferences)")
            }
            429 => bail!(
                "Readwise is limiting requests; {} of {total} were sent, try again in a minute",
                out.sent
            ),
            s => bail!(
                "Readwise answered {s}: {}",
                scrub(r.body.chars().take(200).collect::<String>().trim(), token)
            ),
        }
        for (id, i) in chunk {
            ledger.sent.insert(id.clone(), fingerprint(i));
        }
        if let Some(f) = ledger_file {
            ledger.save(f)?;
        }
        out.sent += chunk.len();
        out.batches += 1;
        progress(out.sent, total);
    }
    Ok(out)
}

/// A line for the toast after [`send`].
pub fn summary(o: &Outcome) -> String {
    match (o.sent, o.already) {
        (0, 0) => tr("No highlights to send"),
        (0, a) => format!("Readwise is up to date ({a} already sent)"),
        (s, _) => trf(
            "Sent {} to Readwise",
            &[&ntr("{} highlight", "{} highlights", s)],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn item(text: &str, note: Option<&str>) -> AnnotationItem {
        AnnotationItem {
            book_title: "Dune".into(),
            book_author: Some("Frank Herbert".into()),
            chapter: None,
            text: text.into(),
            note: note.map(str::to_string),
            style: HighlightStyle::Highlight,
            color: None,
            created: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            location: None,
        }
    }

    fn many(n: usize) -> Vec<AnnotationItem> {
        (0..n)
            .map(|i| item(&format!("quote number {i}"), None))
            .collect()
    }

    /// A fake Readwise on a local port: answers every request with `status`, records
    /// (authorization header, body).
    fn fake(status: u16) -> (String, Arc<Mutex<Vec<(String, String)>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/v2/highlights/", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for mut c in l.incoming().flatten() {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let (head_end, len) = loop {
                    let n = c.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break (0, 0);
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (p + 4, len);
                    }
                };
                while buf.len() < head_end + len {
                    let n = c.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let auth = head
                    .lines()
                    .find(|l| l.to_lowercase().starts_with("authorization:"))
                    .unwrap_or("")
                    .to_string();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                s2.lock().unwrap().push((auth, body));
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]"
                );
                let _ = c.write_all(reply.as_bytes());
            }
        });
        (url, seen)
    }

    fn run(
        url: &str,
        token: &str,
        items: &[AnnotationItem],
        ledger: &mut Ledger,
    ) -> Result<Outcome> {
        send(
            &Http,
            token,
            url,
            items,
            ledger,
            None,
            &mut |_, _| {},
            &|| false,
        )
    }

    #[test]
    fn highlights_go_out_in_batches_with_the_token_in_the_header_and_the_ledger_remembers() {
        let (url, seen) = fake(200);
        let items = many(250);
        let mut ledger = Ledger::default();
        let calls = RefCell::new(Vec::new());
        let o = send(
            &Http,
            "sekret-token",
            &url,
            &items,
            &mut ledger,
            None,
            &mut |d, t| calls.borrow_mut().push((d, t)),
            &|| false,
        )
        .unwrap();
        assert_eq!((o.sent, o.batches), (250, 3));
        assert_eq!(*calls.borrow(), vec![(100, 250), (200, 250), (250, 250)]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert!(seen[0].0.to_lowercase().contains("token sekret-token"));
        let v: serde_json::Value = serde_json::from_str(&seen[2].1).unwrap();
        assert_eq!(v["highlights"].as_array().unwrap().len(), 50);
        assert_eq!(v["highlights"][0]["title"], "Dune");
        drop(seen);
        let again = run(&url, "sekret-token", &items, &mut ledger).unwrap();
        assert_eq!(
            (again.sent, again.already),
            (0, 250),
            "nothing is sent twice"
        );
    }

    #[test]
    fn an_edited_note_is_sent_again_and_only_that() {
        let (url, seen) = fake(200);
        let mut ledger = Ledger::default();
        run(&url, "t", &many(3), &mut ledger).unwrap();
        let mut items = many(3);
        items[1].note = Some("now with a note".into());
        let o = run(&url, "t", &items, &mut ledger).unwrap();
        assert_eq!(o.sent, 1);
        let seen = seen.lock().unwrap();
        assert!(seen[1].1.contains("now with a note") && !seen[1].1.contains("number 0"));
    }

    #[test]
    fn a_rejected_token_stops_without_marking_anything_sent_and_never_shows_the_token() {
        let (url, _) = fake(401);
        let mut ledger = Ledger::default();
        let e = run(&url, "sekret-token", &many(2), &mut ledger).unwrap_err();
        let msg = format!("{e:#}");
        assert!(msg.contains("did not accept the access token"));
        assert!(!msg.contains("sekret-token"));
        assert!(ledger.is_empty());
    }

    #[test]
    fn a_server_error_and_a_rate_limit_are_reported_and_progress_is_kept() {
        struct Flaky(Mutex<u32>);
        impl Poster for Flaky {
            fn post(&self, _: &str, _: &str, _: &str) -> Result<Reply> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                Ok(Reply {
                    status: if *n == 1 { 200 } else { 429 },
                    body: String::new(),
                })
            }
        }
        let mut ledger = Ledger::default();
        let e = send(
            &Flaky(Mutex::new(0)),
            "t",
            "http://x",
            &many(150),
            &mut ledger,
            None,
            &mut |_, _| {},
            &|| false,
        )
        .unwrap_err();
        assert!(format!("{e}").contains("100 of 150 were sent"));
        assert_eq!(ledger.len(), 100, "the accepted batch stays remembered");
        let (url, _) = fake(500);
        let e = run(&url, "t", &many(1), &mut Ledger::default()).unwrap_err();
        assert!(format!("{e}").contains("answered 500"));
    }

    #[test]
    fn an_unreachable_server_error_does_not_contain_the_token() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x/sekret-token", l.local_addr().unwrap());
        drop(l);
        let e = run(&url, "sekret-token", &many(1), &mut Ledger::default()).unwrap_err();
        assert!(!format!("{e:#}").contains("sekret-token"));
        assert_eq!(scrub("a sekret b", "sekret"), "a … b");
    }

    #[test]
    fn cancel_stops_before_the_next_batch_and_a_missing_token_is_refused() {
        let (url, seen) = fake(200);
        let mut ledger = Ledger::default();
        let e = send(
            &Http,
            "t",
            &url,
            &many(5),
            &mut ledger,
            None,
            &mut |_, _| {},
            &|| true,
        )
        .unwrap_err();
        assert!(format!("{e}").contains("Cancelled"));
        assert!(seen.lock().unwrap().is_empty());
        let e = run(&url, "  ", &many(1), &mut ledger).unwrap_err();
        assert!(format!("{e}").contains("no Readwise access token"));
    }

    #[test]
    fn bookmarks_and_empty_quotes_are_not_pending_and_the_ledger_survives_a_restart() {
        let mut bm = item("p. 3", None);
        bm.style = HighlightStyle::Bookmark;
        let items = vec![
            bm,
            item("  ", None),
            item("real quote", None),
            item("real quote", Some("dup")),
        ];
        assert_eq!(pending(&items, &Ledger::default()).len(), 1);
        let dir = std::env::temp_dir().join(format!("bookx-rw-{}", uuid::Uuid::new_v4()));
        let f = dir.join("sent.json");
        let (url, _) = fake(200);
        let mut l = Ledger::default();
        send(
            &Http,
            "t",
            &url,
            &items,
            &mut l,
            Some(&f),
            &mut |_, _| {},
            &|| false,
        )
        .unwrap();
        let back = Ledger::load(&f);
        assert_eq!(back, l);
        assert!(pending(&items[2..3], &back).is_empty());
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(
            summary(&Outcome {
                sent: 1,
                ..Default::default()
            }),
            "Sent 1 highlight to Readwise"
        );
        assert!(
            summary(&Outcome {
                already: 4,
                ..Default::default()
            })
            .contains("up to date")
        );
    }
}
