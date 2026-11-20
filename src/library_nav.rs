//! What the library window's sidebar and selection decide, without GTK: the views (Home, Recent,
//! Favourites, Notes & Highlights), the favourites list, the right-hand details panel following the
//! selection, and how a click or right-click changes the selection (bug 8).

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
}

impl View {
    pub const ALL: [View; 4] = [View::Home, View::Recent, View::Favourites, View::Notes];

    /// The text kept in the settings.
    pub fn key(self) -> &'static str {
        match self {
            View::Home => "home",
            View::Recent => "recent",
            View::Favourites => "favourites",
            View::Notes => "notes",
        }
    }

    pub fn from_key(key: &str) -> View {
        View::ALL
            .into_iter()
            .find(|v| v.key() == key)
            .unwrap_or_default()
    }

    /// Whether the content area shows books (the Notes view shows highlights instead).
    pub fn shows_books(self) -> bool {
        self != View::Notes
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
        View::Home | View::Notes => ids,
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

/// Takes the trailing ellipsis off a label (menu entries, buttons, dialog titles) but keeps it on
/// progress texts such as "Loading…" (the first word ends in "ing").
pub fn plain_label(label: &str) -> String {
    let t = label.trim_end();
    let Some(body) = t.strip_suffix('…').or_else(|| t.strip_suffix("...")) else {
        return label.to_string();
    };
    let first = body.split_whitespace().next().unwrap_or("");
    if first.len() > 4 && first.to_lowercase().ends_with("ing") {
        return label.to_string();
    }
    body.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn labels_lose_their_ellipsis_but_progress_keeps_it() {
        assert_eq!(plain_label("Add Books…"), "Add Books");
        assert_eq!(plain_label("Go to..."), "Go to");
        assert_eq!(plain_label("Preferences"), "Preferences");
        assert_eq!(plain_label("Loading…"), "Loading…");
        assert_eq!(plain_label("Checking 3 of 9…"), "Checking 3 of 9…");
        assert_eq!(plain_label("Opening PDF…"), "Opening PDF…");
        assert_eq!(plain_label("Rename…"), "Rename");
        assert_eq!(plain_label("Setting up…"), "Setting up…");
    }

    /// No label of the interface (tr / N_ strings: menu entries, buttons, titles) ends in "…"
    /// unless it is a progress text such as "Loading…".
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
            for call in ["tr(\"", "trf(\"", "N_(\"", "ntr(\""] {
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
}
