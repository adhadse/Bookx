//! Anchoring highlights by their **text**.
//!
//! A highlight is stored as a chapter (`spine`) and a character range in the reader's text of
//! that chapter ([`crate::doc::Doc::reader_text`]). Offsets that come from anywhere else (an
//! e-reader, another version of the book, an older Bookx) are only a *hint*: this module finds
//! where the quote really is, and says so when it is not anywhere.
//!
//! * [`locate`] searches one text: exact, then ignoring what devices change (quotes, dashes,
//!   soft hyphens, ligatures, runs of white space), then ignoring case, then fuzzy (the ends of
//!   the quote must be found and the middle must be alike). The occurrence closest to the hint wins.
//! * [`resolve`] searches the chapters, the hinted one first, then its neighbours, then the rest.
//! * [`verify`] says whether a stored range still holds the quote.
//! * [`Library::repair_highlights`] runs both over every stored highlight (one operation in the
//!   log, so undo restores all of them). A highlight nothing matches is **never painted at a
//!   guessed place**: it becomes *unanchored* ([`is_unanchored`]): kept with its text and note,
//!   listed in the notes, shown nowhere on the page, and its old offsets stay in the record.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::book::OpenBook;
use crate::history::OpKind;
use crate::library::{Highlight, Library};

// -- the unanchored state ------------------------------------------------------------------------

/// Whether the highlight has no place in the text. Such a highlight keeps its `spine` and its
/// old offsets as `start = -1 - old_start`, `end = -1 - old_end` (see [`hint_of`]).
pub fn is_unanchored(h: &Highlight) -> bool {
    h.start < 0
}

/// A highlight of a PDF holds page rectangles, not text offsets.
fn is_pdf(h: &Highlight) -> bool {
    h.style.starts_with("pdf|")
}

/// `(spine, start, end)` as the highlight had them before it lost its place, or as they are.
pub fn hint_of(h: &Highlight) -> (usize, i32, i32) {
    if is_unanchored(h) {
        (h.spine, -1 - h.start, (-1 - h.end).max(0))
    } else {
        (h.spine, h.start, h.end)
    }
}

/// Takes the highlight off the page; the old offsets stay in the record.
pub fn unanchor(h: &mut Highlight) {
    if !is_unanchored(h) {
        h.start = -1 - h.start.max(0);
        h.end = -1 - h.end.max(0);
    }
}

/// Puts the highlight at a verified place.
pub fn place(h: &mut Highlight, spine: usize, start: usize, end: usize) {
    h.spine = spine;
    h.start = start as i32;
    h.end = end as i32;
}

/// The state of a new highlight that has no place (`spine` is where it is thought to belong).
pub fn unplaced(spine: usize) -> (usize, i32, i32) {
    (spine, -1, -1)
}

// -- normalising -----------------------------------------------------------------------------------

/// Text brought to a form in which what devices and stores change does not matter, with the
/// index in the source of every character.
struct Norm {
    chars: Vec<char>,
    /// Index in the source of the character `chars[i]` came from.
    origin: Vec<usize>,
}

fn ligature(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{fb00}' => "ff",
        '\u{fb01}' => "fi",
        '\u{fb02}' => "fl",
        '\u{fb03}' => "ffi",
        '\u{fb04}' => "ffl",
        '\u{fb05}' | '\u{fb06}' => "st",
        '\u{133}' => "ij",
        '\u{153}' => "oe",
        '\u{e6}' => "ae",
        _ => return None,
    })
}

impl Norm {
    fn new(src: &[char], fold_case: bool) -> Norm {
        let mut chars = Vec::with_capacity(src.len());
        let mut origin = Vec::with_capacity(src.len());
        let mut last_space = true;
        for (i, &c) in src.iter().enumerate() {
            let mut push = |c: char| {
                chars.push(c);
                origin.push(i);
            };
            match c {
                '\u{ad}' | '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}' => {}
                '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}' | '\u{2032}' | '\u{b4}'
                | '\u{60}' => {
                    last_space = false;
                    push('\'');
                }
                '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{201f}' | '\u{2033}' | '\u{ab}'
                | '\u{bb}' => {
                    last_space = false;
                    push('"');
                }
                '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
                | '\u{2212}' => {
                    last_space = false;
                    push('-');
                }
                '\u{2026}' => {
                    last_space = false;
                    for _ in 0..3 {
                        push('.');
                    }
                }
                c if c.is_whitespace() => {
                    if !last_space {
                        push(' ');
                    }
                    last_space = true;
                }
                c => {
                    last_space = false;
                    if let Some(l) = ligature(c) {
                        for x in l.chars() {
                            push(x);
                        }
                    } else if fold_case {
                        for x in c.to_lowercase() {
                            push(x);
                        }
                    } else {
                        push(c);
                    }
                }
            }
        }
        while chars.last() == Some(&' ') {
            chars.pop();
            origin.pop();
        }
        Norm { chars, origin }
    }

