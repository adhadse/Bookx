//! `mail::send` against an in-test fake SMTP server (a `std` TCP listener on a thread speaking
//! EHLO / STARTTLS / AUTH / MAIL / RCPT / DATA / QUIT, with real TLS from rustls). No network, no
//! shared state: every test starts its own server on port 0.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine;
use bookx_serve::mail::{send, Address, Attachment, Message, Security, SmtpConfig};
use mail_parser::MimeHeaders;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

// ------------------------------------------------------------------------------- fake server

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encryption {
    /// Cleartext only, no STARTTLS offered.
    Off,
    /// STARTTLS offered; AUTH only advertised after the upgrade.
    StartTls,
    /// TLS from the first byte.
    Implicit,
}

#[derive(Clone)]
struct Behaviour {
    tls: Encryption,
    /// Mechanisms advertised in `250-AUTH`, e.g. "PLAIN LOGIN"; empty = no AUTH.
    auth: &'static str,
    /// The password the server accepts.
    password: &'static str,
    /// Recipients answered with 550.
    reject_rcpt: &'static str,
    /// Accept the connection but never say a word.
    mute: bool,
}

impl Default for Behaviour {
    fn default() -> Behaviour {
        Behaviour {
            tls: Encryption::Off,
            auth: "",
            password: "s3cret",
            reject_rcpt: "",
            mute: false,
        }
    }
}

#[derive(Debug, Default)]
struct Seen {
    /// Every command line, AUTH payloads included (decoded where it is base64).
    commands: Vec<String>,
    ehlo: Vec<String>,
    mail_from: Option<String>,
    rcpt_to: Vec<String>,
    data: Option<Vec<u8>>,
    /// `(user, password)` received through AUTH.
    credentials: Option<(String, String)>,
    /// TLS was active when AUTH arrived / when DATA arrived.
    tls_at_auth: Option<bool>,
    tls_at_data: Option<bool>,
    quit: bool,
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
    /// Only while the upgrade to TLS is in progress.
    Taken,
}

impl Stream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(b),
            Stream::Tls(s) => s.read(b),
            Stream::Taken => Ok(0),
        }
    }
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.write_all(b),
            Stream::Tls(s) => {
                s.write_all(b)?;
                s.flush()
            }
            Stream::Taken => Ok(()),
        }
    }
}

struct Conn {
    s: Stream,
    buf: Vec<u8>,
}

impl Conn {
    fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 65536];
        match self.s.read(&mut chunk) {
            Ok(0) | Err(_) => false,
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                true
            }
        }
    }

    fn line(&mut self) -> Option<String> {
        loop {
            if let Some(i) = self.buf.windows(2).position(|w| w == b"\r\n") {
                let l: Vec<u8> = self.buf.drain(..i + 2).collect();
                return Some(String::from_utf8_lossy(&l[..i]).into_owned());
            }
            if !self.fill() {
                return None;
            }
        }
    }

    /// The message body up to the lone dot, with dot-stuffing undone.
    fn data(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.windows(5).position(|w| w == b"\r\n.\r\n") {
                let raw: Vec<u8> = self.buf.drain(..i + 5).collect();
                let mut body = raw[..i + 2].to_vec(); // keep the CRLF that ends the last line
                let mut out = Vec::with_capacity(body.len());
                let mut at_line_start = true;
                let mut k = 0;
                while k < body.len() {
                    if at_line_start && body[k] == b'.' {
                        k += 1; // drop the stuffed dot
                    }
                    if k < body.len() {
                        at_line_start = body[k] == b'\n';
                        out.push(body[k]);
                        k += 1;
                    }
                }
                body.clear();
                return Some(out);
            }
            if !self.fill() {
                return None;
            }
        }
    }

    fn say(&mut self, text: &str) {
        let _ = self.s.write_all(text.as_bytes());
    }
}

fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .unwrap_or_default()
}

fn text(b: Vec<u8>) -> String {
    String::from_utf8_lossy(&b).into_owned()
}

fn server_tls(cert: &rcgen::CertifiedKey<rcgen::KeyPair>) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
            )
            .unwrap(),
    )
}

