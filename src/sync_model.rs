//! "Sync" is the one umbrella for everything that moves books, reading progress, highlights and
//! notes between Bookx and something else. This file is the information architecture and the state
//! the Sync window (task cards) and Preferences → Sync (setup checklist) show, as plain data so it
//! is unit tested without GTK. See `docs/sync-ux-notes.md` for why it is shaped like this.

use crate::i18n::{N_, tr, trf};

/// Title of the umbrella: menu entry, window and Preferences page.
pub const TITLE: &str = N_("Sync");
/// Id of the Preferences page.
pub const PAGE_ID: &str = "sync";
/// Names the single Sync page used to have, still accepted by searches and `prefs:pick=`.
pub const LEGACY_PAGES: [&str; 2] = ["Devices and Sharing", "Devices"];

/// Said once, on the Preferences page: what the name means.
pub const BRAND_EXPLAINED: &str = N_(
    "Bookx sync (extended KOSync) is compatible with KOReader’s KOSync progress protocol. Bookx’s extension also syncs highlights, notes and bookmarks when the Bookx plugin is installed in KOReader. Everything goes over one connection to this computer; no cloud service is involved.",
);

/// How a group of settings reaches the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Wifi,
    Usb,
    Servers,
    Advanced,
}

/// One group of Preferences → Sync, in the order it is shown.
pub struct Group {
    pub id: &'static str,
    pub title: &'static str,
    #[allow(dead_code)] // documents the information architecture; the tests read it
    pub connection: Connection,
    pub order: i32,
}

/// The groups of Preferences → Sync: what most people need first, everything technical in the
/// last one (a collapsed “Advanced” row). Registered by `ui::sync_prefs`.
pub const GROUPS: [Group; 6] = [
    Group {
        id: "checklist",
        title: N_("Setup checklist"),
        connection: Connection::Wifi,
        order: 10,
    },
    Group {
        id: "activity",
        title: N_("Connected devices"),
        connection: Connection::Wifi,
        order: 12,
    },
    Group {
        id: "what",
        title: N_("What Bookx sync does"),
        connection: Connection::Wifi,
        order: 20,
    },
    Group {
        id: "usb",
        title: N_("USB device sync"),
        connection: Connection::Usb,
        order: 30,
    },
    Group {
        id: "servers",
        title: N_("OPDS server"),
        connection: Connection::Servers,
        order: 40,
    },
    Group {
        id: "advanced",
        title: N_("Advanced"),
        connection: Connection::Advanced,
        order: 50,
    },
];

/// Where a group sits on the page.
pub fn order(id: &str) -> i32 {
    GROUPS.iter().find(|g| g.id == id).map_or(1000, |g| g.order)
}

/// The translated title of a group of Preferences → Sync.
pub fn title(id: &str) -> String {
    GROUPS
        .iter()
        .find(|g| g.id == id)
        .map(|g| tr(g.title))
        .unwrap_or_default()
}

/// Does a search or `prefs:pick=` name mean the Sync page under its former name (the whole name)?
pub fn is_legacy_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    LEGACY_PAGES.iter().any(|l| n == l.to_lowercase())
}

// ---- words ------------------------------------------------------------------------------------

/// First screen of the Sync window: what Sync is for, in one plain sentence.
pub const WINDOW_SENTENCE: &str = N_(
    "Sync your Kobo or other KOReader device over Wi-Fi: reading position, highlights, notes and bookmarks. Use USB for devices without KOReader.",
);

/// Second sentence of the Sync window: what the Wi-Fi sync does not do (issue #39: nobody should wait for
/// books to arrive that Bookx never copies).
pub const WINDOW_BOOKS_NOTE: &str = N_("Sync never copies or deletes books. It moves only reading position, highlights, notes and bookmarks, for books that are already in your library. To get books from an e-reader into Bookx, copy the files to this computer and use Add Books. To put books on an e-reader, use Send to Device.");

/// Under the buttons of the empty library: where books on an e-reader come from.
pub const WELCOME_DEVICE_NOTE: &str = N_("Books on an e-reader? Sync never copies or deletes books: copy the files to this computer and use Add Books. To put books on a device, use Send to Device.");

