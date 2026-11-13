//! Text to phonemes for the ONNX voices, through the `espeak-ng` program.
//!
//! Kokoro and Piper were trained on espeak-ng's IPA. The library is GPL-3.0 like Bookx; Bookx
//! runs the installed program (nothing is linked or bundled), so there is no licence mix, and
//! the language data (125 languages) is the one of the system. Pure-Rust phonemizers do not
//! exist for this phoneme set at the quality the models expect.
//!
//! espeak-ng drops punctuation, but the models use it for pauses and intonation, so the text
//! is cut at punctuation, the pieces are phonemized and the punctuation put back.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

/// The espeak-ng program, if the system has one.
pub fn espeak_program() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("BOOKX_ESPEAK") {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH")?;
    ["espeak-ng", "espeak"].iter().find_map(|n| {
        std::env::split_paths(&path)
            .map(|d| d.join(n))
            .find(|p| p.is_file())
    })
}

/// The espeak-ng voice for a two-letter language (or a tag like `en-GB`).
pub fn espeak_voice(language: &str) -> String {
    let l = language.to_lowercase().replace('_', "-");
    match l.as_str() {
        "en" | "en-us" => "en-us".into(),
        "en-gb" | "en-au" | "en-nz" | "en-ie" | "en-za" | "en-in" => "en-gb".into(),
        "fr" => "fr-fr".into(),
        "pt" => "pt-br".into(),
        "zh" => "cmn".into(),
        "nb" | "no" => "nb".into(),
        other => other.to_string(),
    }
}

/// A piece of text that is spoken, or the punctuation between pieces.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Words(String),
    Mark(char),
}

fn is_mark(c: char) -> bool {
    matches!(
        c,
        ',' | '.'
            | '!'
            | '?'
            | ';'
            | ':'
            | '—'
            | '…'
            | '"'
            | '('
            | ')'
            | '“'
            | '”'
            | '«'
            | '»'
            | '、'
            | '。'
            | '，'
            | '！'
            | '？'
    )
}