struct Fake {
    port: u16,
    root: Vec<u8>,
    handle: JoinHandle<Seen>,
}

impl Fake {
    fn start(b: Behaviour) -> Fake {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let root = cert.cert.der().to_vec();
        let tls_cfg = server_tls(&cert);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut seen = Seen::default();
            let Ok((tcp, _)) = listener.accept() else {
                return seen;
            };
            tcp.set_read_timeout(Some(Duration::from_secs(3))).ok();
            if b.mute {
                std::thread::sleep(Duration::from_secs(2));
                return seen;
            }
            let stream = if b.tls == Encryption::Implicit {
                let c = rustls::ServerConnection::new(tls_cfg.clone()).unwrap();
                Stream::Tls(Box::new(rustls::StreamOwned::new(c, tcp)))
            } else {
                Stream::Plain(tcp)
            };
            let mut c = Conn {
                s: stream,
                buf: Vec::new(),
            };
            serve(&mut c, &b, &tls_cfg, &mut seen);
            seen
        });
        Fake { port, root, handle }
    }

    fn config(&self, host: &str, security: Security) -> SmtpConfig {
        let mut c = SmtpConfig::new(host);
        c.port = self.port;
        c.security = security;
        c.timeout = Duration::from_secs(10);
        c.extra_roots = vec![self.root.clone()];
        c.helo_name = "bookx.test".into();
        c
    }

    fn finish(self) -> Seen {
        self.handle.join().unwrap()
    }
}

fn serve(c: &mut Conn, b: &Behaviour, tls_cfg: &Arc<rustls::ServerConfig>, seen: &mut Seen) {
    let mut tls_on = b.tls == Encryption::Implicit;
    c.say("220 fake.test ESMTP ready\r\n");
    while let Some(line) = c.line() {
        let up = line.to_ascii_uppercase();
        seen.commands.push(line.clone());
        if up.starts_with("EHLO") {
            seen.ehlo.push(line[5..].to_string());
            let mut r = String::from("250-fake.test greets you\r\n");
            if b.tls == Encryption::StartTls && !tls_on {
                r.push_str("250-STARTTLS\r\n");
            }
            let auth_ok = b.tls == Encryption::Off || tls_on;
            if !b.auth.is_empty() && auth_ok {
                r.push_str(&format!("250-AUTH {}\r\n", b.auth));
            }
            r.push_str("250 8BITMIME\r\n");
            c.say(&r);
        } else if up == "STARTTLS" && b.tls == Encryption::StartTls && !tls_on {
            c.say("220 go ahead\r\n");
            let Stream::Plain(tcp) = std::mem::replace(&mut c.s, Stream::Taken) else {
                return;
            };
            let conn = rustls::ServerConnection::new(tls_cfg.clone()).unwrap();
            c.s = Stream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)));
            tls_on = true;
        } else if up.starts_with("AUTH PLAIN") {
            seen.tls_at_auth = Some(tls_on);
            let arg = line[10..].trim().to_string();
            let payload = if arg.is_empty() {
                c.say("334 \r\n");
                c.line().unwrap_or_default()
            } else {
                arg
            };
            let raw = text(b64(&payload));
            let mut parts = raw.split('\0');
            let _authzid = parts.next();
            let (u, p) = (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            );
            seen.commands.push(format!("(plain {u}:{p})"));
            finish_auth(c, b, seen, u, p);
        } else if up.starts_with("AUTH LOGIN") {
            seen.tls_at_auth = Some(tls_on);
            let user = if line.len() > 11 {
                text(b64(&line[11..]))
            } else {
                c.say("334 VXNlcm5hbWU6\r\n");
                text(b64(&c.line().unwrap_or_default()))
            };
            c.say("334 UGFzc3dvcmQ6\r\n");
            let pass = text(b64(&c.line().unwrap_or_default()));
            finish_auth(c, b, seen, user, pass);
        } else if up.starts_with("MAIL FROM:") {
            seen.mail_from = Some(angle(&line));
            c.say("250 sender ok\r\n");
        } else if up.starts_with("RCPT TO:") {
            let to = angle(&line);
            if !b.reject_rcpt.is_empty() && to == b.reject_rcpt {
                c.say("550 5.1.1 no such mailbox\r\n");
            } else {
                seen.rcpt_to.push(to);
                c.say("250 recipient ok\r\n");
            }
        } else if up == "DATA" {
            seen.tls_at_data = Some(tls_on);
            c.say("354 end with <CRLF>.<CRLF>\r\n");
            seen.data = c.data();
            c.say("250 2.0.0 queued\r\n");
        } else if up == "QUIT" {
            seen.quit = true;
            c.say("221 bye\r\n");
            return;
        } else if up == "RSET" || up == "NOOP" {
            c.say("250 ok\r\n");
        } else {
            c.say("502 not implemented\r\n");
        }
    }
}

