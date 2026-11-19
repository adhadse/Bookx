//! Example editor plugin (Bookx plugin ABI v2): removes trailing spaces and tabs from every line of
//! the book's `.xhtml`, `.html`, `.htm` and `.css` files. Dependency free; the JSON handling is
//! deliberately minimal (`bookx-plugin`'s docs list the host calls).

use std::alloc::{alloc as rs_alloc, dealloc as rs_dealloc, Layout};

#[link(wasm_import_module = "bookx_host")]
extern "C" {
    fn editor_call(op_ptr: i32, op_len: i32, in_ptr: i32, in_len: i32, out: i32) -> i32;
}

#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    unsafe { rs_alloc(Layout::from_size_align(len.max(1) as usize, 1).unwrap()) as i32 }
}

#[no_mangle]
pub extern "C" fn dealloc(ptr: i32, len: i32) {
    unsafe {
        rs_dealloc(
            ptr as *mut u8,
            Layout::from_size_align(len.max(1) as usize, 1).unwrap(),
        )
    }
}

fn slice<'a>(ptr: i32, len: i32) -> &'a [u8] {
    unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
}

/// Calls the host; returns the reply JSON text.
fn host(op: &str, input: &str) -> String {
    let mut out = [0u32; 2];
    let code = unsafe {
        editor_call(
            op.as_ptr() as i32,
            op.len() as i32,
            input.as_ptr() as i32,
            input.len() as i32,
            out.as_mut_ptr() as i32,
        )
    };
    if out[1] == 0 {
        return format!("{{\"err\":\"host call failed ({code})\"}}");
    }
    String::from_utf8_lossy(slice(out[0] as i32, out[1] as i32)).into_owned()
}

fn quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The JSON string that starts at `from` (just after the opening quote), unescaped.
fn unquote(s: &str, from: usize) -> Option<String> {
    let mut out = String::new();
    let mut it = s[from..].chars();
    while let Some(c) = it.next() {
        match c {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = it.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                o => out.push(o),
            },
            c => out.push(c),
        }
    }
    None
}

fn string_after(json: &str, key: &str) -> Option<String> {
    let at = json.find(key)? + key.len();
    unquote(json, at)
}

fn trim_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (body, cr) = match line.strip_suffix('\r') {
            Some(b) => (b, "\r"),
            None => (line, ""),
        };
        out.push_str(body.trim_end_matches([' ', '\t']));
        out.push_str(cr);
    }
    out
}

fn run() -> Result<String, String> {
    let list = host("list_files", "{}");
    let mut files = Vec::new();
    let mut rest = list.as_str();
    // the list is `{"ok":["a","b"]}`: take every quoted item after the `[`
    if let Some(i) = rest.find('[') {
        rest = &rest[i + 1..];
        while let Some(q) = rest.find('"') {
            let Some(name) = unquote(rest, q + 1) else {
                break;
            };
            rest = &rest[q + 1 + quote(&name).len() - 1..];
            files.push(name);
        }
    } else {
        return Err("cannot list the book's files".into());
    }
    let mut changed = 0;
    for f in files {
        let lower = f.to_ascii_lowercase();
        if !(lower.ends_with(".xhtml")
            || lower.ends_with(".html")
            || lower.ends_with(".htm")
            || lower.ends_with(".css"))
        {
            continue;
        }
        let reply = host("read_file", &format!("{{\"path\":{}}}", quote(&f)));
        let Some(text) = string_after(&reply, "\"text\":\"") else {
            continue;
        };
        let fixed = trim_lines(&text);
        if fixed != text {
            let r = host(
                "write_file",
                &format!("{{\"path\":{},\"text\":{}}}", quote(&f), quote(&fixed)),
            );
            if !r.starts_with("{\"ok\"") {
                return Err(format!("cannot write {f}: {r}"));
            }
            changed += 1;
        }
    }
    let msg = match changed {
        0 => "No trailing whitespace found".to_string(),
        1 => "Trimmed 1 file".to_string(),
        n => format!("Trimmed {n} files"),
    };
    host("show_message", &format!("{{\"text\":{}}}", quote(&msg)));
    Ok(msg)
}

fn reply(s: String) -> i64 {
    let b = s.into_bytes().into_boxed_slice();
    let len = b.len();
    let ptr = Box::leak(b).as_ptr() as i64;
    (ptr << 32) | len as i64
}

#[no_mangle]
pub extern "C" fn bookx_call(op_ptr: i32, op_len: i32, in_ptr: i32, in_len: i32) -> i64 {
    let op = slice(op_ptr, op_len);
    let input = String::from_utf8_lossy(slice(in_ptr, in_len)).into_owned();
    if op != b"run_action" {
        return -2;
    }
    if !input.contains("\"trim\"") {
        return -3;
    }
    match run() {
        Ok(m) => reply(format!("{{\"ok\":{}}}", quote(&m))),
        Err(e) => reply(format!("{{\"err\":{}}}", quote(&e))),
    }
}
