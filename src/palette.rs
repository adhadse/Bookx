//! The command palette (Ctrl+Shift+P or Ctrl+K), without GTK: the registry of commands, the entries
//! it lists (commands, recent books, books, settings) and the fuzzy ranking.
//!
//! Commands are `gio` actions (`app.*`, `lib.*`, `reader.*`) so that the palette runs exactly what
//! the menus run. Every action named here must exist in its window: the UI scenario `palette_registry`
//! checks that.

use crate::i18n::{N_, tr};

/// Where the palette was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Library,
    Reader,
}

/// What the entry is, in the order the palette prefers on a tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Command,
    Recent,
    Setting,
    /// A file of the book open in the editor.
    File,
    Book,
}

impl Kind {
    pub fn display(self) -> String {
        tr(self.label())
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Command => N_("Command"),
            Kind::Recent => N_("Recent"),
            Kind::Setting => N_("Setting"),
            Kind::File => N_("File"),
            Kind::Book => N_("Book"),
        }
    }
}

/// What happens when the entry is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Activate a `gio` action by its full name (`app.preferences`).
    Action(String),
    /// Open a book in the reader.
    Book(u64),
    /// Open Preferences on the row with this title.
    Setting(String),
    /// Activate an action with a string parameter (an editor Tools menu item).
    ActionWith(String, String),
    /// An editor command by id (see [`EDITOR_COMMANDS`]).
    Editor(String),
    /// Open this file of the book in the editor.
    File(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    /// Display text of the shortcut ("Ctrl+O"), empty when there is none.
    pub shortcut: String,
    pub target: Target,
    pub keywords: String,
}

/// One registered command.
pub struct Command {
    pub action: &'static str,
    pub title: &'static str,
    /// GTK accelerator ("<Control>o"), shown as "Ctrl+O".
    pub accel: &'static str,
    pub keywords: &'static str,
    pub scope: Option<Scope>,
}

macro_rules! cmd {
    ($a:expr_2021, $t:expr_2021, $acc:expr_2021, $k:expr_2021, $s:expr_2021) => {
        Command {
            action: $a,
            title: $t,
            accel: $acc,
            keywords: $k,
            scope: $s,
        }
    };
}

const LIB: Option<Scope> = Some(Scope::Library);
const RDR: Option<Scope> = Some(Scope::Reader);

