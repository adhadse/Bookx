#![forbid(unsafe_code)]
//! `bookx-cli`: the command-line face of Bookx (roadmap X2/X12, decision D7).
//!
//! All behaviour lives in [`run`], which takes its arguments and output streams as parameters, so
//! every command is unit-testable without spawning a process. Exit codes: `0` success,
//! `1` the command ran but found problems (for example `check` found errors),
//! `2` usage or runtime failure.

mod backup;
mod catalogue;
mod columns;
mod device_check;
mod extras;
mod highlights;
mod libtools;
mod mcp;
mod more;
mod replace;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bookx_convert::transforms::{AutoTocMode, ProfileKind};
use bookx_convert::writers::EpubVersion;
use bookx_convert::{ConvertOptions, PresetStore, builtin_presets, convert_file, default_writers};
use bookx_core::archive::Archive;
use bookx_core::book::OpenBook;
use bookx_core::check::{Severity, check_book};
use bookx_core::library::{AddOutcome, Library, SortKey};
use bookx_core::opf::Metadata;
use bookx_formats::{default_readers, read_any, supported_extensions};
use bookx_serve::server::{LibrarySnapshot, Server, ServerConfig, User};
use clap::{Parser, Subcommand};

thread_local! {
    static TEST_VARS: std::cell::RefCell<std::collections::HashMap<String, Option<String>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// An environment variable as the commands see it. A test can change what *its own thread* sees with
/// [`override_var`] (the process environment is shared by every test thread, and writing it is unsafe in Rust 2024).
pub(crate) fn var(name: &str) -> Option<String> {
    if let Some(v) = TEST_VARS.with(|m| m.borrow().get(name).cloned()) {
        return v;
    }
    std::env::var(name).ok()
}

/// For tests: from now on this thread sees `name` as `value` (`None`: not set), whatever the real environment says.
pub fn override_var(name: &str, value: Option<&str>) {
    TEST_VARS.with(|m| {
        m.borrow_mut()
            .insert(name.to_string(), value.map(String::from))
    });
}

/// Process-level context that tests can replace.
pub struct Env {
    /// Library used when `--library` is not given.
    pub default_library: PathBuf,
    /// Raised to stop `serve`.
    pub stop: Arc<AtomicBool>,
}

impl Env {
    pub fn from_process() -> Env {
        let default_library = std::env::var_os("BOOKX_LIBRARY")
            .map(PathBuf::from)
            .unwrap_or_else(default_library_dir);
        Env {
            default_library,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Same default as the GTK app: `~/Documents/Bookx Library`.
pub fn default_library_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.document_dir().map(|p| p.join("Bookx Library")))
        .or_else(|| directories::BaseDirs::new().map(|d| d.data_dir().join("bookx/Library")))
        .unwrap_or_else(|| PathBuf::from("Bookx Library"))
}

#[derive(Parser, Debug)]
#[command(
    name = "bookx-cli",
    version,
    about = "Convert, inspect, check and manage e-books"
)]
struct Cli {
    /// Library folder (default: $BOOKX_LIBRARY or ~/Documents/Bookx Library)
    #[arg(long, global = true)]
    library: Option<PathBuf>,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List the input and output formats
    Formats {
        #[arg(long)]
        json: bool,
    },
    /// Convert a book to another format (chosen by the output extension)
    Convert(ConvertArgs),
    /// Show or change metadata
    #[command(subcommand)]
    Meta(MetaCmd),
    /// Print what a book contains: metadata, table of contents tree, word, image and link counts
    Inspect {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Open a book in the Bookx editor (starts the app): `edit book.epub OEBPS/ch1.xhtml --select-text "some words"`
    Edit {
        file: PathBuf,
        /// Files inside the book to open in tabs
        names: Vec<String>,
        /// Select the first occurrence of this text in the last file
        #[arg(long)]
        select_text: Option<String>,
    },
    /// Check an e-book for structural problems
    Check {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Validate the folder layout of a mounted e-reader and the books on it
    DeviceCheck {
        /// Mount point of the device
        mount: PathBuf,
        /// kobo | kindle | pocketbook | generic (default: detected from the layout)
        #[arg(long)]
        kind: Option<String>,
        /// Check only this book (path relative to the mount, or a file name in the books folder)
        #[arg(long)]
        book: Option<String>,
        /// Check every book on the device, not only those in the folder Bookx sends to
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Manage the library
    #[command(subcommand)]
    Library(LibCmd),
    /// Search the library with the Calibre-style query language
    Search {
        query: String,
        #[arg(long, default_value = "title")]
        sort: String,
        #[arg(long)]
        desc: bool,
        #[arg(long)]
        json: bool,
    },
    /// Serve the library over OPDS / the web (and KOSync)
    Serve(ServeArgs),
    /// Back up the library (book files, covers, catalogue, notes) to a drive, share, cloud-drive
    /// folder or WebDAV address, incrementally; list, restore and check backups
    #[command(subcommand)]
    Backup(backup::BackupCmd),
    /// Fix highlights: `highlights repair` corrects colours and styles from what the e-readers say
    #[command(subcommand)]
    Highlights(highlights::HighlightsCmd),
    /// Run the Model Context Protocol server so AI assistants can use the library (stdio, or --http)
    Mcp(mcp::McpArgs),
    /// Print the library (or the books a search finds) as CSV or JSON with the columns you choose
    Catalog {
        /// csv, json or epub (a catalogue book; needs --output; convert it with `bookx-cli convert` for other formats)
        #[arg(long, default_value = "csv")]
        format: String,
        /// Comma separated: id,title,authors,series,index,tags,formats,publisher,language,published,size,
        /// and #label for a custom column
        #[arg(long)]
        columns: Option<String>,
        /// The file to write (needed for epub; csv and json go to the screen)
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// A library search (default: every book)
        query: Vec<String>,
    },
    /// Build the EPUB of a news recipe (TOML or JSON) from its feeds
    FetchNews { recipe: PathBuf, output: PathBuf },
    /// Repair a book with the editor's automatic fixes and write the result (exit 1 if problems remain)
    Polish {
        file: PathBuf,
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List the embedded fonts of a book, which are unused, and which families are not embedded
    Fonts {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// The searches saved in the app
    #[command(subcommand)]
    SavedSearch(more::SavedCmd),
    /// Plugins: run an editor action over the library
    #[command(subcommand)]
    Plugin(more::PluginCmd),
    /// List the built-in conversion presets
    Presets {
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args, Debug)]
struct ConvertArgs {
    input: PathBuf,
    output: PathBuf,
    /// Built-in preset name (see `presets`) or a JSON preset file
    #[arg(long)]
    preset: Option<String>,
    /// kindle | kobo | eink | tablet
    #[arg(long)]
    device: Option<String>,
    /// 2 or 3 (EPUB and KEPUB output)
    #[arg(long)]
    epub_version: Option<String>,
    #[arg(long)]
    smarten: bool,
    #[arg(long)]
    unsmarten: bool,
    /// Join hard-wrapped lines (threshold 0..1, default 0.4)
    #[arg(long, num_args = 0..=1, default_missing_value = "0.4")]
    unwrap_lines: Option<f64>,
    #[arg(long)]
    remove_unused_css: bool,
    #[arg(long)]
    remove_blank_paragraphs: bool,
    #[arg(long)]
    remove_fake_margins: bool,
    /// Build a table of contents from headings when missing; `force` replaces an existing one
    #[arg(long, num_args = 0..=1, default_missing_value = "if-missing")]
    auto_toc: Option<String>,
    #[arg(long)]
    split_large_html: Option<usize>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    author: Vec<String>,
    #[arg(long)]
    series: Option<String>,
    #[arg(long)]
    series_index: Option<f64>,
    #[arg(long)]
    tag: Vec<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    publisher: Option<String>,
    /// Image file to use as the cover
    #[arg(long)]
    cover: Option<PathBuf>,
    /// Worker threads (0 = all cores)
    #[arg(long)]
    jobs: Option<usize>,
    /// Do not print progress
    #[arg(long, short)]
    quiet: bool,
    /// Print what the output format lost and what was changed; give a file name to save it
    /// instead (`.json` for machine readable output)
    #[arg(long, num_args = 0..=1, default_missing_value = "-")]
    report: Option<String>,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)]
enum MetaCmd {
    /// Print the metadata of a book
    Show {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Change metadata of an EPUB in place
    Set(MetaSetArgs),
}

#[derive(clap::Args, Debug)]
struct MetaSetArgs {
    file: PathBuf,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    author: Vec<String>,
    #[arg(long)]
    series: Option<String>,
    #[arg(long)]
    series_index: Option<f64>,
    #[arg(long)]
    tag: Vec<String>,
    #[arg(long)]
    publisher: Option<String>,
    #[arg(long)]
    language: Option<String>,
    #[arg(long)]
    description: Option<String>,
    /// 0..=5 stars
    #[arg(long)]
    rating: Option<u8>,
    /// `scheme:value`, repeatable (for example isbn:9781234567890)
    #[arg(long)]
    identifier: Vec<String>,
    /// Image file to use as the cover
    #[arg(long)]
    cover: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum LibCmd {
    /// Add books or folders (other formats are converted to EPUB on the way in)
    Add {
        paths: Vec<PathBuf>,
        #[arg(long, short)]
        recursive: bool,
        #[arg(long)]
        allow_duplicates: bool,
    },
    /// List all books
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one book
    Info {
        id: u64,
        #[arg(long)]
        json: bool,
    },
    /// Remove books from the library
    Remove {
        ids: Vec<u64>,
        /// Also delete the files
        #[arg(long)]
        delete_files: bool,
    },
    /// Copy a book out of the library
    Export { id: u64, dest: PathBuf },
    /// Print the whole catalogue (every field of every book) as JSON: a backup and an export
    Dump,
    /// The custom columns: list, add, remove and set values
    #[command(subcommand)]
    Columns(columns::ColumnsCmd),
    /// Search and replace in one field of many books (one undo step; --dry-run only shows)
    Replace(replace::ReplaceArgs),
    #[command(flatten)]
    Tools(libtools::ToolsCmd),
    #[command(flatten)]
    Catalogue(catalogue::CatalogueCmd),
    #[command(flatten)]
    Extras(extras::ExtrasCmd),
}

#[derive(clap::Args, Debug)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    #[arg(long, default_value_t = 8080)]
    port: u16,
    /// `name:password`, repeatable; without users access is open
    #[arg(long)]
    user: Vec<String>,
    #[arg(long)]
    read_only: bool,
    /// Enable the KOReader progress-sync endpoints, stored in this JSON file
    #[arg(long)]
    kosync: Option<PathBuf>,
    #[arg(long, default_value = "Bookx Library")]
    title: String,
    /// Another host name the server may be reached by, such as the public name behind a reverse
    /// proxy (repeatable; also `BOOKX_ALLOWED_HOSTS`). IP addresses, `localhost` and `.local` names always work
    #[arg(long = "allow-host")]
    allow_host: Vec<String>,
    /// An extra login `name:password[:collection,collection]`, repeatable. The account sees only the
    /// books with one of the listed tags (all books when none are listed); the password is stored
    /// only as a hash in memory
    #[arg(long = "account")]
    account: Vec<String>,
    /// Collections (tags) that visitors without a login may read, repeatable; without it a login is always needed
    #[arg(long = "guest-collection")]
    guest_collection: Vec<String>,
    /// Address or network (`10.0.0.0/8`) of a reverse proxy whose `X-Forwarded-For` is believed, so the
    /// failed-login limit counts each real client (repeatable). Without it the header is never trusted
    #[arg(long = "trusted-proxy")]
    trusted_proxy: Vec<String>,
    /// The sub-path a reverse proxy publishes the server under, such as `/books`; links start with it
    #[arg(long = "public-prefix")]
    public_prefix: Option<String>,
    /// Serve HTTPS: PEM certificate chain (needs --tls-key)
    #[arg(long, requires = "tls_key")]
    tls_cert: Option<PathBuf>,
    /// Serve HTTPS: PEM private key (needs --tls-cert)
    #[arg(long, requires = "tls_cert")]
    tls_key: Option<PathBuf>,
}

/// Runs the command line. Returns the process exit code.
pub fn run(args: &[String], env: &Env, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 2 } else { 0 };
            let text = e.to_string();
            let _ = if code == 0 {
                write!(out, "{text}")
            } else {
                write!(err, "{text}")
            };
            return code;
        }
    };
    libtools::init();
    let root = cli
        .library
        .clone()
        .unwrap_or_else(|| env.default_library.clone());
    match dispatch(cli, &root, env, out, err) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(err, "error: {e:#}");
            2
        }
    }
}

