//! Reader for StarDict dictionaries (`.ifo`, `.idx`, `.dict` / `.dict.dz`, optional `.syn`).
//!
//! `.dict.dz` files are inflated completely (with a size cap) instead of using the
//! dictzip random-access chunk table; a dictzip file is a valid gzip stream.

use anyhow::{anyhow, bail, Context, Result};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Largest inflated `.dict.dz` we accept.
pub const MAX_DICT_BYTES: u64 = 512 * 1024 * 1024;
/// Largest single article we accept.
pub const MAX_ENTRY_BYTES: u32 = 16 * 1024 * 1024;

/// Parsed `.ifo` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub bookname: String,
    pub version: String,
    pub wordcount: u64,
    pub synwordcount: u64,
    pub idxfilesize: Option<u64>,
    pub sametypesequence: Option<String>,
    pub idxoffsetbits: u8,
    pub author: Option<String>,
    pub description: Option<String>,
}

/// Parses the text of an `.ifo` file.
pub fn parse_ifo(text: &str) -> Result<Info> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    let magic = lines.next().unwrap_or("").trim();
    if magic != "StarDict's dict ifo file" {
        bail!("not a StarDict .ifo file (bad magic line {magic:?})");
    }
    let mut info = Info {
        bookname: String::new(),
        version: String::new(),
        wordcount: 0,
        synwordcount: 0,
        idxfilesize: None,
        sametypesequence: None,
        idxoffsetbits: 32,
        author: None,
        description: None,
    };
    let mut have_wordcount = false;
    for line in lines {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "version" => info.version = v.to_string(),
            "bookname" => info.bookname = v.to_string(),
            "wordcount" => {
                info.wordcount = v.parse().with_context(|| format!("bad wordcount {v:?}"))?;
                have_wordcount = true;
            }
            "synwordcount" => {
                info.synwordcount = v
                    .parse()
                    .with_context(|| format!("bad synwordcount {v:?}"))?
            }
            "idxfilesize" => {
                info.idxfilesize = Some(
                    v.parse()
                        .with_context(|| format!("bad idxfilesize {v:?}"))?,
                )
            }
            "sametypesequence" => info.sametypesequence = Some(v.to_string()),
            "idxoffsetbits" => {
                info.idxoffsetbits = match v {
                    "32" => 32,
                    "64" => 64,
                    other => bail!("unsupported idxoffsetbits {other:?}"),
                }
            }
            "author" => info.author = Some(v.to_string()),
            "description" => info.description = Some(v.to_string()),
            _ => {}
        }
    }
    if info.bookname.is_empty() {
        bail!(".ifo has no bookname");
    }
    if !have_wordcount {
        bail!(".ifo has no wordcount");
    }
    Ok(info)
}

/// One `.idx` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexItem {
    pub word: String,
    pub offset: u64,
    pub size: u32,
}

/// Parses the bytes of an `.idx` file with 32 or 64 bit offsets.
pub fn parse_index(bytes: &[u8], offset_bits: u8) -> Result<Vec<IndexItem>> {
    let off_len = match offset_bits {
        32 => 4,
        64 => 8,
        other => bail!("unsupported idxoffsetbits {other}"),
    };
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < bytes.len() {
        let rel = bytes[pos..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| anyhow!("index truncated: unterminated word at byte {pos}"))?;
        let word = std::str::from_utf8(&bytes[pos..pos + rel])
            .map_err(|_| anyhow!("index word at byte {pos} is not valid UTF-8"))?
            .to_string();
        let p = pos + rel + 1;
        if bytes.len() < p + off_len + 4 {
            bail!("index truncated: entry for {word:?} lacks offset/size");
        }
        let offset = if off_len == 8 {
            u64::from_be_bytes(bytes[p..p + 8].try_into()?)
        } else {
            u32::from_be_bytes(bytes[p..p + 4].try_into()?) as u64
        };
        let size = u32::from_be_bytes(bytes[p + off_len..p + off_len + 4].try_into()?);
        out.push(IndexItem { word, offset, size });
        pos = p + off_len + 4;
    }
    Ok(out)
}

/// One typed field of an article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub kind: char,
    pub data: Vec<u8>,
}

