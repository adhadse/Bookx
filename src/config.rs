//! Persistent application settings (JSON in the XDG config dir).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::core::library::SortKey;
use crate::ui::canvas::{CanvasStyle, LayoutMode, Palette};

pub const APP_ID: &str = "com.adhadse.Bookx";
pub const APP_NAME: &str = "Bookx";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `stable`, `alpha`, `beta`, `rc` or `prerelease`, from the version's pre-release part (build.rs).
pub const CHANNEL: &str = env!("BOOKX_CHANNEL");
/// The short git commit of the build; empty when the build had no git (see build.rs).
pub const COMMIT: &str = env!("BOOKX_GIT_COMMIT");
/// `debug` or `release`.
pub const PROFILE: &str = env!("BOOKX_PROFILE");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReaderTheme {
    #[default]
    Auto,
    Light,
    Sepia,
    Dark,
    Black,
}

impl ReaderTheme {
    pub const ALL: [ReaderTheme; 5] = [
        Self::Auto,
        Self::Light,
        Self::Sepia,
        Self::Dark,
        Self::Black,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Auto => crate::i18n::N_("Follow system"),
            Self::Light => crate::i18n::N_("Light"),
            Self::Sepia => crate::i18n::N_("Sepia"),
            Self::Dark => crate::i18n::N_("Dark"),
            Self::Black => crate::i18n::N_("Black"),
        }
    }

    pub fn resolve(&self, system_dark: bool) -> ReaderTheme {
        match self {
            Self::Auto if system_dark => Self::Dark,
            Self::Auto => Self::Light,
            t => *t,
        }
    }

    pub fn palette(&self, system_dark: bool) -> Palette {
        match self.resolve(system_dark) {
            Self::Sepia => Palette {
                bg: (0.957, 0.925, 0.847),
                fg: (0.263, 0.204, 0.133),
                link: (0.55, 0.28, 0.1),
                muted: (0.5, 0.42, 0.32),
                accent: (0.72, 0.45, 0.15),
            },
            Self::Dark => Palette {
                bg: (0.12, 0.12, 0.13),
                fg: (0.83, 0.83, 0.82),
                link: (0.53, 0.75, 1.0),
                muted: (0.55, 0.55, 0.57),
                accent: (0.47, 0.67, 0.98),
            },
            Self::Black => Palette {
                bg: (0.0, 0.0, 0.0),
                fg: (0.8, 0.8, 0.8),
                link: (0.53, 0.75, 1.0),
                muted: (0.5, 0.5, 0.5),
                accent: (0.47, 0.67, 0.98),
            },
            _ => Palette {
                bg: (1.0, 1.0, 1.0),
                fg: (0.11, 0.11, 0.12),
                link: (0.1, 0.4, 0.8),
                muted: (0.45, 0.45, 0.47),
                accent: (0.21, 0.52, 0.89),
            },
        }
    }

    pub fn css_class(&self, system_dark: bool) -> &'static str {
        match self.resolve(system_dark) {
            Self::Sepia => "reader-sepia",
            Self::Dark => "reader-dark",
            Self::Black => "reader-black",
            _ => "reader-light",
        }
    }
}

/// Where the page and text colours come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorMode {
    /// The chosen theme; the book's own text colours are ignored.
    #[default]
    Theme,
    /// What the book's CSS says (text colours and page colour), on a light page like a browser.
    Original,
    /// Text and page colours picked by the user.
    Custom,
}

/// `#rrggbb` (or `rrggbb`) as 0..1 channels.
pub fn parse_hex(s: &str) -> Option<(f64, f64, f64)> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 || !t.is_ascii() {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&t[i..i + 2], 16)
            .ok()
            .map(|v| v as f64 / 255.0)
    };
    Some((c(0)?, c(2)?, c(4)?))
}

/// `#rrggbb` for 0..1 channels.
pub fn to_hex(c: (f64, f64, f64)) -> String {
    let b = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.0), b(c.1), b(c.2))
}

fn luminance(c: (f64, f64, f64)) -> f64 {
    0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
}

fn mix(a: (f64, f64, f64), b: (f64, f64, f64), t: f64) -> (f64, f64, f64) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReaderPrefs {
    pub theme: ReaderTheme,
    pub colors: ColorMode,
    /// Text colour of [`ColorMode::Custom`] as `#rrggbb`.
    pub custom_fg: String,
    /// Page colour of [`ColorMode::Custom`] as `#rrggbb`.
    pub custom_bg: String,
    pub font_family: String,
    pub font_size: f64,
    pub line_spacing: f64,
    pub margin: f64,
    pub justify: bool,
    pub hyphenate: bool,
    /// Knuth-Plass line breaking in justified text (off: Pango's own breaking).
    pub optimal_breaks: bool,
    /// Apply the book's own styling (classes for italic, bold, alignment, hidden text …).
    pub publisher_styles: bool,
    pub tap_zones: crate::gestures::TapZones,
    pub touch_ui: crate::gestures::TouchUi,
    pub layout: LayoutMode,
    pub translate_to: String,
}

impl Default for ReaderPrefs {
    fn default() -> Self {
        Self {
            theme: ReaderTheme::Auto,
            colors: ColorMode::Theme,
            custom_fg: "#2b2b2b".into(),
            custom_bg: "#fbf7ee".into(),
            font_family: "Serif".into(),
            font_size: 14.0,
            line_spacing: 1.25,
            margin: 56.0,
            justify: true,
            hyphenate: true,
            optimal_breaks: true,
            publisher_styles: true,
            tap_zones: crate::gestures::TapZones::default(),
            touch_ui: crate::gestures::TouchUi::default(),
            layout: LayoutMode::TwoColumns,
            translate_to: "en".into(),
        }
    }
}

impl ReaderPrefs {
    /// The colours of the page: the theme, the user's own pair, or the plain light page that the
    /// book's colours are drawn on.
    pub fn palette(&self, system_dark: bool) -> Palette {
        match self.colors {
            ColorMode::Theme => self.theme.palette(system_dark),
            ColorMode::Original => ReaderTheme::Light.palette(false),
            ColorMode::Custom => {
                let base = ReaderTheme::Light.palette(false);
                let fg = parse_hex(&self.custom_fg).unwrap_or(base.fg);
                let bg = parse_hex(&self.custom_bg).unwrap_or(base.bg);
                let dark = luminance(bg) < 0.5;
                Palette {
                    bg,
                    fg,
                    link: if dark {
                        (0.53, 0.75, 1.0)
                    } else {
                        (0.1, 0.4, 0.8)
                    },
                    muted: mix(fg, bg, 0.5),
                    accent: if dark {
                        (0.47, 0.67, 0.98)
                    } else {
                        (0.21, 0.52, 0.89)
                    },
                }
            }
        }
    }

