//! Downloads of model packs: resumable (`Range`), hash-verified, with progress and cancel.
//!
//! A file is written to `<name>.part`; a part from an earlier attempt is continued when the
//! server supports ranges. Only a file whose SHA-256 matches the catalog is moved into place;
//! anything else is deleted and reported.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest, Sha256};

use crate::models::{ArchiveKind, Extract, FileSpec, Origin, Pack, Store};
use crate::synth::Cancel;

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub pack: String,
    pub file: String,
    /// Bytes of the whole plan done / to do.
    pub done: u64,
    pub total: u64,
    /// Which origin of the file is being used (0 = the first; more when it was slow or failed).
    pub origin: usize,
}

/// One agent for every download: its connection pool keeps the TLS sessions (and the connections
/// to the CDN) alive between the requests of a download, so a resume or a next segment does not
/// pay for another handshake.
fn agent(fast_fail: bool) -> ureq::Agent {
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    static FAST: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    // an origin that has another one behind it must answer quickly (connect and first byte), so that a
    // dead or throttled one is given up soon; the last one is waited for
    if fast_fail {
        return FAST
            .get_or_init(|| {
                ureq::Agent::config_builder()
                    .timeout_connect(Some(Duration::from_secs(8)))
                    .timeout_recv_response(Some(Duration::from_secs(15)))
                    .timeout_recv_body(Some(Duration::from_secs(4 * 3600)))
                    .http_status_as_error(false)
                    .max_idle_connections(32)
                    .max_idle_connections_per_host(16)
                    .user_agent("Bookx/0.2 (model download)")
                    .build()
                    .into()
            })
            .clone();
    }
    AGENT
        .get_or_init(|| {
            ureq::Agent::config_builder()
                .timeout_connect(Some(Duration::from_secs(30)))
                .timeout_recv_response(Some(Duration::from_secs(60)))
                .timeout_recv_body(Some(Duration::from_secs(4 * 3600)))
                .http_status_as_error(false)
                .max_idle_connections(32)
                .max_idle_connections_per_host(16)
                .user_agent("Bookx/0.2 (model download)")
                .build()
                .into()
        })
        .clone()
}

/// How a download is split up. The defaults suit the real packs; the tests make them small.
#[derive(Debug, Clone)]
pub struct Tuning {
    /// Connections used at once for one big file (1 = a single stream).
    pub segments: usize,
    /// Files smaller than this are fetched as one stream.
    pub min_parallel_bytes: u64,
    /// A segment is never smaller than this.
    pub min_segment_bytes: u64,
    /// Read/write buffer of a stream.
    pub buf_bytes: usize,
    /// Tries of one segment after the connection broke, before the download fails.
    pub retries: u32,
    pub retry_pause: Duration,
    /// This origin has another one behind it: short connect and first-byte timeouts.
    pub fast_fail: bool,
    /// The file is the bytes `range_base..range_base + size` of the URL (a member of a big zip):
    /// every Range request is shifted by this and a server without ranges cannot be used.
    pub range_base: u64,
    /// Give up on this origin (error "too slow") when the speed stays below `.0` bytes per second over
    /// a window of `.1`. Used for origins that have another one behind them.
    pub min_speed: Option<(u64, Duration)>,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            segments: 8,
            min_parallel_bytes: 8 << 20,
            min_segment_bytes: 1 << 20,
            buf_bytes: 256 << 10,
            retries: 3,
            retry_pause: Duration::from_millis(500),
            fast_fail: false,
            range_base: 0,
            min_speed: None,
        }
    }
}

impl Tuning {
    /// The tuning for an origin that is not the last one: quick to give up.
    pub fn with_fallback_behind(&self) -> Tuning {
        Tuning {
            fast_fail: true,
            min_speed: self.min_speed.or(Some((100_000, Duration::from_secs(5)))),
            ..self.clone()
        }
    }
}

/// Watches the speed of a download over a sliding window.
pub struct Watchdog {
    min_bps: u64,
    window: Duration,
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
    started: std::time::Instant,
}

impl Watchdog {
    pub fn new(min: Option<(u64, Duration)>) -> Option<Watchdog> {
        let (min_bps, window) = min?;
        Some(Watchdog {
            min_bps,
            window,
            samples: Default::default(),
            started: std::time::Instant::now(),
        })
    }

    /// True when the speed over the last window is below the minimum (only judged once a full window
    /// has passed since the start).
    pub fn too_slow(&mut self, now: std::time::Instant, total: u64) -> bool {
        self.samples.push_back((now, total));
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= self.window {
            self.samples.pop_front();
        }
        if now.duration_since(self.started) < self.window {
            return false;
        }
        let (t0, b0) = self.samples[0];
        let secs = now.duration_since(t0).as_secs_f64();
        secs >= self.window.as_secs_f64() * 0.8
            && (total.saturating_sub(b0) as f64 / secs) < self.min_bps as f64
    }
}

/// One piece of a segmented download: bytes `start..end` of the file, `done` of them already
/// written (from `start` on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seg {
    pub start: u64,
    pub end: u64,
    pub done: u64,
}

impl Seg {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Cuts `start..end` into at most `n` pieces of at least `min` bytes (the last takes the rest).
pub fn split(start: u64, end: u64, n: usize, min: u64) -> Vec<Seg> {
    if end <= start {
        return vec![];
    }
    let total = end - start;
    let n = (total / min.max(1)).clamp(1, n.max(1) as u64);
    let piece = total.div_ceil(n);
    let mut out = Vec::new();
    let mut at = start;
    while at < end {
        let e = (at + piece).min(end);
        out.push(Seg {
            start: at,
            end: e,
            done: 0,
        });
        at = e;
    }
    out
}

/// The bytes from the start of the file that are all there (what can be hashed already).
pub fn contiguous_prefix(segs: &[Seg]) -> u64 {
    let mut n = 0;
    for s in segs {
        n += s.done;
        if s.done < s.len() {
            break;
        }
    }
    n
}

/// The plan as text for the sidecar file (`start end done` per line).
pub fn plan_text(segs: &[Seg]) -> String {
    segs.iter()
        .map(|s| format!("{} {} {}\n", s.start, s.end, s.done))
        .collect()
}

/// Reads a saved plan; `None` unless it covers `0..size` without gaps and every `done` fits.
pub fn parse_plan(text: &str, size: u64) -> Option<Vec<Seg>> {
    let mut out: Vec<Seg> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Vec<u64> = line
            .split_whitespace()
            .map(|n| n.parse().ok())
            .collect::<Option<_>>()?;
        let [start, end, done] = v[..] else {
            return None;
        };
        let want = out.last().map_or(0, |l| l.end);
        if start != want || end <= start || done > end - start {
            return None;
        }
        out.push(Seg { start, end, done });
    }
    (out.last()?.end == size).then_some(out)
}

fn map_path(dest: &Path) -> PathBuf {
    let mut s = part_path(dest).into_os_string();
    s.push(".map");
    PathBuf::from(s)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

static OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Offline mode: while on, nothing new is downloaded (files already complete are still accepted).
pub fn set_offline(on: bool) {
    OFFLINE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn is_offline() -> bool {
    OFFLINE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Fetches `url` to `dest`, checks size and SHA-256. `on_bytes` gets the bytes received.
pub fn download_file(
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    download_file_tuned(
        &Tuning::default(),
        url,
        dest,
        sha256,
        size,
        cancel,
        on_bytes,
    )
}

/// [`download_file`] with its own [`Tuning`]. Big files come in parallel segments (HTTP `Range`)
/// when the server supports them, and as one stream otherwise; the checksum is computed while the
/// bytes arrive, so the end has no separate pass over the file.
pub fn download_file_tuned(
    tuning: &Tuning,
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    // already there and right: nothing to do
    if dest.exists() && fs::metadata(dest).map(|m| m.len()).unwrap_or(0) == size {
        if sha256.is_empty() || sha256_file(dest)? == sha256 {
            on_bytes(size);
            return Ok(());
        }
        let _ = fs::remove_file(dest);
    }
    if is_offline() {
        bail!("Offline mode is on (Preferences → Privacy): nothing is downloaded");
    }
    let part = part_path(dest);
    if tuning.segments > 1 && size >= tuning.min_parallel_bytes.max(2 * tuning.min_segment_bytes) {
        if segmented(tuning, url, dest, sha256, size, cancel, on_bytes)? {
            return Ok(());
        }
        // the server does not do ranges: one stream, from the start
        let _ = fs::remove_file(&part);
        let _ = fs::remove_file(map_path(dest));
    } else {
        let _ = fs::remove_file(map_path(dest));
    }
    let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let have = if have > size {
        let _ = fs::remove_file(&part);
        0
    } else {
        have
    };
    // the checksum is made on the way: what is already in the part is read first
    let mut hasher = Sha256::new();
    if have > 0 {
        feed_file(&mut hasher, &part, have)?;
    }
    let mut hasher = Some(hasher);
    if have < size {
        fetch_into(
            tuning,
            url,
            &part,
            have,
            size,
            cancel,
            on_bytes,
            &mut hasher,
        )?;
    } else {
        on_bytes(have);
    }
    let len = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if len != size {
        let _ = fs::remove_file(&part);
        bail!(
            "the download of {url} has the wrong size ({len} instead of {size} bytes); try again"
        );
    }
    let got = format!("{:x}", hasher.take().unwrap_or_default().finalize());
    finish(&part, dest, url, &got, sha256)
}

/// Checks the checksum and moves the part into place (or deletes it).
fn finish(part: &Path, dest: &Path, url: &str, got: &str, want: &str) -> Result<()> {
    // an empty `want`: the bytes are not a whole file with a known hash (a member of a zip); the
    // extracted file is checked instead
    if !want.is_empty() && got != want {
        let _ = fs::remove_file(part);
        let _ = fs::remove_file(map_path(dest));
        bail!(
            "the checksum of {url} does not match (got {got}, expected {want}): the file was damaged or changed; it was deleted"
        );
    }
    fs::rename(part, dest).with_context(|| format!("moving the download to {}", dest.display()))?;
    let _ = fs::remove_file(map_path(dest));
    Ok(())
}

fn feed_file(h: &mut Sha256, path: &Path, len: u64) -> Result<()> {
    let mut f = File::open(path)?.take(len);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        h.update(&buf[..n]);
    }
}

#[allow(clippy::too_many_arguments)]
fn fetch_into(
    tuning: &Tuning,
    url: &str,
    part: &Path,
    mut have: u64,
    size: u64,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
    hasher: &mut Option<Sha256>,
) -> Result<()> {
    let base = tuning.range_base;
    let mut req = agent(tuning.fast_fail).get(url);
    if base > 0 {
        req = req.header(
            "Range",
            &format!("bytes={}-{}", base + have, base + size - 1),
        );
    } else if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| anyhow!("cannot reach {url}: {e}"))?;
    let status = resp.status().as_u16();
    match status {
        206 => {}
        200 if base > 0 => {
            bail!("the server of {url} does not do ranges, so a part of the file cannot be read")
        }
        200 => {
            // the server ignores ranges: start over
            have = 0;
            *hasher = Some(Sha256::new());
        }
        416 => {
            // our part is longer than the file or complete: start over
            let _ = fs::remove_file(part);
            *hasher = Some(Sha256::new());
            return fetch_into(tuning, url, part, 0, size, cancel, on_bytes, hasher);
        }
        other => bail!("the server answered {other} for {url}"),
    }
    let mut out = if have > 0 {
        OpenOptions::new().append(true).open(part)?
    } else {
        File::create(part)?
    };
    on_bytes(have);
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; tuning.buf_bytes.max(4096)];
    let mut total = have;
    let mut dog = Watchdog::new(tuning.min_speed);
    loop {
        if cancel.is_cancelled() {
            // the part stays for the next attempt
            bail!("cancelled");
        }
        if let Some(d) = dog.as_mut()
            && d.too_slow(std::time::Instant::now(), total)
        {
            bail!(
                "too slow: {url} gives less than {} KB/s",
                tuning.min_speed.map_or(0, |m| m.0 / 1000)
            );
        }
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("reading {url}"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            drop(out);
            let _ = fs::remove_file(part);
            bail!("the server sent more than the {size} bytes expected for {url}");
        }
        out.write_all(&buf[..n])?;
        if let Some(h) = hasher.as_mut() {
            h.update(&buf[..n]);
        }
        on_bytes(n as u64);
    }
    out.flush()?;
    Ok(())
}

