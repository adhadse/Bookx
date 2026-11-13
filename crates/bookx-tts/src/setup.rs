//! Getting the first voice ready: what to tell the user while the default voice downloads in the
//! background, why a download failed, and how to install `espeak-ng`. No GTK.

/// Why a download did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The server could not be reached or the connection dropped; the part is kept.
    Offline,
    Cancelled,
    /// The checksum or size did not match; the file was deleted.
    Damaged,
    Other(String),
}

/// Sorts the error text of [`crate::download`] into a [`Failure`].
pub fn classify(message: &str) -> Failure {
    let m = message.to_lowercase();
    if m == "cancelled" || m.ends_with(": cancelled") {
        Failure::Cancelled
    } else if m.contains("cannot reach") || m.contains("reading http") || m.contains("timed out") {
        Failure::Offline
    } else if m.contains("checksum") || m.contains("wrong size") {
        Failure::Damaged
    } else {
        Failure::Other(message.to_string())
    }
}

/// What to show for a failure (plain English; the UI translates the pieces it knows).
pub fn failure_message(f: &Failure) -> String {
    match f {
        Failure::Offline => "You seem to be offline. Connect to the internet and try again; the part already downloaded is kept. Meanwhile you can use a voice server (Preferences → Text to Speech).".into(),
        Failure::Cancelled => "The download was cancelled; the part is kept for the next try".into(),
        Failure::Damaged => "The downloaded file was damaged and was deleted. Try again".into(),
        Failure::Other(m) => format!("The download failed: {m}"),
    }
}

/// What to show for an error of the speech engine: plain words and what to do, never the raw engine
/// text. The raw text belongs in a log, not on the card.
pub fn human_error(raw: &str) -> String {
    let m = raw.to_lowercase();
    if m.contains("only one environment") || m.contains("busy with another book") {
        "The voice was still busy with the other book. Press play to try again.".into()
    } else if m.contains("cannot load onnx runtime") || m.contains("cannot start onnx runtime") {
        "The speech runtime could not be loaded. Download it again in Preferences → Text to Speech, then press play.".into()
    } else if m.contains("cannot load the model") || m.contains("starting the voice engine") {
        "The voice could not be loaded. Remove it and download it again in Preferences → Text to Speech, or press play to try again.".into()
    } else if m.contains("cannot reach") || m.contains("connection") || m.contains("timed out") {
        "The voice server did not answer. Check that it is running, then press play to try again."
            .into()
    } else if m.contains("no audio player") || m.contains("pacat") {
        "No audio player was found. Install PulseAudio or PipeWire tools (pacat), then press play."
            .into()
    } else {
        "Reading aloud stopped because of a problem with the voice. Press play to try again.".into()
    }
}

/// 0..=100 of a download.
pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        0
    } else {
        ((done.min(total) as f64 / total as f64) * 100.0).floor() as u32
    }
}

/// The line of the player while the voice downloads.
pub fn preparing_text(done: u64, total: u64) -> String {
    format!("Preparing voice {}%", percent(done, total))
}

/// Download speed over a sliding window of recent samples, so the figure on screen follows what the
/// network does now and not the average since the start.
#[derive(Debug, Clone, Default)]
pub struct Throughput {
    samples: std::collections::VecDeque<(u64, u64)>,
}

/// How far back the speed looks.
const WINDOW_MS: u64 = 5_000;

impl Throughput {
    /// Records that `done` bytes were there at `ms` (any steady clock, in milliseconds).
    pub fn push(&mut self, ms: u64, done: u64) {
        if let Some(&(t, d)) = self.samples.back() {
            if ms < t || done < d {
                // a restart of the download (or a new file): begin again
                self.samples.clear();
            }
        }
        self.samples.push_back((ms, done));
        while self.samples.len() > 2 && ms.saturating_sub(self.samples[1].0) >= WINDOW_MS {
            self.samples.pop_front();
        }
    }

    /// Bytes per second over the window; 0 until two samples are apart in time.
    pub fn bytes_per_sec(&self) -> f64 {
        let (Some(&(t0, d0)), Some(&(t1, d1))) = (self.samples.front(), self.samples.back()) else {
            return 0.0;
        };
        if t1 <= t0 {
            return 0.0;
        }
        (d1 - d0) as f64 * 1000.0 / (t1 - t0) as f64
    }

    /// Seconds left for `remaining` bytes at the current speed; `None` while the speed is unknown.
    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        let v = self.bytes_per_sec();
        (v >= 1.0).then(|| (remaining as f64 / v).ceil() as u64)
    }
}

/// `3.6 MB/s`, `850 KB/s`.
pub fn format_speed(bytes_per_sec: f64) -> String {
    let v = if bytes_per_sec.is_finite() {
        bytes_per_sec.max(0.0)
    } else {
        0.0
    };
    if v >= 1e6 {
        format!("{:.1} MB/s", v / 1e6)
    } else {
        format!("{:.0} KB/s", (v / 1e3).max(0.0))
    }
}

