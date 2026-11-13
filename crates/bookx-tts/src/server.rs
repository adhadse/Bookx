//! Speech from a server you run yourself: anything that speaks the OpenAI audio API
//! (`POST /v1/audio/speech`: Kokoro-FastAPI, openedai-speech, LocalAI, Speaches, Chatterbox and
//! Orpheus wrappers, ...), the XTTS API server (`POST /tts_to_audio/`, XTTS v2) or the Fish
//! Speech API server (`POST /v1/tts`). The server's licence is the user's business: Bookx
//! only sends text and plays the audio that comes back.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::prefs::ServerPrefs;
use crate::session::Voice;
use crate::synth::{parse_wav, pcm_from_s16le, Pcm, SynthRequest, Synthesizer};

const MAX_ANSWER: u64 = 256 * 1024 * 1024;

pub struct ServerSynth {
    cfg: ServerPrefs,
    agent: ureq::Agent,
    api_key: Option<String>,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent("Bookx/0.2 (read aloud)")
        .build()
        .into()
}

/// The XTTS language code of a two-letter code.
pub fn xtts_language(lang: &str) -> &str {
    match lang {
        "zh" => "zh-cn",
        "" => "en",
        other => other,
    }
}

impl ServerSynth {
    pub fn new(cfg: &ServerPrefs) -> ServerSynth {
        ServerSynth {
            cfg: cfg.clone(),
            agent: agent(Duration::from_secs(180)),
            api_key: Some(cfg.api_key.trim().to_string())
                .filter(|k| !k.is_empty())
                .or_else(|| {
                    std::env::var("BOOKX_TTS_API_KEY")
                        .ok()
                        .filter(|k| !k.is_empty())
                }),
        }
    }

    /// `base` + `path`, without doubling a `/v1` the user typed into the address.
    pub fn endpoint(base: &str, path: &str) -> String {
        let base = base.trim().trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if let (Some(b), Some(p)) = (base.strip_suffix("/v1"), path.strip_prefix("v1/")) {
            return format!("{b}/v1/{p}");
        }
        format!("{base}/{path}")
    }

    fn url(&self, path: &str) -> String {
        Self::endpoint(&self.cfg.url, path)
    }

    fn get(&self, path: &str) -> Result<Vec<u8>> {
        let mut req = self.agent.get(&self.url(path));
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let resp = req.call().map_err(|e| connection_error(&self.cfg.url, e))?;
        read_answer(resp)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Vec<u8>> {
        let mut req = self.agent.post(&self.url(path));
        if let Some(k) = &self.api_key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let resp = req
            .header("Content-Type", "application/json")
            .send(body.to_string().as_bytes())
            .map_err(|e| connection_error(&self.cfg.url, e))?;
        read_answer(resp)
    }

    /// The request for one piece of text, as (path, body).
    pub fn request(cfg: &ServerPrefs, req: &SynthRequest) -> (&'static str, Value) {
        let voice = if req.voice.is_empty() {
            cfg.voice.as_str()
        } else {
            req.voice
        };
        match cfg.kind.as_str() {
            "xtts" => (
                "tts_to_audio/",
                json!({
                    "text": req.text,
                    "speaker_wav": voice,
                    "language": xtts_language(req.language),
                    "speed": req.speed,
                }),
            ),
            "fish" => (
                "v1/tts",
                json!({
                    "text": req.text,
                    "reference_id": if voice.is_empty() { Value::Null } else { json!(voice) },
                    "format": "wav",
                    "streaming": false,
                    "prosody": { "speed": req.speed },
                }),
            ),
            _ => (
                "v1/audio/speech",
                json!({
                    "model": cfg.model,
                    "input": req.text,
                    "voice": if voice.is_empty() { "alloy" } else { voice },
                    "response_format": "wav",
                    "speed": req.speed,
                }),
            ),
        }
    }

    /// What the server says it can do: a short test sentence is spoken and measured.
    pub fn test(cfg: &ServerPrefs) -> Result<String> {
        let mut s = ServerSynth::new(cfg);
        let voices = s.voices().unwrap_or_default();
        let t = Instant::now();
        let pcm = s.synthesize(&SynthRequest {
            text: "Hello. This is a test of the voice server.",
            voice: "",
            language: "en",
            speed: 1.0,
            cancel: &crate::synth::Cancel::new(),
        })?;
        Ok(format!(
            "Connected: {} voices listed, {:.1} s of audio at {} Hz in {:.1} s",
            voices.len(),
            pcm.duration_secs(),
            pcm.rate,
            t.elapsed().as_secs_f64()
        ))
    }
}

fn connection_error(url: &str, e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Timeout(_) => anyhow!("the voice server at {url} took too long to answer"),
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) | ureq::Error::HostNotFound => {
            anyhow!("cannot reach the voice server at {url}: is it running?")
        }
        other => anyhow!("the voice server at {url}: {other}"),
    }
}

