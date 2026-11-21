//! What the library window's sidebar and selection decide, without GTK: the views (Home, Recent,
//! Favourites, Notes & Highlights), the favourites list, the right-hand details panel following the
//! selection, and how a click or right-click changes the selection (bug 8).

use crate::core::library::SortKey;
use crate::i18n::N_;
use std::collections::HashSet;

/// The values of the Continue Reading banner setting (`""` is automatic).
pub const BANNER_MODES: [&str; 4] = ["", "large", "compact", "hidden"];

/// The two kinds of Continue Reading banner: the hero for the book in progress when the app was
/// just opened (or after a long absence), and the slim strip on the visits after that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Hero,
    Strip,
}

/// What this run of the app has shown so far (a timestamp in Unix seconds).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeroSession {
    /// When the hero was last on screen (while it is shown: now).
    pub last_hero: Option<i64>,
    /// The hero is on screen now (the user has not left Home since it appeared).
    pub active: bool,
}

/// After this long without the hero the next visit gets it again.
/// Where a click in the sidebar goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTarget<'a> {
    /// Home, Recent, Favourites, Notes & Highlights: all the books of that view.
    View(View),
    /// A filter row (an author, tag, series, shelf, saved search…): the search it stands for.
    Filter(&'a str),
}

/// What the search bar shows after a sidebar click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
}

/// A filter row opens the search bar with its query, so it is clear a filter is active; a view
/// closes the bar and clears the query (whatever was typed by hand too).
pub fn search_for(target: NavTarget) -> SearchState {
    match target {
        NavTarget::Filter(q) if !q.trim().is_empty() => SearchState {
            open: true,
            query: q.to_string(),
        },
        _ => SearchState {
            open: false,
            query: String::new(),
        },
    }
}

/// The smallest library window (the narrow layout: sidebar as overlay, minimal header).
pub const MIN_WIDTH: i32 = 360;
pub const MIN_HEIGHT: i32 = 294;
/// The narrowest the sidebar is when docked, and how wide the sidebar and the details panel get
/// (the split views size them as a share of the window, up to these).
pub const SIDEBAR_MIN: i32 = 250;
pub const SIDEBAR_MAX: i32 = 320;
pub const DETAILS_MAX: i32 = 380;
/// The room the books need next to the sidebar, and next to the sidebar and the details panel.
pub const CONTENT_MIN: i32 = 670;
pub const GRID_MIN: i32 = 470;
/// Up to this window width (in sp) the sidebar is an overlay.
pub const SIDEBAR_COLLAPSE_AT: f64 = (SIDEBAR_MIN + CONTENT_MIN) as f64;
/// Up to this width the details panel is an overlay (the widest sidebar and panel and a grid of
/// `GRID_MIN` do not fit).
pub const DETAILS_COLLAPSE_AT: f64 = (SIDEBAR_MAX + DETAILS_MAX + GRID_MIN) as f64;
/// Below this width the header drops its optional buttons into the view options.
pub const HEADER_COMPACT_AT: f64 = 600.0;

/// How the library window is laid out at a width (in sp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowLayout {
    pub sidebar_overlay: bool,
    pub details_overlay: bool,
    pub header_compact: bool,
}

pub fn window_layout(width: i32) -> WindowLayout {
    let w = width as f64;
    WindowLayout {
        sidebar_overlay: w <= SIDEBAR_COLLAPSE_AT,
        details_overlay: w <= DETAILS_COLLAPSE_AT || w <= SIDEBAR_COLLAPSE_AT,
        header_compact: w <= HEADER_COMPACT_AT,
    }
}

pub const HERO_AWAY_SECS: i64 = 30 * 60;

/// Which banner a visit to Home gets: the hero on a cold start, while its visit goes on and after
/// half an hour away; the strip on the visits in between.
pub fn banner_kind(s: &HeroSession, now: i64) -> BannerKind {
    if s.active {
        return BannerKind::Hero;
    }
    match s.last_hero {
        None => BannerKind::Hero,
        Some(t) if now - t >= HERO_AWAY_SECS => BannerKind::Hero,
        Some(_) => BannerKind::Strip,
    }
}

/// The hero is on screen.
pub fn hero_shown(s: &mut HeroSession, now: i64) {
    s.active = true;
    s.last_hero = Some(now);
}

/// The user left Home (opened a book, switched view): the hero visit is over.
pub fn left_home(s: &mut HeroSession, now: i64) {
    if s.active {
        s.active = false;
        s.last_hero = Some(now);
    }
}

/// The layouts of the banner, from the widest to the narrowest. The widths and heights between
/// the breakpoints are continuous (the height grows with the viewport).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// text | big cover (and the other books being read, when there is room)
    Wide,
    /// text | medium cover
    Medium,
    /// one slim row: small cover | title, author, progress | Continue
    Strip,
    /// title and a play button
    Mini,
}

/// The widths (px of the book area) where the layout changes.
pub const WIDE_FROM: f64 = 900.0;
pub const MEDIUM_FROM: f64 = 640.0;
pub const STRIP_FROM: f64 = 420.0;
/// The other books being read show as chips from this width on.
pub const CHIPS_FROM: f64 = 1150.0;
/// The hero needs 232 px (its text, title, buttons and cover) and a quarter of the viewport is the most it may take on
/// a small one, so it comes from a viewport of 928 px; below it the strip.
pub const HERO_MIN_VH: f64 = 928.0;
/// The footprint of the strip: 64 px card and its margins.
pub const STRIP_FOOTPRINT: f64 = 88.0;

/// How the banner looks: its layout, its height when the books are at the top (`full`, with
/// margins) and the height it shrinks to (`slim`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BannerPlan {
    pub variant: Variant,
    pub full: f64,
    pub slim: f64,
}

impl BannerPlan {
    /// The big forms (a cover that fills the card).
    pub fn hero(&self) -> bool {
        matches!(self.variant, Variant::Wide | Variant::Medium)
    }
}

/// The hero's footprint: a quarter of the viewport on a small one, growing to a third (up to 520 px).
pub fn hero_height(vh: f64) -> f64 {
    let t = ((vh - HERO_MIN_VH) / 172.0).clamp(0.0, 1.0);
    (vh * (0.25 + 0.075 * t)).clamp(232.0, 520.0)
}

/// The banner for a setting (`""` adaptive, `large`, `compact`, `hidden`), the kind a visit gets
/// and a viewport of `vh` x `vw` pixels (the books and the banner together); `None` when hidden.
pub fn banner_plan(mode: &str, kind: BannerKind, vh: f64, vw: f64) -> Option<BannerPlan> {
    let kind = match mode {
        "hidden" => return None,
        "compact" => BannerKind::Strip,
        "large" => BannerKind::Hero,
        _ => kind,
    };
    let strip = |vw: f64| BannerPlan {
        variant: if vw >= STRIP_FROM {
            Variant::Strip
        } else {
            Variant::Mini
        },
        full: STRIP_FOOTPRINT,
        slim: STRIP_FOOTPRINT,
    };
    if kind == BannerKind::Strip || vh < HERO_MIN_VH || vw < MEDIUM_FROM {
        return Some(strip(vw));
    }
    let wide = vw >= WIDE_FROM;
    let h = hero_height(vh);
    Some(BannerPlan {
        variant: if wide { Variant::Wide } else { Variant::Medium },
        // a medium window has less to show: a little lower, never under 232
        full: if wide { h } else { (h * 0.9).max(232.0) },
        slim: STRIP_FOOTPRINT,
    })
}

/// "about 25 min left" for a book of `size` bytes read to `progress` (0-1): a rough guess from the
/// file size (about 2 characters of text per byte of the zipped book, 6 characters a word, 250
/// words a minute). Empty when there is nothing sensible to say.
pub fn time_left_text(size: u64, progress: f64) -> String {
    let total = size as f64 * 2.0 / 6.0 / 250.0;
    let left = (total * (1.0 - progress.clamp(0.0, 1.0))).round() as u64;
    match left {
        0 => String::new(),
        1..=59 => crate::i18n::trf("about {} left", &[&format!("{left} min")]),
        m => crate::i18n::trf(
            "about {} left",
            &[&format!("{} h {:02} min", m / 60, m % 60)],
        ),
    }
}