    /// The range of the source that `chars[a..b]` came from.
    fn source_range(&self, a: usize, b: usize) -> (usize, usize) {
        (self.origin[a], self.origin[b - 1] + 1)
    }
}

/// Quote and passage are the same text for the purpose of anchoring.
pub fn same_text(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (a, b) = (Norm::new(&a, true), Norm::new(&b, true));
    !a.chars.is_empty() && a.chars == b.chars
}

// -- searching -------------------------------------------------------------------------------------

/// Where a quote was found: a character range of the searched text and how sure that is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub kind: Match,
}

/// How the quote matched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Match {
    Exact,
    /// Equal after normalising quotes, dashes, hyphens, ligatures and white space.
    Normalised,
    /// Equal when case is ignored too.
    CaseFolded,
    /// Alike (this similarity, 0..1): the device text differs a little.
    Fuzzy(f32),
}

fn occurrences(hay: &[char], needle: &[char]) -> Vec<usize> {
    let n = needle.len();
    if n == 0 || hay.len() < n {
        return Vec::new();
    }
    let first = needle[0];
    let mut out = Vec::new();
    for i in 0..=hay.len() - n {
        if hay[i] == first && hay[i..i + n] == *needle {
            out.push(i);
        }
    }
    out
}

fn closest(positions: &[usize], hint: Option<usize>) -> Option<usize> {
    match hint {
        Some(h) => positions
            .iter()
            .copied()
            .min_by_key(|p| (*p as i64 - h as i64).unsigned_abs()),
        None => positions.first().copied(),
    }
}

/// Shortest quote that is searched for (shorter ones match too many places).
pub const MIN_QUOTE: usize = 8;
/// Least similarity a fuzzy match must reach.
pub const FUZZY_MIN: f32 = 0.88;

/// Finds `quote` in `hay` without the fuzzy step. `hint` is a character index of `hay` near which
/// the quote is expected; of several occurrences the closest wins.
pub fn locate_exact(hay: &[char], quote: &str, hint: Option<usize>) -> Option<Found> {
    let q: Vec<char> = quote.trim().chars().collect();
    if q.len() < MIN_QUOTE {
        return None;
    }
    if let Some(p) = closest(&occurrences(hay, &q), hint) {
        return Some(Found {
            start: p,
            end: p + q.len(),
            kind: Match::Exact,
        });
    }
    for (fold, kind) in [(false, Match::Normalised), (true, Match::CaseFolded)] {
        let (h, n) = (Norm::new(hay, fold), Norm::new(&q, fold));
        if n.chars.len() < MIN_QUOTE {
            return None;
        }
        let hits = occurrences(&h.chars, &n.chars);
        let hint_n = hint.map(|x| h.origin.partition_point(|o| *o < x));
        if let Some(p) = closest(&hits, hint_n) {
            let (start, end) = h.source_range(p, p + n.chars.len());
            return Some(Found { start, end, kind });
        }
    }
    None
}

fn bigrams(c: &[char]) -> HashMap<(char, char), u32> {
    let mut m = HashMap::new();
    for w in c.windows(2) {
        *m.entry((w[0], w[1])).or_insert(0) += 1;
    }
    m
}

/// Dice coefficient of the character bigrams.
fn similarity(a: &[char], b: &[char]) -> f32 {
    if a.len() < 2 || b.len() < 2 {
        return 0.0;
    }
    let (ma, mb) = (bigrams(a), bigrams(b));
    let common: u32 = ma
        .iter()
        .map(|(k, v)| (*v).min(mb.get(k).copied().unwrap_or(0)))
        .sum();
    2.0 * common as f32 / ((a.len() - 1) + (b.len() - 1)) as f32
}