fn finish_auth(c: &mut Conn, b: &Behaviour, seen: &mut Seen, u: String, p: String) {
    if p == b.password {
        seen.credentials = Some((u, p));
        c.say("235 2.7.0 authenticated\r\n");
    } else {
        seen.credentials = Some((u, p));
        c.say("535 5.7.8 bad credentials\r\n");
    }
}

fn angle(line: &str) -> String {
    let (a, b) = (line.find('<'), line.rfind('>'));
    match (a, b) {
        (Some(a), Some(b)) if b > a => line[a + 1..b].to_string(),
        _ => String::new(),
    }
}

// --------------------------------------------------------------------------------- fixtures

fn addr(s: &str) -> Address {
    Address::parse(s).unwrap()
}

fn message() -> Message {
    Message::new(
        addr("Reader <me@example.com>"),
        vec![addr("dev_xyz@kindle.com"), addr("Second <two@example.org>")],
        "Sent from Bookx – Größe",
        "Hello,\n.a line that starts with a dot\nbye\n",
    )
    .attach(Attachment::new(
        "Löwe.epub",
        (0..50_000u32).map(|i| (i % 251) as u8).collect(),
    ))
}

fn check_delivered(seen: &Seen, m: &Message) {
    assert_eq!(seen.mail_from.as_deref(), Some("me@example.com"));
    assert_eq!(seen.rcpt_to, ["dev_xyz@kindle.com", "two@example.org"]);
    assert!(seen.quit, "QUIT: {:?}", seen.commands);
    let data = seen.data.as_ref().expect("DATA received");
    let p = mail_parser::MessageParser::default().parse(data).unwrap();
    assert_eq!(p.subject(), Some(m.subject.as_str()));
    assert_eq!(
        p.body_text(0).unwrap().replace("\r\n", "\n").trim_end(),
        m.body.trim_end()
    );
    assert_eq!(p.attachment_count(), m.attachments.len());
    for (i, a) in m.attachments.iter().enumerate() {
        let got = p.attachment(i as u32).unwrap();
        assert_eq!(got.attachment_name(), Some(a.file_name.as_str()));
        assert_eq!(got.contents(), &a.data[..]);
    }
}

// ----------------------------------------------------------------------------------- tests

#[test]
fn cleartext_delivery_follows_the_smtp_dialogue() {
    let f = Fake::start(Behaviour::default());
    let m = message();
    send(&f.config("127.0.0.1", Security::None), &m).unwrap();
    let seen = f.finish();
    check_delivered(&seen, &m);
    assert_eq!(seen.ehlo, ["bookx.test"]);
    assert!(seen.credentials.is_none());
    // the order of the dialogue
    let verbs: Vec<String> = seen
        .commands
        .iter()
        .map(|c| c.split([' ', ':']).next().unwrap().to_ascii_uppercase())
        .collect();
    assert_eq!(verbs, ["EHLO", "MAIL", "RCPT", "RCPT", "DATA", "QUIT"]);
    assert_eq!(seen.tls_at_data, Some(false));
}

#[test]
fn dot_stuffing_and_crlf_survive_the_wire() {
    let f = Fake::start(Behaviour::default());
    let mut m = Message::new(
        addr("me@example.com"),
        vec![addr("you@example.org")],
        "dots",
        ".\n..\n.leading\nnormal\n.",
    );
    m.date = None;
    send(&f.config("127.0.0.1", Security::None), &m).unwrap();
    let seen = f.finish();
    let p = mail_parser::MessageParser::default()
        .parse(seen.data.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        p.body_text(0).unwrap().replace("\r\n", "\n").trim_end(),
        ".\n..\n.leading\nnormal\n."
    );
}

