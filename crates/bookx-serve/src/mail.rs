//! E-mail for "send to Kindle" (or any recipient): builds the RFC 5322 / MIME message and sends it
//! over SMTP. Both jobs are done by [`lettre`]; this module adds Bookx's validation (addresses,
//! header injection, boundaries, the Send-to-Kindle size limit) and a small, explicit
//! configuration for the SMTP connection.
//!
//! * [`build`] returns the message as 7-bit ASCII bytes with CRLF line ends (non-ASCII subjects,
//!   display names and file names use RFC 2047 / RFC 2231 encoding).
//! * [`send`] delivers a [`Message`] to an SMTP server: plain (only for loopback hosts or when
//!   explicitly allowed), `STARTTLS` or implicit TLS, with `AUTH PLAIN` / `AUTH LOGIN`.
//!   Credentials are never sent over an unencrypted connection to a non-loopback host.

use std::net::IpAddr;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use lettre::message::header::ContentType;
use lettre::message::{Attachment as MimeAttachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Certificate, Tls, TlsParameters};
use lettre::{SmtpTransport, Transport};
use uuid::Uuid;

/// Amazon rejects attachments larger than this per e-mail.
pub const KINDLE_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// An e-mail address with an optional display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || e.len() > 254 || domain.contains('@') {
        return false;
    }
    let ok = |c: char| {
        c.is_ascii_graphic()
            && !matches!(
                c,
                '<' | '>' | '"' | '\\' | ',' | ';' | ':' | '(' | ')' | '[' | ']'
            )
    };
    e.chars().all(ok)
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && c != '\t')
}

impl Address {
    pub fn new(email: &str) -> Result<Address> {
        let email = email.trim();
        if !valid_email(email) {
            bail!("invalid e-mail address: {email:?}");
        }
        Ok(Address {
            name: None,
            email: email.to_string(),
        })
    }

    /// Sets the display name (empty after trimming means none). Control characters are refused
    /// when the message is built.
    pub fn with_name(mut self, name: &str) -> Address {
        let n = name.trim();
        self.name = (!n.is_empty()).then(|| n.to_string());
        self
    }

    /// Accepts `a@b.c`, `Name <a@b.c>` and `"Quoted, Name" <a@b.c>`.
    pub fn parse(s: &str) -> Result<Address> {
        let s = s.trim();
        if has_control(s) {
            bail!("invalid e-mail address: control characters");
        }
        if let (Some(open), true) = (s.rfind('<'), s.ends_with('>')) {
            let email = &s[open + 1..s.len() - 1];
            let name = s[..open].trim();
            let name = name
                .strip_prefix('"')
                .and_then(|n| n.strip_suffix('"'))
                .map(|n| n.replace("\\\"", "\"").replace("\\\\", "\\"))
                .unwrap_or_else(|| name.to_string());
            return Ok(Address::new(email)?.with_name(&name));
        }
        Address::new(s)
    }

    pub fn domain(&self) -> &str {
        self.email
            .rsplit_once('@')
            .map(|(_, d)| d)
            .unwrap_or("localhost")
    }

    fn mailbox(&self) -> Result<Mailbox> {
        if !valid_email(&self.email) {
            bail!("invalid e-mail address: {:?}", self.email);
        }
        if self.name.as_deref().is_some_and(has_control) {
            bail!("invalid display name: control characters");
        }
        let addr = self
            .email
            .parse::<lettre::Address>()
            .map_err(|e| anyhow!("invalid e-mail address {:?}: {e}", self.email))?;
        Ok(Mailbox::new(self.name.clone(), addr))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub file_name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

impl Attachment {
    /// MIME type is guessed from the extension.
    pub fn new(file_name: &str, data: Vec<u8>) -> Attachment {
        Attachment {
            file_name: file_name.to_string(),
            mime: attachment_mime(file_name).to_string(),
            data,
        }
    }
}

/// MIME type for book attachments.
pub fn attachment_mime(file_name: &str) -> &'static str {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "epub" => "application/epub+zip",
        "mobi" => "application/x-mobipocket-ebook",
        "azw" | "azw3" => "application/vnd.amazon.ebook",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "html" | "htm" => "text/html",
        "rtf" => "application/rtf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub from: Address,
    pub to: Vec<Address>,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<Attachment>,
    /// Defaults to now.
    pub date: Option<DateTime<Utc>>,
    /// Defaults to a random `<uuid@domain-of-from>`.
    pub message_id: Option<String>,
    /// Defaults to a random boundary. Must be a valid MIME boundary.
    pub boundary: Option<String>,
}

impl Message {
    pub fn new(from: Address, to: Vec<Address>, subject: &str, body: &str) -> Message {
        Message {
            from,
            to,
            subject: subject.to_string(),
            body: body.to_string(),
            attachments: Vec::new(),
            date: None,
            message_id: None,
            boundary: None,
        }
    }

