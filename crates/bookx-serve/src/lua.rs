//! A small reader for the Lua data files KOReader writes (`return { ["key"] = value, ... }`).
//!
//! Only data is understood: tables, strings, numbers, booleans and `nil`; no code is ever run.
//! The sidecar of a heavily annotated book is a few hundred kilobytes, so this is a single pass.

/// A Lua value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    /// Entries in file order; keys are text (`["a"]`, `a =`) or the number as text (`[1]`).
    Table(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(t) => t.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// The values of a table in file order (keys ignored).
    pub fn items(&self) -> impl Iterator<Item = &Value> {
        let t: &[(String, Value)] = match self {
            Value::Table(t) => t,
            _ => &[],
        };
        t.iter().map(|(_, v)| v)
    }
}

const MAX_DEPTH: usize = 40;

/// Parses the text of a settings file. Accepts a leading `return`, comments, and a bare table.
pub fn parse(text: &str) -> Option<Value> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    if p.b[p.i..].starts_with(b"return") {
        p.i += 6;
    }
    p.value(0)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        loop {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.b[self.i.min(self.b.len())..].starts_with(b"--") {
                while self.i < self.b.len() && self.b[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                return;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'{' => self.table(depth),
            b'"' | b'\'' => self.string().map(Value::Str),
            c if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => self.number(),
            _ => {
                let w = self.word();
                match w.as_str() {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    "nil" => Some(Value::Nil),
                    _ => None,
                }
            }
        }
    }

    fn word(&mut self) -> String {
        let s = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_alphanumeric() || self.b[self.i] == b'_')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.b[s..self.i]).into_owned()
    }

    fn number(&mut self) -> Option<Value> {
        let s = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_digit()
                || matches!(
                    self.b[self.i],
                    b'.' | b'-' | b'+' | b'e' | b'E' | b'x' | b'X'
                )
                || self.b[self.i].is_ascii_hexdigit())
        {
            self.i += 1;
        }
        let t = std::str::from_utf8(&self.b[s..self.i]).ok()?;
        if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return i64::from_str_radix(h, 16)
                .ok()
                .map(|n| Value::Num(n as f64));
        }
        t.parse().ok().map(Value::Num)
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            if c == quote {
                return Some(String::from_utf8_lossy(&out).into_owned());
            }
            if c != b'\\' {
                out.push(c);
                continue;
            }
            let e = self.peek()?;
            self.i += 1;
            match e {
                b'n' | b'\n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'0'..=b'9' => {
                    let mut n = (e - b'0') as u32;
                    for _ in 0..2 {
                        match self.peek() {
                            Some(d) if d.is_ascii_digit() => {
                                n = n * 10 + (d - b'0') as u32;
                                self.i += 1;
                            }
                            _ => break,
                        }
                    }
                    out.push(n.min(255) as u8);
                }
                other => out.push(other),
            }
        }
    }

    fn table(&mut self, depth: usize) -> Option<Value> {
        self.i += 1; // {
        let mut entries: Vec<(String, Value)> = Vec::new();
        let mut next_index = 1usize;
        loop {
            self.ws();
            match self.peek()? {
                b'}' => {
                    self.i += 1;
                    return Some(Value::Table(entries));
                }
                b',' | b';' => {
                    self.i += 1;
                }
                b'[' => {
                    self.i += 1;
                    self.ws();
                    let key = match self.value(depth + 1)? {
                        Value::Str(s) => s,
                        Value::Num(n) if n.fract() == 0.0 => format!("{}", n as i64),
                        Value::Num(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => return None,
                    };
                    self.ws();
                    if self.peek()? != b']' {
                        return None;
                    }
                    self.i += 1;
                    self.ws();
                    if self.peek()? != b'=' {
                        return None;
                    }
                    self.i += 1;
                    let v = self.value(depth + 1)?;
                    entries.push((key, v));
                }
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    let save = self.i;
                    let name = self.word();
                    self.ws();
                    if self.peek() == Some(b'=') && self.b.get(self.i + 1) != Some(&b'=') {
                        self.i += 1;
                        let v = self.value(depth + 1)?;
                        entries.push((name, v));
                    } else {
                        self.i = save;
                        let v = self.value(depth + 1)?;
                        entries.push((next_index.to_string(), v));
                        next_index += 1;
                    }
                }
                _ => {
                    let v = self.value(depth + 1)?;
                    entries.push((next_index.to_string(), v));
                    next_index += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_koreader_file_is_read_with_continued_strings_and_nesting() {
        let v = parse(
            "-- /mnt/onboard/A.sdr/metadata.epub.lua\nreturn {\n    [\"annotations\"] = {\n        [1] = {\n            [\"text\"] = \"line one\\\nline two \\\"q\\\"\",\n            [\"pageno\"] = 23,\n        },\n    },\n    [\"copt_font_size\"] = 16.5,\n    [\"flag\"] = true,\n    [\"percent_finished\"] = 1e-3,\n    [\"empty\"] = {},\n}\n",
        )
        .unwrap();
        let a = v.get("annotations").unwrap().items().next().unwrap();
        assert_eq!(a.str("text"), Some("line one\nline two \"q\""));
        assert_eq!(a.num("pageno"), Some(23.0));
        assert_eq!(v.num("copt_font_size"), Some(16.5));
        assert_eq!(v.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(v.num("percent_finished"), Some(0.001));
        assert_eq!(v.get("empty").unwrap().items().count(), 0);
    }

    #[test]
    fn positional_and_bare_keys_and_hostile_input() {
        let v = parse("{ 10, 20, name = \"x\", [3] = 5 }").unwrap();
        assert_eq!(v.num("1"), Some(10.0));
        assert_eq!(v.num("2"), Some(20.0));
        assert_eq!(v.str("name"), Some("x"));
        assert_eq!(v.num("3"), Some(5.0));
        assert!(parse("").is_none());
        assert!(parse("return { [\"a\"] = ").is_none());
        assert!(parse("return { [\"a\"] = \"unterminated").is_none());
        let deep = format!("return {}{}", "{".repeat(500), "}".repeat(500));
        assert!(parse(&deep).is_none(), "depth is bounded");
    }

    #[test]
    fn code_is_never_run() {
        assert!(parse("return os.execute('x')").is_none());
        assert!(parse("return { a = os.execute('x') }").is_none());
    }
}