/// What the probe of a server found out.
struct Probe {
    /// Where the bytes really are (after the redirects: asked once, not once per segment).
    url: String,
}

/// Asks for the first byte only: a `206` means ranges work; the final address of the file is
/// kept so that the segments do not each follow the redirect again.
fn probe(tuning: &Tuning, url: &str, size: u64) -> Result<Option<Probe>> {
    use ureq::ResponseExt;
    let base = tuning.range_base;
    let resp = agent(tuning.fast_fail)
        .get(url)
        .header("Range", &format!("bytes={base}-{base}"))
        .call()
        .map_err(|e| anyhow!("cannot reach {url}: {e}"))?;
    match resp.status().as_u16() {
        206 => {
            let total = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|n| n.trim().parse::<u64>().ok());
            if base == 0 && total.is_some_and(|t| t != size) {
                bail!(
                    "the download of {url} has the wrong size ({} instead of {size} bytes); try again",
                    total.unwrap_or(0)
                );
            }
            Ok(Some(Probe {
                url: resp.get_uri().to_string(),
            }))
        }
        200 => Ok(None),
        other => bail!("the server answered {other} for {url}"),
    }
}

/// The parallel download. `Ok(false)` when the server has no ranges.
fn segmented(
    tuning: &Tuning,
    url: &str,
    dest: &Path,
    sha256: &str,
    size: u64,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<bool> {
    use std::os::unix::fs::FileExt;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    let Some(probe) = probe(tuning, url, size)? else {
        return Ok(false);
    };
    let part = part_path(dest);
    let map = map_path(dest);
    // the plan: a saved one, else the old single-stream part as a finished first piece, else new
    let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mut segs = fs::read_to_string(&map)
        .ok()
        .and_then(|t| parse_plan(&t, size))
        .filter(|_| have == size);
    if segs.is_none() {
        let prefix = if have > 0 && have < size { have } else { 0 };
        let mut v = Vec::new();
        if prefix > 0 {
            v.push(Seg {
                start: 0,
                end: prefix,
                done: prefix,
            });
        }
        v.extend(split(
            prefix,
            size,
            tuning.segments,
            tuning.min_segment_bytes,
        ));
        segs = Some(v);
    }
    let segs = segs.unwrap_or_default();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&part)?;
    if file.metadata()?.len() != size {
        file.set_len(size)?;
    }
    let done: Vec<AtomicU64> = segs.iter().map(|s| AtomicU64::new(s.done)).collect();
    let snapshot = |done: &[AtomicU64]| -> Vec<Seg> {
        segs.iter()
            .zip(done)
            .map(|(s, d)| Seg {
                done: d.load(Ordering::SeqCst),
                ..*s
            })
            .collect()
    };
    let save = |done: &[AtomicU64]| {
        let _ = fs::write(&map, plan_text(&snapshot(done)));
    };
    save(&done);
    let abort = AtomicBool::new(false);
    let failure: Mutex<Option<String>> = Mutex::new(None);
    let finished = AtomicUsize::new(0);
    let digest: Mutex<Option<String>> = Mutex::new(None);
    let mut reported = 0u64;
    let start_total: u64 = done.iter().map(|d| d.load(Ordering::SeqCst)).sum();
    on_bytes(start_total);
    reported += start_total;

    std::thread::scope(|sc| {
        for (i, seg) in segs.iter().enumerate() {
            let (done_i, file, abort, failure, finished, probe_url) = (
                &done[i],
                &file,
                &abort,
                &failure,
                &finished,
                probe.url.as_str(),
            );
            sc.spawn(move || {
                let r = run_segment(tuning, probe_url, file, *seg, done_i, cancel, abort);
                if let Err(e) = r {
                    let mut f = failure.lock().unwrap();
                    if f.is_none() {
                        *f = Some(e.to_string());
                    }
                    abort.store(true, Ordering::SeqCst);
                }
                finished.fetch_add(1, Ordering::SeqCst);
            });
        }
        // the checksum follows the front of the file as it fills
        {
            let (done, abort, digest, file) = (&done, &abort, &digest, &file);
            let segs = &segs;
            sc.spawn(move || {
                let mut h = Sha256::new();
                let mut hashed = 0u64;
                let mut buf = vec![0u8; 1 << 20];
                while hashed < size {
                    if abort.load(Ordering::SeqCst) || cancel.is_cancelled() {
                        return;
                    }
                    let now: Vec<Seg> = segs
                        .iter()
                        .zip(done.iter())
                        .map(|(s, d)| Seg {
                            done: d.load(Ordering::SeqCst),
                            ..*s
                        })
                        .collect();
                    let upto = contiguous_prefix(&now);
                    if upto <= hashed {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    let n = ((upto - hashed) as usize).min(buf.len());
                    if file.read_exact_at(&mut buf[..n], hashed).is_err() {
                        return;
                    }
                    h.update(&buf[..n]);
                    hashed += n as u64;
                }
                *digest.lock().unwrap() = Some(format!("{:x}", h.finalize()));
            });
        }
        let mut last_save = std::time::Instant::now();
        let mut dog = Watchdog::new(tuning.min_speed);
        while finished.load(Ordering::SeqCst) < segs.len() {
            std::thread::sleep(Duration::from_millis(40));
            let total: u64 = done.iter().map(|d| d.load(Ordering::SeqCst)).sum();
            if let Some(d) = dog.as_mut()
                && d.too_slow(std::time::Instant::now(), total)
            {
                let mut f = failure.lock().unwrap();
                if f.is_none() {
                    *f = Some(format!(
                        "too slow: {url} gives less than {} KB/s",
                        tuning.min_speed.map_or(0, |m| m.0 / 1000)
                    ));
                }
                abort.store(true, Ordering::SeqCst);
            }
            if total > reported {
                on_bytes(total - reported);
                reported = total;
            }
            if cancel.is_cancelled() {
                abort.store(true, Ordering::SeqCst);
            }
            if last_save.elapsed() >= Duration::from_secs(1) {
                save(&done);
                last_save = std::time::Instant::now();
            }
        }
        let total: u64 = done.iter().map(|d| d.load(Ordering::SeqCst)).sum();
        if total > reported {
            on_bytes(total - reported);
        }
    });
    save(&done);
    if let Some(e) = failure.lock().unwrap().take() {
        bail!("{e}");
    }
    if cancel.is_cancelled() {
        bail!("cancelled");
    }
    let total: u64 = done.iter().map(|d| d.load(Ordering::SeqCst)).sum();
    if total != size {
        bail!(
            "the download of {url} has the wrong size ({total} instead of {size} bytes); try again"
        );
    }
    // the hasher may still be on the last bytes
    let got = loop {
        if let Some(d) = digest.lock().unwrap().take() {
            break d;
        }
        // (it always finishes: everything is written by now)
        std::thread::sleep(Duration::from_millis(5));
    };
    drop(file);
    finish(&part, dest, url, &got, sha256)?;
    Ok(true)
}

/// Fetches one segment, over as many requests as it takes (a broken connection is continued from
/// the byte it stopped at).
fn run_segment(
    tuning: &Tuning,
    url: &str,
    file: &File,
    seg: Seg,
    done: &std::sync::atomic::AtomicU64,
    cancel: &Cancel,
    abort: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use std::os::unix::fs::FileExt;
    use std::sync::atomic::Ordering;
    let stop = || cancel.is_cancelled() || abort.load(Ordering::SeqCst);
    let mut failures = 0u32;
    let mut buf = vec![0u8; tuning.buf_bytes.max(4096)];
    loop {
        if stop() {
            return Ok(());
        }
        let from = seg.start + done.load(Ordering::SeqCst);
        if from >= seg.end {
            return Ok(());
        }
        let mut progressed = false;
        let attempt: Result<()> = (|| {
            let resp = agent(tuning.fast_fail)
                .get(url)
                .header(
                    "Range",
                    &format!(
                        "bytes={}-{}",
                        tuning.range_base + from,
                        tuning.range_base + seg.end - 1
                    ),
                )
                .call()
                .map_err(|e| anyhow!("cannot reach {url}: {e}"))?;
            let status = resp.status().as_u16();
            if status != 206 {
                bail!("the server answered {status} for a range of {url}");
            }
            let mut body = resp.into_body();
            let mut reader = body.as_reader();
            let mut pos = from;
            while pos < seg.end {
                if stop() {
                    return Ok(());
                }
                let want = ((seg.end - pos) as usize).min(buf.len());
                let n = reader
                    .read(&mut buf[..want])
                    .with_context(|| format!("reading {url}"))?;
                if n == 0 {
                    break;
                }
                file.write_all_at(&buf[..n], pos)?;
                pos += n as u64;
                done.fetch_add(n as u64, Ordering::SeqCst);
                progressed = true;
            }
            Ok(())
        })();
        match attempt {
            Ok(()) => {
                if seg.start + done.load(Ordering::SeqCst) >= seg.end || stop() {
                    return Ok(());
                }
                // the server ended the answer early: go on from there
                failures += 1;
            }
            Err(e) => {
                if progressed {
                    failures = 0;
                }
                failures += 1;
                if failures > tuning.retries {
                    return Err(e);
                }
            }
        }
        if failures > tuning.retries {
            bail!("the server keeps cutting the download of {url}");
        }
        std::thread::sleep(tuning.retry_pause * failures.max(1));
    }
}

/// The most a member may be when its size is not pinned (nothing the app downloads is bigger).
const MAX_MEMBER: u64 = 512 << 20;

/// Unpacks the member of a `.tgz` whose path ends with `suffix` to `dest`.
pub fn extract_member(archive: &Path, suffix: &str, dest: &Path) -> Result<()> {
    extract_member_checked(archive, suffix, dest, None)
}

/// [`extract_member`] with what the file must be: `expect` is its SHA-256 and size. Only a regular
/// file is taken (links and devices are ignored), a path that tries to leave the archive's folder
/// (`..`, absolute) is refused, a member that is bigger than expected is refused before it is written,
/// and the written file is checked against the pinned hash (and deleted when it differs).
pub fn extract_member_checked(
    archive: &Path,
    suffix: &str,
    dest: &Path,
    expect: Option<(&str, u64)>,
) -> Result<()> {
    use std::path::Component;
    let f = File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    for entry in ar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let text = path.to_string_lossy().replace('\\', "/");
        if !text.ends_with(suffix) {
            continue;
        }
        if path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            bail!("the archive has an unsafe path ({text}) and was not unpacked");
        }
        let cap = expect.map_or(MAX_MEMBER, |e| e.1);
        if entry.size() > cap || (expect.is_some() && entry.size() != cap) {
            bail!(
                "{text} in the archive is {} bytes, not the {cap} expected: not unpacked",
                entry.size()
            );
        }
        let tmp = part_path(dest);
        let mut out = File::create(&tmp)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 256 << 10];
        let mut total = 0u64;
        let mut src = (&mut entry).take(cap + 1);
        loop {
            let n = src.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > cap {
                drop(out);
                let _ = fs::remove_file(&tmp);
                bail!(
                    "{text} in the archive is bigger than the {cap} bytes expected: not unpacked"
                );
            }
            out.write_all(&buf[..n])?;
            h.update(&buf[..n]);
        }
        out.flush()?;
        drop(out);
        if let Some((sha, _)) = expect {
            let got = format!("{:x}", h.finalize());
            if got != sha {
                let _ = fs::remove_file(&tmp);
                bail!(
                    "the checksum of the unpacked library does not match (got {got}, expected {sha}): it was deleted"
                );
            }
        }
        fs::rename(&tmp, dest)?;
        return Ok(());
    }
    bail!("{suffix} is not in the archive")
}