fn read_answer(mut resp: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
    let status = resp.status().as_u16();
    let mut body = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(MAX_ANSWER)
        .read_to_end(&mut body)
        .context("reading the answer of the voice server")?;
    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(&body);
        let text: String = text.chars().take(200).collect();
        bail!("the voice server answered {status}: {text}");
    }
    Ok(body)
}

/// Audio from a server: WAV, or raw 16-bit samples at `raw_rate`.
pub fn decode_audio(bytes: &[u8], raw_rate: u32) -> Result<Pcm> {
    if bytes.starts_with(b"RIFF") {
        return parse_wav(bytes);
    }
    if bytes.starts_with(b"ID3") || bytes.starts_with(&[0xff, 0xfb]) {
        bail!("the voice server sent MP3; Bookx needs it to send WAV (response_format \"wav\")");
    }
    if bytes.starts_with(b"OggS") || bytes.starts_with(b"fLaC") {
        bail!("the voice server sent compressed audio; Bookx needs WAV");
    }
    if bytes.first() == Some(&b'{') || bytes.first() == Some(&b'<') {
        let t: String = String::from_utf8_lossy(bytes).chars().take(160).collect();
        bail!("the voice server did not send audio: {t}");
    }
    if bytes.len() < 2 {
        bail!("the voice server sent no audio");
    }
    Ok(pcm_from_s16le(bytes, raw_rate))
}

impl Synthesizer for ServerSynth {
    fn id(&self) -> String {
        format!(
            "server-{}-{}-{}",
            self.cfg.kind, self.cfg.url, self.cfg.model
        )
    }

    fn voices(&mut self) -> Result<Vec<Voice>> {
        let list = match self.cfg.kind.as_str() {
            "xtts" => self.get("speakers_list"),
            "fish" => return Ok(Vec::new()),
            _ => self.get("v1/audio/voices"),
        };
        let Ok(bytes) = list else {
            // servers without a voice list are fine: the voice is typed in the settings
            return Ok(Vec::new());
        };
        Ok(parse_voice_list(&bytes))
    }

    fn synthesize(&mut self, req: &SynthRequest) -> Result<Pcm> {
        if req.cancel.is_cancelled() {
            bail!("cancelled");
        }
        let (path, body) = Self::request(&self.cfg, req);
        let bytes = self.post(path, &body)?;
        decode_audio(&bytes, 24000)
    }
}

