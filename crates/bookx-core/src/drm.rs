//! Detects copy protection (DRM) in book files from structural markers, so Bookx can say
//! clearly why a book cannot be opened.
//!
//! Bookx does **not** remove DRM and never tries to: this module only looks at headers and
//! package files (`META-INF/encryption.xml`, the MOBI encryption flag, the KFX DRM header).

use std::io::{Cursor, Read};
use std::path::Path;

/// The protection scheme that was recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrmKind {
    /// Adobe Digital Editions (ADEPT), EPUB or PDF.
    AdobeAdept,
    /// Amazon Kindle (AZW/AZW3/MOBI with an encryption type, or KFX with DRM).
    Kindle,
    /// Rakuten Kobo.
    Kobo,
    /// Apple Books (FairPlay).
    AppleFairPlay,
    /// Readium LCP.
    Lcp,
    /// The content is encrypted but the scheme is not known.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drm {
    pub kind: DrmKind,
    /// What was found, for the log and tests (`META-INF/rights.xml`, `MOBI encryption type 2`).
    pub evidence: String,
}

impl DrmKind {
    pub fn label(self) -> &'static str {
        match self {
            DrmKind::AdobeAdept => "Adobe ADEPT",
            DrmKind::Kindle => "Kindle",
            DrmKind::Kobo => "Kobo",
            DrmKind::AppleFairPlay => "Apple FairPlay",
            DrmKind::Lcp => "Readium LCP",
            DrmKind::Other => "encrypted",
        }
    }
}

impl Drm {
    /// Short text for the badge: `Protected (Kindle)`.
    pub fn badge(&self) -> String {
        format!("Protected ({})", self.kind.label())
    }

    /// The message shown to the user.
    pub fn message(&self) -> String {
        format!(
            "This book is protected with {} DRM. Bookx cannot open it, and Bookx does not remove DRM.",
            self.kind.label()
        )
    }
}

fn found(kind: DrmKind, evidence: &str) -> Option<Drm> {
    Some(Drm {
        kind,
        evidence: evidence.into(),
    })
}