/// Every command of the palette. `scope: None` is offered everywhere.
pub const COMMANDS: &[Command] = &[
    cmd!(
        "lib.add-books",
        N_("Add Books"),
        "<Control>o",
        "import open file epub",
        LIB
    ),
    cmd!(
        "lib.add-web",
        N_("Add from the Web"),
        "",
        "import url article page",
        LIB
    ),
    cmd!(
        "lib.add-folder",
        N_("Add Folder"),
        "",
        "import directory",
        LIB
    ),
    cmd!(
        "lib.add-folder-recursive",
        N_("Add Folder and Subfolders"),
        "",
        "import directory recursive",
        LIB
    ),
    cmd!(
        "lib.news",
        N_("News Subscriptions"),
        "",
        "rss feed atom subscribe",
        LIB
    ),
    cmd!(
        "lib.open-folder",
        N_("Open Library Folder"),
        "",
        "files show directory",
        LIB
    ),
    cmd!(
        "lib.duplicates",
        N_("Find Duplicates"),
        "",
        "merge same copies",
        LIB
    ),
    cmd!(
        "lib.opds",
        N_("Online Catalogues"),
        "",
        "opds gutenberg standard ebooks download browse",
        LIB
    ),
    cmd!(
        "lib.saved",
        N_("Saved Searches"),
        "",
        "virtual library filter query",
        LIB
    ),
    cmd!(
        "lib.citations",
        N_("Export Citations"),
        "",
        "bibliography bibtex ris csl json apa references cite zotero",
        LIB
    ),
    cmd!(
        "lib.health",
        N_("Library Health"),
        "",
        "check repair backup integrity",
        LIB
    ),
    cmd!(
        "lib.calibre",
        N_("Import Calibre Library"),
        "",
        "migrate",
        LIB
    ),
    cmd!(
        "lib.export-calibre",
        N_("Export as Calibre Library"),
        "",
        "migrate backup leave",
        LIB
    ),
    cmd!(
        "lib.focus-search",
        N_("Search the Library"),
        "<Control>f",
        "find filter",
        LIB
    ),
    cmd!(
        "lib.select-all",
        N_("Select All Books"),
        "<Control>a",
        "",
        LIB
    ),
    cmd!(
        "lib.view-home",
        N_("Show All Books"),
        "",
        "home library sidebar",
        LIB
    ),
    cmd!(
        "lib.view-recent",
        N_("Show Recent Books"),
        "",
        "recent opened added sidebar",
        LIB
    ),
    cmd!(
        "lib.view-favourites",
        N_("Show Favourites"),
        "",
        "starred favorites sidebar",
        LIB
    ),
    cmd!(
        "lib.view-notes",
        N_("Show Notes & Highlights"),
        "",
        "annotations quotes search sidebar",
        LIB
    ),
    cmd!(
        "lib.toggle-sidebar",
        N_("Show or Hide the Sidebar"),
        "F9",
        "navigation panel hide show",
        LIB
    ),
    cmd!(
        "lib.customize-sidebar",
        N_("Customize Sidebar"),
        "",
        "show hide reorder entries groups destinations",
        LIB
    ),
    cmd!(
        "lib.browse-shelves",
        N_("Browse Shelves"),
        "",
        "collections sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-authors",
        N_("Browse Authors"),
        "",
        "writers sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-series",
        N_("Browse Series"),
        "",
        "sagas sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-tags",
        N_("Browse Tags"),
        "",
        "labels sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-publishers",
        N_("Browse Publishers"),
        "",
        "sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-languages",
        N_("Browse Languages"),
        "",
        "sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-ratings",
        N_("Browse Ratings"),
        "",
        "stars sidebar",
        LIB
    ),
    cmd!(
        "lib.browse-status",
        N_("Browse Reading Status"),
        "",
        "finished unread reading sidebar",
        LIB
    ),
    cmd!(
        "lib.favourite",
        N_("Add to Favourites"),
        "<Control>d",
        "star favorite toggle",
        LIB
    ),
    cmd!("lib.series", N_("Series Overview"), "", "order gaps", LIB),
    cmd!(
        "lib.works",
        N_("Editions & Formats"),
        "",
        "same work health merge",
        LIB
    ),
    cmd!(
        "lib.attention",
        N_("Needs Attention"),
        "",
        "weak metadata health",
        LIB
    ),
    cmd!(
        "reader.next-page",
        N_("Next Page"),
        "Right",
        "turn forward",
        RDR
    ),
    cmd!(
        "reader.prev-page",
        N_("Previous Page"),
        "Left",
        "turn back",
        RDR
    ),
    cmd!(
        "reader.next-chapter",
        N_("Next Chapter"),
        "<Control>Right",
        "section forward",
        RDR
    ),
    cmd!(
        "reader.prev-chapter",
        N_("Previous Chapter"),
        "<Control>Left",
        "section back",
        RDR
    ),
    cmd!(
        "reader.start",
        N_("Go to the Start of the Book"),
        "Home",
        "beginning first",
        RDR
    ),
    cmd!(
        "reader.end",
        N_("Go to the End of the Book"),
        "End",
        "last",
        RDR
    ),
    cmd!(
        "reader.bookmark",
        N_("Toggle Bookmark"),
        "<Control>b",
        "mark page",
        RDR
    ),
    cmd!(
        "reader.search",
        N_("Search in Book"),
        "<Control>f",
        "find text",
        RDR
    ),
    cmd!(
        "reader.goto",
        "Go to",
        "<Control>g",
        "page number percent location chapter jump printed",
        RDR
    ),
    cmd!(
        "reader.map",
        "Book Map",
        "m",
        "chapters overview strip thumbnails page browser bookmarks highlights",
        RDR
    ),
    cmd!(
        "reader.vocab-add",
        N_("Add to Vocabulary"),
        "<Control><Shift>v",
        "word term phrase own definition glossary name jargon save",
        RDR
    ),
    cmd!(
        "reader.vocab-terms",
        N_("Terms in this Book"),
        "",
        "vocabulary glossary words definitions list",
        RDR
    ),
    cmd!(
        "reader.ruler",
        "Reading Ruler",
        "",
        "line focus guide dim band concentrate dyslexia accessibility",
        RDR
    ),
    cmd!(
        "reader.ruler-up",
        "Move Ruler Up",
        "<Alt>Up",
        "line focus guide band",
        RDR
    ),
    cmd!(
        "reader.ruler-down",
        "Move Ruler Down",
        "<Alt>Down",
        "line focus guide band",
        RDR
    ),
    cmd!(
        "reader.new-theme",
        "New Custom Theme",
        "",
        "colours colors custom palette editor import export reading theme",
        RDR
    ),
    cmd!(
        "reader.use-as-default",
        "Use This Look as the Default for New Books",
        "",
        "font size spacing margins theme defaults settings",
        RDR
    ),
    cmd!(
        "reader.reset-look",
        "Reset This Book to the Default Look",
        "",
        "font size spacing margins theme defaults settings profile",
        RDR
    ),
    cmd!(
        "reader.jump-back",
        "Go Back to the Previous Place",
        "<Alt>Left",
        "history return link jump",
        RDR
    ),
    cmd!(
        "reader.jump-forward",
        "Go Forward Again",
        "<Alt>Right",
        "history redo jump",
        RDR
    ),
    cmd!(
        "reader.contents",
        N_("Contents"),
        "F9",
        "table of contents chapters toc sidebar",
        RDR
    ),
    cmd!(
        "reader.notes",
        N_("Bookmarks and Highlights"),
        "<Control><Shift>b",
        "notes annotations sidebar",
        RDR
    ),
    cmd!(
        "reader.larger",
        N_("Larger Text"),
        "<Control>plus",
        "font size zoom increase",
        RDR
    ),
    cmd!(
        "reader.smaller",
        N_("Smaller Text"),
        "<Control>minus",
        "font size zoom decrease",
        RDR
    ),
    cmd!(
        "reader.theme-light",
        N_("Theme: Light"),
        "",
        "appearance colours white",
        RDR
    ),
    cmd!(
        "reader.theme-sepia",
        N_("Theme: Sepia"),
        "",
        "appearance colours paper",
        RDR
    ),
    cmd!(
        "reader.theme-dark",
        N_("Theme: Dark"),
        "",
        "appearance colours night black",
        RDR
    ),
    cmd!(
        "reader.layout-two",
        N_("Layout: Two Columns"),
        "",
        "pages spread",
        RDR
    ),
    cmd!(
        "reader.layout-single",
        N_("Layout: Single Column"),
        "",
        "page",
        RDR
    ),
    cmd!(
        "reader.layout-scroll",
        N_("Layout: Continuous Scroll"),
        "",
        "scrolling",
        RDR
    ),
    cmd!(
        "reader.fullscreen",
        N_("Toggle Fullscreen"),
        "F11",
        "full screen",
        RDR
    ),
    cmd!(
        "reader.reload",
        N_("Reload the Book"),
        "<Control>r",
        "refresh",
        RDR
    ),
    cmd!(
        "reader.rsvp",
        N_("Speed Reading"),
        "",
        "rsvp fast read words per minute",
        RDR
    ),
    cmd!(
        "reader.auto-turn",
        N_("Auto-Turn Pages"),
        "",
        "automatic page timer slideshow",
        RDR
    ),
    cmd!(
        "reader.copy-citation",
        N_("Copy with Citation"),
        "<Control><Shift>c",
        "quote reference source",
        RDR
    ),
    cmd!(
        "reader.link-hints",
        N_("Show Link Hints"),
        "<Control>semicolon",
        "keyboard links follow letters",
        RDR
    ),
    cmd!(
        "reader.properties",
        N_("Book Properties"),
        "<Control>i",
        "info metadata details",
        RDR
    ),
    cmd!(
        "reader.reset-size",
        N_("Reset Text Size"),
        "<Control>0",
        "font zoom default normal",
        RDR
    ),
    cmd!(
        "reader.cycle-theme",
        N_("Next Theme"),
        "t",
        "light sepia dark black colours switch cycle",
        RDR
    ),
    cmd!(
        "reader.read-aloud",
        N_("Read Aloud (Play / Pause)"),
        "r",
        "speech tts voice listen narrate",
        RDR
    ),
    cmd!(
        "reader.open",
        N_("Open Another Book"),
        "<Control>o",
        "file switch library",
        RDR
    ),
    cmd!(
        "reader.back",
        N_("Back to the Library"),
        "Escape",
        "close leave",
        RDR
    ),
    cmd!(
        "app.stats",
        N_("Reading Statistics"),
        "",
        "streak time pages speed",
        None
    ),
    cmd!(
        "app.vocab",
        N_("Vocabulary"),
        "",
        "words flashcards lookup",
        None
    ),
    cmd!(
        "app.sync",
        N_("Sync"),
        "",
        "share library serve opds network bookx sync koreader usb device wifi",
        None
    ),
    cmd!(
        "app.devices",
        N_("Device"),
        "",
        "kindle kobo e-reader send sync",
        None
    ),
    cmd!(
        "app.preferences",
        N_("Preferences"),
        "<Control>comma",
        "settings options configuration",
        None
    ),
    cmd!(
        "app.help",
        N_("Online Help"),
        "F1",
        "manual wiki documentation guide",
        None
    ),
    cmd!(
        "app.shortcuts",
        N_("Keyboard Shortcuts"),
        "<Control>question",
        "keys help",
        None
    ),
    cmd!("app.about", N_("About Bookx"), "", "version licence", None),
    cmd!(
        "app.copy-diagnostics",
        N_("Copy Diagnostics"),
        "",
        "bug report support version troubleshoot",
        None
    ),
    cmd!(
        "app.report-problem",
        N_("Report a Problem"),
        "",
        "bug issue feedback crash log beta support",
        None
    ),
    cmd!("app.quit", N_("Quit"), "<Control>q", "exit close", None),
];