pub const STEP_SERVER: &str = N_("Turn on the Bookx sync server");
pub const STEP_SERVER_HELP: &str = N_(
    "Starts the Bookx sync server on this computer, opens it to your local network so the e-reader can reach it, and creates a library password if you have none (shown under Advanced).",
);
/// The same, shortened for the setup card of the Sync window.
pub const STEP_SERVER_HINT: &str = N_(
    "Starts the server, opens it to your local network and creates a library password if you have none.",
);
/// The persistent banner of the Sync window while the server is open to the network.
pub const LAN_BANNER: &str = N_("Other devices on your network can reach this server");
pub const STEP_ACCOUNT: &str = N_("Create your sync account");
pub const STEP_ACCOUNT_HELP: &str = N_(
    "Choose any user name and password. Copy these into Bookx Sync on the e-reader. This is not the OPDS server password. There is no existing KOReader account. If the server is not on yet, switch on step 1 first.",
);
pub const STEP_PLUGIN: &str = N_("Install the Bookx plugin on KOReader");
pub const STEP_PLUGIN_HELP: &str = N_(
    "Copies the plugin into KOReader’s plugins folder on the connected e-reader, or into a folder you choose. Restart KOReader afterwards. Without the plugin only the reading position is synced.",
);
pub const STEP_TEST: &str = N_("Test on the device");
pub const STEP_TEST_HELP: &str = N_(
    "On the e-reader: open a book → top menu → More tools → Bookx Sync → Sync now. Enter this computer’s address, the user name and the password from step 2.",
);

pub const WHAT_SYNCS: &str = N_(
    "Reading position, highlights, notes and bookmarks, over the same connection, when the Bookx plugin is installed. Without the plugin: reading position only.",
);

/// Said under the fields of the account step and in the Sync window.
pub const ACCOUNT_ONE_LINER: &str =
    N_("No account yet? Open Preferences → Sync and press Register.");
pub const ACCOUNT_NO_SERVER: &str = N_(
    "Start the server first (step 1), or type the address of another sync server under Advanced.",
);
pub const ACCOUNT_KOSYNC_OFF: &str =
    N_("Turn on the Bookx sync server (step 1) first: the server does not take accounts before.");

/// Why Log in and Register cannot work yet, if they cannot: no address, or the address of this computer's server
/// while the Bookx sync server is off. `None` when the buttons can be pressed.
pub fn account_block(
    address: &str,
    own: Option<&str>,
    bookx_sync_on: bool,
) -> Option<&'static str> {
    let address = address.trim().trim_end_matches('/');
    if address.is_empty() {
        Some(ACCOUNT_NO_SERVER)
    } else if own.is_some_and(|u| u == address) && !bookx_sync_on {
        Some(ACCOUNT_KOSYNC_OFF)
    } else {
        None
    }
}

/// The title and text of the account row of step 2: who is logged in, or why not. The password
/// can be missing although a user name is saved when the keyring was locked or is gone.
pub fn account_status(
    user: &str,
    has_password: bool,
    logged_in: bool,
    keyring_ok: bool,
) -> (String, String) {
    let user = user.trim();
    if user.is_empty() {
        (
            tr("Log in"),
            tr(
                "Register creates the account on the server; Log in checks it. The result shows here.",
            ),
        )
    } else if !has_password {
        let why = if keyring_ok {
            tr("Enter the password.")
        } else {
            tr(
                "The password could not be read from the system keyring (locked or not running). Unlock it and restart Bookx, or type the password again.",
            )
        };
        (trf("Password needed for {}", &[&user]), why)
    } else if logged_in {
        (
            trf("Logged in as {}", &[&user]),
            tr("Reading positions sync with this account. Press Log in to check it again."),
        )
    } else {
        (
            trf("Account saved: {}", &[&user]),
            tr("Not checked yet. Press Log in to check it."),
        )
    }
}

/// The tooltip of Log in and Register while they cannot be pressed.
pub const ACCOUNT_BLOCKED_TIP: &str = N_("Turn on the server first");

/// Where the Bookx plugin is in KOReader.
pub const KOREADER_PATH: &str = N_("In KOReader: open a book → top menu → More tools → Bookx Sync");
/// Where KOReader’s own progress sync is.
pub const KOREADER_PROGRESS_PATH: &str =
    N_("In KOReader: Tools → Progress sync → Custom sync server");

/// Said once, above the requirements table.
pub const STOCK_NOTE: &str = N_(
    "Stock Kobo and Kindle Wi-Fi sync is not supported: those readers sync over the USB cable only. Wi-Fi sync needs KOReader on the e-reader. “bookx” is the only address path: http://<host-ip>:<port>/bookx",
);

/// The message of a failed Register or Log in with a hint for the usual cause.
pub fn account_hint(error: &str, register: bool) -> String {
    let taken = error.contains("already registered");
    if register && taken {
        format!(
            "{error}. {}",
            tr("Choose another user name, or press Log in if this account is yours.")
        )
    } else {
        error.to_string()
    }
}

