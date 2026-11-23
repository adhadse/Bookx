//! End-to-end tests through the public API with fixtures assembled by hand.

use bookx_lookup::dictset::{DictionarySet, LookupHistory, WebSource};
use bookx_lookup::export::{
    AnnotationItem, ExportFormat, ExportOptions, HighlightStyle, export, export_files,
    readwise_payload,
};
use bookx_lookup::rsvp::{JumpHistory, RsvpConfig, schedule};
use bookx_lookup::search::{SearchOptions, search};
use bookx_lookup::stardict::StarDict;
use bookx_lookup::stats::{
    RecordOutcome, Session, StatsConfig, StatsStore, daily_totals, heatmap, reading_speed_wpm,
    streaks,
};
use bookx_lookup::vocab::{AnkiOptions, FlashcardSession, VocabStore, export_anki_tsv};
use chrono::{Duration, NaiveDate, TimeZone, Utc, Weekday};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "bookx-lookup-it-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// "cat" -> "a feline", "dog" -> "a canine" written as literal bytes:
/// idx = word\0 + u32be offset + u32be size.
const IFO: &str = "StarDict's dict ifo file\nversion=2.4.2\nbookname=Tiny\nwordcount=2\nidxfilesize=24\nsametypesequence=m\n";
const IDX: &[u8] = b"cat\0\x00\x00\x00\x00\x00\x00\x00\x08dog\0\x00\x00\x00\x08\x00\x00\x00\x08";
const DICT: &[u8] = b"a felinea canine";

#[test]
fn hand_written_stardict_files_are_readable() {
    let dir = tmp("golden");
    std::fs::write(dir.join("tiny.ifo"), IFO).unwrap();
    std::fs::write(dir.join("tiny.idx"), IDX).unwrap();
    std::fs::write(dir.join("tiny.dict"), DICT).unwrap();
    let d = StarDict::open(&dir).unwrap();
    assert_eq!(d.name(), "Tiny");
    assert_eq!(d.lookup("Cat").unwrap()[0].plain_text(), "a feline");
    assert_eq!(d.lookup("dog").unwrap()[0].plain_text(), "a canine");
    assert_eq!(d.suggest("", 10), vec!["cat", "dog"]);
    std::fs::remove_dir_all(dir).ok();
}

