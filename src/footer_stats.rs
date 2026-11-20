//! The reading statistics shown in the footer of the reader: which items, in which order, how they
//! read. No GTK: the reader fills in [`Facts`] and shows the text this module makes.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Item {
    PageOfTotal,
    ChapterPage,
    Percent,
    TimeLeftChapter,
    TimeLeftBook,
    Speed,
    Clock,
    Battery,
    ChapterTitle,
    BookTitle,
    Author,
    ProgressBar,
    ChaptersRead,
    SessionTime,
    TodayReading,
}

impl Item {
    pub const ALL: [Item; 15] = [
        Item::PageOfTotal,
        Item::ChapterPage,
        Item::Percent,
        Item::TimeLeftChapter,
        Item::TimeLeftBook,
        Item::Speed,
        Item::Clock,
        Item::Battery,
        Item::ChapterTitle,
        Item::BookTitle,
        Item::Author,
        Item::ProgressBar,
        Item::ChaptersRead,
        Item::SessionTime,
        Item::TodayReading,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Item::PageOfTotal => "page",
            Item::ChapterPage => "chapter_page",
            Item::Percent => "percent",
            Item::TimeLeftChapter => "left_chapter",
            Item::TimeLeftBook => "left_book",
            Item::Speed => "speed",
            Item::Clock => "clock",
            Item::Battery => "battery",
            Item::ChapterTitle => "chapter",
            Item::BookTitle => "title",
            Item::Author => "author",
            Item::ProgressBar => "bar",
            Item::ChaptersRead => "chapters",
            Item::SessionTime => "session",
            Item::TodayReading => "today",
        }
    }

    pub fn from_key(k: &str) -> Option<Item> {
        Item::ALL.into_iter().find(|i| i.key() == k)
    }

    /// The name in the settings (untranslated; the caller runs it through `tr`).
    pub fn label(self) -> &'static str {
        match self {
            Item::PageOfTotal => crate::i18n::N_("Page X of Y"),
            Item::ChapterPage => crate::i18n::N_("Page in the chapter"),
            Item::Percent => crate::i18n::N_("Percent read"),
            Item::TimeLeftChapter => crate::i18n::N_("Time left in the chapter"),
            Item::TimeLeftBook => crate::i18n::N_("Time left in the book"),
            Item::Speed => crate::i18n::N_("Reading speed"),
            Item::Clock => crate::i18n::N_("Current time"),
            Item::Battery => crate::i18n::N_("Battery"),
            Item::ChapterTitle => crate::i18n::N_("Chapter title"),
            Item::BookTitle => crate::i18n::N_("Book title"),
            Item::Author => crate::i18n::N_("Author"),
            Item::ProgressBar => crate::i18n::N_("Progress bar"),
            Item::ChaptersRead => crate::i18n::N_("Chapters read"),
            Item::SessionTime => crate::i18n::N_("Time reading this session"),
            Item::TodayReading => crate::i18n::N_("Reading today"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub item: Item,
    pub on: bool,
}

/// Where the text goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// The first item at the left, the others at the right.
    Spread,
    /// One line in the middle of the page: between the two columns of a two-column page, in
    /// the middle of the column of a single-column page.
    Centered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FooterConfig {
    /// Every item once, in the order shown.
    pub items: Vec<Entry>,
    pub separator: String,
    /// `None`: the default, one centred line (the gutter between two columns, the middle of a
    /// single column). Only a position the user chose is stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// Text size in percent of the normal footer size.
    pub size_pct: u32,
    /// Opacity of the text in percent.
    pub opacity_pct: u32,
}

pub const SEPARATORS: [&str; 4] = ["  ·  ", "  |  ", "  •  ", "   "];
pub const SIZE_RANGE: (u32, u32) = (70, 160);
pub const OPACITY_RANGE: (u32, u32) = (30, 100);

impl Default for FooterConfig {
    fn default() -> Self {
        Preset::Standard.config()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Minimal,
    Standard,
    Everything,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Minimal, Preset::Standard, Preset::Everything];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Minimal => crate::i18n::N_("Minimal"),
            Preset::Standard => crate::i18n::N_("Standard"),
            Preset::Everything => crate::i18n::N_("Everything"),
        }
    }

    pub fn config(self) -> FooterConfig {
        use Item::*;
        let on: &[Item] = match self {
            Preset::Minimal => &[PageOfTotal],
            Preset::Standard => &[PageOfTotal, ChapterTitle, TimeLeftChapter, ProgressBar],
            Preset::Everything => &Item::ALL,
        };
        let mut items: Vec<Entry> = on.iter().map(|&item| Entry { item, on: true }).collect();
        for item in Item::ALL {
            if !on.contains(&item) {
                items.push(Entry { item, on: false });
            }
        }
        FooterConfig {
            items,
            separator: SEPARATORS[0].into(),
            align: None,
            size_pct: 100,
            opacity_pct: if self == Preset::Minimal { 70 } else { 100 },
        }
    }
}

