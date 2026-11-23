//! Real RAR archives (CBR) through a helper program.
//!
//! There is no pure-Rust RAR decoder with a licence Bookx can ship, so the archive is handed to
//! `unrar` or 7-Zip when one is installed. Rules (docs/security.md, no shell, no `PATH` lookup):
//!
//! * the program is looked up in fixed system directories only and started with an argument
//!   vector, never through a shell; nothing from the archive reaches the command line;
//! * the archive is listed first: entry count, each size and the total are checked before a
//!   byte is decoded, and only the listing's own sizes are trusted for the split of the output;
//! * the contents never touch the disk: the helper writes every file to its standard output,
//!   one after the other, and Bookx cuts that stream by the listed sizes and keeps only the
//!   comic's pictures and `ComicInfo.xml` (so a hostile name cannot name a path);
//! * the output is read with a hard limit, the helper gets a time limit and is killed after it.
//!
//! Without a helper the reader says so and names what to install.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::common::{MAX_ENTRY, MAX_TOTAL};
use crate::helper::{DIRS, TempFile, banner, find_in, run};

/// RAR 1.5-4 and 5 signatures.
pub const RAR_MAGIC: &[u8] = b"Rar!\x1a\x07";

/// Most entries listed in an archive Bookx will unpack.
pub const MAX_LISTED: usize = 20_000;
/// The helper must finish within this time.
const TIME_LIMIT: Duration = Duration::from_secs(120);
/// A program that can unpack RAR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tool {
    Unrar(PathBuf),
    SevenZip(PathBuf),
}

impl Tool {
    fn path(&self) -> &Path {
        match self {
            Tool::Unrar(p) | Tool::SevenZip(p) => p,
        }
    }
}

/// The helper to use: `unrar` (checked to be the original, not `unrar-free`), else 7-Zip.
pub fn find_tool() -> Option<Tool> {
    find_tool_in(DIRS)
}

fn find_tool_in(dirs: &[&str]) -> Option<Tool> {
    if let Some(p) = find_in(dirs, "unrar") {
        if banner(&p, &[]).contains("UNRAR") {
            return Some(Tool::Unrar(p));
        }
    }
    for n in ["7zz", "7z"] {
        if let Some(p) = find_in(dirs, n) {
            if banner(&p, &[]).contains("7-Zip") && lists_rar(&banner(&p, &["i"])) {
                return Some(Tool::SevenZip(p));
            }
        }
    }
    None
}

/// Whether the `7z i` format listing includes RAR (7-Zip 26 builds without it, and then cannot open any RAR).
fn lists_rar(formats: &str) -> bool {
    formats.lines().any(|l| {
        l.split_whitespace()
            .any(|w| w.eq_ignore_ascii_case("rar") || w.eq_ignore_ascii_case("rar5"))
    })
}

/// A listed file of the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub size: u64,
}

/// Parses the technical listing of `unrar lt`.
pub fn parse_unrar_list(text: &str) -> Vec<Listed> {
    let mut out = Vec::new();
    let (mut name, mut kind, mut size): (Option<String>, String, u64) = (None, String::new(), 0);
    let mut flush = |name: &mut Option<String>, kind: &mut String, size: &mut u64| {
        if let Some(n) = name.take() {
            if kind.eq_ignore_ascii_case("file") {
                out.push(Listed {
                    name: n,
                    size: *size,
                });
            }
        }
        kind.clear();
        *size = 0;
    };
    for line in text.lines() {
        let line = line.trim_start();
        if let Some(n) = line.strip_prefix("Name: ") {
            flush(&mut name, &mut kind, &mut size);
            name = Some(n.to_string());
        } else if let Some(t) = line.strip_prefix("Type: ") {
            kind = t.trim().to_string();
        } else if let Some(s) = line.strip_prefix("Size: ") {
            size = s.trim().parse().unwrap_or(0);
        }
    }
    flush(&mut name, &mut kind, &mut size);
    out
}

/// Parses `7z l -slt -ba` (one block of `Key = Value` lines per entry, blank line between).
pub fn parse_7z_list(text: &str) -> Vec<Listed> {
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let (mut name, mut size, mut dir) = (None, 0u64, false);
        for line in block.lines() {
            if let Some(n) = line.strip_prefix("Path = ") {
                name = Some(n.to_string());
            } else if let Some(s) = line.strip_prefix("Size = ") {
                size = s.trim().parse().unwrap_or(0);
            } else if let Some(f) = line.strip_prefix("Folder = ") {
                dir |= f.trim() == "+";
            } else if let Some(a) = line.strip_prefix("Attributes = ") {
                dir |= a.trim_start().starts_with('D');
            }
        }
        if let (Some(name), false) = (name, dir) {
            out.push(Listed { name, size });
        }
    }
    out
}