impl Field {
    /// Text content for the textual kinds (lowercase type letters).
    pub fn text(&self) -> Option<String> {
        if self.kind.is_ascii_lowercase() {
            Some(String::from_utf8_lossy(&self.data).into_owned())
        } else {
            None
        }
    }
}

/// Splits an article into fields according to `sametypesequence` (or the
/// per-field type prefix when it is absent).
pub fn decode_fields(data: &[u8], sametypesequence: Option<&str>) -> Result<Vec<Field>> {
    let mut out = Vec::new();
    match sametypesequence {
        Some(seq) if !seq.is_empty() => {
            let kinds: Vec<char> = seq.chars().collect();
            let mut pos = 0;
            for (i, &kind) in kinds.iter().enumerate() {
                check_kind(kind)?;
                let last = i + 1 == kinds.len();
                if last {
                    out.push(Field {
                        kind,
                        data: data[pos.min(data.len())..].to_vec(),
                    });
                } else if kind.is_ascii_lowercase() {
                    let rel = data[pos.min(data.len())..]
                        .iter()
                        .position(|&b| b == 0)
                        .ok_or_else(|| anyhow!("article truncated in field {i} ({kind})"))?;
                    out.push(Field {
                        kind,
                        data: data[pos..pos + rel].to_vec(),
                    });
                    pos += rel + 1;
                } else {
                    let (d, next) = read_sized(data, pos, i)?;
                    out.push(Field { kind, data: d });
                    pos = next;
                }
            }
        }
        _ => {
            let mut pos = 0;
            let mut i = 0;
            while pos < data.len() {
                let kind = data[pos] as char;
                check_kind(kind)?;
                pos += 1;
                if kind.is_ascii_lowercase() {
                    let rel = data[pos..].iter().position(|&b| b == 0);
                    let end = rel.map_or(data.len(), |r| pos + r);
                    out.push(Field {
                        kind,
                        data: data[pos..end].to_vec(),
                    });
                    pos = (end + 1).min(data.len().max(end));
                } else {
                    let (d, next) = read_sized(data, pos, i)?;
                    out.push(Field { kind, data: d });
                    pos = next;
                }
                i += 1;
            }
        }
    }
    Ok(out)
}

fn check_kind(kind: char) -> Result<()> {
    if kind.is_ascii_alphabetic() {
        Ok(())
    } else {
        bail!("invalid article field type {kind:?}")
    }
}

fn read_sized(data: &[u8], pos: usize, i: usize) -> Result<(Vec<u8>, usize)> {
    if data.len() < pos + 4 {
        bail!("article truncated: field {i} lacks its size");
    }
    let n = u32::from_be_bytes(data[pos..pos + 4].try_into()?) as usize;
    let start = pos + 4;
    if data.len() < start + n {
        bail!("article truncated: field {i} claims {n} bytes");
    }
    Ok((data[start..start + n].to_vec(), start + n))
}

/// A dictionary article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub word: String,
    pub fields: Vec<Field>,
}

impl Entry {
    /// Readable text of all textual fields (HTML/XDXF/Pango markup is stripped).
    pub fn plain_text(&self) -> String {
        let mut parts = Vec::new();
        for f in &self.fields {
            let Some(t) = f.text() else { continue };
            match f.kind {
                'h' | 'g' | 'x' => parts.push(strip_markup(&t)),
                'm' | 'l' | 't' | 'y' | 'k' | 'w' => parts.push(t),
                _ => {}
            }
        }
        parts.retain(|p| !p.trim().is_empty());
        parts.join("\n")
    }
}

/// Removes tags from simple HTML/XDXF/Pango markup and decodes basic entities.
pub fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut closed = false;
            for t in chars.by_ref() {
                if t == '>' {
                    closed = true;
                    break;
                }
                tag.push(t);
            }
            if !closed {
                out.push('<');
                out.push_str(&tag);
                break;
            }
            let name = tag
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(name.as_str(), "br" | "p" | "li" | "div" | "tr") && !out.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push(c);
        }
    }
    let decoded = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    decoded.trim().to_string()
}

enum Data {
    File { file: Mutex<File>, len: u64 },
    Mem(Vec<u8>),
}

