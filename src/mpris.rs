//! Media keys and desktop media controls for read aloud: Bookx shows up as an MPRIS player
//! (`org.mpris.MediaPlayer2.Bookx.instance<pid>`) while the player card is open, so play/pause, next
//! and previous from the keyboard, the GNOME shell or `playerctl` reach the reader. The D-Bus side
//! runs on zbus' own thread; calls arrive as [`Control`] values on a channel the reader polls.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::tts_player::Phase;

/// What the desktop asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Play,
    Pause,
    PlayPause,
    Stop,
    /// Next / previous sentence.
    Next,
    Previous,
}

/// The MPRIS `PlaybackStatus` of a player phase.
pub fn playback_status(p: &Phase) -> &'static str {
    match p {
        Phase::Playing => "Playing",
        Phase::Paused | Phase::Preparing(_) => "Paused",
        _ => "Stopped",
    }
}

/// What the desktop shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub status: &'static str,
    pub title: String,
    pub chapter: String,
    pub can_next: bool,
    pub can_previous: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            status: "Stopped",
            title: String::new(),
            chapter: String::new(),
            can_next: false,
            can_previous: false,
        }
    }
}

/// The `Metadata` dictionary of a snapshot.
pub fn metadata(s: &Snapshot) -> HashMap<String, OwnedValue> {
    let mut m: HashMap<String, OwnedValue> = HashMap::new();
    let mut put = |k: &str, v: Value<'_>| {
        if let Ok(v) = OwnedValue::try_from(v) {
            m.insert(k.to_string(), v);
        }
    };
    put(
        "mpris:trackid",
        Value::ObjectPath(ObjectPath::from_static_str_unchecked(
            "/com/adhadse/Bookx/reading",
        )),
    );
    put("xesam:title", Value::from(s.chapter.clone()));
    put("xesam:album", Value::from(s.title.clone()));
    put("xesam:artist", Value::from(vec![s.title.clone()]));
    m
}

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {}
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "Bookx".into()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "com.adhadse.Bookx".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Player {
    snap: Arc<Mutex<Snapshot>>,
    tx: Mutex<Sender<Control>>,
}