/// The arguments `bookx` gets for `bookx-cli edit`.
pub fn edit_args(file: &Path, names: &[String], select: Option<&str>) -> Vec<String> {
    let mut a = vec!["--edit".to_string(), file.display().to_string()];
    a.extend(names.iter().cloned());
    if let Some(t) = select {
        a.push("--select-text".into());
        a.push(t.to_string());
    }
    a
}

/// `bookx` next to this program if there is one, else the one on the PATH.
fn bookx_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("bookx")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("bookx"))
}

fn edit(file: &Path, names: &[String], select: Option<&str>, err: &mut dyn Write) -> Result<i32> {
    if !file.exists() {
        writeln!(err, "error: {} does not exist", file.display())?;
        return Ok(2);
    }
    let file = std::fs::canonicalize(file)?;
    std::process::Command::new(bookx_binary())
        .args(edit_args(&file, names, select))
        .spawn()
        .map_err(|e| anyhow::anyhow!("could not start bookx: {e}"))?;
    Ok(0)
}

fn dispatch(
    cli: Cli,
    root: &Path,
    env: &Env,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<i32> {
    match cli.command {
        Cmd::Formats { json } => formats(json, out),
        Cmd::Presets { json } => presets(json, out),
        Cmd::Convert(a) => convert(a, out, err),
        Cmd::Meta(MetaCmd::Show { file, json }) => meta_show(&file, json, out),
        Cmd::Meta(MetaCmd::Set(a)) => meta_set(a, out),
        Cmd::Inspect { file, json } => inspect(&file, json, out),
        Cmd::Edit {
            file,
            names,
            select_text,
        } => edit(&file, &names, select_text.as_deref(), err),
        Cmd::Check { file, json } => check(&file, json, out),
        Cmd::DeviceCheck {
            mount,
            kind,
            book,
            all,
            json,
        } => {
            let kind = match kind {
                Some(k) => Some(
                    bookx_serve::devices::DeviceKind::from_slug(&k.to_lowercase()).ok_or_else(
                        || anyhow!("unknown device kind {k:?} (kobo, kindle, pocketbook, generic)"),
                    )?,
                ),
                None => None,
            };
            let report = device_check::check_device_with(&mount, kind, book.as_deref(), all)?;
            device_check::write_report(&report, json, out)?;
            Ok(if report.passed() { 0 } else { 1 })
        }
        Cmd::Library(c) => {
            migrate_note(root, err)?;
            library(c, root, out, err)
        }
        Cmd::Search {
            query,
            sort,
            desc,
            json,
        } => {
            migrate_note(root, err)?;
            search(root, &query, &sort, desc, json, out)
        }
        Cmd::Serve(a) => {
            migrate_note(root, err)?;
            serve(a, root, env, out)
        }
        Cmd::Highlights(c) => {
            migrate_note(root, err)?;
            highlights::run(c, root, out)
        }
        Cmd::Backup(c) => backup::run(c, root, out, err),
        Cmd::Mcp(a) => mcp::run(a, root, env, out, err),
        Cmd::Catalog {
            format,
            columns,
            output,
            query,
        } => {
            migrate_note(root, err)?;
            more::catalog(
                root,
                &query.join(" "),
                &format,
                columns.as_deref(),
                output.as_deref(),
                out,
            )
        }
        Cmd::FetchNews { recipe, output } => more::fetch_news(
            &recipe,
            &output,
            &bookx_sources::fetch::UreqFetcher::new(),
            out,
        ),
        Cmd::Polish { file, output, json } => more::polish(&file, &output, json, out),
        Cmd::Fonts { file, json } => more::fonts(&file, json, out),
        Cmd::SavedSearch(c) => {
            migrate_note(root, err)?;
            more::saved(c, root, out)
        }
        Cmd::Plugin(c) => {
            migrate_note(root, err)?;
            more::plugin(c, root, out)
        }
    }
}

/// Reads any supported file into an EPUB archive. EPUB itself is opened directly, and an `.epub`
/// that is not a valid archive is an error rather than being guessed at as text.
fn read_archive(name: &str, bytes: &[u8]) -> Result<Archive> {
    let is_epub_name = name.to_ascii_lowercase().ends_with(".epub");
    if is_epub_name || bytes.starts_with(b"PK") && looks_like_epub(bytes) {
        return Archive::from_bytes(bytes.to_vec())
            .with_context(|| format!("`{name}` is not a valid EPUB"));
    }
    read_any(name, bytes)
}

fn looks_like_epub(bytes: &[u8]) -> bool {
    // a stored `mimetype` entry is the first file of every EPUB
    bytes.len() > 58
        && &bytes[30..38] == b"mimetype"
        && bytes[38..].starts_with(b"application/epub+zip")
}

/// Book files below `dir` whose extension some reader understands.
fn collect_supported(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let exts = supported_extensions();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                collect_supported(&p, true, out);
            }
        } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            let e = e.to_ascii_lowercase();
            e == "epub" || exts.iter().any(|x| *x == e)
        }) {
            out.push(p);
        }
    }
}

