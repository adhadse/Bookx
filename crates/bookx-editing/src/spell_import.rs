//! Managing Hunspell dictionaries (CE-279): list the ones in a set of folders, import a `.dic`/`.aff` pair or a
//! LibreOffice `.oxt` extension (a zip) into the user's folder, remove them again.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Largest file taken out of an extension (bytes). Real dictionaries are a few MB.
const MAX_ENTRY: u64 = 80 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictInfo {
    /// File stem: `en_GB`.
    pub lang: String,
    pub aff: PathBuf,
    pub dic: PathBuf,
    /// Number of words announced on the first line of the `.dic` (0 when unreadable).
    pub words: usize,
}

/// Number of words a `.dic` file announces, read from its first line without loading the file.
pub fn dic_word_count(dic: &Path) -> usize {
    let Ok(f) = std::fs::File::open(dic) else {
        return 0;
    };
    let mut buf = Vec::new();
    let _ = f.take(64).read_to_end(&mut buf);
    let first = String::from_utf8_lossy(&buf);
    first
        .lines()
        .next()
        .map(|l| l.trim_start_matches('\u{feff}').trim())
        .and_then(|l| l.parse().ok())
        .unwrap_or(0)
}

/// All `name.aff` + `name.dic` pairs directly in `dirs`. When a name is in several folders, the first wins.
pub fn list(dirs: &[PathBuf]) -> Vec<DictInfo> {
    let mut found: BTreeMap<String, DictInfo> = BTreeMap::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("aff") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let dic = p.with_extension("dic");
            if dic.is_file() && !found.contains_key(stem) {
                found.insert(
                    stem.to_string(),
                    DictInfo {
                        lang: stem.to_string(),
                        words: dic_word_count(&dic),
                        aff: p.clone(),
                        dic,
                    },
                );
            }
        }
    }
    found.into_values().collect()
}

/// Dictionary names become file names: only letters, digits, `_` and `-`.
pub fn safe_stem(stem: &str) -> Option<String> {
    let ok = !stem.is_empty()
        && stem.len() <= 40
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| stem.to_string())
}

fn check_pair(aff: &str, dic: &str, name: &str) -> Result<()> {
    spellbook::Dictionary::new(aff, dic)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("“{name}” is not a usable Hunspell dictionary: {e}"))
}

fn install(dest: &Path, stem: &str, aff: &str, dic: &str) -> Result<DictInfo> {
    check_pair(aff, dic, stem)?;
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    let (a, d) = (
        dest.join(format!("{stem}.aff")),
        dest.join(format!("{stem}.dic")),
    );
    std::fs::write(&a, aff)?;
    std::fs::write(&d, dic)?;
    Ok(DictInfo {
        lang: stem.to_string(),
        words: dic_word_count(&d),
        aff: a,
        dic: d,
    })
}

/// Copies a Hunspell pair into `dest`. Either file may be given; the other is looked up next to it. The files
/// must be UTF-8 (`SET UTF-8`) and are checked by loading them.
pub fn import_pair(file: &Path, dest: &Path) -> Result<DictInfo> {
    let (aff, dic) = (file.with_extension("aff"), file.with_extension("dic"));
    let stem = file
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(safe_stem)
        .context("The file name may only use letters, digits, _ and -")?;
    let a = std::fs::read_to_string(&aff)
        .with_context(|| format!("Reading {} (UTF-8 expected)", aff.display()))?;
    let d = std::fs::read_to_string(&dic)
        .with_context(|| format!("Reading {} (UTF-8 expected)", dic.display()))?;
    install(dest, &stem, &a, &d)
}

fn read_entry<R: Read>(mut f: zip::read::ZipFile<'_, R>) -> Result<String> {
    let name = f.name().map(|n| n.into_owned()).unwrap_or_default();
    if f.size() > MAX_ENTRY {
        bail!("{name} is too large");
    }
    let mut bytes = Vec::new();
    (&mut f).take(MAX_ENTRY).read_to_end(&mut bytes)?;
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{name} is not UTF-8 text"))
}