/// A read-only view of a file on a server that does Range requests, for the `zip` crate: the zip's
/// directory sits at the end, so only a few blocks are fetched.
struct RangeReader {
    url: String,
    len: u64,
    pos: u64,
    fast: bool,
    block: Vec<u8>,
    block_start: u64,
}

impl RangeReader {
    const BLOCK: u64 = 256 << 10;

    fn open(url: &str, fast: bool) -> Result<RangeReader> {
        let resp = agent(fast)
            .get(url)
            .header("Range", "bytes=0-0")
            .call()
            .map_err(|e| anyhow!("cannot reach {url}: {e}"))?;
        if resp.status().as_u16() != 206 {
            bail!("the server of {url} does not do ranges, so one member of it cannot be read");
        }
        let len = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|n| n.trim().parse::<u64>().ok())
            .ok_or_else(|| anyhow!("{url} does not say how big it is"))?;
        Ok(RangeReader {
            url: url.to_string(),
            len,
            pos: 0,
            fast,
            block: Vec::new(),
            block_start: 0,
        })
    }
}

impl std::io::Read for RangeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len || out.is_empty() {
            return Ok(0);
        }
        let in_block =
            self.pos >= self.block_start && self.pos < self.block_start + self.block.len() as u64;
        if !in_block {
            let end = (self.pos + Self::BLOCK).min(self.len) - 1;
            let resp = agent(self.fast)
                .get(&self.url)
                .header("Range", &format!("bytes={}-{end}", self.pos))
                .call()
                .map_err(std::io::Error::other)?;
            if resp.status().as_u16() != 206 {
                return Err(std::io::Error::other("the server stopped doing ranges"));
            }
            let mut body = resp.into_body();
            self.block.clear();
            body.as_reader().read_to_end(&mut self.block)?;
            self.block_start = self.pos;
        }
        let off = (self.pos - self.block_start) as usize;
        let n = out.len().min(self.block.len() - off);
        out[..n].copy_from_slice(&self.block[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for RangeReader {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let new = match to {
            std::io::SeekFrom::Start(n) => n as i128,
            std::io::SeekFrom::End(d) => self.len as i128 + d as i128,
            std::io::SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if new < 0 {
            return Err(std::io::Error::other("seek before the start"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// Where a zip member's bytes are in the file, and what they must be.
#[derive(Debug, Clone, PartialEq)]
pub struct ZipEntryInfo {
    pub data_start: u64,
    pub compressed: u64,
    pub size: u64,
    pub crc32: u32,
    pub deflated: bool,
}

/// Reads the directory of the zip behind `url` (a few Range requests) and finds `member`.
fn zip_entry(url: &str, member: &str, fast: bool) -> Result<ZipEntryInfo> {
    let reader = RangeReader::open(url, fast)?;
    let mut za = zip::ZipArchive::new(reader)
        .map_err(|e| anyhow!("{url} is not a zip file I can read: {e}"))?;
    let e = za
        .by_name_raw(member.as_bytes())
        .map_err(|_| anyhow!("{member} is not in the package at {url}"))?;
    // zip method numbers: 0 stored, 8 deflated (the inflating is done here, with flate2)
    let method = e.compression();
    let deflated = if method == zip::CompressionMethod::STORE {
        false
    } else if method == zip::CompressionMethod::DEFLATE {
        true
    } else {
        bail!("{member} uses the compression {method:?}, which is not supported")
    };
    Ok(ZipEntryInfo {
        data_start: e
            .data_start()
            .ok_or_else(|| anyhow!("cannot find where {member} starts in {url}"))?,
        compressed: e.compressed_size(),
        size: e.size(),
        crc32: e.crc32(),
        deflated,
    })
}

/// Gets one member of a big zip: the directory is read with Range requests, the member's bytes are
/// downloaded (in parallel segments, resumable) to `<final>.download`, then inflated to the final file
/// while the SHA-256 and the zip's own CRC are computed; the pinned hash of the extracted file decides.
fn fetch_zip_member(
    tuning: &Tuning,
    o: &Origin,
    x: &Extract,
    member: &str,
    final_path: &Path,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    let info = zip_entry(&o.url, member, tuning.fast_fail)?;
    if info.size != x.size || info.compressed != o.size {
        bail!(
            "the package at {} has changed ({member} is {} bytes ({} packed), expected {} ({}))",
            o.url,
            info.size,
            info.compressed,
            x.size,
            o.size
        );
    }
    let mut raw_name = final_path.as_os_str().to_owned();
    raw_name.push(".download");
    let raw = PathBuf::from(raw_name);
    let t = Tuning {
        range_base: info.data_start,
        ..tuning.clone()
    };
    download_file_tuned(&t, &o.url, &raw, "", info.compressed, cancel, on_bytes)?;
    // inflate to the final file, checking as it goes
    let tmp = part_path(final_path);
    let result = (|| -> Result<()> {
        let src = File::open(&raw)?;
        let mut reader: Box<dyn Read> = if info.deflated {
            Box::new(flate2::read::DeflateDecoder::new(src))
        } else {
            Box::new(src)
        };
        let mut out = File::create(&tmp)?;
        let mut h = Sha256::new();
        let mut crc = flate2::Crc::new();
        let mut buf = vec![0u8; 256 << 10];
        let mut total = 0u64;
        loop {
            if cancel.is_cancelled() {
                bail!("cancelled");
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > x.size {
                bail!(
                    "{member} unpacks to more than the {} bytes expected",
                    x.size
                );
            }
            out.write_all(&buf[..n])?;
            h.update(&buf[..n]);
            crc.update(&buf[..n]);
        }
        out.flush()?;
        if total != x.size || crc.sum() != info.crc32 {
            bail!("the download of {member} is damaged (size or CRC); it was deleted");
        }
        let got = format!("{:x}", h.finalize());
        if got != x.sha256 {
            bail!(
                "the checksum of the unpacked library does not match (got {got}, expected {}): it was deleted",
                x.sha256
            );
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            fs::rename(&tmp, final_path)?;
            let _ = fs::remove_file(&raw);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            if e.to_string() != "cancelled" {
                // what was downloaded is not what it should be: do not keep it for a resume
                let _ = fs::remove_file(&raw);
                let _ = fs::remove_file(map_path(&raw));
                let _ = fs::remove_file(part_path(&raw));
            }
            Err(e)
        }
    }
}

/// Installs one pack. `done_before`/`total` place the bytes in a bigger plan.
pub fn install_pack(
    store: &Store,
    pack: &Pack,
    cancel: &Cancel,
    done_before: u64,
    total: u64,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let dir = store.dir_of(pack);
    fs::create_dir_all(&dir)?;
    let mut done = done_before;
    for f in &pack.files {
        install_file(store, pack, f, cancel, &mut done, total, progress)?;
    }
    store
        .mark_installed(pack)
        .with_context(|| format!("writing the marker of {}", pack.id))
}

fn install_file(
    store: &Store,
    pack: &Pack,
    f: &FileSpec,
    cancel: &Cancel,
    done: &mut u64,
    total: u64,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    install_file_tuned(
        &Tuning::default(),
        store,
        pack,
        f,
        cancel,
        done,
        total,
        progress,
    )
}

/// Installs one file from its origins in order: the first that works wins. An origin that has another
/// one behind it must connect and answer quickly and keep up a minimum speed (see
/// [`Tuning::with_fallback_behind`]); a slow, unreachable or damaged one is dropped for the next.
#[allow(clippy::too_many_arguments)]
pub fn install_file_tuned(
    tuning: &Tuning,
    store: &Store,
    pack: &Pack,
    f: &FileSpec,
    cancel: &Cancel,
    done: &mut u64,
    total: u64,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let final_path = store.dir_of(pack).join(&f.rel);
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let base = *done;
    // already there (put there by hand, or left by an install whose marker was lost) and exactly the
    // pinned file: nothing to download, and it works without a network
    if f.extract.is_some()
        && fs::metadata(&final_path).is_ok_and(|m| m.len() == f.installed_size())
        && sha256_file(&final_path).is_ok_and(|h| h == f.installed_sha())
    {
        *done = base + f.size;
        progress(Progress {
            pack: pack.id.clone(),
            file: f.rel.clone(),
            done: base + f.size,
            total,
            origin: 0,
        });
        return Ok(());
    }
    let origins = f.origins();
    let mut last_err = None;
    for (i, o) in origins.iter().enumerate() {
        let last = i + 1 == origins.len();
        let t = if last {
            tuning.clone()
        } else {
            tuning.with_fallback_behind()
        };
        let mut file_done = 0u64;
        let r = install_from(&t, o, &final_path, cancel, &mut |n| {
            // a single report of an old part at the start is also an increment from 0
            file_done += n;
            progress(Progress {
                pack: pack.id.clone(),
                file: f.rel.clone(),
                done: base + file_done.min(f.size),
                total,
                origin: i,
            });
        });
        match r {
            Ok(()) => {
                *done = base + f.size;
                return Ok(());
            }
            Err(e) => {
                if cancel.is_cancelled() || e.to_string() == "cancelled" {
                    return Err(e);
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("{} has no origin", f.rel)))
}

/// One origin of a file. The partial download is kept for the next try only when it belongs to this
/// origin (same hash, or the same URL and member).
fn install_from(
    tuning: &Tuning,
    o: &Origin,
    final_path: &Path,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    let target = match &o.extract {
        None => final_path.to_path_buf(),
        Some(_) => {
            let mut s = final_path.as_os_str().to_owned();
            s.push(".download");
            PathBuf::from(s)
        }
    };
    claim_part(&target, o);
    let r = install_from_claimed(tuning, o, &target, final_path, cancel, on_bytes);
    if r.is_ok() {
        let mut name = part_path(&target).into_os_string();
        name.push(".origin");
        let _ = fs::remove_file(PathBuf::from(name));
    }
    r
}

fn install_from_claimed(
    tuning: &Tuning,
    o: &Origin,
    target: &Path,
    final_path: &Path,
    cancel: &Cancel,
    on_bytes: &mut dyn FnMut(u64),
) -> Result<()> {
    let target = target.to_path_buf();
    match &o.extract {
        None => download_file_tuned(tuning, &o.url, &target, &o.sha256, o.size, cancel, on_bytes),
        Some(x) => match &x.kind {
            ArchiveKind::Tgz => {
                download_file_tuned(tuning, &o.url, &target, &o.sha256, o.size, cancel, on_bytes)?;
                let r = extract_member_checked(
                    &target,
                    &x.member_suffix,
                    final_path,
                    Some((&x.sha256, x.size)),
                );
                if let Err(e) = &r {
                    // an archive that does not hold what it should is not kept for a resume either
                    let _ = e;
                    let _ = fs::remove_file(&target);
                }
                r?;
                let _ = fs::remove_file(&target);
                Ok(())
            }
            ArchiveKind::ZipMember { member } => {
                fetch_zip_member(tuning, o, x, member, final_path, cancel, on_bytes)
            }
        },
    }
}

/// Remembers which origin the partial files of `target` belong to; another origin's part is deleted.
fn claim_part(target: &Path, o: &Origin) {
    let id = if o.sha256.is_empty() {
        format!("{}#{:?}", o.url, o.extract.as_ref().map(|x| &x.kind))
    } else {
        o.sha256.clone()
    };
    let mut name = part_path(target).into_os_string();
    name.push(".origin");
    let marker = PathBuf::from(name);
    if fs::read_to_string(&marker).ok().as_deref() != Some(id.as_str()) {
        let _ = fs::remove_file(part_path(target));
        let _ = fs::remove_file(map_path(target));
        let _ = fs::write(&marker, id);
    }
}

/// Installs several packs one after the other with one progress line.
pub fn install_all(
    store: &Store,
    packs: &[Pack],
    cancel: &Cancel,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let total: u64 = packs.iter().map(|p| p.download_size()).sum();
    let mut done = 0u64;
    for p in packs {
        install_pack(store, p, cancel, done, total, progress)?;
        done += p.download_size();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ArchiveKind, Extract, PackKind};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-dl-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sha(b: &[u8]) -> String {
        format!("{:x}", Sha256::digest(b))
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Mode {
        Ranges,
        NoRanges,
        /// Stops after this many bytes (a dropped connection) the first time.
        Drops(usize),
    }

    /// Serves `files` (path -> bytes), honours `Range` unless told not to. Records the
    /// `Range` headers it saw.
    fn serve(
        files: Vec<(&'static str, Vec<u8>)>,
        mode: Mode,
    ) -> (String, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let ranges: Arc<Mutex<Vec<String>>> = Arc::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let (r2, h2) = (ranges.clone(), hits.clone());
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let n = h2.fetch_add(1, Ordering::SeqCst);
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut range = None;
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_lowercase().strip_prefix("range: bytes=") {
                        range = Some(v.trim().trim_end_matches('-').parse::<usize>().unwrap());
                        r2.lock().unwrap().push(h.trim().to_string());
                    }
                }
                let Some((_, body)) = files.iter().find(|f| f.0 == path) else {
                    let _ = s.write_all(
                        b"HTTP/1.1 404 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                };
                let (status, start) = match (range, mode) {
                    (Some(a), Mode::Ranges | Mode::Drops(_)) => (206, a),
                    _ => (200, 0),
                };
                let mut chunk = &body[start..];
                let cut = matches!(mode, Mode::Drops(_)) && n == 0;
                if let (true, Mode::Drops(k)) = (cut, mode) {
                    chunk = &chunk[..k.min(chunk.len())];
                }
                let _ = write!(
                    s,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    if cut { body.len() - start } else { chunk.len() }
                );
                let _ = s.write_all(chunk);
            }
        });
        (url, ranges, hits)
    }

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 251) as u8).collect()
    }

    #[test]
    fn a_file_is_downloaded_verified_and_reported() {
        let body = data(300_000);
        let (url, _, _) = serve(vec![("/f.bin", body.clone())], Mode::Ranges);
        let dir = tmp();
        let dest = dir.join("sub/f.bin");
        let mut seen = 0u64;
        download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |n| seen += n,
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(seen, body.len() as u64);
        assert!(!part_path(&dest).exists());
        // a second call finds the file and does not need the network
        download_file(
            "http://127.0.0.1:1/x",
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_wrong_checksum_deletes_the_file_and_says_so() {
        let body = data(10_000);
        let (url, _, _) = serve(vec![("/f.bin", body.clone())], Mode::Ranges);
        let dir = tmp();
        let dest = dir.join("f.bin");
        let e = download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(b"other"),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("checksum"), "{e}");
        assert!(!dest.exists() && !part_path(&dest).exists());
        // wrong size
        let e = download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(&body),
            20_000,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("wrong size"), "{e}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_part_from_an_earlier_attempt_is_continued_with_a_range_request() {
        let body = data(200_000);
        let (url, ranges, _) = serve(vec![("/f.bin", body.clone())], Mode::Ranges);
        let dir = tmp();
        let dest = dir.join("f.bin");
        fs::write(part_path(&dest), &body[..80_000]).unwrap();
        let mut seen = 0u64;
        download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |n| seen += n,
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(ranges.lock().unwrap().len(), 1);
        assert!(ranges.lock().unwrap()[0].contains("bytes=80000-"));
        assert_eq!(seen, body.len() as u64, "progress counts the part too");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_server_without_ranges_makes_us_start_over() {
        let body = data(120_000);
        let (url, _, _) = serve(vec![("/f.bin", body.clone())], Mode::NoRanges);
        let dir = tmp();
        let dest = dir.join("f.bin");
        fs::write(part_path(&dest), &body[..50_000]).unwrap();
        download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_dropped_connection_leaves_a_part_that_the_next_try_finishes() {
        let body = data(200_000);
        let (url, ranges, _) = serve(vec![("/f.bin", body.clone())], Mode::Drops(60_000));
        let dir = tmp();
        let dest = dir.join("f.bin");
        let u = format!("{url}/f.bin");
        let first = download_file(
            &u,
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        );
        assert!(first.is_err(), "the cut connection is an error");
        let have = fs::metadata(part_path(&dest)).unwrap().len();
        assert!(have > 0 && have < body.len() as u64, "{have}");
        download_file(
            &u,
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert!(ranges.lock().unwrap()[0].contains(&format!("bytes={have}-")));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cancelling_stops_and_keeps_the_part() {
        let body = data(2_000_000);
        let (url, _, _) = serve(vec![("/f.bin", body.clone())], Mode::Ranges);
        let dir = tmp();
        let dest = dir.join("f.bin");
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        let e = download_file(
            &format!("{url}/f.bin"),
            &dest,
            &sha(&body),
            body.len() as u64,
            &cancel,
            &mut |_| c2.cancel(),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(e, "cancelled");
        assert!(!dest.exists());
        assert!(part_path(&dest).exists(), "kept for resuming");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn errors_of_the_server_are_reported() {
        let (url, _, _) = serve(vec![], Mode::Ranges);
        let dir = tmp();
        let e = download_file(
            &format!("{url}/missing"),
            &dir.join("x"),
            &sha(b""),
            5,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("404"), "{e}");
        let e = download_file(
            "http://127.0.0.1:1/x",
            &dir.join("y"),
            &sha(b""),
            5,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("cannot reach"), "{e}");
        fs::remove_dir_all(&dir).ok();
    }

    fn tgz(member: &str, content: &[u8]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            let mut h = tar::Header::new_gnu();
            h.set_size(6);
            h.set_mode(0o644);
            b.append_data(&mut h, "pkg/README", &b"readme"[..]).unwrap();
            let mut h = tar::Header::new_gnu();
            h.set_size(content.len() as u64);
            h.set_mode(0o755);
            b.append_data(&mut h, member, content).unwrap();
            b.finish().unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    /// (file, bytes to serve, and for an archive: member suffix and the extracted content)
    type FakeFile<'a> = (&'a str, &'a Vec<u8>, Option<(&'a str, &'a Vec<u8>)>);

    fn fake_pack(url: &str, files: Vec<FakeFile>) -> Pack {
        Pack {
            id: "fake".into(),
            kind: PackKind::PiperVoice,
            title: "Fake".into(),
            summary: String::new(),
            languages: vec!["en".into()],
            license_name: "MIT".into(),
            license_url: String::new(),
            license_note: "x".into(),
            requires: vec![],
            dir: "fake".into(),
            files: files
                .into_iter()
                .map(|(rel, bytes, ex)| FileSpec {
                    rel: rel.into(),
                    url: format!("{url}/{rel}"),
                    sha256: sha(bytes),
                    size: bytes.len() as u64,
                    extract: ex.map(|(s, content)| Extract {
                        kind: ArchiveKind::Tgz,
                        member_suffix: s.into(),
                        sha256: sha(content),
                        size: content.len() as u64,
                    }),
                    fallbacks: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn a_pack_is_installed_with_one_progress_line_and_an_extracted_archive() {
        let lib = data(50_000);
        let archive = tgz("onnxruntime-linux-x64-9.9/lib/libonnxruntime.so.9.9", &lib);
        let model = data(70_000);
        let (url, _, _) = serve(
            vec![
                ("/lib.tgz", archive.clone()),
                ("/model.onnx", model.clone()),
            ],
            Mode::Ranges,
        );
        let mut pack = fake_pack(
            &url,
            vec![
                (
                    "lib.tgz",
                    &archive,
                    Some(("/lib/libonnxruntime.so.9.9", &lib)),
                ),
                ("model.onnx", &model, None),
            ],
        );
        pack.files[0].rel = "lib.so".into();
        pack.files[0].url = format!("{url}/lib.tgz");
        let root = tmp();
        let store = Store::new(&root);
        let mut lines: Vec<Progress> = Vec::new();
        install_all(
            &store,
            std::slice::from_ref(&pack),
            &Cancel::new(),
            &mut |p| lines.push(p),
        )
        .unwrap();
        assert_eq!(fs::read(root.join("fake/lib.so")).unwrap(), lib);
        assert_eq!(fs::read(root.join("fake/model.onnx")).unwrap(), model);
        assert!(
            !root.join("fake/lib.so.download").exists(),
            "the archive is removed"
        );
        assert!(store.is_installed(&pack));
        let total = (archive.len() + model.len()) as u64;
        let last = lines.last().unwrap();
        assert_eq!((last.done, last.total), (total, total));
        assert!(
            lines.windows(2).all(|w| w[0].done <= w[1].done),
            "progress never goes back"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_member_that_is_not_in_the_archive_is_an_error() {
        let a = tgz("pkg/other", b"x");
        let dir = tmp();
        let p = dir.join("a.tgz");
        fs::write(&p, a).unwrap();
        let e = extract_member(&p, "/lib/libx.so", &dir.join("out"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("not in the archive"));
        fs::remove_dir_all(&dir).ok();
    }

    /// Opt-in: downloads the real catalog entry `BOOKX_TTS_LIVE_PACK` (default: the ONNX
    /// runtime) with its requirements to `BOOKX_TTS_DIR` (or a temporary store) to check URLs and hashes.
    #[test]
    fn live_download_of_a_real_pack() {
        if std::env::var("BOOKX_TTS_LIVE").is_err() {
            return;
        }
        let id = std::env::var("BOOKX_TTS_LIVE_PACK").unwrap_or_else(|_| "onnxruntime".into());
        let pack = crate::models::find_pack(&id).expect("a pack of the catalog");
        let root = std::env::var("BOOKX_TTS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| tmp());
        let store = Store::new(&root);
        let plan = crate::models::install_plan(&crate::models::catalog(), &store, &id);
        let t = std::time::Instant::now();
        let mut last = 0u64;
        install_all(&store, &plan, &Cancel::new(), &mut |p| last = p.done).unwrap();
        assert!(store.is_installed(&pack));
        println!(
            "downloaded {id}: {last} bytes in {:.1}s",
            t.elapsed().as_secs_f64()
        );
    }
}

#[cfg(test)]
mod seg_tests {
    //! The parallel downloader against a local server that can be slow, drop connections, or refuse
    //! ranges.
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Mode {
        Ranges,
        NoRanges,
        /// The first connection that asks for more than a byte is cut after this many bytes.
        CutFirst(usize),
    }

    pub(super) struct Server {
        pub(super) url: String,
        pub(super) ranges: Arc<Mutex<Vec<String>>>,
        pub(super) max_active: Arc<AtomicUsize>,
    }

    /// Serves `/f.bin`; every 16 KiB takes `delay_ms`, so a download takes a while and the
    /// connections overlap.
    pub(super) fn serve(body: Vec<u8>, mode: Mode, delay_ms: u64) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/f.bin", l.local_addr().unwrap());
        let ranges: Arc<Mutex<Vec<String>>> = Arc::default();
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let cut_done = Arc::new(AtomicUsize::new(0));
        let body = Arc::new(body);
        let (r2, a2, m2) = (ranges.clone(), active, max_active.clone());
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (body, r2, a2, m2, cut_done) = (
                    body.clone(),
                    r2.clone(),
                    a2.clone(),
                    m2.clone(),
                    cut_done.clone(),
                );
                std::thread::spawn(move || {
                    let mut s = s;
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let mut line = String::new();
                    let _ = r.read_line(&mut line);
                    let mut range: Option<(usize, Option<usize>)> = None;
                    loop {
                        let mut h = String::new();
                        if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if let Some(v) = h.to_lowercase().strip_prefix("range: bytes=") {
                            let v = v.trim();
                            r2.lock().unwrap().push(v.to_string());
                            let (a, b) = v.split_once('-').unwrap();
                            range = Some((a.parse().unwrap(), b.parse().ok()));
                        }
                    }
                    let n = body.len();
                    let (status, a, b) = match (range, mode) {
                        (Some((a, b)), Mode::Ranges | Mode::CutFirst(_)) => {
                            (206, a, b.unwrap_or(n - 1).min(n - 1))
                        }
                        _ => (200, 0, n - 1),
                    };
                    let now = a2.fetch_add(1, Ordering::SeqCst) + 1;
                    m2.fetch_max(now, Ordering::SeqCst);
                    let len = b + 1 - a;
                    let head = if status == 206 {
                        format!(
                            "HTTP/1.1 206 P\r\nContent-Length: {len}\r\nContent-Range: bytes {a}-{b}/{n}\r\nConnection: close\r\n\r\n"
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 O\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"
                        )
                    };
                    let _ = s.write_all(head.as_bytes());
                    let cut = match mode {
                        Mode::CutFirst(k)
                            if len > 1 && cut_done.fetch_add(1, Ordering::SeqCst) == 0 =>
                        {
                            Some(k)
                        }
                        _ => None,
                    };
                    let mut sent = 0;
                    for chunk in body[a..=b].chunks(16 * 1024) {
                        if delay_ms > 0 && len > 1 {
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                        if cut.is_some_and(|k| sent >= k) {
                            break;
                        }
                        if s.write_all(chunk).is_err() {
                            break;
                        }
                        sent += chunk.len();
                    }
                    a2.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Server {
            url,
            ranges,
            max_active,
        }
    }

    pub(super) fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 13 % 251) as u8).collect()
    }

    pub(super) fn sha(b: &[u8]) -> String {
        format!("{:x}", Sha256::digest(b))
    }

    pub(super) fn dir() -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-seg-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn small(segments: usize) -> Tuning {
        Tuning {
            segments,
            min_parallel_bytes: 64 << 10,
            min_segment_bytes: 64 << 10,
            buf_bytes: 16 << 10,
            retries: 2,
            retry_pause: Duration::from_millis(10),
            ..Tuning::default()
        }
    }

    fn run(
        t: &Tuning,
        srv: &Server,
        dest: &Path,
        body: &[u8],
        cancel: &Cancel,
    ) -> (Result<()>, u64) {
        let mut seen = 0u64;
        let r = download_file_tuned(
            t,
            &srv.url,
            dest,
            &sha(body),
            body.len() as u64,
            cancel,
            &mut |n| seen += n,
        );
        (r, seen)
    }

    #[test]
    fn the_plan_pieces_cover_the_file_and_read_back() {
        let segs = split(0, 1000, 4, 100);
        assert_eq!(segs.len(), 4);
        assert_eq!((segs[0].start, segs[3].end), (0, 1000));
        assert!(segs.windows(2).all(|w| w[0].end == w[1].start));
        // never smaller than the minimum, never more than asked
        assert_eq!(split(0, 250, 8, 100).len(), 2);
        assert_eq!(split(0, 50, 8, 100).len(), 1);
        assert!(split(5, 5, 4, 1).is_empty());
        assert_eq!(split(300, 1000, 2, 1)[0].start, 300);
        // the saved plan
        let mut p = split(0, 1000, 4, 100);
        p[0].done = 250;
        p[1].done = 100;
        assert_eq!(parse_plan(&plan_text(&p), 1000).unwrap(), p);
        assert_eq!(
            contiguous_prefix(&p),
            350,
            "stops at the first piece not finished"
        );
        p[0].done = 250;
        p[0].done = p[0].len();
        assert!(contiguous_prefix(&p) >= p[0].len());
        // a plan that does not fit is not trusted
        assert!(parse_plan("0 10 0\n20 30 0\n", 30).is_none(), "a gap");
        assert!(
            parse_plan("0 10 11\n", 10).is_none(),
            "more done than the piece"
        );
        assert!(parse_plan("0 10 0\n", 99).is_none(), "another file size");
        assert!(parse_plan("garbage", 10).is_none());
        assert!(parse_plan("", 10).is_none());
    }

    #[test]
    fn a_big_file_comes_in_parallel_ranges_and_is_verified_on_the_way() {
        let body = data(2 << 20);
        let srv = serve(body.clone(), Mode::Ranges, 2);
        let d = dir();
        let dest = d.join("f.bin");
        let (r, seen) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(
            seen,
            body.len() as u64,
            "progress adds up to the size, once"
        );
        let ranges = srv.ranges.lock().unwrap().clone();
        assert!(ranges.contains(&"0-0".to_string()), "the probe: {ranges:?}");
        assert_eq!(ranges.len(), 5, "the probe and four segments: {ranges:?}");
        assert!(
            srv.max_active.load(Ordering::SeqCst) >= 3,
            "connections overlapped"
        );
        assert!(!part_path(&dest).exists() && !map_path(&dest).exists());
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn parallel_is_faster_than_one_stream_when_the_server_limits_each_connection() {
        let body = data(2 << 20);
        let d = dir();
        let srv = serve(body.clone(), Mode::Ranges, 6);
        let t = Instant::now();
        run(&small(1), &srv, &d.join("one.bin"), &body, &Cancel::new())
            .0
            .unwrap();
        let single = t.elapsed();
        let t = Instant::now();
        run(&small(4), &srv, &d.join("four.bin"), &body, &Cancel::new())
            .0
            .unwrap();
        let four = t.elapsed();
        assert!(
            four.as_secs_f64() < single.as_secs_f64() * 0.6,
            "single {single:?}, four segments {four:?}"
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_saved_plan_resumes_only_what_is_missing() {
        let body = data(1 << 20);
        let srv = serve(body.clone(), Mode::Ranges, 0);
        let d = dir();
        let dest = d.join("f.bin");
        let size = body.len() as u64;
        let mut plan = split(0, size, 4, 64 << 10);
        // segments 0 and 2 are half there, 1 is finished
        let mut part = vec![0u8; body.len()];
        for (i, frac) in [(0usize, 2u64), (1, 1), (2, 2)] {
            let s = &mut plan[i];
            s.done = s.len() / frac;
            part[s.start as usize..(s.start + s.done) as usize]
                .copy_from_slice(&body[s.start as usize..(s.start + s.done) as usize]);
        }
        fs::write(part_path(&dest), &part).unwrap();
        fs::write(map_path(&dest), plan_text(&plan)).unwrap();
        let (r, seen) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(seen, size, "progress counts what was already there");
        let ranges = srv.ranges.lock().unwrap().clone();
        // the probe, segment 0 from the middle, segment 2 from the middle and the untouched segment 3
        assert_eq!(ranges.len(), 4, "{ranges:?}");
        let from = |s: &Seg| format!("{}-{}", s.start + s.done, s.end - 1);
        assert!(ranges.contains(&from(&plan[0])));
        assert!(ranges.contains(&from(&plan[2])));
        assert!(ranges.contains(&from(&plan[3])));
        assert!(
            !ranges
                .iter()
                .any(|r| r.starts_with(&format!("{}-", plan[1].start + plan[1].done)))
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn an_old_single_stream_part_is_kept_as_the_first_piece() {
        let body = data(1 << 20);
        let srv = serve(body.clone(), Mode::Ranges, 0);
        let d = dir();
        let dest = d.join("f.bin");
        fs::write(part_path(&dest), &body[..300_000]).unwrap();
        let (r, _) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        let ranges = srv.ranges.lock().unwrap().clone();
        assert!(
            !ranges.iter().any(|r| r.starts_with("0-") && r != "0-0"),
            "the first 300000 bytes are not fetched again: {ranges:?}"
        );
        assert!(
            ranges.iter().any(|r| r.starts_with("300000-")),
            "{ranges:?}"
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_server_without_ranges_gets_one_stream() {
        let body = data(1 << 20);
        let srv = serve(body.clone(), Mode::NoRanges, 0);
        let d = dir();
        let dest = d.join("f.bin");
        let (r, seen) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(seen, body.len() as u64);
        assert!(!part_path(&dest).exists() && !map_path(&dest).exists());
        assert!(
            srv.max_active.load(Ordering::SeqCst) <= 2,
            "no parallel connections"
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_wrong_checksum_deletes_everything_and_says_so() {
        let body = data(1 << 20);
        let srv = serve(body.clone(), Mode::Ranges, 0);
        let d = dir();
        let dest = d.join("f.bin");
        let e = download_file_tuned(
            &small(4),
            &srv.url,
            &dest,
            &sha(b"something else"),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("checksum"), "{e}");
        assert!(!dest.exists() && !part_path(&dest).exists() && !map_path(&dest).exists());
        // a size that is not the file's
        let e = download_file_tuned(
            &small(4),
            &srv.url,
            &dest,
            &sha(&body),
            body.len() as u64 + 5,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("wrong size"), "{e}");
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn cancelling_keeps_the_plan_and_the_next_try_finishes_it() {
        let body = data(2 << 20);
        let srv = serve(body.clone(), Mode::Ranges, 8);
        let d = dir();
        let dest = d.join("f.bin");
        let cancel = Cancel::new();
        let mut seen = 0u64;
        let c2 = cancel.clone();
        let r = download_file_tuned(
            &small(4),
            &srv.url,
            &dest,
            &sha(&body),
            body.len() as u64,
            &cancel,
            &mut |n| {
                seen += n;
                if seen > 300_000 {
                    c2.cancel();
                }
            },
        );
        assert_eq!(r.unwrap_err().to_string(), "cancelled");
        assert!(!dest.exists());
        let plan = parse_plan(
            &fs::read_to_string(map_path(&dest)).unwrap(),
            body.len() as u64,
        )
        .expect("the plan was saved");
        let got: u64 = plan.iter().map(|s| s.done).sum();
        assert!(got > 0 && got < body.len() as u64, "{got}");
        srv.ranges.lock().unwrap().clear();
        let (r, seen) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        assert_eq!(seen, body.len() as u64);
        let asked: u64 = srv
            .ranges
            .lock()
            .unwrap()
            .iter()
            .filter(|r| *r != "0-0")
            .map(|r| {
                let (a, b) = r.split_once('-').unwrap();
                b.parse::<u64>().unwrap() + 1 - a.parse::<u64>().unwrap()
            })
            .sum();
        assert!(
            asked <= body.len() as u64 - got + 4 * 16 * 1024,
            "asked {asked} for {} missing",
            body.len() as u64 - got
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_connection_cut_in_the_middle_of_a_segment_is_continued() {
        let body = data(1 << 20);
        let srv = serve(body.clone(), Mode::CutFirst(40_000), 1);
        let d = dir();
        let dest = d.join("f.bin");
        let (r, _) = run(&small(4), &srv, &dest, &body, &Cancel::new());
        r.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);
        // 4 segments + the probe + at least one more request for the piece that was cut
        assert!(srv.ranges.lock().unwrap().len() >= 6);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_server_that_is_gone_fails_with_a_message_the_app_can_sort() {
        let body = data(1 << 20);
        let d = dir();
        let dest = d.join("f.bin");
        let e = download_file_tuned(
            &small(4),
            "http://127.0.0.1:1/f.bin",
            &dest,
            &sha(&body),
            body.len() as u64,
            &Cancel::new(),
            &mut |_| {},
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            crate::setup::classify(&e),
            crate::setup::Failure::Offline,
            "{e}"
        );
        fs::remove_dir_all(&d).ok();
    }
}

#[cfg(test)]
mod origin_tests {
    //! Several origins for one file: a slow, dead or wrong first origin gives way to the next; one
    //! member of a big zip is read with Range requests; archives are unpacked safely and the extracted
    //! file is checked against its pinned hash.
    use super::seg_tests::{Mode, data, dir, serve, sha};
    use super::*;
    use crate::models::{PackKind, Store};
    use std::io::Write;

    const MEMBER: &str = "runtimes/linux-x64/native/libonnxruntime.so";

    fn tgz_with(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut raw = Vec::new();
        {
            let mut b = tar::Builder::new(&mut raw);
            for (path, content, kind) in entries {
                let mut h = tar::Header::new_gnu();
                // write the name by hand: the builder refuses `..` and absolute paths, which is
                // exactly what these archives must contain
                let name = path.as_bytes();
                h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
                h.set_size(if *kind == b'0' {
                    content.len() as u64
                } else {
                    0
                });
                h.set_mode(0o644);
                h.set_entry_type(tar::EntryType::new(*kind));
                if *kind == b'2' {
                    h.set_link_name("/etc/passwd").unwrap();
                }
                h.set_cksum();
                let body: &[u8] = if *kind == b'0' { content } else { &[] };
                b.append(&h, body).unwrap();
            }
            b.finish().unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    /// A zip like the NuGet package: a big filler member, then the library, deflated. Returns the zip
    /// and the size of the library's compressed bytes.
    fn nupkg(lib: &[u8]) -> (Vec<u8>, u64) {
        let mut cur = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut cur);
            let stored = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            w.start_file("runtimes/win-x64/native/onnxruntime.dll", stored)
                .unwrap();
            w.write_all(&vec![7u8; 3_000_000]).unwrap();
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            w.start_file(MEMBER, deflated).unwrap();
            w.write_all(lib).unwrap();
            w.start_file("tail.txt", stored).unwrap();
            w.write_all(b"the end").unwrap();
            w.finish().unwrap();
        }
        let bytes = cur.into_inner();
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let csize = za.by_name_raw(MEMBER.as_bytes()).unwrap().compressed_size();
        (bytes, csize)
    }

    /// Bytes that do not compress (a repeating pattern would make a tiny archive).
    fn noise(n: usize) -> Vec<u8> {
        let mut x = 0x2545F4914F6CDD1Du64;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    fn lib_extract(kind: ArchiveKind, suffix: &str, lib: &[u8]) -> Extract {
        Extract {
            kind,
            member_suffix: suffix.into(),
            sha256: sha(lib),
            size: lib.len() as u64,
        }
    }

    fn tgz_origin(url: &str, archive: &[u8], lib: &[u8]) -> Origin {
        Origin {
            url: url.into(),
            sha256: sha(archive),
            size: archive.len() as u64,
            extract: Some(lib_extract(
                ArchiveKind::Tgz,
                "/lib/libonnxruntime.so.9.9",
                lib,
            )),
        }
    }

    fn zip_origin(url: &str, csize: u64, lib: &[u8]) -> Origin {
        Origin {
            url: url.into(),
            sha256: String::new(),
            size: csize,
            extract: Some(lib_extract(
                ArchiveKind::ZipMember {
                    member: MEMBER.into(),
                },
                "",
                lib,
            )),
        }
    }

    fn file_of(origins: Vec<Origin>) -> FileSpec {
        let first = origins[0].clone();
        FileSpec {
            rel: "libonnxruntime.so".into(),
            url: first.url,
            sha256: first.sha256,
            size: first.size,
            extract: first.extract,
            fallbacks: origins[1..].to_vec(),
        }
    }

    fn pack_of(f: FileSpec) -> Pack {
        Pack {
            id: "rt".into(),
            kind: PackKind::Runtime,
            title: "rt".into(),
            summary: String::new(),
            languages: vec![],
            license_name: "MIT".into(),
            license_url: String::new(),
            license_note: "x".into(),
            requires: vec![],
            dir: "rt".into(),
            files: vec![f],
        }
    }

    fn quick() -> Tuning {
        Tuning {
            segments: 4,
            min_parallel_bytes: 64 << 10,
            min_segment_bytes: 64 << 10,
            buf_bytes: 16 << 10,
            retries: 1,
            retry_pause: Duration::from_millis(10),
            // an origin with another behind it must keep up 100 KB/s, judged over 600 ms
            min_speed: Some((100_000, Duration::from_millis(600))),
            ..Tuning::default()
        }
    }

    struct Run {
        result: Result<()>,
        origins_seen: Vec<usize>,
        installed: Option<Vec<u8>>,
        root: PathBuf,
        store: Store,
        pack: Pack,
    }

    fn install(f: FileSpec, t: &Tuning) -> Run {
        let root = dir();
        let store = Store::new(&root);
        let pack = pack_of(f);
        let mut seen = Vec::new();
        let mut done = 0;
        let r = install_file_tuned(
            t,
            &store,
            &pack,
            &pack.files[0],
            &Cancel::new(),
            &mut done,
            pack.files[0].size,
            &mut |p| {
                if seen.last() != Some(&p.origin) {
                    seen.push(p.origin);
                }
            },
        );
        let installed = fs::read(store.dir_of(&pack).join("libonnxruntime.so")).ok();
        Run {
            result: r,
            origins_seen: seen,
            installed,
            root,
            store,
            pack,
        }
    }

    fn cleanup(r: &Run) {
        fs::remove_dir_all(&r.root).ok();
    }

    #[test]
    fn the_watchdog_judges_the_speed_over_its_window() {
        let t0 = std::time::Instant::now();
        let mut d = Watchdog::new(Some((100_000, Duration::from_millis(500)))).unwrap();
        // fast: 1 MB/s
        for i in 0..8u64 {
            let now = t0 + Duration::from_millis(i * 100);
            assert!(!d.too_slow(now, i * 100_000), "{i}");
        }
        // it stalls: after a window of nothing it is judged slow
        let mut slow = false;
        for i in 8..20u64 {
            slow |= d.too_slow(t0 + Duration::from_millis(i * 100), 700_000);
        }
        assert!(slow);
        // never judged during the first window (a slow start is not a slow origin)
        let mut d = Watchdog::new(Some((100_000, Duration::from_millis(500)))).unwrap();
        assert!(!d.too_slow(std::time::Instant::now(), 0));
        assert!(Watchdog::new(None).is_none());
    }

    #[test]
    fn one_member_of_a_big_zip_is_read_with_range_requests_and_checked() {
        let lib = data(400_000);
        let (zip_bytes, csize) = nupkg(&lib);
        let srv = serve(zip_bytes.clone(), Mode::Ranges, 0);
        let run = install(file_of(vec![zip_origin(&srv.url, csize, &lib)]), &quick());
        run.result.as_ref().unwrap();
        assert_eq!(run.installed.as_deref(), Some(&lib[..]));
        // not the whole zip: the filler member of 3 MB was never asked for
        let asked: u64 = srv
            .ranges
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                let (a, b) = r.split_once('-').unwrap();
                let a: u64 = a.parse().unwrap();
                let b: u64 = b.parse().unwrap_or(zip_bytes.len() as u64 - 1);
                b + 1 - a
            })
            .sum();
        assert!(
            asked < 2 * csize + 600_000 && (asked as usize) < zip_bytes.len() / 2,
            "asked {asked} bytes of {} (the member is {csize} packed)",
            zip_bytes.len()
        );
        assert!(
            run.store.is_installed(&run.pack) || {
                // the marker is written by install_pack, not by the file: write it and check detection
                run.store.mark_installed(&run.pack).unwrap();
                run.store.is_installed(&run.pack)
            }
        );
        let marker =
            fs::read_to_string(run.store.dir_of(&run.pack).join(".installed-rt.json")).unwrap();
        assert!(
            marker.contains(&sha(&lib)),
            "the marker holds the hash of the extracted file"
        );
        cleanup(&run);
    }

    #[test]
    fn a_slow_first_origin_is_given_up_for_the_next() {
        let lib = noise(300_000);
        let archive = tgz_with(&[("onnxruntime-9.9/lib/libonnxruntime.so.9.9", &lib, b'0')]);
        // 16 KB every 250 ms is 64 KB/s, under the 100 KB/s minimum
        let slow = serve(archive.clone(), Mode::Ranges, 250);
        let fast = serve(archive.clone(), Mode::Ranges, 0);
        let t0 = std::time::Instant::now();
        let run = install(
            file_of(vec![
                tgz_origin(&slow.url, &archive, &lib),
                tgz_origin(&fast.url, &archive, &lib),
            ]),
            // one stream: four parallel connections of 64 KB/s would add up to more than the minimum
            &Tuning {
                segments: 1,
                ..quick()
            },
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.installed.as_deref(), Some(&lib[..]));
        assert_eq!(
            run.origins_seen,
            vec![0, 1],
            "it began at the first and went on at the second"
        );
        assert!(
            t0.elapsed() < Duration::from_secs(4),
            "{:?}: it did not sit out the slow download",
            t0.elapsed()
        );
        cleanup(&run);
    }

    #[test]
    fn the_last_origin_is_waited_for_however_slow() {
        let lib = noise(120_000);
        let archive = tgz_with(&[("o/lib/libonnxruntime.so.9.9", &lib, b'0')]);
        let slow = serve(archive.clone(), Mode::Ranges, 120);
        let run = install(
            file_of(vec![tgz_origin(&slow.url, &archive, &lib)]),
            &quick(),
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.origins_seen, vec![0]);
        cleanup(&run);
    }

    #[test]
    fn an_unreachable_first_origin_gives_way_and_all_failing_says_why() {
        let lib = data(100_000);
        let archive = tgz_with(&[("o/lib/libonnxruntime.so.9.9", &lib, b'0')]);
        let good = serve(archive.clone(), Mode::Ranges, 0);
        let run = install(
            file_of(vec![
                tgz_origin("http://127.0.0.1:1/dead.tgz", &archive, &lib),
                tgz_origin(&good.url, &archive, &lib),
            ]),
            &quick(),
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.origins_seen, vec![1], "nothing came from the dead one");
        cleanup(&run);
        let run = install(
            file_of(vec![
                tgz_origin("http://127.0.0.1:1/a.tgz", &archive, &lib),
                tgz_origin("http://127.0.0.1:1/b.tgz", &archive, &lib),
            ]),
            &quick(),
        );
        let e = run.result.as_ref().unwrap_err().to_string();
        assert!(
            e.contains("cannot reach") && e.contains("b.tgz"),
            "the last origin's error: {e}"
        );
        assert_eq!(crate::setup::classify(&e), crate::setup::Failure::Offline);
        assert!(run.installed.is_none());
        cleanup(&run);
    }

    #[test]
    fn a_wrong_archive_hash_or_a_wrong_library_inside_falls_to_the_next_origin() {
        let lib = data(200_000);
        let archive = tgz_with(&[("o/lib/libonnxruntime.so.9.9", &lib, b'0')]);
        let good = serve(archive.clone(), Mode::Ranges, 0);
        // 1. the archive of the first origin is not what its hash says
        let mut bad_hash = tgz_origin(&good.url, &archive, &lib);
        bad_hash.sha256 = sha(b"something else");
        let run = install(
            file_of(vec![bad_hash, tgz_origin(&good.url, &archive, &lib)]),
            &quick(),
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.origins_seen, vec![0, 1]);
        cleanup(&run);
        // 2. the archive is right but the library in it is not the pinned one (the hash of the archive
        //    is of that archive): the extracted file is checked and deleted
        let other = data(200_000).iter().map(|b| b ^ 1).collect::<Vec<u8>>();
        let wrong_archive = tgz_with(&[("o/lib/libonnxruntime.so.9.9", &other, b'0')]);
        let wrong = serve(wrong_archive.clone(), Mode::Ranges, 0);
        let mut o = tgz_origin(&wrong.url, &wrong_archive, &lib);
        o.sha256 = sha(&wrong_archive);
        let run = install(file_of(vec![o.clone()]), &quick());
        let e = run.result.as_ref().unwrap_err().to_string();
        assert!(e.contains("checksum of the unpacked library"), "{e}");
        assert!(run.installed.is_none(), "the wrong library was deleted");
        assert!(!part_path(&run.store.dir_of(&run.pack).join("libonnxruntime.so")).exists());
        cleanup(&run);
        // and with a good origin behind it
        let run = install(
            file_of(vec![o, tgz_origin(&good.url, &archive, &lib)]),
            &quick(),
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.installed.as_deref(), Some(&lib[..]));
        cleanup(&run);
    }

    #[test]
    fn a_zip_member_that_is_not_the_pinned_library_is_refused_and_deleted() {
        let lib = data(150_000);
        let other: Vec<u8> = lib.iter().map(|b| b.wrapping_add(1)).collect();
        // the package holds `other` where the pin says `lib`
        let (zip_bytes, csize) = nupkg(&other);
        let srv = serve(zip_bytes, Mode::Ranges, 0);
        let mut o = zip_origin(&srv.url, csize, &other);
        o.extract.as_mut().unwrap().sha256 = sha(&lib); // pinned: the real library
        let run = install(file_of(vec![o.clone()]), &quick());
        let e = run.result.as_ref().unwrap_err().to_string();
        assert!(e.contains("checksum of the unpacked library"), "{e}");
        assert!(run.installed.is_none());
        let dl = run
            .store
            .dir_of(&run.pack)
            .join("libonnxruntime.so.download");
        assert!(
            !dl.exists(),
            "the downloaded bytes are not kept for a resume"
        );
        cleanup(&run);
        // a package that changed its size is noticed before anything is downloaded
        let (zip_bytes, csize) = nupkg(&lib);
        let srv = serve(zip_bytes, Mode::Ranges, 0);
        let mut changed = zip_origin(&srv.url, csize + 5, &lib);
        changed.size = csize + 5;
        let run = install(file_of(vec![changed]), &quick());
        assert!(run.result.unwrap_err().to_string().contains("has changed"));
    }

    #[test]
    fn a_server_without_ranges_cannot_give_a_zip_member_so_the_next_origin_is_used() {
        let lib = data(100_000);
        let (zip_bytes, csize) = nupkg(&lib);
        let no_ranges = serve(zip_bytes, Mode::NoRanges, 0);
        let archive = tgz_with(&[("o/lib/libonnxruntime.so.9.9", &lib, b'0')]);
        let good = serve(archive.clone(), Mode::Ranges, 0);
        let run = install(
            file_of(vec![
                zip_origin(&no_ranges.url, csize, &lib),
                tgz_origin(&good.url, &archive, &lib),
            ]),
            &quick(),
        );
        run.result.as_ref().unwrap();
        assert_eq!(run.origins_seen, vec![1]);
        cleanup(&run);
    }

    #[test]
    fn an_unsafe_oversized_or_linked_member_is_not_unpacked() {
        let d = dir();
        let lib = data(5_000);
        let suffix = "/lib/libonnxruntime.so.9.9";
        let pinned = (sha(&lib), lib.len() as u64);
        let pin = Some((pinned.0.as_str(), pinned.1));
        // a path that climbs out of the archive's folder
        let evil = d.join("evil.tgz");
        fs::write(
            &evil,
            tgz_with(&[("../x/lib/libonnxruntime.so.9.9", &lib, b'0')]),
        )
        .unwrap();
        let e = extract_member_checked(&evil, suffix, &d.join("out.so"), pin)
            .unwrap_err()
            .to_string();
        assert!(e.contains("unsafe path"), "{e}");
        assert!(!d.join("out.so").exists());
        // an absolute path
        fs::write(
            &evil,
            tgz_with(&[("/abs/lib/libonnxruntime.so.9.9", &lib, b'0')]),
        )
        .unwrap();
        assert!(extract_member_checked(&evil, suffix, &d.join("out.so"), pin).is_err());
        // a member bigger than the pinned size is refused before it is written
        let big = data(50_000);
        fs::write(
            &evil,
            tgz_with(&[("o/lib/libonnxruntime.so.9.9", &big, b'0')]),
        )
        .unwrap();
        let e = extract_member_checked(&evil, suffix, &d.join("out.so"), pin)
            .unwrap_err()
            .to_string();
        assert!(e.contains("not the") || e.contains("bigger"), "{e}");
        assert!(!d.join("out.so").exists() && !part_path(&d.join("out.so")).exists());
        // without a pinned size nothing above the global cap is taken either: the cap is applied by `take`
        // a symlink with the right name is not a file and is passed over
        fs::write(
            &evil,
            tgz_with(&[("o/lib/libonnxruntime.so.9.9", &[], b'2')]),
        )
        .unwrap();
        let e = extract_member_checked(&evil, suffix, &d.join("out.so"), pin)
            .unwrap_err()
            .to_string();
        assert!(e.contains("is not in the archive"), "{e}");
        // the good one
        fs::write(
            &evil,
            tgz_with(&[("o/lib/libonnxruntime.so.9.9", &lib, b'0')]),
        )
        .unwrap();
        extract_member_checked(&evil, suffix, &d.join("out.so"), pin).unwrap();
        assert_eq!(fs::read(d.join("out.so")).unwrap(), lib);
        // the old entry point still works for archives without a pin
        extract_member(&evil, suffix, &d.join("plain.so")).unwrap();
        assert_eq!(fs::read(d.join("plain.so")).unwrap(), lib);
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_part_of_one_origin_is_not_used_for_another() {
        let d = dir();
        let target = d.join("lib.so.download");
        let a = Origin {
            url: "http://a/x".into(),
            sha256: "aa".repeat(32),
            size: 10,
            extract: None,
        };
        let same_file_elsewhere = Origin {
            url: "http://mirror/x".into(),
            ..a.clone()
        };
        let other = Origin {
            sha256: "bb".repeat(32),
            ..a.clone()
        };
        claim_part(&target, &a);
        fs::write(part_path(&target), b"half").unwrap();
        claim_part(&target, &same_file_elsewhere);
        assert!(
            part_path(&target).exists(),
            "a mirror of the same file (same hash) keeps the part"
        );
        claim_part(&target, &other);
        assert!(
            !part_path(&target).exists(),
            "another file does not inherit it"
        );
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_library_put_in_the_folder_by_hand_is_taken_without_any_download() {
        let lib = data(80_000);
        // both origins are dead: only the file on disk can make this work
        let f = file_of(vec![
            zip_origin("http://127.0.0.1:1/a.nupkg", 1234, &lib),
            tgz_origin("http://127.0.0.1:1/b.tgz", &[1, 2, 3], &lib),
        ]);
        let root = dir();
        let store = Store::new(&root);
        let pack = pack_of(f);
        fs::create_dir_all(store.dir_of(&pack)).unwrap();
        fs::write(store.dir_of(&pack).join("libonnxruntime.so"), &lib).unwrap();
        let mut done = 0;
        install_file_tuned(
            &quick(),
            &store,
            &pack,
            &pack.files[0],
            &Cancel::new(),
            &mut done,
            1234,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(done, 1234);
        // a file of the wrong content is not taken (and then the origins are tried, and fail here)
        fs::write(
            store.dir_of(&pack).join("libonnxruntime.so"),
            data(80_000).iter().map(|b| b ^ 9).collect::<Vec<_>>(),
        )
        .unwrap();
        let mut done = 0;
        assert!(
            install_file_tuned(
                &quick(),
                &store,
                &pack,
                &pack.files[0],
                &Cancel::new(),
                &mut done,
                1234,
                &mut |_| {}
            )
            .is_err()
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_catalogue_lists_nuget_first_and_github_behind_it() {
        let rt = crate::models::find_pack("onnxruntime").unwrap();
        let f = &rt.files[0];
        let o = f.origins();
        assert_eq!(o.len(), 2);
        assert!(o[0].url.contains("nuget"), "{}", o[0].url);
        assert!(matches!(
            o[0].extract.as_ref().unwrap().kind,
            ArchiveKind::ZipMember { .. }
        ));
        assert!(
            o[1].url
                .contains("github.com/microsoft/onnxruntime/releases")
        );
        assert_eq!(o[1].extract.as_ref().unwrap().kind, ArchiveKind::Tgz);
        // same library either way
        assert_eq!(
            o[0].extract.as_ref().unwrap().sha256,
            o[1].extract.as_ref().unwrap().sha256
        );
        assert_eq!(
            o[0].extract.as_ref().unwrap().size,
            o[1].extract.as_ref().unwrap().size
        );
    }
}