/// What the banner shows after the books were scrolled by `offset` pixels of banner: the first
/// part shrinks the card to its slim size, the rest slides it off. Returns
/// (card height, visible height, how far the card is slid up).
pub fn banner_geometry(plan: BannerPlan, offset: f64) -> (f64, f64, f64) {
    let off = offset.clamp(0.0, plan.full);
    let shrink = (plan.full - plan.slim).max(0.0);
    if off <= shrink {
        let t = if shrink > 0.0 { off / shrink } else { 0.0 };
        let h = plan.full + (plan.slim - plan.full) * t;
        (h, h, 0.0)
    } else {
        let slide = off - shrink;
        (plan.slim, (plan.slim - slide).max(0.0), slide)
    }
}

/// The wheel moves the banner first, then the books: returns (new offset in pixels, whether the
/// banner used the scroll).
pub fn banner_scroll(offset: f64, banner_h: f64, books_at_top: bool, delta: f64) -> (f64, bool) {
    if banner_h <= 0.0 {
        return (0.0, false);
    }
    if delta > 0.0 && offset < banner_h {
        return ((offset + delta).min(banner_h), true);
    }
    if delta < 0.0 && books_at_top && offset > 0.0 {
        return ((offset + delta).max(0.0), true);
    }
    (offset, false)
}

/// Takes the `fav:` terms (`fav:true`, `fav:false`, `-fav:true`) out of a search: the rest of the
/// query and what they ask for (`Some(true)` favourites only, `Some(false)` the others).
pub fn split_fav(query: &str) -> (String, Option<bool>) {
    let mut rest = Vec::new();
    let mut want = None;
    for t in query.split_whitespace() {
        let (neg, body) = match t.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, t),
        };
        let lower = body.to_lowercase();
        let arg = lower
            .strip_prefix("fav:")
            .or_else(|| lower.strip_prefix("favourite:"))
            .or_else(|| lower.strip_prefix("favorite:"));
        match arg {
            Some("true" | "yes" | "1" | "") => want = Some(!neg),
            Some("false" | "no" | "0") => want = Some(neg),
            _ => rest.push(t),
        }
    }
    (rest.join(" "), want)
}

/// How many books the *Recent* view lists.
pub const RECENT_LIMIT: usize = 30;

/// The top entries of the sidebar. The shelves, saved searches and browse groups below them are
/// searches (see `shelves::reference_for`, `saved_searches::reference_for`) and not views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Home,
    Recent,
    Favourites,
    Notes,
    /// Online (OPDS) catalogues: a page in the main view.
    Catalogues,
}

impl View {
    pub const ALL: [View; 5] = [
        View::Home,
        View::Recent,
        View::Favourites,
        View::Notes,
        View::Catalogues,
    ];

    /// The text kept in the settings.
    pub fn key(self) -> &'static str {
        match self {
            View::Home => "home",
            View::Recent => "recent",
            View::Favourites => "favourites",
            View::Notes => "notes",
            View::Catalogues => "catalogues",
        }
    }

    pub fn from_key(key: &str) -> View {
        View::ALL
            .into_iter()
            .find(|v| v.key() == key)
            .unwrap_or_default()
    }

    /// Whether the content area shows books (Notes & Highlights and Online Catalogues are pages
    /// of their own).
    pub fn shows_books(self) -> bool {
        !matches!(self, View::Notes | View::Catalogues)
    }
}

/// One entry of the library sidebar: a view or page (Home, Recent, Favourites, Notes & Highlights,
/// Online Catalogues), a group (Shelves, Audiobooks, Saved Searches, one browse group per
/// category) or Library Health, which is pinned at the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SidebarItem {
    Home,
    Recent,
    Favourites,
    Notes,
    Catalogues,
    Shelves,
    Audiobooks,
    SavedSearches,
    Authors,
    Series,
    Tags,
    Publishers,
    Languages,
    Ratings,
    Status,
    Health,
}

impl SidebarItem {
    /// The items in their default order (Health last: it is not movable).
    pub const ALL: [SidebarItem; 16] = [
        SidebarItem::Home,
        SidebarItem::Recent,
        SidebarItem::Favourites,
        SidebarItem::Notes,
        SidebarItem::Catalogues,
        SidebarItem::Shelves,
        SidebarItem::Audiobooks,
        SidebarItem::SavedSearches,
        SidebarItem::Authors,
        SidebarItem::Series,
        SidebarItem::Tags,
        SidebarItem::Publishers,
        SidebarItem::Languages,
        SidebarItem::Ratings,
        SidebarItem::Status,
        SidebarItem::Health,
    ];

    /// The text kept in the settings.
    pub fn key(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Recent => "recent",
            Self::Favourites => "favourites",
            Self::Notes => "notes",
            Self::Catalogues => "catalogues",
            Self::Shelves => "shelves",
            Self::Audiobooks => "audiobooks",
            Self::SavedSearches => "saved-searches",
            Self::Authors => "authors",
            Self::Series => "series",
            Self::Tags => "tags",
            Self::Publishers => "publishers",
            Self::Languages => "languages",
            Self::Ratings => "ratings",
            Self::Status => "status",
            Self::Health => "health",
        }
    }

    pub fn from_key(key: &str) -> Option<SidebarItem> {
        Self::ALL.into_iter().find(|i| i.key() == key)
    }

    /// The view this item opens (the rows with a selection mark), when it is one.
    pub fn view(self) -> Option<View> {
        match self {
            Self::Home => Some(View::Home),
            Self::Recent => Some(View::Recent),
            Self::Favourites => Some(View::Favourites),
            Self::Notes => Some(View::Notes),
            Self::Catalogues => Some(View::Catalogues),
            _ => None,
        }
    }

    pub fn of_view(view: View) -> SidebarItem {
        match view {
            View::Home => Self::Home,
            View::Recent => Self::Recent,
            View::Favourites => Self::Favourites,
            View::Notes => Self::Notes,
            View::Catalogues => Self::Catalogues,
        }
    }

    /// The browse group of a category.
    pub fn of_category(cat: crate::core::library::Category) -> SidebarItem {
        use crate::core::library::Category as C;
        match cat {
            C::Authors => Self::Authors,
            C::Series => Self::Series,
            C::Tags => Self::Tags,
            C::Publishers => Self::Publishers,
            C::Languages => Self::Languages,
            C::Ratings => Self::Ratings,
            C::Status => Self::Status,
        }
    }

    /// Home is the way back to the books, so it always stays.
    pub fn can_hide(self) -> bool {
        self != Self::Home
    }

    /// Library Health sits at the bottom of the sidebar whatever the order is.
    pub fn can_move(self) -> bool {
        self != Self::Health
    }

    /// The name in the sidebar and in the *Customize Sidebar* dialog (English; the caller
    /// translates it).
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => N_("Home"),
            Self::Recent => N_("Recent"),
            Self::Favourites => N_("Favourites"),
            Self::Notes => N_("Notes & Highlights"),
            Self::Catalogues => N_("Online Catalogues"),
            Self::Shelves => N_("Shelves"),
            Self::Audiobooks => N_("Audiobooks"),
            Self::SavedSearches => N_("Saved Searches"),
            Self::Authors => N_("Authors"),
            Self::Series => N_("Series"),
            Self::Tags => N_("Tags"),
            Self::Publishers => N_("Publishers"),
            Self::Languages => N_("Languages"),
            Self::Ratings => N_("Ratings"),
            Self::Status => N_("Reading status"),
            Self::Health => N_("Library Health"),
        }
    }
}

/// Which sidebar items a library shows and in what order; kept per library in the settings.
/// The default (empty) layout is the order of [`SidebarItem::ALL`] with everything shown.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SidebarLayout {
    /// Item keys in the order the user chose; items missing here (a newer version added them)
    /// follow their default neighbour.
    pub order: Vec<String>,
    /// Item keys the user hid.
    pub hidden: Vec<String>,
}

impl SidebarLayout {
    /// Every movable item once, in the user's order.
    pub fn ordered(&self) -> Vec<SidebarItem> {
        let mut out: Vec<SidebarItem> = Vec::new();
        for k in &self.order {
            if let Some(i) = SidebarItem::from_key(k) {
                if i.can_move() && !out.contains(&i) {
                    out.push(i);
                }
            }
        }
        // an item the saved order does not know goes right after the item before it by default
        for (n, i) in SidebarItem::ALL.iter().enumerate() {
            if !i.can_move() || out.contains(i) {
                continue;
            }
            let after = SidebarItem::ALL[..n]
                .iter()
                .rev()
                .find_map(|p| out.iter().position(|o| o == p));
            out.insert(after.map_or(0, |p| p + 1), *i);
        }
        out
    }

    pub fn is_hidden(&self, item: SidebarItem) -> bool {
        item.can_hide() && self.hidden.iter().any(|k| k == item.key())
    }