// ---- state ------------------------------------------------------------------------------------

/// What is switched on, read from the settings and the running server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// A progress-sync account (the Bookx sync client) is switched on.
    pub kosync_client: bool,
    /// A user name and password are saved for it.
    pub account: bool,
    /// The saved user name (shown as the status of the account step).
    pub account_name: String,
    /// The library server is running.
    pub server_running: bool,
    /// The server accepts connections from other devices on the network (not only this computer).
    pub lan: bool,
    /// The server also answers Bookx sync requests.
    pub server_kosync: bool,
    /// Highlights and notes from the KOReader plugin are merged.
    pub annotations: bool,
    /// The plugin was copied to a device or folder from here.
    pub plugin_copied: bool,
    /// A KOReader device has sent something to this computer.
    pub device_seen: bool,
    /// Name of the connected USB e-reader, if any.
    pub usb_device: Option<String>,
    /// Sync automatically when a device connects.
    pub usb_auto: bool,
    /// Offline mode refuses online look-ups (it does not stop your own servers).
    pub offline: bool,
}

/// Bookx sync as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    /// The sync server is not running.
    Off,
    /// It runs and waits for a device.
    Ready,
    /// A device has synced with it.
    Connected,
}

impl Readiness {
    pub fn key(self) -> &'static str {
        match self {
            Readiness::Off => "off",
            Readiness::Ready => "ready",
            Readiness::Connected => "connected",
        }
    }

    /// “Bookx sync is off / ready / connected”.
    pub fn headline(self) -> String {
        match self {
            Readiness::Off => tr("Bookx sync is off"),
            Readiness::Ready => tr("Bookx sync is ready"),
            Readiness::Connected => tr("Bookx sync is connected"),
        }
    }
}

/// The four steps of the Wi-Fi setup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checklist {
    pub server: bool,
    pub account: bool,
    pub plugin: bool,
    pub tested: bool,
}

impl Checklist {
    /// The steps with their state, in order.
    pub fn steps(&self) -> [(u8, &'static str, bool); 4] {
        [
            (1, STEP_SERVER, self.server),
            (2, STEP_ACCOUNT, self.account),
            (3, STEP_PLUGIN, self.plugin),
            (4, STEP_TEST, self.tested),
        ]
    }

    /// The first step that is not done yet.
    pub fn next(&self) -> Option<u8> {
        self.steps().iter().find(|s| !s.2).map(|s| s.0)
    }

    /// Server, account and plugin are in place (the test on the device is the last step).
    pub fn ready(&self) -> bool {
        self.server && self.account && self.plugin
    }

    /// `1010`-style digits for the tests.
    pub fn digits(&self) -> String {
        self.steps()
            .iter()
            .map(|s| if s.2 { '1' } else { '0' })
            .collect()
    }
}

/// One of the three choices of the Sync window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub id: &'static str,
    pub title: String,
    pub use_when: String,
    pub status: String,
    pub button: String,
}

impl Snapshot {
    /// Wi-Fi sync is on when Bookx syncs with a server, or is one for KOReader.
    pub fn wifi_on(&self) -> bool {
        self.kosync_client || (self.server_running && self.server_kosync)
    }

    /// The server runs and other devices on the network can reach it (the banner of the window).
    pub fn lan_open(&self) -> bool {
        self.server_running && self.lan
    }

    pub fn usb_connected(&self) -> bool {
        self.usb_device.is_some()
    }

    pub fn opds_on(&self) -> bool {
        self.server_running
    }

    pub fn readiness(&self) -> Readiness {
        if !(self.server_running && self.server_kosync) {
            Readiness::Off
        } else if self.device_seen {
            Readiness::Connected
        } else {
            Readiness::Ready
        }
    }

    pub fn checklist(&self) -> Checklist {
        Checklist {
            server: self.server_running && self.server_kosync,
            account: self.account,
            plugin: self.plugin_copied,
            tested: self.device_seen,
        }
    }

    /// The state and the short status text of each of the four steps (what the stepper shows).
    pub fn step_statuses(&self) -> [(bool, String); 4] {
        let c = self.checklist();
        [
            (c.server, if c.server { tr("On") } else { tr("Off") }),
            (
                c.account,
                if c.account {
                    self.account_name.clone()
                } else {
                    tr("Not created yet")
                },
            ),
            (
                c.plugin,
                if c.plugin {
                    tr("Copied")
                } else {
                    tr("Not checked")
                },
            ),
            (
                c.tested,
                if c.tested {
                    tr("A device has synced")
                } else {
                    tr("Not checked")
                },
            ),
        ]
    }

