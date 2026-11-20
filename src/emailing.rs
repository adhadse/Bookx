//! Sending a book by e-mail (roadmap P17), e.g. to a Kindle's "Send to Kindle" address.
//! SMTP itself lives in `bookx-serve::mail`; this is the settings and the glue. No GTK in here.

use crate::i18n::tr;
use std::path::Path;

use anyhow::{bail, Context, Result};
use bookx_serve::mail::{Address, Attachment, Message, Security, SmtpConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Encryption {
    /// `STARTTLS` on the submission port (587).
    #[default]
    StartTls,
    /// TLS from the first byte (465).
    Tls,
    /// No encryption; only accepted for this computer (a local mail server).
    None,
}

impl Encryption {
    pub const ALL: [Encryption; 3] = [Encryption::StartTls, Encryption::Tls, Encryption::None];

    pub fn label(self) -> &'static str {
        match self {
            Encryption::StartTls => crate::i18n::N_("STARTTLS (port 587)"),
            Encryption::Tls => crate::i18n::N_("TLS (port 465)"),
            Encryption::None => crate::i18n::N_("None (local server only)"),
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Encryption::StartTls => 587,
            Encryption::Tls => 465,
            Encryption::None => 25,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailSettings {
    pub host: String,
    pub port: u16,
    pub encryption: Encryption,
    pub username: String,
    /// Kept in the system keyring (the settings file only when there is none), see `secrets.rs`.
    pub password: String,
    /// Sender address (`Name <me@example.com>` or just the address).
    pub from: String,
    /// Default recipient, e.g. the "Send to Kindle" address.
    pub to: String,
}

impl Default for EmailSettings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 587,
            encryption: Encryption::StartTls,
            username: String::new(),
            password: String::new(),
            from: String::new(),
            to: String::new(),
        }
    }
}

/// Amazon rejects larger attachments, so the limit is only enforced for Kindle addresses.
fn is_kindle(a: &Address) -> bool {
    let d = a.domain().to_ascii_lowercase();
    d == "kindle.com" || d.ends_with(".kindle.com")
}

impl EmailSettings {
    /// What is still missing before a book can be sent, in words for the user.
    pub fn problem(&self) -> Option<String> {
        if self.host.trim().is_empty() {
            return Some(tr("Enter the mail server in Preferences → E-mail"));
        }
        if Address::parse(&self.from).is_err() {
            return Some(tr("Enter your own e-mail address in Preferences → E-mail"));
        }
        if Address::parse(&self.to).is_err() {
            return Some(tr(
                "Enter the address to send books to in Preferences → E-mail",
            ));
        }
        if !self.username.is_empty() && self.password.is_empty() {
            return Some(tr(
                "Enter the password of the mail account in Preferences → E-mail",
            ));
        }
        None
    }

    pub fn smtp(&self) -> SmtpConfig {
        let mut c = SmtpConfig::new(self.host.trim());
        c.port = self.port;
        c.security = match self.encryption {
            Encryption::StartTls => Security::StartTls,
            Encryption::Tls => Security::Tls,
            Encryption::None => Security::None,
        };
        if !self.username.is_empty() {
            c.credentials = Some((self.username.clone(), self.password.clone()));
        }
        c
    }

    /// The message that carries `file` as an attachment.
    pub fn message(
        &self,
        title: &str,
        author: &str,
        file_name: &str,
        data: Vec<u8>,
    ) -> Result<Message> {
        let from = Address::parse(&self.from)?;
        let to = Address::parse(&self.to)?;
        let body = if author.trim().is_empty() {
            format!("“{title}”, sent from Bookx.\n")
        } else {
            format!("“{title}” by {author}, sent from Bookx.\n")
        };
        let mut m = Message::new(from, vec![to.clone()], title, &body);
        m.attachments.push(Attachment::new(file_name, data));
        if is_kindle(&to) {
            m.check_kindle_limit()?;
        }
        Ok(m)
    }
}

/// A file name that is safe in a mail header and keeps the extension.
fn attachment_name(title: &str, ext: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clean: String = clean.chars().take(80).collect();
    let clean = if clean.is_empty() {
        "book".to_string()
    } else {
        clean
    };
    format!("{clean}.{ext}")
}