impl FooterConfig {
    /// Makes a config read from a file usable: every item exactly once, values in range.
    pub fn normalized(mut self) -> FooterConfig {
        let mut seen: Vec<Item> = Vec::new();
        self.items.retain(|e| {
            if seen.contains(&e.item) {
                false
            } else {
                seen.push(e.item);
                true
            }
        });
        for item in Item::ALL {
            if !seen.contains(&item) {
                self.items.push(Entry { item, on: false });
            }
        }
        self.size_pct = self.size_pct.clamp(SIZE_RANGE.0, SIZE_RANGE.1);
        self.opacity_pct = self.opacity_pct.clamp(OPACITY_RANGE.0, OPACITY_RANGE.1);
        self
    }

    pub fn is_on(&self, item: Item) -> bool {
        self.items.iter().any(|e| e.item == item && e.on)
    }

    pub fn set_on(&mut self, item: Item, on: bool) {
        if let Some(e) = self.items.iter_mut().find(|e| e.item == item) {
            e.on = on;
        }
    }

    /// Moves the item at `from` to `to` (both indices into [`FooterConfig::items`]).
    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return;
        }
        let e = self.items.remove(from);
        self.items.insert(to, e);
    }

    /// Which preset this config is, if it is exactly one.
    pub fn preset(&self) -> Option<Preset> {
        let mut me = self.clone();
        me.align = Some(self.align());
        Preset::ALL.into_iter().find(|p| {
            let mut c = p.config();
            c.align = Some(c.align());
            c == me
        })
    }

    /// Where the text goes (the default is the middle).
    pub fn align(&self) -> Align {
        self.align.unwrap_or(Align::Centered)
    }

    /// Versions before 2026-11-13 stored the position of the Standard preset (left and right) as
    /// if it were a choice. A stored Standard footer with that position was not chosen by the
    /// user, so it goes back to the default (the middle). Run once per config.
    pub fn migrate_old_default(&mut self) {
        let mut old = Preset::Standard.config();
        old.align = Some(Align::Spread);
        if *self == old {
            self.align = None;
        }
    }

    /// Whether anything shown changes with the clock (the reader then refreshes it).
    pub fn needs_clock(&self) -> bool {
        [
            Item::Clock,
            Item::Battery,
            Item::SessionTime,
            Item::TodayReading,
        ]
        .into_iter()
        .any(|i| self.is_on(i))
    }
}

/// What the reader knows right now.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// "12 of 340", or "Page iv" in a book with printed pages.
    pub page_text: String,
    pub chapter_page: usize,
    pub chapter_pages: usize,
    /// 0..=1
    pub fraction: f64,
    pub chapter_secs_left: Option<u64>,
    pub book_secs_left: Option<u64>,
    pub wpm: Option<f64>,
    pub clock: String,
    pub battery: Option<u8>,
    pub chapter_title: String,
    pub book_title: String,
    pub author: String,
    /// Chapter being read (1-based) and how many there are.
    pub chapter_no: usize,
    pub chapters: usize,
    pub session_secs: u64,
    pub today_secs: u64,
}

/// "N minutes to complete", the sentence the footer always had.
pub fn minutes_to_complete(secs: u64) -> String {
    let m = secs.div_ceil(60);
    if m <= 1 {
        "less than a minute to complete".into()
    } else {
        format!("{m} minutes to complete")
    }
}

fn duration(secs: u64) -> String {
    bookx_lookup::stats::format_duration(secs)
}

/// The text of one item; `None` when there is nothing to say (no battery, no speed yet).
pub fn item_text(item: Item, f: &Facts) -> Option<String> {
    let t = match item {
        Item::PageOfTotal => f.page_text.clone(),
        Item::ChapterPage => {
            if f.chapter_pages == 0 {
                return None;
            }
            format!("{} / {} in chapter", f.chapter_page, f.chapter_pages)
        }
        Item::Percent => format!("{:.0}%", (f.fraction * 100.0).clamp(0.0, 100.0)),
        Item::TimeLeftChapter => minutes_to_complete(f.chapter_secs_left?),
        Item::TimeLeftBook => format!("{} left in book", duration(f.book_secs_left?)),
        Item::Speed => format!("{:.0} words/min", f.wpm.filter(|w| *w > 0.0)?),
        Item::Clock => f.clock.clone(),
        Item::Battery => format!("{}%", f.battery?.min(100)),
        Item::ChapterTitle => f.chapter_title.clone(),
        Item::BookTitle => f.book_title.clone(),
        Item::Author => f.author.clone(),
        Item::ProgressBar => return None,
        Item::ChaptersRead => {
            if f.chapters == 0 {
                return None;
            }
            format!("Chapter {} of {}", f.chapter_no.max(1), f.chapters)
        }
        Item::SessionTime => format!("{} this session", duration(f.session_secs)),
        Item::TodayReading => format!("{} today", duration(f.today_secs)),
    };
    (!t.trim().is_empty()).then_some(t)
}