impl Data {
    fn read(&self, offset: u64, size: u32) -> Result<Vec<u8>> {
        if size > MAX_ENTRY_BYTES {
            bail!("article of {size} bytes exceeds the {MAX_ENTRY_BYTES} byte limit");
        }
        let end = offset
            .checked_add(size as u64)
            .ok_or_else(|| anyhow!("article offset overflows"))?;
        match self {
            Data::Mem(v) => {
                if end > v.len() as u64 {
                    bail!(
                        "article at {offset}+{size} lies beyond the data file ({} bytes)",
                        v.len()
                    );
                }
                Ok(v[offset as usize..end as usize].to_vec())
            }
            Data::File { file, len } => {
                if end > *len {
                    bail!("article at {offset}+{size} lies beyond the data file ({len} bytes)");
                }
                let mut f = file
                    .lock()
                    .map_err(|_| anyhow!("dictionary file lock poisoned"))?;
                f.seek(SeekFrom::Start(offset))?;
                let mut buf = vec![0u8; size as usize];
                f.read_exact(&mut buf)?;
                Ok(buf)
            }
        }
    }
}

struct Key {
    folded: String,
    display: String,
    item: usize,
}

/// An opened StarDict dictionary.
pub struct StarDict {
    info: Info,
    items: Vec<IndexItem>,
    keys: Vec<Key>,
    data: Data,
}