// -- formats / presets ----------------------------------------------------------------------

fn formats(json: bool, out: &mut dyn Write) -> Result<i32> {
    let mut inputs = supported_extensions();
    if !inputs.contains(&"epub") {
        inputs.push("epub");
        inputs.sort();
    }
    let writers = default_writers();
    let mut outputs: Vec<(&str, &str)> = writers.iter().map(|w| (w.id(), w.extension())).collect();
    outputs.sort();
    if json {
        let v = serde_json::json!({
            "input": inputs,
            "output": outputs.iter().map(|(id, ext)| serde_json::json!({"id": id, "extension": ext})).collect::<Vec<_>>(),
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
    } else {
        writeln!(out, "Input:  {}", inputs.join(", "))?;
        let names: Vec<&str> = outputs.iter().map(|(_, ext)| *ext).collect();
        writeln!(out, "Output: {}", names.join(", "))?;
    }
    Ok(0)
}

fn presets(json: bool, out: &mut dyn Write) -> Result<i32> {
    let store = builtin_presets();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&store)?)?;
    } else {
        for p in &store.presets {
            writeln!(
                out,
                "{}  ->  {}",
                p.name,
                p.output_format.as_deref().unwrap_or("any")
            )?;
        }
    }
    Ok(0)
}

// -- convert ------------------------------------------------------------------------------

