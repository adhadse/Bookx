//! SSRF guard for every URL that did not come from the user's own configuration.
//!
//! A feed, a book, a redirect or a metadata record can name `http://169.254.169.254/` or
//! `http://192.168.1.1/admin`. [`UreqFetcher`](crate::fetch::UreqFetcher) therefore resolves names
//! itself ([`GuardedResolver`], plugged into ureq) and connects only to addresses that
//! [`is_blocked_ip`] accepts. Because the check is on the very addresses the socket connects to, it
//! holds for redirects (each hop is resolved again) and against DNS rebinding (there is no second
//! lookup between the check and the connection). A host the user typed or configured (an OPDS
//! catalogue on the LAN, the address of an article to import) is registered with [`trust_host`] and
//! may be private.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Mutex, OnceLock};

use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

/// Redirects followed at most.
pub const MAX_REDIRECTS: u32 = 5;

fn trusted() -> &'static Mutex<HashSet<String>> {
    static T: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Lower-cased host without brackets, port or trailing dot.
pub fn normalize_host(host: &str) -> String {
    let h = host.trim();
    let h = h
        .strip_prefix('[')
        .and_then(|h| h.split(']').next())
        .unwrap_or(h);
    h.trim_end_matches('.').to_ascii_lowercase()
}

/// The host of a URL string, for [`trust_host`] and tests. `None` when it has no host.
pub fn host_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url.trim()).ok()?;
    u.host_str().map(normalize_host).filter(|h| !h.is_empty())
}

/// The user configured or typed this host: it may resolve to a private address.
pub fn trust_host(host: &str) {
    let h = normalize_host(host);
    if !h.is_empty() {
        trusted()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(h);
    }
}

/// [`trust_host`] for the host of a URL the user configured or typed.
pub fn trust_url(url: &str) {
    if let Some(h) = host_of(url) {
        trust_host(&h);
    }
}

pub fn is_trusted(host: &str) -> bool {
    trusted()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(&normalize_host(host))
}

fn v4_blocked(a: Ipv4Addr) -> bool {
    let o = a.octets();
    a.is_loopback()
        || a.is_unspecified()
        || a.is_broadcast()
        || a.is_multicast()
        || a.is_private()
        || a.is_link_local()
        || o[0] == 0
        // 100.64.0.0/10 shared address space (carrier NAT, Alibaba metadata 100.100.100.200)
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        // 192.0.0.0/24 IETF protocol assignments, 192.0.2/24, 198.51.100/24, 203.0.113/24 documentation
        || (o[0] == 192 && o[1] == 0 && (o[2] == 0 || o[2] == 2))
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        // 198.18.0.0/15 benchmarking
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
        // 240.0.0.0/4 reserved
        || o[0] >= 240
}