fn sibling(ifo: &Path, ext: &str) -> PathBuf {
    let stem = ifo
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    ifo.with_file_name(format!("{stem}.{ext}"))
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

impl StarDict {
    /// Opens a dictionary from its `.ifo` file or from a directory containing one.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let ifo = if path.is_dir() {
            let mut found: Vec<PathBuf> = fs::read_dir(path)
                .with_context(|| format!("reading {}", path.display()))?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "ifo"))
                .collect();
            found.sort();
            found
                .into_iter()
                .next()
                .ok_or_else(|| anyhow!("no .ifo file in {}", path.display()))?
        } else {
            path.to_path_buf()
        };
        let text =
            fs::read_to_string(&ifo).with_context(|| format!("reading {}", ifo.display()))?;
        let info = parse_ifo(&text).with_context(|| format!("parsing {}", ifo.display()))?;

        let idx_path = sibling(&ifo, "idx");
        let idx_bytes =
            fs::read(&idx_path).with_context(|| format!("reading {}", idx_path.display()))?;
        if let Some(expected) = info.idxfilesize {
            if expected != idx_bytes.len() as u64 {
                bail!(
                    "{}: idxfilesize says {expected} bytes but the file has {}",
                    idx_path.display(),
                    idx_bytes.len()
                );
            }
        }
        let items = parse_index(&idx_bytes, info.idxoffsetbits)
            .with_context(|| format!("parsing {}", idx_path.display()))?;
        if items.len() as u64 != info.wordcount {
            bail!(
                "wordcount says {} but the index has {} entries",
                info.wordcount,
                items.len()
            );
        }

        let dz = sibling(&ifo, "dict.dz");
        let plain = sibling(&ifo, "dict");
        let data = if dz.is_file() {
            let f = File::open(&dz).with_context(|| format!("opening {}", dz.display()))?;
            let mut buf = Vec::new();
            GzDecoder::new(f)
                .take(MAX_DICT_BYTES + 1)
                .read_to_end(&mut buf)
                .with_context(|| format!("inflating {}", dz.display()))?;
            if buf.len() as u64 > MAX_DICT_BYTES {
                bail!(
                    "{} inflates to more than {MAX_DICT_BYTES} bytes",
                    dz.display()
                );
            }
            Data::Mem(buf)
        } else if plain.is_file() {
            let f = File::open(&plain)?;
            let len = f.metadata()?.len();
            Data::File {
                file: Mutex::new(f),
                len,
            }
        } else {
            bail!(
                "missing dictionary data file ({} or .dict.dz)",
                plain.display()
            );
        };

        let mut keys: Vec<Key> = items
            .iter()
            .enumerate()
            .map(|(i, it)| Key {
                folded: fold(&it.word),
                display: it.word.clone(),
                item: i,
            })
            .collect();

        let syn_path = sibling(&ifo, "syn");
        if syn_path.is_file() {
            let bytes = fs::read(&syn_path)?;
            let mut pos = 0;
            while pos < bytes.len() {
                let rel = bytes[pos..]
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or_else(|| anyhow!("synonym file truncated at byte {pos}"))?;
                let word = std::str::from_utf8(&bytes[pos..pos + rel])
                    .map_err(|_| anyhow!("synonym at byte {pos} is not valid UTF-8"))?;
                let p = pos + rel + 1;
                if bytes.len() < p + 4 {
                    bail!("synonym file truncated after {word:?}");
                }
                let target = u32::from_be_bytes(bytes[p..p + 4].try_into()?) as usize;
                if target >= items.len() {
                    bail!("synonym {word:?} points to missing entry {target}");
                }
                keys.push(Key {
                    folded: fold(word),
                    display: word.to_string(),
                    item: target,
                });
                pos = p + 4;
            }
        }
        keys.sort_by(|a, b| a.folded.cmp(&b.folded).then(a.item.cmp(&b.item)));

        Ok(Self {
            info,
            items,
            keys,
            data,
        })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn name(&self) -> &str {
        &self.info.bookname
    }

    /// Number of headwords in the index (synonyms not counted).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn read_entry(&self, item: usize) -> Result<Entry> {
        let it = &self.items[item];
        let bytes = self.data.read(it.offset, it.size)?;
        let fields = decode_fields(&bytes, self.info.sametypesequence.as_deref())
            .with_context(|| format!("decoding article {:?}", it.word))?;
        Ok(Entry {
            word: it.word.clone(),
            fields,
        })
    }

    /// Case-insensitive exact lookup (headwords and synonyms). Articles whose
    /// headword matches the query's case exactly come first.
    pub fn lookup(&self, word: &str) -> Result<Vec<Entry>> {
        let key = fold(word);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let start = self
            .keys
            .partition_point(|k| k.folded.as_str() < key.as_str());
        let mut seen = Vec::new();
        for k in &self.keys[start..] {
            if k.folded != key {
                break;
            }
            if !seen.contains(&k.item) {
                seen.push(k.item);
            }
        }
        let mut entries = seen
            .into_iter()
            .map(|i| self.read_entry(i))
            .collect::<Result<Vec<_>>>()?;
        let exact = word.trim();
        entries.sort_by_key(|e| e.word != exact);
        Ok(entries)
    }

    /// Headwords (and synonyms) starting with `prefix`, case-insensitively, in index order.
    pub fn suggest(&self, prefix: &str, limit: usize) -> Vec<String> {
        let key = fold(prefix);
        let start = self
            .keys
            .partition_point(|k| k.folded.as_str() < key.as_str());
        let mut out: Vec<String> = Vec::new();
        for k in &self.keys[start..] {
            if out.len() >= limit || !k.folded.starts_with(&key) {
                break;
            }
            if !out.contains(&k.display) {
                out.push(k.display.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{write_dict, DictSpec, TmpDir};
    use std::io::Write;

    fn m(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    fn sample(dir: &Path, bits: u8, dz: bool) -> PathBuf {
        let mut spec = DictSpec::new(
            "sample",
            vec![
                ("zebra", m("striped horse")),
                ("Apple", m("a fruit")),
                ("apple", m("lowercase apple")),
                ("café", m("coffee shop ☕")),
                ("Ünïcode", m("ünï")),
                ("apply", m("to put on")),
            ],
        );
        spec.bits = bits;
        spec.dz = dz;
        write_dict(dir, &spec)
    }

    #[test]
    fn ifo_parsing_happy_path_and_fields() {
        let i = parse_ifo("StarDict's dict ifo file\nversion=3.0.0\nbookname=My Dict\nwordcount=2\nidxfilesize=10\nsametypesequence=h\nidxoffsetbits=64\nauthor=A\ndescription=D\nsynwordcount=3\nunknown=1\n").unwrap();
        assert_eq!(i.bookname, "My Dict");
        assert_eq!(i.wordcount, 2);
        assert_eq!(i.idxfilesize, Some(10));
        assert_eq!(i.sametypesequence.as_deref(), Some("h"));
        assert_eq!(i.idxoffsetbits, 64);
        assert_eq!(i.author.as_deref(), Some("A"));
        assert_eq!(i.description.as_deref(), Some("D"));
        assert_eq!(i.synwordcount, 3);
    }

    #[test]
    fn ifo_accepts_bom_and_defaults_to_32_bits() {
        let i = parse_ifo("\u{feff}StarDict's dict ifo file\nbookname=B\nwordcount=0\n").unwrap();
        assert_eq!(i.idxoffsetbits, 32);
        assert_eq!(i.version, "");
    }

    #[test]
    fn ifo_errors() {
        let e = |s: &str| parse_ifo(s).unwrap_err().to_string();
        assert!(e("hello\n").contains("bad magic"));
        assert!(e("").contains("bad magic"));
        assert!(e("StarDict's dict ifo file\nwordcount=1\n").contains("bookname"));
        assert!(e("StarDict's dict ifo file\nbookname=x\n").contains("wordcount"));
        assert!(e("StarDict's dict ifo file\nbookname=x\nwordcount=z\n").contains("wordcount"));
        assert!(
            e("StarDict's dict ifo file\nbookname=x\nwordcount=1\nidxoffsetbits=16\n")
                .contains("idxoffsetbits")
        );
        assert!(
            e("StarDict's dict ifo file\nbookname=x\nwordcount=1\nidxfilesize=q\n")
                .contains("idxfilesize")
        );
        assert!(
            e("StarDict's dict ifo file\nbookname=x\nwordcount=1\nsynwordcount=q\n")
                .contains("synwordcount")
        );
    }

    #[test]
    fn index_hand_assembled_32_and_64() {
        let b32: &[u8] = b"ab\0\0\0\0\x05\0\0\0\x03cd\0\0\0\x01\0\0\0\0\x07";
        let items = parse_index(b32, 32).unwrap();
        assert_eq!(
            items,
            vec![
                IndexItem {
                    word: "ab".into(),
                    offset: 5,
                    size: 3
                },
                IndexItem {
                    word: "cd".into(),
                    offset: 256,
                    size: 7
                }
            ]
        );
        let b64: &[u8] = b"x\0\0\0\0\x01\0\0\0\0\0\0\0\x09";
        let items = parse_index(b64, 64).unwrap();
        assert_eq!(items[0].offset, 0x0000_0001_0000_0000);
        assert_eq!(items[0].size, 9);
    }

    #[test]
    fn index_errors_never_panic() {
        assert!(parse_index(b"", 32).unwrap().is_empty());
        assert!(parse_index(b"abc", 32)
            .unwrap_err()
            .to_string()
            .contains("unterminated"));
        assert!(parse_index(b"abc\0\0\0", 32)
            .unwrap_err()
            .to_string()
            .contains("lacks offset"));
        assert!(parse_index(b"abc\0\0\0\0\0\0\0\0", 64)
            .unwrap_err()
            .to_string()
            .contains("lacks offset"));
        assert!(parse_index(b"\xff\xfe\0\0\0\0\0\0\0\0\0", 32)
            .unwrap_err()
            .to_string()
            .contains("UTF-8"));
        assert!(parse_index(b"a\0", 16).is_err());
        for n in 0..14 {
            let full = b"hello\0\0\0\0\x01\0\0\0\x02";
            let r = parse_index(&full[..n.min(full.len())], 32);
            assert_eq!(r.is_ok(), n == 0 || n == 14.min(full.len()));
        }
    }

    #[test]
    fn fields_sametypesequence_single_and_multi() {
        let f = decode_fields(b"hello", Some("m")).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text().unwrap(), "hello");
        let f = decode_fields(b"phon\0<b>def</b>", Some("mh")).unwrap();
        assert_eq!(f[0].kind, 'm');
        assert_eq!(f[0].text().unwrap(), "phon");
        assert_eq!(f[1].kind, 'h');
        assert_eq!(f[1].text().unwrap(), "<b>def</b>");
        let f = decode_fields(b"\0\0\0\x03abcrest", Some("Wm")).unwrap();
        assert_eq!(f[0].data, b"abc");
        assert!(f[0].text().is_none());
        assert_eq!(f[1].data, b"rest");
    }

    #[test]
    fn fields_without_sametypesequence() {
        let f = decode_fields(b"mone\0hTwo\0W\0\0\0\x02xyg last", None).unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].text().unwrap(), "one");
        assert_eq!(f[1].text().unwrap(), "Two");
        assert_eq!(f[2].data, b"xy");
        assert_eq!(f[3].kind, 'g');
        assert_eq!(f[3].text().unwrap(), " last");
        assert!(decode_fields(b"", None).unwrap().is_empty());
        let f = decode_fields(b"mno terminator", None).unwrap();
        assert_eq!(f[0].text().unwrap(), "no terminator");
    }

    #[test]
    fn fields_errors() {
        assert!(decode_fields(b"abc", Some("mh")).is_err());
        assert!(decode_fields(b"ab", Some("W1")).is_err());
        assert!(decode_fields(b"\0\0\0", Some("Wm")).is_err());
        assert!(decode_fields(b"\0\0\0\x09ab", Some("Wm")).is_err());
        assert!(decode_fields(b"1abc", None).is_err());
        assert!(decode_fields(b"W\0\0", None).is_err());
        assert!(decode_fields(b"W\0\0\0\x09a", None).is_err());
        assert!(decode_fields(b"x", Some("1")).is_err());
    }

    #[test]
    fn markup_stripping() {
        assert_eq!(strip_markup("<b>bold</b> &amp; <i>it</i>"), "bold & it");
        assert_eq!(strip_markup("a<br>b<br/>c"), "a\nb\nc");
        assert_eq!(
            strip_markup("&lt;x&gt; &quot;q&quot; &apos;a&apos;&nbsp;z"),
            "<x> \"q\" 'a' z"
        );
        assert_eq!(strip_markup("<p>one</p><p>two</p>"), "one\ntwo");
        assert_eq!(strip_markup("broken <b"), "broken <b");
        assert_eq!(strip_markup(""), "");
    }

    #[test]
    fn entry_plain_text_per_type() {
        let e = Entry {
            word: "w".into(),
            fields: vec![
                Field {
                    kind: 'm',
                    data: m("plain"),
                },
                Field {
                    kind: 'h',
                    data: m("<b>html</b>"),
                },
                Field {
                    kind: 'x',
                    data: m("<k>x</k><def>dxdf</def>"),
                },
                Field {
                    kind: 'W',
                    data: vec![1, 2],
                },
                Field {
                    kind: 't',
                    data: m("   "),
                },
            ],
        };
        assert_eq!(e.plain_text(), "plain\nhtml\nxdxdf");
    }

    #[test]
    fn open_lookup_32bit_plain_and_dz_agree() {
        for dz in [false, true] {
            let t = TmpDir::new("sd");
            let ifo = sample(t.path(), 32, dz);
            let d = StarDict::open(&ifo).unwrap();
            assert_eq!(d.name(), "sample");
            assert_eq!(d.len(), 6);
            assert!(!d.is_empty());
            assert_eq!(d.info().wordcount, 6);
            let r = d.lookup("zebra").unwrap();
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].plain_text(), "striped horse");
            assert_eq!(d.lookup("ZEBRA").unwrap()[0].word, "zebra");
        }
    }

    #[test]
    fn open_by_directory_and_64bit() {
        let t = TmpDir::new("sd64");
        sample(t.path(), 64, false);
        let d = StarDict::open(t.path()).unwrap();
        assert_eq!(d.info().idxoffsetbits, 64);
        assert_eq!(d.lookup("café").unwrap()[0].plain_text(), "coffee shop ☕");
        let t2 = TmpDir::new("sd64dz");
        sample(t2.path(), 64, true);
        let d = StarDict::open(t2.path()).unwrap();
        assert_eq!(d.lookup("apply").unwrap()[0].plain_text(), "to put on");
    }

    #[test]
    fn duplicates_exact_case_first() {
        let t = TmpDir::new("dup");
        let ifo = sample(t.path(), 32, false);
        let d = StarDict::open(&ifo).unwrap();
        let r = d.lookup("apple").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].word, "apple");
        assert_eq!(r[1].word, "Apple");
        let r = d.lookup("Apple").unwrap();
        assert_eq!(r[0].word, "Apple");
        assert_eq!(r[0].plain_text(), "a fruit");
    }

    #[test]
    fn unicode_lookup_and_empty_query() {
        let t = TmpDir::new("uni");
        let d = StarDict::open(sample(t.path(), 32, false)).unwrap();
        assert_eq!(d.lookup("CAFÉ").unwrap().len(), 1);
        assert_eq!(d.lookup("ünïcode").unwrap().len(), 1);
        assert!(d.lookup("").unwrap().is_empty());
        assert!(d.lookup("   ").unwrap().is_empty());
        assert!(d.lookup("missing").unwrap().is_empty());
        assert!(d.lookup("zzzz").unwrap().is_empty());
        assert!(d.lookup("a").unwrap().is_empty());
    }

    #[test]
    fn suggest_prefix_limit_and_order() {
        let t = TmpDir::new("sug");
        let d = StarDict::open(sample(t.path(), 32, false)).unwrap();
        assert_eq!(d.suggest("app", 10), vec!["Apple", "apple", "apply"]);
        assert_eq!(d.suggest("APP", 2), vec!["Apple", "apple"]);
        assert!(d.suggest("app", 0).is_empty());
        assert!(d.suggest("q", 5).is_empty());
        assert_eq!(d.suggest("", 2).len(), 2);
        assert_eq!(d.suggest("caf", 5), vec!["café"]);
    }

    #[test]
    fn synonyms_resolve_to_headword() {
        let t = TmpDir::new("syn");
        let mut spec = DictSpec::new("syn", vec![("alpha", m("first")), ("beta", m("second"))]);
        // sorted index: alpha=0, beta=1
        spec.syn = vec![("aleph", 0), ("Beta", 1), ("b", 1)];
        let d = StarDict::open(write_dict(t.path(), &spec)).unwrap();
        assert_eq!(d.lookup("aleph").unwrap()[0].word, "alpha");
        let r = d.lookup("beta").unwrap();
        assert_eq!(
            r.len(),
            1,
            "headword and synonym of one entry are deduplicated"
        );
        assert!(d.suggest("al", 5).contains(&"aleph".to_string()));
    }

    #[test]
    fn synonym_errors() {
        let t = TmpDir::new("synbad");
        let mut spec = DictSpec::new("s", vec![("a", m("x"))]);
        spec.syn = vec![("q", 5)];
        let err = StarDict::open(write_dict(t.path(), &spec)).err().unwrap();
        assert!(err.to_string().contains("missing entry"));

        let syn = t.path().join("s.syn");
        fs::write(&syn, b"abc").unwrap();
        assert!(StarDict::open(t.path().join("s.ifo")).is_err());
        fs::write(&syn, b"abc\0\0").unwrap();
        assert!(StarDict::open(t.path().join("s.ifo")).is_err());
        fs::write(&syn, b"\xff\0\0\0\0\0").unwrap();
        assert!(StarDict::open(t.path().join("s.ifo")).is_err());
    }

    #[test]
    fn missing_files_are_errors() {
        let t = TmpDir::new("miss");
        assert!(StarDict::open(t.path().join("nope.ifo")).is_err());
        assert!(StarDict::open(t.path())
            .err()
            .unwrap()
            .to_string()
            .contains("no .ifo"));
        let ifo = sample(t.path(), 32, false);
        fs::remove_file(t.path().join("sample.dict")).unwrap();
        assert!(StarDict::open(&ifo)
            .err()
            .unwrap()
            .to_string()
            .contains("missing dictionary data"));
        fs::remove_file(t.path().join("sample.idx")).unwrap();
        assert!(StarDict::open(&ifo).is_err());
    }

    #[test]
    fn truncated_idx_is_rejected() {
        let t = TmpDir::new("trunc");
        let ifo = sample(t.path(), 32, false);
        let idx = t.path().join("sample.idx");
        let bytes = fs::read(&idx).unwrap();
        for cut in [1usize, 3, 7, bytes.len() - 1] {
            fs::write(&idx, &bytes[..bytes.len() - cut]).unwrap();
            assert!(StarDict::open(&ifo).is_err(), "cut {cut}");
        }
        // wrong idxfilesize only
        fs::write(&idx, &bytes).unwrap();
        let text = fs::read_to_string(&ifo)
            .unwrap()
            .replace("idxfilesize=", "idxfilesize=9");
        fs::write(&ifo, text).unwrap();
        let e = StarDict::open(&ifo).err().unwrap();
        assert!(format!("{e:#}").contains("idxfilesize"));
    }

    #[test]
    fn wordcount_mismatch_is_rejected() {
        let t = TmpDir::new("wc");
        let ifo = sample(t.path(), 32, false);
        let text = fs::read_to_string(&ifo)
            .unwrap()
            .replace("wordcount=6", "wordcount=7");
        fs::write(&ifo, text).unwrap();
        let e = StarDict::open(&ifo).err().unwrap();
        assert!(e.to_string().contains("wordcount"));
    }

    #[test]
    fn bad_offsets_error_instead_of_panicking() {
        for dz in [false, true] {
            let t = TmpDir::new("badoff");
            let ifo = sample(t.path(), 32, dz);
            let idx = t.path().join("sample.idx");
            let mut bytes = fs::read(&idx).unwrap();
            // make "Apple" (first record) point far beyond the data file
            let p = bytes.iter().position(|&b| b == 0).unwrap() + 1;
            bytes[p..p + 4].copy_from_slice(&0x00ff_0000u32.to_be_bytes());
            fs::write(&idx, bytes).unwrap();
            let d = StarDict::open(&ifo).unwrap();
            let e = d.lookup("apple").unwrap_err();
            assert!(e.to_string().contains("beyond"), "{e}");
            // other words still fine
            assert_eq!(d.lookup("zebra").unwrap().len(), 1);
        }
    }

    #[test]
    fn oversized_and_overflowing_entries_error() {
        let d = Data::Mem(vec![0; 10]);
        assert!(d.read(0, MAX_ENTRY_BYTES + 1).is_err());
        assert!(d.read(u64::MAX, 5).is_err());
        assert!(d.read(8, 3).is_err());
        assert_eq!(d.read(8, 2).unwrap().len(), 2);
        assert_eq!(d.read(10, 0).unwrap().len(), 0);
    }

    #[test]
    fn corrupt_gzip_data_is_an_error() {
        let t = TmpDir::new("gz");
        let ifo = sample(t.path(), 32, true);
        fs::write(t.path().join("sample.dict.dz"), b"not gzip at all").unwrap();
        assert!(StarDict::open(&ifo).is_err());
    }

    #[test]
    fn dz_preferred_over_plain_when_both_exist() {
        let t = TmpDir::new("both");
        let ifo = sample(t.path(), 32, true);
        fs::write(t.path().join("sample.dict"), b"garbage").unwrap();
        let d = StarDict::open(&ifo).unwrap();
        assert_eq!(d.lookup("zebra").unwrap()[0].plain_text(), "striped horse");
    }

    #[test]
    fn sametypesequence_variants_in_files() {
        let t = TmpDir::new("sts");
        let mut spec = DictSpec::new("h", vec![("a", b"<b>bold</b><br>line".to_vec())]);
        spec.sts = Some("h");
        let d = StarDict::open(write_dict(t.path(), &spec)).unwrap();
        assert_eq!(d.lookup("a").unwrap()[0].plain_text(), "bold\nline");

        let t = TmpDir::new("sts2");
        let mut spec = DictSpec::new("n", vec![("a", b"mtext\0hhtml".to_vec())]);
        spec.sts = None;
        let d = StarDict::open(write_dict(t.path(), &spec)).unwrap();
        assert_eq!(d.lookup("a").unwrap()[0].fields.len(), 2);
    }

    #[test]
    fn dz_file_is_a_valid_gzip_stream() {
        let t = TmpDir::new("gzv");
        sample(t.path(), 32, true);
        let raw = fs::read(t.path().join("sample.dict.dz")).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b]);
        let mut out = Vec::new();
        GzDecoder::new(&raw[..]).read_to_end(&mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("striped horse"));
        let _ = std::io::sink().write(&[]);
    }

    #[test]
    fn sorted_lookup_matches_linear_scan_for_many_words() {
        let t = TmpDir::new("many");
        let words: Vec<String> = (0..500)
            .map(|i| format!("w{:03}x{}", (i * 37) % 500, i))
            .collect();
        let entries: Vec<(&str, Vec<u8>)> = words
            .iter()
            .map(|w| (w.as_str(), w.as_bytes().to_vec()))
            .collect();
        let d = StarDict::open(write_dict(t.path(), &DictSpec::new("many", entries))).unwrap();
        for w in &words {
            let r = d.lookup(w).unwrap();
            assert_eq!(r.len(), 1);
            assert_eq!(r[0].plain_text(), *w);
        }
        assert_eq!(
            d.suggest("w00", 1000).len(),
            words.iter().filter(|w| w.starts_with("w00")).count()
        );
    }
}