fn parse_device(s: &str) -> Result<ProfileKind> {
    ProfileKind::ALL
        .iter()
        .copied()
        .find(|k| k.id() == s.to_ascii_lowercase())
        .ok_or_else(|| {
            let ids: Vec<&str> = ProfileKind::ALL.iter().map(|k| k.id()).collect();
            anyhow!("unknown device `{s}` (use one of: {})", ids.join(", "))
        })
}

fn image_ext_ok(path: &Path) -> Result<Vec<u8>> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let known = data.starts_with(&[0xFF, 0xD8])
        || data.starts_with(b"\x89PNG")
        || data.starts_with(b"GIF8");
    if !known {
        bail!("{} is not a JPEG, PNG or GIF image", path.display());
    }
    Ok(data)
}

fn options_from(a: &ConvertArgs) -> Result<ConvertOptions> {
    let mut o = match &a.preset {
        None => ConvertOptions::default(),
        Some(name) => {
            let p = Path::new(name);
            if p.is_file() {
                let store = PresetStore::load(p)?;
                store
                    .presets
                    .into_iter()
                    .next()
                    .map(|p| p.options)
                    .ok_or_else(|| anyhow!("{} contains no preset", p.display()))?
            } else {
                builtin_presets()
                    .presets
                    .into_iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                    .map(|p| p.options)
                    .ok_or_else(|| anyhow!("unknown preset `{name}` (see `presets`)"))?
            }
        }
    };
    if let Some(d) = &a.device {
        o.device = Some(parse_device(d)?);
    }
    if let Some(v) = &a.epub_version {
        o.epub_version = Some(match v.as_str() {
            "2" => EpubVersion::V2,
            "3" => EpubVersion::V3,
            other => bail!("--epub-version must be 2 or 3, not `{other}`"),
        });
    }
    o.smarten_punctuation |= a.smarten;
    o.unsmarten_punctuation |= a.unsmarten;
    if a.smarten && a.unsmarten {
        bail!("--smarten and --unsmarten cannot be combined");
    }
    if let Some(t) = a.unwrap_lines {
        if !(0.0..=1.0).contains(&t) {
            bail!("--unwrap-lines must be between 0 and 1");
        }
        o.unwrap_lines = Some(t);
    }
    o.remove_unused_css |= a.remove_unused_css;
    o.remove_blank_paragraphs |= a.remove_blank_paragraphs;
    o.remove_fake_margins |= a.remove_fake_margins;
    if let Some(m) = &a.auto_toc {
        o.auto_toc = Some(match m.as_str() {
            "if-missing" => AutoTocMode::IfMissing,
            "force" => AutoTocMode::Force,
            other => bail!("--auto-toc takes `if-missing` or `force`, not `{other}`"),
        });
    }
    if let Some(n) = a.split_large_html {
        o.split_large_html = Some(n);
    }
    if let Some(j) = a.jobs {
        o.jobs = j;
    }
    if let Some(t) = &a.title {
        o.metadata.title = Some(t.clone());
    }
    if !a.author.is_empty() {
        o.metadata.authors = Some(a.author.clone());
    }
    if let Some(s) = &a.series {
        o.metadata.series = Some(s.clone());
    }
    if let Some(i) = a.series_index {
        o.metadata.series_index = Some(i);
    }
    if !a.tag.is_empty() {
        o.metadata.tags = Some(a.tag.clone());
    }
    if let Some(l) = &a.language {
        o.metadata.language = Some(l.clone());
    }
    if let Some(p) = &a.publisher {
        o.metadata.publisher = Some(p.clone());
    }
    if let Some(c) = &a.cover {
        o.metadata.cover = Some(image_ext_ok(c)?);
    }
    Ok(o)
}