    /// Style class for the reader's bars so that they match the page.
    pub fn chrome_class(&self, system_dark: bool) -> &'static str {
        match self.colors {
            ColorMode::Theme => self.theme.css_class(system_dark),
            _ => {
                if luminance(self.palette(system_dark).bg) < 0.5 {
                    "reader-dark"
                } else {
                    "reader-light"
                }
            }
        }
    }

    pub fn canvas_style(&self, system_dark: bool) -> CanvasStyle {
        CanvasStyle {
            original_colors: self.colors == ColorMode::Original,
            palette: self.palette(system_dark),
            font_family: self.font_family.clone(),
            font_size: self.font_size,
            line_spacing: self.line_spacing,
            margin: self.margin,
            justify: self.justify,
            hyphenate: self.hyphenate,
            optimal_breaks: self.optimal_breaks,
            paragraph_spacing: None,
            fixed: false,
            layout: self.layout,
            ..CanvasStyle::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Scheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub library_dir: PathBuf,
    pub scheme: Scheme,
    pub view_mode: ViewMode,
    pub sort_key: SortKey,
    pub sort_ascending: bool,
    pub cover_size: i32,
    /// The columns of the table view the user picked (titles in English); empty = the ones that fit
    /// the width (`library_nav::table_columns_for_width`).
    pub table_columns: Vec<String>,
    /// The custom columns (labels) the table shows.
    pub table_custom: Vec<String>,
    /// Fill in the author, title and series of a new book from its file name when the book has none.
    pub guess_filename: bool,
    /// Tags written like `Fiction.Fantasy` show as a tree in the sidebar.
    pub tag_tree: bool,
    /// The file name patterns (see `library_add::parse_patterns`); empty = the built-in ones.
    pub filename_patterns: String,
    /// The table is sorted by a custom column (see `library_custom::CustomSort`).
    pub custom_sort: Option<crate::library_custom::CustomSort>,
    /// The details panel is wanted (its header toggle is on): a selected book then shows it.
    pub details_pane: bool,
    /// Sidebar groups the user opened, as `library folder|group` (a closed group is not listed).
    pub sidebar_open: Vec<String>,
    /// Books that are editions of the same work (same title and author, same ISBN, or linked by
    /// the user) show as one card with an edition list. Off by default: it hides books in the grid.
    pub group_editions: bool,
    /// The sidebar entry shown last (`library_nav::Section::key`), restored at the next start.
    pub sidebar_view: String,
    /// Books the user starred (the Favourites entry of the sidebar), by catalogue id.
    pub favourites: Vec<u64>,
    /// The Continue Reading banner: `auto` (compact in a small window), `large`, `compact`, `hidden`.
    pub banner_mode: String,
    pub copy_on_add: bool,
    pub reader: ReaderPrefs,
    /// Page-turn animation, auto-turn, night schedule, citation, page turner keys, archive (the
    /// same for every book, unlike the look in `reader`).
    pub aids: crate::aids_prefs::ReaderAids,
    /// Style tweaks of the reader: for all books and per book (R21).
    pub tweaks: crate::tweaks::TweakStore,
    /// Read aloud: speed and voice.
    pub tts: crate::tts::TtsPrefs,
    pub share: crate::sharing::ShareSettings,
    pub email: crate::emailing::EmailSettings,
    /// Automatic backups and one-time notes about the library.
    pub health: crate::library_health::HealthPrefs,
    /// Bulk metadata fetch and the check before converting or sending.
    pub metadata_prefs: crate::bulk_fetch::MetadataPrefs,
    /// Device window: format, save-path template, Kobo shelf, WebDAV folder.
    pub device: crate::devices::DeviceSettings,
    /// Syncing reading progress and notes with a device, sending notes along.
    pub device_sync: crate::device_sync::SyncPrefs,
    /// Reading position sync with a KOReader progress-sync server (the password is kept here).
    pub kosync: crate::kosync_sync::KoSyncPrefs,
    /// Readwise access token (kept in the keyring, see `secret_fields`).
    pub readwise_token: String,
    /// Wallabag and other web accounts (the secrets are kept in the keyring, see `secret_fields`).
    pub web_accounts: crate::web_accounts::WebAccounts,
    /// User names for paywalled news sites (the passwords are in the keyring, see `secret_fields`).
    pub news_sites: Vec<crate::news_sites::SiteAccount>,
    /// Own web look-up sites and the history switch (Preferences → Dictionaries).
    pub lookup: crate::lookup_prefs::LookupPrefs,
    /// Which metadata source wins for which field.
    pub source_priority: crate::source_priority::SourcePriority,
    /// Notes & Highlights: which kind of entry is listed (`all`, `highlights`, `notes`, `bookmarks`).
    pub notes_kind: String,
    /// The Device window lists books instead of showing covers.
    pub device_list_view: bool,
    /// E-ink mode: pure black on white, no animations, bars that stay.
    pub eink: bool,
    pub window_width: i32,
    pub window_height: i32,
    pub maximized: bool,
    /// Set once the clean (v2) layout defaults were applied to an older config.
    #[serde(default = "missing_flag")]
    pub clean_ui_applied: bool,
    /// Named searches; `search:name` in a query stands for one.
    pub saved_searches: Vec<crate::saved_searches::SavedSearch>,
    /// The saved search that narrows the whole library, by name.
    pub virtual_library: Option<String>,
    /// OPDS catalogues the user added (Project Gutenberg and Standard Ebooks are built in).
    /// Interface language code (`es`), empty = follow the system.
    pub language: String,
    /// Reading goals shown in the statistics window.
    pub goals: crate::goals::GoalPrefs,
    /// Following a link to a footnote shows the note in a popup.
    pub footnote_popups: bool,
    /// Terms saved with your own definition are marked in the text (dotted underline).
    pub mark_vocabulary: bool,
    /// Speed reading: words per minute.
    pub rsvp_wpm: u32,
    /// The read-aloud and speed-reading cards of each book: open or closed, where, and the place.
    pub cards: crate::reader_cards::CardsStore,
    pub opds_catalogs: Vec<crate::opds_browse::Catalog>,
    /// The old named reading profiles: read once to migrate them to `book_looks`, never written.
    #[serde(skip_serializing)]
    pub profiles: crate::reading_profiles::Profiles,
    /// The look of each book (every change made in the reader is kept here); `reader` holds the
    /// defaults of books without one.
    pub book_looks: crate::reading_profiles::BookLooks,
    /// While a book is open `reader` is that book's look and this holds the defaults; the settings
    /// file always gets the defaults.
    #[serde(skip)]
    pub reader_defaults: Option<ReaderPrefs>,
    /// The reader's own colour themes, shared by every book.
    pub reader_themes: crate::reader_themes::Themes,
    /// The statistics in the reader's footer, for every book.
    pub footer: crate::footer_stats::FooterConfig,
    /// Books with a footer of their own (by catalogue id).
    pub footer_books: std::collections::BTreeMap<u64, crate::footer_stats::FooterConfig>,
    /// The footer position that older versions stored as a default was reset to "middle" (once).
    #[serde(default = "missing_flag")]
    pub footer_pos_migrated: bool,
    /// Automatic dictionary downloads and what was already looked at.
    pub dictionaries: crate::dict_auto::Prefs,
    /// Offline mode: nothing is downloaded or looked up online (`crate::privacy`).
    pub offline: bool,
    /// The version whose beta notice the user has seen or dismissed (`crate::beta`); empty = none.
    pub beta_notice_seen: String,
    /// The reading ruler of the reader (`crate::reading_ruler`).
    pub ruler: crate::reading_ruler::RulerPrefs,
    /// Which sidebar entries the library shows and in what order, per library folder
    /// (`library_nav::SidebarLayout`); a library with no entry has the default sidebar.
    pub sidebar_layouts: std::collections::BTreeMap<String, crate::library_nav::SidebarLayout>,
}

impl Config {
    /// The sidebar layout of the library in this folder (the default one when none was saved).
    pub fn sidebar_layout(&self, library_key: &str) -> crate::library_nav::SidebarLayout {
        self.sidebar_layouts
            .get(library_key)
            .cloned()
            .unwrap_or_default()
    }

    /// Remembers the sidebar layout of a library; the default layout is not stored.
    pub fn set_sidebar_layout(
        &mut self,
        library_key: &str,
        layout: crate::library_nav::SidebarLayout,
    ) {
        if layout.is_default() {
            self.sidebar_layouts.remove(library_key);
        } else {
            self.sidebar_layouts.insert(library_key.to_string(), layout);
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            library_dir: default_library_dir(),
            scheme: Scheme::System,
            view_mode: ViewMode::Grid,
            sort_key: SortKey::Title,
            sort_ascending: true,
            cover_size: 150,
            table_columns: Vec::new(),
            table_custom: Vec::new(),
            guess_filename: true,
            tag_tree: false,
            filename_patterns: String::new(),
            custom_sort: None,
            details_pane: true,
            sidebar_open: Vec::new(),
            group_editions: false,
            sidebar_view: String::new(),
            favourites: Vec::new(),
            banner_mode: String::new(),
            copy_on_add: true,
            reader: ReaderPrefs::default(),
            aids: Default::default(),
            tweaks: crate::tweaks::TweakStore::default(),
            tts: crate::tts::TtsPrefs::default(),
            share: crate::sharing::ShareSettings::default(),
            email: crate::emailing::EmailSettings::default(),
            health: crate::library_health::HealthPrefs::default(),
            metadata_prefs: crate::bulk_fetch::MetadataPrefs::default(),
            device: crate::devices::DeviceSettings::default(),
            device_sync: crate::device_sync::SyncPrefs::default(),
            kosync: crate::kosync_sync::KoSyncPrefs::default(),
            readwise_token: String::new(),
            web_accounts: Default::default(),
            news_sites: Vec::new(),
            lookup: Default::default(),
            source_priority: Default::default(),
            notes_kind: "all".into(),
            device_list_view: false,
            eink: false,
            window_width: 1280,
            window_height: 800,
            maximized: false,
            clean_ui_applied: true,
            saved_searches: Vec::new(),
            virtual_library: None,
            language: String::new(),
            goals: crate::goals::GoalPrefs::default(),
            footnote_popups: true,
            mark_vocabulary: true,
            rsvp_wpm: crate::rsvp_view::DEFAULT_WPM,
            cards: Default::default(),
            opds_catalogs: Vec::new(),
            profiles: Default::default(),
            book_looks: Default::default(),
            reader_defaults: None,
            reader_themes: Default::default(),
            footer: Default::default(),
            footer_books: Default::default(),
            footer_pos_migrated: true,
            dictionaries: Default::default(),
            offline: false,
            beta_notice_seen: String::new(),
            ruler: Default::default(),
            sidebar_layouts: Default::default(),
        }
    }
}

/// A config written before the flag existed must read as "not applied yet".
fn missing_flag() -> bool {
    false
}

pub fn default_library_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(|p| p.join("Bookx Library")))
        .or_else(|| directories::BaseDirs::new().map(|d| d.data_dir().join("bookx/Library")))
        .unwrap_or_else(|| PathBuf::from("Bookx Library"))
}