#[test]
fn starttls_then_auth_plain_sends_credentials_only_inside_tls() {
    let f = Fake::start(Behaviour {
        tls: Encryption::StartTls,
        auth: "PLAIN LOGIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::StartTls);
    cfg.credentials = Some(("reader@example.com".into(), "s3cret".into()));
    let m = message();
    send(&cfg, &m).unwrap();
    let seen = f.finish();
    check_delivered(&seen, &m);
    assert_eq!(
        seen.credentials,
        Some(("reader@example.com".into(), "s3cret".into()))
    );
    assert_eq!(seen.tls_at_auth, Some(true), "AUTH must follow STARTTLS");
    assert_eq!(seen.tls_at_data, Some(true));
    assert_eq!(seen.ehlo.len(), 2, "EHLO is repeated after STARTTLS");
    assert!(seen.commands.iter().any(|c| c == "STARTTLS"));
    assert!(seen.commands.iter().any(|c| c.starts_with("AUTH PLAIN")));
}

#[test]
fn implicit_tls_with_auth_login() {
    let f = Fake::start(Behaviour {
        tls: Encryption::Implicit,
        auth: "LOGIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::Tls);
    cfg.credentials = Some(("üser".into(), "s3cret".into()));
    let m = message();
    send(&cfg, &m).unwrap();
    let seen = f.finish();
    check_delivered(&seen, &m);
    assert_eq!(seen.credentials, Some(("üser".into(), "s3cret".into())));
    assert!(seen.commands.iter().any(|c| c.starts_with("AUTH LOGIN")));
    assert_eq!(seen.tls_at_auth, Some(true));
}

#[test]
fn cleartext_auth_to_loopback_is_allowed() {
    let f = Fake::start(Behaviour {
        auth: "PLAIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("127.0.0.1", Security::None);
    cfg.credentials = Some(("u".into(), "s3cret".into()));
    send(&cfg, &message()).unwrap();
    let seen = f.finish();
    assert_eq!(seen.credentials, Some(("u".into(), "s3cret".into())));
    assert_eq!(seen.tls_at_auth, Some(false));
}

#[test]
fn wrong_password_fails_without_leaking_it_and_sends_no_mail() {
    let f = Fake::start(Behaviour {
        tls: Encryption::Implicit,
        auth: "PLAIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::Tls);
    cfg.credentials = Some(("u".into(), "wr0ng-password".into()));
    let e = send(&cfg, &message()).unwrap_err().to_string();
    assert!(!e.contains("wr0ng-password"), "{e}");
    assert!(e.contains("failed"), "{e}");
    let seen = f.finish();
    assert!(seen.data.is_none() && seen.mail_from.is_none());
}

#[test]
fn a_rejected_recipient_is_an_error_and_nothing_is_delivered() {
    let f = Fake::start(Behaviour {
        reject_rcpt: "two@example.org",
        ..Behaviour::default()
    });
    let e = send(&f.config("127.0.0.1", Security::None), &message())
        .unwrap_err()
        .to_string();
    assert!(e.contains("failed"), "{e}");
    let seen = f.finish();
    assert!(seen.data.is_none());
    assert_eq!(seen.rcpt_to, ["dev_xyz@kindle.com"]);
}

#[test]
fn starttls_is_required_not_opportunistic() {
    // the server does not offer STARTTLS: a StartTls client must not carry on in cleartext
    let f = Fake::start(Behaviour {
        tls: Encryption::Off,
        auth: "PLAIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::StartTls);
    cfg.credentials = Some(("u".into(), "s3cret".into()));
    assert!(send(&cfg, &message()).is_err());
    let seen = f.finish();
    assert!(seen.credentials.is_none(), "password sent in cleartext!");
    assert!(seen.data.is_none() && seen.mail_from.is_none());
}

#[test]
fn an_untrusted_certificate_is_refused_before_any_credentials_are_sent() {
    let f = Fake::start(Behaviour {
        tls: Encryption::StartTls,
        auth: "PLAIN",
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::StartTls);
    cfg.extra_roots.clear(); // the self-signed certificate is not trusted
    cfg.credentials = Some(("u".into(), "s3cret".into()));
    assert!(send(&cfg, &message()).is_err());
    let seen = f.finish();
    assert!(seen.credentials.is_none());
    assert!(seen.data.is_none());
}

#[test]
fn a_certificate_for_another_host_is_refused() {
    let f = Fake::start(Behaviour {
        tls: Encryption::Implicit,
        ..Behaviour::default()
    });
    // the certificate says "localhost"; we ask for another name that resolves locally
    let mut cfg = f.config("localhost", Security::Tls);
    cfg.host = "127.0.0.1".into();
    assert!(send(&cfg, &message()).is_err());
    let seen = f.finish();
    assert!(seen.data.is_none());
}

#[test]
fn refused_connection_is_reported() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut cfg = SmtpConfig::new("127.0.0.1");
    cfg.port = port;
    cfg.security = Security::None;
    cfg.timeout = Duration::from_secs(3);
    let e = send(&cfg, &message()).unwrap_err().to_string();
    assert!(e.contains(&format!("127.0.0.1:{port}")), "{e}");
}

#[test]
fn a_silent_server_times_out() {
    let f = Fake::start(Behaviour {
        mute: true,
        ..Behaviour::default()
    });
    let mut cfg = f.config("127.0.0.1", Security::None);
    cfg.timeout = Duration::from_millis(300);
    let started = Instant::now();
    assert!(send(&cfg, &message()).is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    f.finish();
}

#[test]
fn unencrypted_password_to_a_remote_host_is_refused_without_connecting() {
    let mut cfg = SmtpConfig::new("192.0.2.1"); // TEST-NET-1: nothing answers
    cfg.security = Security::None;
    cfg.credentials = Some(("u".into(), "p".into()));
    cfg.timeout = Duration::from_millis(200);
    let started = Instant::now();
    let e = send(&cfg, &message()).unwrap_err().to_string();
    assert!(e.contains("without encryption"), "{e}");
    assert!(started.elapsed() < Duration::from_millis(150));
}

#[test]
fn header_injection_never_reaches_the_wire() {
    let f = Fake::start(Behaviour::default());
    let mut m = message();
    m.subject = "Hi\r\nBcc: evil@example.net".into();
    assert!(send(&f.config("127.0.0.1", Security::None), &m).is_err());
    // the server saw a connection at most, never a message
    let seen = f.handle_after_drop();
    assert!(seen.data.is_none() && seen.mail_from.is_none());
}

#[test]
fn a_large_attachment_goes_through() {
    let f = Fake::start(Behaviour::default());
    let data: Vec<u8> = (0..8 * 1024 * 1024u32)
        .map(|i| (i ^ (i >> 9)) as u8)
        .collect();
    let m = Message::new(addr("a@b.c"), vec![addr("x@y.z")], "big", "see attachment")
        .attach(Attachment::new("big.pdf", data.clone()));
    send(&f.config("127.0.0.1", Security::None), &m).unwrap();
    let seen = f.finish();
    let p = mail_parser::MessageParser::default()
        .parse(seen.data.as_ref().unwrap())
        .unwrap();
    assert!(p.attachment(0).unwrap().contents() == &data[..]);
}

impl Fake {
    /// For tests where the client never connects: unblocks the accept and collects what was seen.
    fn handle_after_drop(self) -> Seen {
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        self.handle.join().unwrap()
    }
}

#[test]
fn root_certificates_are_der_not_pem() {
    let f = Fake::start(Behaviour {
        tls: Encryption::Implicit,
        ..Behaviour::default()
    });
    let mut cfg = f.config("localhost", Security::Tls);
    cfg.extra_roots =
        vec![b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".to_vec()];
    let e = format!("{:#}", send(&cfg, &message()).unwrap_err());
    assert!(e.contains("certificate"), "{e}");
    f.handle_after_drop();
    let _ = CertificateDer::from(vec![0u8]);
}
