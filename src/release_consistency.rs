//! Tests only: the version, changelog, metainfo, package metadata and release workflow agree
//! (the same checks as `build-aux/release-check.sh`, but pure and part of `cargo test`).

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn read(rel: &str) -> String {
        let p = root().join(rel);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    fn version_of(cargo_toml: &str) -> Option<String> {
        cargo_toml.lines().find_map(|l| {
            l.strip_prefix("version = \"")
                .and_then(|r| r.strip_suffix('"'))
                .map(str::to_string)
        })
    }

    /// The newest `## [x.y.z]` heading of a Keep-a-Changelog file.
    fn changelog_version(text: &str) -> Option<String> {
        text.lines().find_map(|l| {
            let r = l.strip_prefix("## [")?;
            let v = &r[..r.find(']')?];
            v.chars().next()?.is_ascii_digit().then(|| v.to_string())
        })
    }

    fn metainfo_version(text: &str) -> Option<String> {
        let i = text.find("<release version=\"")? + "<release version=\"".len();
        Some(text[i..i + text[i..].find('"')?].to_string())
    }

    #[test]
    fn version_parsers_work() {
        assert_eq!(
            version_of("[package]\nname = \"x\"\nversion = \"1.2.3\"\n").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            changelog_version("# Changelog\n## [Unreleased]\n## [0.4.1] - 2026-01-01\n## [0.4.0]")
                .as_deref(),
            Some("0.4.1")
        );
        assert_eq!(
            metainfo_version("<releases><release version=\"2.0\" date=\"x\">").as_deref(),
            Some("2.0")
        );
        assert_eq!(changelog_version("## [Unreleased]"), None);
    }

    #[test]
    fn every_crate_the_changelog_and_the_metainfo_agree_on_the_version() {
        let version = version_of(&read("Cargo.toml")).unwrap();
        for entry in std::fs::read_dir(root().join("crates")).unwrap() {
            let dir = entry.unwrap().path();
            let manifest = dir.join("Cargo.toml");
            if manifest.is_file() {
                let v = version_of(&std::fs::read_to_string(&manifest).unwrap());
                assert_eq!(
                    v.as_deref(),
                    Some(version.as_str()),
                    "{}",
                    manifest.display()
                );
            }
        }
        assert_eq!(
            changelog_version(&read("CHANGELOG.md")).as_deref(),
            Some(version.as_str()),
            "newest CHANGELOG.md heading"
        );
        assert_eq!(
            metainfo_version(&read("data/com.adhadse.Bookx.metainfo.xml")).as_deref(),
            Some(version.as_str()),
            "newest metainfo release"
        );
    }

    /// Source files named by the deb and rpm metadata (`data/…`, docs) must exist; build outputs
    /// (`target/release/…`) are checked by name only.
    #[test]
    fn deb_and_rpm_assets_exist_and_ship_both_programs() {
        let cargo = read("Cargo.toml");
        let at = cargo.find("[package.metadata.deb]").expect("deb metadata");
        let meta = &cargo[at..];
        for needle in [
            "[package.metadata.generate-rpm]",
            "target/release/bookx\"",
            "target/release/bookx-cli\"",
            "usr/share/applications/",
            "/usr/share/metainfo/com.adhadse.Bookx.metainfo.xml",
        ] {
            assert!(meta.contains(needle), "packaging metadata lacks {needle}");
        }
        let mut seen = 0;
        for part in meta.split('"') {
            let is_source = part.starts_with("data/")
                || ["README.md", "NOTICE.md", "CHANGELOG.md", "LICENSE"].contains(&part);
            if is_source {
                assert!(Path::new(&root().join(part)).exists(), "missing {part}");
                seen += 1;
            }
        }
        assert!(seen >= 10, "only {seen} asset sources found");
    }

    #[test]
    fn deb_and_rpm_install_the_same_paths_as_the_makefile() {
        let make = read("Makefile");
        let cargo = read("Cargo.toml");
        for dest in [
            "share/applications/com.adhadse.Bookx.desktop",
            "share/metainfo/com.adhadse.Bookx.metainfo.xml",
            "share/mime/packages/com.adhadse.Bookx.xml",
            "share/icons/hicolor/scalable/apps/com.adhadse.Bookx.svg",
            "share/icons/hicolor/symbolic/apps/com.adhadse.Bookx-symbolic.svg",
        ] {
            let in_make = dest.replace("com.adhadse.Bookx", "$(APP_ID)");
            assert!(make.contains(&in_make), "Makefile lacks {in_make}");
            assert!(
                cargo.contains(dest.rsplit('/').next().unwrap()),
                "Cargo.toml lacks {dest}"
            );
        }
    }

    #[test]
    fn the_release_workflow_checks_before_it_publishes() {
        let wf = read(".github/workflows/release.yml");
        assert!(wf.contains("tags: [\"v*\"]"), "runs on version tags");
        assert!(wf.contains("release-check.sh"), "runs the release check");
        assert!(wf.contains("SHA256SUMS"), "publishes checksums");
        assert!(wf.contains("--locked"), "builds with the lock file");
        assert!(
            !wf.contains("git push") && !wf.contains("git tag"),
            "the workflow never creates tags"
        );
    }

    #[test]
    fn the_flatpak_manifest_stays_minimal() {
        let m = read("build-aux/com.adhadse.Bookx.json");
        for broad in [
            "--filesystem=host",
            "--filesystem=home",
            "--device=all",
            "--socket=session-bus",
            "--socket=system-bus",
            "--socket=ssh-auth",
        ] {
            assert!(!m.contains(broad), "too broad: {broad}");
        }
        assert!(m.contains("\"org.gnome.Platform\""));
    }

    /// `(major, minor, patch)` of the locked version of a crate.
    fn locked(lock: &str, name: &str) -> Option<(u32, u32, u32)> {
        let at = lock.find(&format!("[[package]]\nname = \"{name}\"\nversion = \""))?;
        let rest = &lock[at..];
        let v = rest
            .lines()
            .nth(2)?
            .strip_prefix("version = \"")?
            .strip_suffix('"')?;
        let mut it = v.split('.').map(|n| n.parse::<u32>().ok());
        Some((it.next()??, it.next()??, it.next()??))
    }

    /// RUSTSEC-2026-0560 to 0565 (SMTP command injection, forged replies, panics): fixed in lettre 0.11.24.
    #[test]
    fn lettre_is_past_the_smtp_advisories() {
        let lock = read("Cargo.lock");
        let v = locked(&lock, "lettre").expect("lettre is locked");
        assert!(
            v >= (0, 11, 24),
            "lettre {v:?} has known advisories (RUSTSEC-2026-0560..0565)"
        );
        assert_eq!(
            locked("[[package]]\nname = \"x\"\nversion = \"1.2.3\"\n", "x"),
            Some((1, 2, 3))
        );
    }
}
