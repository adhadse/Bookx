//! Running the user's own sound player without a shell.
//!
//! `BOOKX_TTS_PLAYER` used to be handed to `sh -c`, which made every `$(...)`, backtick, `;` and
//! redirect in the variable run. It is now split into words with quoting only (no expansion), the
//! program has to be an absolute path, and `{rate}` is substituted per word after splitting, so a
//! value can never add words or shell syntax. The built-in players are looked up in fixed system
//! directories, never through `PATH`.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

/// Where the built-in players (`pacat`, `pw-cat`, `aplay`) may live.
pub const SYSTEM_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin", "/app/bin"];

/// Splits `spec` into words: whitespace separates, `'...'` is literal, `"..."` allows `\"` and `\\`,
/// a backslash outside quotes escapes the next character. Nothing is expanded.
pub fn split_words(spec: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut chars = spec.chars();
    while let Some(c) = chars.next() {
        match c {
            '\0' => return Err(anyhow!("the player command contains a NUL")),
            '\'' => {
                have = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err(anyhow!("unterminated quote in the player command")),
                    }
                }
            }
            '"' => {
                have = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            Some(o) => {
                                cur.push('\\');
                                cur.push(o);
                            }
                            None => {
                                return Err(anyhow!("unterminated quote in the player command"));
                            }
                        },
                        Some(c) => cur.push(c),
                        None => return Err(anyhow!("unterminated quote in the player command")),
                    }
                }
            }
            '\\' => {
                have = true;
                match chars.next() {
                    Some(e) => cur.push(e),
                    None => cur.push('\\'),
                }
            }
            c if c.is_whitespace() => {
                if have {
                    words.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            c => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        words.push(cur);
    }
    Ok(words)
}

/// The program and arguments for a user-written player command. `subs` are `("{rate}", "24000")`
/// pairs applied to each argument after splitting.
pub fn parse_player(spec: &str, subs: &[(&str, String)]) -> Result<(PathBuf, Vec<String>)> {
    let mut words = split_words(spec)?.into_iter();
    let program = words
        .next()
        .ok_or_else(|| anyhow!("the player command is empty"))?;
    let p = Path::new(&program);
    if !p.is_absolute() {
        return Err(anyhow!(
            "the player must be an absolute program path such as /usr/bin/paplay: it is run directly, not through a shell or PATH"
        ));
    }
    if p.components().any(|c| c.as_os_str() == "..") {
        return Err(anyhow!("the player path may not contain .."));
    }
    let args = words
        .map(|mut w| {
            for (k, v) in subs {
                w = w.replace(k, v);
            }
            w
        })
        .collect();
    Ok((p.to_path_buf(), args))
}

/// A built-in player by name, searched only in [`SYSTEM_DIRS`].
pub fn system_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') || name.is_empty() {
        return None;
    }
    SYSTEM_DIRS
        .iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_with_quotes_and_no_expansion() {
        assert_eq!(
            split_words(r#"/usr/bin/x -a 'b c' "d \" e" f\ g"#).unwrap(),
            ["/usr/bin/x", "-a", "b c", "d \" e", "f g"]
        );
        assert_eq!(
            split_words("/x $(touch /tmp/pwn) `id` ; rm -rf ~ > /etc/x").unwrap(),
            [
                "/x",
                "$(touch",
                "/tmp/pwn)",
                "`id`",
                ";",
                "rm",
                "-rf",
                "~",
                ">",
                "/etc/x"
            ]
        );
        assert!(split_words("/x 'open").is_err());
        assert!(split_words("/x \"open").is_err());
        assert!(split_words("/x\0y").is_err());
        assert_eq!(split_words("''").unwrap(), [""]);
    }

    #[test]
    fn the_program_must_be_an_absolute_path() {
        assert!(parse_player("pacat --raw", &[]).is_err());
        assert!(parse_player("./pacat", &[]).is_err());
        assert!(parse_player("sh -c id", &[]).is_err());
        assert!(parse_player("   ", &[]).is_err());
        assert!(parse_player("/usr/../bin/sh", &[]).is_err());
        let (p, a) =
            parse_player("/usr/bin/pacat --rate={rate}", &[("{rate}", "8000".into())]).unwrap();
        assert_eq!(p, Path::new("/usr/bin/pacat"));
        assert_eq!(a, ["--rate=8000"]);
    }

    #[test]
    fn substitution_cannot_add_words() {
        let (_, a) = parse_player("/x {rate}", &[("{rate}", "1 2; id".into())]).unwrap();
        assert_eq!(a, ["1 2; id"]);
    }

    #[test]
    fn system_players_are_not_looked_up_through_path() {
        assert!(system_program("../bin/sh").is_none());
        assert!(system_program("/bin/sh").is_none());
        assert!(system_program("").is_none());
        // a program only on PATH (here: a name that exists nowhere in the system dirs) is not found
        assert!(system_program("definitely-not-a-bookx-player").is_none());
    }
}