    pub fn attach(mut self, a: Attachment) -> Message {
        self.attachments.push(a);
        self
    }

    /// Total attachment size.
    pub fn attachment_bytes(&self) -> u64 {
        self.attachments.iter().map(|a| a.data.len() as u64).sum()
    }

    /// Fails if the attachments would be rejected by Amazon's 50 MB limit.
    pub fn check_kindle_limit(&self) -> Result<()> {
        let n = self.attachment_bytes();
        if n > KINDLE_MAX_BYTES {
            bail!(
                "attachments are {} MB, the Send-to-Kindle limit is {} MB",
                n.div_ceil(1024 * 1024),
                KINDLE_MAX_BYTES / (1024 * 1024)
            );
        }
        Ok(())
    }
}

fn valid_boundary(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 70
        && !b.ends_with(' ')
        && b.chars()
            .all(|c| c.is_ascii_alphanumeric() || "'()+_,-./:=? ".contains(c))
}

fn valid_mime(m: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$&-^_.+".contains(c))
    };
    m.split_once('/').is_some_and(|(t, s)| token(t) && token(s))
}

fn to_lettre(msg: &Message) -> Result<lettre::Message> {
    if msg.to.is_empty() {
        bail!("message has no recipient");
    }
    if has_control(&msg.subject) {
        bail!("invalid subject: control characters");
    }
    let id = match &msg.message_id {
        Some(m) => m.trim().to_string(),
        None => format!("<{}@{}>", Uuid::new_v4().simple(), msg.from.domain()),
    };
    let inner = id.strip_prefix('<').and_then(|i| i.strip_suffix('>'));
    if inner.is_none_or(|i| i.is_empty() || i.contains(['<', '>']))
        || id.chars().any(|c| c.is_control() || c == ' ')
    {
        bail!("invalid Message-ID: {id:?}");
    }
    let boundary = match &msg.boundary {
        Some(b) if valid_boundary(b) => Some(b.clone()),
        Some(b) => bail!("invalid MIME boundary: {b:?}"),
        None => None,
    };

    let mut b = lettre::Message::builder()
        .from(msg.from.mailbox()?)
        .subject(msg.subject.clone())
        .message_id(Some(id));
    for to in &msg.to {
        b = b.to(to.mailbox()?);
    }
    b = match msg.date {
        Some(d) => b.date(d.into()),
        None => b.date_now(),
    };

    let text = SinglePart::plain(msg.body.clone());
    let built = if msg.attachments.is_empty() {
        b.singlepart(text)
    } else {
        if let Some(bd) = &boundary {
            if msg.body.contains(bd.as_str()) {
                bail!("boundary collides with the message body");
            }
        }
        let mut multi = match boundary {
            Some(bd) => MultiPart::mixed().boundary(bd).singlepart(text),
            None => MultiPart::mixed().singlepart(text),
        };
        for a in &msg.attachments {
            if has_control(&a.file_name) {
                bail!("invalid attachment file name: control characters");
            }
            if !valid_mime(&a.mime) {
                bail!("invalid attachment MIME type: {:?}", a.mime);
            }
            let ct = ContentType::parse(&a.mime)
                .map_err(|e| anyhow!("invalid attachment MIME type {:?}: {e}", a.mime))?;
            let name = if a.file_name.trim().is_empty() {
                "attachment".to_string()
            } else {
                a.file_name.clone()
            };
            multi = multi.singlepart(MimeAttachment::new(name).body(a.data.clone(), ct));
        }
        b.multipart(multi)
    };
    built.context("building the message")
}