fn convert(a: ConvertArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32> {
    if !a.input.is_file() {
        bail!("{} does not exist", a.input.display());
    }
    if a.input == a.output {
        bail!("input and output are the same file");
    }
    let options = options_from(&a)?;
    if a.input
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
    {
        writeln!(
            err,
            "note: PDF to e-book conversion is experimental (the text is reflowed; columns, tables and formulas may come out wrong)"
        )?;
    }
    let readers = default_readers();
    let cancel = AtomicBool::new(false);
    let quiet = a.quiet;
    let last = std::cell::Cell::new(255u8);
    let lines = std::cell::RefCell::new(Vec::<String>::new());
    let progress = |p: bookx_convert::Progress| {
        if !quiet && p.percent != last.get() {
            last.set(p.percent);
            lines
                .borrow_mut()
                .push(format!("{:>3}%  {}", p.percent, p.stage));
        }
    };
    let result = convert_file(&readers, &a.input, &a.output, &options, &progress, &cancel);
    for l in lines.borrow().iter() {
        writeln!(err, "{l}")?;
    }
    let done = result?;
    writeln!(
        out,
        "{} -> {} ({} bytes)",
        a.input.display(),
        a.output.display(),
        done.bytes.len()
    )?;
    if let Some(dest) = &a.report {
        let json = dest.ends_with(".json");
        let text = if json {
            serde_json::to_string_pretty(&done.report)?
        } else {
            done.report.to_text()
        };
        if dest == "-" {
            write!(out, "{text}")?;
            if json {
                writeln!(out)?;
            }
        } else {
            std::fs::write(dest, text).with_context(|| format!("writing {dest}"))?;
            writeln!(out, "report saved to {dest}")?;
        }
    }
    Ok(0)
}

fn inspect(file: &Path, json: bool, out: &mut dyn Write) -> Result<i32> {
    let book = open_any(file)?;
    let d = bookx_convert::inspect::Digest::of(&book);
    let toc = book.toc();
    if json {
        let tree = bookx_core::toc::flatten(&toc)
            .into_iter()
            .map(|f| serde_json::json!({"depth": f.depth, "title": f.title, "href": f.href}))
            .collect::<Vec<_>>();
        let v = serde_json::json!({
            "metadata": book.metadata(),
            "cover": d.has_cover,
            "toc": tree,
            "words": d.words.len(),
            "images": d.images,
            "internal_links": d.internal_links,
            "broken_links": d.broken_links,
            "spine": book.spine_paths().len(),
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
        return Ok(0);
    }
    write_metadata(book.metadata(), d.has_cover, out)?;
    writeln!(out, "Spine:       {} documents", book.spine_paths().len())?;
    writeln!(out, "Words:       {}", d.words.len())?;
    writeln!(out, "Images:      {}", d.images)?;
    writeln!(
        out,
        "Links:       {} internal, {} broken",
        d.internal_links, d.broken_links
    )?;
    writeln!(
        out,
        "Contents:    {} entries",
        bookx_core::toc::flatten(&toc).len()
    )?;
    for f in bookx_core::toc::flatten(&toc) {
        writeln!(out, "{}- {}", "  ".repeat(f.depth + 1), f.title)?;
    }
    Ok(0)
}

// -- meta ---------------------------------------------------------------------------------

fn open_any(path: &Path) -> Result<OpenBook> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("input");
    let archive =
        read_archive(name, &bytes).with_context(|| format!("opening {}", path.display()))?;
    OpenBook::from_archive(archive)
}

fn meta_show(file: &Path, json: bool, out: &mut dyn Write) -> Result<i32> {
    let book = open_any(file)?;
    let m = book.metadata();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(m)?)?;
    } else {
        write_metadata(m, book.cover_bytes().is_some(), out)?;
    }
    Ok(0)
}