/// "<Control><Shift>p" as "Ctrl+Shift+P".
pub fn accel_label(accel: &str) -> String {
    if accel.is_empty() {
        return String::new();
    }
    let mut out = Vec::new();
    let mut rest = accel;
    while let Some(r) = rest.strip_prefix('<') {
        let Some((m, tail)) = r.split_once('>') else {
            break;
        };
        out.push(match m {
            "Control" | "Primary" => "Ctrl".to_string(),
            other => other.to_string(),
        });
        rest = tail;
    }
    out.push(match rest {
        "comma" => ",".into(),
        "plus" => "+".into(),
        "minus" => "-".into(),
        "question" => "?".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => k.to_string(),
    });
    out.join("+")
}

/// The commands offered in a window.
pub fn commands(scope: Scope) -> Vec<Entry> {
    COMMANDS
        .iter()
        .filter(|c| c.scope.is_none() || c.scope == Some(scope))
        .map(|c| Entry {
            kind: Kind::Command,
            title: tr(c.title),
            detail: String::new(),
            shortcut: crate::shortcuts::display(c.action, c.accel),
            target: Target::Action(c.action.to_string()),
            keywords: c.keywords.to_string(),
        })
        .collect()
}

/// The editor's own commands: (id, title, accelerator, keywords). The editor maps the id to what it does.
pub const EDITOR_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("save", N_("Save"), "<Control>s", "write book"),
    (
        "find",
        N_("Find and Replace"),
        "<Control>f",
        "search replace regex",
    ),
    ("check", N_("Check Book"), "F7", "validate problems errors"),
    ("go-to-line", N_("Go to Line"), "<Control>l", "jump"),
    (
        "quick-open",
        N_("Open File"),
        "<Control>p",
        "quick open file switch",
    ),
    ("preview", N_("Toggle Preview"), "", "live render"),
    (
        "sidebar",
        N_("Show or Hide the Sidebar"),
        "",
        "files contents book panel",
    ),
    (
        "beautify",
        N_("Reformat Markup"),
        "",
        "beautify format pretty indent reformat",
    ),
    (
        "mark",
        N_("Mark Selection"),
        "<Control><Shift>m",
        "highlight",
    ),
    ("metadata", N_("Edit Metadata"), "", "title author cover"),
    ("find-all", N_("Find All in Book"), "", "search results"),
    (
        "clear-marks",
        N_("Clear Marked Text"),
        "",
        "highlight remove",
    ),
    (
        "insert-snippet",
        N_("Insert Snippet"),
        "",
        "template expand",
    ),
    ("preferences", N_("Editor Preferences"), "", "settings font"),
    ("close-tab", N_("Close Tab"), "", "file"),
    ("next-file", N_("Next File"), "", "step spine reading order"),
    (
        "previous-file",
        N_("Previous File"),
        "",
        "step spine reading order",
    ),
];

