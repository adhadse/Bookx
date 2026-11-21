//! The MCP server inside the app (Preferences → MCP): settings, the token in the system
//! keyring, starting and stopping the HTTP server, and the connection snippets for assistants.
//! The server itself is the `bookx-mcp` crate; the page is `ui/mcp_settings.rs`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use bookx_mcp::http::{HttpConfig, HttpHandle};
use bookx_mcp::{Config, WritePolicy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const DEFAULT_PORT: u16 = 8765;
const TOKEN_KEY: &str = "mcp-token";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    /// Run the HTTP server while Bookx runs. Off until the user switches it on.
    pub enabled: bool,
    pub port: u16,
    /// Assistants may change the library. Off until the user switches it on.
    pub write: bool,
    /// The token, kept here only when there is no keyring to hold it.
    pub token_fallback: String,
}

impl Default for McpSettings {
    fn default() -> McpSettings {
        McpSettings {
            enabled: false,
            port: DEFAULT_PORT,
            write: false,
            token_fallback: String::new(),
        }
    }
}

impl McpSettings {
    pub fn problem(&self) -> Option<&'static str> {
        (self.port < 1024).then_some("Choose a port between 1024 and 65535.")
    }
}

pub fn settings_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("mcp.json"))
}

pub fn load_from(path: &Path) -> McpSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, s: &McpSettings) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load() -> McpSettings {
    settings_path().map(|p| load_from(&p)).unwrap_or_default()
}

pub fn save(s: &McpSettings) {
    if let Some(p) = settings_path() {
        let _ = save_to(&p, s);
    }
}

/// The access token: from the keyring, made on first use. Without a keyring it is kept in the
/// settings file, which is said on the page.
pub fn token(settings: &mut McpSettings) -> String {
    let vault = crate::secrets::vault();
    let mut t = settings.token_fallback.clone();
    vault.load(&mut [(TOKEN_KEY.to_string(), &mut t)]);
    if t.is_empty() {
        t = bookx_mcp::generate_token();
        store_token(settings, &t);
    }
    t
}

fn store_token(settings: &mut McpSettings, token: &str) {
    let mut copy = token.to_string();
    crate::secrets::vault().persist(&mut [(TOKEN_KEY.to_string(), &mut copy)]);
    // a token the keyring did not take stays in the settings file, so it is not lost
    settings.token_fallback = copy;
    save(settings);
}

pub fn regenerate_token(settings: &mut McpSettings) -> String {
    let t = bookx_mcp::generate_token();
    store_token(settings, &t);
    t
}

/// A token as shown on screen: enough to recognise it, not enough to use it.
pub fn masked(token: &str) -> String {
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••••••{tail}")
}

// -- the running server --------------------------------------------------------------------------------

thread_local! {
    static HOST: RefCell<Option<HttpHandle>> = const { RefCell::new(None) };
}

pub fn running() -> bool {
    HOST.with(|h| h.borrow().is_some())
}

pub fn url() -> Option<String> {
    HOST.with(|h| h.borrow().as_ref().map(|h| h.url()))
}

pub fn stop() {
    if let Some(h) = HOST.with(|h| h.borrow_mut().take()) {
        h.endpoint.server.app.stop();
        h.shutdown();
    }
}

/// (Re)starts the server for the library in `root` and returns its address.
pub fn start(root: &Path, settings: &mut McpSettings) -> Result<String> {
    stop();
    if let Some(p) = settings.problem() {
        bail!(p);
    }
    let token = token(settings);
    let mut cfg = Config::new(root);
    cfg.write = if settings.write {
        // this process is the app and holds the library lock
        WritePolicy::InApp
    } else {
        WritePolicy::Off
    };
    cfg.stats_path = crate::config::stats_path();
    cfg.app_name = "bookx".into();
    let server = bookx_mcp::new_server(cfg);
    let mut http = HttpConfig::new(&token);
    http.port = settings.port;
    let handle = bookx_mcp::http::start(server, http).map_err(|e| {
        anyhow::anyhow!(
            "Could not start the assistant server on port {}: {e:#}",
            settings.port
        )
    })?;
    let url = handle.url();
    HOST.with(|h| *h.borrow_mut() = Some(handle));
    Ok(url)
}

