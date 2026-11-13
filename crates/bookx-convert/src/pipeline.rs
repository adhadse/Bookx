//! The conversion pipeline: read -> transforms -> write, with progress and cancellation.

use std::cell::Cell;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{anyhow, bail, Context, Result};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{pick_reader, FormatReader, FormatWriter, Registry, TransformReport};
use bookx_core::convert::{import_to_epub, is_importable};

use crate::mobi::{Azw3Writer, MobiWriter};
use crate::options::ConvertOptions;
use crate::writers::{default_writers, EpubWriter, KepubWriter};

/// Returned (inside `anyhow::Error`) when the cancel flag was raised. Check with
/// `err.downcast_ref::<Cancelled>()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conversion cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// 0..=100, never decreasing within one conversion.
    pub percent: u8,
    pub stage: String,
}

#[derive(Debug)]
pub struct Converted {
    pub bytes: Vec<u8>,
    /// Extension of the output without the dot (`"epub"`, `"kepub.epub"`, `"mobi"` …).
    pub extension: String,
    pub reports: Vec<(String, TransformReport)>,
    /// What the output format could not hold and what the transforms changed.
    pub report: crate::report::ConversionReport,
}

struct Tracker<'a> {
    cb: &'a dyn Fn(Progress),
    cancel: &'a AtomicBool,
    last: Cell<u8>,
}

impl Tracker<'_> {
    fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(anyhow!(Cancelled));
        }
        Ok(())
    }

    fn report(&self, percent: u8, stage: &str) {
        let p = percent.min(100).max(self.last.get());
        self.last.set(p);
        (self.cb)(Progress {
            percent: p,
            stage: stage.to_string(),
        });
    }
}

fn normalized_ext(ext: &str) -> String {
    let e = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if e == "kepub.epub" {
        "kepub".into()
    } else {
        e
    }
}

/// Opens the input: EPUB directly, other formats through `readers`, and TXT/Markdown/HTML through
/// the built-in importer when no reader claims them.
pub fn read_input(
    readers: &Registry<dyn FormatReader>,
    bytes: &[u8],
    input_name: &str,
) -> Result<OpenBook> {
    let lower = input_name.to_ascii_lowercase();
    let archive = if lower.ends_with(".epub") || lower.ends_with(".kepub") {
        Archive::from_bytes(bytes.to_vec())?
    } else if let Ok(reader) = pick_reader(readers, input_name, &bytes[..bytes.len().min(512)]) {
        reader
            .read(bytes)
            .with_context(|| format!("reading {input_name} as {}", reader.id()))?
    } else if is_importable(Path::new(input_name)) {
        import_with_core(bytes, input_name)?
    } else if bytes.starts_with(b"PK")
        && Archive::from_bytes(bytes.to_vec()).is_ok_and(|a| a.contains("META-INF/container.xml"))
    {
        Archive::from_bytes(bytes.to_vec())?
    } else {
        bail!("no reader for `{input_name}`");
    };
    let mut book = OpenBook::from_archive(archive)?;
    title_from_file_name(&mut book, input_name);
    Ok(book)
}

/// Books that came without a title (plain text, some PDB/RB files) are named after the file, like
/// Calibre does, instead of "Untitled".
fn title_from_file_name(book: &mut OpenBook, input_name: &str) {
    let title = book.metadata().title.trim().to_string();
    if !title.is_empty() && !title.eq_ignore_ascii_case("untitled") && title != "Unknown" {
        return;
    }
    let base = input_name.rsplit(['/', '\\']).next().unwrap_or(input_name);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s).trim();
    if stem.is_empty() {
        return;
    }
    let mut md = book.metadata().clone();
    md.title = stem.to_string();
    let _ = book.set_metadata(md);
}