    /// The label of the one big button of the status card.
    pub fn primary(&self) -> String {
        if self.readiness() != Readiness::Off && self.checklist().ready() {
            tr("Sync now")
        } else {
            match self.first_missing_step() {
                Some(2) => tr("Create account"),
                Some(3) => tr("Install the plugin"),
                _ => tr("Set up Bookx sync"),
            }
        }
    }

    /// The first of the four steps that is not done yet (1-4), the one the big button names.
    pub fn first_missing_step(&self) -> Option<u8> {
        self.checklist().next()
    }

    /// True when the big button syncs (otherwise it opens the setup).
    pub fn primary_syncs(&self) -> bool {
        self.readiness() != Readiness::Off && self.checklist().ready()
    }

    /// The three choices: Wi-Fi with KOReader, a USB device, sharing books.
    pub fn cards(&self) -> [Card; 3] {
        [
            Card {
                id: "wifi",
                title: tr("Bookx sync with KOReader (Wi-Fi)"),
                use_when: tr(
                    "Use this when your Kobo, Kindle, Android or other e-reader runs KOReader.",
                ),
                status: self.readiness().headline(),
                button: if self.checklist().ready() {
                    tr("Open setup guide")
                } else {
                    tr("Start setup")
                },
            },
            Card {
                id: "usb",
                title: tr("USB device"),
                use_when: tr(
                    "Use this when your e-reader is connected by a cable. It also works for devices without KOReader.",
                ),
                status: match &self.usb_device {
                    Some(n) => format!("{} ({n})", tr("connected")),
                    None => tr("not connected"),
                },
                button: tr("Open device window"),
            },
            Card {
                id: "opds",
                title: tr("OPDS server"),
                use_when: tr(
                    "Use this to let another device or app download books from this computer.",
                ),
                status: if self.opds_on() { tr("on") } else { tr("off") },
                button: if self.opds_on() {
                    tr("Stop OPDS server")
                } else {
                    tr("Start OPDS server")
                },
            },
        ]
    }

    /// “Wi-Fi sync: on”, “USB device: connected”, “OPDS: on” (and offline mode when it is on).
    pub fn lines(&self) -> Vec<Line> {
        let on_off = |on: bool| if on { tr("on") } else { tr("off") };
        let mut v = vec![
            Line {
                key: "wifi",
                on: self.wifi_on(),
                text: format!("{}: {}", tr("Wi-Fi sync"), on_off(self.wifi_on())),
            },
            Line {
                key: "usb",
                on: self.usb_connected(),
                text: format!(
                    "{}: {}",
                    tr("USB device"),
                    match &self.usb_device {
                        Some(n) => format!("{} ({n})", tr("connected")),
                        None => tr("not connected"),
                    }
                ),
            },
            Line {
                key: "opds",
                on: self.opds_on(),
                text: format!("{}: {}", tr("OPDS"), on_off(self.opds_on())),
            },
        ];
        if self.offline {
            v.push(Line {
                key: "offline",
                on: true,
                text: format!("{}: {}", tr("Offline mode"), on_off(true)),
            });
        }
        v
    }

    /// The same, in the fixed English words the UI tests grep for.
    pub fn dump(&self) -> String {
        format!(
            "wifi={} usb={} opds={} kosync_server={} annotations={} offline={} state={} steps={} next={} primary={}",
            if self.wifi_on() { "on" } else { "off" },
            if self.usb_connected() {
                "connected"
            } else {
                "not-connected"
            },
            if self.opds_on() { "on" } else { "off" },
            self.server_kosync,
            self.annotations,
            self.offline,
            self.readiness().key(),
            self.checklist().digits(),
            self.checklist()
                .next()
                .map_or("-".to_string(), |n| n.to_string()),
            if self.primary_syncs() {
                "sync-now"
            } else {
                "setup"
            },
        )
    }
}

/// One line of the status block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub key: &'static str,
    pub on: bool,
    pub text: String,
}