/// `["a","b"]`, `{"voices":["a"]}`, `{"voices":[{"id":"a"}]}`, `{"voices":[{"name":"a"}]}`.
pub fn parse_voice_list(bytes: &[u8]) -> Vec<Voice> {
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let list = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .get("voices")
            .or_else(|| o.get("speakers"))
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut out: Vec<Voice> = list
        .iter()
        .filter_map(|x| {
            let name = match x {
                Value::String(s) => s.clone(),
                Value::Object(o) => ["id", "name", "voice_id"]
                    .iter()
                    .find_map(|k| o.get(*k).and_then(|v| v.as_str()))?
                    .to_string(),
                _ => return None,
            };
            let language = x
                .get("language")
                .or_else(|| x.get("lang"))
                .and_then(|l| l.as_str())
                .unwrap_or("")
                .to_string();
            (!name.is_empty()).then_some(Voice { name, language })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::Cancel;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(String, String, Vec<(String, String)>)>>>;

    /// A server that answers `routes` (path -> (status, content type, body)) and records the
    /// requests. Serves connections until the process ends.
    fn serve(routes: Vec<(&'static str, u16, &'static str, Vec<u8>)>) -> (String, Seen) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", l.local_addr().unwrap());
        let seen: Seen = Arc::default();
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let routes = routes.clone();
                let seen = s2.clone();
                std::thread::spawn(move || handle(stream, &routes, &seen));
            }
        });
        (url, seen)
    }

    fn handle(
        mut stream: TcpStream,
        routes: &[(&'static str, u16, &'static str, Vec<u8>)],
        seen: &Seen,
    ) {
        let mut r = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let mut parts = line.split_whitespace();
        let (method, path) = (
            parts.next().unwrap_or("").to_string(),
            parts.next().unwrap_or("").to_string(),
        );
        let mut headers = Vec::new();
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap();
            if h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                headers.push((k.to_lowercase(), v.trim().to_string()));
            }
        }
        let mut body = vec![0u8; len];
        r.read_exact(&mut body).unwrap();
        seen.lock().unwrap().push((
            format!("{method} {path}"),
            String::from_utf8_lossy(&body).to_string(),
            headers,
        ));
        let (status, ctype, out) = routes
            .iter()
            .find(|x| x.0 == path)
            .map(|x| (x.1, x.2, x.3.clone()))
            .unwrap_or((404, "text/plain", b"no such route".to_vec()));
        let _ = write!(
            stream,
            "HTTP/1.1 {status} X\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            out.len()
        );
        let _ = stream.write_all(&out);
    }

    fn tone(n: usize) -> Vec<u8> {
        Pcm {
            rate: 22050,
            samples: vec![0.3; n],
        }
        .to_wav()
    }

    fn cfg(url: &str, kind: &str) -> ServerPrefs {
        ServerPrefs {
            kind: kind.into(),
            url: url.into(),
            model: "m1".into(),
            voice: "v-default".into(),
            api_key: String::new(),
        }
    }

    fn say(s: &mut ServerSynth, text: &str, voice: &str, lang: &str) -> Result<Pcm> {
        s.synthesize(&SynthRequest {
            text,
            voice,
            language: lang,
            speed: 1.25,
            cancel: &Cancel::new(),
        })
    }

    #[test]
    fn the_openai_request_has_the_fields_servers_expect() {
        let (url, seen) = serve(vec![("/v1/audio/speech", 200, "audio/wav", tone(2205))]);
        let mut s = ServerSynth::new(&cfg(&url, "openai"));
        let pcm = say(&mut s, "Hello there.", "", "en").unwrap();
        assert_eq!(pcm.rate, 22050);
        assert!((pcm.duration_secs() - 0.1).abs() < 1e-6);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "POST /v1/audio/speech");
        let body: Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(body["input"], "Hello there.");
        assert_eq!(body["model"], "m1");
        assert_eq!(
            body["voice"], "v-default",
            "the configured voice when none is picked"
        );
        assert_eq!(body["response_format"], "wav");
        assert_eq!(body["speed"], 1.25);
        let ct = seen[0].2.iter().find(|h| h.0 == "content-type").unwrap();
        assert_eq!(ct.1, "application/json");
    }

    #[test]
    fn the_key_from_the_settings_is_sent_as_bearer_and_is_not_in_debug_output() {
        let (url, seen) = serve(vec![("/v1/audio/speech", 200, "audio/wav", tone(100))]);
        let mut c = cfg(&url, "openai");
        c.api_key = " keyring-key ".into();
        assert!(!format!("{c:?}").contains("keyring-key"));
        let mut s = ServerSynth::new(&c);
        say(&mut s, "Hi.", "", "en").unwrap();
        let seen = seen.lock().unwrap();
        let auth = seen[0].2.iter().find(|h| h.0 == "authorization").unwrap();
        assert_eq!(auth.1, "Bearer keyring-key");
    }

    #[test]
    fn a_picked_voice_beats_the_configured_one_and_a_v1_address_is_not_doubled() {
        let (url, seen) = serve(vec![("/v1/audio/speech", 200, "audio/wav", tone(100))]);
        let mut s = ServerSynth::new(&cfg(&format!("{url}/v1/"), "openai"));
        say(&mut s, "Hi.", "nova", "en").unwrap();
        let b: Value = serde_json::from_str(&seen.lock().unwrap()[0].1).unwrap();
        assert_eq!(b["voice"], "nova");
        assert_eq!(
            ServerSynth::endpoint("http://h:1/v1", "v1/audio/speech"),
            "http://h:1/v1/audio/speech"
        );
        assert_eq!(
            ServerSynth::endpoint("http://h:1/", "/tts_to_audio/"),
            "http://h:1/tts_to_audio/"
        );
    }

    #[test]
    fn the_xtts_server_gets_speaker_and_language() {
        let (url, seen) = serve(vec![
            ("/tts_to_audio/", 200, "audio/wav", tone(441)),
            (
                "/speakers_list",
                200,
                "application/json",
                br#"["Ana","Bob"]"#.to_vec(),
            ),
        ]);
        let mut s = ServerSynth::new(&cfg(&url, "xtts"));
        say(&mut s, "Hallo.", "Ana", "de").unwrap();
        say(&mut s, "你好。", "Ana", "zh").unwrap();
        let v = s.voices().unwrap();
        assert_eq!(
            v.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["Ana", "Bob"]
        );
        let seen = seen.lock().unwrap();
        let b: Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(b["speaker_wav"], "Ana");
        assert_eq!(b["language"], "de");
        let b: Value = serde_json::from_str(&seen[1].1).unwrap();
        assert_eq!(b["language"], "zh-cn");
    }

    #[test]
    fn the_fish_server_gets_a_json_request_for_wav() {
        let (url, seen) = serve(vec![("/v1/tts", 200, "audio/wav", tone(441))]);
        let mut c = cfg(&url, "fish");
        c.voice = String::new();
        let mut s = ServerSynth::new(&c);
        say(&mut s, "Hello.", "", "en").unwrap();
        let b: Value = serde_json::from_str(&seen.lock().unwrap()[0].1).unwrap();
        assert_eq!(b["format"], "wav");
        assert_eq!(b["streaming"], false);
        assert_eq!(b["reference_id"], Value::Null);
        assert_eq!(b["prosody"]["speed"], 1.25);
        assert!(s.voices().unwrap().is_empty());
    }

    #[test]
    fn voice_lists_come_in_several_shapes() {
        let names = |b: &str| -> Vec<String> {
            parse_voice_list(b.as_bytes())
                .into_iter()
                .map(|v| v.name)
                .collect()
        };
        assert_eq!(names(r#"["b","a","a"]"#), ["a", "b"]);
        assert_eq!(names(r#"{"voices":["x","y"]}"#), ["x", "y"]);
        assert_eq!(
            names(r#"{"voices":[{"id":"z"},{"name":"w"},{"nothing":1}]}"#),
            ["w", "z"]
        );
        assert!(names("garbage").is_empty());
        let v = parse_voice_list(br#"{"voices":[{"id":"fr1","language":"fr"}]}"#);
        assert_eq!(v[0].language, "fr");
    }

    #[test]
    fn a_server_without_a_voice_list_is_fine() {
        let (url, _) = serve(vec![]);
        let mut s = ServerSynth::new(&cfg(&url, "openai"));
        assert!(s.voices().unwrap().is_empty());
    }

    #[test]
    fn errors_say_what_is_wrong() {
        let (url, _) = serve(vec![(
            "/v1/audio/speech",
            500,
            "application/json",
            br#"{"error":"model not loaded"}"#.to_vec(),
        )]);
        let mut s = ServerSynth::new(&cfg(&url, "openai"));
        let e = say(&mut s, "Hi.", "", "en").unwrap_err().to_string();
        assert!(e.contains("500") && e.contains("model not loaded"), "{e}");

        // nothing listens there
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let dead = format!("http://{}", l.local_addr().unwrap());
        drop(l);
        let mut s = ServerSynth::new(&cfg(&dead, "openai"));
        let e = say(&mut s, "Hi.", "", "en").unwrap_err().to_string();
        assert!(e.contains("cannot reach the voice server"), "{e}");

        let c = Cancel::new();
        c.cancel();
        let e = s
            .synthesize(&SynthRequest {
                text: "x",
                voice: "",
                language: "",
                speed: 1.0,
                cancel: &c,
            })
            .unwrap_err()
            .to_string();
        assert_eq!(e, "cancelled");
    }

    #[test]
    fn answers_that_are_not_wav_are_explained() {
        assert!(decode_audio(b"ID3\x04rest", 24000)
            .unwrap_err()
            .to_string()
            .contains("MP3"));
        assert!(decode_audio(b"OggS....", 24000)
            .unwrap_err()
            .to_string()
            .contains("compressed"));
        assert!(decode_audio(br#"{"detail":"bad"}"#, 24000)
            .unwrap_err()
            .to_string()
            .contains("bad"));
        assert!(decode_audio(b"", 24000).is_err());
        // raw 16-bit samples (the OpenAI "pcm" format)
        let p = decode_audio(&[0, 0x40, 0, 0xc0], 24000).unwrap();
        assert_eq!((p.rate, p.samples.len()), (24000, 2));
    }

    #[test]
    fn the_connection_test_reports_voices_and_timing() {
        let (url, _) = serve(vec![
            ("/v1/audio/speech", 200, "audio/wav", tone(22050)),
            (
                "/v1/audio/voices",
                200,
                "application/json",
                br#"{"voices":["a","b","c"]}"#.to_vec(),
            ),
        ]);
        let msg = ServerSynth::test(&cfg(&url, "openai")).unwrap();
        assert!(
            msg.starts_with("Connected: 3 voices listed, 1.0 s of audio at 22050 Hz"),
            "{msg}"
        );
        assert!(ServerSynth::test(&cfg("http://127.0.0.1:1", "openai")).is_err());
    }

    #[test]
    fn works_through_the_pipeline_like_the_app_does() {
        use crate::pipeline::PipelineEngine;
        use crate::session::{Session, State, Update};
        use crate::sink::MemorySink;
        let (url, _) = serve(vec![("/v1/audio/speech", 200, "audio/wav", tone(1100))]);
        let sink = MemorySink::default();
        let mut e = PipelineEngine::new(
            "server",
            Box::new(ServerSynth::new(&cfg(&url, "openai"))),
            Box::new(sink.clone()),
            None,
            3,
        );
        let mut s = Session::new();
        s.load(&mut e, "One. Two.");
        s.play_from(&mut e, 0);
        let mut got = Vec::new();
        let t = Instant::now();
        while s.state() != State::Stopped && t.elapsed() < Duration::from_secs(10) {
            got.extend(s.tick(&mut e));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got.last(), Some(&Update::Finished), "{got:?}");
        assert_eq!(sink.written.lock().unwrap().len(), 2200);
        assert_eq!(*sink.rate.lock().unwrap(), 22050);
    }
}
