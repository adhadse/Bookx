//! Privacy: the offline switch and the list of everything Bookx can connect to (roadmap X15,
//! PP-C-8, PP-C-22, PP-G-18). Bookx has no telemetry, no account and no update check; every
//! connection below is something the user started, and offline mode refuses the ones that go
//! through the shared HTTP client, the Wikipedia/translate look-ups and the downloads.
//!
//! The wiki page *Privacy* repeats this list; the test `ledger_matches_the_wiki` keeps them in step.

use crate::i18n::{N_, tr, trf};

/// One thing Bookx can connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connection {
    pub feature: &'static str,
    /// Where it goes.
    pub destination: &'static str,
    /// What leaves the computer.
    pub sends: &'static str,
    /// Whether offline mode stops it.
    pub offline_blocks: bool,
}

/// Every connection Bookx makes, and only when the user uses the feature.
pub const LEDGER: [Connection; 14] = [
    Connection {
        feature: N_("Fetch Metadata and covers"),
        destination: "openlibrary.org, covers.openlibrary.org, googleapis.com (Google Books), query.wikidata.org",
        sends: N_("title, author, ISBN of the books you look up"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Wikipedia look-up in the reader"),
        destination: "<language>.wikipedia.org, upload.wikimedia.org (pictures)",
        sends: N_("the selected word or phrase"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Translate in the reader"),
        destination: "translate.googleapis.com",
        sends: N_("the selected text"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Online catalogues"),
        destination: N_("gutenberg.org, standardebooks.org and the catalogues you add"),
        sends: N_("your search words; books you download"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Add from the Web and news subscriptions"),
        destination: N_("the addresses you give"),
        sends: N_("a plain request for the page or feed"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Check Links in the editor"),
        destination: N_("the web addresses found in the book"),
        sends: N_("a plain request per link"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Dictionary downloads"),
        destination: "download.wikdict.com, github.com",
        sends: N_("the file you ask for"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Voice and model downloads"),
        destination: "huggingface.co, github.com",
        sends: N_("the file you ask for"),
        offline_blocks: true,
    },
    Connection {
        feature: N_("Read aloud with a voice server"),
        destination: N_("the server address you enter"),
        sends: N_("the text being read"),
        offline_blocks: false,
    },
    Connection {
        feature: N_("Send by e-mail"),
        destination: N_("the mail server you enter"),
        sends: N_("the book and the message"),
        offline_blocks: false,
    },
    Connection {
        feature: N_("Cloud folders (WebDAV) for devices and backups"),
        destination: N_("the WebDAV address you enter"),
        sends: N_("book files or encrypted backups"),
        offline_blocks: false,
    },
    Connection {
        feature: N_("Bookx sync (reading position, highlights, notes)"),
        destination: N_("the sync server you enter, or your own Bookx"),
        sends: N_("a book fingerprint and your position"),
        offline_blocks: false,
    },
    Connection {
        feature: N_("Sync: OPDS server and Bookx sync"),
        destination: N_(
            "nothing: other devices connect to this computer, only while sharing is on and only with the password",
        ),
        sends: N_("the books you share"),
        offline_blocks: false,
    },
    Connection {
        feature: N_("Readwise upload"),
        destination: "readwise.io",
        sends: N_("your highlights, when you press Upload"),
        offline_blocks: false,
    },
];

/// Is any network use refused right now (the setting or the `BOOKX_OFFLINE` variable)?
pub fn is_offline() -> bool {
    std::env::var_os("BOOKX_OFFLINE").is_some() || bookx_sources::fetch::is_offline()
}

/// Turns offline mode on or off in the crates that do the downloading.
pub fn apply(on: bool) {
    // safe mode keeps every download off whatever the setting says
    let on = on || crate::safe_mode::active();
    bookx_sources::fetch::set_offline(on);
    bookx_tts::download::set_offline(on);
}

/// The ledger row as one line for a list.
pub fn describe(c: &Connection) -> String {
    let dest = if c.destination.contains(' ') {
        tr(c.destination)
    } else {
        c.destination.to_string()
    };
    let mut line = trf("{} · sends {}", &[&dest, &tr(c.sends)]);
    if !c.offline_blocks {
        line.push_str(&tr(" · not stopped by offline mode"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_is_complete_and_unique() {
        let mut names: Vec<&str> = LEDGER.iter().map(|c| c.feature).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), LEDGER.len());
        assert!(
            LEDGER
                .iter()
                .all(|c| !c.destination.is_empty() && !c.sends.is_empty())
        );
        // things the user types an address for are never blocked silently by a global switch
        assert!(LEDGER.iter().filter(|c| !c.offline_blocks).all(|c| {
            c.destination.contains("you enter")
                || c.destination.starts_with("nothing")
                || c.destination.contains("readwise")
        }));
    }

    #[test]
    fn ledger_matches_the_wiki() {
        let wiki = include_str!("../wiki/docs/privacy.md");
        for c in LEDGER.iter() {
            assert!(
                wiki.contains(c.feature),
                "wiki/docs/privacy.md lacks {}",
                c.feature
            );
        }
    }

    #[test]
    fn descriptions_mention_the_exception() {
        let blocked = describe(&LEDGER[0]);
        assert!(!blocked.contains("not stopped"));
        let free = LEDGER.iter().find(|c| !c.offline_blocks).unwrap();
        assert!(describe(free).contains("not stopped by offline mode"));
    }

    #[test]
    fn there_is_no_telemetry_entry() {
        assert!(
            LEDGER
                .iter()
                .all(|c| !c.feature.to_lowercase().contains("telemetry")
                    && !c.feature.to_lowercase().contains("update")
                    && !c.feature.to_lowercase().contains("analytics"))
        );
    }
}
