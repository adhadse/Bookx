//! Installs a dictionary of the catalogue: download (resumable, hash-checked, with the next origin when
//! one fails: `bookx_tts::download`), unpack safely, check that the StarDict reader opens it, and
//! put it into the dictionaries folder with a marker that says which language it is for. Also tells
//! which languages the folder already has dictionaries for. No GTK.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::dict_catalog::{Dict, Format};
use bookx_tts::synth::Cancel;

/// The name of the file that marks a dictionary Bookx installed.
pub const MARKER: &str = "bookx-dict.json";
/// No file of a dictionary is bigger than this once unpacked.
pub const MAX_FILE: u64 = 1 << 30;
/// Nor all files together.
pub const MAX_TOTAL: u64 = 2 << 30;
/// A dictionary has a handful of files.
pub const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub lang: String,
    pub id: String,
    pub title: String,
    pub licence: String,
    pub origin: String,
    pub installed: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub marker: Marker,
    pub dir: PathBuf,
}

/// A member of an archive as a file name of the dictionary folder, or `None` when it must be
/// ignored. Errors for names that try to leave the folder (`..`, absolute, drive letters).
fn member_name(name: &str) -> Result<Option<String>> {
    if name.contains('\0') || name.contains('\\') {
        bail!("the archive has an unsafe name ({name:?}) and was not unpacked");
    }
    let p = Path::new(name);
    if p.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("the archive has an unsafe path ({name}) and was not unpacked");
    }
    let Some(base) = p.file_name().and_then(|f| f.to_str()) else {
        return Ok(None);
    };
    // only what a StarDict dictionary is made of; READMEs, pictures and folders are left out
    let lower = base.to_ascii_lowercase();
    let keep = [".ifo", ".idx", ".idx.gz", ".dict", ".dict.dz", ".syn"]
        .iter()
        .any(|e| lower.ends_with(e));
    Ok(keep.then(|| base.to_string()))
}

struct Budget {
    files: usize,
    total: u64,
}

impl Budget {
    fn take(&mut self, name: &str, size: u64) -> Result<()> {
        self.files += 1;
        self.total = self.total.saturating_add(size);
        if self.files > MAX_ENTRIES {
            bail!("the archive has too many files");
        }
        if size > MAX_FILE {
            bail!("{name} is {size} bytes: too big, not unpacked");
        }
        if self.total > MAX_TOTAL {
            bail!("the archive is too big once unpacked");
        }
        Ok(())
    }
}

/// Copies at most `size` bytes of `r` into `dest`; fails when there is more than announced.
fn copy_capped(r: &mut dyn Read, dest: &Path, announced: u64) -> Result<u64> {
    let mut f = fs::File::create(dest)?;
    let mut n = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let k = r.read(&mut buf)?;
        if k == 0 {
            break;
        }
        n += k as u64;
        if n > announced.max(1) || n > MAX_FILE {
            drop(f);
            let _ = fs::remove_file(dest);
            bail!("a file in the archive is bigger than it says: not unpacked");
        }
        f.write_all(&buf[..k])?;
    }
    Ok(n)
}

/// Unpacks the StarDict files of an archive into `dest` (flat: the folders inside are dropped).
/// Refuses unsafe paths, links, too many or too big files.
pub fn unpack(archive: &Path, format: Format, dest: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(dest)?;
    let mut got = Vec::new();
    let mut budget = Budget { files: 0, total: 0 };
    match format {
        Format::Zip => {
            let mut z = zip::ZipArchive::new(fs::File::open(archive)?)
                .map_err(|e| anyhow!("not a zip file: {e}"))?;
            for i in 0..z.len() {
                let mut e = z.by_index(i).map_err(|e| anyhow!("bad zip: {e}"))?;
                if e.is_dir() || e.is_symlink() {
                    continue;
                }
                let raw = e
                    .name()
                    .map_err(|e| anyhow!("bad name in the zip: {e}"))?
                    .to_string();
                let Some(name) = member_name(&raw)? else {
                    continue;
                };
                budget.take(&name, e.size())?;
                let size = e.size();
                copy_capped(&mut e, &dest.join(&name), size)?;
                got.push(name);
            }
        }
        Format::TarGz | Format::TarBz2 => {
            let f = fs::File::open(archive)?;
            let r: Box<dyn Read> = if format == Format::TarGz {
                Box::new(flate2::read::GzDecoder::new(f))
            } else {
                Box::new(bzip2::read::BzDecoder::new(f))
            };
            let mut ar = tar::Archive::new(r);
            for e in ar.entries()? {
                let mut e = e?;
                if !e.header().entry_type().is_file() {
                    continue;
                }
                let path = e.path()?.to_string_lossy().into_owned();
                let Some(name) = member_name(&path)? else {
                    continue;
                };
                budget.take(&name, e.size())?;
                let size = e.size();
                copy_capped(&mut e, &dest.join(&name), size)?;
                got.push(name);
            }
        }
    }
    if !got.iter().any(|n| n.to_ascii_lowercase().ends_with(".ifo")) {
        bail!("the archive has no StarDict dictionary in it");
    }
    Ok(got)
}