impl Player {
    fn send(&self, c: Control) {
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(c);
        }
    }
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        self.send(Control::Next);
    }
    fn previous(&self) {
        self.send(Control::Previous);
    }
    fn pause(&self) {
        self.send(Control::Pause);
    }
    fn play_pause(&self) {
        self.send(Control::PlayPause);
    }
    fn stop(&self) {
        self.send(Control::Stop);
    }
    fn play(&self) {
        self.send(Control::Play);
    }
    fn seek(&self, _offset: i64) {}
    fn set_position(&self, _track: ObjectPath<'_>, _position: i64) {}
    fn open_uri(&self, _uri: String) {}

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.snap
            .lock()
            .map(|s| s.status)
            .unwrap_or("Stopped")
            .into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        0
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.snap.lock().map(|s| metadata(&s)).unwrap_or_default()
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.snap.lock().map(|s| s.can_next).unwrap_or(false)
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.snap.lock().map(|s| s.can_previous).unwrap_or(false)
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

const PATH: &str = "/org/mpris/MediaPlayer2";

/// A running MPRIS service. Dropping it leaves the bus.
pub struct Mpris {
    conn: zbus::blocking::Connection,
    snap: Arc<Mutex<Snapshot>>,
    pub name: String,
    pub rx: Receiver<Control>,
}

impl Mpris {
    /// Registers on the session bus (or on `address`, for tests). `None` when there is no bus.
    pub fn start(address: Option<&str>) -> Option<Mpris> {
        let name = format!(
            "org.mpris.MediaPlayer2.Bookx.instance{}",
            std::process::id()
        );
        let (tx, rx) = channel();
        let snap = Arc::new(Mutex::new(Snapshot::default()));
        let builder = match address {
            Some(a) => zbus::blocking::connection::Builder::address(a),
            None => zbus::blocking::connection::Builder::session(),
        }
        .ok()?;
        let conn = builder
            .name(name.as_str())
            .ok()?
            .serve_at(PATH, Root)
            .ok()?
            .serve_at(
                PATH,
                Player {
                    snap: snap.clone(),
                    tx: Mutex::new(tx),
                },
            )
            .ok()?
            .build()
            .ok()?;
        Some(Mpris {
            conn,
            snap,
            name,
            rx,
        })
    }

    /// Tells the desktop what is playing (only when it changed).
    pub fn update(&self, s: Snapshot) {
        {
            let Ok(mut cur) = self.snap.lock() else {
                return;
            };
            if *cur == s {
                return;
            }
            *cur = s;
        }
        let Ok(iface) = self.conn.object_server().interface::<_, Player>(PATH) else {
            return;
        };
        let emitter = iface.signal_emitter().clone();
        let this = iface.get();
        zbus::block_on(async {
            let _ = this.playback_status_changed(&emitter).await;
            let _ = this.metadata_changed(&emitter).await;
            let _ = this.can_go_next_changed(&emitter).await;
            let _ = this.can_go_previous_changed(&emitter).await;
        });
    }

    /// Everything the desktop asked for since the last call.
    pub fn controls(&self) -> Vec<Control> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    #[test]
    fn phases_map_to_playback_statuses() {
        assert_eq!(playback_status(&Phase::Playing), "Playing");
        assert_eq!(playback_status(&Phase::Paused), "Paused");
        assert_eq!(playback_status(&Phase::Preparing(3)), "Paused");
        assert_eq!(playback_status(&Phase::Stopped), "Stopped");
        assert_eq!(playback_status(&Phase::NeedsEspeak), "Stopped");
        assert_eq!(playback_status(&Phase::Failed("x".into())), "Stopped");
    }

    #[test]
    fn metadata_names_the_book_and_the_chapter() {
        let m = metadata(&Snapshot {
            title: "Alice".into(),
            chapter: "Down the Rabbit-Hole".into(),
            ..Default::default()
        });
        let title: String = m["xesam:title"].clone().try_into().unwrap();
        let album: String = m["xesam:album"].clone().try_into().unwrap();
        assert_eq!(title, "Down the Rabbit-Hole");
        assert_eq!(album, "Alice");
        assert!(m.contains_key("mpris:trackid"));
    }

    /// A private bus: the test never touches the real session bus (and so can run in parallel).
    fn private_bus() -> Option<(std::process::Child, String)> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let out = child.stdout.take()?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(out), &mut line).ok()?;
        let addr = line.trim().to_string();
        (!addr.is_empty()).then_some((child, addr))
    }

    #[test]
    fn the_desktop_can_play_pause_and_read_the_status() {
        let Some((mut bus, addr)) = private_bus() else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let m = Mpris::start(Some(&addr)).expect("registers on the private bus");
        let name = m.name.clone();
        let client = zbus::blocking::connection::Builder::address(addr.as_str())
            .unwrap()
            .build()
            .unwrap();
        let proxy = zbus::blocking::Proxy::new(
            &client,
            name.as_str(),
            PATH,
            "org.mpris.MediaPlayer2.Player",
        )
        .unwrap();
        proxy.call_method("PlayPause", &()).unwrap();
        proxy.call_method("Next", &()).unwrap();
        proxy.call_method("Previous", &()).unwrap();
        proxy.call_method("Stop", &()).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            m.controls(),
            vec![
                Control::PlayPause,
                Control::Next,
                Control::Previous,
                Control::Stop
            ]
        );
        let status: String = proxy.get_property("PlaybackStatus").unwrap();
        assert_eq!(status, "Stopped");
        m.update(Snapshot {
            status: "Playing",
            title: "Alice".into(),
            chapter: "One".into(),
            can_next: true,
            can_previous: false,
        });
        // the proxy caches properties and learns of changes from PropertiesChanged
        let mut status = String::new();
        for _ in 0..40 {
            status = proxy.get_property("PlaybackStatus").unwrap();
            if status == "Playing" {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(status, "Playing");
        let mut next = false;
        for _ in 0..40 {
            next = proxy.get_property("CanGoNext").unwrap();
            if next {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(next);
        let root =
            zbus::blocking::Proxy::new(&client, name.as_str(), PATH, "org.mpris.MediaPlayer2")
                .unwrap();
        let id: String = root.get_property("Identity").unwrap();
        assert_eq!(id, "Bookx");
        drop(proxy);
        drop(root);
        drop(m);
        let _ = bus.kill();
        let _ = bus.wait();
    }
}