/// Builds the complete message (headers, blank line, body) as ASCII bytes.
pub fn build(msg: &Message) -> Result<Vec<u8>> {
    Ok(to_lettre(msg)?.formatted())
}

/// How the SMTP connection is secured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Cleartext. Only allowed for loopback hosts unless [`SmtpConfig::allow_insecure`] is set,
    /// and never together with credentials to any other host.
    None,
    /// Connect in cleartext, then upgrade with `STARTTLS` (required: no downgrade). Port 587.
    StartTls,
    /// TLS from the first byte ("SMTPS"). Port 465.
    Tls,
}

/// Where and how to send.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// `(user, password)`; `AUTH PLAIN` or `AUTH LOGIN`, whichever the server offers.
    pub credentials: Option<(String, String)>,
    /// Connect, read and write timeout.
    pub timeout: Duration,
    /// Name announced in `EHLO`; `localhost` by default.
    pub helo_name: String,
    /// Extra trusted root certificates (DER), e.g. a private CA. The web PKI roots are always
    /// trusted.
    pub extra_roots: Vec<Vec<u8>>,
    /// Allow a cleartext connection to a non-loopback host (without credentials).
    pub allow_insecure: bool,
}

impl SmtpConfig {
    /// `STARTTLS` on port 587, no credentials.
    pub fn new(host: &str) -> SmtpConfig {
        SmtpConfig {
            host: host.to_string(),
            port: 587,
            security: Security::StartTls,
            credentials: None,
            timeout: Duration::from_secs(30),
            helo_name: "localhost".into(),
            extra_roots: Vec::new(),
            allow_insecure: false,
        }
    }

    fn is_loopback(&self) -> bool {
        self.host.eq_ignore_ascii_case("localhost")
            || self
                .host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    }

    fn check(&self) -> Result<()> {
        if self.host.trim().is_empty() || self.host.contains(char::is_whitespace) {
            bail!("invalid SMTP host: {:?}", self.host);
        }
        if self.security == Security::None && !self.is_loopback() {
            if self.credentials.is_some() {
                bail!(
                    "refusing to send the SMTP password without encryption to {}",
                    self.host
                );
            }
            if !self.allow_insecure {
                bail!("refusing an unencrypted SMTP connection to {}", self.host);
            }
        }
        if let Some((u, p)) = &self.credentials {
            if has_control(u) || has_control(p) {
                bail!("invalid SMTP credentials: control characters");
            }
        }
        Ok(())
    }

    fn transport(&self) -> Result<SmtpTransport> {
        self.check()?;
        let tls = || -> Result<TlsParameters> {
            let mut p = TlsParameters::builder(self.host.clone());
            for der in &self.extra_roots {
                p = p.add_root_certificate(
                    Certificate::from_der(der.clone()).context("invalid root certificate")?,
                );
            }
            p.build().context("preparing TLS")
        };
        let mode = match self.security {
            Security::None => Tls::None,
            Security::StartTls => Tls::Required(tls()?),
            Security::Tls => Tls::Wrapper(tls()?),
        };
        let mut b = SmtpTransport::builder_dangerous(self.host.clone())
            .port(self.port)
            .tls(mode)
            .timeout(Some(self.timeout))
            .hello_name(lettre::transport::smtp::extension::ClientId::Domain(
                self.helo_name.clone(),
            ));
        if let Some((u, p)) = &self.credentials {
            b = b
                .credentials(Credentials::new(u.clone(), p.clone()))
                .authentication(vec![Mechanism::Plain, Mechanism::Login]);
        }
        Ok(b.build())
    }
}

