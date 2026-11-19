//! Validation of EPUB 3 Media Overlays (CE-422): the package wiring, the SMIL files, their references
//! and the durations. [`check_overlays`] returns messages in the format of [`crate::validate`] (rule
//! ids `MED-xxx`, listed in [`MEDIA_RULES`]); `validate()` calls it for every book.

use std::collections::{HashMap, HashSet};

use bookx_core::archive::Archive;
use bookx_core::util::{parent_dir, parse_xml, resolve_href};

use crate::markup::defined_ids;
use crate::media_overlay::{duration_ms, parse_clock, parse_smil, Par};
use crate::media_overlay_opf::SMIL_TYPE;
use crate::validate::{Message, Rule, Severity};

const E: Severity = Severity::Error;
const W: Severity = Severity::Warning;
const U: Severity = Severity::Usage;

pub const MEDIA_RULES: &[Rule] = &[
    Rule {
        id: "MED-001",
        severity: E,
        title: "media-overlay refers to a manifest item that does not exist",
    },
    Rule {
        id: "MED-002",
        severity: E,
        title: "media-overlay refers to an item that is not application/smil+xml",
    },
    Rule {
        id: "MED-003",
        severity: E,
        title: "the media overlay document is missing from the container",
    },
    Rule {
        id: "MED-004",
        severity: E,
        title: "the media overlay document is not well-formed or too large",
    },
    Rule {
        id: "MED-005",
        severity: E,
        title: "a media overlay text reference points at a missing file or id",
    },
    Rule {
        id: "MED-006",
        severity: E,
        title: "a media overlay audio reference points at a missing file",
    },
    Rule {
        id: "MED-007",
        severity: W,
        title: "a media overlay par has an unusable clip or reference and is ignored",
    },
    Rule {
        id: "MED-008",
        severity: W,
        title: "the media:duration of an overlay is missing or differs from its clips",
    },
    Rule {
        id: "MED-009",
        severity: W,
        title: "the package has overlays but no total media:duration",
    },
    Rule {
        id: "MED-010",
        severity: W,
        title: "a SMIL item in the manifest is not used by any chapter",
    },
    Rule {
        id: "MED-011",
        severity: U,
        title: "the audio is not in a core media type; some reading systems cannot play it",
    },
    Rule {
        id: "MED-012",
        severity: W,
        title: "the overlay of a chapter reads the text of another document",
    },
];

/// Audio types every reading system must support.
pub const CORE_AUDIO: &[&str] = &["audio/mpeg", "audio/mp4", "audio/ogg"];

struct Item {
    id: String,
    path: String,
    media_type: String,
    overlay: Option<String>,
}

struct Meta {
    property: String,
    refines: Option<String>,
    text: String,
}

fn msg(id: &str, file: &str, text: String) -> Message {
    let severity = MEDIA_RULES
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.severity)
        .unwrap_or(E);
    Message {
        id: id.to_string(),
        severity,
        file: file.to_string(),
        line: None,
        col: None,
        text,
    }
}

