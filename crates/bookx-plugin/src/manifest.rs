//! `plugin.toml`: parsing and validation.
//!
//! ```toml
//! id = "org.example.openlib"          # [a-z0-9][a-z0-9._-]{0,63}
//! name = "Example metadata source"
//! version = "1.2.3"                   # digits separated by dots
//! api_version = 1                     # must equal `API_VERSION`
//! kind = "metadata-source"            # metadata-source | format-reader | transform | format-writer | device
//! wasm = "plugin.wasm"                # optional, file inside the plugin folder
//! description = "…"                   # optional
//! permissions = ["network:openlibrary.org"]
//! extensions = ["fb3"]                # format-reader only (required there)
//! extension = "fb3"                   # format-writer only (required there)
//! memory_pages = 512                  # optional, 64 KiB pages, 1..=4096
//! fuel = 100000000                    # optional, instruction budget per call
//! timeout_ms = 5000                   # optional, 1..=60000
//! ```

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

/// The only ABI version this host speaks.
pub const API_VERSION: u32 = 1;
pub const DEFAULT_MEMORY_PAGES: u32 = 512;
pub const MAX_MEMORY_PAGES: u32 = 4096;
pub const DEFAULT_FUEL: u64 = 100_000_000;
pub const MAX_FUEL: u64 = 10_000_000_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
pub const MAX_TIMEOUT_MS: u64 = 60_000;
/// Upper bound on the size of a manifest file we are willing to parse.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginKind {
    MetadataSource,
    FormatReader,
    Transform,
    FormatWriter,
    Device,
}

impl PluginKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PluginKind::MetadataSource => "metadata-source",
            PluginKind::FormatReader => "format-reader",
            PluginKind::Transform => "transform",
            PluginKind::FormatWriter => "format-writer",
            PluginKind::Device => "device",
        }
    }
}

/// A capability the user grants to a plugin. Everything not listed is denied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permission {
    /// `http_get` may contact this exact host name (lower case, no port, no wildcard).
    Network(String),
}

impl Permission {
    /// Parses `network:<host>`. `fs:read` / `fs:write` and everything else are rejected.
    pub fn parse(s: &str) -> Result<Permission> {
        if s == "fs:read" || s == "fs:write" || s.starts_with("fs:") {
            bail!("permission `{s}` is never granted in API v1");
        }
        let Some(host) = s.strip_prefix("network:") else {
            bail!("unknown permission `{s}`");
        };
        if host.is_empty() {
            bail!("permission `{s}` has no host");
        }
        if host.contains('*') {
            bail!("permission `{s}`: wildcards are not allowed");
        }
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            || host.starts_with('.')
            || host.ends_with('.')
            || host.starts_with('-')
            || host.contains("..")
        {
            bail!("permission `{s}`: `{host}` is not a plain host name");
        }
        Ok(Permission::Network(host.to_ascii_lowercase()))
    }

    pub fn to_manifest_string(&self) -> String {
        match self {
            Permission::Network(h) => format!("network:{h}"),
        }
    }
}

/// Resource limits applied to every call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub memory_pages: u32,
    pub fuel: u64,
    pub timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_pages: DEFAULT_MEMORY_PAGES,
            fuel: DEFAULT_FUEL,
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    name: String,
    version: String,
    api_version: u32,
    kind: String,
    #[serde(default)]
    wasm: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    extension: Option<String>,
    #[serde(default)]
    memory_pages: Option<u32>,
    #[serde(default)]
    fuel: Option<u64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

/// A validated manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub wasm: String,
    pub description: String,
    pub permissions: Vec<Permission>,
    pub extensions: Vec<String>,
    pub extension: Option<String>,
    pub limits: Limits,
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    id.len() <= 64
        && chars.all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
        })
}

fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 32
        && v.split('.')
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn valid_extension(e: &str) -> bool {
    !e.is_empty()
        && e.len() <= 16
        && e.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

fn valid_wasm_name(w: &str) -> bool {
    w.ends_with(".wasm")
        && w.len() > ".wasm".len()
        && !w.contains('/')
        && !w.contains('\\')
        && !w.contains("..")
        && !w.contains('\0')
}

impl Manifest {
    /// Parses and validates manifest text (does not touch the file system).
    pub fn parse(text: &str) -> Result<Manifest> {
        let raw: RawManifest = toml::from_str(text).map_err(|e| anyhow!("invalid TOML: {e}"))?;
        if !valid_id(&raw.id) {
            bail!(
                "invalid id `{}` (use lower case letters, digits, `.`, `_`, `-`; max 64)",
                raw.id
            );
        }
        if raw.name.trim().is_empty() {
            bail!("name must not be empty");
        }
        if raw.name.len() > 128 {
            bail!("name is longer than 128 bytes");
        }
        if !valid_version(&raw.version) {
            bail!(
                "invalid version `{}` (digits separated by dots)",
                raw.version
            );
        }
        if raw.api_version != API_VERSION {
            bail!(
                "api_version {} is not supported (this host speaks {API_VERSION})",
                raw.api_version
            );
        }
        let kind = match raw.kind.as_str() {
            "metadata-source" => PluginKind::MetadataSource,
            "format-reader" => PluginKind::FormatReader,
            "transform" => PluginKind::Transform,
            "format-writer" => PluginKind::FormatWriter,
            "device" => PluginKind::Device,
            other => bail!("unknown kind `{other}`"),
        };
        let wasm = raw.wasm.unwrap_or_else(|| "plugin.wasm".to_string());
        if !valid_wasm_name(&wasm) {
            bail!("invalid wasm file name `{wasm}` (must be a `*.wasm` file inside the plugin folder)");
        }
        let mut permissions: Vec<Permission> = Vec::new();
        for p in &raw.permissions {
            let perm = Permission::parse(p)?;
            if !permissions.contains(&perm) {
                permissions.push(perm);
            }
        }
        let mut extensions = Vec::new();
        for e in &raw.extensions {
            let e = e.to_ascii_lowercase();
            if !valid_extension(&e) {
                bail!("invalid extension `{e}` (lower case letters and digits, no dot)");
            }
            if !extensions.contains(&e) {
                extensions.push(e);
            }
        }
        let extension = match raw.extension {
            Some(e) => {
                let e = e.to_ascii_lowercase();
                if !valid_extension(&e) {
                    bail!("invalid extension `{e}` (lower case letters and digits, no dot)");
                }
                Some(e)
            }
            None => None,
        };
        match kind {
            PluginKind::FormatReader if extensions.is_empty() => {
                bail!("a format-reader needs `extensions = [...]`")
            }
            PluginKind::FormatWriter if extension.is_none() => {
                bail!("a format-writer needs `extension = \"...\"`")
            }
            _ => {}
        }
        let mut limits = Limits::default();
        if let Some(p) = raw.memory_pages {
            if p == 0 || p > MAX_MEMORY_PAGES {
                bail!("memory_pages must be 1..={MAX_MEMORY_PAGES}, got {p}");
            }
            limits.memory_pages = p;
        }
        if let Some(f) = raw.fuel {
            if f == 0 || f > MAX_FUEL {
                bail!("fuel must be 1..={MAX_FUEL}, got {f}");
            }
            limits.fuel = f;
        }
        if let Some(t) = raw.timeout_ms {
            if t == 0 || t > MAX_TIMEOUT_MS {
                bail!("timeout_ms must be 1..={MAX_TIMEOUT_MS}, got {t}");
            }
            limits.timeout_ms = t;
        }
        Ok(Manifest {
            id: raw.id,
            name: raw.name,
            version: raw.version,
            kind,
            wasm,
            description: raw.description,
            permissions,
            extensions,
            extension,
            limits,
        })
    }

    /// Reads `<dir>/plugin.toml`, validates it and checks that the wasm file exists.
    pub fn load(dir: &Path) -> Result<Manifest> {
        let path = dir.join("plugin.toml");
        let meta = std::fs::metadata(&path).with_context(|| format!("no {}", path.display()))?;
        if meta.len() > MAX_MANIFEST_BYTES {
            bail!("plugin.toml is larger than {MAX_MANIFEST_BYTES} bytes");
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let m = Manifest::parse(&text)?;
        let wasm = dir.join(&m.wasm);
        if !wasm.is_file() {
            bail!("wasm file `{}` is missing", m.wasm);
        }
        Ok(m)
    }

    pub fn wasm_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.wasm)
    }

    /// Hosts `http_get` may contact.
    pub fn allowed_hosts(&self) -> Vec<&str> {
        self.permissions
            .iter()
            .map(|Permission::Network(h)| h.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"
id = "a.b"
name = "A"
version = "1.0"
api_version = 1
kind = "transform"
"#;

    fn with(extra: &str) -> String {
        format!("{MIN}\n{extra}")
    }

    fn err(text: &str) -> String {
        format!("{:#}", Manifest::parse(text).unwrap_err())
    }

    #[test]
    fn minimal_manifest_gets_defaults() {
        let m = Manifest::parse(MIN).unwrap();
        assert_eq!(m.id, "a.b");
        assert_eq!(m.wasm, "plugin.wasm");
        assert_eq!(m.limits, Limits::default());
        assert!(m.permissions.is_empty());
        assert_eq!(m.kind, PluginKind::Transform);
        assert_eq!(m.description, "");
    }

    #[test]
    fn full_manifest_round_values() {
        let m = Manifest::parse(
            r#"
id = "org.x-y_z.1"
name = "Reader"
version = "1.2.3"
api_version = 1
kind = "format-reader"
wasm = "r.wasm"
description = "d"
permissions = ["network:Example.com", "network:example.com", "network:a-b.org"]
extensions = ["FB3", "fb3", "x1"]
memory_pages = 10
fuel = 5
timeout_ms = 7
"#,
        )
        .unwrap();
        assert_eq!(m.wasm, "r.wasm");
        assert_eq!(m.allowed_hosts(), vec!["example.com", "a-b.org"]);
        assert_eq!(m.extensions, vec!["fb3", "x1"]);
        assert_eq!(
            m.limits,
            Limits {
                memory_pages: 10,
                fuel: 5,
                timeout_ms: 7
            }
        );
        assert_eq!(m.kind.as_str(), "format-reader");
    }

    #[test]
    fn every_kind_parses() {
        for (k, kind) in [
            ("metadata-source", PluginKind::MetadataSource),
            ("transform", PluginKind::Transform),
            ("device", PluginKind::Device),
        ] {
            let t = MIN.replace("\"transform\"", &format!("\"{k}\""));
            let m = Manifest::parse(&t).unwrap();
            assert_eq!(m.kind, kind);
            assert_eq!(kind.as_str(), k);
        }
        let t = MIN.replace("\"transform\"", "\"format-writer\"") + "extension = \"Txt\"\n";
        assert_eq!(
            Manifest::parse(&t).unwrap().extension.as_deref(),
            Some("txt")
        );
        assert_eq!(PluginKind::FormatWriter.as_str(), "format-writer");
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let e = err(&MIN.replace("transform", "teleporter"));
        assert!(e.contains("unknown kind `teleporter`"), "{e}");
    }

    #[test]
    fn api_version_mismatch_is_rejected() {
        for v in [0, 2, 99] {
            let e = err(&MIN.replace("api_version = 1", &format!("api_version = {v}")));
            assert!(
                e.contains(&format!("api_version {v} is not supported")),
                "{e}"
            );
        }
    }

    #[test]
    fn id_rules() {
        for bad in ["", "A", "-a", ".a", "a b", "a/b", "ä", &"a".repeat(65)] {
            let e = err(&MIN.replace("a.b", bad));
            assert!(e.contains("invalid id"), "{bad}: {e}");
        }
        for good in ["a", "0", "a-b_c.d", &"a".repeat(64)] {
            assert!(Manifest::parse(&MIN.replace("a.b", good)).is_ok(), "{good}");
        }
    }

    #[test]
    fn name_rules() {
        assert!(err(&MIN.replace("name = \"A\"", "name = \"  \"")).contains("name must not"));
        let long = format!("name = \"{}\"", "n".repeat(129));
        assert!(err(&MIN.replace("name = \"A\"", &long)).contains("longer than 128"));
    }

    #[test]
    fn version_rules() {
        for bad in ["", "1.", ".1", "1..2", "v1", "1.0-beta", "1 0"] {
            let e = err(&MIN.replace("1.0", bad));
            assert!(e.contains("invalid version"), "{bad}: {e}");
        }
        assert!(err(&MIN.replace("1.0", &"1".repeat(33))).contains("invalid version"));
    }

    #[test]
    fn permission_rules() {
        for (p, msg) in [
            ("fs:read", "never granted"),
            ("fs:write", "never granted"),
            ("fs:anything", "never granted"),
            ("exec:ls", "unknown permission"),
            ("", "unknown permission"),
            ("network:", "no host"),
            ("network:*.example.com", "wildcards"),
            ("network:https://x.org", "not a plain host"),
            ("network:x.org/path", "not a plain host"),
            ("network:x.org:80", "not a plain host"),
            ("network:.x.org", "not a plain host"),
            ("network:x.org.", "not a plain host"),
            ("network:-x.org", "not a plain host"),
            ("network:x..org", "not a plain host"),
        ] {
            let e = err(&with(&format!("permissions = [\"{p}\"]")));
            assert!(e.contains(msg), "{p}: {e}");
        }
    }

    #[test]
    fn permission_roundtrip_string() {
        let p = Permission::parse("network:Foo.Org").unwrap();
        assert_eq!(p, Permission::Network("foo.org".into()));
        assert_eq!(p.to_manifest_string(), "network:foo.org");
    }

    #[test]
    fn wasm_name_rules() {
        for bad in [
            "a.txt",
            ".wasm",
            "../x.wasm",
            "sub/x.wasm",
            "sub\\\\x.wasm",
            "a..wasm",
        ] {
            let e = err(&with(&format!("wasm = \"{bad}\"")));
            assert!(e.contains("invalid wasm file name"), "{bad}: {e}");
        }
    }

    #[test]
    fn reader_needs_extensions_and_writer_needs_extension() {
        let r = MIN.replace("transform", "format-reader");
        assert!(err(&r).contains("needs `extensions"));
        let w = MIN.replace("transform", "format-writer");
        assert!(err(&w).contains("needs `extension"));
        assert!(err(&(r.clone() + "extensions = [\"a.b\"]")).contains("invalid extension"));
        assert!(err(&(r + "extensions = [\"\"]")).contains("invalid extension"));
        assert!(err(&(w + "extension = \".txt\"")).contains("invalid extension"));
    }

    #[test]
    fn limit_bounds() {
        for (k, bad) in [
            ("memory_pages", "0"),
            ("memory_pages", "4097"),
            ("fuel", "0"),
            ("fuel", "10000000001"),
            ("timeout_ms", "0"),
            ("timeout_ms", "60001"),
        ] {
            let e = err(&with(&format!("{k} = {bad}")));
            assert!(e.contains(k), "{k}={bad}: {e}");
        }
        let ok = with("memory_pages = 4096\nfuel = 10000000000\ntimeout_ms = 60000");
        assert!(Manifest::parse(&ok).is_ok());
    }

    #[test]
    fn unknown_fields_and_bad_toml_are_rejected() {
        assert!(err(&with("surprise = 1")).contains("invalid TOML"));
        assert!(err("this is = = not toml").contains("invalid TOML"));
        assert!(err("").contains("invalid TOML"));
        assert!(
            err(&MIN.replace("api_version = 1", "api_version = \"1\"")).contains("invalid TOML")
        );
    }

    #[test]
    fn load_checks_files() {
        let dir = std::env::temp_dir().join(format!("bookx-plugin-man-{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let e = format!("{:#}", Manifest::load(&dir).unwrap_err());
        assert!(e.contains("plugin.toml"), "{e}");
        std::fs::write(dir.join("plugin.toml"), MIN).unwrap();
        let e = format!("{:#}", Manifest::load(&dir).unwrap_err());
        assert!(e.contains("`plugin.wasm` is missing"), "{e}");
        std::fs::write(dir.join("plugin.wasm"), b"x").unwrap();
        let m = Manifest::load(&dir).unwrap();
        assert_eq!(m.wasm_path(&dir), dir.join("plugin.wasm"));
        std::fs::write(
            dir.join("plugin.toml"),
            format!("{MIN}\n# {}", "x".repeat(70_000)),
        )
        .unwrap();
        let e = format!("{:#}", Manifest::load(&dir).unwrap_err());
        assert!(e.contains("larger than"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn uuid_like() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "{}-{}-{:?}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed),
            std::thread::current().id()
        )
        .replace(['(', ')'], "")
    }
}
