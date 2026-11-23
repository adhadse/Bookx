//! A folder, a zip or a single file into an [`AudioBook`]: parts in natural order, chapters,
//! the book's tags and its cover.
//!
//! Zip entries are untrusted: names are checked ([`safe_rel`]), sizes are counted while unpacking
//! (the sizes in the zip are not believed), only audio files and pictures are unpacked.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::format::has_audio_extension;
use crate::limits::*;
use crate::model::{AudioBook, Part};
use crate::probe::{ChapterMark, Cover, FileInfo, probe_file};
use crate::text::{natural_key, title_from_file_name};

/// The result of a scan.
#[derive(Debug, Clone)]
pub struct Scanned {
    pub book: AudioBook,
    pub cover: Option<Cover>,
    /// Absolute paths of the parts, in order.
    pub files: Vec<PathBuf>,
    /// The folder that part paths in `book` are relative to (the file's folder for a single file).
    pub root: PathBuf,
    /// Files that were skipped, and why (not audio, unreadable).
    pub skipped: Vec<String>,
}

fn is_image_name(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "gif"
        )
    })
}

fn hidden(name: &str) -> bool {
    name.starts_with('.') || name == "__MACOSX"
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>, images: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_PARTS {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if hidden(&name) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_symlink() {
            continue; // a link could lead anywhere
        }
        if ft.is_dir() {
            walk(&p, depth + 1, out, images);
        } else if has_audio_extension(&p) {
            if out.len() < MAX_PARTS {
                out.push(p);
            }
        } else if is_image_name(&p) && images.len() < 64 {
            images.push(p);
        }
    }
}

/// Audio files (natural order of their relative path) and pictures under `dir`.
pub fn list_audio(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut audio, mut images) = (Vec::new(), Vec::new());
    walk(dir, 0, &mut audio, &mut images);
    audio.sort_by_key(|p| natural_key(&p.strip_prefix(dir).unwrap_or(p).to_string_lossy()));
    images.sort_by_key(|p| natural_key(&p.to_string_lossy()));
    (audio, images)
}

/// Is this path an audiobook that can be imported: an audio file, or a folder that has some?
pub fn looks_like_audiobook(path: &Path) -> bool {
    if path.is_file() {
        let zip = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        return has_audio_extension(path) || (zip && zip_has_audio(path));
    }
    if path.is_dir() {
        let (a, _) = list_audio(path);
        return !a.is_empty();
    }
    false
}

fn zip_has_audio(path: &Path) -> bool {
    let Ok(f) = File::open(path) else {
        return false;
    };
    let Ok(mut z) = zip::ZipArchive::new(f) else {
        return false;
    };
    (0..z.len().min(MAX_ZIP_ENTRIES)).any(|i| {
        z.by_index_raw(i)
            .ok()
            .and_then(|e| safe_rel(&e.name().ok()?))
            .is_some_and(|p| has_audio_extension(&p))
    })
}