/// What the footer shows: the two text parts and whether the progress bar is on.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rendered {
    /// Text at the start (empty when centred).
    pub left: String,
    /// The middle line (empty when spread).
    pub middle: String,
    /// Text at the end (empty when centred).
    pub right: String,
    pub bar: bool,
}

pub fn render(cfg: &FooterConfig, f: &Facts) -> Rendered {
    let parts: Vec<String> = cfg
        .items
        .iter()
        .filter(|e| e.on)
        .filter_map(|e| item_text(e.item, f))
        .collect();
    let mut out = Rendered {
        bar: cfg.is_on(Item::ProgressBar),
        ..Default::default()
    };
    match cfg.align() {
        Align::Centered => out.middle = parts.join(&cfg.separator),
        Align::Spread => {
            if let Some((first, rest)) = parts.split_first() {
                out.left = first.clone();
                out.right = rest.join(&cfg.separator);
            }
        }
    }
    out
}

/// The charge of the first battery in `dir` (`/sys/class/power_supply`), `None` on a desktop.
pub fn read_battery(dir: &Path) -> Option<u8> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("BAT"))
        })
        .collect();
    names.sort();
    let cap = std::fs::read_to_string(names.first()?.join("capacity")).ok()?;
    cap.trim().parse::<u8>().ok().map(|v| v.min(100))
}

