//! Safe mode (X19): `bookx --safe-mode` (or `BOOKX_SAFE_MODE=1`) starts Bookx the way a first run would,
//! to get out of a crash loop caused by a plugin, a patch, a setting or a network service:
//!
//! * plugins and patches are not loaded (their folders are replaced by empty ones for this run);
//! * network features are off: the sharing server, the MCP server, KOSync, and every fetch by the app
//!   (`bookx_sources::fetch::set_offline`);
//! * the settings start at their defaults, and **nothing is saved** to the settings file while safe mode
//!   is on, so the real settings come back at the next normal start.
//!
//! The library and the books are untouched and open as usual.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether the arguments (without the program name) or the environment ask for safe mode.
pub fn requested(args: &[String], env: Option<&str>) -> bool {
    args.iter().any(|a| a == "--safe-mode")
        || matches!(env, Some(v) if !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}

/// Whether this run is in safe mode.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Reads the command line and the environment; call first thing in `main`.
pub fn init() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !requested(&args, std::env::var("BOOKX_SAFE_MODE").ok().as_deref()) {
        return;
    }
    enter();
}

/// Switches safe mode on for the rest of the process.
pub fn enter() {
    ACTIVE.store(true, Ordering::Relaxed);
    let empty = std::env::temp_dir().join(format!("bookx-safe-mode-{}", std::process::id()));
    let plugins = empty.join("plugins");
    let patches = empty.join("patches");
    let _ = std::fs::create_dir_all(&plugins);
    let _ = std::fs::create_dir_all(&patches);
    // Only this process (and children) see these: plugin and patch discovery read them (`plugins::plugins_dir`).
    std::env::set_var("BOOKX_PLUGINS", &plugins);
    std::env::set_var("BOOKX_PATCHES", &patches);
    crate::privacy::apply(true);
}

/// What the start-up note says is switched off.
pub fn summary() -> String {
    crate::i18n::tr(
        "Safe mode: plugins, patches and network features are off, settings are at their defaults and are not saved.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_flag_or_the_variable_asks_for_it() {
        assert!(requested(&a(&["--safe-mode"]), None));
        assert!(requested(&a(&["x.epub", "--safe-mode"]), None));
        assert!(requested(&[], Some("1")));
        assert!(requested(&[], Some("yes")));
        assert!(!requested(&[], Some("0")));
        assert!(!requested(&[], Some("")));
        assert!(!requested(&[], Some("false")));
        assert!(!requested(&[], None));
        assert!(!requested(&a(&["--safe-mode-not"]), None));
    }
}