/// A zip entry name as a safe relative path, or `None` (absolute, `..`, hidden, backslashes, drive
/// letters, NUL, too long).
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    if name.is_empty()
        || name.len() > 300
        || name.contains('\0')
        || name.contains('\\')
        || name.starts_with('/')
    {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in name.split('/') {
        if comp.is_empty() {
            continue;
        }
        if comp == ".." || comp == "." || hidden(comp) || comp.contains(':') {
            return None;
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Unpacks the audio files and pictures of `zip_path` into `dest` and returns their number.
pub fn unpack_zip(zip_path: &Path, dest: &Path) -> Result<usize> {
    let f = File::open(zip_path).with_context(|| format!("opening {}", zip_path.display()))?;
    let mut z = zip::ZipArchive::new(f).context("not a readable zip file")?;
    if z.len() > MAX_ZIP_ENTRIES {
        bail!("the zip has too many entries ({})", z.len());
    }
    fs::create_dir_all(dest)?;
    let (mut total, mut n) = (0u64, 0usize);
    for i in 0..z.len() {
        let mut e = match z.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if e.is_dir() || e.is_symlink() {
            continue;
        }
        let Some(rel) = e.name().ok().and_then(|n| safe_rel(&n)) else {
            continue;
        };
        if !(has_audio_extension(&rel) || is_image_name(&rel)) {
            continue;
        }
        let target = dest.join(&rel);
        if !target.starts_with(dest) {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        // the declared size is not trusted: stop at the budget whatever the entry says
        let budget = MAX_ZIP_TOTAL.saturating_sub(total);
        let copied = std::io::copy(&mut (&mut e).take(budget + 1), &mut out)?;
        out.flush()?;
        total += copied;
        if total > MAX_ZIP_TOTAL {
            let _ = fs::remove_file(&target);
            bail!("the zip unpacks to more than {} GiB", MAX_ZIP_TOTAL >> 30);
        }
        n += 1;
    }
    Ok(n)
}

/// Scans a zip: unpacks it into `dest` (which must not be inside the zip's own folder logic) and scans that.
pub fn scan_zip(zip_path: &Path, dest: &Path) -> Result<Scanned> {
    if unpack_zip(zip_path, dest)? == 0 {
        bail!("no audio files in {}", zip_path.display());
    }
    let stem = zip_path.file_stem().unwrap_or_default().to_string_lossy();
    scan_folder_as(dest, title_from_file_name(&stem))
}

/// A single audio file (an `.m4b`, or one long `.mp3`).
pub fn scan_file(path: &Path) -> Result<Scanned> {
    let info = probe_file(path)?;
    let root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    build(
        &root,
        vec![(path.to_path_buf(), name, info)],
        None,
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    )
}

/// A folder of parts (any depth up to [`MAX_DEPTH`]).
pub fn scan_folder(dir: &Path) -> Result<Scanned> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    scan_folder_as(dir, title_from_file_name(&name))
}

/// Like [`scan_folder`]; `fallback_title` names the book when the tags do not.
pub fn scan_folder_as(dir: &Path, fallback_title: String) -> Result<Scanned> {
    let (files, images) = list_audio(dir);
    if files.is_empty() {
        bail!("no audio files in {}", dir.display());
    }
    let mut parts = Vec::with_capacity(files.len());
    let mut skipped = Vec::new();
    for p in files {
        let rel = p
            .strip_prefix(dir)
            .unwrap_or(&p)
            .to_string_lossy()
            .into_owned();
        match probe_file(&p) {
            Ok(info) => parts.push((p, rel, info)),
            Err(e) => skipped.push(format!("{rel}: {e}")),
        }
    }
    if parts.is_empty() {
        bail!("none of the audio files in {} could be read", dir.display());
    }
    let image = images.into_iter().find_map(|p| {
        let md = fs::metadata(&p).ok()?;
        if md.len() as usize > MAX_COVER_BYTES {
            return None;
        }
        Cover::from_bytes(&fs::read(&p).ok()?, "")
    });
    let mut s = build(dir, parts, image, fallback_title)?;
    s.skipped = skipped;
    Ok(s)
}

fn build(
    root: &Path,
    parts: Vec<(PathBuf, String, FileInfo)>,
    folder_cover: Option<Cover>,
    fallback_title: String,
) -> Result<Scanned> {
    let first = &parts[0].2;
    let single = parts.len() == 1;
    let mut title = if single {
        first.tags.book_title().to_string()
    } else {
        // parts share an album; without one the folder names the book
        first.tags.album.clone()
    };
    if title.is_empty() && single {
        title = first.tags.title.clone();
    }
    if title.is_empty() {
        title = fallback_title;
    }
    let authors = if first.tags.authors.is_empty() {
        Vec::new()
    } else {
        first.tags.authors.clone()
    };
    let titles_differ = parts
        .iter()
        .map(|p| p.2.tags.title.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
        > 1;
    let book = AudioBook {
        title,
        authors,
        narrator: first.tags.narrator_or_composer().to_string(),
        series: first.tags.series.clone(),
        series_index: first.tags.series_index,
        language: first.tags.language.clone(),
        description: first.tags.comment.clone(),
        ..Default::default()
    };
    let cover = parts
        .iter()
        .find_map(|p| p.2.cover.clone())
        .or(folder_cover);
    let mut files = Vec::new();
    let mut with_marks = Vec::new();
    for (path, rel, info) in parts {
        let stem = Path::new(&rel)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let part_title = if !info.tags.title.is_empty() && (titles_differ || single) {
            info.tags.title.clone()
        } else {
            title_from_file_name(&stem)
        };
        files.push(path);
        let marks: Vec<ChapterMark> = info.chapters.clone();
        with_marks.push((
            Part {
                path: rel,
                title: part_title,
                duration_ms: info.duration_ms,
                format: info.format,
            },
            marks,
        ));
    }
    Ok(Scanned {
        book: AudioBook::assemble(book, with_marks),
        cover,
        files,
        root: root.to_path_buf(),
        skipped: Vec::new(),
    })
}

/// The manifest written next to an imported audiobook, so opening it does not need a rescan.
pub fn save_manifest(book: &AudioBook, path: &Path) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(book)?)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_manifest(path: &Path) -> Option<AudioBook> {
    let md = fs::metadata(path).ok()?;
    if md.len() > 64 * 1024 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testgen::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-audio-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(dir: &Path, name: &str, data: &[u8]) {
        let p = dir.join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }

    fn zip_of(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (n, d) in entries {
                z.start_file(
                    *n,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
                z.write_all(d).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn a_folder_of_numbered_wavs_becomes_one_book_in_order() {
        let d = tmp().join("The Road Home");
        for n in [10, 2, 1, 3] {
            write(
                &d,
                &format!("Chapter {n}.wav"),
                &wav_silence(8000, 1000 * n as u64),
            );
        }
        write(&d, "notes.txt", b"hello");
        write(&d, "cover.png", &png_1x1());
        let s = scan_folder(&d).unwrap();
        assert_eq!(s.book.title, "The Road Home");
        let names: Vec<_> = s.book.parts.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Chapter 1.wav",
                "Chapter 2.wav",
                "Chapter 3.wav",
                "Chapter 10.wav"
            ]
        );
        assert_eq!(s.book.total_ms, 16_000);
        assert_eq!(s.book.chapters.len(), 4);
        assert_eq!(s.book.chapters[0].title, "Chapter 1");
        assert_eq!(s.cover.unwrap().ext, "png");
        assert_eq!(s.files.len(), 4);
    }

    #[test]
    fn tags_decide_title_author_and_narrator() {
        let d = tmp().join("whatever");
        for n in 1..=3u32 {
            let mut f = id3_tag(&[
                id3_text("TALB", "Real Title"),
                id3_text("TPE1", "Ann Author"),
                id3_text("TCOM", "Nora Narrator"),
                id3_text("TIT2", &format!("Part {n}")),
            ]);
            f.extend_from_slice(
                &std::fs::read(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tiny_notag.mp3"),
                )
                .unwrap(),
            );
            write(&d, &format!("{n:02}.mp3"), &f);
        }
        let s = scan_folder(&d).unwrap();
        assert_eq!(s.book.title, "Real Title");
        assert_eq!(s.book.authors, vec!["Ann Author"]);
        assert_eq!(s.book.narrator, "Nora Narrator");
        assert_eq!(s.book.parts[1].title, "Part 2");
        assert_eq!(s.book.parts.len(), 3);
    }

    #[test]
    fn a_single_m4b_gets_its_embedded_chapters_and_cover() {
        let d = tmp();
        let p = d.join("Big Book.m4b");
        fs::write(
            &p,
            mp4_file(&Mp4Spec {
                duration_ms: 100_000,
                chapters: vec![("Start".into(), 0), ("Middle".into(), 50_000)],
                title: "Big Book".into(),
                artist: "Ann".into(),
                narrator: "Nora".into(),
                series: "Saga".into(),
                cover: Some(png_1x1()),
                ..Default::default()
            }),
        )
        .unwrap();
        let s = scan_file(&p).unwrap();
        assert_eq!(s.book.title, "Big Book");
        assert_eq!(s.book.narrator, "Nora");
        assert_eq!(s.book.series, "Saga");
        assert_eq!(s.book.chapters.len(), 2);
        assert_eq!(s.book.chapters[1].start_ms, 50_000);
        assert!(s.cover.is_some());
        assert_eq!(s.book.parts[0].path, "Big Book.m4b");
    }

    #[test]
    fn unreadable_files_are_skipped_not_fatal() {
        let d = tmp();
        write(&d, "1.wav", &wav_silence(8000, 500));
        write(&d, "2.mp3", b"not really mp3 data at all, just text");
        let s = scan_folder(&d).unwrap();
        assert_eq!(s.book.parts.len(), 1);
        assert_eq!(s.skipped.len(), 1);
        assert!(scan_folder(&tmp()).is_err());
        let only_bad = tmp();
        write(&only_bad, "x.mp3", b"nope");
        assert!(scan_folder(&only_bad).is_err());
    }

    #[test]
    fn zip_entries_are_checked_and_unpacked() {
        let d = tmp();
        let z = d.join("book.zip");
        fs::write(
            &z,
            zip_of(&[
                ("My Book/01.wav", wav_silence(8000, 1000)),
                ("My Book/02.wav", wav_silence(8000, 2000)),
                ("../escape.wav", wav_silence(8000, 100)),
                ("/abs.wav", wav_silence(8000, 100)),
                ("C:/drive.wav", wav_silence(8000, 100)),
                ("a\\b.wav", wav_silence(8000, 100)),
                ("__MACOSX/My Book/._01.wav", vec![0; 10]),
                ("My Book/readme.txt", b"x".to_vec()),
                ("My Book/folder.jpg", png_1x1()),
            ]),
        )
        .unwrap();
        assert!(looks_like_audiobook(&z));
        let dest = d.join("out");
        let s = scan_zip(&z, &dest).unwrap();
        assert_eq!(s.book.parts.len(), 2);
        assert_eq!(s.book.total_ms, 3000);
        assert!(!d.join("escape.wav").exists());
        assert!(!dest.join("readme.txt").exists());
        assert!(
            s.cover.is_some(),
            "folder.jpg holds a PNG: the bytes decide, not the name"
        );
        let all: Vec<_> = walkdir(&dest);
        assert!(all.iter().all(|p| p.starts_with(&dest)), "{all:?}");
    }

    fn walkdir(d: &Path) -> Vec<PathBuf> {
        let mut v = Vec::new();
        for e in fs::read_dir(d).unwrap().flatten() {
            v.push(e.path());
            if e.path().is_dir() {
                v.extend(walkdir(&e.path()));
            }
        }
        v
    }

    #[test]
    fn zips_without_audio_or_with_junk_are_refused() {
        let d = tmp();
        let z = d.join("none.zip");
        fs::write(&z, zip_of(&[("a.txt", b"x".to_vec())])).unwrap();
        assert!(!looks_like_audiobook(&z));
        assert!(scan_zip(&z, &d.join("o")).is_err());
        let bad = d.join("bad.zip");
        fs::write(&bad, b"PK\x03\x04 garbage").unwrap();
        assert!(scan_zip(&bad, &d.join("o2")).is_err());
        assert!(!looks_like_audiobook(&bad));
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_rel("a/b.mp3"), Some(PathBuf::from("a/b.mp3")));
        assert_eq!(safe_rel("a//b.mp3"), Some(PathBuf::from("a/b.mp3")));
        for bad in [
            "",
            "/a",
            "../a",
            "a/../b",
            "a/./b",
            ".hidden/a.mp3",
            "a/.b.mp3",
            "C:/a",
            "a\\b",
            "a\0b",
            "__MACOSX/a",
        ] {
            assert_eq!(safe_rel(bad), None, "{bad:?}");
        }
        assert_eq!(safe_rel(&"a/".repeat(200)), None, "too long");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let d = tmp();
        let other = tmp();
        write(&other, "secret.wav", &wav_silence(8000, 100));
        write(&d, "1.wav", &wav_silence(8000, 100));
        std::os::unix::fs::symlink(&other, d.join("link")).unwrap();
        std::os::unix::fs::symlink(other.join("secret.wav"), d.join("2.wav")).unwrap();
        let s = scan_folder(&d).unwrap();
        assert_eq!(s.book.parts.len(), 1);
    }

    #[test]
    fn manifest_round_trip() {
        let d = tmp();
        write(&d, "1.wav", &wav_silence(8000, 500));
        let s = scan_folder(&d).unwrap();
        let m = d.join("m.json");
        save_manifest(&s.book, &m).unwrap();
        assert_eq!(load_manifest(&m).unwrap(), s.book);
        fs::write(&m, b"{broken").unwrap();
        assert!(load_manifest(&m).is_none());
        assert!(load_manifest(&d.join("none.json")).is_none());
    }
}
