//! The colour and style of highlights that come from a device, as far as the plan is concerned
//! (the table of words is `bookx_core::devstyle`). No GTK, unit tested.
//!
//! Rules (they are what a re-import or a sync does with a quote the library already has):
//!
//! * **The look comes with the highlight.** A new highlight is stored with the colour and style the
//!   device gave it; what the device does not say stays Bookx's default.
//! * **Two highlights of the same words in different colours are two highlights.**
//! * **The device's word is taken only when the device changed it.** The *ledger*
//!   (`devstyle::DeviceLooks`) holds what the device said the last time; if it says the same now,
//!   the library's look (maybe changed by the reader in Bookx) is left alone; if it says something
//!   else, the device changed it later and its word wins.
//! * **Repair of older imports.** A highlight imported before this was understood has no ledger entry
//!   and Bookx's default look; when the device says something else, the device's look is taken (the
//!   reader never chose the default: every import got it). One that has any other look, and no ledger
//!   entry, is left alone (the reader may have chosen it); the device's word is remembered.

use std::collections::{BTreeMap, HashSet};

use crate::core::devstyle::{self, Look};
use crate::core::library::Highlight;
use bookx_serve::syncplan::{self, Annot};

/// What a plan says about the look of one library highlight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookIn {
    pub id: String,
    /// The look to give the highlight (`None`: leave it as it is).
    pub set: Option<Look>,
    /// What the device said, for the ledger.
    pub device: Look,
}

/// The look a device row asks for: parts it does not say are `None`.
pub type Want = (Option<String>, Option<String>);

fn annot(text: &str) -> Annot {
    Annot {
        text: text.to_string(),
        note: String::new(),
        when: None,
    }
}

fn own_look(h: &Highlight) -> Look {
    devstyle::full_look(&h.color, &h.style)
}

/// The look the device is known to have for a highlight: the ledger's, else the highlight's own.
fn device_side(h: &Highlight, ledger: &BTreeMap<String, Look>) -> Look {
    ledger.get(&h.id).cloned().unwrap_or_else(|| own_look(h))
}

fn matches(want: &Want, l: &Look) -> bool {
    want.0.as_ref().is_none_or(|c| *c == l.color) && want.1.as_ref().is_none_or(|s| *s == l.style)
}

/// What to do with a device row whose text a library highlight may already have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// No unclaimed highlight has these words: the row is a highlight the library lacks.
    New,
    /// This highlight is the row; maybe with a new look.
    Existing(LookIn),
}

/// Picks the library highlight a device row is, and marks it `claimed` (a highlight is the row of
/// one device highlight only, so two rows with the same words are two highlights).
pub fn pick(
    highlights: &[Highlight],
    ledger: &BTreeMap<String, Look>,
    claimed: &mut HashSet<String>,
    text: &str,
    want: &Want,
) -> Pick {
    let a = annot(text);
    let mut cands: Vec<&Highlight> = highlights
        .iter()
        .filter(|h| !claimed.contains(&h.id) && syncplan::same_highlight(&annot_of(h), &a))
        .collect();
    if cands.is_empty() {
        return Pick::New;
    }
    // the same words beat a longer passage that merely contains them: "service discovery" on its
    // own is not the long highlight it is part of
    let words = syncplan::norm(text);
    if cands.iter().any(|h| syncplan::norm(&h.text) == words) {
        cands.retain(|h| syncplan::norm(&h.text) == words);
    }
    // the same look first: that is the same highlight, unchanged
    let same = cands
        .iter()
        .find(|h| matches(want, &device_side(h, ledger)))
        .copied();
    let h = same.unwrap_or(cands[0]);
    claimed.insert(h.id.clone());
    let device = {
        let mut d = device_side(h, ledger);
        if let Some(c) = &want.0 {
            d.color = c.clone();
        }
        if let Some(s) = &want.1 {
            d.style = s.clone();
        }
        d
    };
    let set = if same.is_some() || (want.0.is_none() && want.1.is_none()) {
        None
    } else if ledger.contains_key(&h.id) {
        // the device said something else than the last time: it changed it later
        Some(device.clone())
    } else if devstyle::is_default_look(&h.color, &h.style) {
        // imported before colours were read: the reader never chose this look
        Some(device.clone())
    } else {
        None
    };
    Pick::Existing(LookIn {
        id: h.id.clone(),
        set: set.map(|mut l| {
            // a part the device does not say stays as it is
            if want.0.is_none() {
                l.color = own_look(h).color;
            }
            if want.1.is_none() {
                l.style = own_look(h).style;
            }
            l
        }),
        device,
    })
}