fn write_metadata(m: &Metadata, has_cover: bool, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Title:       {}", m.title)?;
    writeln!(out, "Author(s):   {}", m.authors.join(" & "))?;
    if !m.series.is_empty() {
        writeln!(out, "Series:      {} #{}", m.series, m.series_index)?;
    }
    if !m.publisher.is_empty() {
        writeln!(out, "Publisher:   {}", m.publisher)?;
    }
    if !m.language.is_empty() {
        writeln!(out, "Language:    {}", m.language)?;
    }
    if !m.tags.is_empty() {
        writeln!(out, "Tags:        {}", m.tags.join(", "))?;
    }
    if m.rating > 0 {
        writeln!(out, "Rating:      {}/5", m.rating / 2)?;
    }
    for i in &m.identifiers {
        writeln!(out, "Identifier:  {}:{}", i.scheme, i.value)?;
    }
    writeln!(out, "Cover:       {}", if has_cover { "yes" } else { "no" })?;
    if !m.description.is_empty() {
        writeln!(out, "Description: {}", m.description)?;
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.bookx-tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("out")
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn meta_set(a: MetaSetArgs, out: &mut dyn Write) -> Result<i32> {
    if !a
        .file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("epub"))
    {
        bail!("metadata can only be written to .epub files; convert the book first");
    }
    if let Some(r) = a.rating {
        if r > 5 {
            bail!("--rating must be 0..=5");
        }
    }
    let mut book = OpenBook::open(&a.file)?;
    let mut md = book.metadata().clone();
    if let Some(v) = a.title {
        md.title_sort = bookx_core::util::title_sort(&v);
        md.title = v;
    }
    if !a.author.is_empty() {
        md.author_sort = a
            .author
            .first()
            .map(|n| bookx_core::util::author_sort(n))
            .unwrap_or_default();
        md.authors = a.author;
    }
    if let Some(v) = a.series {
        md.series = v;
    }
    if let Some(v) = a.series_index {
        md.series_index = v;
    }
    if !a.tag.is_empty() {
        md.tags = a.tag;
    }
    if let Some(v) = a.publisher {
        md.publisher = v;
    }
    if let Some(v) = a.language {
        md.language = v;
    }
    if let Some(v) = a.description {
        md.description = v;
    }
    if let Some(r) = a.rating {
        md.rating = r * 2;
    }
    for id in &a.identifier {
        if !id.contains(':') {
            bail!("identifier `{id}` must look like scheme:value");
        }
    }
    if !a.identifier.is_empty() {
        md.set_identifiers_from_string(&a.identifier.join(", "));
    }
    book.set_metadata(md)?;
    if let Some(c) = &a.cover {
        let data = image_ext_ok(c)?;
        let ext = if data.starts_with(b"\x89PNG") {
            "png"
        } else if data.starts_with(b"GIF8") {
            "gif"
        } else {
            "jpg"
        };
        book.set_cover(data, ext)?;
    }
    write_atomic(&a.file, &book.archive.to_bytes()?)?;
    writeln!(out, "updated {}", a.file.display())?;
    Ok(0)
}

// -- check --------------------------------------------------------------------------------

fn check(file: &Path, json: bool, out: &mut dyn Write) -> Result<i32> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("input");
    let archive: Archive = read_archive(name, &bytes)?;
    let issues = check_book(&archive);
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let label = |s: Severity| match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    if json {
        let list: Vec<_> = issues
            .iter()
            .map(|i| {
                serde_json::json!({
                    "severity": label(i.severity),
                    "file": i.file,
                    "line": i.line,
                    "message": i.message,
                })
            })
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"errors": errors, "issues": list}))?
        )?;
    } else if issues.is_empty() {
        writeln!(out, "no problems found")?;
    } else {
        for i in &issues {
            let loc = i.line.map(|l| format!(":{l}")).unwrap_or_default();
            writeln!(
                out,
                "{}: {}{}: {}",
                label(i.severity),
                i.file,
                loc,
                i.message
            )?;
        }
        writeln!(out, "{} problem(s), {errors} error(s)", issues.len())?;
    }
    Ok(if errors > 0 { 1 } else { 0 })
}

// -- library ------------------------------------------------------------------------------

fn sort_key(s: &str) -> Result<SortKey> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "title" => SortKey::Title,
        "author" => SortKey::Author,
        "added" | "date-added" => SortKey::DateAdded,
        "last-read" | "lastread" => SortKey::LastRead,
        "published" => SortKey::Published,
        "rating" => SortKey::Rating,
        "series" => SortKey::Series,
        "size" => SortKey::Size,
        "progress" => SortKey::Progress,
        other => bail!(
            "unknown sort key `{other}` (title, author, added, last-read, published, rating, series, size, progress)"
        ),
    })
}

fn book_json(b: &bookx_core::library::Book) -> serde_json::Value {
    serde_json::json!({
        "id": b.id,
        "title": b.meta.title,
        "authors": b.meta.authors,
        "series": b.meta.series,
        "series_index": b.meta.series_index,
        "tags": b.meta.tags,
        "file": b.file,
        "formats": b.all_formats().iter().map(|f| f.format.clone()).collect::<Vec<_>>(),
        "protected": b.drm,
        "size": b.size,
        "progress": b.progress,
        "status": b.status(),
        "custom": b.custom,
    })
}

fn print_books(lib: &Library, ids: &[u64], json: bool, out: &mut dyn Write) -> Result<()> {
    let books: Vec<_> = ids.iter().filter_map(|id| lib.get(*id)).collect();
    if json {
        let v: Vec<_> = books.iter().map(|b| book_json(b)).collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
    } else {
        for b in books {
            writeln!(
                out,
                "{:>5}  {}  —  {}",
                b.id,
                b.meta.title,
                b.meta.authors.join(" & ")
            )?;
        }
    }
    Ok(())
}