    /// The items to show, in order; Library Health (pinned at the bottom) is not among them.
    pub fn visible(&self) -> Vec<SidebarItem> {
        self.ordered()
            .into_iter()
            .filter(|i| !self.is_hidden(*i))
            .collect()
    }

    pub fn show_health(&self) -> bool {
        !self.is_hidden(SidebarItem::Health)
    }

    pub fn set_hidden(&mut self, item: SidebarItem, hide: bool) {
        self.hidden.retain(|k| k != item.key());
        if hide && item.can_hide() {
            self.hidden.push(item.key().to_string());
        }
        self.tidy();
    }

    /// Moves an item up (`-1`) or down (`1`) among all the items, hidden ones included, so the
    /// place a hidden item comes back to is the one it was left in.
    pub fn move_item(&mut self, item: SidebarItem, delta: i32) -> bool {
        if !item.can_move() {
            return false;
        }
        let mut all = self.ordered();
        let Some(at) = all.iter().position(|i| *i == item) else {
            return false;
        };
        let to = at as i32 + delta.signum();
        if to < 0 || to as usize >= all.len() {
            return false;
        }
        all.swap(at, to as usize);
        self.order = all.iter().map(|i| i.key().to_string()).collect();
        self.tidy();
        true
    }

    pub fn reset(&mut self) {
        *self = SidebarLayout::default();
    }

    /// Whether this is the default layout (nothing worth saving).
    pub fn is_default(&self) -> bool {
        self.hidden.is_empty() && self.is_default_order()
    }

    /// Keeps the saved form small: nothing is stored for the default.
    fn tidy(&mut self) {
        self.hidden.sort();
        self.hidden.dedup();
        if self.is_default_order() {
            self.order.clear();
        }
    }

    fn is_default_order(&self) -> bool {
        let default: Vec<SidebarItem> = SidebarItem::ALL
            .into_iter()
            .filter(|i| i.can_move())
            .collect();
        self.ordered() == default
    }
}

/// The view to show when the one on screen was hidden from the sidebar.
pub fn view_after_hiding(current: View, layout: &SidebarLayout) -> View {
    if layout.is_hidden(SidebarItem::of_view(current)) {
        View::Home
    } else {
        current
    }
}

/// A book for the Recent view: when it was last opened and when it was added (Unix seconds).
#[derive(Debug, Clone, Copy)]
pub struct Stamp {
    pub id: u64,
    pub last_read: Option<i64>,
    pub added: i64,
}

/// The books opened or added most recently, newest first.
pub fn recent(stamps: &[Stamp], limit: usize) -> Vec<u64> {
    let mut v: Vec<(i64, u64)> = stamps
        .iter()
        .map(|s| (s.last_read.unwrap_or(i64::MIN).max(s.added), s.id))
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(limit).map(|(_, id)| id).collect()
}

/// Narrows the books a search found to the ones of a view. `recent_ids` is [`recent`]'s answer.
/// The Recent view lists them newest first, whatever the sort order, when nothing was searched.
pub fn apply_view(
    view: View,
    ids: Vec<u64>,
    favourites: &[u64],
    recent_ids: &[u64],
    searching: bool,
) -> Vec<u64> {
    match view {
        View::Home | View::Notes | View::Catalogues => ids,
        View::Favourites => {
            let fav: HashSet<u64> = favourites.iter().copied().collect();
            ids.into_iter().filter(|i| fav.contains(i)).collect()
        }
        View::Recent => {
            let want: HashSet<u64> = ids.iter().copied().collect();
            if searching {
                let r: HashSet<u64> = recent_ids.iter().copied().collect();
                ids.into_iter().filter(|i| r.contains(i)).collect()
            } else {
                recent_ids
                    .iter()
                    .copied()
                    .filter(|i| want.contains(i))
                    .collect()
            }
        }
    }
}

/// Stars or unstars these books: when all of them are favourites they are all removed, otherwise
/// all are added. Returns whether they are favourites afterwards.
pub fn toggle_favourites(favs: &mut Vec<u64>, ids: &[u64]) -> bool {
    if ids.is_empty() {
        return false;
    }
    let all = ids.iter().all(|i| favs.contains(i));
    if all {
        favs.retain(|f| !ids.contains(f));
        false
    } else {
        for id in ids {
            if !favs.contains(id) {
                favs.push(*id);
            }
        }
        true
    }
}

/// Drops favourites of books that are gone.
pub fn prune_favourites(favs: &mut Vec<u64>, existing: &HashSet<u64>) -> bool {
    let before = favs.len();
    favs.retain(|f| existing.contains(f));
    favs.len() != before
}

/// What the right-hand panel shows for a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Details {
    /// Nothing is selected: the panel is closed.
    Hidden,
    One,
    /// Several books: a summary with the bulk actions.
    Many(usize),
}

pub fn details_for(selected: usize) -> Details {
    match selected {
        0 => Details::Hidden,
        1 => Details::One,
        n => Details::Many(n),
    }
}

/// The selection after a right-click on `clicked`: a book outside the selection becomes the only
/// selected one, one inside keeps the whole selection (as in Files). The flag tells whether the
/// selection changed.
pub fn context_selection(selected: &[u64], clicked: u64) -> (Vec<u64>, bool) {
    if selected.contains(&clicked) {
        (selected.to_vec(), false)
    } else {
        (vec![clicked], true)
    }
}

/// The selection after Ctrl+click: the book is added or taken out.
pub fn ctrl_click(selected: &[u64], clicked: u64) -> Vec<u64> {
    if selected.contains(&clicked) {
        selected.iter().copied().filter(|i| *i != clicked).collect()
    } else {
        let mut v = selected.to_vec();
        v.push(clicked);
        v
    }
}

/// The selection after Shift+click: the books between the anchor and the clicked one, in the order
/// shown. Without an anchor only the clicked book.
pub fn shift_click(shown: &[u64], anchor: Option<u64>, clicked: u64) -> Vec<u64> {
    let Some(ai) = anchor.and_then(|a| shown.iter().position(|i| *i == a)) else {
        return vec![clicked];
    };
    let Some(ci) = shown.iter().position(|i| *i == clicked) else {
        return vec![clicked];
    };
    let (lo, hi) = (ai.min(ci), ai.max(ci));
    shown[lo..=hi].to_vec()
}

/// How a click on a book was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Plain,
    Ctrl,
    Shift,
}

/// The selection after a click on `clicked` (`shown` is the order on screen).
pub fn pick(
    selected: &[u64],
    shown: &[u64],
    anchor: Option<u64>,
    clicked: u64,
    how: Pick,
) -> Vec<u64> {
    match how {
        Pick::Plain => vec![clicked],
        Pick::Ctrl => ctrl_click(selected, clicked),
        Pick::Shift => shift_click(shown, anchor, clicked),
    }
}

/// A click that hits no book: clears the selection (nothing happens when there is none).
pub fn empty_click(selected: &[u64]) -> Vec<u64> {
    let _ = selected;
    Vec::new()
}

/// Takes the trailing ellipsis off a label (menu entries, buttons, dialog titles, progress texts
/// such as "Loading"): the interface has none.
pub fn plain_label(label: &str) -> String {
    let t = label.trim_end();
    let Some(body) = t.strip_suffix('…').or_else(|| t.strip_suffix("...")) else {
        return label.to_string();
    };
    body.trim_end().to_string()
}

/// Header capitalization (the rule for buttons, menu items, tooltips and dialog headings): every
/// word starts with a capital letter, except the small words in the middle ("to", "of", "as").
/// Words with digits, `{}` placeholders, access keys and `Ctrl+O`-style hints are left alone.
#[cfg(test)]
pub fn is_header_caps(label: &str) -> bool {
    const SMALL: [&str; 17] = [
        "a", "an", "the", "of", "to", "for", "in", "on", "and", "or", "as", "from", "with", "by",
        "at", "per", "into",
    ];
    let words: Vec<&str> = label.split_whitespace().collect();
    words.iter().enumerate().all(|(i, w)| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric());
        let Some(first) = w.chars().next() else {
            return true;
        };
        if !first.is_alphabetic() || w.chars().any(|c| c.is_ascii_digit()) {
            return true;
        }
        if i > 0 && SMALL.contains(&w) {
            return true;
        }
        first.is_uppercase()
    })
}

/// What the content header shows: with at least one book selected the title becomes "N selected"
/// and a Cancel button appears at the start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderState {
    pub selecting: bool,
    pub selected: usize,
}

pub fn header_state(selected: usize) -> HeaderState {
    HeaderState {
        selecting: selected >= 1,
        selected,
    }
}