/// Is this entry something a comic uses?
fn wanted(name: &str) -> bool {
    let file = name.rsplit(['/', '\\']).next().unwrap_or("");
    file.eq_ignore_ascii_case("ComicInfo.xml")
        || crate::comic::is_page_name(&name.replace('\\', "/"))
}

/// Cuts the helper's stream by the listed sizes and keeps the wanted entries.
pub fn split_stream(listed: &[Listed], stream: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let total = listed.iter().fold(0u64, |a, l| a.saturating_add(l.size));
    if total != stream.len() as u64 {
        bail!("the RAR archive does not match its listing");
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    for l in listed {
        let end = pos + l.size as usize;
        if wanted(&l.name) {
            let name = l.name.replace('\\', "/");
            if name.split('/').any(|s| s == "..") {
                pos = end;
                continue;
            }
            out.push((name, stream[pos..end].to_vec()));
        }
        pos = end;
    }
    Ok(out)
}

/// Checks the listing against the limits.
pub fn check_listing(listed: &[Listed]) -> Result<()> {
    if listed.is_empty() {
        bail!("the RAR archive is empty");
    }
    if listed.len() > MAX_LISTED {
        bail!("the RAR archive has too many entries");
    }
    let mut total = 0u64;
    for l in listed {
        if l.size > MAX_ENTRY {
            bail!("`{}` is too large", l.name);
        }
        total = total.saturating_add(l.size);
    }
    if total > MAX_TOTAL {
        bail!("the RAR archive unpacks to more than Bookx accepts");
    }
    Ok(())
}

/// The files of a RAR archive that a comic uses (`ComicInfo.xml` and pictures).
pub fn extract_with(tool: &Tool, bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    if !bytes.starts_with(RAR_MAGIC) {
        bail!("not a RAR archive");
    }
    let tmp = TempFile::create(bytes, "rar")?;
    let arc = tmp.0.as_os_str();
    use std::ffi::OsStr;
    let listing_limit = 64 << 20;
    let (listed, stream_args): (Vec<Listed>, Vec<&OsStr>) = match tool {
        Tool::Unrar(p) => {
            let text = run(
                p,
                &[OsStr::new("lt"), OsStr::new("-p-"), OsStr::new("--"), arc],
                listing_limit,
                TIME_LIMIT,
            )?;
            (
                parse_unrar_list(&String::from_utf8_lossy(&text)),
                vec![
                    OsStr::new("p"),
                    OsStr::new("-inul"),
                    OsStr::new("-p-"),
                    OsStr::new("--"),
                    arc,
                ],
            )
        }
        Tool::SevenZip(p) => {
            let text = run(
                p,
                &[
                    OsStr::new("l"),
                    OsStr::new("-slt"),
                    OsStr::new("-ba"),
                    OsStr::new("-p"),
                    OsStr::new("--"),
                    arc,
                ],
                listing_limit,
                TIME_LIMIT,
            )?;
            (
                parse_7z_list(&String::from_utf8_lossy(&text)),
                vec![
                    OsStr::new("e"),
                    OsStr::new("-so"),
                    OsStr::new("-y"),
                    OsStr::new("-bd"),
                    OsStr::new("--"),
                    arc,
                ],
            )
        }
    };
    check_listing(&listed)?;
    let total = listed.iter().fold(0u64, |a, l| a.saturating_add(l.size));
    let stream = run(tool.path(), &stream_args, total, TIME_LIMIT)?;
    split_stream(&listed, &stream)
}

/// The files of a RAR archive, using the helper found on this system.
pub fn extract(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let tool = find_tool().context(
        "RAR comic archives (CBR) need the `unrar` or `7z` program; install it, or convert the file to CBZ",
    )?;
    extract_with(&tool, bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn crc32(data: &[u8]) -> u32 {
        let mut c = flate2::Crc::new();
        c.update(data);
        c.sum()
    }

    /// A RAR 1.5-4 archive with stored (uncompressed) files, written by hand so tests need no `rar`.
    pub fn stored_rar(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = RAR_MAGIC.to_vec();
        out.push(0);
        // main header
        let mut main = vec![0x73, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0];
        let crc = crc32(&main) as u16;
        let mut head = crc.to_le_bytes().to_vec();
        head.append(&mut main);
        out.extend(head);
        for (name, data) in files {
            let mut h = vec![0x74u8];
            h.extend(0x8000u16.to_le_bytes());
            h.extend((32 + name.len() as u16).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
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

    #[test]
    fn the_unrar_listing_is_parsed() {
        let text = "\nArchive: a.rar\n\n        Name: a/1.jpg\n        Type: File\n        Size: 12\n Packed size: 12\n\n        Name: a\n        Type: Directory\n        Size: 0\n\n        Name: ComicInfo.xml\n        Type: File\n        Size: 5\n";
        assert_eq!(
            parse_unrar_list(text),
            vec![
                Listed {
                    name: "a/1.jpg".into(),
                    size: 12
                },
                Listed {
                    name: "ComicInfo.xml".into(),
                    size: 5
                }
            ]
        );
    }

    #[test]
    fn the_7z_listing_is_parsed() {
        let text = "Path = a/1.jpg\nSize = 12\nFolder = -\nAttributes = A\n\nPath = a\nSize = 0\nFolder = +\nAttributes = D\n\nPath = b.png\nSize = 7\nFolder = -\n";
        let v = parse_7z_list(text);
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].name, "b.png");
        assert_eq!(v[1].size, 7);
    }

    #[test]
    fn the_stream_is_cut_by_the_listed_sizes_and_only_pictures_are_kept() {
        let listed = vec![
            Listed {
                name: "x/../evil.jpg".into(),
                size: 2,
            },
            Listed {
                name: "readme.txt".into(),
                size: 3,
            },
            Listed {
                name: "p/1.jpg".into(),
                size: 2,
            },
        ];
        let out = split_stream(&listed, b"aabbbcc").unwrap();
        assert_eq!(out, vec![("p/1.jpg".to_string(), b"cc".to_vec())]);
        assert!(split_stream(&listed, b"short").is_err());
    }

    #[test]
    fn the_limits_refuse_a_bomb_before_anything_is_unpacked() {
        let too_many: Vec<Listed> = (0..MAX_LISTED + 1)
            .map(|i| Listed {
                name: format!("{i}.jpg"),
                size: 1,
            })
            .collect();
        assert!(check_listing(&too_many).is_err());
        let big = vec![Listed {
            name: "a.jpg".into(),
            size: MAX_ENTRY + 1,
        }];
        assert!(check_listing(&big).is_err());
        let sum: Vec<Listed> = (0..5)
            .map(|i| Listed {
                name: format!("{i}.jpg"),
                size: MAX_ENTRY,
            })
            .collect();
        assert!(check_listing(&sum).is_err());
        assert!(check_listing(&[]).is_err());
    }

    #[test]
    fn a_7zip_without_the_rar_format_is_not_a_helper() {
        let with = "Formats:\n  0   C  Rar5 rar r00 \n  0  Zip zip\n";
        let without = "Formats:\n  0  Zip zip jar\n  0  7z 7z\nCodecs:\n";
        assert!(lists_rar(with));
        assert!(!lists_rar(without));
        assert!(!lists_rar(""));
    }

    #[test]
    fn no_helper_in_the_directories_means_no_tool() {
        assert_eq!(find_tool_in(&["/nonexistent-bookx-dir"]), None);
    }

    #[test]
    fn a_stored_archive_is_unpacked_when_a_helper_exists() {
        let Some(tool) = find_tool() else {
            eprintln!("skipped: no unrar or 7z");
            return;
        };
        let img = crate::comic::tests::img(1, "jpg");
        let rar = stored_rar(&[
            ("pages/2.jpg", &img),
            ("notes.txt", b"hi"),
            ("ComicInfo.xml", b"<ComicInfo/>"),
        ]);
        let files = extract_with(&tool, &rar).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["pages/2.jpg", "ComicInfo.xml"]);
        assert_eq!(files[0].1, img);
    }

    #[test]
    fn garbage_with_the_magic_is_an_error_not_a_hang() {
        let Some(tool) = find_tool() else { return };
        let mut v = RAR_MAGIC.to_vec();
        v.extend(vec![0xAB; 500]);
        assert!(extract_with(&tool, &v).is_err());
        assert!(extract_with(&tool, b"PK").is_err());
    }

    #[test]
    fn a_helper_that_runs_too_long_is_killed() {
        let sleep = Path::new("/usr/bin/sleep");
        if !sleep.is_file() {
            return;
        }
        let t = std::time::Instant::now();
        let r = run(
            sleep,
            &[std::ffi::OsStr::new("30")],
            10,
            Duration::from_millis(200),
        );
        assert!(r.is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