/// Reads the current state: the settings, the running server and a connected device.
pub fn snapshot(
    cfg: &crate::config::Config,
    server_running: bool,
    usb: Option<String>,
    device_seen: bool,
) -> Snapshot {
    Snapshot {
        kosync_client: cfg.kosync.enabled,
        account: !cfg.kosync.user.trim().is_empty() && !cfg.kosync.password.is_empty(),
        account_name: cfg.kosync.user.trim().to_string(),
        server_running,
        lan: cfg.share.lan,
        server_kosync: cfg.share.kosync,
        annotations: cfg.share.annotations && cfg.share.kosync,
        plugin_copied: cfg.share.plugin_copied,
        device_seen,
        usb_device: usb,
        usb_auto: cfg.device_sync.auto_sync,
        offline: cfg.offline,
    }
}

// ---- requirements table (Advanced) ------------------------------------------------------------

/// What a sync type needs on each side. One row of the Requirements table.
pub struct Requirement {
    pub id: &'static str,
    pub sync_type: &'static str,
    pub works_between: &'static str,
    pub bookx_needs: &'static str,
    pub reader_needs: &'static str,
    pub connection: &'static str,
    pub syncs: &'static str,
}

/// The column names of the table, in order (the first is the row title).
pub const COLUMNS: [&str; 6] = [
    N_("Sync type"),
    N_("Works between"),
    N_("What Bookx needs"),
    N_("What the e-reader needs"),
    N_("Connection"),
    N_("What syncs"),
];

pub const REQUIREMENTS: [Requirement; 5] = [
    Requirement {
        id: "progress",
        sync_type: N_("Reading progress (KOReader)"),
        works_between: N_("Bookx and KOReader on a Kobo, Kindle, Android or PocketBook reader"),
        bookx_needs: N_(
            "The Bookx sync server on, and a sync account you make in Bookx under Preferences → Sync (Register); no existing KOReader account is needed",
        ),
        reader_needs: N_(
            "KOReader with Progress sync set to a custom sync server at the same address http://<host-ip>:<port>/bookx and that user name and password (or create the account from KOReader’s own Progress sync menu)",
        ),
        connection: N_("Wi-Fi or local network"),
        syncs: N_("Reading position only (highlights, notes and bookmarks need the Bookx plugin)"),
    },
    Requirement {
        id: "notes",
        sync_type: N_("Highlights, notes and bookmarks (KOReader)"),
        works_between: N_("Bookx and KOReader"),
        bookx_needs: N_("The Bookx sync server on and “Sync highlights, notes and bookmarks” on"),
        reader_needs: N_(
            "KOReader with the Bookx Sync plugin installed and set up. Not available on stock Kobo or Kindle software",
        ),
        connection: N_("Wi-Fi or local network, the same one"),
        syncs: N_("Highlights with their text, notes, colours and styles, and bookmarks"),
    },
    Requirement {
        id: "usb",
        sync_type: N_("USB device sync"),
        works_between: N_("Bookx and a Kobo or other e-reader mounted as a USB drive"),
        bookx_needs: N_(
            "Nothing to switch on; the USB switches on this page choose what is synced",
        ),
        reader_needs: N_(
            "Connected with the cable and mounted. Bookx reads the Kobo’s own reading state and KOReader’s sidecar files",
        ),
        connection: N_("USB cable, no Wi-Fi"),
        syncs: N_(
            "Books, reading progress, highlights and notes. Stock Kobo notes and progress are imported; stock firmware has no wireless sync with Bookx",
        ),
    },
    Requirement {
        id: "opds",
        sync_type: N_("OPDS server (catalogue and web page)"),
        works_between: N_("Bookx and reader apps or e-readers that browse OPDS"),
        bookx_needs: N_("The OPDS server on"),
        reader_needs: N_(
            "An app or browser on the same network, or one that can reach this computer (KOReader, Moon+ Reader, Librera, Kybook)",
        ),
        connection: N_("Wi-Fi or local network"),
        syncs: N_("Books and the catalogue only"),
    },
    Requirement {
        id: "mail",
        sync_type: N_("E-mail and exports"),
        works_between: N_("Bookx and any address, file or app"),
        bookx_needs: N_(
            "A mail account under Preferences → E-mail to send books; nothing for exports",
        ),
        reader_needs: N_("Nothing: the other side only receives a file"),
        connection: N_("Internet for e-mail; none for exports"),
        syncs: N_("Book files, annotations and citations only"),
    },
];

#[cfg(test)]
mod tests {
    #[test]
    fn the_account_row_says_who_is_logged_in_and_why_it_cannot_tell() {
        let t = |u, pw, li, k| super::account_status(u, pw, li, k).0;
        assert_eq!(t("", false, false, true), "Log in");
        assert_eq!(t("alice", true, true, true), "Logged in as alice");
        assert_eq!(t("alice", true, false, true), "Account saved: alice");
        assert_eq!(t("alice", false, true, true), "Password needed for alice");
        let (_, why) = super::account_status("alice", false, true, false);
        assert!(why.contains("keyring"), "a locked keyring is named: {why}");
    }