/// Called at start-up: runs the server when the user left it switched on.
pub fn autostart(root: &Path) {
    let mut s = load();
    if s.enabled {
        if let Err(e) = start(root, &mut s) {
            tracing::warn!("{e:#}");
        }
    }
}

/// Assistants that talked to the server in the last ten minutes: `(name, calls, seconds idle)`.
pub fn clients() -> Vec<(String, u64, i64)> {
    HOST.with(|h| {
        h.borrow()
            .as_ref()
            .map(|h| {
                h.endpoint
                    .server
                    .app
                    .clients(Some(std::time::Duration::from_secs(600)))
                    .into_iter()
                    .map(|c| {
                        let name = if c.version.is_empty() {
                            c.name.clone()
                        } else {
                            format!("{} {}", c.name, c.version)
                        };
                        (
                            name,
                            c.calls,
                            (chrono::Utc::now() - c.last_seen).num_seconds().max(0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// How the assistant list reads.
pub fn clients_text(list: &[(String, u64, i64)]) -> String {
    if list.is_empty() {
        return crate::i18n::tr("No assistant has connected recently.");
    }
    list.iter()
        .map(|(n, calls, idle)| {
            let when = if *idle < 5 {
                "just now".to_string()
            } else if *idle < 90 {
                format!("{idle} s ago")
            } else {
                format!("{} min ago", idle / 60)
            };
            format!(
                "{n}: {calls} request{}, last {when}",
                if *calls == 1 { "" } else { "s" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// -- connection snippets -------------------------------------------------------------------------------------

/// The assistants the page can write a configuration for.
pub const CLIENTS: [(&str, &str); 4] = [
    ("claude-desktop", "Claude Desktop"),
    ("claude-code", "Claude Code"),
    ("cursor", "Cursor"),
    ("opencode", "opencode"),
];

/// The `bookx-cli` that belongs to this installation: next to the app when it is there, else the
/// name (found on `PATH`).
pub fn cli_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("bookx-cli")))
        .filter(|p| p.exists())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "bookx-cli".into())
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The text to paste for `client`. `url` is `Some` for the HTTP variant, which also needs the token.
pub fn snippet(client: &str, cli: &str, library: &Path, url: Option<(&str, &str)>) -> String {
    let lib = library.display().to_string();
    let args = json!(["--library", lib, "mcp"]);
    let header = |t: &str| format!("Bearer {t}");
    let v: Value = match (client, url) {
        ("claude-desktop", None) => json!({"mcpServers":{"bookx":{"command":cli,"args":args}}}),
        ("claude-desktop", Some((u, t))) => json!({"mcpServers":{"bookx":{"command":"npx",
            "args":["-y","mcp-remote",u,"--header",format!("Authorization:{}", header(t))]}}}),
        ("cursor", None) => json!({"mcpServers":{"bookx":{"command":cli,"args":args}}}),
        ("cursor", Some((u, t))) => {
            json!({"mcpServers":{"bookx":{"url":u,"headers":{"Authorization":header(t)}}}})
        }
        ("opencode", None) => {
            json!({"mcp":{"bookx":{"type":"local","command":[cli,"--library",lib,"mcp"],"enabled":true}}})
        }
        ("opencode", Some((u, t))) => {
            json!({"mcp":{"bookx":{"type":"remote","url":u,"headers":{"Authorization":header(t)},"enabled":true}}})
        }
        ("claude-code", None) => {
            return format!(
                "claude mcp add bookx -- {} --library {} mcp",
                quote(cli),
                quote(&lib)
            );
        }
        ("claude-code", Some((u, t))) => {
            return format!(
                "claude mcp add --transport http bookx {} --header {}",
                quote(u),
                quote(&format!("Authorization: {}", header(t)))
            );
        }
        _ => return String::new(),
    };
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-mcp-host-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn settings_are_off_by_default_and_round_trip() {
        let d = tmp();
        let p = d.join("mcp.json");
        assert_eq!(load_from(&p), McpSettings::default());
        let s = McpSettings::default();
        assert!(!s.enabled && !s.write);
        assert_eq!(s.port, 8765);
        let s2 = McpSettings {
            enabled: true,
            port: 9000,
            write: true,
            token_fallback: "x".into(),
        };
        save_to(&p, &s2).unwrap();
        assert_eq!(load_from(&p), s2);
        // a damaged file means the defaults, and older files without every field still load
        std::fs::write(&p, "{{{").unwrap();
        assert_eq!(load_from(&p), McpSettings::default());
        std::fs::write(&p, r#"{"enabled":true}"#).unwrap();
        assert!(load_from(&p).enabled);
        assert_eq!(load_from(&p).port, 8765);
        assert!(McpSettings {
            port: 80,
            ..Default::default()
        }
        .problem()
        .is_some());
        assert!(s.problem().is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_token_lives_in_the_vault_and_survives_a_restart_of_the_page() {
        let mut s = McpSettings::default();
        let t = token(&mut s);
        assert_eq!(t.len(), 64);
        assert_eq!(token(&mut s), t, "the same token is returned");
        // the test keyring takes it, so nothing stays in the settings
        assert_eq!(s.token_fallback, "");
        let t2 = regenerate_token(&mut s);
        assert_ne!(t, t2);
        assert_eq!(token(&mut s), t2);
        assert!(masked(&t2).ends_with(&t2[60..]) && !masked(&t2).contains(&t2[..8]));
    }

    #[test]
    fn snippets_are_valid_for_each_assistant() {
        let lib = Path::new("/home/me/Bookx Library");
        for (id, _) in CLIENTS {
            let s = snippet(id, "/app/bin/bookx-cli", lib, None);
            assert!(s.contains("/app/bin/bookx-cli"), "{id}");
            assert!(s.contains("Bookx Library"), "{id}");
            assert!(
                !s.contains("Bearer"),
                "the command form carries no token: {id}"
            );
            if id != "claude-code" {
                serde_json::from_str::<Value>(&s).unwrap_or_else(|e| panic!("{id}: {e}"));
            }
            let h = snippet(
                id,
                "bookx-cli",
                lib,
                Some(("http://127.0.0.1:8765/mcp", "TOK")),
            );
            assert!(h.contains("TOK") && h.contains("8765/mcp"), "{id}");
            if id != "claude-code" {
                serde_json::from_str::<Value>(&h).unwrap_or_else(|e| panic!("{id}: {e}"));
            }
        }
        let oc: Value = serde_json::from_str(&snippet("opencode", "bookx-cli", lib, None)).unwrap();
        assert_eq!(oc["mcp"]["bookx"]["command"][3], "mcp");
        let cc = snippet("claude-code", "bookx-cli", Path::new("/it's here"), None);
        assert!(cc.contains(r"'/it'\''s here'"), "{cc}");
        assert_eq!(snippet("nope", "x", lib, None), "");
    }

    #[test]
    fn the_client_list_reads_naturally() {
        assert!(clients_text(&[]).contains("No assistant"));
        let t = clients_text(&[("Claude 1.0".into(), 1, 2), ("Cursor".into(), 5, 300)]);
        assert!(t.contains("Claude 1.0: 1 request, last just now"));
        assert!(t.contains("Cursor: 5 requests, last 5 min ago"));
    }

    #[test]
    fn the_server_starts_stops_and_reports_clients() {
        let d = tmp();
        let root = d.join("lib");
        bookx_store::open_library(&root).unwrap();
        let mut s = McpSettings {
            port: 0,
            ..Default::default()
        };
        // port 0 is below the allowed range for the page, so ask for a free high port by trying
        assert!(start(&root, &mut s).is_err());
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        s.port = port.max(1024);
        let url = start(&root, &mut s).unwrap();
        assert!(running() && url.ends_with("/mcp") && url.contains(&s.port.to_string()));
        assert_eq!(self::url().as_deref(), Some(url.as_str()));
        assert!(clients().is_empty());
        stop();
        assert!(!running());
        let _ = std::fs::remove_dir_all(d);
    }
}