/// Finds a passage that is *like* the quote: its first and last words are found and the text
/// between them is alike (a device may leave out a word, change a hyphen or add a footnote mark).
fn locate_fuzzy(hay: &[char], quote: &str, hint: Option<usize>) -> Option<Found> {
    const END: usize = 16;
    let q: Vec<char> = quote.trim().chars().collect();
    let (h, n) = (Norm::new(hay, true), Norm::new(&q, true));
    let len = n.chars.len();
    if len < 2 * END + 8 {
        return None;
    }
    let head = &n.chars[..END];
    let tail = &n.chars[len - END..];
    let heads = occurrences(&h.chars, head);
    let tails = occurrences(&h.chars, tail);
    let hay_n = h.chars.len();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for &a in &heads {
        let lo = a + (len as f32 * 0.7) as usize;
        let hi = a + (len as f32 * 1.4) as usize + END;
        let mut joined = false;
        for &t in tails.iter().filter(|t| **t + END >= lo && **t + END <= hi) {
            spans.push((a, t + END));
            joined = true;
        }
        if !joined {
            // the end differs: the best end within a little of the quote's length
            let ends = (a + (len as f32 * 0.85) as usize)..=(a + (len as f32 * 1.15) as usize);
            let best_end = ends
                .filter(|e| *e <= hay_n)
                .map(|e| (similarity(&n.chars, &h.chars[a..e]), e))
                .max_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((_, e)) = best_end {
                spans.push((a, e));
            }
        }
    }
    for &t in &tails {
        let e = t + END;
        if heads
            .iter()
            .any(|a| *a < e && e <= a + (len as f32 * 1.4) as usize + END)
        {
            continue;
        }
        // the start differs: the best start within a little of the quote's length
        let lo = e.saturating_sub((len as f32 * 1.15) as usize);
        let hi = e.saturating_sub((len as f32 * 0.85) as usize);
        let best_start = (lo..=hi)
            .map(|a| (similarity(&n.chars, &h.chars[a..e]), a))
            .max_by(|x, y| x.0.total_cmp(&y.0));
        if let Some((_, a)) = best_start {
            spans.push((a, e));
        }
    }
    let mut best: Option<(f32, usize, usize)> = None;
    for (a, b) in spans {
        if b <= a {
            continue;
        }
        let s = similarity(&n.chars, &h.chars[a..b]);
        if s < FUZZY_MIN {
            continue;
        }
        let better = match best {
            None => true,
            Some((bs, ba, _)) => {
                let d = |x: usize| hint.map_or(0, |h| (x as i64 - h as i64).unsigned_abs());
                s > bs + 0.02 || ((s - bs).abs() <= 0.02 && d(a) < d(ba))
            }
        };
        if better {
            best = Some((s, a, b));
        }
    }
    let (s, a, b) = best?;
    let (start, end) = h.source_range(a, b);
    Some(Found {
        start,
        end,
        kind: Match::Fuzzy(s),
    })
}

/// Finds `quote` in `hay`: exact, normalised, case folded, then fuzzy.
pub fn locate(hay: &[char], quote: &str, hint: Option<usize>) -> Option<Found> {
    locate_exact(hay, quote, hint).or_else(|| locate_fuzzy(hay, quote, hint))
}

/// Whether `hay[start..end]` holds the quote (ignoring what [`Norm`] ignores).
pub fn verify(hay: &[char], start: i32, end: i32, quote: &str) -> bool {
    if start < 0 || end <= start || start as usize >= hay.len() {
        return false;
    }
    let (a, b) = (start as usize, (end as usize).min(hay.len()));
    let q: Vec<char> = quote.chars().collect();
    let (x, y) = (Norm::new(&hay[a..b], true), Norm::new(&q, true));
    !y.chars.is_empty() && x.chars == y.chars
}

// -- searching a book --------------------------------------------------------------------------------

/// The chapters of a book as the reader numbers their text.
pub trait Chapters {
    fn count(&self) -> usize;
    /// The text of chapter `spine`: usually one string, two when the book's own styles change
    /// what is shown (hidden elements).
    fn texts(&self, spine: usize) -> Vec<std::sync::Arc<Vec<char>>>;
}