fn import_with_core(bytes: &[u8], input_name: &str) -> Result<Archive> {
    let file = Path::new(input_name)
        .file_name()
        .and_then(|f| f.to_str())
        .filter(|f| !f.is_empty())
        .unwrap_or("input.txt");
    let dir = std::env::temp_dir().join(format!("bookx-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file);
    let result = std::fs::write(&path, bytes)
        .map_err(anyhow::Error::from)
        .and_then(|_| import_to_epub(&path));
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn pick_writer(output_ext: &str, options: &ConvertOptions) -> Result<Box<dyn FormatWriter>> {
    let ext = normalized_ext(output_ext);
    Ok(match ext.as_str() {
        "epub" => {
            let mut w = EpubWriter::default();
            if let Some(v) = options.epub_version {
                w.version = v;
            }
            Box::new(w)
        }
        "kepub" => {
            let mut w = KepubWriter::default();
            if let Some(v) = options.epub_version {
                w.version = v;
            }
            Box::new(w)
        }
        "mobi" => Box::new(MobiWriter {
            inline_toc: options.mobi_inline_toc,
        }),
        "azw3" | "kf8" => Box::new(Azw3Writer),
        "fb2" => Box::new(crate::writers::Fb2Writer),
        "htmlz" => Box::new(crate::writers::HtmlzWriter),
        "txtz" => Box::new(crate::writers::TxtzWriter),
        "txt" => Box::new(crate::writers::TxtWriter),
        "md" | "markdown" => Box::new(crate::writers::MarkdownWriter),
        "html" | "htm" => Box::new(crate::writers::HtmlWriter),
        "docx" => Box::new(crate::writers::DocxWriter {
            options: options.docx.clone(),
        }),
        "rtf" => Box::new(crate::writers::RtfWriter),
        "pdb" => Box::new(crate::writers::EReaderWriter {
            compression: options.pdb_compression,
        }),
        "pmlz" => Box::new(crate::writers::PmlzWriter),
        "rb" => Box::new(crate::writers::RbWriter),
        "snb" => Box::new(crate::writers::SnbWriter),
        "oeb" => Box::new(crate::writers::OebWriter),
        "zip" => Box::new(crate::writers::ZipWriter),
        other => {
            let known: Vec<&str> = default_writers().iter().map(|w| w.id()).collect();
            bail!(
                "unsupported output format `{other}` (supported: {})",
                known.join(", ")
            )
        }
    })
}

/// Converts `input` to `output_ext` applying the transforms selected by `options`.
///
/// `progress` is called with non-decreasing percentages and ends with 100. `cancel` is checked
/// between stages; raising it makes the function return an error that downcasts to [`Cancelled`].
pub fn convert(
    readers: &Registry<dyn FormatReader>,
    input: &[u8],
    input_name: &str,
    output_ext: &str,
    options: &ConvertOptions,
    progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<Converted> {
    let t = Tracker {
        cb: progress,
        cancel,
        last: Cell::new(0),
    };
    t.check()?;
    // fail early on unsupported targets
    let writer = pick_writer(output_ext, options)?;
    t.report(0, "reading");
    let mut book = read_input(readers, input, input_name)?;
    t.report(10, "read");
    t.check()?;

    let transforms = options.transforms();
    let n = transforms.len().max(1);
    let mut reports = Vec::new();
    for (i, tr) in transforms.iter().enumerate() {
        t.check()?;
        t.report((10 + 70 * i / n) as u8, tr.id());
        let r = tr
            .apply(&mut book)
            .with_context(|| format!("transform `{}` failed", tr.id()))?;
        reports.push((tr.id().to_string(), r));
        t.report((10 + 70 * (i + 1) / n) as u8, tr.id());
    }
    t.check()?;
    let report = crate::report::build_report(&book, &normalized_ext(output_ext), &reports);
    t.report(85, "writing");
    let bytes = writer
        .write(&book)
        .with_context(|| format!("writing {}", writer.extension()))?;
    t.check()?;
    t.report(100, "done");
    Ok(Converted {
        bytes,
        extension: writer.extension().to_string(),
        reports,
        report,
    })
}

/// Like [`convert`], reading and writing files. The output appears atomically (temporary file +
/// rename) and nothing is left behind when the conversion fails or is cancelled.
pub fn convert_file(
    readers: &Registry<dyn FormatReader>,
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    progress: &dyn Fn(Progress),
    cancel: &AtomicBool,
) -> Result<Converted> {
    let bytes = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    let name = input
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("input");
    let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("");
    let ext = if output
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".kepub.epub"))
    {
        "kepub"
    } else {
        ext
    };
    let out = convert(readers, &bytes, name, ext, options, progress, cancel)?;
    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = output.with_file_name(format!(
        ".{}.bookx-tmp",
        output.file_name().and_then(|n| n.to_str()).unwrap_or("out")
    ));
    let write = std::fs::write(&tmp, &out.bytes).and_then(|_| {
        if cancel.load(Ordering::SeqCst) {
            Err(std::io::Error::other(Cancelled))
        } else {
            std::fs::rename(&tmp, output)
        }
    });
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        if e.get_ref().is_some_and(|inner| inner.is::<Cancelled>()) {
            return Err(anyhow!(Cancelled));
        }
        return Err(e).with_context(|| format!("writing {}", output.display()));
    }
    Ok(out)
}
