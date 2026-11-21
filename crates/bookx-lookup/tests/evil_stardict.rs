//! Hostile StarDict dictionaries: errors, never panics, hangs or reads beyond the data.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookx_lookup::stardict::{decode_fields, parse_ifo, parse_index, strip_markup, StarDict};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "bookx-evil-sd-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn ifo(words: usize, extra: &str) -> String {
    format!("StarDict's dict ifo file\nversion=2.4.2\nbookname=Evil\nwordcount={words}\nidxfilesize=0\n{extra}")
}

#[test]
fn random_bytes_never_panic_the_parsers() {
    for seed in 1..80u64 {
        let b = noise(seed, (seed as usize * 53) % 4000);
        let _ = parse_index(&b, 32);
        let _ = parse_index(&b, 64);
        let _ = decode_fields(&b, Some("mh"));
        let _ = decode_fields(&b, None);
        let _ = parse_ifo(&String::from_utf8_lossy(&b));
        let _ = strip_markup(&String::from_utf8_lossy(&b));
    }
}

#[test]
fn ifo_with_absurd_values_is_an_error_or_harmless() {
    for t in [
        ifo(usize::MAX, ""),
        "StarDict's dict ifo file\nwordcount=-1\n".to_string(),
        "StarDict's dict ifo file\nwordcount=99999999999999999999999\n".to_string(),
        "StarDict's dict ifo file\nidxoffsetbits=7\nwordcount=1\n".to_string(),
        "\u{feff}StarDict's dict ifo file\n".to_string(),
        "x".repeat(1 << 20),
    ] {
        let _ = parse_ifo(&t);
    }
}

#[test]
fn an_entry_whose_offset_and_size_point_past_the_data_is_an_error() {
    let d = tmp("past");
    let mut idx = b"word\0".to_vec();
    idx.extend_from_slice(&0xffff_fff0u32.to_be_bytes()); // offset
    idx.extend_from_slice(&0xffff_ffffu32.to_be_bytes()); // size
    std::fs::write(d.join("d.idx"), &idx).unwrap();
    std::fs::write(d.join("d.dict"), b"tiny").unwrap();
    let text = format!(
        "StarDict's dict ifo file\nversion=2.4.2\nbookname=Evil\nwordcount=1\nidxfilesize={}\nsametypesequence=m\n",
        idx.len()
    );
    std::fs::write(d.join("d.ifo"), text).unwrap();
    let t = Instant::now();
    if let Ok(sd) = StarDict::open(d.join("d.ifo")) {
        assert!(sd.lookup("word").is_err() || sd.lookup("word").unwrap().is_empty());
    }
    assert!(t.elapsed() < Duration::from_secs(2));
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_synonym_file_pointing_at_nothing_is_refused() {
    let d = tmp("syn");
    let mut idx = b"a\0".to_vec();
    idx.extend_from_slice(&0u32.to_be_bytes());
    idx.extend_from_slice(&1u32.to_be_bytes());
    std::fs::write(d.join("d.idx"), &idx).unwrap();
    std::fs::write(d.join("d.dict"), b"x").unwrap();
    std::fs::write(
        d.join("d.ifo"),
        format!(
            "StarDict's dict ifo file\nversion=2.4.2\nbookname=E\nwordcount=1\nidxfilesize={}\nsametypesequence=m\nsynwordcount=1\n",
            idx.len()
        ),
    )
    .unwrap();
    let mut syn = b"b\0".to_vec();
    syn.extend_from_slice(&999u32.to_be_bytes());
    std::fs::write(d.join("d.syn"), syn).unwrap();
    assert!(StarDict::open(d.join("d.ifo")).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_folder_with_no_dictionary_and_a_missing_data_file_are_errors() {
    let d = tmp("none");
    assert!(StarDict::open(&d).is_err());
    std::fs::write(d.join("d.ifo"), ifo(0, "")).unwrap();
    assert!(StarDict::open(d.join("d.ifo")).is_err());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn markup_with_deep_or_unclosed_tags_is_stripped_in_bounded_time() {
    let t = Instant::now();
    let s = format!("{}text{}", "<b>".repeat(100_000), "<".repeat(100_000));
    let _ = strip_markup(&s);
    assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
}