/// The actions of the selection bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    Read,
    EditMetadata,
    AddToShelf,
    SendToDevice,
    Convert,
    Remove,
}

/// The actions the bar offers for this many selected books: Read only for one book, the bulk
/// actions only for several.
pub fn bulk_actions(count: usize) -> Vec<BulkAction> {
    use BulkAction::*;
    match count {
        0 => Vec::new(),
        1 => vec![Read, EditMetadata, Remove],
        _ => vec![EditMetadata, AddToShelf, SendToDevice, Convert, Remove],
    }
}

/// `(inline, overflow)`: in a narrow window only Read and Remove stay as buttons and everything else
/// moves into the "More" menu, so no action is ever unreachable.
pub fn split_bulk(count: usize, narrow: bool) -> (Vec<BulkAction>, Vec<BulkAction>) {
    let all = bulk_actions(count);
    if !narrow {
        return (all, Vec::new());
    }
    all.into_iter()
        .partition(|a| matches!(a, BulkAction::Read | BulkAction::Remove))
}

/// Gives every label of one menu its own access key: the first letter (word initials first) not yet
/// taken by an earlier label gets the underscore of GTK mnemonics; a label that already has an
/// underscore keeps it written as `__`. Labels with no free letter stay as they are.
pub fn access_keys(labels: &[String]) -> Vec<String> {
    let mut taken: Vec<char> = Vec::new();
    labels
        .iter()
        .map(|l| {
            let chars: Vec<char> = l.chars().collect();
            let initials = chars.iter().enumerate().filter(|(i, c)| {
                c.is_alphanumeric() && (*i == 0 || !chars[*i - 1].is_alphanumeric())
            });
            let others = chars
                .iter()
                .enumerate()
                .filter(|(_, c)| c.is_alphanumeric());
            let pick = initials
                .chain(others)
                .find(|(_, c)| !taken.contains(&c.to_lowercase().next().unwrap_or(**c)));
            let escaped = |from: usize, to: usize| -> String {
                chars[from..to]
                    .iter()
                    .map(|c| {
                        if *c == '_' {
                            "__".to_string()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            };
            match pick {
                Some((at, c)) => {
                    taken.push(c.to_lowercase().next().unwrap_or(*c));
                    format!("{}_{}", escaped(0, at), escaped(at, chars.len()))
                }
                None => escaped(0, chars.len()),
            }
        })
        .collect()
}

/// The Trash folders to bring back for these books: for each book the newest entry. `entries` are
/// `(folder name, book id, time of deletion)`.
pub fn newest_trash_names(entries: &[(String, u64, i64)], ids: &[u64]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| {
            entries
                .iter()
                .filter(|(_, b, _)| b == id)
                .max_by_key(|(_, _, at)| *at)
                .map(|(n, _, _)| n.clone())
        })
        .collect()
}

/// The toast after Move to Trash (it has an Undo button).
pub fn moved_to_trash_text(n: usize) -> String {
    crate::i18n::ntr("{} book moved to Trash", "{} books moved to Trash", n)
}

/// The toast after Undo.
pub fn restored_text(n: usize) -> String {
    crate::i18n::ntr("{} book restored", "{} books restored", n)
}

/// The table's columns (their titles in English, as the table names them), the room each needs in
/// pixels and whether the width may hide it. In the order of the table.
pub const TABLE_COLUMNS: [(&str, i32, bool); 11] = [
    ("★", 30, false),
    ("Title", 280, false),
    ("Editions", 50, true),
    ("Author(s)", 160, false),
    ("Series", 140, true),
    ("Rating", 80, true),
    ("Read", 56, true),
    ("Published", 100, true),
    ("Added", 100, true),
    ("Size", 80, true),
    ("Tags", 150, true),
];

/// What each column costs on top of its own width (header arrow, separator).
pub const COLUMN_PAD: i32 = 14;

/// Which optional columns win the room first, the first one is the last to go.
const COLUMN_PRIORITY: [&str; 8] = [
    "Read",
    "Added",
    "Size",
    "Series",
    "Tags",
    "Rating",
    "Published",
    "Editions",
];

/// The columns that fit `px` pixels of table: the always-visible ones, then the others in priority
/// order while they fit. The result is in table order.
pub fn table_columns_for_width(px: i32) -> Vec<&'static str> {
    let mut used: i32 = TABLE_COLUMNS
        .iter()
        .filter(|(_, _, optional)| !optional)
        .map(|(_, w, _)| *w + COLUMN_PAD)
        .sum();
    let mut keep: Vec<&str> = Vec::new();
    for name in COLUMN_PRIORITY {
        let w = TABLE_COLUMNS
            .iter()
            .find(|(n, _, _)| *n == name)
            .map_or(0, |(_, w, _)| *w);
        if used + w + COLUMN_PAD > px {
            break;
        }
        used += w + COLUMN_PAD;
        keep.push(name);
    }
    TABLE_COLUMNS
        .iter()
        .filter(|(n, _, optional)| !optional || keep.contains(n))
        .map(|(n, _, _)| *n)
        .collect()
}

/// The columns shown: the ones the user picked (always with the required ones), else those that fit.
pub fn effective_columns(chosen: &[String], px: i32) -> Vec<&'static str> {
    if chosen.is_empty() {
        return table_columns_for_width(px);
    }
    TABLE_COLUMNS
        .iter()
        .filter(|(n, _, optional)| !optional || chosen.iter().any(|c| c == n))
        .map(|(n, _, _)| *n)
        .collect()
}

/// The saved choice after switching one column on or off (a required column cannot go).
pub fn toggle_column(shown: &[&str], name: &str, on: bool) -> Vec<String> {
    let optional = TABLE_COLUMNS
        .iter()
        .any(|(n, _, optional)| *n == name && *optional);
    TABLE_COLUMNS
        .iter()
        .filter(|(n, _, o)| {
            *o && if *n == name {
                on && optional
            } else {
                shown.contains(n)
            }
        })
        .map(|(n, _, _)| n.to_string())
        .collect()
}

/// The table column that shows a sort key (`None`: the key has no column, such as Last read).
pub fn column_for_sort(key: SortKey) -> Option<&'static str> {
    Some(match key {
        SortKey::Title => "Title",
        SortKey::Author => "Author(s)",
        SortKey::DateAdded => "Added",
        SortKey::LastRead => return None,
        SortKey::Published => "Published",
        SortKey::Rating => "Rating",
        SortKey::Series => "Series",
        SortKey::Size => "Size",
        SortKey::Progress => "Read",
    })
}

/// The sort key a click on this column header means.
pub fn sort_for_column(name: &str) -> Option<SortKey> {
    SortKey::ALL
        .into_iter()
        .find(|k| column_for_sort(*k) == Some(name))
}

/// How many entries of a sidebar group show before "Show All".
pub const SIDEBAR_TOP: usize = 8;

/// The most used `n` entries of a sidebar group (by count, then name), still in the order they came
/// in, and how many are left out. A group that has at most `n + 1` entries shows all of them: one
/// hidden row is no saving.
pub fn top_n(items: Vec<(String, usize)>, n: usize) -> (Vec<(String, usize)>, usize) {
    if items.len() <= n + 1 {
        return (items, 0);
    }
    let mut ranked: Vec<usize> = (0..items.len()).collect();
    ranked.sort_by(|a, b| {
        items[*b]
            .1
            .cmp(&items[*a].1)
            .then_with(|| items[*a].0.cmp(&items[*b].0))
    });
    let mut keep: Vec<usize> = ranked.into_iter().take(n).collect();
    keep.sort_unstable();
    let hidden = items.len() - keep.len();
    let kept = keep.into_iter().map(|i| items[i].clone()).collect();
    (kept, hidden)
}

/// The name a saved search gets when the user does not type one: its first words.
pub fn default_search_name(query: &str) -> String {
    let q = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name: String = q.chars().take(30).collect();
    if q.chars().count() > 30 {
        name.push('…');
    }
    name
}

/// The key of a sidebar group in `Config::sidebar_open`.
pub fn open_key(library: &str, group: &str) -> String {
    format!("{library}|{group}")
}

/// Whether a group was left open in this library.
pub fn is_open(open: &[String], library: &str, group: &str) -> bool {
    open.contains(&open_key(library, group))
}

/// The saved list after a group was opened or closed.
pub fn set_open(open: &mut Vec<String>, library: &str, group: &str, on: bool) {
    let k = open_key(library, group);
    open.retain(|o| *o != k);
    if on {
        open.push(k);
    }
}