pub fn system_battery() -> Option<u8> {
    read_battery(Path::new("/sys/class/power_supply"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            page_text: "12 of 340".into(),
            chapter_page: 3,
            chapter_pages: 20,
            fraction: 0.456,
            chapter_secs_left: Some(600),
            book_secs_left: Some(3 * 3600 + 300),
            wpm: Some(231.6),
            clock: "14:05".into(),
            battery: Some(80),
            chapter_title: "Chapter 2".into(),
            book_title: "Dune".into(),
            author: "Frank Herbert".into(),
            chapter_no: 2,
            chapters: 12,
            session_secs: 1500,
            today_secs: 4000,
        }
    }

    #[test]
    fn every_preset_is_centred_by_default() {
        for p in Preset::ALL {
            assert_eq!(p.config().align(), Align::Centered);
            assert_eq!(p.config().align, None, "the default is not stored");
        }
        let r = render(&Preset::Standard.config(), &facts());
        assert_eq!(
            r.middle,
            "12 of 340  ·  Chapter 2  ·  10 minutes to complete"
        );
        assert!(r.left.is_empty() && r.right.is_empty());
        assert!(r.bar);
    }

    #[test]
    fn left_and_right_stay_selectable() {
        let mut c = Preset::Standard.config();
        c.align = Some(Align::Spread);
        let r = render(&c, &facts());
        assert_eq!(r.left, "12 of 340");
        assert_eq!(r.right, "Chapter 2  ·  10 minutes to complete");
        assert_eq!(r.middle, "");
        assert_eq!(c.preset(), None, "a chosen position is a custom footer");
        c.align = Some(Align::Centered);
        assert_eq!(
            c.preset(),
            Some(Preset::Standard),
            "an explicit middle is the preset too"
        );
    }

    #[test]
    fn the_old_stored_default_goes_back_to_the_middle() {
        // what versions before this one wrote for a user who never touched the position
        let mut old = Preset::Standard.config();
        old.align = Some(Align::Spread);
        let json = serde_json::to_string(&old).unwrap();
        assert!(json.contains("\"align\":\"spread\""));
        let mut c: FooterConfig = serde_json::from_str(&json).unwrap();
        c.migrate_old_default();
        assert_eq!(c.align, None);
        assert_eq!(c.align(), Align::Centered);
        // a user who also changed something else keeps the position they chose
        let mut mine = old.clone();
        mine.set_on(Item::Percent, true);
        mine.migrate_old_default();
        assert_eq!(mine.align, Some(Align::Spread));
        // an explicit middle stays as it is
        let mut mid = Preset::Everything.config();
        mid.align = Some(Align::Centered);
        mid.migrate_old_default();
        assert_eq!(mid.align, Some(Align::Centered));
        // a config without a position reads as the default
        let d: FooterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(d.align(), Align::Centered);
        assert!(!serde_json::to_string(&Preset::Standard.config())
            .unwrap()
            .contains("align"));
    }

    #[test]
    fn minimal_is_one_centred_item_without_a_bar() {
        let r = render(&Preset::Minimal.config(), &facts());
        assert_eq!(r.middle, "12 of 340");
        assert!(r.left.is_empty() && r.right.is_empty());
        assert!(!r.bar);
    }

    #[test]
    fn everything_shows_each_item_in_order() {
        let r = render(&Preset::Everything.config(), &facts());
        assert_eq!(
            r.middle,
            "12 of 340  ·  3 / 20 in chapter  ·  46%  ·  10 minutes to complete  ·  3 h 05 min left in book  ·  232 words/min  ·  14:05  ·  80%  ·  Chapter 2  ·  Dune  ·  Frank Herbert  ·  Chapter 2 of 12  ·  25 min this session  ·  1 h 07 min today"
        );
        assert!(r.bar);
    }

    #[test]
    fn items_without_data_are_left_out() {
        let mut f = facts();
        f.battery = None;
        f.wpm = None;
        f.book_secs_left = None;
        assert_eq!(item_text(Item::Battery, &f), None);
        assert_eq!(item_text(Item::Speed, &f), None);
        assert_eq!(item_text(Item::TimeLeftBook, &f), None);
        let r = render(&Preset::Everything.config(), &f);
        assert!(!r.middle.contains("words/min") && !r.middle.contains("80%"));
        assert!(!r.middle.contains("  ·    ·  "));
    }

    #[test]
    fn minutes_wording_matches_the_old_footer() {
        assert_eq!(minutes_to_complete(0), "less than a minute to complete");
        assert_eq!(minutes_to_complete(60), "less than a minute to complete");
        assert_eq!(minutes_to_complete(61), "2 minutes to complete");
    }

    #[test]
    fn order_and_switches_change_the_line() {
        let mut c = Preset::Everything.config();
        for e in c.items.iter_mut() {
            e.on = matches!(e.item, Item::Percent | Item::Clock | Item::Author);
        }
        c.move_item(0, 14);
        c.move_item(9, 0);
        let first_on: Vec<Item> = c.items.iter().filter(|e| e.on).map(|e| e.item).collect();
        assert_eq!(first_on[0], Item::Author);
        c.separator = " | ".into();
        c.align = Some(Align::Centered);
        let r = render(&c, &facts());
        assert_eq!(r.middle, "Frank Herbert | 46% | 14:05");
        c.move_item(99, 0);
        assert_eq!(c.items.len(), 15);
        assert_eq!(c.preset(), None);
        assert_eq!(Preset::Standard.config().preset(), Some(Preset::Standard));
    }

    #[test]
    fn a_config_from_a_file_is_repaired() {
        let c: FooterConfig = serde_json::from_str(
            r#"{"items":[{"item":"clock","on":true},{"item":"clock","on":false}],"size_pct":5,"opacity_pct":900}"#,
        )
        .unwrap();
        let c = c.normalized();
        assert_eq!(c.items.len(), 15);
        assert!(c.is_on(Item::Clock));
        assert_eq!((c.size_pct, c.opacity_pct), (70, 100));
        let d: FooterConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(d, FooterConfig::default(), "missing keys take the defaults");
    }

    #[test]
    fn keys_round_trip() {
        for i in Item::ALL {
            assert_eq!(Item::from_key(i.key()), Some(i));
        }
        let json = serde_json::to_string(&Preset::Everything.config()).unwrap();
        let back: FooterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Preset::Everything.config());
    }

    #[test]
    fn the_clock_is_only_needed_for_live_items() {
        assert!(!Preset::Standard.config().needs_clock());
        assert!(Preset::Everything.config().needs_clock());
    }

    #[test]
    fn battery_is_read_from_a_power_supply_folder() {
        let d = std::env::temp_dir().join(format!("bookx-bat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(read_battery(&d), None);
        std::fs::create_dir_all(d.join("AC")).unwrap();
        std::fs::create_dir_all(d.join("BAT0")).unwrap();
        assert_eq!(read_battery(&d), None);
        std::fs::write(d.join("BAT0/capacity"), "57\n").unwrap();
        assert_eq!(read_battery(&d), Some(57));
        let _ = std::fs::remove_dir_all(&d);
    }
}