/// Where [`resolve`] found a quote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub spine: usize,
    pub found: Found,
}

fn spine_order(count: usize, hint: Option<usize>) -> Vec<usize> {
    let mut v: Vec<usize> = (0..count).collect();
    if let Some(h) = hint {
        v.sort_by_key(|s| ((*s as i64 - h as i64).unsigned_abs(), *s));
    }
    v
}

/// Finds a quote in the chapters. `spine` and `offset` are where it is expected, when anything
/// says so; of several chapters that hold it the one closest to `spine` wins.
pub fn resolve(
    ch: &dyn Chapters,
    quote: &str,
    spine: Option<usize>,
    offset: Option<usize>,
) -> Option<Resolved> {
    let order = spine_order(ch.count(), spine);
    for fuzzy in [false, true] {
        for &s in &order {
            let hint = (Some(s) == spine).then_some(offset).flatten();
            for hay in ch.texts(s) {
                let f = if fuzzy {
                    locate_fuzzy(&hay, quote, hint)
                } else {
                    locate_exact(&hay, quote, hint)
                };
                if let Some(found) = f {
                    return Some(Resolved { spine: s, found });
                }
            }
        }
    }
    None
}

/// Like [`resolve`] for a quote the book holds more than once and that is already highlighted at some
/// of those places: the first exact occurrence (hinted chapter first) that is not in `taken`
/// (`(spine, start, end)`). `None` when every occurrence is taken or there is no exact one.
pub fn resolve_avoiding(
    ch: &dyn Chapters,
    quote: &str,
    spine: Option<usize>,
    taken: &[(usize, i32, i32)],
) -> Option<Resolved> {
    let q: Vec<char> = quote.trim().chars().collect();
    if q.len() < MIN_QUOTE {
        return None;
    }
    for s in spine_order(ch.count(), spine) {
        for hay in ch.texts(s) {
            for p in occurrences(&hay, &q) {
                if !taken.contains(&(s, p as i32, (p + q.len()) as i32)) {
                    return Some(Resolved {
                        spine: s,
                        found: Found {
                            start: p,
                            end: p + q.len(),
                            kind: Match::Exact,
                        },
                    });
                }
            }
        }
    }
    None
}

/// The chapters of an open book, parsed on first use.
pub struct BookChapters {
    book: OpenBook,
    paths: Vec<String>,
    cache: Mutex<HashMap<usize, Vec<Arc<Vec<char>>>>>,
}

impl BookChapters {
    pub fn new(book: OpenBook) -> Self {
        BookChapters {
            paths: book.spine_paths(),
            book,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn book(&self) -> &OpenBook {
        &self.book
    }

    /// The spine index of a chapter file.
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.paths.iter().position(|p| p == path)
    }
}

impl Chapters for BookChapters {
    fn count(&self) -> usize {
        self.paths.len()
    }

    fn texts(&self, spine: usize) -> Vec<Arc<Vec<char>>> {
        if let Some(v) = self.cache.lock().ok().and_then(|c| c.get(&spine).cloned()) {
            return v;
        }
        let mut out: Vec<Arc<Vec<char>>> = Vec::new();
        if let Some(p) = self.paths.get(spine) {
            let plain = self
                .book
                .chapter_doc(p)
                .map(|d| d.reader_text())
                .unwrap_or_default();
            let styled = self
                .book
                .chapter_doc_styled(p)
                .map(|d| d.reader_text())
                .unwrap_or_default();
            out.push(Arc::new(plain.chars().collect()));
            if styled != plain {
                out.push(Arc::new(styled.chars().collect()));
            }
        }
        if let Ok(mut c) = self.cache.lock() {
            c.insert(spine, out.clone());
        }
        out
    }
}

// -- repairing stored highlights ---------------------------------------------------------------------

/// What happened to one stored highlight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Repair {
    /// The range holds the quote.
    Fine,
    /// Moved to where the quote is.
    Moved,
    /// The quote is nowhere: the highlight lost its place.
    Unanchored,
    /// Was unanchored and is placed again.
    Anchored,
    /// Is unanchored and still cannot be placed (or there is nothing to check).
    Unchanged,
}