/// Imports the dictionaries inside a LibreOffice extension (`.oxt`, a zip with `.aff`/`.dic` pairs). Hyphenation
/// and thesaurus files are skipped. Returns what was installed; fails when nothing usable is inside.
pub fn import_oxt(oxt: &Path, dest: &Path) -> Result<Vec<DictInfo>> {
    let file = std::fs::File::open(oxt).with_context(|| format!("opening {}", oxt.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).context("This is not a LibreOffice extension (zip)")?;
    let mut pairs: BTreeMap<String, (Option<usize>, Option<usize>)> = BTreeMap::new();
    for i in 0..zip.len() {
        let Ok(e) = zip.by_index(i) else { continue };
        if e.is_dir() {
            continue;
        }
        let name = e.name().map(|n| n.replace('\\', "/")).unwrap_or_default();
        let base = name.rsplit('/').next().unwrap_or("");
        let Some((stem, ext)) = base.rsplit_once('.') else {
            continue;
        };
        if stem.starts_with("hyph_") || stem.starts_with("th_") || stem.starts_with("README") {
            continue;
        }
        let slot = pairs.entry(stem.to_string()).or_default();
        match ext {
            "aff" => slot.0 = Some(i),
            "dic" => slot.1 = Some(i),
            _ => {}
        }
    }
    let mut done = Vec::new();
    let mut last_err = None;
    for (stem, (a, d)) in pairs {
        let (Some(a), Some(d)) = (a, d) else { continue };
        let Some(stem) = safe_stem(&stem) else {
            continue;
        };
        let res = (|| {
            let aff = read_entry(zip.by_index(a)?)?;
            let dic = read_entry(zip.by_index(d)?)?;
            install(dest, &stem, &aff, &dic)
        })();
        match res {
            Ok(info) => done.push(info),
            Err(e) => last_err = Some(e),
        }
    }
    if done.is_empty() {
        return Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!("No Hunspell dictionary (.aff and .dic) found in the extension")
        }));
    }
    Ok(done)
}