/// Past this width the cover grid stops growing (one row would hold too many covers).
pub const GRID_MAX_WIDTH: i32 = 1800;

/// The margin on each side of the grid that keeps it at most `cap` wide and centred.
pub fn grid_side_margin(width: i32, cap: i32) -> i32 {
    ((width - cap).max(0)) / 2
}

/// The description a card gives to assistive technology: author, progress, favourite and shelves.
pub fn card_description(
    authors: &str,
    progress_pct: u32,
    favourite: bool,
    shelves: &[String],
) -> String {
    use crate::i18n::{tr, trf};
    let mut parts: Vec<String> = Vec::new();
    if !authors.is_empty() {
        parts.push(authors.to_string());
    }
    if progress_pct >= 100 {
        parts.push(tr("finished"));
    } else if progress_pct > 0 {
        parts.push(trf("{}% read", &[&progress_pct]));
    }
    if favourite {
        parts.push(tr("favourite"));
    }
    if !shelves.is_empty() {
        parts.push(trf("on shelves {}", &[&shelves.join(", ")]));
    }
    parts.join(", ")
}

/// The address in pasted text: one `http(s)` link and nothing else (pasting a sentence does nothing).
pub fn pasted_url(text: &str) -> Option<String> {
    let t = text.trim();
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))?;
    if rest.is_empty() || t.chars().any(char::is_whitespace) || !rest.contains('.') {
        return None;
    }
    Some(t.to_string())
}

/// The icon of the View options button follows the active view.
pub fn view_button_icon(table: bool) -> &'static str {
    if table {
        "view-list-symbolic"
    } else {
        "view-grid-symbolic"
    }
}