/// Checks one highlight against the chapters and fixes it. Never touches its text or note.
pub fn repair_one(ch: &dyn Chapters, h: &mut Highlight) -> Repair {
    if is_pdf(h) || h.text.trim().is_empty() {
        return Repair::Unchanged;
    }
    let was_unanchored = is_unanchored(h);
    let (spine, start, end) = hint_of(h);
    if !was_unanchored
        && ch
            .texts(spine)
            .iter()
            .any(|t| verify(t, start, end, &h.text))
    {
        return Repair::Fine;
    }
    match resolve(ch, &h.text, Some(spine), Some(start.max(0) as usize)) {
        Some(r)
            if !was_unanchored
                && (r.spine, r.found.start as i32, r.found.end as i32) == (spine, start, end) =>
        {
            // the text differs a little from the book (found by similarity) but is where it is
            Repair::Fine
        }
        Some(r) => {
            place(h, r.spine, r.found.start, r.found.end);
            if was_unanchored {
                Repair::Anchored
            } else {
                Repair::Moved
            }
        }
        None if was_unanchored => Repair::Unchanged,
        None => {
            unanchor(h);
            Repair::Unanchored
        }
    }
}

/// What a repair pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepairReport {
    pub checked: usize,
    pub fine: usize,
    pub moved: usize,
    pub unanchored: usize,
    pub anchored: usize,
    /// Books with highlights that could not be opened (not an EPUB or a damaged file).
    pub unreadable: usize,
}

impl RepairReport {
    pub fn changed(&self) -> usize {
        self.moved + self.unanchored + self.anchored
    }
}

impl Library {
    /// Re-checks every stored highlight against its book's text (see the module notes). One
    /// operation in the log holds the highlights as they were; `undo_last` puts them back.
    /// `only` limits the pass to some books.
    pub fn repair_highlights(&mut self, only: Option<&[u64]>) -> Result<RepairReport> {
        self.writable()?;
        let mut report = RepairReport::default();
        let mut before: Vec<(u64, Vec<Highlight>)> = Vec::new();
        let ids: Vec<u64> = self
            .books
            .iter()
            .filter(|b| b.highlights.iter().any(|h| !is_pdf(h)))
            .map(|b| b.id)
            .filter(|id| only.is_none_or(|o| o.contains(id)))
            .collect();
        for id in ids {
            let Some(book) = self.get(id) else { continue };
            let Ok(open) = self.open_book(id) else {
                report.unreadable += 1;
                continue;
            };
            let chapters = BookChapters::new(open);
            let mut fixed = book.highlights.clone();
            let mut changed = false;
            for h in fixed.iter_mut() {
                let r = repair_one(&chapters, h);
                if !is_pdf(h) && !h.text.trim().is_empty() {
                    report.checked += 1;
                }
                match r {
                    Repair::Fine => report.fine += 1,
                    Repair::Moved => report.moved += 1,
                    Repair::Unanchored => report.unanchored += 1,
                    Repair::Anchored => report.anchored += 1,
                    Repair::Unchanged => {}
                }
                changed |= matches!(r, Repair::Moved | Repair::Unanchored | Repair::Anchored);
            }
            if changed {
                let old = std::mem::replace(
                    &mut self.get_mut(id).expect("book is there").highlights,
                    fixed,
                );
                before.push((id, old));
            }
        }
        if !before.is_empty() {
            self.save()?;
            let label = format!("Re-checked the place of {} highlights", report.changed());
            self.log(label, OpKind::RepairHighlights { before });
        }
        Ok(report)
    }

    /// Puts a change of highlights that was made outside (a colour taken from a device, say) in the
    /// operation log: `before` are the highlights of every book that changed, as they were.
    pub fn record_highlight_edit(&mut self, label: String, before: Vec<(u64, Vec<Highlight>)>) {
        if !before.is_empty() {
            self.log(label, OpKind::RepairHighlights { before });
        }
    }

