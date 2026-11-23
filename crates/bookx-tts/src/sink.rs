//! Where the audio goes.
//!
//! The default sink pipes raw 16-bit samples into a sound server's command line player
//! (`pacat` for PulseAudio and PipeWire, `pw-cat`, `aplay` for ALSA): no native library to link,
//! so the build needs no sound headers and a Flatpak only needs the PulseAudio socket. The
//! pipeline feeds it at the speed of playback, so stopping is exact: the player is killed and a
//! new one starts with the next sentence.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

pub trait AudioSink: Send {
    /// Plays mono 16-bit samples after what was written before. May block.
    fn write(&mut self, rate: u32, samples: &[i16]) -> Result<()>;
    /// Drops what is buffered and silences the output.
    fn flush(&mut self);
    /// What the output buffers before a sample is heard.
    fn latency(&self) -> Duration {
        Duration::ZERO
    }
}

/// A sink that keeps the samples (tests).
#[derive(Clone, Default)]
pub struct MemorySink {
    pub written: Arc<Mutex<Vec<i16>>>,
    pub flushes: Arc<Mutex<usize>>,
    pub rate: Arc<Mutex<u32>>,
}

impl AudioSink for MemorySink {
    fn write(&mut self, rate: u32, samples: &[i16]) -> Result<()> {
        *self.rate.lock().unwrap() = rate;
        self.written.lock().unwrap().extend_from_slice(samples);
        Ok(())
    }

    fn flush(&mut self) {
        *self.flushes.lock().unwrap() += 1;
    }
}

/// A command line player: program and arguments for a sample rate.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// The players to try, in order, for `rate` Hz mono s16le on stdin.
pub fn player_candidates(rate: u32) -> Vec<PlayerCommand> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        PlayerCommand {
            program: "pacat".into(),
            args: {
                let mut a = s(&[
                    "--playback",
                    "--raw",
                    "--format=s16le",
                    "--channels=1",
                    "--latency-msec=100",
                    "--client-name=Bookx",
                    "--stream-name=Read aloud",
                ]);
                a.push(format!("--rate={rate}"));
                a
            },
        },
        PlayerCommand {
            program: "pw-cat".into(),
            args: {
                let mut a = s(&["--playback", "--raw", "--format", "s16", "--channels", "1"]);
                a.extend(["--rate".into(), rate.to_string(), "-".into()]);
                a
            },
        },
        PlayerCommand {
            program: "aplay".into(),
            args: {
                let mut a = s(&["-q", "-t", "raw", "-f", "S16_LE", "-c", "1"]);
                a.extend(["-r".into(), rate.to_string(), "-".into()]);
                a
            },
        },
    ]
}

fn in_path(program: &str) -> bool {
    crate::command::system_program(program).is_some()
}

/// Pipes samples into a command line player.
pub struct CommandSink {
    rate: u32,
    child: Option<Child>,
    /// `$BOOKX_TTS_PLAYER`: an absolute program path and arguments (no shell) that take the samples
    /// on stdin, `{rate}` replaced in each argument.
    custom: Option<String>,
}

impl CommandSink {
    pub fn new() -> CommandSink {
        CommandSink {
            rate: 0,
            child: None,
            custom: std::env::var("BOOKX_TTS_PLAYER")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }

    /// Is there a player on this machine?
    pub fn available() -> bool {
        std::env::var_os("BOOKX_TTS_PLAYER").is_some()
            || player_candidates(24000).iter().any(|c| in_path(&c.program))
    }

    fn spawn(&self, rate: u32) -> Result<Child> {
        if let Some(cmd) = &self.custom {
            let (program, args) =
                crate::command::parse_player(cmd, &[("{rate}", rate.to_string())])
                    .context("BOOKX_TTS_PLAYER")?;
            return Command::new(program)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("starting the player from BOOKX_TTS_PLAYER");
        }
        for c in player_candidates(rate) {
            let Some(program) = crate::command::system_program(&c.program) else {
                continue;
            };
            if let Ok(child) = Command::new(program)
                .args(&c.args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                return Ok(child);
            }
        }
        Err(anyhow!(
            "no audio player found: install PulseAudio or PipeWire (pacat, pw-cat) or alsa-utils (aplay)"
        ))
    }
}

impl Default for CommandSink {
    fn default() -> Self {
        CommandSink::new()
    }
}

impl AudioSink for CommandSink {
    fn write(&mut self, rate: u32, samples: &[i16]) -> Result<()> {
        if self.child.is_some() && rate != self.rate {
            self.flush();
        }
        if self.child.is_none() {
            self.child = Some(self.spawn(rate)?);
            self.rate = rate;
        }
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let stdin = self
            .child
            .as_mut()
            .and_then(|c| c.stdin.as_mut())
            .ok_or_else(|| anyhow!("the audio player has no input"))?;
        if let Err(e) = stdin.write_all(&bytes).and_then(|_| stdin.flush()) {
            self.flush();
            return Err(e).context("the audio player stopped");
        }
        Ok(())
    }