/// The editor's commands as palette entries.
pub fn editor_commands() -> Vec<Entry> {
    EDITOR_COMMANDS
        .iter()
        .map(|(id, title, accel, keywords)| Entry {
            kind: Kind::Command,
            title: tr(title),
            detail: tr("Editor"),
            shortcut: accel_label(accel),
            target: Target::Editor(id.to_string()),
            keywords: keywords.to_string(),
        })
        .collect()
}

/// Menu label without the ellipsis and mnemonic underscores.
pub fn clean_label(label: &str) -> String {
    label
        .replace('…', "")
        .replace("...", "")
        .replace('_', "")
        .trim()
        .to_string()
}

/// An item of a menu of the editor (`group` is its menu path, like "Tools" or "Files › Restore Checkpoint").
pub fn menu_entry(group: &str, label: &str, action: &str, param: Option<&str>) -> Entry {
    Entry {
        kind: Kind::Command,
        title: clean_label(label),
        detail: group.to_string(),
        shortcut: String::new(),
        target: match param {
            Some(p) => Target::ActionWith(action.to_string(), p.to_string()),
            None => Target::Action(action.to_string()),
        },
        keywords: group.to_string(),
    }
}

/// A file of the book in the editor.
pub fn file_entry(path: &str) -> Entry {
    let name = path.rsplit('/').next().unwrap_or(path);
    Entry {
        kind: Kind::File,
        title: name.to_string(),
        detail: path.to_string(),
        shortcut: String::new(),
        target: Target::File(path.to_string()),
        keywords: path.to_string(),
    }
}