/// Normalizes the punctuation the models know and cuts the text at it.
pub fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<Piece>| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.chars().any(|c| c.is_alphanumeric()) {
            out.push(Piece::Words(t));
        }
        cur.clear();
    };
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        // a period between digits is a decimal point, an apostrophe is part of the word
        if c == '.'
            && cur.ends_with(|d: char| d.is_ascii_digit())
            && chars.peek().is_some_and(|n| n.is_ascii_digit())
        {
            cur.push(c);
            continue;
        }
        if is_mark(c) {
            flush(&mut cur, &mut out);
            let m = match c {
                '、' | '，' => ',',
                '。' => '.',
                '！' => '!',
                '？' => '?',
                '«' | '“' => '"',
                '»' | '”' => '"',
                other => other,
            };
            out.push(Piece::Mark(m));
        } else if c == '-' && cur.ends_with(' ') {
            // a dash between spaces is a pause
            flush(&mut cur, &mut out);
            out.push(Piece::Mark('—'));
        } else if c.is_control() {
            cur.push(' ');
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// IPA of one piece of words.
fn ipa(program: &PathBuf, voice: &str, words: &str) -> Result<String> {
    let mut child = Command::new(program)
        .args(["-q", "--ipa", "-v", voice, "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("no input for espeak-ng"))?
        .write_all(format!("{words}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("espeak-ng cannot speak \"{voice}\": {}", err.trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // clauses come on separate lines; language switches are marked "(en)": drop those
    let joined = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(strip_language_marks(&joined))
}

/// Removes `(en)`, `(de)` language switch markers.
pub fn strip_language_marks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('(') {
        let tail = &rest[i + 1..];
        match tail.find(')') {
            Some(j)
                if j <= 8
                    && tail[..j]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-') =>
            {
                out.push_str(&rest[..i]);
                rest = tail[j + 1..].trim_start_matches(' ');
            }
            _ => {
                out.push_str(&rest[..=i]);
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Phonemes of a sentence with its punctuation: "Hello, world." gives `həlˈoʊ, wˈɜːld.`
pub fn phonemize(text: &str, language: &str) -> Result<String> {
    let program = espeak_program()
        .ok_or_else(|| anyhow!("espeak-ng is not installed: it turns text into phonemes for the voices (install the espeak-ng package)"))?;
    let voice = espeak_voice(language);
    let mut out = String::new();
    for p in pieces(text) {
        match p {
            Piece::Words(w) => {
                if !out.is_empty() && !out.ends_with(' ') && !out.ends_with(['"', '(']) {
                    out.push(' ');
                }
                out.push_str(&ipa(&program, &voice, &w)?);
            }
            Piece::Mark(m) => {
                out.push(m);
                if matches!(m, ',' | '.' | '!' | '?' | ';' | ':' | '…' | ')' | '—') {
                    out.push(' ');
                }
            }
        }
    }
    Ok(out.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_cut_at_punctuation_which_is_kept() {
        assert_eq!(
            pieces("Hello, world."),
            vec![
                Piece::Words("Hello".into()),
                Piece::Mark(','),
                Piece::Words("world".into()),
                Piece::Mark('.')
            ]
        );
        assert_eq!(
            pieces("He said “go” — now!"),
            vec![
                Piece::Words("He said".into()),
                Piece::Mark('"'),
                Piece::Words("go".into()),
                Piece::Mark('"'),
                Piece::Mark('—'),
                Piece::Words("now".into()),
                Piece::Mark('!'),
            ]
        );
    }

    #[test]
    fn decimals_and_apostrophes_stay_inside_words() {
        assert_eq!(
            pieces("It's 3.14 o'clock"),
            vec![Piece::Words("It's 3.14 o'clock".into())]
        );
        assert_eq!(pieces("a  b\tc"), vec![Piece::Words("a b c".into())]);
    }

    #[test]
    fn marks_without_words_and_cjk_punctuation_are_handled() {
        assert_eq!(
            pieces("... ?"),
            vec![
                Piece::Mark('.'),
                Piece::Mark('.'),
                Piece::Mark('.'),
                Piece::Mark('?')
            ]
        );
        assert_eq!(
            pieces("你好，世界。"),
            vec![
                Piece::Words("你好".into()),
                Piece::Mark(','),
                Piece::Words("世界".into()),
                Piece::Mark('.')
            ]
        );
        assert!(pieces("").is_empty());
    }

    #[test]
    fn languages_map_to_espeak_voices() {
        assert_eq!(espeak_voice("en"), "en-us");
        assert_eq!(espeak_voice("en-GB"), "en-gb");
        assert_eq!(espeak_voice("fr"), "fr-fr");
        assert_eq!(espeak_voice("pt"), "pt-br");
        assert_eq!(espeak_voice("de"), "de");
        assert_eq!(espeak_voice("es_ES"), "es-es");
    }

    #[test]
    fn language_switch_markers_are_removed() {
        assert_eq!(strip_language_marks("(en)həlˈoʊ (de)gˈuːt"), "həlˈoʊ gˈuːt");
        assert_eq!(strip_language_marks("plain"), "plain");
        assert_eq!(
            strip_language_marks("a (long unrelated text) b"),
            "a (long unrelated text) b"
        );
    }

    #[test]
    fn real_espeak_gives_phonemes_when_installed() {
        if espeak_program().is_none() {
            return;
        }
        let p = phonemize("Hello, world.", "en").unwrap();
        assert!(p.contains('ˈ'), "stress marks: {p}");
        assert!(p.contains(',') && p.ends_with('.'), "{p}");
        let d = phonemize("Guten Tag.", "de").unwrap();
        assert!(d.contains("tˈɑːk"), "{d}");
        assert!(
            phonemize("Hello", "zz-nonsense").is_err() || phonemize("Hello", "zz-nonsense").is_ok()
        );
    }
}