fn v6_blocked(a: Ipv6Addr) -> bool {
    if a.is_loopback() || a.is_unspecified() || a.is_multicast() {
        return true;
    }
    let s = a.segments();
    // IPv4 inside IPv6: mapped (::ffff:a.b.c.d), compatible (::a.b.c.d), NAT64 (64:ff9b::/96)
    let embedded =
        |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if s[..5] == [0, 0, 0, 0, 0] && (s[5] == 0xffff || s[5] == 0) {
        return v4_blocked(embedded(s[6], s[7]));
    }
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return v4_blocked(embedded(s[6], s[7]));
    }
    // 6to4 (2002::/16) carries an IPv4 address in bits 16..48
    if s[0] == 0x2002 {
        return v4_blocked(embedded(s[1], s[2]));
    }
    // Teredo 2001::/32 (the client address is obfuscated: refuse the whole prefix)
    if s[0] == 0x2001 && s[1] == 0 {
        return true;
    }
    (s[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local, incl. AWS fd00:ec2::254
        || (s[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (s[0] & 0xffc0) == 0xfec0 // fec0::/10 site-local (deprecated)
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || s[0] == 0x0100 && s[1..4] == [0, 0, 0] // 100::/64 discard
}

/// Loopback, link-local (including the 169.254.169.254 metadata address), private, shared,
/// documentation, multicast, unspecified and reserved ranges, for IPv4, IPv6 and IPv4 hidden in IPv6.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => v4_blocked(a),
        IpAddr::V6(a) => v6_blocked(a),
    }
}

/// Which of `addrs` may be connected to for `host`: all of them for a trusted host, only the
/// public ones otherwise.
pub fn filter_addrs<T: Copy>(host: &str, addrs: &[T], ip_of: impl Fn(&T) -> IpAddr) -> Vec<T> {
    if is_trusted(host) {
        return addrs.to_vec();
    }
    addrs
        .iter()
        .filter(|a| !is_blocked_ip(ip_of(a)))
        .copied()
        .collect()
}

/// ureq resolver that refuses to hand out private addresses. A configured proxy is trusted: the
/// name server on its far side is the proxy's business, and a proxy on `localhost` is common.
#[derive(Debug, Default)]
pub struct GuardedResolver {
    inner: DefaultResolver,
}

impl Resolver for GuardedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = self.inner.resolve(uri, config, timeout)?;
        let host = uri.host().unwrap_or("");
        if config
            .proxy()
            .is_some_and(|p| normalize_host(p.host()) == normalize_host(host))
        {
            return Ok(all);
        }
        let ok = filter_addrs(host, &all, |a| a.ip());
        let mut out = self.inner.empty();
        for a in ok {
            out.push(a);
        }
        if out.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "{} is a private or local address; Bookx only connects there to a host you configured",
                    normalize_host(host)
                ),
            )));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(s: &str) -> bool {
        is_blocked_ip(s.parse().unwrap())
    }

    #[test]
    fn private_local_and_metadata_addresses_are_blocked() {
        for ip in [
            "127.0.0.1",
            "127.255.255.254",
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.169.254",
            "169.254.0.1",
            "100.64.0.1",
            "100.100.100.200",
            "100.127.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fd00:ec2::254",
            "fdff::1",
            "fe80::1",
            "febf::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.1.2.3",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::1",
            "2002:a9fe:a9fe::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:db8::1",
        ] {
            assert!(blocked(ip), "{ip} must be blocked");
        }
    }

    #[test]
    fn public_addresses_pass() {
        for ip in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.216.34",
            "172.15.255.255",
            "172.32.0.1",
            "100.63.255.255",
            "100.128.0.1",
            "169.253.1.1",
            "192.169.0.1",
            "198.17.0.1",
            "198.20.0.1",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2a00:1450:4001::1",
            "::ffff:8.8.8.8",
            "2002:0808:0808::1",
            "64:ff9b::808:808",
        ] {
            assert!(!blocked(ip), "{ip} must pass");
        }
    }

    #[test]
    fn a_trusted_host_may_be_private_everything_else_may_not() {
        let addrs = [
            "10.0.0.5".parse::<IpAddr>().unwrap(),
            "8.8.8.8".parse().unwrap(),
        ];
        assert_eq!(
            filter_addrs("evil-guard-test.example", &addrs, |a| *a),
            vec![addrs[1]],
            "only the public one of a mixed answer is kept"
        );
        trust_url("http://My-NAS.local.:8080/opds");
        assert!(is_trusted("my-nas.local"));
        assert!(is_trusted("MY-NAS.LOCAL."));
        assert_eq!(filter_addrs("my-nas.local", &addrs, |a| *a), addrs.to_vec());
        assert!(!is_trusted("other-nas.local"));
        trust_host("[::1]");
        assert!(is_trusted("::1"));
    }

    #[test]
    fn hosts_are_normalised() {
        assert_eq!(normalize_host(" Example.COM. "), "example.com");
        assert_eq!(normalize_host("[FE80::1]"), "fe80::1");
        assert_eq!(
            host_of("https://User@Example.com:8443/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("http://[::1]:80/").as_deref(), Some("::1"));
        assert_eq!(host_of("not a url"), None);
    }
}