/// Opens the library in `root` (an old `bookx-library.json` is moved into the database).
fn open_lib(root: &Path) -> Result<bookx_store::OpenedLibrary> {
    bookx_store::open_library(root)
}

/// Moves an old `bookx-library.json` into the database before a command runs, and says so.
fn migrate_note(root: &Path, err: &mut dyn Write) -> Result<()> {
    if root.join(bookx_store::LEGACY_JSON).exists() {
        if let Some(n) = bookx_store::open_library(root)?.migrated {
            writeln!(
                err,
                "note: moved {n} book(s) from {} into {} (the old file was kept as {}.migrated)",
                bookx_store::LEGACY_JSON,
                bookx_store::LIBRARY_DB,
                bookx_store::LEGACY_JSON
            )?;
        }
    }
    Ok(())
}

fn existing_library(root: &Path) -> Result<Library> {
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    Ok(open_lib(root)?.library)
}

fn search(
    root: &Path,
    query: &str,
    sort: &str,
    desc: bool,
    json: bool,
    out: &mut dyn Write,
) -> Result<i32> {
    let key = sort_key(sort)?;
    let lib = existing_library(root)?;
    let ids = lib.search(query, key, !desc);
    print_books(&lib, &ids, json, out)?;
    Ok(if ids.is_empty() { 1 } else { 0 })
}