    fn flush(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn latency(&self) -> Duration {
        Duration::from_millis(if self.custom.is_some() { 0 } else { 120 })
    }
}

impl Drop for CommandSink {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An executable player script (a real file, run directly).
    fn write_player(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("player.sh");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn a_custom_player_is_never_run_through_a_shell() {
        let dir = std::env::temp_dir().join(format!("bookx-sink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("pwned");
        for spec in [
            format!("cat > /dev/null; touch {}", marker.display()),
            format!("sh -c 'touch {}'", marker.display()),
            format!("/bin/cat $(touch {})", marker.display()),
        ] {
            let sink = CommandSink {
                rate: 0,
                child: None,
                custom: Some(spec),
            };
            if let Ok(mut c) = sink.spawn(8000) {
                drop(c.stdin.take());
                let _ = c.wait();
            }
        }
        assert!(!marker.exists(), "a shell ran part of BOOKX_TTS_PLAYER");
        let sink = CommandSink {
            rate: 0,
            child: None,
            custom: Some("cat".into()),
        };
        assert!(sink.spawn(8000).is_err(), "a PATH lookup is not allowed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn players_get_the_rate_and_raw_mono_format() {
        let c = player_candidates(22050);
        assert_eq!(c[0].program, "pacat");
        assert!(c[0].args.contains(&"--rate=22050".to_string()));
        assert!(c[0].args.contains(&"--format=s16le".to_string()));
        assert!(c[1].args.contains(&"22050".to_string()));
        assert!(
            c[1].args.contains(&"--raw".to_string()),
            "pw-cat needs --raw for samples on stdin"
        );
        assert!(c[2].args.contains(&"S16_LE".to_string()));
        assert!(c.iter().all(|p| !p.args.is_empty()));
    }

    #[test]
    fn a_custom_player_receives_the_samples_on_stdin() {
        let dir = std::env::temp_dir().join(format!("bookx-sink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out.raw");
        let script = write_player(&dir, &format!("cat > '{}'", out.display()));
        let mut sink = CommandSink {
            rate: 0,
            child: None,
            custom: Some(format!("{} {{rate}}", script.display())),
        };
        sink.write(8000, &[1, -2, 300]).unwrap();
        sink.write(8000, &[4]).unwrap();
        // closing stdin lets `cat` finish
        let mut child = sink.child.take().unwrap();
        drop(child.stdin.take());
        child.wait().unwrap();
        let raw = std::fs::read(&out).unwrap();
        assert_eq!(raw, [1, 0, 0xfe, 0xff, 0x2c, 1, 4, 0]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_kills_the_player_and_the_next_write_starts_a_new_one() {
        let dir = std::env::temp_dir().join(format!("bookx-sink-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = write_player(&dir, "cat > /dev/null");
        let mut sink = CommandSink {
            rate: 0,
            child: None,
            custom: Some(script.display().to_string()),
        };
        sink.write(8000, &[0; 10]).unwrap();
        let first = sink.child.as_ref().unwrap().id();
        sink.flush();
        assert!(sink.child.is_none());
        sink.write(8000, &[0; 10]).unwrap();
        assert_ne!(sink.child.as_ref().unwrap().id(), first);
        // a different rate needs a new player too
        let second = sink.child.as_ref().unwrap().id();
        sink.write(16000, &[0; 10]).unwrap();
        assert_ne!(sink.child.as_ref().unwrap().id(), second);
    }

    #[test]
    fn memory_sink_keeps_what_was_written() {
        let mut m = MemorySink::default();
        m.write(24000, &[1, 2]).unwrap();
        m.flush();
        assert_eq!(*m.written.lock().unwrap(), vec![1, 2]);
        assert_eq!(*m.flushes.lock().unwrap(), 1);
        assert_eq!(*m.rate.lock().unwrap(), 24000);
    }
}