/// A dictzip file: gzip with the `RA` extra field (version 1, one chunk) around raw deflate.
fn dictzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    let deflated = enc.finish().unwrap();
    let mut crc = flate2::Crc::new();
    crc.update(data);

    let mut ra = Vec::new();
    ra.extend_from_slice(&1u16.to_le_bytes()); // version
    ra.extend_from_slice(&(58315u16).to_le_bytes()); // chunk length
    ra.extend_from_slice(&1u16.to_le_bytes()); // chunk count
    ra.extend_from_slice(&(deflated.len() as u16).to_le_bytes());
    let mut extra = Vec::new();
    extra.extend_from_slice(b"RA");
    extra.extend_from_slice(&(ra.len() as u16).to_le_bytes());
    extra.extend_from_slice(&ra);

    let mut out = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 3];
    out.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    out.extend_from_slice(&extra);
    out.extend_from_slice(&deflated);
    out.extend_from_slice(&crc.sum().to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[test]
fn dictzip_with_ra_header_is_readable() {
    let dir = tmp("dictzip");
    std::fs::write(dir.join("tiny.ifo"), IFO).unwrap();
    std::fs::write(dir.join("tiny.idx"), IDX).unwrap();
    std::fs::write(dir.join("tiny.dict.dz"), dictzip(DICT)).unwrap();
    let d = StarDict::open(dir.join("tiny.ifo")).unwrap();
    assert_eq!(d.lookup("dog").unwrap()[0].plain_text(), "a canine");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn dictionary_set_history_and_web_sources_work_together() {
    let dir = tmp("set");
    std::fs::write(dir.join("tiny.ifo"), IFO).unwrap();
    std::fs::write(dir.join("tiny.idx"), IDX).unwrap();
    std::fs::write(dir.join("tiny.dict"), DICT).unwrap();
    let mut set = DictionarySet::new();
    set.add(StarDict::open(&dir).unwrap());
    let mut history = LookupHistory::new(5);
    for w in ["Cats,", "dog", "unknownword", "cats"] {
        let r = set.lookup_all(w);
        history.record(w);
        match w {
            "unknownword" => assert!(r.results.is_empty()),
            _ => assert_eq!(r.results.len(), 1, "{w}"),
        }
    }
    assert_eq!(
        history.entries(),
        vec!["cats", "unknownword", "dog", "Cats,"]
    );
    let web = WebSource::new("Wikt", "https://en.wiktionary.org/wiki/{word}").unwrap();
    assert_eq!(
        web.url("café au lait").unwrap(),
        "https://en.wiktionary.org/wiki/caf%C3%A9%20au%20lait"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn lookup_to_vocabulary_to_anki_and_review() {
    let now = Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap();
    let mut v = VocabStore::new();
    v.add("ephemeral", Some("An ephemeral joy."), Some("b1"), now)
        .unwrap();
    v.set_definition("ephemeral", "short-lived").unwrap();
    let tsv = export_anki_tsv(
        &v,
        &AnkiOptions {
            headers: false,
            tag: "bookx".into(),
        },
    );
    let cols: Vec<&str> = tsv.trim_end().split('\t').collect();
    assert_eq!(cols.len(), 3);
    assert_eq!(cols[0], "ephemeral");
    assert!(cols[1].contains("<b>ephemeral</b>"));
    assert_eq!(cols[2], "bookx b1");

    let mut s = FlashcardSession::new(&v, now, 10);
    assert_eq!(s.current(), Some("ephemeral"));
    s.answer(&mut v, 5, now).unwrap();
    assert!(s.is_finished());
    assert_eq!(v.due(now).len(), 0);
    assert_eq!(v.due(now + Duration::days(1)).len(), 1);

    let dir = tmp("vocab");
    let p = dir.join("vocab.json");
    v.save(&p).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(raw["entries"][0]["word"], "ephemeral");
    assert_eq!(VocabStore::load(&p).unwrap(), v);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn reading_week_statistics() {
    let mut st = StatsStore::new(StatsConfig {
        day_start_minutes: 240,
        ..Default::default()
    })
    .unwrap();
    let mut outcomes = Vec::new();
    for day in 0..7 {
        // 00:30 each night, belongs to the previous reading day
        let start = Utc.with_ymd_and_hms(2025, 6, 2 + day, 0, 30, 0).unwrap();
        let s = Session {
            book_id: "b".into(),
            start,
            end: start + Duration::minutes(30),
            pages: 10,
            words: 3000,
            characters: 18000,
            start_pos: 0.0,
            end_pos: 0.1,
        };
        outcomes.push(st.record(s).unwrap());
    }
    assert!(outcomes.iter().all(|o| *o == RecordOutcome::Recorded));
    let daily = st.daily(&Utc);
    assert_eq!(daily.len(), 7);
    assert_eq!(
        *daily.keys().next().unwrap(),
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap()
    );
    assert_eq!(daily, daily_totals(st.sessions(), &st.config, &Utc));
    let today = NaiveDate::from_ymd_opt(2025, 6, 7).unwrap();
    assert_eq!(streaks(&daily, today, 60).current, 7);
    let wpm = reading_speed_wpm(st.sessions()).unwrap();
    assert!((wpm - 100.0).abs() < 1e-9);
    let h = heatmap(&daily, 2025, Weekday::Mon).unwrap();
    assert_eq!(h.columns.len(), 53);
    assert_eq!(
        h.columns
            .iter()
            .flatten()
            .flatten()
            .filter(|s| **s > 0)
            .count(),
        7
    );

    let dir = tmp("stats");
    let p = dir.join("stats.json");
    st.save(&p).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(raw["sessions"].as_array().unwrap().len(), 7);
    assert_eq!(raw["config"]["day_start_minutes"], 240);
    assert_eq!(StatsStore::load(&p).unwrap(), st);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn search_with_cancel_from_another_thread() {
    let chapters: Vec<String> = (0..50)
        .map(|i| format!("chapter {i}: the quick brown fox ").repeat(2000))
        .collect();
    let cancel = AtomicBool::new(false);
    let opts = SearchOptions {
        max_results: 1_000_000,
        ..Default::default()
    };
    std::thread::scope(|s| {
        let h = s.spawn(|| search(&chapters, "fox", &opts, Some(&cancel)).unwrap());
        cancel.store(true, Ordering::Relaxed);
        let out = h.join().unwrap();
        // either finished before the flag was seen or was cancelled; never a partial panic
        assert!(out.cancelled || out.hits.len() == 100_000);
    });
    let all = search(&chapters[..2], "FOX", &SearchOptions::default(), None).unwrap();
    assert_eq!(all.hits.len(), 500);
    assert!(all.truncated);
    assert!(all.hits[0].snippet.contains("fox"));
}

#[test]
fn rsvp_and_jump_history_smoke() {
    let s = schedule("Hello, world. This is RSVP!", &RsvpConfig::default()).unwrap();
    assert_eq!(s.len(), 5);
    assert!(s[1].duration_ms > s[0].duration_ms);
    let mut h = JumpHistory::new(4);
    h.jump(0usize, 100);
    assert_eq!(h.back(120), Some(&0));
    assert_eq!(h.forward(0), Some(&120));
}

#[test]
fn annotations_export_all_formats_and_files() {
    let created = Utc.with_ymd_and_hms(2025, 2, 3, 4, 5, 6).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap();
    let items = vec![AnnotationItem {
        book_title: "A <Book>".into(),
        book_author: Some("Zoë Ünïcode".into()),
        chapter: Some("One".into()),
        text: "Text with `ticks` and <tags> & ampersand".into(),
        note: Some("note ☕".into()),
        style: HighlightStyle::Highlight,
        color: Some("#ff0".into()),
        created,
        location: Some("7".into()),
    }];
    let o = ExportOptions::default();
    for f in [
        ExportFormat::Markdown,
        ExportFormat::Text,
        ExportFormat::Html,
        ExportFormat::Json,
        ExportFormat::KindleClippings,
        ExportFormat::Obsidian,
    ] {
        let out = export(&items, f, &o, now);
        assert!(out.contains("Zoë Ünïcode"), "{f:?}");
    }
    let html = export(&items, ExportFormat::Html, &o, now);
    assert!(html.contains("&lt;tags&gt; &amp; ampersand"));
    assert!(!html.contains("<tags>"));
    let files = export_files(
        &items,
        ExportFormat::Markdown,
        &o,
        "{title}-{date:%Y%m%d}",
        now,
    )
    .unwrap();
    assert_eq!(files[0].name, "A _Book_-20250301.md");
    let rw = readwise_payload(&items, &o);
    assert_eq!(rw["highlights"][0]["location"], 7);
}
