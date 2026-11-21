//! Tests only: the desktop file, the AppStream metainfo and the MIME definitions that ship in
//! `data/` agree with each other and with the formats Bookx reads (X20).

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    fn data(name: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    /// `(extension, media type)` for every format Bookx opens that a file manager should offer it
    /// for. Extensions of readers that have no sensible media type (rb, pml, snb, …) are listed in
    /// [`UNLISTED`] so a new reader forces a decision.
    const LISTED: &[(&str, &str)] = &[
        ("epub", "application/epub+zip"),
        ("kepub", "application/x-kobo-epub+zip"),
        ("mobi", "application/x-mobipocket-ebook"),
        ("prc", "application/x-mobipocket-ebook"),
        ("azw", "application/vnd.amazon.ebook"),
        ("azw3", "application/vnd.amazon.mobi8-ebook"),
        ("fb2", "application/x-fictionbook+xml"),
        ("fbz", "application/x-zip-compressed-fb2"),
        ("cbz", "application/vnd.comicbook+zip"),
        ("cbr", "application/vnd.comicbook-rar"),
        ("cbt", "application/x-cbt"),
        ("djvu", "image/vnd.djvu"),
        ("djv", "image/vnd.djvu"),
        ("pdf", "application/pdf"),
        ("rtf", "application/rtf"),
        ("odt", "application/vnd.oasis.opendocument.text"),
        (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("lrf", "application/x-sony-bbeb"),
        ("chm", "application/vnd.ms-htmlhelp"),
    ];

    /// Readable extensions that are deliberately not registered with the desktop (generic text,
    /// obscure or plain-text based formats that would make Bookx appear for unrelated files).
    const UNLISTED: &[&str] = &[
        "txt", "md", "markdown", "html", "htm", "xhtml", "htmlz", "txtz", "ott", "azw4", "cbc",
        "lit", "pdb", "pml", "pmlz", "rb", "snb", "tcr", "text", "xht", "mobi8", "kf8", "oeb",
    ];

    fn desktop_mime_types() -> BTreeSet<String> {
        data("com.adhadse.Bookx.desktop")
            .lines()
            .find_map(|l| l.strip_prefix("MimeType="))
            .expect("MimeType key")
            .split(';')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_desktop_file_registers_every_listed_format_and_the_link_scheme() {
        let have = desktop_mime_types();
        for (ext, mime) in LISTED {
            assert!(have.contains(*mime), ".{ext}: {mime} is not in MimeType");
        }
        assert!(have.contains("x-scheme-handler/bookx"));
        let wanted: BTreeSet<String> = LISTED
            .iter()
            .map(|(_, m)| m.to_string())
            .chain(["x-scheme-handler/bookx".to_string()])
            .collect();
        assert_eq!(have, wanted, "MimeType lists types the table does not know");
    }

    #[test]
    fn every_extension_bookx_reads_is_listed_or_deliberately_left_out() {
        let listed: BTreeSet<&str> = LISTED.iter().map(|(e, _)| *e).collect();
        let unknown: Vec<String> = crate::import::extensions()
            .into_iter()
            .filter(|e| !listed.contains(e.as_str()) && !UNLISTED.contains(&e.as_str()))
            .collect();
        assert!(
            unknown.is_empty(),
            "new readable formats {unknown:?}: add each to LISTED (with its media type, in the \
             desktop file and the metainfo) or to UNLISTED"
        );
    }

    #[test]
    fn the_desktop_file_launches_bookx_with_files_and_links() {
        let d = data("com.adhadse.Bookx.desktop");
        let exec = d.lines().find_map(|l| l.strip_prefix("Exec=")).unwrap();
        assert!(exec.starts_with("bookx "), "{exec}");
        assert!(exec.contains("%U"), "links and files need %U: {exec}");
        assert!(d.contains(&format!("Icon={}", crate::config::APP_ID)));
        assert!(d.contains("Type=Application"));
        let icon = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/icons/hicolor/scalable/apps")
            .join(format!("{}.svg", crate::config::APP_ID));
        assert!(icon.exists(), "{}", icon.display());
    }

    #[test]
    fn the_metainfo_matches_the_desktop_file() {
        let x = data("com.adhadse.Bookx.metainfo.xml");
        let doc = roxmltree::Document::parse(&x).expect("metainfo is XML");
        let text_of = |tag: &str| {
            doc.descendants()
                .find(|n| n.has_tag_name(tag))
                .and_then(|n| n.text())
                .unwrap_or("")
                .to_string()
        };
        assert_eq!(text_of("id"), crate::config::APP_ID);
        assert_eq!(
            text_of("launchable"),
            format!("{}.desktop", crate::config::APP_ID)
        );
        let provided: BTreeSet<String> = doc
            .descendants()
            .filter(|n| n.has_tag_name("mediatype"))
            .filter_map(|n| n.text().map(str::to_string))
            .collect();
        assert_eq!(provided, desktop_mime_types());
        assert!(
            doc.descendants().any(|n| n.has_tag_name("content_rating")),
            "OARS rating"
        );
        assert!(doc
            .descendants()
            .any(|n| n.has_tag_name("translation") && n.text() == Some("bookx")));
        let version = crate::config::VERSION;
        assert!(
            doc.descendants()
                .any(|n| n.has_tag_name("release") && n.attribute("version") == Some(version)),
            "metainfo has no release for {version}"
        );
        // screenshots point at images that exist in the wiki assets
        for img in doc.descendants().filter(|n| n.has_tag_name("image")) {
            let url = img.text().unwrap_or("");
            let name = url.rsplit('/').next().unwrap();
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("wiki/docs/assets")
                .join(name);
            assert!(p.exists(), "screenshot {url}: {} is missing", p.display());
        }
    }

    #[test]
    fn media_types_shared_mime_info_lacks_are_defined_by_bookx() {
        let x = data("com.adhadse.Bookx.mime.xml");
        let doc = roxmltree::Document::parse(&x).expect("mime XML");
        let defined: BTreeSet<String> = doc
            .descendants()
            .filter(|n| n.has_tag_name("mime-type"))
            .filter_map(|n| n.attribute("type").map(str::to_string))
            .collect();
        for t in [
            "application/x-kobo-epub+zip",
            "application/vnd.amazon.ebook",
        ] {
            assert!(defined.contains(t), "{t} must be defined in the mime file");
        }
        // Types we define must really be ones the desktop file registers.
        for t in &defined {
            assert!(
                desktop_mime_types().contains(t),
                "{t} is defined but unused"
            );
        }
        // The globs cover the extensions that need them.
        let globs: String = doc
            .descendants()
            .filter(|n| n.has_tag_name("glob"))
            .filter_map(|n| n.attribute("pattern"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(globs.contains("*.kepub") && globs.contains("*.azw"));
    }

    #[test]
    fn the_flatpak_manifest_grants_only_what_bookx_uses() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("build-aux/com.adhadse.Bookx.json");
        let m: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        assert_eq!(m["id"], crate::config::APP_ID);
        let args: Vec<&str> = m["finish-args"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|a| a.as_str())
            .collect();
        for needed in [
            "--share=network",
            "--socket=wayland",
            "--filesystem=xdg-documents",
            "--filesystem=/run/media",
            "--filesystem=xdg-run/gvfs",
            "--socket=pulseaudio",
            "--own-name=org.mpris.MediaPlayer2.Bookx.*",
        ] {
            assert!(args.contains(&needed), "missing {needed}");
        }
        for broad in ["--filesystem=host", "--filesystem=home", "--device=all"] {
            assert!(!args.contains(&broad), "{broad} is broader than needed");
        }
    }
}
