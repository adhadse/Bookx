//! Hostile DjVu input: the document goes to DjVuLibre (when installed) and what comes back is
//! parsed here. The reader must answer garbage with an error, never a panic or a hang, and a lying
//! page must not allocate what it claims.

use bookx_formats::djvu::{DjvuReader, is_djvu, parse_pnm, read_with};
use bookx_formats::read_any;

fn header(kind: &[u8]) -> Vec<u8> {
    let mut v = b"AT&TFORM\0\0\0\x20".to_vec();
    v.extend(kind);
    v
}

#[test]
fn garbage_with_the_magic_is_an_error() {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for n in [16usize, 17, 100, 5000] {
        let mut v = header(b"DJVM");
        for _ in 0..n {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push((x >> 24) as u8);
        }
        assert!(read_any("a.djvu", &v).is_err());
        assert!(read_any("a.bin", &v).is_err());
    }
    assert!(is_djvu(&header(b"DJVU")));
    assert!(djvu_reader_read(&header(b"DJVU")).is_err());
}

fn djvu_reader_read(b: &[u8]) -> anyhow::Result<bookx_core::archive::Archive> {
    use bookx_core::contracts::FormatReader;
    DjvuReader.read(b)
}

#[test]
fn a_page_that_lies_about_its_size_is_refused_without_allocating() {
    // header claims 4 GiB of pixels, data is a few bytes
    assert!(parse_pnm(b"P6\n65535 65535\n255\nabc").is_err());
    assert!(parse_pnm(b"P5\n4294967295 4294967295\n255\nabc").is_err());
    assert!(parse_pnm(b"P6\n-1 5\n255\nabc").is_err());
    // header longer than the data, a comment that never ends, a thousand digits
    assert!(parse_pnm(b"P6\n#").is_err());
    let mut many = b"P6\n".to_vec();
    many.extend(vec![b'1'; 3000]);
    assert!(parse_pnm(&many).is_err());
}

#[test]
fn a_tool_that_is_installed_still_gets_nothing_but_an_error_for_a_bad_document() {
    let Some(tools) = bookx_formats::djvu::find_tools() else {
        return;
    };
    assert!(read_with(&tools, &header(b"DJVUINFO")).is_err());
    assert!(read_with(&tools, b"short").is_err());
}