/// Deletes an imported dictionary (both files) from `dir`.
pub fn remove(dir: &Path, lang: &str) -> Result<()> {
    let stem = safe_stem(lang).context("invalid dictionary name")?;
    let mut any = false;
    for ext in ["aff", "dic"] {
        let p = dir.join(format!("{stem}.{ext}"));
        if p.is_file() {
            std::fs::remove_file(&p).with_context(|| format!("deleting {}", p.display()))?;
            any = true;
        }
    }
    if !any {
        bail!("“{lang}” is not in {}", dir.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const AFF: &str = "SET UTF-8\n";
    const DIC: &str = "2\nhello\nworld\n";

    fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-simp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_oxt(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (n, b) in entries {
            z.start_file(*n, opts).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn lists_pairs_and_counts_words() {
        let d = dir();
        std::fs::write(d.join("en_GB.aff"), AFF).unwrap();
        std::fs::write(d.join("en_GB.dic"), DIC).unwrap();
        std::fs::write(d.join("lonely.aff"), AFF).unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        let other = dir();
        std::fs::write(other.join("en_GB.aff"), AFF).unwrap();
        std::fs::write(other.join("en_GB.dic"), "99\n").unwrap();
        std::fs::write(other.join("de_DE.aff"), AFF).unwrap();
        std::fs::write(other.join("de_DE.dic"), "\u{feff}7\n").unwrap();
        let l = list(&[d.clone(), other.clone()]);
        assert_eq!(
            l.iter().map(|i| i.lang.as_str()).collect::<Vec<_>>(),
            ["de_DE", "en_GB"]
        );
        assert_eq!(l[1].words, 2, "the first folder wins");
        assert_eq!(l[0].words, 7, "a BOM does not matter");
        assert!(list(&[PathBuf::from("/nonexistent-bookx")]).is_empty());
    }

    #[test]
    fn imports_a_pair_from_either_file_and_rejects_bad_ones() {
        let src = dir();
        let dest = dir().join("sub/hunspell");
        std::fs::write(src.join("fr_FR.aff"), AFF).unwrap();
        std::fs::write(src.join("fr_FR.dic"), DIC).unwrap();
        let info = import_pair(&src.join("fr_FR.dic"), &dest).unwrap();
        assert_eq!((info.lang.as_str(), info.words), ("fr_FR", 2));
        assert!(dest.join("fr_FR.aff").is_file() && dest.join("fr_FR.dic").is_file());
        assert!(import_pair(&src.join("fr_FR.aff"), &dest).is_ok());
        std::fs::write(src.join("bad name.aff"), AFF).unwrap();
        std::fs::write(src.join("bad name.dic"), DIC).unwrap();
        assert!(import_pair(&src.join("bad name.dic"), &dest).is_err());
        std::fs::write(src.join("xx.aff"), AFF).unwrap();
        assert!(
            import_pair(&src.join("xx.dic"), &dest).is_err(),
            "missing .dic"
        );
        std::fs::write(src.join("yy.aff"), [0xff, 0xfe, 0xfd]).unwrap();
        std::fs::write(src.join("yy.dic"), DIC).unwrap();
        let e = import_pair(&src.join("yy.aff"), &dest).unwrap_err();
        assert!(format!("{e:#}").contains("UTF-8"));
    }

    #[test]
    fn imports_dictionaries_from_an_oxt_and_ignores_the_rest() {
        let d = dir();
        let oxt = d.join("dict-es.oxt");
        make_oxt(
            &oxt,
            &[
                ("META-INF/manifest.xml", b"<manifest/>"),
                ("dictionaries/es_ES.aff", AFF.as_bytes()),
                ("dictionaries/es_ES.dic", DIC.as_bytes()),
                ("dictionaries/es_MX.aff", AFF.as_bytes()),
                ("dictionaries/es_MX.dic", DIC.as_bytes()),
                ("dictionaries/hyph_es_ES.dic", b"hyph"),
                ("dictionaries/th_es_ES.dat", b"th"),
                ("dictionaries/../../evil.aff", AFF.as_bytes()),
                ("dictionaries/README_es.txt", b"read me"),
            ],
        );
        let dest = d.join("out");
        let got = import_oxt(&oxt, &dest).unwrap();
        let mut names: Vec<_> = got.iter().map(|i| i.lang.clone()).collect();
        names.sort();
        assert_eq!(names, ["es_ES", "es_MX"]);
        let mut files: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        files.sort();
        assert_eq!(files, ["es_ES.aff", "es_ES.dic", "es_MX.aff", "es_MX.dic"]);
        assert!(!d.join("evil.aff").exists() && !dest.join("evil.aff").exists());
        // the installed dictionary loads
        let mut c = crate::spell::SpellChecker::new();
        c.add_dictionary_files("es-ES", &dest.join("es_ES.aff"), &dest.join("es_ES.dic"))
            .unwrap();
        assert_eq!(c.check_word("es", "hello"), Some(true));
    }

    #[test]
    fn oxt_errors_are_readable() {
        let d = dir();
        let plain = d.join("x.oxt");
        std::fs::write(&plain, b"not a zip").unwrap();
        assert!(
            import_oxt(&plain, &d.join("o"))
                .unwrap_err()
                .to_string()
                .contains("zip")
        );
        let empty = d.join("e.oxt");
        make_oxt(&empty, &[("readme.txt", b"hi")]);
        assert!(
            import_oxt(&empty, &d.join("o"))
                .unwrap_err()
                .to_string()
                .contains("No Hunspell")
        );
        let broken = d.join("b.oxt");
        make_oxt(
            &broken,
            &[
                ("a/xx.aff", AFF.as_bytes()),
                ("a/xx.dic", b"3\nhello\n\xff"),
            ],
        );
        assert!(import_oxt(&broken, &d.join("o")).is_err());
        assert!(import_oxt(&d.join("missing.oxt"), &d.join("o")).is_err());
    }

    #[test]
    fn removing_deletes_both_files_only_for_safe_names() {
        let d = dir();
        std::fs::write(d.join("a_B.aff"), AFF).unwrap();
        std::fs::write(d.join("a_B.dic"), DIC).unwrap();
        remove(&d, "a_B").unwrap();
        assert!(!d.join("a_B.aff").exists() && !d.join("a_B.dic").exists());
        assert!(remove(&d, "a_B").is_err());
        assert!(remove(&d, "../x").is_err());
        assert_eq!(safe_stem("en-GB_x1").as_deref(), Some("en-GB_x1"));
        assert_eq!(safe_stem("a b"), None);
    }
}
