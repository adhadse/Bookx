//! A [`Fetcher`] that tells the flight recorder what it did: the scheme, the kind of host (never the
//! host), the HTTP status or the class of the failure, and how long it took. Wrap the real fetcher of a
//! feature with [`traced`]; the feature's code does not change.

use std::sync::Arc;

use anyhow::Result;
use bookx_sources::fetch::Fetcher;

use crate::trace::{Area, Op};

/// The kind of host in a URL, as a word: `ip_private`, `ip_public`, `localhost`, `hostname`, `domain`.
pub fn host_kind(url: &str) -> &'static str {
    let class = crate::diag_addr::host_class(
        url.split_once("://")
            .map_or(url, |(_, r)| r)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .rsplit('@')
            .next()
            .unwrap_or("")
            .rsplit_once(':')
            .map_or_else(
                || url.split_once("://").map_or(url, |(_, r)| r).split(['/', '?', '#']).next().unwrap_or(""),
                |(h, _)| h,
            ),
    );
    match class.as_str() {
        "<ip:private>" | "<ip:cgnat>" | "<ip6:private>" => "ip_private",
        "<ip:loopback>" | "<ip6:loopback>" | "localhost" => "localhost",
        c if c.starts_with("<ip") => "ip_public",
        "<hostname:local>" | "<hostname>" => "hostname",
        _ => "domain",
    }
}

fn start(area: Area, method: &'static str, url: &str) -> Op {
    let mut t = crate::trace::op(area, "request");
    t.word("method", method)
        .word("scheme", if url.starts_with("https://") { "https" } else { "http" })
        .word("host", host_kind(url));
    t
}

struct Traced {
    inner: Arc<dyn Fetcher>,
    area: Area,
}

impl Fetcher for Traced {
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        let mut t = start(self.area, "get", url);
        let r = self.inner.get(url, headers);
        match &r {
            Ok(b) => {
                t.size_class("body", b.len() as u64).ok();
            }
            Err(e) => t.fail(&format!("{e:#}")),
        }
        r
    }

    fn post(&self, url: &str, headers: &[(String, String)], content_type: &str, body: &[u8]) -> Result<Vec<u8>> {
        let mut t = start(self.area, "post", url);
        let r = self.inner.post(url, headers, content_type, body);
        match &r {
            Ok(b) => {
                t.size_class("body", b.len() as u64).ok();
            }
            Err(e) => t.fail(&format!("{e:#}")),
        }
        r
    }
}

/// `inner`, recording every request under `area`.
pub fn traced(inner: Arc<dyn Fetcher>, area: Area) -> Arc<dyn Fetcher> {
    Arc::new(Traced { inner, area })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_sources::fetch::{MockFetcher, MockReply};

    #[test]
    fn hosts_are_only_told_by_kind() {
        for (u, k) in [
            ("http://192.168.1.5:30260/api", "ip_private"),
            ("https://203.0.113.9/x", "ip_public"),
            ("http://localhost:8080", "localhost"),
            ("http://nas/opds", "hostname"),
            ("https://nas.local:8443/", "hostname"),
            ("https://books.example.org/opds?x=1", "domain"),
            ("https://u:p@books.example.org:8443/opds", "domain"),
        ] {
            assert_eq!(host_kind(u), k, "{u}");
        }
    }

    #[test]
    fn a_traced_request_records_the_status_and_not_the_address() {
        let inner: Arc<dyn Fetcher> = Arc::new(MockFetcher::new().on("https://books.example.org/opds", MockReply::Status(401)));
        let f = traced(inner, Area::Opds);
        let r = f.get("https://books.example.org/opds", &[]);
        assert!(r.is_err());
        let e = crate::trace::snapshot()
            .into_iter()
            .rev()
            .find(|e| e.area == Area::Opds && e.name == "request")
            .unwrap();
        assert!(!e.ok);
        assert_eq!(e.field("host"), Some("domain"));
        assert_eq!(e.field("scheme"), Some("https"));
        assert!(!e.to_line().contains("books.example.org"), "{}", e.to_line());
        assert!(e.class.is_some());
    }
}