/// A book of the library. `recent` books are listed first when nothing is typed.
pub fn book_entry(id: u64, title: &str, author: &str, recent: bool) -> Entry {
    Entry {
        kind: if recent { Kind::Recent } else { Kind::Book },
        title: title.to_string(),
        detail: author.to_string(),
        shortcut: String::new(),
        target: Target::Book(id),
        keywords: author.to_string(),
    }
}

/// A row of the Preferences dialog.
pub fn setting_entry(page: &str, section: &str, title: &str, subtitle: &str) -> Entry {
    Entry {
        kind: Kind::Setting,
        title: title.to_string(),
        detail: if section.is_empty() {
            page.to_string()
        } else {
            format!("{page} › {section}")
        },
        shortcut: String::new(),
        target: Target::Setting(title.to_string()),
        keywords: format!("{section} {subtitle}"),
    }
}

/// How well `query` (one word) matches `text`, case-insensitive. `None` when it does not.
/// A match is either the letters as they are in the text, or letters in order where each one starts
/// a word or sits in the same word as the one before (so "prf" finds "Preferences" and "lh" finds
/// "Library Health", but "pref" does not wander through "Sync the reading position").
/// Higher is better: early, word starts, runs and short texts score more.
pub fn fuzzy(query: &str, text: &str) -> Option<i64> {
    let q: Vec<char> = query.to_lowercase().chars().collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let lq = q.len();
    if t.len() < lq {
        return None;
    }
    let word_start = |i: usize| i == 0 || !t[i - 1].is_alphanumeric();
    let tail = (t.len() as i64 - lq as i64) / 4;
    let mut best: Option<i64> = None;
    // the letters as they are in the text
    if let Some(p) = (0..=t.len() - lq).find(|p| t[*p..*p + lq] == q[..]) {
        let place = if p == 0 {
            50
        } else if word_start(p) {
            35
        } else {
            10
        };
        best = Some(10 * lq as i64 + 12 * (lq as i64 - 1) + place - p as i64 - tail);
    }
    // letters in order: f[j] = best score with the current letter matched at text index j
    let same_word = |a: usize, b: usize| t[a..b].iter().all(|c| c.is_alphanumeric());
    let mut f: Vec<Option<i64>> = (0..t.len())
        .map(|j| {
            (t[j] == q[0] && word_start(j)).then(|| 10 + 15 + 12 * i64::from(j == 0) - j as i64)
        })
        .collect();
    for qc in &q[1..] {
        let mut g = vec![None; t.len()];
        for j in 1..t.len() {
            if t[j] != *qc {
                continue;
            }
            let mut b: Option<i64> = None;
            for (k, prev) in f.iter().enumerate().take(j) {
                let Some(sc) = *prev else { continue };
                let link = if k + 1 == j {
                    12
                } else if same_word(k + 1, j) || word_start(j) {
                    0
                } else {
                    continue;
                };
                let add = 10 + if word_start(j) { 15 } else { 0 } + link;
                b = Some(b.map_or(sc + add, |x| x.max(sc + add)));
            }
            g[j] = b;
        }
        f = g;
    }
    if let Some(m) = f.into_iter().flatten().max() {
        let m = m - tail;
        best = Some(best.map_or(m, |b| b.max(m)));
    }
    best
}