/// Where the vocabulary is kept: next to the configuration file.
pub fn vocab_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("vocabulary.json")))
}

/// Where reading statistics are kept: next to the configuration file.
pub fn stats_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("stats.json")))
}

/// How long each part of a book was on screen (the bars of the book map): next to the settings.
pub fn dwell_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("dwell.json")))
}

/// The user's keyboard shortcuts: next to the configuration file.
pub fn shortcuts_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("shortcuts.json")))
}

/// The order of shelves and of the books on them: next to the configuration file.
pub fn shelves_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("shelves.json")))
}

/// The list of libraries behind the switcher: next to the configuration file.
pub fn libraries_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("libraries.json")))
}

pub fn news_path() -> Option<PathBuf> {
    config_path().and_then(|p| p.parent().map(|d| d.join("news.json")))
}

/// The keyring key of a backup destination's WebDAV password.
pub fn backup_password_key(id: &str) -> String {
    format!("backup-{id}-password")
}

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("config.json"))
}

/// Where the settings file lives, as text (empty when there is no home).
pub fn config_file_path_text() -> String {
    config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

impl Config {
    /// The footer statistics of a book: its own setting, else the common one.
    pub fn footer_for(&self, book: u64) -> crate::footer_stats::FooterConfig {
        self.footer_books
            .get(&book)
            .unwrap_or(&self.footer)
            .clone()
            .normalized()
    }

    /// Once: a footer position that was only the old default becomes "default" (the middle).
    pub fn migrate_footer_position(&mut self) {
        if self.footer_pos_migrated {
            return;
        }
        self.footer.migrate_old_default();
        for f in self.footer_books.values_mut() {
            f.migrate_old_default();
        }
        self.footer_pos_migrated = true;
    }

    pub fn load() -> Config {
        if crate::safe_mode::active() {
            // defaults, and `save` does nothing: the real settings come back at the next normal start
            let mut cfg = Config::default();
            if let Ok(dir) = std::env::var("BOOKX_LIBRARY") {
                cfg.library_dir = PathBuf::from(dir);
            }
            return cfg;
        }
        let text = config_path().and_then(|p| std::fs::read_to_string(p).ok());
        let (mut cfg, migrated) = Config::from_text(text.as_deref(), crate::secrets::vault());
        let dir = config_path().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        if migrated {
            // passwords found in the file: the notice is shown once, and the copies are scrubbed
            SCRUBBED.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Some(d) = &dir {
                scrub_copies(d);
            }
        }
        let migrated = cfg.migrate_profiles(dir.as_deref()) || migrated;
        if let Ok(dir) = std::env::var("BOOKX_LIBRARY") {
            cfg.library_dir = PathBuf::from(dir);
        }
        if migrated {
            // passwords left the file (keyring, encrypted store, or memory only), or profiles became
            // per-book looks: rewrite the file
            cfg.save();
        }
        cfg
    }

    /// Old named reading profiles become per-book looks; the ones no book used are written to
    /// `reading-profiles-backup.json` in `dir` (the settings folder). True when something changed.
    pub fn migrate_profiles(&mut self, dir: Option<&std::path::Path>) -> bool {
        if self.profiles.is_empty() {
            return false;
        }
        let m =
            crate::reading_profiles::migrate(&self.profiles, &self.reader, &mut self.book_looks);
        if let (Some(text), Some(dir)) = (m.backup, dir) {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join("reading-profiles-backup.json"), text);
        }
        self.profiles = Default::default();
        true
    }

    /// Parses the settings file and gets the passwords from `vault`. The flag is true when
    /// passwords were found in plain text: they were moved into the vault or, when it cannot hold
    /// them, kept in memory for this session. Either way the file must be saved (that removes them).
    pub fn from_text(text: Option<&str>, vault: &bookx_secrets::Vault) -> (Config, bool) {
        let mut cfg: Config = text
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or_default();
        let m = vault.load(&mut cfg.secret_fields());
        cfg.migrate_footer_position();
        (cfg, m.moved + m.unsaved > 0)
    }

    /// Where the passwords live: in the keyring, under these keys.
    pub fn secret_fields(&mut self) -> Vec<(String, &mut String)> {
        let mut v: Vec<(String, &mut String)> = vec![
            ("share-password".into(), &mut self.share.password),
            ("email-password".into(), &mut self.email.password),
            ("kosync-password".into(), &mut self.kosync.password),
            ("readwise-token".into(), &mut self.readwise_token),
            (
                "wallabag-client-secret".into(),
                &mut self.web_accounts.wallabag_client_secret,
            ),
            (
                "wallabag-password".into(),
                &mut self.web_accounts.wallabag_password,
            ),
            (
                "wallabag-token".into(),
                &mut self.web_accounts.wallabag_token,
            ),
            (
                "hardcover-token".into(),
                &mut self.web_accounts.hardcover_token,
            ),
            ("tts-api-key".into(), &mut self.tts.server.api_key),
            (
                "device-webdav-password".into(),
                &mut self.device.webdav.password,
            ),
            (
                "backup-passphrase".into(),
                &mut self.health.backup.passphrase,
            ),
        ];
        for d in self.health.backup.destinations.iter_mut() {
            v.push((backup_password_key(&d.id), &mut d.password));
        }
        for c in self.opds_catalogs.iter_mut() {
            v.push((crate::opds_browse::password_key(&c.id), &mut c.password));
        }
        for a in self.news_sites.iter_mut() {
            v.push((crate::news_sites::secret_key(&a.host), &mut a.password));
        }
        v
    }

    /// The text of the settings file: the passwords are in the vault (or nowhere), **never** in the
    /// text, whatever state the vault is in.
    pub fn file_text(&self, vault: &bookx_secrets::Vault) -> Option<String> {
        let mut copy = self.clone();
        if let Some(d) = &self.reader_defaults {
            copy.reader = d.clone();
        }
        vault.persist(&mut copy.secret_fields());
        // belt and braces: whatever the vault did, no secret leaves in the text
        for (_, v) in copy.secret_fields() {
            v.clear();
        }
        serde_json::to_string_pretty(&copy).ok()
    }

    pub fn save(&self) {
        if crate::safe_mode::active() {
            return;
        }
        let Some(p) = config_path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Some(json) = self.file_text(crate::secrets::vault()) {
            // written next to the file and renamed, so a crash never leaves half a file; private to
            // the user from the first byte
            let tmp = p.with_extension("json.tmp");
            let wrote = write_private(&tmp, json.as_bytes());
            if let Err(e) = &wrote {
                crate::trace::event(crate::trace::Area::Settings, "save").fail(&e.to_string());
            }
            if wrote.is_ok() && std::fs::rename(&tmp, &p).is_err() {
                crate::trace::event(crate::trace::Area::Settings, "save").fail_class(crate::trace::ErrorClass::Io);
                let _ = std::fs::remove_file(&tmp);
            }
            bookx_secrets::restrict_file(&p);
        }
    }
}