/// `25 s`, `1 min 20 s`, `2 h 5 min`.
pub fn format_eta(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

/// The line of a download: `42 MB of 130 MB · 3.6 MB/s · 25 s left`.
pub fn progress_line(done: u64, total: u64, t: &Throughput) -> String {
    let mb = |b: u64| format!("{:.0} MB", b as f64 / 1e6);
    let mut line = format!("{} of {}", mb(done), mb(total));
    if t.bytes_per_sec() >= 1.0 {
        line += &format!(" · {}", format_speed(t.bytes_per_sec()));
        if let Some(eta) = t.eta_secs(total.saturating_sub(done)) {
            line += &format!(" · {} left", format_eta(eta));
        }
    }
    line
}

/// The command that installs `espeak-ng` on the distribution described by the text of
/// `/etc/os-release`; `None` when it is not one we know (the UI then names the package only).
pub fn espeak_install_command(os_release: &str) -> Option<String> {
    let mut ids: Vec<String> = Vec::new();
    for line in os_release.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            ids.insert(0, v.trim().trim_matches('"').to_lowercase());
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            ids.extend(
                v.trim()
                    .trim_matches('"')
                    .split_whitespace()
                    .map(str::to_lowercase),
            );
        }
    }
    ids.iter().find_map(|id| {
        Some(
            match id.as_str() {
                "fedora" | "rhel" | "centos" => "sudo dnf install espeak-ng",
                "debian" | "ubuntu" | "linuxmint" | "pop" => "sudo apt install espeak-ng",
                "arch" | "manjaro" | "endeavouros" => "sudo pacman -S espeak-ng",
                "opensuse" | "opensuse-tumbleweed" | "opensuse-leap" | "suse" => {
                    "sudo zypper install espeak-ng"
                }
                "alpine" => "sudo apk add espeak-ng",
                "void" => "sudo xbps-install espeak-ng",
                "nixos" => "nix-shell -p espeak-ng",
                _ => return None,
            }
            .to_string(),
        )
    })
}

/// True inside a Flatpak sandbox, where `espeak-ng` is part of the app.
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_sorted_for_the_message() {
        assert_eq!(classify("cancelled"), Failure::Cancelled);
        assert_eq!(
            classify("cannot reach https://x/y: io: Name or service not known"),
            Failure::Offline
        );
        assert_eq!(classify("reading https://x: broken pipe"), Failure::Offline);
        assert_eq!(
            classify("the checksum of u does not match (got a, expected b)"),
            Failure::Damaged
        );
        assert_eq!(classify("disk full"), Failure::Other("disk full".into()));
        assert!(failure_message(&Failure::Offline).contains("offline"));
        assert!(failure_message(&Failure::Offline).contains("voice server"));
        assert!(failure_message(&Failure::Other("x".into())).contains("x"));
    }

    #[test]
    fn progress_is_shown_in_whole_percent() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(42, 100), 42);
        assert_eq!(percent(999, 1000), 99);
        assert_eq!(percent(2000, 1000), 100);
        assert_eq!(preparing_text(21, 50), "Preparing voice 42%");
    }

    #[test]
    fn the_install_command_follows_the_distribution() {
        let fedora = "NAME=Fedora\nID=fedora\nVERSION_ID=44\n";
        assert_eq!(
            espeak_install_command(fedora).unwrap(),
            "sudo dnf install espeak-ng"
        );
        let mint = "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(
            espeak_install_command(mint).unwrap(),
            "sudo apt install espeak-ng"
        );
        let derived = "ID=cachyos\nID_LIKE=arch\n";
        assert_eq!(
            espeak_install_command(derived).unwrap(),
            "sudo pacman -S espeak-ng"
        );
        assert!(espeak_install_command("ID=plan9\n").is_none());
        assert!(espeak_install_command("").is_none());
    }

    #[test]
    fn throughput_follows_the_recent_speed_and_gives_an_eta() {
        let mut t = Throughput::default();
        assert_eq!(t.bytes_per_sec(), 0.0);
        t.push(0, 0);
        assert_eq!(t.eta_secs(100), None, "one sample tells nothing");
        t.push(1000, 2_000_000);
        assert_eq!(t.bytes_per_sec(), 2_000_000.0);
        assert_eq!(t.eta_secs(10_000_000), Some(5));
        // the network slows down: after the window the old fast part is forgotten
        for s in 2..=12 {
            t.push(s * 1000, 2_000_000 + (s - 1) * 100_000);
        }
        let v = t.bytes_per_sec();
        assert!((v - 100_000.0).abs() < 20_000.0, "{v}");
        // a restart begins again
        t.push(13_000, 0);
        assert_eq!(t.bytes_per_sec(), 0.0);
        t.push(14_000, 500_000);
        assert_eq!(t.bytes_per_sec(), 500_000.0);
    }

    #[test]
    fn speed_and_time_read_well() {
        assert_eq!(format_speed(3_650_000.0), "3.6 MB/s");
        assert_eq!(format_speed(850_000.0), "850 KB/s");
        assert_eq!(format_speed(f64::NAN), "0 KB/s");
        assert_eq!(format_eta(25), "25 s");
        assert_eq!(format_eta(80), "1 min 20 s");
        assert_eq!(format_eta(7500), "2 h 5 min");
        let mut t = Throughput::default();
        t.push(0, 0);
        assert_eq!(progress_line(0, 130_000_000, &t), "0 MB of 130 MB");
        t.push(1000, 4_000_000);
        assert_eq!(
            progress_line(4_000_000, 130_000_000, &t),
            "4 MB of 130 MB · 4.0 MB/s · 32 s left"
        );
    }

    #[test]
    fn engine_errors_become_plain_sentences_with_what_to_do() {
        for raw in [
            "starting the voice engine: only one environment is allowed per process",
            "cannot load ONNX Runtime from /x/libonnxruntime.so: no such file",
            "cannot load the model /x/model.onnx: bad",
            "cannot reach http://127.0.0.1:1: refused",
            "something nobody has seen before: 0x7f",
        ] {
            let h = human_error(raw);
            assert!(h.ends_with('.'), "{h}");
            assert!(
                h.to_lowercase().contains("play") || h.contains("Preferences"),
                "{h}"
            );
            assert!(
                !h.contains("0x7f") && !h.contains("onnx") && !h.contains("environment"),
                "raw text leaked: {h}"
            );
        }
        assert!(human_error("only one environment is allowed per process").contains("busy"));
    }
}