fn annot_of(h: &Highlight) -> Annot {
    Annot {
        text: h.text.clone(),
        note: h.note.clone(),
        when: None,
    }
}

/// The part of a look the device gave, as a ledger entry or a new highlight's colour and style.
pub fn want_of(color: &Option<String>, style: &Option<String>) -> Want {
    devstyle::look_of(color.as_deref(), style.as_deref())
}

/// Whether a highlight already has the look a row asks for (the parts it says).
pub fn has_look(h: &Highlight, want: &Want) -> bool {
    matches(want, &own_look(h))
}

/// Applies a plan's look changes; returns how many highlights changed and the ledger entries to
/// remember (`(highlight id, device look)`).
pub fn apply(highlights: &mut [Highlight], looks: &[LookIn]) -> (usize, Vec<(String, Look)>) {
    let mut changed = 0;
    let mut ledger = Vec::new();
    for l in looks {
        let Some(h) = highlights.iter_mut().find(|h| h.id == l.id) else {
            continue;
        };
        if let Some(set) = &l.set {
            if own_look(h) != *set {
                h.color = set.color.clone();
                h.style = set.style.clone();
                changed += 1;
            }
        }
        ledger.push((l.id.clone(), l.device.clone()));
    }
    (changed, ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hl(id: &str, text: &str, color: &str, style: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: 1,
            color: color.into(),
            style: style.into(),
            note: String::new(),
            text: text.into(),
            created: chrono::Utc::now(),
        }
    }

    fn want(c: &str, s: &str) -> Want {
        (
            Some(c).filter(|c| !c.is_empty()).map(String::from),
            Some(s).filter(|s| !s.is_empty()).map(String::from),
        )
    }

    const T: &str = "The same sentence in two colours.";

    #[test]
    fn two_rows_with_the_same_words_in_two_colours_are_two_highlights() {
        let lib = [hl("a", T, "yellow", "background")];
        let (mut claimed, ledger) = (HashSet::new(), BTreeMap::new());
        let p1 = pick(&lib, &ledger, &mut claimed, T, &want("yellow", ""));
        assert!(matches!(&p1, Pick::Existing(l) if l.id == "a" && l.set.is_none()));
        let p2 = pick(&lib, &ledger, &mut claimed, T, &want("red", ""));
        assert_eq!(p2, Pick::New, "the second colour is a highlight of its own");
    }

    #[test]
    fn an_older_import_with_the_default_look_takes_the_devices_look() {
        let lib = [hl("a", T, "yellow", "")];
        let (mut claimed, ledger) = (HashSet::new(), BTreeMap::new());
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("blue", "underline"))
        else {
            panic!()
        };
        assert_eq!(
            l.set,
            Some(Look {
                color: "blue".into(),
                style: "underline".into()
            })
        );
        let mut hs = lib.to_vec();
        let (n, ledger) = apply(&mut hs, &[l]);
        assert_eq!(n, 1);
        assert_eq!(
            (hs[0].color.as_str(), hs[0].style.as_str()),
            ("blue", "underline")
        );
        assert_eq!(ledger[0].1.color, "blue");
    }

    #[test]
    fn a_part_the_device_does_not_say_is_left_alone() {
        let lib = [hl("a", T, "yellow", "")];
        let (mut claimed, ledger) = (HashSet::new(), BTreeMap::new());
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("green", "")) else {
            panic!()
        };
        assert_eq!(
            l.set,
            Some(Look {
                color: "green".into(),
                style: "background".into()
            })
        );
        // a style the reader chose (not Bookx's default look) is not taken for a device's
        let lib = [hl("a", T, "yellow", "underline")];
        let mut claimed = HashSet::new();
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("green", "")) else {
            panic!()
        };
        assert_eq!(l.set, None);
    }

    #[test]
    fn a_look_the_reader_chose_in_bookx_survives_a_reimport_until_the_device_changes_it() {
        // the device said yellow; the reader made it red in Bookx
        let lib = [hl("a", T, "red", "background")];
        let mut ledger = BTreeMap::new();
        ledger.insert(
            "a".to_string(),
            Look {
                color: "yellow".into(),
                style: "background".into(),
            },
        );
        let mut claimed = HashSet::new();
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("yellow", "")) else {
            panic!()
        };
        assert_eq!(
            l.set, None,
            "the device says what it said before: nothing is reset"
        );
        // later the device changes it to green
        let mut claimed = HashSet::new();
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("green", "")) else {
            panic!()
        };
        assert_eq!(l.set.as_ref().map(|s| s.color.as_str()), Some("green"));
    }

    #[test]
    fn a_non_default_look_without_a_ledger_is_not_overwritten() {
        let lib = [hl("a", T, "purple", "background")];
        let (mut claimed, ledger) = (HashSet::new(), BTreeMap::new());
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &want("blue", "")) else {
            panic!()
        };
        assert_eq!(l.set, None);
        assert_eq!(l.device.color, "blue", "the device's word is remembered");
    }

    #[test]
    fn rows_without_a_look_change_nothing() {
        let lib = [hl("a", T, "red", "")];
        let (mut claimed, ledger) = (HashSet::new(), BTreeMap::new());
        let Pick::Existing(l) = pick(&lib, &ledger, &mut claimed, T, &(None, None)) else {
            panic!()
        };
        assert_eq!(l.set, None);
    }

    // -- through the real readers, the plan and the library ---------------------------------------

    use crate::core::convert::{Chapter, create_epub};
    use crate::core::library::{AddOutcome, Library};
    use crate::core::opf::Metadata;
    use std::path::{Path, PathBuf};

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-looks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn para(i: usize) -> String {
        format!("Paragraph number {i} has some words in it, and a few more besides.")
    }

    fn library(dir: &Path) -> (Library, u64) {
        let body: String = (0..30).map(|i| format!("<p>{}</p>", para(i))).collect();
        let md = Metadata {
            title: "Micro".into(),
            authors: vec!["Sam".into()],
            language: "en".into(),
            ..Default::default()
        };
        let ch = [Chapter {
            title: "One".into(),
            body: format!("<h1>One</h1>{body}"),
        }];
        let src = dir.join("m.epub");
        std::fs::write(&src, create_epub(&md, &ch, None).to_bytes().unwrap()).unwrap();
        let mut lib = Library::in_memory(&dir.join("lib")).unwrap();
        let AddOutcome::Added(id) = lib.add_path(&src, true).unwrap() else {
            panic!()
        };
        (lib, id)
    }

    /// A Kobo database with highlights in the four colours the Kobo has, a bookmark that is no
    /// highlight, and a note.
    fn kobo(dir: &Path, rows: &[(&str, i64, &str, &str)]) -> PathBuf {
        let db = dir.join("KoboReader.sqlite");
        bookx_serve::kobo::create_real_schema_database(&db).unwrap();
        let cid = "file:///mnt/onboard/m.epub";
        let (first, rest) = rows.split_first().unwrap();
        bookx_serve::kobo::add_test_highlight(&db, cid, "Micro", "Sam", first.2, first.3).unwrap();
        // the first row of the helper is colour 0; set the colour asked for
        bookx_serve::kobo::set_test_color_all(&db, first.1).unwrap();
        for (_, color, text, note) in rest {
            bookx_serve::kobo::add_test_highlight_colored(
                &db,
                cid,
                text,
                note,
                *color,
                "highlight",
            )
            .unwrap();
        }
        db
    }

    fn run(lib: &mut Library, paths: &[PathBuf]) -> crate::annotation_import::ImportPlan {
        let (items, problems) = crate::annotation_import::read_paths(paths);
        assert!(problems.is_empty(), "{problems:?}");
        let mut books = crate::device_sync::library_books(&lib.books);
        crate::device_sync::attach_looks(&mut books, &lib.root);
        let plan =
            crate::annotation_import::plan(&items, &books, &crate::device_sync::book_locator(lib));
        crate::annotation_import::apply(lib, &plan);
        plan
    }

    fn looks(lib: &Library, id: u64) -> Vec<(String, String, String)> {
        let mut v: Vec<_> = lib
            .get(id)
            .unwrap()
            .highlights
            .iter()
            .map(|h| {
                (
                    h.text
                        .chars()
                        .skip(17)
                        .take(2)
                        .collect::<String>()
                        .trim()
                        .to_string(),
                    h.color.clone(),
                    h.style.clone(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_kobo_with_four_colours_keeps_all_four() {
        let d = tmp();
        let (mut lib, id) = library(&d);
        let db = kobo(
            &d,
            &[
                ("a", 0, &para(3), ""),
                ("b", 1, &para(4), ""),
                ("c", 2, &para(5), ""),
                ("d", 3, &para(6), "a note"),
            ],
        );
        run(&mut lib, std::slice::from_ref(&db));
        let got = looks(&lib, id);
        assert_eq!(
            got.iter().map(|g| g.1.as_str()).collect::<Vec<_>>(),
            ["yellow", "pink", "blue", "green"],
            "one colour per highlight, as the device had them: {got:?}"
        );
        // a dog-ear is no highlight and does not come in
        assert_eq!(lib.get(id).unwrap().highlights.len(), 4);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn koreader_colours_and_drawers_are_kept_and_unknown_colours_as_written() {
        let d = tmp();
        let (mut lib, id) = library(&d);
        let entry = |n: usize, color: &str, drawer: &str| {
            format!(
                "  [{n}] = {{ [\"text\"] = {:?}, [\"color\"] = \"{color}\", [\"drawer\"] = \"{drawer}\", [\"pos0\"] = \"/body/DocFragment[1]/body/p[{n}]/text().0\", [\"datetime\"] = \"2026-08-17 11:22:15\" }},\n",
                para(n + 7)
            )
        };
        let lua = format!(
            "return {{\n [\"annotations\"] = {{\n{}{}{}{}{} }},\n [\"doc_props\"] = {{ [\"title\"] = \"Micro\", [\"authors\"] = \"Sam\" }},\n}}\n",
            entry(1, "yellow", "lighten"),
            entry(2, "red", "underscore"),
            entry(3, "green", "strikeout"),
            entry(4, "olive", "invert"),
            entry(5, "purple", "lighten"),
        );
        let sdr = d.join("Micro.sdr");
        std::fs::create_dir_all(&sdr).unwrap();
        std::fs::write(sdr.join("metadata.epub.lua"), lua).unwrap();
        run(&mut lib, &[sdr.join("metadata.epub.lua")]);
        let hs = &lib.get(id).unwrap().highlights;
        let look = |n: usize| {
            let h = hs.iter().find(|h| h.text == para(n + 7)).unwrap();
            (h.color.as_str(), h.style.as_str())
        };
        assert_eq!(look(1), ("yellow", "background"));
        assert_eq!(look(2), ("red", "underline"));
        assert_eq!(look(3), ("green", "strikethrough"));
        assert_eq!(
            look(4),
            ("olive", "invert"),
            "a colour Bookx's palette lacks is kept as written"
        );
        assert_eq!(look(5), ("purple", "background"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_same_words_in_two_colours_stay_two_highlights() {
        let d = tmp();
        let (mut lib, id) = library(&d);
        let db = kobo(&d, &[("a", 0, &para(9), ""), ("b", 2, &para(9), "")]);
        let plan = run(&mut lib, std::slice::from_ref(&db));
        assert_eq!(plan.new_highlights(), 2, "{}", plan.text());
        let got = looks(&lib, id);
        assert_eq!(
            got.iter().map(|g| g.1.as_str()).collect::<Vec<_>>(),
            ["blue", "yellow"]
        );
        // reading it again changes nothing and adds nothing
        let again = run(&mut lib, &[db]);
        assert_eq!((again.new_highlights(), again.already()), (0, 2));
        assert_eq!(lib.get(id).unwrap().highlights.len(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_reimport_keeps_what_the_reader_changed_in_bookx_until_the_device_changes_it() {
        let d = tmp();
        let (mut lib, id) = library(&d);
        let db = kobo(&d, &[("a", 3, &para(2), "")]);
        run(&mut lib, std::slice::from_ref(&db));
        assert_eq!(lib.get(id).unwrap().highlights[0].color, "green");
        // the reader makes it red in Bookx; the device still says green
        lib.get_mut(id).unwrap().highlights[0].color = "red".into();
        run(&mut lib, std::slice::from_ref(&db));
        assert_eq!(lib.get(id).unwrap().highlights[0].color, "red", "not reset");
        // now the device says blue
        bookx_serve::kobo::set_test_color_all(&db, 2).unwrap();
        run(&mut lib, &[db]);
        assert_eq!(
            lib.get(id).unwrap().highlights[0].color,
            "blue",
            "the device changed it later"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    /// The owner's bug: everything imported earlier is yellow. The next read of the device fixes
    /// it, as one step in the operation log that undo takes back.
    #[test]
    fn highlights_imported_with_the_wrong_colour_are_repaired_and_it_can_be_undone() {
        let d = tmp();
        let (mut lib, id) = library(&d);
        let db = kobo(&d, &[("a", 1, &para(4), ""), ("b", 2, &para(5), "")]);
        // what an older Bookx made of them: yellow, no ledger
        let hs: Vec<Highlight> = [4, 5]
            .iter()
            .map(|n| {
                let mut h = hl(&format!("old{n}"), &para(*n), "yellow", "");
                h.start = 5;
                h.end = 9;
                h
            })
            .collect();
        lib.get_mut(id).unwrap().highlights = hs.clone();
        let before_ops = lib.history().len();
        run(&mut lib, &[db]);
        let got: Vec<_> = lib
            .get(id)
            .unwrap()
            .highlights
            .iter()
            .map(|h| h.color.clone())
            .collect();
        assert_eq!(got, ["pink", "blue"]);
        assert_eq!(lib.history().len(), before_ops + 1, "one step in the log");
        lib.undo_last().unwrap();
        assert_eq!(lib.get(id).unwrap().highlights, hs);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn what_bookx_writes_for_koreader_and_in_its_json_reads_back_with_the_same_look() {
        let notes = crate::send_notes::Notes::new(
            &[
                hl("1", "First quote of some length", "yellow", ""),
                hl("2", "Second quote of some length", "red", "underline"),
                hl("3", "Third quote of some length", "olive", "strikethrough"),
                hl("4", "Fourth quote of some length", "blue", "invert"),
            ],
            &[],
        );
        let lua = crate::send_notes::koreader_sidecar("Micro", &[], &notes);
        let back = crate::annotation_import::read_koreader_lua(&lua, Path::new("Micro.sdr/m.lua"));
        let want = [
            ("yellow", "background"),
            ("red", "underline"),
            ("olive", "strikethrough"),
            ("blue", "invert"),
        ];
        for (it, (c, s)) in back.iter().zip(want) {
            let (color, style) = want_of(&it.color, &it.style);
            assert_eq!(
                (color.as_deref(), style.as_deref()),
                (Some(c), Some(s)),
                "{}",
                it.text
            );
        }
        // Bookx's JSON export
        let mut b = serde_json::json!({"title": "Micro", "authors": ["Sam"], "highlights": []});
        b["highlights"] = serde_json::json!([
            {"text": "Quote with a colour", "color": "orange", "style": "squiggly", "note": "", "chapter": "One"}
        ]);
        let items = bookx_lookup::import::parse_json(&b.to_string()).unwrap();
        assert_eq!(
            want_of(&items[0].color, &items[0].style),
            (Some("orange".into()), Some("squiggly".into()))
        );
    }

    #[test]
    fn dedupe_of_an_import_file_keeps_the_same_words_in_two_colours() {
        use bookx_lookup::import::{Imported, dedupe};
        let mk = |c: Option<&str>| Imported {
            book_title: "Micro".into(),
            book_author: None,
            text: "The same sentence twice over.".into(),
            note: String::new(),
            location: None,
            chapter: None,
            created: None,
            bookmark: false,
            color: c.map(String::from),
            style: None,
        };
        assert_eq!(dedupe(vec![mk(Some("yellow")), mk(Some("blue"))]).len(), 2);
        assert_eq!(
            dedupe(vec![mk(Some("yellow")), mk(Some("yellow"))]).len(),
            1
        );
        assert_eq!(
            dedupe(vec![mk(None), mk(Some("blue"))]).len(),
            1,
            "a source without colours adds none of its own"
        );
    }
}