fn folder_name(lang: &str, id: &str) -> String {
    format!("{lang}-{id}")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Downloads and installs `dict` for `lang` into `root`. `progress` gets (bytes done, bytes total) of the
/// download. The origins are tried in order; the first that gives a dictionary the reader can open wins.
pub fn install(
    dict: &Dict,
    lang: &str,
    root: &Path,
    cancel: &Cancel,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Installed> {
    fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    let work = root.join(".downloads");
    fs::create_dir_all(&work)?;
    let mut errors = Vec::new();
    for o in &dict.origins {
        if cancel.is_cancelled() {
            bail!("cancelled");
        }
        let archive = work.join(format!("{}-{}.archive", dict.id, &o.sha256[..12]));
        let r = bookx_tts::download::download_file(
            &o.url,
            &archive,
            &o.sha256,
            o.size,
            cancel,
            &mut |n| progress(n, o.size),
        )
        .map_err(|e| anyhow!("{e:#}"))
        .and_then(|()| install_archive(dict, lang, root, &archive, o.format, &o.url));
        let _ = fs::remove_file(&archive);
        match r {
            Ok(i) => return Ok(i),
            Err(e) => {
                if cancel.is_cancelled() {
                    bail!("cancelled");
                }
                errors.push(format!("{}: {e:#}", o.url));
            }
        }
    }
    bail!("{}", errors.join("\n"))
}

/// The second half of [`install`], for an archive that is already here (and checked).
pub fn install_archive(
    dict: &Dict,
    lang: &str,
    root: &Path,
    archive: &Path,
    format: Format,
    origin: &str,
) -> Result<Installed> {
    let tmp = root
        .join(".downloads")
        .join(format!("unpack-{}-{}", dict.id, std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let result = (|| {
        unpack(archive, format, &tmp)?;
        // the reader must be able to use it
        let d = bookx_lookup::stardict::StarDict::open(&tmp)
            .map_err(|e| anyhow!("the dictionary cannot be read: {e:#}"))?;
        if d.is_empty() {
            bail!("the dictionary is empty");
        }
        let marker = Marker {
            lang: lang.to_string(),
            id: dict.id.clone(),
            title: dict.title.clone(),
            licence: dict.licence.clone(),
            origin: origin.to_string(),
            installed: now(),
        };
        fs::write(tmp.join(MARKER), serde_json::to_string_pretty(&marker)?)?;
        let dest = root.join(folder_name(lang, &dict.id));
        let _ = fs::remove_dir_all(&dest);
        fs::rename(&tmp, &dest).with_context(|| format!("moving it to {}", dest.display()))?;
        Ok(Installed { marker, dir: dest })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// The dictionaries Bookx installed in `root`.
pub fn managed(root: &Path) -> Vec<Installed> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let dir = e.path();
        if !dir.is_dir() || e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if let Some(m) = fs::read_to_string(dir.join(MARKER))
            .ok()
            .and_then(|t| serde_json::from_str::<Marker>(&t).ok())
        {
            out.push(Installed { marker: m, dir });
        }
    }
    out.sort_by(|a, b| {
        a.marker
            .lang
            .cmp(&b.marker.lang)
            .then(a.marker.id.cmp(&b.marker.id))
    });
    out
}

/// Removes an installed dictionary (only a folder that has our marker).
pub fn remove(i: &Installed) -> Result<()> {
    if !i.dir.join(MARKER).is_file() {
        bail!("not a dictionary Bookx installed");
    }
    fs::remove_dir_all(&i.dir)?;
    Ok(())
}

/// The languages a dictionary called `bookname` is for: the first language it names (a bilingual
/// dictionary is looked up in the language of its headwords), by name (`German-English`) or by a
/// code in brackets (`(de-en)`). Empty when the name says nothing.
pub fn languages_in_name(bookname: &str) -> Vec<String> {
    let lower = bookname.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for (code, name) in crate::dict_catalog::LANGUAGE_NAMES {
        let needle = name.to_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find(&needle) {
            let at = from + i;
            let before_ok = at == 0
                || !lower[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphabetic());
            let after = at + needle.len();
            let after_ok = !lower[after..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic());
            if before_ok && after_ok && best.as_ref().is_none_or(|(b, _)| at < *b) {
                best = Some((at, (*code).to_string()));
            }
            from = at + needle.len();
        }
    }
    // "(de-en)" and "[fr]"
    if let Some(open) = lower.find(['(', '[']) {
        let inner: String = lower[open + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
            .collect();
        if let Some(first) = inner.split('-').next() {
            if first.len() == 2
                && crate::dict_catalog::language_name(first) != first
                && best.is_none()
            {
                best = Some((open, first.to_string()));
            }
        }
    }
    best.map(|(_, c)| vec![c]).unwrap_or_default()
}

/// Which languages the dictionaries in a folder are for.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Presence {
    pub langs: HashSet<String>,
    /// A dictionary that does not say which language it is for (the user's own): it counts for every
    /// language, the user put it there.
    pub universal: bool,
}

impl Presence {
    pub fn has(&self, lang: &str) -> bool {
        self.universal || self.langs.contains(lang)
    }
}

pub fn presence(root: &Path) -> Presence {
    let mut p = Presence::default();
    let managed_dirs: HashSet<PathBuf> = managed(root)
        .into_iter()
        .map(|i| {
            p.langs.insert(i.marker.lang.clone());
            i.dir
        })
        .collect();
    let loaded = crate::dictionaries::dictionary_dirs(root);
    for d in loaded {
        if managed_dirs.contains(&d) {
            continue;
        }
        let ifo = fs::read_dir(&d)
            .ok()
            .and_then(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().is_some_and(|x| x == "ifo"))
            })
            .and_then(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let name = ifo
            .lines()
            .find_map(|l| l.strip_prefix("bookname="))
            .unwrap_or("");
        let langs = languages_in_name(name);
        if langs.is_empty() {
            p.universal = true;
        }
        p.langs.extend(langs);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-dinst-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tiny StarDict: "hello" and "world".
    fn stardict_files(base: &str) -> Vec<(String, Vec<u8>)> {
        let dict = b"greeting".to_vec();
        let dict2 = b"planet".to_vec();
        let mut idx = Vec::new();
        for (w, off, len) in [
            ("hello", 0u32, dict.len() as u32),
            ("world", dict.len() as u32, dict2.len() as u32),
        ] {
            idx.extend_from_slice(w.as_bytes());
            idx.push(0);
            idx.extend_from_slice(&off.to_be_bytes());
            idx.extend_from_slice(&len.to_be_bytes());
        }
        let mut all = dict.clone();
        all.extend_from_slice(&dict2);
        let ifo = format!(
            "StarDict's dict ifo file\nversion=2.4.2\nbookname=Test German-English (de-en)\nwordcount=2\nidxfilesize={}\nsametypesequence=m\n",
            idx.len()
        );
        vec![
            (format!("{base}.ifo"), ifo.into_bytes()),
            (format!("{base}.idx"), idx),
            (format!("{base}.dict"), all),
        ]
    }

    fn make_tgz(path: &Path, prefix: &str, extra: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        for (n, data) in stardict_files("d")
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .chain(extra.iter().copied())
        {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, format!("{prefix}{n}"), data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap();
    }

    fn evil_tgz(path: &Path, name: &str) {
        let f = fs::File::create(path).unwrap();
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        h.set_mode(0o644);
        h.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        h.set_cksum();
        b.append(&h, &b"x"[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
    }

    fn dict() -> Dict {
        crate::dict_catalog::Catalog::parse(
            &format!(
                r#"{{"version":1,"languages":[{{"code":"de","name":"German","dictionaries":[{{"id":"t-de","title":"Test","kind":"bilingual","source":"s","licence":"CC","licence_url":"https://l","size":1,"origins":[{{"url":"http://127.0.0.1:1/x","format":"tar.gz","sha256":"{}","size":1}}]}}]}}]}}"#,
                "0".repeat(64)
            ),
            true,
        )
        .unwrap()
        .languages[0]
            .dictionaries[0]
            .clone()
    }

    #[test]
    fn a_tar_gz_is_unpacked_flat_and_installed_with_its_marker() {
        let root = tmp("tgz");
        let a = root.join("a.tgz");
        make_tgz(
            &a,
            "German-English dictionary stardict/",
            &[("README.txt", b"hi"), ("res/pic.png", b"png")],
        );
        let i = install_archive(&dict(), "de", &root, &a, Format::TarGz, "x").unwrap();
        assert_eq!(i.dir, root.join("de-t-de"));
        let mut names: Vec<String> = fs::read_dir(&i.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["bookx-dict.json", "d.dict", "d.idx", "d.ifo"],
            "only the dictionary files, no folders"
        );
        assert_eq!(managed(&root), vec![i.clone()]);
        // and the reader finds the word
        let loaded = crate::dictionaries::load(&root);
        assert_eq!(loaded.set.names().len(), 1);
        assert!(!loaded.set.lookup_all("hello").results.is_empty());
        // installing again replaces it
        let again = install_archive(&dict(), "de", &root, &a, Format::TarGz, "x").unwrap();
        assert_eq!(again.dir, i.dir);
        assert_eq!(managed(&root).len(), 1);
        assert!(!root.join(".downloads").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("unpack")));
        remove(&again).unwrap();
        assert!(managed(&root).is_empty());
    }

    #[test]
    fn zip_and_bzip2_archives_work_too() {
        let root = tmp("zip");
        let z = root.join("a.zip");
        {
            let mut w = zip::ZipWriter::new(fs::File::create(&z).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.add_directory("wikdict-de-en/", o).unwrap();
            for (n, d) in stardict_files("stardict") {
                w.start_file(format!("wikdict-de-en/{n}"), o).unwrap();
                w.write_all(&d).unwrap();
            }
            w.finish().unwrap();
        }
        assert_eq!(unpack(&z, Format::Zip, &root.join("u1")).unwrap().len(), 3);
        let b = root.join("a.tar.bz2");
        {
            let mut t = tar::Builder::new(bzip2::write::BzEncoder::new(
                fs::File::create(&b).unwrap(),
                bzip2::Compression::fast(),
            ));
            for (n, d) in stardict_files("s") {
                let mut h = tar::Header::new_gnu();
                h.set_size(d.len() as u64);
                h.set_mode(0o644);
                h.set_cksum();
                t.append_data(&mut h, n, &d[..]).unwrap();
            }
            t.into_inner().unwrap().finish().unwrap();
        }
        assert_eq!(
            unpack(&b, Format::TarBz2, &root.join("u2")).unwrap().len(),
            3
        );
    }

    #[test]
    fn unsafe_archives_are_refused_and_leave_nothing_behind() {
        let root = tmp("evil");
        for name in [
            "../evil.ifo",
            "/etc/evil.ifo",
            "a/../../evil.idx",
            "C:\\x\\evil.ifo",
        ] {
            let a = root.join("e.tgz");
            evil_tgz(&a, name);
            let r = install_archive(&dict(), "de", &root, &a, Format::TarGz, "x");
            assert!(r.is_err(), "{name} must be refused");
            assert!(
                !root.join("evil.ifo").exists()
                    && !root.parent().unwrap().join("evil.ifo").exists()
            );
            assert!(managed(&root).is_empty());
        }
        assert!(member_name("a/b/c.ifo").unwrap().as_deref() == Some("c.ifo"));
        assert!(member_name("x.txt").unwrap().is_none());
        assert!(member_name("nul\0.ifo").is_err());
    }

    #[test]
    fn an_archive_without_a_dictionary_or_with_a_broken_one_is_refused() {
        let root = tmp("bad");
        let a = root.join("n.tgz");
        {
            let f = fs::File::create(&a).unwrap();
            let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
                f,
                flate2::Compression::fast(),
            ));
            let mut h = tar::Header::new_gnu();
            h.set_size(2);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "readme.txt", &b"hi"[..]).unwrap();
            b.into_inner().unwrap().finish().unwrap();
        }
        assert!(
            install_archive(&dict(), "de", &root, &a, Format::TarGz, "x")
                .unwrap_err()
                .to_string()
                .contains("no StarDict")
        );
        // an .ifo that the reader cannot open
        let b = root.join("b.tgz");
        make_tgz(&b, "", &[]);
        let f = fs::File::create(&b).unwrap();
        let mut t = tar::Builder::new(flate2::write::GzEncoder::new(
            f,
            flate2::Compression::fast(),
        ));
        for (n, d) in [
            ("x.ifo", &b"garbage"[..]),
            ("x.idx", &b""[..]),
            ("x.dict", &b""[..]),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(d.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            t.append_data(&mut h, n, d).unwrap();
        }
        t.into_inner().unwrap().finish().unwrap();
        assert!(install_archive(&dict(), "de", &root, &b, Format::TarGz, "x").is_err());
        assert!(managed(&root).is_empty());
        assert!(unpack(&root.join("missing"), Format::Zip, &root.join("o")).is_err());
    }

    #[test]
    fn budgets_stop_a_bomb() {
        let mut b = Budget { files: 0, total: 0 };
        assert!(b.take("a.dict", MAX_FILE + 1).is_err());
        let mut b = Budget { files: 0, total: 0 };
        for _ in 0..MAX_ENTRIES {
            b.take("a", 1).unwrap();
        }
        assert!(b.take("a", 1).is_err(), "too many files");
        let mut b = Budget { files: 0, total: 0 };
        b.take("a", MAX_FILE).unwrap();
        b.take("b", MAX_FILE).unwrap();
        assert!(b.take("c", 1).is_err(), "too much in all");
        // a member that is bigger than it announced is cut off
        let d = tmp("cap");
        let mut r: &[u8] = &[7u8; 100];
        assert!(copy_capped(&mut r, &d.join("f"), 10).is_err());
        assert!(!d.join("f").exists());
    }

    #[test]
    fn languages_are_read_from_the_name_of_a_dictionary() {
        assert_eq!(
            languages_in_name("German-English Wiktionary dictionary (de-en)"),
            ["de"]
        );
        assert_eq!(languages_in_name("English-German"), ["en"]);
        assert_eq!(
            languages_in_name("Czech-English Wiktionary dictionary (cs-en)"),
            ["cs"]
        );
        assert_eq!(languages_in_name("Oxford Spanish Dictionary"), ["es"]);
        assert_eq!(languages_in_name("My words (fr)"), ["fr"]);
        assert!(languages_in_name("Webster's Revised Unabridged Dictionary").is_empty());
        assert!(
            languages_in_name("Germanic roots").is_empty(),
            "a name must be a whole word"
        );
    }

    #[test]
    fn a_dictionary_of_the_user_without_a_language_counts_for_all() {
        let root = tmp("pres");
        assert!(!presence(&root).has("de"));
        // the user's own, named by language
        let own = root.join("mine");
        fs::create_dir_all(&own).unwrap();
        for (n, d) in stardict_files("m") {
            fs::write(own.join(n), d).unwrap();
        }
        let p = presence(&root);
        assert!(p.has("de") && !p.has("fr") && !p.universal, "named German");
        // a user dictionary with an unspecific name
        let any = root.join("any");
        fs::create_dir_all(&any).unwrap();
        for (n, d) in stardict_files("a") {
            let d = if n.ends_with(".ifo") {
                String::from_utf8(d)
                    .unwrap()
                    .replace("Test German-English (de-en)", "My dictionary")
                    .into_bytes()
            } else {
                d
            };
            fs::write(any.join(n), d).unwrap();
        }
        assert!(presence(&root).has("fr"), "unspecific: all languages");
        // the dictionaries Bookx installed are exact
        let r2 = tmp("pres2");
        let a = r2.join("a.tgz");
        make_tgz(&a, "", &[]);
        install_archive(&dict(), "de", &r2, &a, Format::TarGz, "x").unwrap();
        let p = presence(&r2);
        assert!(p.has("de") && !p.has("en") && !p.universal);
    }

    /// A one-file HTTP server for the download.
    fn serve(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        use std::io::BufRead;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/d.tgz", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            l.set_nonblocking(false).unwrap();
            for s in l.incoming().take(6) {
                let Ok(mut s) = s else { continue };
                let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                let mut range_from = 0usize;
                while r.read_line(&mut line).unwrap_or(0) > 2 {
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range_from = v
                            .trim()
                            .trim_end_matches('-')
                            .split('-')
                            .next()
                            .and_then(|n| n.parse().ok())
                            .unwrap_or(0);
                    }
                    line.clear();
                }
                let part = &body[range_from.min(body.len())..];
                let head = if range_from > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n", part.len(), range_from, body.len() - 1, body.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", part.len())
                };
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(part);
            }
        });
        (url, h)
    }

    #[test]
    fn the_next_origin_is_used_when_the_first_fails() {
        use sha2::Digest;
        let root = tmp("net");
        let a = root.join("src.tgz");
        make_tgz(&a, "x/", &[]);
        let body = fs::read(&a).unwrap();
        let sha = format!("{:x}", sha2::Sha256::digest(&body));
        let (good, _h) = serve(body.clone());
        let mut d = dict();
        d.origins = vec![
            crate::dict_catalog::Origin {
                url: "http://127.0.0.1:1/nothing".into(),
                format: Format::TarGz,
                sha256: sha.clone(),
                size: body.len() as u64,
            },
            crate::dict_catalog::Origin {
                url: good.clone(),
                format: Format::TarGz,
                sha256: "f".repeat(64),
                size: body.len() as u64,
            },
            crate::dict_catalog::Origin {
                url: good,
                format: Format::TarGz,
                sha256: sha,
                size: body.len() as u64,
            },
        ];
        let mut last = (0, 0);
        let i = install(
            &d,
            "de",
            &root.join("dicts"),
            &Cancel::new(),
            &mut |a, b| last = (a, b),
        )
        .unwrap();
        assert_eq!(i.marker.id, "t-de");
        assert_eq!(last.0, last.1, "the progress reached the end");
        assert!(i.marker.origin.contains("127.0.0.1:"));
        // a wrong hash everywhere: nothing installed, every reason reported
        let mut bad = d.clone();
        bad.origins.truncate(2);
        let e = install(
            &bad,
            "fr",
            &root.join("dicts2"),
            &Cancel::new(),
            &mut |_, _| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("127.0.0.1"), "{e}");
        assert!(managed(&root.join("dicts2")).is_empty());
        // cancelled before it starts
        let c = Cancel::new();
        c.cancel();
        assert!(install(&d, "de", &root.join("dicts3"), &c, &mut |_, _| {})
            .unwrap_err()
            .to_string()
            .contains("cancelled"));
    }

    /// The real archives of the catalogue, when `build-aux/dict-catalog.py generate|verify` has fetched
    /// them into its cache: each must unpack, be opened by the StarDict reader and have many words.
    #[test]
    fn the_real_archives_of_the_catalogue_open_in_the_reader() {
        use sha2::Digest;
        let cache = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".cache/bookx-dict-catalog");
        let Ok(rd) = fs::read_dir(&cache) else {
            return; // nothing fetched on this machine
        };
        let mut by_sha = std::collections::HashMap::new();
        for e in rd.flatten() {
            if e.path().extension().is_some() {
                continue;
            }
            if let Ok(b) = fs::read(e.path()) {
                by_sha.insert(format!("{:x}", sha2::Sha256::digest(&b)), e.path());
            }
        }
        let mut opened = 0;
        for l in &crate::dict_catalog::catalog().languages {
            for d in &l.dictionaries {
                for o in &d.origins {
                    let Some(p) = by_sha.get(&o.sha256) else {
                        continue;
                    };
                    let root = tmp(&format!("real-{}", d.id));
                    let i = install_archive(d, &l.code, &root, p, o.format, &o.url)
                        .unwrap_or_else(|e| panic!("{}: {e:#}", d.id));
                    let loaded = crate::dictionaries::load(&root);
                    assert_eq!(loaded.set.names().len(), 1, "{}", d.id);
                    let dd = bookx_lookup::stardict::StarDict::open(&i.dir).unwrap();
                    assert!(dd.len() > 1000, "{}: only {} words", d.id, dd.len());
                    opened += 1;
                    let _ = fs::remove_dir_all(&root);
                }
            }
        }
        eprintln!("{opened} real dictionaries opened");
    }
}