/// Set when the settings file held passwords in plain text (an older version): the app tells the
/// user once that they were removed.
static SCRUBBED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True once after a start that found passwords in the settings file.
pub fn take_scrub_notice() -> bool {
    SCRUBBED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Creates `path` (mode 0600 on Unix, replacing any old file) and writes `data`.
fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

/// Copies of the settings that an older version, an editor or the user left next to
/// `config.json` can hold the same plain text passwords. The half-written temp file is removed;
/// `.old`, `.bak`, `~` and `.orig` copies that parse as settings are rewritten without secrets.
/// The folder is made private.
pub fn scrub_copies(dir: &std::path::Path) {
    bookx_secrets::restrict_dir(dir);
    let _ = std::fs::remove_file(dir.join("config.json.tmp"));
    for name in [
        "config.json.old",
        "config.json.bak",
        "config.json~",
        "config.json.orig",
        "config.old.json",
        "config.bak.json",
    ] {
        let path = dir.join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut c) = serde_json::from_str::<Config>(&text) else {
            continue;
        };
        for (_, v) in c.secret_fields() {
            v.clear();
        }
        if let Ok(clean) = serde_json::to_string_pretty(&c) {
            let _ = write_private(&path, clean.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_sync_server_and_the_login_survive_a_save_and_a_load() {
        let vault = bookx_secrets::Vault::new(Box::new(bookx_secrets::MemoryStore::new()));
        let mut c = super::Config::default();
        c.share.running = true;
        c.share.kosync = true;
        c.kosync.user = "alice".into();
        c.kosync.password = "pw-123".into();
        c.kosync.logged_in = true;
        let text = c.file_text(&vault).unwrap();
        assert!(
            !text.contains("pw-123"),
            "the password stays in the keyring"
        );
        let (back, _) = super::Config::from_text(Some(&text), &vault);
        assert!(back.share.running && back.share.kosync && back.share.restore_at_launch());
        assert!(back.kosync.logged_in && back.kosync.password == "pw-123");
        let (old, _) = super::Config::from_text(Some("{}"), &vault);
        assert!(!old.share.running && !old.kosync.logged_in);
    }

    #[test]
    fn an_old_stored_footer_position_goes_back_to_the_default_once() {
        use crate::footer_stats::{Align, Preset};
        let mut old = Preset::Standard.config();
        old.align = Some(Align::Spread);
        let mut chosen = Preset::Everything.config();
        chosen.align = Some(Align::Spread);
        let text =
            serde_json::json!({ "footer": old, "footer_books": { "7": chosen } }).to_string();
        let vault = bookx_secrets::Vault::new(Box::new(bookx_secrets::MemoryStore::new()));
        let (cfg, _) = Config::from_text(Some(&text), &vault);
        assert_eq!(cfg.footer.align, None, "never chosen: the middle");
        assert_eq!(cfg.footer_for(1).align(), Align::Centered);
        assert_eq!(
            cfg.footer_books[&7].align,
            Some(Align::Spread),
            "a changed footer keeps its position"
        );
        assert!(cfg.footer_pos_migrated);
        // the next start does not touch it again: a position chosen later stays
        let mut again = cfg.clone();
        again.footer.align = Some(Align::Spread);
        again.migrate_footer_position();
        assert_eq!(again.footer.align, Some(Align::Spread));
        // a new install is centred
        assert_eq!(Config::default().footer.align(), Align::Centered);
    }

    use super::*;

    #[test]
    fn old_config_without_new_fields_still_loads() {
        let c: Config =
            serde_json::from_str(r#"{"cover_size": 200, "reader": {"font_size": 18.0}}"#).unwrap();
        assert_eq!(c.cover_size, 200);
        assert_eq!(c.reader.font_size, 18.0);
        assert_eq!(c.reader.translate_to, "en");
        assert!(c.reader.justify);
    }

    #[test]
    fn reader_prefs_roundtrip() {
        let p = ReaderPrefs {
            theme: ReaderTheme::Sepia,
            translate_to: "hi".into(),
            ..Default::default()
        };
        let back: ReaderPrefs = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn auto_theme_follows_the_system() {
        assert_eq!(ReaderTheme::Auto.resolve(true), ReaderTheme::Dark);
        assert_eq!(ReaderTheme::Auto.resolve(false), ReaderTheme::Light);
        assert_eq!(ReaderTheme::Sepia.resolve(true), ReaderTheme::Sepia);
        assert_eq!(ReaderTheme::Dark.css_class(false), "reader-dark");
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;
    use bookx_secrets::{MemoryStore, Unavailable, Vault};

    #[test]
    fn old_profiles_become_book_looks_and_are_never_written_again() {
        let mut c: Config = serde_json::from_str(
            r#"{"reader":{"font_size":15.0},"profiles":{"list":[{"name":"Big","font_size":24.0},{"name":"Spare","margin":10.0}],"books":{"4":"Big"}}}"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("bookx-profiles-{}", std::process::id()));
        assert!(c.migrate_profiles(Some(&dir)));
        assert_eq!(c.book_looks.get(4).unwrap().font_size, 24.0);
        assert_eq!(c.reader.font_size, 15.0);
        let backup = std::fs::read_to_string(dir.join("reading-profiles-backup.json")).unwrap();
        assert!(backup.contains("Spare") && !backup.contains("Big"));
        let _ = std::fs::remove_dir_all(&dir);
        // once migrated there is nothing to do, and the file has no profiles any more
        assert!(!c.migrate_profiles(None));
        let text = serde_json::to_string(&c).unwrap();
        assert!(!text.contains("\"profiles\""));
        assert!(text.contains("\"book_looks\""));
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.book_looks.get(4).unwrap().font_size, 24.0);
    }

    #[test]
    fn the_settings_file_keeps_the_defaults_while_a_book_has_its_own_look() {
        let mut c = Config::default();
        c.reader.font_size = 14.0;
        c.reader_defaults = Some(c.reader.clone());
        c.reader.font_size = 22.0; // the open book's look
        let text = c
            .file_text(&Vault::new(Box::new(MemoryStore::new())))
            .unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back.reader.font_size, 14.0);
        assert!(back.reader_defaults.is_none());
    }

    fn with_passwords() -> Config {
        let mut c = Config::default();
        c.share.password = "share-pw".into();
        c.email.password = "mail-pw".into();
        c.kosync.password = "kosync-pw".into();
        c.readwise_token = "rw-token".into();
        c.tts.server.api_key = "tts-key".into();
        c.device.webdav.password = "dav-pw".into();
        c.health.backup.passphrase = "backup-phrase".into();
        c.web_accounts.hardcover_token = "hc-token-sentinel".into();
        c.news_sites.push(crate::news_sites::SiteAccount {
            host: "news.example.com".into(),
            user: "reader".into(),
            password: "site-pw-sentinel".into(),
        });
        c.health
            .backup
            .destinations
            .push(bookx_backup::Destination::webdav(
                "d1",
                "Nextcloud",
                "https://c.example",
                "bob",
                "nc-pw",
            ));
        c
    }

    #[test]
    fn passwords_do_not_reach_the_settings_file() {
        let v = Vault::new(Box::new(MemoryStore::new()));
        let text = with_passwords().file_text(&v).unwrap();
        for secret in [
            "share-pw",
            "mail-pw",
            "kosync-pw",
            "rw-token",
            "tts-key",
            "dav-pw",
            "backup-phrase",
            "nc-pw",
            "hc-token-sentinel",
            "site-pw-sentinel",
        ] {
            assert!(!text.contains(secret), "{secret} is in the file");
        }
        assert!(text.contains("Nextcloud"), "the rest of the settings is");
    }

    #[test]
    fn passwords_come_back_from_the_vault_on_load() {
        let v = Vault::new(Box::new(MemoryStore::new()));
        let text = with_passwords().file_text(&v).unwrap();
        let (c, migrated) = Config::from_text(Some(&text), &v);
        assert!(!migrated);
        assert_eq!(c.share.password, "share-pw");
        assert_eq!(c.health.backup.passphrase, "backup-phrase");
        assert_eq!(c.health.backup.destinations[0].password, "nc-pw");
        assert_eq!(c.device.webdav.password, "dav-pw");
        assert_eq!(c.web_accounts.hardcover_token, "hc-token-sentinel");
        assert_eq!(c.news_sites[0].password, "site-pw-sentinel");
        assert_eq!(c.news_sites[0].user, "reader");
    }

    #[test]
    fn plain_text_passwords_of_an_older_version_are_migrated() {
        let old = serde_json::to_string(&with_passwords()).unwrap();
        assert!(old.contains("share-pw"), "the old file had them");
        let v = Vault::new(Box::new(MemoryStore::new()));
        let (c, migrated) = Config::from_text(Some(&old), &v);
        assert!(migrated, "the caller must rewrite the file");
        assert_eq!(c.email.password, "mail-pw", "still usable in memory");
        let new_text = c.file_text(&v).unwrap();
        assert!(!new_text.contains("mail-pw") && !new_text.contains("nc-pw"));
        let (again, migrated) = Config::from_text(Some(&new_text), &v);
        assert!(!migrated);
        assert_eq!(again.email.password, "mail-pw");
    }

    #[test]
    fn without_a_keyring_the_file_never_has_a_password() {
        let v = Vault::new(Box::new(Unavailable("no Secret Service".into())));
        let text = with_passwords().file_text(&v).unwrap();
        for s in SENTINELS {
            assert!(!text.contains(s), "{s} is in the file");
        }
        let (c, migrated) = Config::from_text(Some(&text), &v);
        assert!(!migrated, "nothing in the file, nothing to rewrite");
        assert!(c.share.password.is_empty(), "and nothing comes back");
    }

    const SENTINELS: [&str; 10] = [
        "hc-token-sentinel",
        "site-pw-sentinel",
        "share-pw",
        "mail-pw",
        "kosync-pw",
        "rw-token",
        "tts-key",
        "dav-pw",
        "backup-phrase",
        "nc-pw",
    ];

    /// Every secret field, set to a sentinel, over every state a vault can be in.
    fn all_secrets_set() -> Config {
        let mut c = with_passwords();
        c.opds_catalogs.push(crate::opds_browse::Catalog {
            id: "cat1".into(),
            name: "Cat".into(),
            url: "https://opds.example".into(),
            user: "u".into(),
            password: "sentinel-opds".into(),
        });
        c.health
            .backup
            .destinations
            .push(bookx_backup::Destination::webdav(
                "d2",
                "Second",
                "https://d2.example",
                "bob",
                "sentinel-d2",
            ));
        let n = c.secret_fields().len();
        for (i, (k, v)) in c.secret_fields().into_iter().enumerate() {
            if v.is_empty() {
                *v = format!("sentinel-{i}-{k}");
            }
        }
        assert!(n >= 11, "fields: {n}");
        c
    }

    fn secrets_of(c: &mut Config) -> Vec<String> {
        c.secret_fields()
            .into_iter()
            .map(|(_, v)| v.clone())
            .collect()
    }

    #[test]
    fn no_vault_state_ever_puts_a_secret_in_the_file() {
        let dir = std::env::temp_dir().join(format!("bookx-cfg-states-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = || {
            bookx_secrets::FileStore::with_kdf(dir.join("secrets.vault"), bookx_secrets::Kdf::TEST)
        };
        let mut states: Vec<(&str, Vault)> = vec![
            ("memory", Vault::new(Box::new(MemoryStore::new()))),
            ("unavailable", Vault::new(Box::new(Unavailable("x".into())))),
            (
                "fresh",
                Vault::with_file(Box::new(Unavailable("x".into())), file()),
            ),
        ];
        let unlocked = Vault::with_file(Box::new(Unavailable("x".into())), file());
        unlocked.create_vault("a long master password").unwrap();
        states.push(("unlocked", unlocked));
        let locked = Vault::with_file(Box::new(Unavailable("x".into())), file());
        assert_eq!(locked.mode(), bookx_secrets::Mode::Locked);
        states.push(("locked", locked));
        let mut c = all_secrets_set();
        let want = secrets_of(&mut c);
        for (name, v) in &states {
            let text = c.file_text(v).unwrap();
            for s in &want {
                assert!(!text.contains(s.as_str()), "{name}: {s} is in the file");
            }
        }
        // the in-memory configuration still has them
        assert_eq!(secrets_of(&mut c), want);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every way Bookx turns settings into text, with a sentinel in every secret field.
    #[test]
    fn no_serialization_path_carries_a_secret() {
        let mut c = all_secrets_set();
        let secrets = secrets_of(&mut c);
        let vaults = [
            Vault::new(Box::new(MemoryStore::new())),
            Vault::new(Box::new(Unavailable("none".into()))),
        ];
        let mut outputs: Vec<(&str, String)> = vec![
            ("settings export", crate::settings_io::export(&c)),
            (
                "mcp settings",
                serde_json::to_string(&crate::mcp_host::McpSettings {
                    token_fallback: "sentinel-mcp-token".into(),
                    ..Default::default()
                })
                .unwrap(),
            ),
        ];
        for v in &vaults {
            outputs.push(("file_text", c.file_text(v).unwrap()));
        }
        for (what, text) in &outputs {
            for s in secrets
                .iter()
                .chain(["sentinel-mcp-token".to_string()].iter())
            {
                assert!(!text.contains(s.as_str()), "{what} contains {s}");
            }
        }
        let dump = format!("{:?}", c.share);
        assert!(
            dump.contains("share-pw"),
            "(the struct itself holds it; only the writers must not)"
        );
    }

    #[test]
    fn plain_text_in_the_file_is_scrubbed_even_without_a_keyring() {
        let old = serde_json::to_string(&all_secrets_set()).unwrap();
        assert!(old.contains("sentinel-opds"));
        let v = Vault::new(Box::new(Unavailable("none".into())));
        let (c, migrated) = Config::from_text(Some(&old), &v);
        assert!(migrated, "the file must be rewritten");
        assert_eq!(
            c.opds_catalogs[0].password, "sentinel-opds",
            "still usable this session"
        );
        let new_text = c.file_text(&v).unwrap();
        for s in secrets_of(&mut all_secrets_set()) {
            assert!(!new_text.contains(&s), "{s} survived");
        }
    }

    #[test]
    fn scrubbing_removes_secrets_from_old_copies_and_makes_the_folder_private() {
        let dir = std::env::temp_dir().join(format!("bookx-cfg-scrub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plain = serde_json::to_string(&all_secrets_set()).unwrap();
        for n in ["config.json.old", "config.json~", "config.json.bak"] {
            std::fs::write(dir.join(n), &plain).unwrap();
        }
        std::fs::write(dir.join("config.json.tmp"), &plain).unwrap();
        scrub_copies(&dir);
        assert!(!dir.join("config.json.tmp").exists());
        for n in ["config.json.old", "config.json~", "config.json.bak"] {
            let t = std::fs::read_to_string(dir.join(n)).unwrap();
            assert!(!t.contains("sentinel-") && !t.contains("share-pw"), "{n}");
            assert!(t.contains("Nextcloud"), "{n} keeps the rest");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m =
                |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(m(&dir), 0o700);
            assert_eq!(m(&dir.join("config.json.old")), 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_created_0600() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!("bookx-cfg-private-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, "old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&p, b"new").unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn a_cleared_password_is_gone_after_the_next_save() {
        let v = Vault::new(Box::new(MemoryStore::new()));
        let mut c = with_passwords();
        c.file_text(&v).unwrap();
        c.email.password.clear();
        let text = c.file_text(&v).unwrap();
        let (back, _) = Config::from_text(Some(&text), &v);
        assert!(back.email.password.is_empty());
    }
}

#[cfg(test)]
mod clean_ui_tests {
    use super::*;

    #[test]
    fn new_installs_start_with_defaults() {
        let c = Config::default();
        assert!(c.sidebar_view.is_empty() && c.favourites.is_empty());
        assert!(!c.device_sync.auto_sync && c.device_sync.include_notes);
        assert!(!c.device_list_view);
    }

    #[test]
    fn the_notes_view_lists_everything_until_a_kind_is_chosen_and_remembers_it() {
        use bookx_lookup::notes_query::KindFilter;
        assert_eq!(
            KindFilter::parse(&Config::default().notes_kind),
            KindFilter::All
        );
        let old: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(KindFilter::parse(&old.notes_kind), KindFilter::All);
        let c = Config {
            notes_kind: KindFilter::Bookmarks.name().into(),
            ..Default::default()
        };
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(KindFilter::parse(&back.notes_kind), KindFilter::Bookmarks);
    }

    #[test]
    fn an_older_config_with_the_panel_toggles_still_loads() {
        // the browse and details panel toggles were removed: the sidebar is always there and the
        // details follow the selection, so their old keys are simply ignored
        let c: Config = serde_json::from_str(
            r#"{"show_tag_browser": true, "show_details": true, "auto_details": false, "device_sync": {"auto_sync": true}}"#,
        )
        .unwrap();
        assert!(c.device_sync.auto_sync && c.device_sync.include_notes);
        assert!(c.sidebar_view.is_empty() && c.favourites.is_empty());
        // reading position sync is off until a server is set up, and an older config has none
        assert!(!c.kosync.enabled && !c.kosync.ready());
        let text = serde_json::to_string(&c).unwrap();
        assert!(!text.contains("auto_details") && !text.contains("show_details"));
    }

    #[test]
    fn favourites_and_sidebar_view_are_saved() {
        let c = Config {
            favourites: vec![3, 9],
            sidebar_view: "recent".into(),
            ..Config::default()
        };
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.favourites, vec![3, 9]);
        assert_eq!(back.sidebar_view, "recent");
    }

    #[test]
    fn the_sidebar_layout_is_kept_per_library() {
        use crate::library_nav::{SidebarItem, SidebarLayout};
        let mut c = Config::default();
        assert!(c.sidebar_layout("/a").is_default());
        let mut l = SidebarLayout::default();
        l.set_hidden(SidebarItem::Notes, true);
        l.move_item(SidebarItem::Catalogues, -1);
        c.set_sidebar_layout("/a", l.clone());
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.sidebar_layout("/a"), l);
        assert!(
            back.sidebar_layout("/b").is_default(),
            "another library is untouched"
        );
        // an old config file has no key at all
        let old: Config = serde_json::from_str("{}").unwrap();
        assert!(old.sidebar_layouts.is_empty());
        // the default is not stored
        let mut c = back;
        c.set_sidebar_layout("/a", SidebarLayout::default());
        assert!(c.sidebar_layouts.is_empty());
    }
}

#[cfg(test)]
mod hyphenation_pref_tests {
    use super::*;

    #[test]
    fn hyphenation_is_on_by_default_and_for_old_configs() {
        assert!(ReaderPrefs::default().hyphenate);
        assert!(ReaderPrefs::default().optimal_breaks);
        assert!(ReaderPrefs::default().canvas_style(false).optimal_breaks);
        let old: ReaderPrefs = serde_json::from_str(r#"{"font_size": 16.0}"#).unwrap();
        assert!(
            old.optimal_breaks,
            "configs from before the setting keep it on"
        );
        // a config written before the setting existed
        let p: ReaderPrefs = serde_json::from_str(r#"{"font_size": 18.0}"#).unwrap();
        assert!(p.hyphenate);
        assert_eq!(p.font_size, 18.0);
    }

    #[test]
    fn touch_settings_default_for_old_configs_and_round_trip() {
        use crate::gestures::{TapZones, TouchUi};
        let p: ReaderPrefs = serde_json::from_str(r#"{"font_size": 12.0}"#).unwrap();
        assert_eq!(p.tap_zones, TapZones::Edges);
        assert_eq!(p.touch_ui, TouchUi::Auto);
        let q: ReaderPrefs =
            serde_json::from_str(r#"{"tap_zones": "left_handed", "touch_ui": "on"}"#).unwrap();
        assert_eq!(q.tap_zones, TapZones::LeftHanded);
        assert_eq!(q.touch_ui, TouchUi::On);
        let json = serde_json::to_string(&q).unwrap();
        assert_eq!(
            serde_json::from_str::<ReaderPrefs>(&json)
                .unwrap()
                .tap_zones,
            TapZones::LeftHanded
        );
    }

    #[test]
    fn the_books_own_styles_are_on_by_default_and_for_old_configs() {
        assert!(ReaderPrefs::default().publisher_styles);
        let p: ReaderPrefs = serde_json::from_str(r#"{"font_size": 12.0}"#).unwrap();
        assert!(p.publisher_styles);
        let off: ReaderPrefs = serde_json::from_str(r#"{"publisher_styles": false}"#).unwrap();
        assert!(!off.publisher_styles);
    }

    #[test]
    fn hyphenation_reaches_the_canvas_style() {
        let mut p = ReaderPrefs::default();
        assert!(p.canvas_style(false).hyphenate);
        p.hyphenate = false;
        assert!(!p.canvas_style(false).hyphenate);
    }

    #[test]
    fn colour_modes_pick_the_right_palette() {
        let mut p = ReaderPrefs {
            theme: ReaderTheme::Dark,
            ..Default::default()
        };
        assert_eq!(p.palette(false), ReaderTheme::Dark.palette(false));
        assert_eq!(p.chrome_class(false), "reader-dark");
        assert!(!p.canvas_style(false).original_colors);
        p.colors = ColorMode::Original;
        assert_eq!(
            p.palette(true),
            ReaderTheme::Light.palette(false),
            "a plain light page"
        );
        assert_eq!(p.chrome_class(true), "reader-light");
        assert!(p.canvas_style(true).original_colors);
        p.colors = ColorMode::Custom;
        p.custom_fg = "#ffcc00".into();
        p.custom_bg = "#101010".into();
        let pal = p.palette(false);
        assert_eq!(to_hex(pal.fg), "#ffcc00");
        assert_eq!(to_hex(pal.bg), "#101010");
        assert_eq!(p.chrome_class(false), "reader-dark");
        assert!(pal.link.2 > 0.9, "light links on a dark page");
        p.custom_bg = "#ffffff".into();
        assert_eq!(p.chrome_class(false), "reader-light");
        p.custom_bg = "nonsense".into();
        assert_eq!(p.palette(false).bg, ReaderTheme::Light.palette(false).bg);
    }

    #[test]
    fn hex_colours_parse_and_print() {
        assert_eq!(parse_hex("#ff8000"), Some((1.0, 128.0 / 255.0, 0.0)));
        assert_eq!(parse_hex("00FF00"), Some((0.0, 1.0, 0.0)));
        for bad in ["", "#fff", "#12345g", "#1234567", "ééé"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
        assert_eq!(to_hex((1.0, 0.0, 0.5)), "#ff0080");
        assert_eq!(to_hex(parse_hex("#3a5f7c").unwrap()), "#3a5f7c");
        assert_eq!(to_hex((2.0, -1.0, 0.0)), "#ff0000");
    }

    #[test]
    fn old_preferences_files_still_load_with_default_colours() {
        let p: ReaderPrefs = serde_json::from_str(r#"{"theme":"Sepia"}"#).unwrap();
        assert_eq!(p.colors, ColorMode::Theme);
        assert!(parse_hex(&p.custom_fg).is_some() && parse_hex(&p.custom_bg).is_some());
    }
}

/// The documented default of every Preferences switch that is stored, walked through the serialized
/// defaults so a changed default or a renamed field fails here and not on a user's machine. Add a row
/// when a switch is added; the wiki's Preferences page names the same defaults.
#[cfg(test)]
mod defaults_tests {
    use super::*;

    fn bool_at(v: &serde_json::Value, path: &str) -> bool {
        let mut cur = v;
        for part in path.split('.') {
            cur = cur
                .get(part)
                .unwrap_or_else(|| panic!("no setting `{path}` in the stored defaults"));
        }
        cur.as_bool()
            .unwrap_or_else(|| panic!("`{path}` is not a switch"))
    }

    #[test]
    fn every_stored_switch_has_its_documented_default() {
        let v = serde_json::to_value(Config::default()).unwrap();
        let documented: &[(&str, bool)] = &[
            // General and library
            ("eink", false),
            ("offline", false),
            ("copy_on_add", true),
            ("details_pane", true),
            ("group_editions", false),
            // Reading
            ("footnote_popups", true),
            ("mark_vocabulary", true),
            ("reader.justify", true),
            ("reader.hyphenate", true),
            ("reader.optimal_breaks", true),
            ("reader.publisher_styles", true),
            ("ruler.enabled", false),
            // Sync: nothing runs, nothing is sent or logged in until the user says so
            ("share.running", false),
            ("share.lan", false),
            ("share.https", false),
            ("share.kosync", false),
            ("share.annotations", true),
            ("share.plugin_copied", false),
            ("kosync.enabled", false),
            ("kosync.by_filename", false),
            ("kosync.logged_in", false),
            ("device_sync.auto_sync", false),
            ("device_sync.include_notes", true),
        ];
        for (path, want) in documented {
            assert_eq!(bool_at(&v, path), *want, "default of `{path}`");
        }
    }

    #[test]
    fn the_editor_switches_have_their_documented_defaults() {
        let v = serde_json::to_value(crate::editor_prefs::EditorPrefs::default()).unwrap();
        // "Scrolling the preview scrolls the code" is on; the others start off
        for (path, want) in [
            ("scroll_both", true),
            ("minimap", true),
            ("show_whitespace", false),
            ("convert_entities", false),
            ("check_accessibility", false),
        ] {
            assert_eq!(bool_at(&v, path), want, "editor default of `{path}`");
        }
    }

    #[test]
    fn e_ink_mode_is_off_and_survives_a_save_and_a_load() {
        let vault = bookx_secrets::Vault::new(Box::new(bookx_secrets::MemoryStore::new()));
        let mut c = Config::default();
        assert!(!c.eink);
        c.eink = true;
        c.share.port = 9191;
        c.kosync.server = "http://192.168.1.5:8080".into();
        c.kosync.user = "alice".into();
        let text = c.file_text(&vault).unwrap();
        let (back, _) = Config::from_text(Some(&text), &vault);
        assert!(back.eink);
        assert_eq!(back.share.port, 9191);
        assert_eq!(back.kosync.server, "http://192.168.1.5:8080");
        assert_eq!(back.kosync.user, "alice");
        let (old, _) = Config::from_text(Some("{}"), &vault);
        assert!(!old.eink, "a file without the key reads as off");
    }
}

#[cfg(test)]
mod file_name_tests {
    use super::*;

    #[test]
    fn every_data_file_sits_next_to_the_settings_file() {
        let Some(base) = config_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
            return; // no home directory in this environment
        };
        for (path, name) in [
            (vocab_path(), "vocabulary.json"),
            (stats_path(), "stats.json"),
            (dwell_path(), "dwell.json"),
            (shortcuts_path(), "shortcuts.json"),
            (shelves_path(), "shelves.json"),
            (libraries_path(), "libraries.json"),
            (news_path(), "news.json"),
        ] {
            assert_eq!(path.unwrap(), base.join(name));
        }
        assert_eq!(
            config_file_path_text(),
            base.join("config.json").display().to_string()
        );
    }

    #[test]
    fn a_backup_destination_has_its_own_keyring_key() {
        assert_eq!(backup_password_key("nas"), "backup-nas-password");
        assert_ne!(backup_password_key("a"), backup_password_key("b"));
    }

    #[test]
    fn the_default_library_lives_in_a_folder_called_bookx_library() {
        let d = default_library_dir();
        assert!(
            d.ends_with("Bookx Library") || d.ends_with("bookx/Library"),
            "{d:?}"
        );
    }

    #[test]
    fn the_scrub_notice_is_given_once() {
        SCRUBBED.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(take_scrub_notice());
        assert!(!take_scrub_notice());
    }

    #[test]
    fn a_book_without_a_footer_of_its_own_uses_the_common_one() {
        let mut c = Config::default();
        let common = c.footer_for(7);
        let mut own = common.clone();
        own.separator = " | ".into();
        c.footer_books.insert(8, own.clone());
        assert_eq!(c.footer_for(7), common);
        assert_eq!(c.footer_for(8).separator, " | ");
    }

    #[test]
    fn the_footer_position_is_migrated_once() {
        let mut c = Config {
            footer_pos_migrated: false,
            ..Config::default()
        };
        c.migrate_footer_position();
        assert!(c.footer_pos_migrated);
        // a profile-less config has nothing to migrate
        assert!(!c.migrate_profiles(None));
    }
}