/// The cover size slider and the "group editions" check box only act on the cover grid.
pub fn grid_only_options_visible(table: bool) -> bool {
    !table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_margin_centres_a_capped_grid() {
        assert_eq!(grid_side_margin(1000, GRID_MAX_WIDTH), 0);
        assert_eq!(grid_side_margin(GRID_MAX_WIDTH, GRID_MAX_WIDTH), 0);
        assert_eq!(grid_side_margin(2200, GRID_MAX_WIDTH), 200);
        assert_eq!(grid_side_margin(2201, GRID_MAX_WIDTH), 200);
    }

    fn st(id: u64, last: Option<i64>, added: i64) -> Stamp {
        Stamp {
            id,
            last_read: last,
            added,
        }
    }

    #[test]
    fn recent_orders_by_open_or_add_time() {
        let s = [
            st(1, None, 10),
            st(2, Some(50), 5),
            st(3, None, 30),
            st(4, Some(20), 1),
        ];
        assert_eq!(recent(&s, 10), vec![2, 3, 4, 1]);
        assert_eq!(recent(&s, 2), vec![2, 3]);
    }

    #[test]
    fn views_filter_the_found_books() {
        let ids = vec![5, 6, 7, 8];
        assert_eq!(apply_view(View::Home, ids.clone(), &[], &[], false), ids);
        assert_eq!(
            apply_view(View::Favourites, ids.clone(), &[8, 6, 1], &[], false),
            vec![6, 8]
        );
        // newest first without a search, the search's own order with one
        assert_eq!(
            apply_view(View::Recent, ids.clone(), &[], &[8, 5, 99], false),
            vec![8, 5]
        );
        assert_eq!(
            apply_view(View::Recent, ids, &[], &[8, 5, 99], true),
            vec![5, 8]
        );
    }

    #[test]
    fn the_hero_comes_on_a_cold_start_and_after_a_long_absence() {
        let mut s = HeroSession::default();
        // cold start
        assert_eq!(banner_kind(&s, 1000), BannerKind::Hero);
        hero_shown(&mut s, 1000);
        // while the visit goes on, more redraws keep the hero
        assert_eq!(banner_kind(&s, 1060), BannerKind::Hero);
        // the user opens a book and comes back a little later: the strip
        left_home(&mut s, 1100);
        assert_eq!(banner_kind(&s, 1100 + 5 * 60), BannerKind::Strip);
        assert_eq!(banner_kind(&s, 1100 + 29 * 60), BannerKind::Strip);
        // away for half an hour: the hero again
        assert_eq!(banner_kind(&s, 1100 + 30 * 60), BannerKind::Hero);
        // leaving without a hero visit changes nothing
        let mut t = HeroSession::default();
        left_home(&mut t, 5);
        assert_eq!(t, HeroSession::default());
    }

    #[test]
    fn the_banner_layout_follows_the_room_without_squares() {
        // the strip is one row whatever the room, 88 px with its margins, a quarter of the viewport at most
        for vh in [352.0, 500.0, 900.0, 1400.0] {
            for vw in [300.0, 500.0, 900.0, 2000.0] {
                let p = banner_plan("", BannerKind::Strip, vh, vw).unwrap();
                assert!(!p.hero() && p.full == STRIP_FOOTPRINT && p.full <= 0.25 * vh + 1e-9);
                assert_eq!(p.variant == Variant::Mini, vw < STRIP_FROM);
            }
        }
        // the hero: wide, medium, then the strip when the width or the height cannot hold it
        assert_eq!(
            banner_plan("", BannerKind::Hero, 1000.0, 1000.0)
                .unwrap()
                .variant,
            Variant::Wide
        );
        assert_eq!(
            banner_plan("", BannerKind::Hero, 1000.0, 700.0)
                .unwrap()
                .variant,
            Variant::Medium
        );
        assert_eq!(
            banner_plan("", BannerKind::Hero, 1000.0, 600.0)
                .unwrap()
                .variant,
            Variant::Strip
        );
        assert_eq!(
            banner_plan("", BannerKind::Hero, 650.0, 2000.0)
                .unwrap()
                .variant,
            Variant::Strip
        );
        // never more than a quarter of a small viewport, 30-35 % of a big one (up to its cap)
        for vh in (928..=1100).step_by(12) {
            let p = banner_plan("", BannerKind::Hero, vh as f64, 1500.0).unwrap();
            let share = p.full / vh as f64;
            assert!(
                (0.25 - 1e-9..=0.325 + 1e-9).contains(&share),
                "{vh}: {share}"
            );
        }
        for vh in [1100.0, 1250.0, 1330.0] {
            let share = banner_plan("", BannerKind::Hero, vh, 2000.0).unwrap().full / vh;
            assert!((0.30..=0.35).contains(&share), "{vh}: {share}");
        }
    }

    #[test]
    fn the_hero_grows_with_the_window_and_never_shrinks_as_it_grows() {
        // the ladder of 12 widths and of 12 heights: monotonic
        let mut last = 0.0;
        for i in 0..12 {
            let vh = 860.0 + 60.0 * i as f64;
            let p = banner_plan("", BannerKind::Hero, vh, 2000.0).unwrap();
            assert!(p.full >= last - 1e-9, "height {vh}");
            last = p.full;
        }
        let order = |v: Variant| match v {
            Variant::Mini => 0,
            Variant::Strip => 1,
            Variant::Medium => 2,
            Variant::Wide => 3,
        };
        let mut last = 0;
        for i in 0..12 {
            let vw = 300.0 + 190.0 * i as f64;
            let v = order(
                banner_plan("", BannerKind::Hero, 1000.0, vw)
                    .unwrap()
                    .variant,
            );
            assert!(v >= last, "width {vw}");
            last = v;
        }
    }

    #[test]
    fn the_banner_setting_overrides_the_visit() {
        assert!(banner_plan("hidden", BannerKind::Hero, 1000.0, 1000.0).is_none());
        assert!(!banner_plan("compact", BannerKind::Hero, 1400.0, 2000.0)
            .unwrap()
            .hero());
        assert!(banner_plan("large", BannerKind::Strip, 1000.0, 1000.0)
            .unwrap()
            .hero());
        assert_eq!(
            banner_plan("whatever", BannerKind::Strip, 900.0, 900.0),
            banner_plan("", BannerKind::Strip, 900.0, 900.0)
        );
    }

    #[test]
    fn the_time_left_is_a_rough_guess_in_minutes_and_hours() {
        assert_eq!(time_left_text(0, 0.0), "");
        assert_eq!(time_left_text(1_000_000, 1.0), "");
        let t = time_left_text(300_000, 0.5);
        assert!(t.starts_with("about ") && t.ends_with(" min left"), "{t}");
        assert!(time_left_text(5_000_000, 0.1).contains(" h "));
    }

    #[test]
    fn the_banner_shrinks_before_it_slides_off() {
        let p = BannerPlan {
            variant: Variant::Wide,
            full: 400.0,
            slim: 100.0,
        };
        assert_eq!(banner_geometry(p, 0.0), (400.0, 400.0, 0.0));
        assert_eq!(banner_geometry(p, 150.0), (250.0, 250.0, 0.0));
        assert_eq!(banner_geometry(p, 300.0), (100.0, 100.0, 0.0));
        assert_eq!(banner_geometry(p, 350.0), (100.0, 50.0, 50.0));
        assert_eq!(banner_geometry(p, 400.0), (100.0, 0.0, 100.0));
        assert_eq!(banner_geometry(p, 999.0), (100.0, 0.0, 100.0));
        // a slim banner only slides off
        let q = BannerPlan {
            variant: Variant::Strip,
            full: 100.0,
            slim: 100.0,
        };
        assert_eq!(banner_geometry(q, 40.0), (100.0, 60.0, 40.0));
        // the visible height never jumps by more than the scrolled amount
        let mut last = banner_geometry(p, 0.0).1;
        for off in 1..=400 {
            let v = banner_geometry(p, off as f64).1;
            assert!(last - v <= 1.0 + 1e-9 && v <= last, "{off}: {last} -> {v}");
            last = v;
        }
    }

    #[test]
    fn the_wheel_moves_the_banner_before_the_books() {
        assert_eq!(banner_scroll(0.0, 100.0, true, 40.0), (40.0, true));
        assert_eq!(banner_scroll(80.0, 100.0, true, 40.0), (100.0, true));
        assert_eq!(banner_scroll(100.0, 100.0, true, 40.0), (100.0, false));
        assert_eq!(banner_scroll(100.0, 100.0, false, -40.0), (100.0, false));
        assert_eq!(banner_scroll(100.0, 100.0, true, -40.0), (60.0, true));
        assert_eq!(banner_scroll(20.0, 100.0, true, -40.0), (0.0, true));
        assert_eq!(banner_scroll(0.0, 100.0, true, -40.0), (0.0, false));
        assert_eq!(banner_scroll(0.0, 0.0, true, 40.0), (0.0, false));
    }

    #[test]
    fn fav_terms_are_split_from_the_search() {
        assert_eq!(split_fav("dune fav:true"), ("dune".into(), Some(true)));
        assert_eq!(
            split_fav("fav:false author:x"),
            ("author:x".into(), Some(false))
        );
        assert_eq!(split_fav("-fav:true"), (String::new(), Some(false)));
        assert_eq!(
            split_fav("favorite:yes tag:a"),
            ("tag:a".into(), Some(true))
        );
        assert_eq!(split_fav("fav:maybe"), ("fav:maybe".into(), None));
        assert_eq!(split_fav("tag:fav"), ("tag:fav".into(), None));
    }

    #[test]
    fn keys_round_trip() {
        for v in View::ALL {
            assert_eq!(View::from_key(v.key()), v);
        }
        assert_eq!(View::from_key("???"), View::Home);
        assert!(!View::Notes.shows_books() && View::Recent.shows_books());
    }

    #[test]
    fn the_default_sidebar_is_today_s_order_plus_catalogues() {
        let l = SidebarLayout::default();
        let keys: Vec<&str> = l.visible().iter().map(|i| i.key()).collect();
        assert_eq!(
            keys,
            [
                "home",
                "recent",
                "favourites",
                "notes",
                "catalogues",
                "shelves",
                "audiobooks",
                "saved-searches",
                "authors",
                "series",
                "tags",
                "publishers",
                "languages",
                "ratings",
                "status"
            ]
        );
        assert!(l.show_health() && l.is_default());
        for i in SidebarItem::ALL {
            assert_eq!(SidebarItem::from_key(i.key()), Some(i));
        }
        for v in View::ALL {
            assert_eq!(SidebarItem::of_view(v).view(), Some(v));
        }
    }

    #[test]
    fn items_hide_and_come_back_in_the_same_place() {
        let mut l = SidebarLayout::default();
        l.set_hidden(SidebarItem::Notes, true);
        l.set_hidden(SidebarItem::Health, true);
        assert!(l.is_hidden(SidebarItem::Notes) && !l.show_health());
        assert!(!l.visible().contains(&SidebarItem::Notes));
        assert!(!l.is_default());
        // Home is the way back and never hides
        l.set_hidden(SidebarItem::Home, true);
        assert!(!l.is_hidden(SidebarItem::Home));
        l.set_hidden(SidebarItem::Notes, false);
        l.set_hidden(SidebarItem::Health, false);
        assert!(l.is_default());
        assert_eq!(
            l,
            SidebarLayout::default(),
            "nothing is kept for the default"
        );
    }

    #[test]
    fn items_move_up_and_down_and_stop_at_the_ends() {
        let mut l = SidebarLayout::default();
        assert!(l.move_item(SidebarItem::Catalogues, -1));
        let v = l.visible();
        assert_eq!(v[3], SidebarItem::Catalogues);
        assert_eq!(v[4], SidebarItem::Notes);
        assert!(
            !l.move_item(SidebarItem::Home, -1),
            "the first item cannot go up"
        );
        let last = *l.visible().last().unwrap();
        assert!(!l.move_item(last, 1), "the last item cannot go down");
        assert!(!l.move_item(SidebarItem::Health, -1), "Health is pinned");
        // moving back restores the default
        assert!(l.move_item(SidebarItem::Catalogues, 1));
        assert!(l.is_default() && l.order.is_empty());
        // a hidden item moves with its place
        l.set_hidden(SidebarItem::Shelves, true);
        assert!(l.move_item(SidebarItem::Shelves, 1));
        assert!(l.is_hidden(SidebarItem::Shelves));
        l.set_hidden(SidebarItem::Shelves, false);
        let k: Vec<_> = l.visible().iter().map(|i| i.key()).collect();
        assert_eq!(&k[5..7], ["audiobooks", "shelves"]);
    }

    #[test]
    fn an_old_or_odd_saved_layout_is_repaired() {
        let l = SidebarLayout {
            order: vec![
                "tags".into(),
                "nonsense".into(),
                "home".into(),
                "tags".into(),
                "health".into(),
            ],
            hidden: vec!["authors".into(), "nonsense".into()],
        };
        let v = l.ordered();
        // every movable item exactly once, the saved ones first
        assert_eq!(v.len(), SidebarItem::ALL.len() - 1);
        assert_eq!(v[0], SidebarItem::Tags);
        assert!(
            v.iter().position(|i| *i == SidebarItem::Tags)
                < v.iter().position(|i| *i == SidebarItem::Home)
        );
        assert!(!v.contains(&SidebarItem::Health));
        // an item the saved order lacks comes after its default predecessor
        assert!(
            v.iter().position(|i| *i == SidebarItem::Recent).unwrap()
                == v.iter().position(|i| *i == SidebarItem::Home).unwrap() + 1
        );
        assert!(l.is_hidden(SidebarItem::Authors));
        let json = serde_json::to_string(&l).unwrap();
        assert_eq!(serde_json::from_str::<SidebarLayout>(&json).unwrap(), l);
        assert_eq!(
            serde_json::from_str::<SidebarLayout>("{}").unwrap(),
            SidebarLayout::default()
        );
    }

    #[test]
    fn a_hidden_view_falls_back_to_home() {
        let mut l = SidebarLayout::default();
        assert_eq!(view_after_hiding(View::Notes, &l), View::Notes);
        l.set_hidden(SidebarItem::Notes, true);
        assert_eq!(view_after_hiding(View::Notes, &l), View::Home);
        assert_eq!(view_after_hiding(View::Recent, &l), View::Recent);
    }

    #[test]
    fn catalogues_are_a_page_not_books() {
        assert!(!View::Catalogues.shows_books() && !View::Notes.shows_books());
        assert_eq!(View::from_key("catalogues"), View::Catalogues);
        assert_eq!(
            apply_view(View::Catalogues, vec![1, 2], &[], &[], false),
            vec![1, 2]
        );
    }

    #[test]
    fn favourites_toggle_as_a_group() {
        let mut f = vec![1];
        assert!(toggle_favourites(&mut f, &[1, 2]));
        assert_eq!(f, vec![1, 2]);
        assert!(!toggle_favourites(&mut f, &[1, 2]));
        assert!(f.is_empty());
        assert!(!toggle_favourites(&mut f, &[]));
        let mut f = vec![1, 2, 3];
        let keep: HashSet<u64> = [2, 3].into();
        assert!(prune_favourites(&mut f, &keep));
        assert_eq!(f, vec![2, 3]);
        assert!(!prune_favourites(&mut f, &keep));
    }

    #[test]
    fn the_details_panel_follows_the_selection() {
        assert_eq!(details_for(0), Details::Hidden);
        assert_eq!(details_for(1), Details::One);
        assert_eq!(details_for(4), Details::Many(4));
    }

    #[test]
    fn right_click_works_like_files() {
        assert_eq!(context_selection(&[1, 2], 3), (vec![3], true));
        assert_eq!(context_selection(&[1, 2], 2), (vec![1, 2], false));
        assert_eq!(context_selection(&[], 4), (vec![4], true));
    }

    #[test]
    fn ctrl_and_shift_click() {
        assert_eq!(ctrl_click(&[1], 2), vec![1, 2]);
        assert_eq!(ctrl_click(&[1, 2], 1), vec![2]);
        let shown = [10, 11, 12, 13, 14];
        assert_eq!(shift_click(&shown, Some(11), 13), vec![11, 12, 13]);
        assert_eq!(shift_click(&shown, Some(13), 11), vec![11, 12, 13]);
        assert_eq!(shift_click(&shown, None, 12), vec![12]);
        assert_eq!(shift_click(&shown, Some(99), 12), vec![12]);
    }

    #[test]
    fn pick_follows_the_modifier() {
        let shown = [1, 2, 3, 4];
        assert_eq!(pick(&[2], &shown, Some(2), 4, Pick::Plain), vec![4]);
        assert_eq!(pick(&[2], &shown, Some(2), 4, Pick::Ctrl), vec![2, 4]);
        assert_eq!(pick(&[2], &shown, Some(2), 4, Pick::Shift), vec![2, 3, 4]);
    }

    #[test]
    fn a_click_on_nothing_clears() {
        assert!(empty_click(&[1, 2]).is_empty());
        assert!(empty_click(&[]).is_empty());
    }

    #[test]
    fn undo_brings_back_the_newest_trash_entry_of_each_book() {
        let e = vec![
            ("1-7".to_string(), 7, 1),
            ("9-7".to_string(), 7, 9),
            ("5-8".to_string(), 8, 5),
            ("6-9".to_string(), 9, 6),
        ];
        assert_eq!(newest_trash_names(&e, &[7, 8]), vec!["9-7", "5-8"]);
        assert!(newest_trash_names(&e, &[1]).is_empty());
        assert_eq!(moved_to_trash_text(1), "1 book moved to Trash");
        assert_eq!(moved_to_trash_text(3), "3 books moved to Trash");
        assert_eq!(restored_text(2), "2 books restored");
    }

    #[test]
    fn the_table_hides_the_low_value_columns_first_as_it_narrows() {
        let wide = table_columns_for_width(1600);
        assert_eq!(wide.len(), TABLE_COLUMNS.len());
        let at_650 = table_columns_for_width(650);
        assert!(
            at_650.contains(&"Title") && at_650.contains(&"Author(s)") && at_650.contains(&"★")
        );
        assert!(at_650.contains(&"Read"));
        for gone in ["Series", "Rating", "Published", "Tags", "Size"] {
            assert!(!at_650.contains(&gone), "{gone} fits 650 px");
        }
        // the required ones never go
        assert_eq!(
            table_columns_for_width(100),
            vec!["★", "Title", "Author(s)"]
        );
        // each step keeps what the narrower one had
        let mut last: Vec<&str> = Vec::new();
        for w in (400..1700).step_by(50) {
            let now = table_columns_for_width(w);
            assert!(
                last.iter().all(|c| now.contains(c)),
                "{w}: {now:?} lost {last:?}"
            );
            last = now;
        }
        // the width that is in the table never exceeds the room (when the required ones fit)
        for w in (520..1700).step_by(37) {
            let used: i32 = table_columns_for_width(w)
                .iter()
                .map(|n| TABLE_COLUMNS.iter().find(|c| c.0 == *n).unwrap().1 + COLUMN_PAD)
                .sum();
            assert!(used <= w, "{used} > {w}");
        }
    }

    #[test]
    fn the_columns_the_user_picks_win_over_the_width() {
        assert_eq!(effective_columns(&[], 650), table_columns_for_width(650));
        let picked = vec!["Tags".to_string()];
        let shown = effective_columns(&picked, 650);
        assert!(shown.contains(&"Tags") && !shown.contains(&"Read"));
        assert!(shown.contains(&"Title"));
        let after = toggle_column(&shown, "Series", true);
        assert_eq!(after, vec!["Series", "Tags"]);
        assert_eq!(toggle_column(&shown, "Tags", false), Vec::<String>::new());
        // Title is not optional: asking for it changes nothing
        assert_eq!(toggle_column(&shown, "Title", false), vec!["Tags"]);
    }

    #[test]
    fn a_header_click_and_the_sort_menu_name_the_same_order() {
        for k in SortKey::ALL {
            match column_for_sort(k) {
                Some(c) => assert_eq!(sort_for_column(c), Some(k), "{c}"),
                None => assert_eq!(k, SortKey::LastRead),
            }
        }
        assert_eq!(sort_for_column("Author(s)"), Some(SortKey::Author));
        assert_eq!(sort_for_column("Tags"), None);
        assert_eq!(sort_for_column("★"), None);
        // every sortable column is a real column of the table
        for k in SortKey::ALL {
            if let Some(c) = column_for_sort(k) {
                assert!(TABLE_COLUMNS.iter().any(|t| t.0 == c), "{c}");
            }
        }
    }

    #[test]
    fn a_long_sidebar_group_shows_its_top_entries_and_counts_the_rest() {
        let items: Vec<(String, usize)> = (0..30).map(|i| (format!("A{i:02}"), i)).collect();
        let (top, hidden) = top_n(items, SIDEBAR_TOP);
        assert_eq!(top.len(), 8);
        assert_eq!(hidden, 22);
        // the top entries keep the order they came in
        assert_eq!(top[0], ("A22".to_string(), 22));
        assert_eq!(top[7], ("A29".to_string(), 29));
        // equal counts go by name
        let (top, hidden) = top_n((0..20).map(|i| (format!("B{i:02}"), 1)).collect(), 3);
        assert_eq!(hidden, 17);
        assert_eq!(
            top.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(),
            ["B00", "B01", "B02"]
        );
        // nine entries with a cap of eight: one hidden row is no saving, so all show
        let (all, hidden) = top_n((0..9).map(|i| (i.to_string(), i)).collect(), 8);
        assert_eq!((all.len(), hidden), (9, 0));
        let (all, hidden) = top_n(Vec::new(), 8);
        assert_eq!((all.len(), hidden), (0, 0));
    }

    #[test]
    fn a_saved_search_without_a_name_gets_its_first_words() {
        assert_eq!(
            default_search_name("  author:herbert   tag:sf "),
            "author:herbert tag:sf"
        );
        assert_eq!(default_search_name(""), "");
        let long = "x".repeat(50);
        let n = default_search_name(&long);
        assert_eq!(n.chars().count(), 31);
        assert!(n.ends_with('…'));
    }

    #[test]
    fn only_a_single_link_counts_as_a_pasted_address() {
        assert_eq!(
            pasted_url("  https://example.com/a?b=1 \n").as_deref(),
            Some("https://example.com/a?b=1")
        );
        assert_eq!(pasted_url("http://x.org").as_deref(), Some("http://x.org"));
        for no in [
            "",
            "hello world",
            "see https://example.com now",
            "https://",
            "ftp://example.com",
            "https://localhost",
            "example.com",
        ] {
            assert_eq!(pasted_url(no), None, "{no}");
        }
    }

    #[test]
    fn opened_groups_are_remembered_per_library() {
        let mut open = Vec::new();
        set_open(&mut open, "/a", "Authors", true);
        set_open(&mut open, "/b", "Authors", true);
        assert!(is_open(&open, "/a", "Authors") && is_open(&open, "/b", "Authors"));
        set_open(&mut open, "/a", "Authors", false);
        assert!(!is_open(&open, "/a", "Authors") && is_open(&open, "/b", "Authors"));
        set_open(&mut open, "/b", "Authors", true);
        assert_eq!(open.len(), 1);
    }

    #[test]
    fn a_card_describes_author_progress_favourite_and_shelves() {
        let shelves = vec!["To read".to_string(), "Sci-fi".to_string()];
        assert_eq!(
            card_description("A. Author", 40, true, &shelves),
            "A. Author, 40% read, favourite, on shelves To read, Sci-fi"
        );
        assert_eq!(card_description("", 0, false, &[]), "");
        assert_eq!(card_description("X", 100, false, &[]), "X, finished");
    }

    #[test]
    fn header_caps_means_capitals_except_small_words() {
        for ok in [
            "Open Library Folder",
            "Add to Shelf",
            "Subscribe to News",
            "Mark as Finished",
            "Open from Start",
            "Move to Trash",
            "Send {} Books",
            "Save This Search",
            "Ctrl+O",
            "Export as Calibre Library",
        ] {
            assert!(is_header_caps(ok), "{ok}");
        }
        for bad in [
            "Open from start",
            "Mark finished",
            "Clear finished",
            "Add to shelf",
        ] {
            assert!(!is_header_caps(bad), "{bad}");
        }
    }

    /// K7: the buttons, menu entries and tooltips of the library windows follow the rule (the
    /// sentence-like ones are listed here on purpose).
    #[test]
    #[allow(clippy::nonminimal_bool)]
    fn library_buttons_and_menu_entries_use_header_caps() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
        let sentence_like = [
            "Nothing is running",
            "Select book",
            "Shelf actions",
            "Show the columns that fit the width",
            "Any time",
        ];
        let mut bad = Vec::new();
        for f in [
            "library.rs",
            "library_extra.rs",
            "library_banner.rs",
            "notes.rs",
            "note_row.rs",
            "jobs.rs",
            "shelves.rs",
        ] {
            let text = std::fs::read_to_string(dir.join(f)).unwrap();
            for call in [
                "Button::with_label(&tr(\"",
                "menu.append(Some(&tr(\"",
                "append(Some(&tr(\"",
                "append_submenu(Some(&tr(\"",
                "set_tooltip_text(Some(&tr(\"",
            ] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(call) {
                    let start = i + call.len();
                    let tail = &rest[start..];
                    let lit = &tail[..tail.find('"').unwrap_or(0)];
                    let before = rest[..i].chars().last().unwrap_or(' ');
                    let tooltip_sentence = call.starts_with("set_tooltip") && lit.contains(' ');
                    if !before.is_alphanumeric()
                        && before != '_'
                        && !lit.contains('\\')
                        && !sentence_like.contains(&lit)
                        && !(tooltip_sentence && lit.split_whitespace().count() > 5)
                        && !is_header_caps(lit)
                    {
                        bad.push(format!("{f}: {lit}"));
                    }
                    rest = &rest[start..];
                }
            }
        }
        bad.sort();
        bad.dedup();
        assert!(bad.is_empty(), "labels not in header caps: {bad:#?}");
    }

    #[test]
    fn labels_and_progress_texts_lose_their_ellipsis() {
        assert_eq!(plain_label("Add Books…"), "Add Books");
        assert_eq!(plain_label("Go to..."), "Go to");
        assert_eq!(plain_label("Preferences"), "Preferences");
        assert_eq!(plain_label("Loading…"), "Loading");
        assert_eq!(plain_label("Checking 3 of 9…"), "Checking 3 of 9");
        assert_eq!(plain_label("Opening PDF…"), "Opening PDF");
        assert_eq!(plain_label("Rename…"), "Rename");
        assert_eq!(plain_label("Setting up…"), "Setting up");
    }

    /// No label of the interface ends in "…" or "...": not the tr / trf / ntr / N_ strings (menu
    /// entries, buttons, titles, placeholders, status and progress texts) and not the literals
    /// given straight to a widget setter or a `format!` that builds a visible text.
    #[test]
    fn no_label_in_the_sources_ends_in_an_ellipsis() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut bad = Vec::new();
        for f in files {
            if f.ends_with("i18n.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap();
            for call in [
                "tr(\"",
                "trf(\"",
                "N_(\"",
                "ntr(\"",
                "set_label(\"",
                "set_title(\"",
                "set_text(\"",
                "set_subtitle(\"",
                "set_placeholder_text(Some(\"",
                "set_tooltip_text(Some(\"",
                "with_label(\"",
            ] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(call) {
                    let start = i + call.len();
                    let tail = &rest[start..];
                    let end = tail.find('"').unwrap_or(0);
                    let lit = &tail[..end];
                    let before = rest[..i].chars().last().unwrap_or(' ');
                    if !before.is_alphanumeric()
                        && before != '_'
                        && !lit.starts_with("...")
                        && plain_label(lit) != lit
                    {
                        bad.push(format!("{}: {lit}", f.display()));
                    }
                    rest = &rest[start..];
                }
            }
        }
        assert!(bad.is_empty(), "labels ending in an ellipsis: {bad:#?}");
    }

    #[test]
    fn the_panels_become_overlays_before_the_books_get_squeezed() {
        for w in [360, 700, 800, 900, 920] {
            assert!(window_layout(w).sidebar_overlay, "{w}");
        }
        // 1100: the sidebar stays and the books keep 800, but the details panel would squeeze them
        let l = window_layout(1100);
        assert!(!l.sidebar_overlay && l.details_overlay);
        let l = window_layout(1400);
        assert!(!l.sidebar_overlay && !l.details_overlay && !l.header_compact);
        // whatever is docked leaves the books enough room
        for w in 360..2000 {
            let l = window_layout(w);
            let mut room = w;
            if !l.sidebar_overlay {
                room -= SIDEBAR_MIN;
                assert!(room >= CONTENT_MIN, "{w}");
            }
            if !l.details_overlay {
                room -= DETAILS_MAX;
                assert!(room >= GRID_MIN, "{w}");
            }
        }
        assert!(window_layout(MIN_WIDTH).header_compact && !window_layout(1100).header_compact);
    }

    #[test]
    fn filter_rows_open_the_search_and_views_clear_it() {
        let s = search_for(NavTarget::Filter("author:=\"Ian Miell\""));
        assert!(s.open);
        assert_eq!(s.query, "author:=\"Ian Miell\"");
        for v in View::ALL {
            assert_eq!(
                search_for(NavTarget::View(v)),
                SearchState {
                    open: false,
                    query: String::new()
                }
            );
        }
        // an empty filter is no filter
        assert!(!search_for(NavTarget::Filter("  ")).open);
    }

    #[test]
    fn the_header_follows_the_selection() {
        assert_eq!(
            header_state(0),
            HeaderState {
                selecting: false,
                selected: 0
            }
        );
        assert!(header_state(1).selecting && header_state(2).selected == 2);
    }

    #[test]
    fn the_bar_offers_bulk_actions_for_several_books_and_read_for_one() {
        use BulkAction::*;
        assert!(bulk_actions(0).is_empty());
        assert_eq!(bulk_actions(1), vec![Read, EditMetadata, Remove]);
        let many = bulk_actions(5);
        assert!(!many.contains(&Read));
        assert_eq!(many.len(), 5);
        for a in [EditMetadata, AddToShelf, SendToDevice, Convert, Remove] {
            assert!(many.contains(&a));
        }
    }

    #[test]
    fn a_narrow_window_moves_the_extra_actions_into_more_and_loses_none() {
        for n in [1, 2, 7] {
            let (inline, more) = split_bulk(n, true);
            let mut both = inline.clone();
            both.extend(more.clone());
            let mut want = bulk_actions(n);
            both.sort_by_key(|a| *a as u8);
            want.sort_by_key(|a| *a as u8);
            assert_eq!(both, want);
            assert!(inline.contains(&BulkAction::Remove));
            assert!(!more.contains(&BulkAction::Remove));
        }
        assert!(split_bulk(3, false).1.is_empty());
    }

    #[test]
    fn access_keys_are_unique_and_prefer_word_initials() {
        let l: Vec<String> = ["Sync", "Saved Searches", "Open Library Folder", "Help"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let k = access_keys(&l);
        assert_eq!(k[0], "_Sync");
        assert_eq!(k[1], "S_aved Searches");
        assert_eq!(k[2], "_Open Library Folder");
        assert_eq!(k[3], "_Help");
        let mut keys: Vec<char> = k
            .iter()
            .filter_map(|s| s.split_once('_').and_then(|(_, r)| r.chars().next()))
            .map(|c| c.to_ascii_lowercase())
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 4);
        // an existing underscore is escaped, and a label with no free letter is left alone
        assert_eq!(access_keys(&["a_b".to_string()]), vec!["_a__b".to_string()]);
        assert_eq!(
            access_keys(&["x".to_string(), "x".to_string()]),
            vec!["_x".to_string(), "x".to_string()]
        );
    }

    #[test]
    fn the_view_button_and_options_follow_the_mode() {
        assert_eq!(view_button_icon(false), "view-grid-symbolic");
        assert_eq!(view_button_icon(true), "view-list-symbolic");
        assert!(grid_only_options_visible(false) && !grid_only_options_visible(true));
    }
}