/// Delivers `msg` through the SMTP server in `cfg`: one connection, `EHLO`, optional `STARTTLS`,
/// optional `AUTH`, `MAIL FROM`, one `RCPT TO` per recipient, `DATA`, `QUIT`.
pub fn send(cfg: &SmtpConfig, msg: &Message) -> Result<()> {
    let mail = to_lettre(msg)?;
    cfg.transport()?.send(&mail).map(|_| ()).map_err(|e| {
        anyhow!(
            "sending the message via {}:{} failed: {e}",
            cfg.host,
            cfg.port
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_parser::MimeHeaders;

    fn addr(s: &str) -> Address {
        Address::parse(s).unwrap()
    }

    fn msg() -> Message {
        Message::new(
            addr("Reader <me@example.com>"),
            vec![addr("dev_xyz@kindle.com")],
            "Hello",
            "Body text",
        )
    }

    fn parse(raw: &[u8]) -> mail_parser::Message<'_> {
        mail_parser::MessageParser::default().parse(raw).unwrap()
    }

    // ----------------------------------------------------------------------- addresses

    #[test]
    fn address_validation() {
        for ok in ["a@b.co", "first.last+tag@sub.example.org", "x@localhost"] {
            assert!(Address::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "plain",
            "@x.com",
            "a@",
            "a@@b.c",
            "a b@c.d",
            "a@b..c",
            "a@.b.c",
            "a@b.c.",
            "a<b>@c.d",
            "a\"b@c.d",
            "ä@b.c",
            "a@b,c@d.e",
        ] {
            assert!(Address::new(bad).is_err(), "{bad:?}");
        }
        assert!(Address::new(&format!("{}@x.io", "a".repeat(250))).is_err());
        assert_eq!(Address::new("  a@b.c ").unwrap().email, "a@b.c");
    }

    #[test]
    fn address_parse_forms() {
        let a = Address::parse("Ann Reader <ann@example.com>").unwrap();
        assert_eq!(
            (a.name.as_deref(), a.email.as_str()),
            (Some("Ann Reader"), "ann@example.com")
        );
        let a = Address::parse("\"Reader, Ann\" <ann@example.com>").unwrap();
        assert_eq!(a.name.as_deref(), Some("Reader, Ann"));
        let a = Address::parse("\"Say \\\"hi\\\"\" <a@b.c>").unwrap();
        assert_eq!(a.name.as_deref(), Some("Say \"hi\""));
        let a = Address::parse("<a@b.c>").unwrap();
        assert_eq!(a.name, None);
        assert_eq!(Address::parse("a@b.c").unwrap().domain(), "b.c");
        assert_eq!(a.domain(), "b.c");
        assert!(Address::parse("Name <not-an-address>").is_err());
        assert!(Address::parse("a@b.c\r\nBcc: evil@x.y").is_err());
        assert!(Address::parse("a@b.c\0").is_err());
        assert!(Address::parse("Na\nme <a@b.c>").is_err());
    }

    #[test]
    fn with_name_trims_and_clears() {
        assert_eq!(
            addr("a@b.c").with_name("  Ann ").name.as_deref(),
            Some("Ann")
        );
        assert_eq!(addr("Ann <a@b.c>").with_name("   ").name, None);
    }

    #[test]
    fn domain_of_odd_values() {
        let a = Address {
            name: None,
            email: "nodomain".into(),
        };
        assert_eq!(a.domain(), "localhost");
    }

    // ------------------------------------------------------------ build / parse round trips

    #[test]
    fn plain_message_round_trips_through_an_independent_parser() {
        let mut m = msg();
        m.date = Some(Utc.with_ymd_and_hms(2024, 3, 9, 8, 7, 6).unwrap());
        m.message_id = Some("<fixed.id@example.com>".into());
        let raw = build(&m).unwrap();
        assert!(raw.is_ascii());
        assert!(!raw.windows(2).any(|w| w == b"\n\n"), "no bare LF");
        let p = parse(&raw);
        assert_eq!(p.subject(), Some("Hello"));
        assert_eq!(p.message_id(), Some("fixed.id@example.com"));
        let from = p.from().unwrap().first().unwrap();
        assert_eq!(from.name(), Some("Reader"));
        assert_eq!(from.address(), Some("me@example.com"));
        let to = p.to().unwrap().first().unwrap();
        assert_eq!(to.address(), Some("dev_xyz@kindle.com"));
        assert_eq!(
            p.date().unwrap().to_timestamp(),
            m.date.unwrap().timestamp()
        );
        assert_eq!(p.body_text(0).unwrap().trim_end(), "Body text");
        assert_eq!(p.attachment_count(), 0);
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("MIME-Version: 1.0\r\n"));
    }

    #[test]
    fn attachment_round_trips_byte_for_byte() {
        let data: Vec<u8> = (0..70_000u32).map(|i| (i * 31 % 256) as u8).collect();
        let m = msg().attach(Attachment::new("Book.epub", data.clone()));
        let raw = build(&m).unwrap();
        assert!(raw.is_ascii());
        let p = parse(&raw);
        assert_eq!(p.attachment_count(), 1);
        let a = p.attachment(0).unwrap();
        assert_eq!(a.attachment_name(), Some("Book.epub"));
        assert_eq!(a.contents(), &data[..]);
        let ct = a.content_type().unwrap();
        assert_eq!(
            format!("{}/{}", ct.ctype(), ct.subtype().unwrap()),
            "application/epub+zip"
        );
        assert_eq!(p.body_text(0).unwrap().trim_end(), "Body text");
        // every line is at most 998 octets (RFC 5322) and base64 lines are short
        assert!(raw.split(|b| *b == b'\n').all(|l| l.len() <= 999));
    }

    #[test]
    fn big_attachment_round_trips() {
        let data: Vec<u8> = (0..6 * 1024 * 1024u32)
            .map(|i| (i ^ (i >> 7)) as u8)
            .collect();
        let m = msg().attach(Attachment::new("big.pdf", data.clone()));
        let raw = build(&m).unwrap();
        let p = parse(&raw);
        assert_eq!(p.attachment(0).unwrap().contents(), &data[..]);
    }

    #[test]
    fn non_ascii_subject_name_and_file_name() {
        let from = addr("a@b.c").with_name("Zoë Müller");
        let to = addr("x@y.z").with_name("日本語 Leser");
        let m = Message::new(from, vec![to], "Grüße 📚 aus Köln – 日本語", "Größe ✓")
            .attach(Attachment::new(
                "Löwe 日本.epub",
                b"PK\x03\x04data".to_vec(),
            ))
            .attach(Attachment::new(
                "naïve \"quoted\" name.mobi",
                b"MOBI".to_vec(),
            ));
        let raw = build(&m).unwrap();
        assert!(raw.is_ascii(), "headers and body must be 7-bit");
        let p = parse(&raw);
        assert_eq!(p.subject(), Some("Grüße 📚 aus Köln – 日本語"));
        assert_eq!(
            p.from().unwrap().first().unwrap().name(),
            Some("Zoë Müller")
        );
        assert_eq!(
            p.to().unwrap().first().unwrap().name(),
            Some("日本語 Leser")
        );
        assert_eq!(p.body_text(0).unwrap().trim_end(), "Größe ✓");
        assert_eq!(
            p.attachment(0).unwrap().attachment_name(),
            Some("Löwe 日本.epub")
        );
        assert_eq!(
            p.attachment(1).unwrap().attachment_name(),
            Some("naïve \"quoted\" name.mobi")
        );
        assert_eq!(p.attachment(0).unwrap().contents(), b"PK\x03\x04data");
    }

    #[test]
    fn long_non_ascii_subject_is_folded_into_short_lines() {
        let subject = "Überlänge ".repeat(40);
        let raw = build(&Message::new(
            addr("a@b.c"),
            vec![addr("x@y.z")],
            &subject,
            "b",
        ))
        .unwrap();
        assert!(raw.is_ascii());
        assert!(raw.split(|b| *b == b'\n').all(|l| l.len() <= 999));
        assert_eq!(parse(&raw).subject(), Some(subject.trim()));
    }

    #[test]
    fn multiple_recipients_and_attachments() {
        let m = Message::new(
            addr("a@b.c"),
            vec![
                addr("One <1@x.y>"),
                addr("2@x.y"),
                addr("\"Three, III\" <3@x.y>"),
            ],
            "S",
            "B",
        )
        .attach(Attachment::new("a.txt", b"first".to_vec()))
        .attach(Attachment::new("b.pdf", b"second".to_vec()))
        .attach(Attachment::new("noext", b"third".to_vec()));
        let p_raw = build(&m).unwrap();
        let p = parse(&p_raw);
        let to: Vec<_> = p
            .to()
            .unwrap()
            .iter()
            .map(|a| a.address().unwrap())
            .collect();
        assert_eq!(to, ["1@x.y", "2@x.y", "3@x.y"]);
        assert_eq!(p.to().unwrap().first().unwrap().name(), Some("One"));
        assert_eq!(p.to().unwrap().last().unwrap().name(), Some("Three, III"));
        assert_eq!(p.attachment_count(), 3);
        let names: Vec<_> = (0..3)
            .map(|i| p.attachment(i).unwrap().attachment_name().unwrap())
            .collect();
        assert_eq!(names, ["a.txt", "b.pdf", "noext"]);
        let c = p.attachment(2).unwrap().content_type().unwrap();
        assert_eq!(
            (c.ctype(), c.subtype()),
            ("application", Some("octet-stream"))
        );
    }

    #[test]
    fn body_with_unicode_and_long_lines() {
        let body = format!(
            "{}\n\nSecond ünï paragraph\r\n.dot-stuffing line\n",
            "x".repeat(5000)
        );
        let raw = build(&Message::new(
            addr("a@b.c"),
            vec![addr("x@y.z")],
            "S",
            &body,
        ))
        .unwrap();
        assert!(raw.is_ascii());
        assert!(raw.split(|b| *b == b'\n').all(|l| l.len() <= 999));
        let p = parse(&raw);
        let got = p.body_text(0).unwrap().replace("\r\n", "\n");
        assert_eq!(got.trim_end(), body.replace("\r\n", "\n").trim_end());
    }

    #[test]
    fn empty_attachment_and_empty_body() {
        let m = Message::new(addr("a@b.c"), vec![addr("x@y.z")], "", "")
            .attach(Attachment::new("empty.txt", Vec::new()));
        let raw = build(&m).unwrap();
        let p = parse(&raw);
        assert_eq!(p.attachment_count(), 1);
        assert!(p.attachment(0).unwrap().contents().is_empty());
        let raw = build(&Message::new(addr("a@b.c"), vec![addr("x@y.z")], "S", "")).unwrap();
        assert_eq!(parse(&raw).subject(), Some("S"));
    }

    #[test]
    fn empty_file_name_gets_a_default() {
        let m = msg()
            .attach(Attachment::new("", b"x".to_vec()))
            .attach(Attachment::new("  ", b"y".to_vec()));
        let p_raw = build(&m).unwrap();
        let p = parse(&p_raw);
        assert_eq!(
            p.attachment(0).unwrap().attachment_name(),
            Some("attachment")
        );
        assert_eq!(
            p.attachment(1).unwrap().attachment_name(),
            Some("attachment")
        );
    }

    #[test]
    fn generated_defaults_are_sane() {
        let a = build(&msg()).unwrap();
        let b = build(&msg()).unwrap();
        let (pa, pb) = (parse(&a), parse(&b));
        let (ia, ib) = (pa.message_id().unwrap(), pb.message_id().unwrap());
        assert_ne!(ia, ib, "random ids");
        assert!(ia.ends_with("@example.com"), "{ia}");
        let now = Utc::now().timestamp();
        assert!((pa.date().unwrap().to_timestamp() - now).abs() < 120);
        // two builds with attachments use different boundaries
        let ma = msg().attach(Attachment::new("a.txt", b"1".to_vec()));
        let (ra, rb) = (build(&ma).unwrap(), build(&ma).unwrap());
        let bound = |r: &[u8]| {
            let t = String::from_utf8_lossy(r).into_owned();
            let i = t.find("boundary=\"").unwrap() + 10;
            t[i..i + t[i..].find('"').unwrap()].to_string()
        };
        assert_ne!(bound(&ra), bound(&rb));
    }

    #[test]
    fn custom_boundary_is_used() {
        let mut m = msg().attach(Attachment::new("a.txt", b"1".to_vec()));
        m.boundary = Some("my-boundary_123".into());
        let raw = String::from_utf8(build(&m).unwrap()).unwrap();
        assert!(raw.contains("boundary=\"my-boundary_123\""), "{raw}");
        assert!(raw.contains("--my-boundary_123--"));
    }

    // ------------------------------------------------------------------ hostile input

    fn fails(m: &Message, needle: &str) {
        let e = build(m).unwrap_err().to_string();
        assert!(e.contains(needle), "{e:?} should mention {needle:?}");
    }

    #[test]
    fn header_injection_in_the_subject_fails() {
        for s in [
            "Hi\r\nBcc: evil@x.y",
            "Hi\nBcc: evil@x.y",
            "Hi\rBcc: evil@x.y",
            "nul\0byte",
        ] {
            let mut m = msg();
            m.subject = s.into();
            fails(&m, "subject");
        }
        // a tab is fine
        let mut m = msg();
        m.subject = "a\tb".into();
        assert!(build(&m).is_ok());
    }

    #[test]
    fn header_injection_in_addresses_fails() {
        let mut m = msg();
        m.from = Address {
            name: None,
            email: "a@b.c\r\nBcc: evil@x.y".into(),
        };
        fails(&m, "invalid e-mail address");
        let mut m = msg();
        m.to = vec![Address {
            name: None,
            email: "a@b.c>\r\nBcc: e@x.y".into(),
        }];
        fails(&m, "invalid e-mail address");
        let mut m = msg();
        m.from = addr("a@b.c").with_name("Evil\r\nBcc: e@x.y");
        fails(&m, "display name");
        let mut m = msg();
        m.to = vec![addr("a@b.c").with_name("x\ny")];
        fails(&m, "display name");
    }

    #[test]
    fn header_injection_in_file_names_fails() {
        for n in [
            "a\r\nContent-Type: x/y.epub",
            "a\n.epub",
            "a\0.epub",
            "a\r.epub",
        ] {
            let m = msg().attach(Attachment::new(n, b"x".to_vec()));
            fails(&m, "file name");
        }
    }

    #[test]
    fn hostile_but_printable_file_names_stay_inside_the_header() {
        for n in [
            "../../etc/passwd",
            "a\";filename=\"b",
            "a b; c=d",
            "semi;colon,comma.epub",
            "percent%20name.epub",
        ] {
            let m = msg().attach(Attachment::new(n, b"x".to_vec()));
            let raw = build(&m).unwrap();
            let p = parse(&raw);
            assert_eq!(p.attachment_count(), 1, "{n}");
            assert_eq!(p.attachment(0).unwrap().attachment_name(), Some(n), "{n}");
            assert!(p.header("Bcc").is_none());
        }
    }

    #[test]
    fn an_encoded_word_in_a_file_name_cannot_inject_headers() {
        // some readers decode RFC 2047 words inside parameters; whatever they show, the message
        // still has exactly one attachment and no extra header
        let m = msg().attach(Attachment::new("=?utf-8?B?ZXZpbA==?=.epub", b"x".to_vec()));
        let raw = build(&m).unwrap();
        let p = parse(&raw);
        assert_eq!(p.attachment_count(), 1);
        assert!(p.header("Bcc").is_none());
    }

    #[test]
    fn error_paths() {
        let mut m = msg();
        m.to.clear();
        fails(&m, "no recipient");
        for bad in [
            "no-brackets",
            "<a b>",
            "<x>\r\n<y>",
            "<>",
            "<a<b>",
            "<id@x>\nBcc: z",
            "<a>b<c>",
        ] {
            let mut m = msg();
            m.message_id = Some(bad.into());
            fails(&m, "Message-ID");
        }
        let mut m = msg();
        m.message_id = Some("<ok@id>".into());
        assert!(build(&m).is_ok());
        for bad in [
            "",
            "has\"quote",
            "line\r\nbreak",
            &"b".repeat(71),
            "trailing ",
        ] {
            let mut m = msg().attach(Attachment::new("a.txt", b"1".to_vec()));
            m.boundary = Some(bad.into());
            fails(&m, "boundary");
        }
        for mime in ["", "noslash", "text/plain\r\nX: y", "a b/c", "text/"] {
            let mut a = Attachment::new("a.txt", b"1".to_vec());
            a.mime = mime.into();
            fails(&msg().attach(a), "MIME type");
        }
    }

    #[test]
    fn boundary_collision_with_the_body_is_detected() {
        let mut m = Message::new(
            addr("a@b.c"),
            vec![addr("x@y.z")],
            "S",
            "see --BOUND-1 here",
        )
        .attach(Attachment::new("a.txt", b"1".to_vec()));
        m.boundary = Some("BOUND-1".into());
        fails(&m, "collides");
    }

    // -------------------------------------------------------------------- misc helpers

    #[test]
    fn mime_types_by_extension() {
        for (n, t) in [
            ("a.epub", "application/epub+zip"),
            ("a.EPUB", "application/epub+zip"),
            ("a.mobi", "application/x-mobipocket-ebook"),
            ("a.azw", "application/vnd.amazon.ebook"),
            ("a.azw3", "application/vnd.amazon.ebook"),
            ("a.pdf", "application/pdf"),
            ("a.txt", "text/plain"),
            ("a.doc", "application/msword"),
            (
                "a.docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            ("a.html", "text/html"),
            ("a.htm", "text/html"),
            ("a.rtf", "application/rtf"),
            ("a.jpg", "image/jpeg"),
            ("a.jpeg", "image/jpeg"),
            ("a.png", "image/png"),
            ("a.xyz", "application/octet-stream"),
            ("noext", "application/octet-stream"),
        ] {
            assert_eq!(attachment_mime(n), t, "{n}");
            assert_eq!(Attachment::new(n, vec![]).mime, t);
        }
    }

    #[test]
    fn kindle_limit() {
        let m = msg().attach(Attachment::new("a.epub", vec![0; 10]));
        assert_eq!(m.attachment_bytes(), 10);
        m.check_kindle_limit().unwrap();
        let mut big = msg();
        big.attachments.push(Attachment::new(
            "a.epub",
            vec![0; KINDLE_MAX_BYTES as usize],
        ));
        big.check_kindle_limit().unwrap(); // exactly the limit
        big.attachments.push(Attachment::new("b.epub", vec![0; 1]));
        let e = big.check_kindle_limit().unwrap_err().to_string();
        assert!(e.contains("51 MB") && e.contains("50 MB"), "{e}");
    }

    // ---------------------------------------------------------------------- smtp config

    #[test]
    fn smtp_config_defaults() {
        let c = SmtpConfig::new("smtp.example.com");
        assert_eq!((c.port, c.security), (587, Security::StartTls));
        assert!(c.credentials.is_none() && !c.allow_insecure);
        assert_eq!(c.helo_name, "localhost");
        assert!(c.check().is_ok());
    }

    #[test]
    fn cleartext_is_only_for_loopback_and_never_with_a_password() {
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "::1",
            "[::1]",
            "127.9.9.9",
        ] {
            let mut c = SmtpConfig::new(host);
            c.security = Security::None;
            c.credentials = Some(("u".into(), "p".into()));
            assert!(c.check().is_ok(), "{host}");
        }
        let mut c = SmtpConfig::new("mail.example.com");
        c.security = Security::None;
        assert!(c.check().unwrap_err().to_string().contains("unencrypted"));
        c.allow_insecure = true;
        assert!(c.check().is_ok());
        c.credentials = Some(("u".into(), "p".into()));
        assert!(c.check().unwrap_err().to_string().contains("password"));
        // TLS modes are fine with credentials
        for s in [Security::StartTls, Security::Tls] {
            c.security = s;
            assert!(c.check().is_ok());
        }
    }

    #[test]
    fn smtp_config_rejects_odd_hosts_and_credentials() {
        for h in ["", "  ", "a b", "host\r\nRCPT"] {
            assert!(SmtpConfig::new(h).check().is_err(), "{h:?}");
        }
        let mut c = SmtpConfig::new("h.example");
        c.credentials = Some(("us\r\ner".into(), "p".into()));
        assert!(c.check().is_err());
        c.credentials = Some(("u".into(), "p\nw".into()));
        assert!(c.check().is_err());
    }

    #[test]
    fn bad_extra_root_certificate_is_reported() {
        let mut c = SmtpConfig::new("h.example");
        c.extra_roots = vec![b"not a certificate".to_vec()];
        let e = format!("{:#}", c.transport().err().unwrap());
        assert!(e.contains("certificate"), "{e}");
    }

    #[test]
    fn send_refuses_an_invalid_message_before_connecting() {
        let mut m = msg();
        m.subject = "x\r\ny".into();
        let mut c = SmtpConfig::new("127.0.0.1");
        c.port = 1; // nothing listens; the error must come from validation
        c.security = Security::None;
        let e = send(&c, &m).unwrap_err().to_string();
        assert!(e.contains("subject"), "{e}");
    }
}