    pub(crate) fn undo_repair_highlights(
        &mut self,
        before: &[(u64, Vec<Highlight>)],
    ) -> Result<()> {
        for (id, hs) in before {
            if let Some(b) = self.get_mut(*id) {
                b.highlights = hs.clone();
            }
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    struct Fixed(Vec<String>);
    impl Chapters for Fixed {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn texts(&self, s: usize) -> Vec<std::sync::Arc<Vec<char>>> {
            vec![std::sync::Arc::new(chars(&self.0[s]))]
        }
    }

    fn hl(spine: usize, start: i32, end: i32, text: &str) -> Highlight {
        Highlight {
            id: "h".into(),
            spine,
            start,
            end,
            color: "yellow".into(),
            style: String::new(),
            note: "my note".into(),
            text: text.into(),
            created: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_repeated_term_is_found_at_each_place_in_turn() {
        let book = Fixed(vec![
            "Request-response is one style. And later Request-response again.".into(),
            "Request-response in the next chapter.".into(),
        ]);
        let q = "Request-response";
        let a = resolve_avoiding(&book, q, Some(0), &[]).unwrap();
        assert_eq!((a.spine, a.found.start), (0, 0));
        let b = resolve_avoiding(&book, q, Some(0), &[(0, 0, 16)]).unwrap();
        assert_eq!((b.spine, b.found.start), (0, 41));
        let c = resolve_avoiding(&book, q, Some(0), &[(0, 0, 16), (0, 41, 57)]).unwrap();
        assert_eq!((c.spine, c.found.start), (1, 0));
        assert!(
            resolve_avoiding(&book, q, Some(0), &[(0, 0, 16), (0, 41, 57), (1, 0, 16)]).is_none()
        );
    }

    #[test]
    fn exact_and_loose_matches_return_the_source_range() {
        let hay = chars("It was “the best” of times — truly.  The ﬁrst of all.");
        let f = locate(&hay, "\"the best\" of times - truly.", None).unwrap();
        assert_eq!(f.kind, Match::Normalised);
        let got: String = hay[f.start..f.end].iter().collect();
        assert_eq!(got, "“the best” of times — truly.");
        let f = locate(&hay, "The first of all.", None).unwrap();
        let got: String = hay[f.start..f.end].iter().collect();
        assert_eq!(got, "The ﬁrst of all.");
        let f = locate(&hay, "THE BEST\" OF TIMES", None).unwrap();
        assert_eq!(f.kind, Match::CaseFolded);
        assert!(locate(&hay, "nothing like it here", None).is_none());
        assert!(locate(&hay, "short", None).is_none());
    }

    #[test]
    fn of_several_occurrences_the_one_closest_to_the_hint_wins() {
        let hay = chars("the same sentence here. xxxxxxxx the same sentence here. yyyyyyyy the same sentence here.");
        let all = occurrences(&hay, &chars("the same sentence here."));
        assert_eq!(all.len(), 3);
        for (i, want) in all.iter().enumerate() {
            let hint = want + 3;
            assert_eq!(
                locate(&hay, "the same sentence here.", Some(hint))
                    .unwrap()
                    .start,
                *want,
                "occurrence {i}"
            );
        }
        assert_eq!(
            locate(&hay, "the same sentence here.", None).unwrap().start,
            0
        );
    }

    #[test]
    fn a_slightly_different_quote_is_found_fuzzily_and_a_different_one_is_not() {
        let hay = chars("Intro text. Independent deployability is the idea that we can make a change to a microservice, deploy it, and release that change to our users, without having to deploy any other microservices. More text follows.");
        let device = "Independent deployability is the idea that we can make a change to a microservice, deploy it and release that change to our users, without having to deploy any other microservices";
        let f = locate(&hay, device, Some(0)).unwrap();
        assert!(matches!(f.kind, Match::Fuzzy(s) if s > 0.9));
        let got: String = hay[f.start..f.end].iter().collect();
        assert!(got.starts_with("Independent deployability") && got.ends_with("microservices"));
        let other = "Independent deployability is a thing about something else entirely, and then some more words that are not in the book at all";
        assert!(locate(&hay, other, None).is_none());
    }

    #[test]
    fn a_device_that_changed_a_word_at_either_end_still_finds_the_passage() {
        let hay = chars("Before. Paragraph 15 of chapter 1 talks about functionality, readers and the world around them in plain words. After it comes more text.");
        let end_changed = "Paragraph 15 of chapter 1 talks about functionality, readers and the world around them in simple words.";
        let f = locate(&hay, end_changed, Some(0)).unwrap();
        assert!(matches!(f.kind, Match::Fuzzy(_)));
        let got: String = hay[f.start..f.end].iter().collect();
        assert!(got.starts_with("Paragraph 15 of chapter 1"), "{got:?}");
        assert!(
            got.ends_with("plain words.") || got.ends_with("plain words"),
            "{got:?}"
        );
        let start_changed = "Chapter 15 of the book talks about functionality, readers and the world around them in plain words.";
        let f = locate(&hay, start_changed, Some(0)).unwrap();
        let got: String = hay[f.start..f.end].iter().collect();
        assert!(got.ends_with("plain words."), "{got:?}");
        assert!(
            got.contains("15 of chapter 1") && !got.contains("Before"),
            "{got:?}"
        );
    }

    #[test]
    fn verify_compares_what_the_range_holds() {
        let hay = chars("Aaa bbb  ccc ddd eee.");
        assert!(verify(&hay, 4, 12, "bbb ccc"));
        assert!(!verify(&hay, 5, 12, "bbb ccc"));
        assert!(!verify(&hay, -1, 12, "bbb ccc"));
        assert!(!verify(&hay, 4, 4, "bbb ccc"));
    }

    #[test]
    fn the_quote_is_searched_in_the_hinted_chapter_then_its_neighbours_then_everywhere() {
        let ch = Fixed(vec![
            "zero: a quote that repeats twice.".into(),
            "one: nothing here".into(),
            "two: a quote that repeats twice. and the unique far away sentence".into(),
        ]);
        assert_eq!(
            resolve(&ch, "a quote that repeats twice.", Some(2), None)
                .unwrap()
                .spine,
            2
        );
        assert_eq!(
            resolve(&ch, "a quote that repeats twice.", Some(0), None)
                .unwrap()
                .spine,
            0
        );
        assert_eq!(
            resolve(&ch, "a quote that repeats twice.", Some(1), None)
                .unwrap()
                .spine,
            0
        );
        assert_eq!(
            resolve(&ch, "the unique far away sentence", Some(0), None)
                .unwrap()
                .spine,
            2
        );
        assert!(resolve(&ch, "this is not in the book anywhere", Some(0), None).is_none());
    }

    #[test]
    fn repairing_moves_a_shifted_range_and_unanchors_what_is_nowhere() {
        let text = "First paragraph of the chapter.\nThe quick brown fox jumps over the lazy dog.\nLast one.";
        let ch = Fixed(vec![text.into()]);
        let a = text.find("The quick").unwrap() as i32;
        let b = a + "The quick brown fox jumps over the lazy dog."
            .chars()
            .count() as i32;
        let quote = "The quick brown fox jumps over the lazy dog.";

        let mut ok = hl(0, a, b, quote);
        assert_eq!(repair_one(&ch, &mut ok), Repair::Fine);
        assert_eq!((ok.start, ok.end), (a, b));

        let mut shifted = hl(0, a + 7, b + 7, quote);
        assert_eq!(repair_one(&ch, &mut shifted), Repair::Moved);
        assert_eq!((shifted.start, shifted.end), (a, b));
        assert_eq!(shifted.note, "my note");

        let mut lost = hl(0, a, b, "A sentence the book never had at all.");
        assert_eq!(repair_one(&ch, &mut lost), Repair::Unanchored);
        assert!(is_unanchored(&lost));
        assert_eq!(
            hint_of(&lost),
            (0, a, b),
            "the old offsets stay in the record"
        );
        assert_eq!(lost.text, "A sentence the book never had at all.");
        assert_eq!(repair_one(&ch, &mut lost), Repair::Unchanged);

        lost.text = quote.into();
        assert_eq!(repair_one(&ch, &mut lost), Repair::Anchored);
        assert_eq!((lost.start, lost.end), (a, b));
    }

    #[test]
    fn unplaced_notes_round_trip_their_hint() {
        let mut h = hl(3, 120, 150, "some text of a quote");
        unanchor(&mut h);
        assert!(is_unanchored(&h) && h.start < 0 && h.end < 0);
        assert_eq!(hint_of(&h), (3, 120, 150));
        unanchor(&mut h);
        assert_eq!(hint_of(&h), (3, 120, 150), "idempotent");
        let n = unplaced(2);
        let mut u = hl(n.0, n.1, n.2, "x");
        assert!(is_unanchored(&u));
        assert_eq!(hint_of(&u).0, 2);
        place(&mut u, 1, 5, 9);
        assert!(!is_unanchored(&u));
    }
}
