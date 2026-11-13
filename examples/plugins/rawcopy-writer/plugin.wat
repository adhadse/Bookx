;; Worked example of a Bookx format-writer plugin (ABI v1, see docs/plugin-development.md).
;;
;; The `write` operation receives   {"book_b64":"<base64 of the book as an EPUB zip>"}
;; and answers                      {"ok":"<base64 of the file to write>"}
;; This writer simply hands the EPUB back unchanged, so the output is a copy of the book with the
;; extension `epubcopy`. It shows the whole protocol without a JSON or base64 library: the answer
;; is the input with a new prefix and the original `"}` suffix.
;;
;; Build:   wat2wasm plugin.wat -o plugin.wasm        (or let the Rust tests assemble it)
(module
  ;; 32 pages = 2 MiB; the bump allocator grows it on demand (limit: `memory_pages` in plugin.toml)
  (memory (export "memory") 32)
  (global $heap (mut i32) (i32.const 1024))

  ;; the reply prefix, 7 bytes: {"ok":"
  (data (i32.const 16) "{\"ok\":\"")

  ;; alloc(len) -> ptr, or -1 when memory cannot grow
  (func $alloc (export "alloc") (param $n i32) (result i32)
    (local $p i32) (local $end i32) (local $have i32)
    (local.set $p (global.get $heap))
    (local.set $end (i32.add (local.get $p) (i32.and (i32.add (local.get $n) (i32.const 7)) (i32.const -8))))
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then
        (if (i32.eq
              (memory.grow (i32.shr_u (i32.add (i32.sub (local.get $end) (local.get $have)) (i32.const 65535)) (i32.const 16)))
              (i32.const -1))
          (then (return (i32.const -1))))))
    (global.set $heap (local.get $end))
    (local.get $p))

  ;; dealloc(ptr, len): a bump allocator frees nothing; every call runs in a fresh instance anyway
  (func (export "dealloc") (param i32 i32))

  ;; bookx_call(op_ptr, op_len, in_ptr, in_len) -> (reply_ptr << 32) | reply_len, or a negative code
  (func (export "bookx_call") (param $op i32) (param $ol i32) (param $ip i32) (param $il i32) (result i64)
    (local $out i32)
    ;; only the operation `write` (5 bytes, starts with 'w') is supported: -2
    (if (i32.or
          (i32.ne (local.get $ol) (i32.const 5))
          (i32.ne (i32.load8_u (local.get $op)) (i32.const 119)))
      (then (return (i64.const -2))))
    ;; {"book_b64":"" + "}  are 15 bytes: anything shorter is not a valid input: -3
    (if (i32.lt_u (local.get $il) (i32.const 15)) (then (return (i64.const -3))))
    ;; reply = prefix (7) + payload (il - 15) + `"}` (2) = il - 6 bytes
    (local.set $out (call $alloc (local.get $il)))
    (if (i32.lt_s (local.get $out) (i32.const 0)) (then (return (i64.const -4))))
    (memory.copy (local.get $out) (i32.const 16) (i32.const 7))
    (memory.copy (i32.add (local.get $out) (i32.const 7))
                 (i32.add (local.get $ip) (i32.const 13))
                 (i32.sub (local.get $il) (i32.const 15)))
    (memory.copy (i32.add (local.get $out) (i32.sub (local.get $il) (i32.const 8)))
                 (i32.add (local.get $ip) (i32.sub (local.get $il) (i32.const 2)))
                 (i32.const 2))
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
      (i64.extend_i32_u (i32.sub (local.get $il) (i32.const 6))))))