    use super::*;

    fn running() -> Snapshot {
        Snapshot {
            server_running: true,
            server_kosync: true,
            ..Default::default()
        }
    }

    #[test]
    fn groups_have_unique_ids_in_order_and_advanced_is_last() {
        let mut ids: Vec<_> = GROUPS.iter().map(|g| g.id).collect();
        assert!(GROUPS.windows(2).all(|w| w[0].order < w[1].order));
        assert_eq!(GROUPS.last().unwrap().id, "advanced");
        assert_eq!(GROUPS[0].id, "checklist");
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), GROUPS.len());
    }

    #[test]
    fn the_old_page_name_is_an_alias() {
        assert!(is_legacy_name("Devices and Sharing"));
        assert!(!is_legacy_name("devices and"));
        assert!(is_legacy_name("  devices and sharing "));
        assert!(!is_legacy_name(""));
    }

    #[test]
    fn everything_is_off_by_default() {
        let s = Snapshot::default();
        assert_eq!(s.readiness(), Readiness::Off);
        assert_eq!(s.readiness().headline(), "Bookx sync is off");
        assert_eq!(s.primary(), "Set up Bookx sync");
        assert!(!s.primary_syncs());
        assert_eq!(s.checklist().digits(), "0000");
        assert_eq!(s.checklist().next(), Some(1));
        let t: Vec<_> = s.lines().into_iter().map(|l| l.text).collect();
        assert_eq!(
            t,
            ["Wi-Fi sync: off", "USB device: not connected", "OPDS: off"]
        );
        assert!(
            s.dump()
                .contains("state=off steps=0000 next=1 primary=setup")
        );
    }

    #[test]
    fn the_big_button_names_the_first_missing_step() {
        assert_eq!(Snapshot::default().primary(), "Set up Bookx sync");
        assert_eq!(Snapshot::default().first_missing_step(), Some(1));
        let no_plugin = Snapshot {
            account: true,
            ..running()
        };
        assert_eq!(no_plugin.first_missing_step(), Some(3));
        assert_eq!(no_plugin.primary(), "Install the plugin");
        let no_account = Snapshot {
            plugin_copied: true,
            ..running()
        };
        assert_eq!(no_account.primary(), "Create account");
    }

    #[test]
    fn the_server_step_says_what_the_switch_opens() {
        let help = STEP_SERVER_HELP.to_lowercase();
        for word in ["starts", "local network", "password", "advanced"] {
            assert!(help.contains(word), "{word}");
        }
        let hint = STEP_SERVER_HINT.to_lowercase();
        assert!(hint.contains("local network") && hint.contains("password"));
    }

    #[test]
    fn the_network_banner_needs_a_running_server_with_lan_access() {
        assert!(!Snapshot::default().lan_open());
        let lan_only = Snapshot {
            lan: true,
            ..Default::default()
        };
        assert!(!lan_only.lan_open(), "a stopped server reaches nobody");
        assert!(
            Snapshot {
                lan: true,
                ..running()
            }
            .lan_open()
        );
        assert!(!running().lan_open(), "this computer only");
    }

    #[test]
    fn a_running_server_is_ready_until_a_device_syncs() {
        let s = running();
        assert_eq!(s.readiness(), Readiness::Ready);
        assert_eq!(s.readiness().headline(), "Bookx sync is ready");
        // set-up is not finished (no account, no plugin): the big button names the missing step
        assert_eq!(s.primary(), "Create account");
        assert_eq!(s.first_missing_step(), Some(2));
        assert_eq!(s.checklist().next(), Some(2));
        let done = Snapshot {
            account: true,
            plugin_copied: true,
            ..running()
        };
        assert_eq!(done.primary(), "Sync now");
        assert!(done.primary_syncs());
        assert_eq!(done.checklist().next(), Some(4));
        let seen = Snapshot {
            device_seen: true,
            ..done
        };
        assert_eq!(seen.readiness(), Readiness::Connected);
        assert_eq!(seen.readiness().headline(), "Bookx sync is connected");
        assert_eq!(seen.checklist().next(), None);
        assert_eq!(seen.checklist().digits(), "1111");
    }

    #[test]
    fn open_server_without_bookx_sync_is_off_for_sync_but_on_for_opds() {
        let s = Snapshot {
            server_running: true,
            ..Default::default()
        };
        assert_eq!(s.readiness(), Readiness::Off);
        assert!(s.opds_on() && !s.wifi_on());
        // the switch is remembered but nothing is served
        let stopped = Snapshot {
            server_kosync: true,
            ..Default::default()
        };
        assert_eq!(stopped.readiness(), Readiness::Off);
        assert!(!stopped.opds_on());
    }

    #[test]
    fn there_are_three_choice_cards_with_one_button_each() {
        let c = Snapshot::default().cards();
        assert_eq!(
            c.iter().map(|c| c.id).collect::<Vec<_>>(),
            ["wifi", "usb", "opds"]
        );
        assert_eq!(c[0].title, "Bookx sync with KOReader (Wi-Fi)");
        assert_eq!(c[0].button, "Start setup");
        assert_eq!(c[1].button, "Open device window");
        assert_eq!(c[1].status, "not connected");
        assert_eq!(c[2].button, "Start OPDS server");
        for card in &c {
            assert!(card.use_when.starts_with("Use this "), "{}", card.id);
            assert!(!card.button.is_empty() && !card.status.is_empty());
        }
        let on = Snapshot {
            server_running: true,
            usb_device: Some("Kobo".into()),
            account: true,
            plugin_copied: true,
            server_kosync: true,
            ..Default::default()
        };
        let c = on.cards();
        assert_eq!(c[0].button, "Open setup guide");
        assert_eq!(c[1].status, "connected (Kobo)");
        assert_eq!(c[2].button, "Stop OPDS server");
        assert_eq!(c[2].status, "on");
    }

    #[test]
    fn the_checklist_has_four_numbered_steps() {
        let steps = Checklist::default().steps();
        assert_eq!(steps.map(|s| s.0), [1, 2, 3, 4]);
        // the stepper draws the number, so the titles carry none
        for (_, title, _) in steps {
            assert!(!title.starts_with(|c: char| c.is_ascii_digit()), "{title}");
        }
        assert!(STEP_ACCOUNT_HELP.contains("not the OPDS server password"));
        assert!(STEP_TEST_HELP.contains("More tools → Bookx Sync → Sync now"));
    }

    #[test]
    fn log_in_waits_for_a_server() {
        assert_eq!(account_block("", None, false), Some(ACCOUNT_NO_SERVER));
        assert_eq!(
            account_block(
                "http://10.0.0.2:8080/bookx",
                Some("http://10.0.0.2:8080/bookx"),
                false
            ),
            Some(ACCOUNT_KOSYNC_OFF)
        );
        assert_eq!(
            account_block(
                "http://10.0.0.2:8080/bookx/",
                Some("http://10.0.0.2:8080/bookx"),
                true
            ),
            None
        );
        assert_eq!(
            account_block("https://sync.example/bookx", None, false),
            None
        );
    }

    #[test]
    fn the_steps_have_a_state_and_a_short_status() {
        let s = Snapshot::default().step_statuses();
        assert_eq!(
            s.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
            ["Off", "Not created yet", "Not checked", "Not checked"]
        );
        assert!(s.iter().all(|x| !x.0));
        let done = Snapshot {
            server_running: true,
            server_kosync: true,
            account: true,
            account_name: "alice".into(),
            plugin_copied: true,
            device_seen: true,
            ..Default::default()
        }
        .step_statuses();
        assert_eq!(
            done.iter().map(|x| x.1.as_str()).collect::<Vec<_>>(),
            ["On", "alice", "Copied", "A device has synced"]
        );
        assert!(done.iter().all(|x| x.0));
    }

    #[test]
    fn a_connected_device_is_named_and_offline_mode_is_shown() {
        let s = Snapshot {
            usb_device: Some("Kobo".into()),
            offline: true,
            ..Default::default()
        };
        let l = s.lines();
        assert_eq!(l[1].text, "USB device: connected (Kobo)");
        assert!(l[1].on);
        assert_eq!(l.last().unwrap().text, "Offline mode: on");
    }

    #[test]
    fn a_taken_name_gets_a_hint() {
        let e = "that user name is already registered (use Log in)";
        assert!(account_hint(e, true).contains("Choose another user name"));
        assert_eq!(account_hint(e, false), e);
        assert_eq!(account_hint("timed out", true), "timed out");
    }

    #[test]
    fn the_snapshot_reads_the_settings() {
        let mut c = crate::config::Config::default();
        c.share.annotations = true;
        assert!(!snapshot(&c, true, None, false).annotations);
        c.share.kosync = true;
        assert!(snapshot(&c, true, None, false).annotations);
        assert!(!snapshot(&c, true, None, false).account);
        c.kosync.user = "alice".into();
        c.kosync.password = "pw".into();
        c.share.plugin_copied = true;
        let s = snapshot(&c, true, None, true);
        assert!(s.account && s.plugin_copied);
        assert_eq!(s.readiness(), Readiness::Connected);
    }

    #[test]
    fn the_shared_library_is_the_opds_server_everywhere() {
        let snap = Snapshot {
            server_running: true,
            ..Snapshot::default()
        };
        let mut texts: Vec<String> = vec![
            STEP_ACCOUNT_HELP.into(),
            title("servers"),
            REQUIREMENTS
                .iter()
                .map(|r| format!("{} {}", r.sync_type, r.bookx_needs))
                .collect(),
        ];
        for c in snap.cards() {
            texts.push(format!("{} {}", c.title, c.button));
        }
        let all = texts.join(" | ");
        assert!(all.contains("OPDS server"), "{all}");
        for old in [
            "Library server",
            "library server",
            "Share Library",
            "Share books",
        ] {
            assert!(!all.contains(old), "`{old}` in {all}");
        }
    }

    #[test]
    fn the_words_name_the_product_and_never_the_old_path() {
        let all = [
            BRAND_EXPLAINED,
            WINDOW_SENTENCE,
            STEP_SERVER,
            STEP_SERVER_HELP,
            STEP_ACCOUNT,
            STEP_ACCOUNT_HELP,
            STEP_PLUGIN,
            STEP_PLUGIN_HELP,
            STEP_TEST,
            STEP_TEST_HELP,
            WHAT_SYNCS,
            ACCOUNT_ONE_LINER,
            ACCOUNT_NO_SERVER,
            ACCOUNT_KOSYNC_OFF,
            ACCOUNT_BLOCKED_TIP,
            STOCK_NOTE,
        ];
        for t in all {
            assert!(!t.contains("/kosync"), "{t}");
        }
        // KOSync appears once: the first mention of the name, with its explanation
        assert!(BRAND_EXPLAINED.starts_with("Bookx sync (extended KOSync) is compatible"));
        for t in [
            WINDOW_SENTENCE,
            STEP_SERVER,
            STEP_ACCOUNT,
            STEP_PLUGIN,
            STEP_TEST,
            WHAT_SYNCS,
        ] {
            assert!(!t.contains("KOSync"), "{t}");
        }
        assert!(WINDOW_SENTENCE.contains("Use USB for devices without KOReader"));
        for t in [WINDOW_BOOKS_NOTE, WELCOME_DEVICE_NOTE] {
            assert!(!t.contains('\n') && !t.contains("KOSync"), "{t}");
        }
    }

    #[test]
    fn the_sync_says_plainly_that_it_does_not_copy_books() {
        // issue #39: "Sync doesn't seem to sync the books themselves"
        for t in [WINDOW_BOOKS_NOTE, WELCOME_DEVICE_NOTE] {
            assert!(t.contains("never copies or deletes books"), "{t}");
            assert!(
                t.contains("Add Books") && t.contains("Send to Device"),
                "{t}"
            );
        }
        assert!(WINDOW_BOOKS_NOTE.contains("already in your library"));
        assert!(STOCK_NOTE.contains("not supported") && STOCK_NOTE.contains("/bookx"));
        assert!(KOREADER_PATH.contains("More tools → Bookx Sync"));
    }

    #[test]
    fn the_requirements_table_is_complete() {
        assert_eq!(REQUIREMENTS.len(), 5);
        for r in &REQUIREMENTS {
            for t in [
                r.sync_type,
                r.works_between,
                r.bookx_needs,
                r.reader_needs,
                r.connection,
                r.syncs,
            ] {
                assert!(!t.trim().is_empty(), "{}", r.id);
                assert!(
                    !t.contains("/kosync") && !t.contains("KOSync"),
                    "{}: {t}",
                    r.id
                );
            }
        }
        let by = |id: &str| REQUIREMENTS.iter().find(|r| r.id == id).unwrap();
        assert!(by("progress").reader_needs.contains(":<port>/bookx"));
        assert!(by("progress").bookx_needs.contains("Register"));
        assert!(by("usb").connection.contains("no Wi-Fi"));
        assert!(by("notes").reader_needs.contains("plugin"));
        assert_eq!(COLUMNS.len(), 6);
    }
}