/// Score of an entry for a typed query: every word must match the title, the detail or the keywords.
pub fn score(entry: &Entry, query: &str) -> Option<i64> {
    let mut total = 0;
    let mut any = false;
    for word in query.split_whitespace() {
        any = true;
        let title = fuzzy(word, &entry.title).map(|s| s * 3);
        let detail = fuzzy(word, &entry.detail);
        let kw = fuzzy(word, &entry.keywords).map(|s| s / 2);
        total += [title, detail, kw].into_iter().flatten().max()?;
    }
    if !any {
        return Some(0);
    }
    Some(total)
}

/// Indexes of `entries` in the order to show them. Without a query: as given (commands, recent
/// books). With one: best score first, commands before recent books before settings before books on ties.
pub fn rank(entries: &[Entry], query: &str, limit: usize) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len())
            .filter(|i| entries[*i].kind != Kind::Book)
            .take(limit)
            .collect();
    }
    let mut hits: Vec<(i64, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| score(e, query).map(|s| (s, i)))
        .collect();
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(entries[a.1].kind.cmp(&entries[b.1].kind))
            .then(entries[a.1].title.cmp(&entries[b.1].title))
    });
    hits.into_iter().take(limit).map(|(_, i)| i).collect()
}

/// Whether the key moves the selection (true = down) in the list; the entry keeps the typing.
pub fn list_move(key: &str, ctrl: bool) -> Option<i32> {
    match key {
        "Down" => Some(1),
        "Up" => Some(-1),
        "n" | "j" if ctrl => Some(1),
        "p" | "k" if ctrl => Some(-1),
        "Page_Down" => Some(5),
        "Page_Up" => Some(-5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Entry> {
        let mut v = commands(Scope::Library);
        v.push(book_entry(1, "Alice in Wonderland", "Lewis Carroll", true));
        v.push(book_entry(2, "Moby Dick", "Herman Melville", false));
        v.push(book_entry(3, "The Preface", "A. Writer", false));
        v.push(setting_entry(
            "Reading",
            "Reader defaults",
            "Hyphenate",
            "Break long words",
        ));
        v.push(setting_entry("General", "Appearance", "Color scheme", ""));
        v
    }

    fn titles(e: &[Entry], idx: &[usize]) -> Vec<String> {
        idx.iter().map(|i| e[*i].title.clone()).collect()
    }

    #[test]
    fn fuzzy_needs_letters_in_order() {
        assert!(fuzzy("pref", "Preferences").is_some());
        assert!(fuzzy("prf", "Preferences").is_some());
        assert!(fuzzy("zzz", "Preferences").is_none());
        assert!(fuzzy("fp", "Preferences").is_none());
        assert_eq!(fuzzy("", "x"), Some(0));
        assert!(
            fuzzy("PREF", "preferences").is_some(),
            "case does not matter"
        );
    }

    #[test]
    fn letters_do_not_wander_between_words() {
        assert!(fuzzy("pref", "Sync the reading position").is_none());
        assert!(
            fuzzy("lh", "Library Health…").is_some(),
            "initials of words"
        );
        assert!(
            fuzzy("libhe", "Library Health").is_some(),
            "start of each word"
        );
        assert!(
            fuzzy("ibrar", "Library Health").is_some(),
            "a run inside a word"
        );
        assert!(fuzzy("ytl", "Library Health").is_none());
    }

    #[test]
    fn better_matches_score_higher() {
        let prefix = fuzzy("pref", "Preferences").unwrap();
        let scattered = fuzzy("pref", "Please run every file").unwrap();
        assert!(prefix > scattered);
        assert!(fuzzy("ab", "ab cd").unwrap() > fuzzy("ab", "a x b").unwrap());
    }

    #[test]
    fn typing_preferences_puts_the_command_first() {
        let e = all();
        let r = rank(&e, "pref", 5);
        assert_eq!(e[r[0]].title, "Preferences");
    }

    #[test]
    fn empty_query_lists_commands_and_recent_books_but_not_every_book() {
        let e = all();
        let r = rank(&e, "", 100);
        let t = titles(&e, &r);
        assert!(t.contains(&"Alice in Wonderland".to_string()));
        assert!(!t.contains(&"Moby Dick".to_string()));
        assert!(t.contains(&"Preferences".to_string()));
    }

    #[test]
    fn books_are_found_by_title_or_author() {
        let e = all();
        let by_title = rank(&e, "moby", 3);
        assert_eq!(e[by_title[0]].title, "Moby Dick");
        let by_author = rank(&e, "melville", 3);
        assert_eq!(e[by_author[0]].title, "Moby Dick");
        assert_eq!(e[rank(&e, "carroll", 3)[0]].target, Target::Book(1));
    }

    #[test]
    fn settings_are_found_by_title_section_or_hint() {
        let e = all();
        assert_eq!(e[rank(&e, "hyphen", 3)[0]].title, "Hyphenate");
        assert_eq!(e[rank(&e, "long words", 3)[0]].title, "Hyphenate");
        let r = rank(&e, "appearance", 3);
        assert!(titles(&e, &r).contains(&"Color scheme".to_string()));
        assert_eq!(
            e[rank(&e, "hyphen", 1)[0]].detail,
            "Reading › Reader defaults"
        );
    }

    #[test]
    fn every_word_must_match() {
        let e = all();
        assert!(rank(&e, "moby zebra", 5).is_empty());
        let r = rank(&e, "lewis alice", 5);
        assert_eq!(e[r[0]].title, "Alice in Wonderland");
    }

    #[test]
    fn limit_and_no_match() {
        let e = all();
        assert_eq!(rank(&e, "a", 3).len(), 3);
        assert!(rank(&e, "qqqqqq", 3).is_empty());
    }

    #[test]
    fn registry_is_consistent() {
        let mut seen = std::collections::HashSet::new();
        for c in COMMANDS {
            assert!(
                seen.insert((c.action, c.scope.is_some_and(|s| s == Scope::Reader))),
                "duplicate {}",
                c.action
            );
            assert!(c.action.contains('.'), "{} is not group.name", c.action);
            assert!(!c.title.is_empty());
            if !c.accel.is_empty() {
                assert!(
                    gtk_free_accel_ok(c.accel),
                    "{} has a bad accelerator {}",
                    c.action,
                    c.accel
                );
            }
            match c.scope {
                Some(Scope::Library) => assert!(c.action.starts_with("lib.")),
                Some(Scope::Reader) => assert!(c.action.starts_with("reader.")),
                None => assert!(c.action.starts_with("app.")),
            }
        }
        assert!(commands(Scope::Library).iter().all(|e| !e.title.is_empty()));
        assert!(commands(Scope::Reader).len() > 20);
        assert!(
            !commands(Scope::Library)
                .iter()
                .any(|e| e.target == Target::Action("reader.next-page".into()))
        );
    }

    fn gtk_free_accel_ok(a: &str) -> bool {
        let rest = a.rsplit('>').next().unwrap_or(a);
        !rest.is_empty() && !rest.contains('<')
    }

    #[test]
    fn accelerators_are_shown_for_people() {
        assert_eq!(accel_label("<Control>o"), "Ctrl+O");
        assert_eq!(accel_label("<Control><Shift>p"), "Ctrl+Shift+P");
        assert_eq!(accel_label("<Control>comma"), "Ctrl+,");
        assert_eq!(accel_label("F11"), "F11");
        assert_eq!(accel_label("Right"), "Right");
        assert_eq!(accel_label(""), "");
    }

    #[test]
    fn list_keys() {
        assert_eq!(list_move("Down", false), Some(1));
        assert_eq!(list_move("Up", false), Some(-1));
        assert_eq!(list_move("n", true), Some(1));
        assert_eq!(list_move("n", false), None);
        assert_eq!(list_move("a", false), None);
    }

    #[test]
    fn editor_palette_finds_commands_menu_items_and_files() {
        let mut e = editor_commands();
        e.push(menu_entry(
            "Tools",
            "Split File at Chapter Headings",
            "tools.split",
            None,
        ));
        e.push(menu_entry(
            "Tools",
            "Fix Problems…",
            "tools.run",
            Some("fix"),
        ));
        e.push(file_entry("OEBPS/text/chapter1.xhtml"));
        assert_eq!(
            e[rank(&e, "save", 3)[0]].target,
            Target::Editor("save".into())
        );
        assert_eq!(
            e[rank(&e, "split", 3)[0]].target,
            Target::Action("tools.split".into())
        );
        assert_eq!(
            e[rank(&e, "fix prob", 3)[0]].target,
            Target::ActionWith("tools.run".into(), "fix".into())
        );
        assert_eq!(e[rank(&e, "chapter1", 3)[0]].kind, Kind::File);
        assert_eq!(e[rank(&e, "save", 3)[0]].shortcut, "Ctrl+S");
    }

    #[test]
    fn menu_labels_are_cleaned_and_editor_ids_unique() {
        assert_eq!(clean_label("_Rename Tag…"), "Rename Tag");
        let mut ids: Vec<&str> = EDITOR_COMMANDS.iter().map(|c| c.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), EDITOR_COMMANDS.len());
    }
}