/// Sends the book file at `path` and returns a line for the user.
pub fn send_book(s: &EmailSettings, path: &Path, title: &str, author: &str) -> Result<String> {
    if let Some(p) = s.problem() {
        bail!(p);
    }
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("epub");
    let msg = s.message(title, author, &attachment_name(title, ext), data)?;
    bookx_serve::mail::send(&s.smtp(), &msg)?;
    Ok(format!("Sent “{title}” to {}", s.to.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn ready() -> EmailSettings {
        EmailSettings {
            host: "mail.example.com".into(),
            username: "me".into(),
            password: "secret".into(),
            from: "Me <me@example.com>".into(),
            to: "me_123@kindle.com".into(),
            ..Default::default()
        }
    }

    /// A one-connection SMTP server on a free port; the received DATA arrives on the channel.
    fn fake_smtp() -> (u16, mpsc::Receiver<(Vec<String>, String)>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            let mut cmds = Vec::new();
            let mut data = String::new();
            let mut in_data = false;
            let _ = w.write_all(b"220 fake ESMTP\r\n");
            let mut line = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        let _ = w.write_all(b"250 queued\r\n");
                    } else {
                        data.push_str(&line);
                    }
                    continue;
                }
                let up = line.trim_end().to_ascii_uppercase();
                cmds.push(line.trim_end().to_string());
                if up.starts_with("EHLO") {
                    let _ = w.write_all(b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n");
                } else if up.starts_with("AUTH") {
                    let _ = w.write_all(b"235 ok\r\n");
                } else if up.starts_with("DATA") {
                    in_data = true;
                    let _ = w.write_all(b"354 go\r\n");
                } else if up.starts_with("QUIT") {
                    let _ = w.write_all(b"221 bye\r\n");
                    break;
                } else {
                    let _ = w.write_all(b"250 ok\r\n");
                }
            }
            let _ = tx.send((cmds, data));
        });
        (port, rx)
    }

    #[test]
    fn missing_settings_are_named() {
        let mut s = EmailSettings::default();
        assert!(s.problem().unwrap().contains("mail server"));
        s.host = "h".into();
        assert!(s.problem().unwrap().contains("your own"));
        s.from = "me@example.com".into();
        assert!(s.problem().unwrap().contains("send books to"));
        s.to = "x@kindle.com".into();
        assert_eq!(s.problem(), None);
        s.username = "me".into();
        assert!(s.problem().unwrap().contains("password"));
        s.password = "pw".into();
        assert_eq!(s.problem(), None);
    }

    #[test]
    fn encryption_maps_to_smtp_security_and_ports() {
        let mut s = ready();
        assert!(matches!(s.smtp().security, Security::StartTls));
        assert_eq!(s.smtp().credentials, Some(("me".into(), "secret".into())));
        s.encryption = Encryption::Tls;
        assert!(matches!(s.smtp().security, Security::Tls));
        s.encryption = Encryption::None;
        s.username.clear();
        assert!(matches!(s.smtp().security, Security::None));
        assert!(s.smtp().credentials.is_none());
        for e in Encryption::ALL {
            assert!(!e.label().is_empty());
        }
        assert_eq!(Encryption::Tls.default_port(), 465);
        assert_eq!(Encryption::StartTls.default_port(), 587);
    }

    #[test]
    fn attachment_names_are_safe_and_keep_the_extension() {
        assert_eq!(
            attachment_name("Dune: Messiah / Part 2", "epub"),
            "Dune Messiah Part 2.epub"
        );
        assert_eq!(attachment_name("", "azw3"), "book.azw3");
        assert_eq!(attachment_name("a\r\nb", "epub"), "a b.epub");
        assert!(attachment_name(&"x".repeat(500), "epub").chars().count() <= 85);
    }

    #[test]
    fn kindle_addresses_enforce_the_size_limit_others_do_not() {
        let big = vec![0u8; (bookx_serve::mail::KINDLE_MAX_BYTES + 1) as usize];
        let s = ready();
        assert!(s.message("T", "A", "t.epub", big.clone()).is_err());
        let mut other = ready();
        other.to = "friend@example.org".into();
        assert!(other.message("T", "A", "t.epub", big).is_ok());
        let m = s.message("Title", "", "t.epub", vec![1, 2, 3]).unwrap();
        assert!(m.body.contains("“Title”") && !m.body.contains(" by "));
    }

    #[test]
    fn a_book_is_sent_to_a_local_mail_server() {
        let (port, rx) = fake_smtp();
        let dir = std::env::temp_dir().join(format!("bookx-mail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.epub");
        std::fs::write(&file, b"PK-fake-epub-bytes").unwrap();
        let s = EmailSettings {
            host: "127.0.0.1".into(),
            port,
            encryption: Encryption::None,
            username: String::new(),
            password: String::new(),
            from: "me@example.com".into(),
            to: "friend@example.org".into(),
        };
        let line = send_book(&s, &file, "My Book", "An Author").unwrap();
        assert!(line.contains("friend@example.org"), "{line}");
        let (cmds, data) = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(
            cmds.iter()
                .any(|c| c.contains("RCPT TO:<friend@example.org>")),
            "{cmds:?}"
        );
        assert!(
            cmds.iter()
                .any(|c| c.contains("MAIL FROM:<me@example.com>")),
            "{cmds:?}"
        );
        assert!(data.contains("Subject: My Book"), "{data}");
        assert!(data.contains("My Book.epub"), "{data}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dead_server_and_a_missing_file_give_readable_errors() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let dir = std::env::temp_dir().join(format!("bookx-mail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("b.epub");
        std::fs::write(&file, b"x").unwrap();
        let mut s = EmailSettings {
            host: "127.0.0.1".into(),
            port,
            encryption: Encryption::None,
            from: "me@example.com".into(),
            to: "friend@example.org".into(),
            ..Default::default()
        };
        let e = send_book(&s, &file, "T", "").unwrap_err();
        assert!(format!("{e:#}").contains("failed"), "{e:#}");
        let e = send_book(&s, &dir.join("gone.epub"), "T", "").unwrap_err();
        assert!(format!("{e:#}").contains("gone.epub"), "{e:#}");
        s.host.clear();
        assert!(send_book(&s, &file, "T", "")
            .unwrap_err()
            .to_string()
            .contains("mail server"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_configuration_files_load_with_empty_mail_settings() {
        let s: EmailSettings = serde_json::from_str(r#"{"host":"h"}"#).unwrap();
        assert_eq!(s.port, 587);
        assert_eq!(s.encryption, Encryption::StartTls);
        assert!(s.to.is_empty());
    }
}