fn add_one(lib: &mut Library, path: &Path, dup: bool) -> Result<AddOutcome> {
    if bookx_core::library::is_supported_file(path) {
        return lib.add_path(path, dup);
    }
    // other formats are converted to EPUB first
    let bytes = std::fs::read(path)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("input");
    let archive = read_archive(name, &bytes)?;
    let dir = std::env::temp_dir().join(format!("bookx-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("book");
    let tmp = dir.join(format!("{stem}.epub"));
    let r = std::fs::write(&tmp, archive.to_bytes()?)
        .map_err(anyhow::Error::from)
        .and_then(|_| lib.add_path(&tmp, dup));
    let _ = std::fs::remove_dir_all(&dir);
    // the book is known to sync servers by the file it came from, not by the temporary EPUB
    if let (Ok(AddOutcome::Added(id)), Ok(sid)) = (&r, bookx_core::docid::partial_md5(path)) {
        if let Some(b) = lib.get_mut(*id) {
            b.source_ids = vec![sid];
        }
        lib.save()?;
    }
    // a PDF is kept next to the reflowed EPUB: the app shows the pages from it
    if let (Ok(AddOutcome::Added(id)), true) = (
        &r,
        path.extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf")),
    ) {
        let _ = lib.add_format(*id, path);
    }
    r
}

fn library(cmd: LibCmd, root: &Path, out: &mut dyn Write, err: &mut dyn Write) -> Result<i32> {
    match cmd {
        LibCmd::Add {
            paths,
            recursive,
            allow_duplicates,
        } => {
            if paths.is_empty() {
                bail!("give at least one file or folder to add");
            }
            let mut files = Vec::new();
            for p in &paths {
                if p.is_dir() {
                    collect_supported(p, recursive, &mut files);
                } else if p.is_file() {
                    files.push(p.clone());
                } else {
                    bail!("{} does not exist", p.display());
                }
            }
            let mut guarded = libtools::open_for_write(root)?;
            let lib = &mut guarded.opened.library;
            let (mut added, mut dups, mut failed) = (0, 0, 0);
            for f in files {
                match add_one(lib, &f, allow_duplicates) {
                    Ok(AddOutcome::Added(id)) => {
                        added += 1;
                        writeln!(out, "added #{id}: {}", f.display())?;
                    }
                    Ok(AddOutcome::Duplicate(id)) => {
                        dups += 1;
                        writeln!(out, "duplicate of #{id}: {}", f.display())?;
                    }
                    Err(e) => {
                        failed += 1;
                        writeln!(err, "failed: {}: {e:#}", f.display())?;
                    }
                }
            }
            lib.save()?;
            writeln!(out, "{added} added, {dups} duplicate(s), {failed} failed")?;
            Ok(if failed > 0 { 1 } else { 0 })
        }
        LibCmd::List { json } => {
            let lib = existing_library(root)?;
            let ids = lib.search("", SortKey::Title, true);
            print_books(&lib, &ids, json, out)?;
            Ok(0)
        }
        LibCmd::Info { id, json } => {
            let lib = existing_library(root)?;
            let b = lib.get(id).ok_or_else(|| anyhow!("no book with id {id}"))?;
            if json {
                writeln!(out, "{}", serde_json::to_string_pretty(&book_json(b))?)?;
            } else {
                writeln!(out, "Id:          {}", b.id)?;
                write_metadata(&b.meta, b.cover.is_some(), out)?;
                writeln!(out, "File:        {}", b.file)?;
                if !b.formats.is_empty() {
                    let all: Vec<String> =
                        b.all_formats().iter().map(|f| f.format.clone()).collect();
                    writeln!(out, "Formats:     {}", all.join(", "))?;
                }
                for (label, value) in &b.custom {
                    writeln!(
                        out,
                        "#{label}:{}{value}",
                        " ".repeat(12usize.saturating_sub(label.len() + 2).max(1))
                    )?;
                }
                if !b.drm.is_empty() {
                    writeln!(
                        out,
                        "Protected:   {} (Bookx cannot open it and does not remove DRM)",
                        b.drm
                    )?;
                }
                let sources = libtools::sources_line(root, b.id);
                if !sources.is_empty() {
                    writeln!(out, "Sources:     {sources}")?;
                }
                writeln!(
                    out,
                    "Status:      {} ({:.0}%)",
                    b.status(),
                    b.progress * 100.0
                )?;
            }
            Ok(0)
        }
        LibCmd::Remove { ids, delete_files } => {
            if ids.is_empty() {
                bail!("give at least one book id");
            }
            existing_library(root)?;
            let mut guarded = libtools::open_for_write(root)?;
            let lib = &mut guarded.opened.library;
            for id in &ids {
                if lib.get(*id).is_none() {
                    bail!("no book with id {id}");
                }
            }
            lib.remove(&ids, delete_files)?;
            lib.save()?;
            writeln!(out, "removed {} book(s)", ids.len())?;
            Ok(0)
        }
        LibCmd::Dump => {
            if !bookx_store::has_library(root) {
                bail!(
                    "no library at {} (add books first, or pass --library)",
                    root.display()
                );
            }
            let opened = open_lib(root)?;
            writeln!(out, "{}", opened.store.export_catalog_json()?)?;
            Ok(0)
        }
        LibCmd::Columns(c) => columns::run(c, root, out),
        LibCmd::Replace(a) => replace::run(a, root, out),
        LibCmd::Tools(t) => libtools::run(t, root, out, err),
        LibCmd::Catalogue(c) => catalogue::run(c, root, out),
        LibCmd::Extras(c) => extras::run(c, root, out),
        LibCmd::Export { id, dest } => {
            let lib = existing_library(root)?;
            if lib.get(id).is_none() {
                bail!("no book with id {id}");
            }
            lib.export_copy(id, &dest)?;
            writeln!(out, "exported to {}", dest.display())?;
            Ok(0)
        }
    }
}

// -- serve --------------------------------------------------------------------------------

fn serve(a: ServeArgs, root: &Path, env: &Env, out: &mut dyn Write) -> Result<i32> {
    let _ = existing_library(root)?;
    let mut users = Vec::new();
    for u in &a.user {
        let (name, password) = u
            .split_once(':')
            .filter(|(n, p)| !n.is_empty() && !p.is_empty())
            .ok_or_else(|| anyhow!("--user must look like name:password"))?;
        users.push(User {
            name: name.into(),
            password: password.into(),
        });
    }
    let root_buf = root.to_path_buf();
    let watcher = bookx_store::CatalogWatcher::new(root);
    let provider: bookx_serve::server::LibraryProvider = Arc::new(move || LibrarySnapshot {
        root: root_buf.clone(),
        books: watcher.books(),
    });
    let mut cfg = ServerConfig::new(provider);
    cfg.bind = a.bind;
    cfg.port = a.port;
    cfg.title = a.title;
    cfg.users = users;
    cfg.read_only = a.read_only;
    cfg.allowed_hosts.extend(a.allow_host.iter().cloned());
    for a in &a.account {
        cfg.accounts
            .push(bookx_serve::accounts::parse_arg(a).map_err(|e| anyhow!("--account: {e}"))?);
    }
    if !a.guest_collection.is_empty() {
        cfg.guest = Some(a.guest_collection.clone());
    }
    cfg.limits.trusted_proxies = bookx_serve::net::parse_proxy_list(&a.trusted_proxy.join(","))
        .map_err(|bad| anyhow!("--trusted-proxy {bad}: expected an IP address or a network such as 10.0.0.0/8 (not /0)"))?;
    if let Some(p) = &a.public_prefix {
        cfg.public_prefix = bookx_serve::server::clean_public_prefix(p)
            .ok_or_else(|| anyhow!("--public-prefix {p}: expected a plain path such as /books"))?;
    }
    if let (Some(cert), Some(key)) = (&a.tls_cert, &a.tls_key) {
        cfg.tls = Some(bookx_serve::net::tls_config(cert, key)?);
    }
    if let Some(path) = a.kosync {
        let store = bookx_serve::kosync::JsonFileStore::open(&path)?;
        cfg.kosync = Some(Arc::new(bookx_serve::kosync::KoSync::new(
            Box::new(store),
            true,
        )));
    }
    let handle = Server::start(cfg)?;
    writeln!(out, "serving on {}", handle.base_url())?;
    out.flush()?;
    while !env.stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(50));
    }
    handle.shutdown();
    Ok(0)
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn edit_arguments_for_the_app() {
        let a = edit_args(
            Path::new("/b.epub"),
            &["ch1.xhtml".into()],
            Some("some words"),
        );
        assert_eq!(
            a,
            vec![
                "--edit",
                "/b.epub",
                "ch1.xhtml",
                "--select-text",
                "some words"
            ]
        );
        assert_eq!(edit_args(Path::new("b"), &[], None), vec!["--edit", "b"]);
    }

    #[test]
    fn edit_of_a_missing_file_fails_without_starting_anything() {
        let mut err = Vec::new();
        let code = edit(Path::new("/definitely/not/here.epub"), &[], None, &mut err).unwrap();
        assert_eq!(code, 2);
        assert!(String::from_utf8_lossy(&err).contains("does not exist"));
    }
}
