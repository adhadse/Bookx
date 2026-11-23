//! Running a helper program on an untrusted file: a private temporary copy, a program found in
//! fixed system directories only (no `PATH`, no shell), an argument vector, a limit on the
//! output and a time limit after which the program is killed. Used by `rar` and `djvu`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Directories searched for the helper programs, in order.
pub(crate) const DIRS: &[&str] = &[
    "/usr/bin",
    "/usr/local/bin",
    "/bin",
    "/app/bin",
    "/opt/homebrew/bin",
    "/snap/bin",
];

pub(crate) fn find_in(dirs: &[&str], name: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

pub(crate) fn banner(program: &Path, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout[..o.stdout.len().min(2048)]).into_owned())
        .unwrap_or_default()
}

/// A private copy of the archive for the helper; removed when dropped.
pub(crate) struct TempFile(pub(crate) PathBuf);

static COUNTER: AtomicU64 = AtomicU64::new(0);

impl TempFile {
    pub(crate) fn create(bytes: &[u8], ext: &str) -> Result<TempFile> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "bookx-helper-{}-{}-{nanos}.{ext}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .context("cannot write a temporary file")?;
        f.write_all(bytes)?;
        Ok(TempFile(path))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs the helper and returns at most `limit` bytes of its output; kills it after the time limit.
pub(crate) fn run(
    tool: &Path,
    args: &[&std::ffi::OsStr],
    limit: u64,
    time: Duration,
) -> Result<Vec<u8>> {
    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("cannot start the RAR helper")?;
    let mut stdout = child.stdout.take().context("no output")?;
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    let start = Instant::now();
    let watchdog = std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            if start.elapsed() > time {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        child
    });
    let mut buf = Vec::new();
    let read = (&mut stdout).take(limit + 1).read_to_end(&mut buf);
    done.store(true, Ordering::Relaxed);
    drop(stdout);
    let mut child = watchdog.join().map_err(|_| anyhow::anyhow!("watchdog"))?;
    if buf.len() as u64 > limit {
        let _ = child.kill();
        let _ = child.wait();
        bail!("the RAR archive unpacks to more than Bookx accepts");
    }
    let status = child.wait()?;
    read?;
    if start.elapsed() > time {
        bail!("unpacking the RAR archive took too long");
    }
    if !status.success() {
        bail!(
            "the RAR helper could not unpack the archive (damaged, or protected with a password)"
        );
    }
    Ok(buf)
}