fn read_package(archive: &Archive) -> Option<(String, Vec<Item>, Vec<Meta>)> {
    let opf_path = bookx_core::opf::find_opf_path(archive).ok()?;
    let text = archive.get_string(&opf_path)?;
    let doc = parse_xml(&text).ok()?;
    let base = parent_dir(&opf_path).to_string();
    let mut items = Vec::new();
    let mut metas = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "item" => {
                let href = n.attribute("href").unwrap_or("");
                items.push(Item {
                    id: n.attribute("id").unwrap_or("").to_string(),
                    path: resolve_href(&base, href).0,
                    media_type: n.attribute("media-type").unwrap_or("").to_string(),
                    overlay: n.attribute("media-overlay").map(str::to_string),
                });
            }
            "meta" => {
                if let Some(p) = n.attribute("property") {
                    metas.push(Meta {
                        property: p.to_string(),
                        refines: n
                            .attribute("refines")
                            .map(|r| r.trim_start_matches('#').to_string()),
                        text: n.text().unwrap_or("").trim().to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    Some((opf_path, items, metas))
}

/// Does the book have any media overlay wiring at all?
pub fn has_overlays(archive: &Archive) -> bool {
    read_package(archive).is_some_and(|(_, items, _)| {
        items
            .iter()
            .any(|i| i.overlay.is_some() || i.media_type == SMIL_TYPE)
    })
}

/// Checks the overlays of `archive`. An empty list for a book without overlays.
pub fn check_overlays(archive: &Archive) -> Vec<Message> {
    let Some((opf_path, items, metas)) = read_package(archive) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let by_id: HashMap<&str, &Item> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    let mut used_smil: HashSet<&str> = HashSet::new();
    let mut any = false;
    let mut ids_cache: HashMap<String, HashSet<String>> = HashMap::new();
    let mut audio_types_seen: HashSet<String> = HashSet::new();
    for ch in items.iter().filter(|i| i.overlay.is_some()) {
        any = true;
        let sid = ch.overlay.as_deref().unwrap_or("");
        let Some(smil) = by_id.get(sid) else {
            out.push(msg(
                "MED-001",
                &opf_path,
                format!("the chapter \"{}\" has media-overlay=\"{sid}\", but the manifest has no such item", ch.id),
            ));
            continue;
        };
        used_smil.insert(sid);
        if smil.media_type != SMIL_TYPE {
            out.push(msg(
                "MED-002",
                &opf_path,
                format!(
                    "\"{sid}\" has the media type {}, not {SMIL_TYPE}",
                    smil.media_type
                ),
            ));
            continue;
        }
        let Some(xml) = archive.get_string(&smil.path) else {
            let id = if archive.contains(&smil.path) {
                "MED-004"
            } else {
                "MED-003"
            };
            out.push(msg(
                id,
                &smil.path,
                format!("cannot read the overlay document {}", smil.path),
            ));
            continue;
        };
        let read = match parse_smil(&smil.path, &xml) {
            Ok(r) => r,
            Err(e) => {
                out.push(msg("MED-004", &smil.path, format!("{e:#}")));
                continue;
            }
        };
        if read.skipped > 0 {
            out.push(msg(
                "MED-007",
                &smil.path,
                format!(
                    "{} par element(s) have no usable text, audio or clip and are ignored",
                    read.skipped
                ),
            ));
        }
        if read.truncated {
            out.push(msg(
                "MED-007",
                &smil.path,
                "too many par elements; the rest is ignored".to_string(),
            ));
        }
        let mut reported_text: HashSet<(String, Option<String>)> = HashSet::new();
        let mut reported_audio: HashSet<String> = HashSet::new();
        let mut other_doc = false;
        for p in &read.pars {
            if p.text_path != ch.path {
                other_doc = true;
            }
            if !archive.contains(&p.text_path) {
                if reported_text.insert((p.text_path.clone(), None)) {
                    out.push(msg(
                        "MED-005",
                        &smil.path,
                        format!("the text file {} does not exist", p.text_path),
                    ));
                }
            } else if let Some(frag) = &p.text_id {
                let ids = ids_cache.entry(p.text_path.clone()).or_insert_with(|| {
                    archive
                        .get_string(&p.text_path)
                        .map(|t| defined_ids(&t).into_iter().collect())
                        .unwrap_or_default()
                });
                if !ids.contains(frag)
                    && reported_text.insert((p.text_path.clone(), Some(frag.clone())))
                {
                    out.push(msg(
                        "MED-005",
                        &smil.path,
                        format!("{}#{frag} does not exist", p.text_path),
                    ));
                }
            }
            if !archive.contains(&p.audio_path) && reported_audio.insert(p.audio_path.clone()) {
                out.push(msg(
                    "MED-006",
                    &smil.path,
                    format!("the audio file {} does not exist", p.audio_path),
                ));
            }
            if let Some(a) = items.iter().find(|i| i.path == p.audio_path) {
                audio_types_seen.insert(a.media_type.clone());
            }
        }
        if other_doc {
            out.push(msg(
                "MED-012",
                &smil.path,
                format!("some pars read text outside {}", ch.path),
            ));
        }
        // duration
        let want = read.pars.iter().map(|p| p.end_ms).max().unwrap_or(0);
        let declared = metas
            .iter()
            .find(|m| m.property == "media:duration" && m.refines.as_deref() == Some(sid))
            .and_then(|m| parse_clock(&m.text));
        match declared {
            None => out.push(msg(
                "MED-008",
                &opf_path,
                format!("no valid media:duration refines \"{sid}\""),
            )),
            Some(d) if d.abs_diff(want) > 1000 => out.push(msg(
                "MED-008",
                &opf_path,
                format!("media:duration of \"{sid}\" says {d} ms, the clips end at {want} ms"),
            )),
            Some(_) => {}
        }
    }
    for it in items.iter().filter(|i| i.media_type == SMIL_TYPE) {
        any = true;
        if !used_smil.contains(it.id.as_str()) {
            out.push(msg(
                "MED-010",
                &opf_path,
                format!("\"{}\" is not the overlay of any chapter", it.id),
            ));
        }
    }
    if any
        && !metas
            .iter()
            .any(|m| m.property == "media:duration" && m.refines.is_none())
    {
        out.push(msg(
            "MED-009",
            &opf_path,
            "add a media:duration for the whole publication".to_string(),
        ));
    }
    let mut kinds: Vec<_> = audio_types_seen
        .into_iter()
        .filter(|t| !CORE_AUDIO.contains(&t.split(';').next().unwrap_or("").trim()))
        .collect();
    kinds.sort();
    for t in kinds {
        out.push(msg(
            "MED-011",
            &opf_path,
            format!("{t} is not a core media type (audio/mpeg, audio/mp4, audio/ogg with Opus)"),
        ));
    }
    out
}

/// The total length the clips of `pars` add up to (convenience for callers that hold [`Par`]s).
pub fn total_ms(per_chapter: &[Vec<Par>]) -> u64 {
    per_chapter.iter().map(|p| duration_ms(p)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_overlay_narrate::{
        narrate_book, remove_narration, Audio, Options, SentenceSynth,
    };
    use crate::testutil::make_book;
    use bookx_core::book::OpenBook;

    struct Synth;
    impl SentenceSynth for Synth {
        fn synth(&mut self, t: &str) -> anyhow::Result<Audio> {
            Ok(Audio {
                rate: 8000,
                samples: vec![0; t.len() * 80],
            })
        }
    }

    fn narrated() -> OpenBook {
        let mut b = make_book(&["<p>One. Two.</p>"]);
        narrate_book(
            &mut b,
            &mut Synth,
            &Options::default(),
            &|| false,
            &mut |_| {},
        )
        .unwrap();
        b
    }

    fn ids(b: &OpenBook) -> Vec<String> {
        let mut v: Vec<String> = check_overlays(&b.archive)
            .into_iter()
            .filter(|m| m.id != "MED-011")
            .map(|m| m.id)
            .collect();
        v.sort();
        v
    }

    fn smil_path(b: &OpenBook) -> String {
        b.archive
            .names()
            .iter()
            .find(|n| n.ends_with(".smil"))
            .cloned()
            .unwrap()
    }

    fn audio_path(b: &OpenBook) -> String {
        b.archive
            .names()
            .iter()
            .find(|n| n.ends_with(".wav"))
            .cloned()
            .unwrap()
    }

    #[test]
    fn a_book_without_overlays_has_no_findings() {
        let b = make_book(&["<p>x</p>"]);
        assert!(check_overlays(&b.archive).is_empty());
        assert!(!has_overlays(&b.archive));
        assert!(has_overlays(&narrated().archive));
    }

    #[test]
    fn broken_overlays_are_found() {
        let b = narrated();
        assert!(ids(&b).is_empty());

        let mut x = narrated();
        let p = smil_path(&x);
        x.archive.remove(&p);
        assert_eq!(ids(&x), vec!["MED-003"]);

        let mut x = narrated();
        let p = smil_path(&x);
        x.archive.set_string(&p, "<smil><par>");
        assert_eq!(ids(&x), vec!["MED-004"]);

        let mut x = narrated();
        let p = audio_path(&x);
        x.archive.remove(&p);
        assert_eq!(ids(&x), vec!["MED-006"]);

        let mut x = narrated();
        let p = smil_path(&x);
        let t = x
            .archive
            .get_string(&p)
            .unwrap()
            .replace("#bxs1\"", "#nope\"");
        x.archive.set_string(&p, &t);
        assert_eq!(ids(&x), vec!["MED-005"]);

        // a clip that ends before it begins and an external reference are ignored pars
        let mut x = narrated();
        let p = smil_path(&x);
        let t = x.archive.get_string(&p).unwrap().replacen(
            "clipBegin=\"0:00:00.000\"",
            "clipBegin=\"0:00:09.000\"",
            1,
        );
        x.archive.set_string(&p, &t);
        let got = ids(&x);
        assert!(got.contains(&"MED-007".to_string()), "{got:?}");

        let mut x = narrated();
        let p = smil_path(&x);
        let t = x
            .archive
            .get_string(&p)
            .unwrap()
            .replace("../audio/", "http://evil.example/");
        x.archive.set_string(&p, &t);
        assert!(ids(&x).contains(&"MED-007".to_string()));

        // wrong duration, no total, orphan smil, wrong media type, missing target
        let mut x = narrated();
        let op = x.package.opf_path.clone();
        let t = x.archive.get_string(&op).unwrap();
        let wrong = t.replacen("refines=\"#mo-", "refines=\"#xx-", 1);
        x.archive.set_string(&op, &wrong);
        assert!(ids(&x).contains(&"MED-008".to_string()));
        let no_total = t.replace(
            "<meta property=\"media:duration\">",
            "<meta property=\"media:other\">",
        );
        x.archive.set_string(&op, &no_total);
        assert_eq!(ids(&x), vec!["MED-009"]);
        let orphan = {
            let mut s = t.clone();
            s = s.replacen(" media-overlay=\"", " data-x=\"", 1);
            s
        };
        x.archive.set_string(&op, &orphan);
        assert!(ids(&x).contains(&"MED-010".to_string()));
        let ty = t.replace("application/smil+xml", "text/plain");
        x.archive.set_string(&op, &ty);
        assert_eq!(ids(&x), vec!["MED-002"]);
        let missing = t.replacen("media-overlay=\"mo-", "media-overlay=\"zz-", 1);
        x.archive.set_string(&op, &missing);
        assert!(ids(&x).contains(&"MED-001".to_string()));
    }

    #[test]
    fn validate_reports_overlay_findings_and_removal_of_a_broken_book_works() {
        let mut x = narrated();
        let p = smil_path(&x);
        x.archive.set_string(&p, "garbage");
        let rep = crate::validate::validate(&x.archive);
        assert!(rep.messages.iter().any(|m| m.id == "MED-004"));
        // removing the narration of a book whose SMIL cannot be read still cleans the package
        let r = remove_narration(&mut x).unwrap();
        assert_eq!(r.chapters, 1);
        assert!(check_overlays(&x.archive).is_empty());
    }

    #[test]
    fn totals() {
        let p = |e| {
            vec![Par {
                text_id: "a".into(),
                begin_ms: 0,
                end_ms: e,
            }]
        };
        assert_eq!(total_ms(&[p(5), p(7)]), 12);
    }
}
