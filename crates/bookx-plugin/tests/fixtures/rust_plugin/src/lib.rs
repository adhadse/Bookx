//! A real plugin compiled from Rust to wasm32-unknown-unknown, used by `tests/rust_plugin.rs`.
//! Dependency free; the JSON handling is deliberately naive.

use std::alloc::{alloc as rs_alloc, dealloc as rs_dealloc, Layout};

#[link(wasm_import_module = "bookx_host")]
extern "C" {
    fn log(level: i32, ptr: i32, len: i32);
    fn now_ms() -> i64;
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

fn title_of(input: &str) -> Option<&str> {
    let start = input.find("\"title\":\"")? + 9;
    let rest = &input[start..];
    Some(&rest[..rest.find('"')?])
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
    let msg = format!("op {}", String::from_utf8_lossy(op));
    unsafe { log(2, msg.as_ptr() as i32, msg.len() as i32) };
    match op {
        b"lookup" => {
            let title = title_of(&input).unwrap_or("?");
            let clock_ok = unsafe { now_ms() } > 1_600_000_000_000;
            reply(format!(
                "{{\"ok\":[{{\"title\":\"{title} (found)\",\"authors\":[\"Rust Plugin\"],\"rating\":{}}}]}}",
                if clock_ok { 10 } else { 0 }
            ))
        }
        b"panic" => panic!("on purpose"),
        _ => -2,
    }
}

#[no_mangle]
pub extern "C" fn bookx_info() -> i64 {
    reply("{\"name\":\"rust fixture\",\"abi\":1}".to_string())
}
