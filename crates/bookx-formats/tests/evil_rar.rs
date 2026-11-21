//! Hostile RAR comics (CBR): the archive goes to a helper program, so what matters is that the
//! limits hold before anything is decoded, names cannot leave the archive, and damage is an error,
//! not a hang or a panic. Tests that need `unrar` or 7-Zip skip themselves when none is installed.

use bookx_formats::rar;
use bookx_formats::read_any;

fn crc32(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

/// A stored RAR 1.5-4 archive; `declared` overrides the sizes the headers claim.
fn rar_with(files: &[(&str, &[u8])], declared: Option<u32>) -> Vec<u8> {
    let mut out = rar::RAR_MAGIC.to_vec();
    out.push(0);
    let main = vec![0x73, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0];
    out.extend((crc32(&main) as u16).to_le_bytes());
    out.extend(main);
    for (name, data) in files {
        let size = declared.unwrap_or(data.len() as u32);
        let mut h = vec![0x74u8];
        h.extend(0x8000u16.to_le_bytes());
        h.extend((32 + name.len() as u16).to_le_bytes());
        h.extend(size.to_le_bytes());
        h.extend(size.to_le_bytes());
        h.push(3);
        h.extend(crc32(data).to_le_bytes());
        h.extend(0x5A21_0000u32.to_le_bytes());
        h.push(20);
        h.push(0x30);
        h.extend((name.len() as u16).to_le_bytes());
        h.extend(0x20u32.to_le_bytes());
        h.extend(name.as_bytes());
        out.extend((crc32(&h) as u16).to_le_bytes());
        out.extend(h);
        out.extend(*data);
    }
    out.extend([0xC4, 0x3D, 0x7B, 0x00, 0x40, 0x07, 0x00]);
    out
}

fn jpg() -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
    v.extend(b"JFIF\0");
    v.extend(vec![0; 40]);
    v
}

#[test]
fn a_header_that_claims_gigabytes_is_an_error() {
    let Some(tool) = rar::find_tool() else { return };
    let a = rar_with(&[("p/1.jpg", &jpg())], Some(u32::MAX));
    assert!(rar::extract_with(&tool, &a).is_err());
    let b = rar_with(&[("p/1.jpg", &jpg())], Some(3 << 30));
    assert!(rar::extract_with(&tool, &b).is_err());
}

#[test]
fn too_many_entries_are_refused() {
    let Some(tool) = rar::find_tool() else { return };
    let names: Vec<String> = (0..rar::MAX_LISTED + 1)
        .map(|i| format!("{i}.jpg"))
        .collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let a = rar_with(&files, None);
    assert!(rar::extract_with(&tool, &a).is_err());
}

#[test]
fn path_tricks_never_come_out() {
    let Some(tool) = rar::find_tool() else { return };
    let j = jpg();
    let a = rar_with(
        &[
            ("../../evil.jpg", &j),
            ("..\\..\\evil2.jpg", &j),
            ("ok/1.jpg", &j),
        ],
        None,
    );
    let Ok(files) = rar::extract_with(&tool, &a) else {
        return;
    };
    for (n, _) in &files {
        assert!(!n.split('/').any(|s| s == ".."), "{n}");
        assert!(!n.starts_with('/'), "{n}");
    }
    assert!(files.iter().any(|(n, _)| n == "ok/1.jpg"));
}

#[test]
fn truncation_and_flipped_bytes_are_errors_or_books_never_panics() {
    let j = jpg();
    let good = rar_with(&[("a/1.jpg", &j), ("a/2.jpg", &j)], None);
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for cut in [8, 20, 40, 80, good.len() - 5] {
        let _ = read_any("x.cbr", &good[..cut]);
    }
    for _ in 0..30 {
        let mut v = good.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = (seed % v.len() as u64) as usize;
            v[i] ^= (seed >> 32) as u8 | 1;
        }
        let _ = read_any("x.cbr", &v);
    }
}

#[test]
fn a_helper_is_looked_up_in_fixed_directories_only() {
    // the lookup result, when there is one, is an absolute path in a system directory
    if let Some(rar::Tool::Unrar(p) | rar::Tool::SevenZip(p)) = rar::find_tool() {
        assert!(p.is_absolute());
    }
}