/// Looks at the file at `path`; `None` when no protection is recognised (or it cannot be read).
pub fn detect(path: &Path) -> Option<Drm> {
    let bytes = std::fs::read(path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    detect_bytes(&name, &bytes)
}

/// Looks at file contents; `name` is only used for its extension and the `kepub` hint.
pub fn detect_bytes(name: &str, bytes: &[u8]) -> Option<Drm> {
    let lower = name.to_lowercase();
    if bytes.starts_with(b"PK") {
        return detect_zip(&lower, bytes);
    }
    if bytes.starts_with(b"%PDF") {
        let head = &bytes[..bytes.len().min(1 << 20)];
        let tail = &bytes[bytes.len().saturating_sub(1 << 20)..];
        for part in [head, tail] {
            if contains(part, b"/Adobe.PubSec") || contains(part, b"/ADEPT") {
                return found(DrmKind::AdobeAdept, "PDF /Encrypt /Adobe.PubSec");
            }
        }
        return None;
    }
    if contains(&bytes[..bytes.len().min(1 << 16)], b"\xeaDRMION\xee") {
        return found(DrmKind::Kindle, "KFX DRM header");
    }
    detect_mobi(bytes)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// PalmDB with a MOBI/KF8 header: the encryption type sits at byte 12 of record 0.
fn detect_mobi(b: &[u8]) -> Option<Drm> {
    let kind = b.get(60..68)?;
    if kind != b"BOOKMOBI" && kind != b"TEXtREAd" {
        return None;
    }
    let records = be16(b, 76)?;
    if records == 0 {
        return None;
    }
    let rec0 = be32(b, 78)? as usize;
    let enc = be16(b, rec0 + 12)?;
    (enc != 0).then(|| Drm {
        kind: DrmKind::Kindle,
        evidence: format!("MOBI encryption type {enc}"),
    })
}

fn detect_zip(lower_name: &str, bytes: &[u8]) -> Option<Drm> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut read = |name: &str| -> Option<String> {
        let mut f = zip.by_name(name).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    };
    if read("META-INF/license.lcpl").is_some() {
        return found(DrmKind::Lcp, "META-INF/license.lcpl");
    }
    if read("META-INF/sinf.xml").is_some() {
        return found(DrmKind::AppleFairPlay, "META-INF/sinf.xml");
    }
    let rights = read("META-INF/rights.xml");
    let enc = read("META-INF/encryption.xml");
    if let Some(e) = &enc {
        if e.contains("readium.org/2014/01/lcp") {
            return found(DrmKind::Lcp, "encryption.xml names the LCP profile");
        }
    }
    if let Some(r) = &rights {
        if r.contains("ns.adobe.com/adept") {
            return found(DrmKind::AdobeAdept, "META-INF/rights.xml (ADEPT)");
        }
    }
    let enc = enc?;
    if !encrypts_content(&enc) {
        return None; // only font obfuscation
    }
    if lower_name.contains("kepub") || lower_name.ends_with(".kobo") {
        return found(DrmKind::Kobo, "encrypted content in a KEPUB");
    }
    if rights.is_some() {
        return found(
            DrmKind::AdobeAdept,
            "META-INF/rights.xml with encrypted content",
        );
    }
    found(
        DrmKind::Other,
        "META-INF/encryption.xml encrypts the content",
    )
}

/// True when `encryption.xml` lists an algorithm that is more than font obfuscation.
fn encrypts_content(xml: &str) -> bool {
    let mut rest = xml;
    while let Some(i) = rest.find("Algorithm=") {
        rest = &rest[i + "Algorithm=".len()..];
        let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let end = rest[1..].find(q).map(|e| e + 1).unwrap_or(rest.len());
        let alg = &rest[1..end];
        let obfuscation = alg == "http://www.idpf.org/2008/embedding"
            || alg == "http://ns.adobe.com/pdf/enc#RC"
            || alg.contains("xmlenc#sha");
        if !obfuscation {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut out));
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("mimetype", opts).unwrap();
            w.write_all(b"application/epub+zip").unwrap();
            for (n, c) in files {
                w.start_file(*n, opts).unwrap();
                w.write_all(c.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        out
    }

    const AES: &str = r#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></EncryptedData></encryption>"#;
    const FONT: &str = r#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/></EncryptedData></encryption>"#;

    fn mobi(enc: u16) -> Vec<u8> {
        let mut b = vec![0u8; 400];
        b[60..68].copy_from_slice(b"BOOKMOBI");
        b[76..78].copy_from_slice(&1u16.to_be_bytes());
        b[78..82].copy_from_slice(&200u32.to_be_bytes());
        b[212..214].copy_from_slice(&enc.to_be_bytes());
        b
    }

    #[test]
    fn adept_epub_is_recognised_by_rights_and_encryption() {
        let z = zip_with(&[
            ("META-INF/encryption.xml", AES),
            (
                "META-INF/rights.xml",
                r#"<rights xmlns="http://ns.adobe.com/adept"/>"#,
            ),
        ]);
        let d = detect_bytes("a.epub", &z).unwrap();
        assert_eq!(d.kind, DrmKind::AdobeAdept);
        assert!(d.badge().contains("Adobe"));
        assert!(d.message().contains("does not remove DRM"));
    }

    #[test]
    fn font_obfuscation_alone_is_not_drm() {
        let z = zip_with(&[("META-INF/encryption.xml", FONT)]);
        assert!(detect_bytes("a.epub", &z).is_none());
        assert!(detect_bytes("a.epub", &zip_with(&[])).is_none());
    }

    #[test]
    fn kobo_lcp_and_apple_are_told_apart() {
        let kobo = zip_with(&[("META-INF/encryption.xml", AES)]);
        assert_eq!(
            detect_bytes("Book.kepub.epub", &kobo).unwrap().kind,
            DrmKind::Kobo
        );
        assert_eq!(
            detect_bytes("Book.epub", &kobo).unwrap().kind,
            DrmKind::Other
        );
        let lcp = zip_with(&[("META-INF/license.lcpl", "{}")]);
        assert_eq!(detect_bytes("a.epub", &lcp).unwrap().kind, DrmKind::Lcp);
        let apple = zip_with(&[("META-INF/sinf.xml", "<sinf/>")]);
        assert_eq!(
            detect_bytes("a.epub", &apple).unwrap().kind,
            DrmKind::AppleFairPlay
        );
    }

    #[test]
    fn mobi_encryption_flag_and_kfx_header() {
        assert!(detect_bytes("a.mobi", &mobi(0)).is_none());
        let d = detect_bytes("a.azw3", &mobi(2)).unwrap();
        assert_eq!(d.kind, DrmKind::Kindle);
        assert_eq!(d.evidence, "MOBI encryption type 2");
        let mut kfx = b"\xeaDRMION\xee".to_vec();
        kfx.extend([0u8; 40]);
        assert_eq!(detect_bytes("a.kfx", &kfx).unwrap().kind, DrmKind::Kindle);
    }

    #[test]
    fn adobe_pdf_and_plain_files() {
        assert_eq!(
            detect_bytes("a.pdf", b"%PDF-1.7 /Encrypt << /Filter /Adobe.PubSec >>")
                .unwrap()
                .kind,
            DrmKind::AdobeAdept
        );
        assert!(detect_bytes("a.pdf", b"%PDF-1.7 plain").is_none());
        assert!(detect_bytes("a.txt", b"hello").is_none());
        assert!(detect_bytes("a.mobi", b"short").is_none());
    }
}
