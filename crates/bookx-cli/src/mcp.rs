//! `bookx-cli mcp`: the Model Context Protocol server (stdio, or Streamable HTTP with a token).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use bookx_mcp::{Config, WritePolicy};

use crate::Env;

#[derive(clap::Args, Debug)]
pub struct McpArgs {
    /// Serve Streamable HTTP on ADDR:PORT (for example 127.0.0.1:8765) instead of stdio
    #[arg(long, value_name = "ADDR:PORT")]
    http: Option<String>,
    /// Bearer token for --http (default: $BOOKX_MCP_TOKEN, else a random one is made and printed)
    #[arg(long)]
    token: Option<String>,
    /// Let assistants change the library (metadata, tags, notes, imports); undoable in the app.
    /// Refused while the Bookx app has the library open.
    #[arg(long)]
    allow_write: bool,
    /// Folder add_book_from_path may import from (repeatable; needs --allow-write)
    #[arg(long = "allow-dir", value_name = "DIR")]
    allow_dir: Vec<PathBuf>,
    /// Let fetch_metadata ask Open Library / Google Books / Wikidata (needs --allow-write)
    #[arg(long)]
    allow_network: bool,
    /// Reading statistics file (default: the app's stats.json)
    #[arg(long)]
    stats: Option<PathBuf>,
    /// Serve HTTPS with this PEM certificate chain (needs --tls-key)
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// Serve HTTPS with this PEM private key (needs --tls-cert)
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
    /// Another web origin that may call the HTTP endpoint (repeatable)
    #[arg(long = "allow-origin")]
    allow_origin: Vec<String>,
}

fn default_stats() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("stats.json"))
}

/// Splits `ADDR:PORT` (IPv6 as `[::1]:8765`).
pub fn split_addr(s: &str) -> Result<(String, u16)> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("--http needs ADDR:PORT, for example 127.0.0.1:8765"))?;
    let port: u16 = port
        .parse()
        .map_err(|_| anyhow::anyhow!("`{port}` is not a port number"))?;
    Ok((host.trim_matches(['[', ']']).to_string(), port))
}

pub fn run(
    a: McpArgs,
    root: &Path,
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32> {
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    if (!a.allow_dir.is_empty() || a.allow_network) && !a.allow_write {
        bail!("--allow-dir and --allow-network need --allow-write");
    }
    let mut cfg = Config::new(root);
    if a.allow_write {
        cfg.write = WritePolicy::Locked;
    }
    cfg.allow_dirs = a.allow_dir.clone();
    cfg.stats_path = a.stats.clone().or_else(default_stats);
    if a.allow_network {
        cfg.allow_network = true;
        cfg.fetcher = Some(Arc::new(bookx_sources::fetch::UreqFetcher::new()));
    }
    let server = bookx_mcp::new_server(cfg);

    let Some(addr) = &a.http else {
        let stdin = std::io::BufReader::new(std::io::stdin());
        bookx_mcp::stdio::serve(server, stdin, std::io::stdout())?;
        return Ok(0);
    };

    let (bind, port) = split_addr(addr)?;
    let token = a.token.clone().or_else(|| {
        std::env::var("BOOKX_MCP_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
    });
    let generated = token.is_none();
    let token = token.unwrap_or_else(bookx_mcp::generate_token);
    let mut http = bookx_mcp::http::HttpConfig::new(&token);
    http.bind = bind;
    http.port = port;
    http.allowed_origins = a.allow_origin.clone();
    if let (Some(cert), Some(key)) = (&a.tls_cert, &a.tls_key) {
        http.tls = Some(bookx_serve::net::tls_config(cert, key)?);
    }
    let handle = bookx_mcp::http::start(server, http)?;
    writeln!(out, "MCP server on {}", handle.url())?;
    if generated {
        writeln!(err, "token: {token}")?;
    }
    out.flush()?;
    while !env.stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
    handle.shutdown();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_split() {
        assert_eq!(
            split_addr("127.0.0.1:8765").unwrap(),
            ("127.0.0.1".into(), 8765)
        );
        assert_eq!(split_addr("[::1]:80").unwrap(), ("::1".into(), 80));
        assert!(split_addr("localhost").is_err());
        assert!(split_addr("localhost:http").is_err());
    }
}
