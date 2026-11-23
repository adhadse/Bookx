//! The *partial MD5* KOReader (and Grimmory, KOSync servers) call a book's "binary" id: the MD5 of
//! up to 1 KiB read at each of the offsets 0, 1 KiB, 4 KiB, 16 KiB, … 1 GiB, stopping at the first
//! offset past the end of the file. Two files have the same id only when those bytes are equal,
//! so the id of a book Bookx rewrote (a re-zipped EPUB, an EPUB made from a PDF) differs from the id
//! of the file it came from: the library keeps the id of the original as [`Book::source_ids`].
//!
//! [`Book::source_ids`]: crate::library::Book::source_ids

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The partial MD5 of any seekable source.
pub fn partial_md5_of<R: Read + Seek>(r: &mut R) -> std::io::Result<String> {
    let mut ctx = md5::Context::new();
    let mut buf = vec![0u8; 1024];
    // i = -1 is offset 0 (KOReader's `1024 << -2` wraps to 0 in 32-bit arithmetic), then 1024 << 2i
    let offsets = std::iter::once(0u64).chain((0..=10).map(|i| 1024u64 << (2 * i)));
    for off in offsets {
        r.seek(SeekFrom::Start(off))?;
        let mut got = 0;
        while got < buf.len() {
            let n = r.read(&mut buf[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        if got == 0 {
            break;
        }
        ctx.consume(&buf[..got]);
    }
    Ok(format!("{:x}", ctx.compute()))
}

/// [`partial_md5_of`] for a file.
pub fn partial_md5(path: &Path) -> std::io::Result<String> {
    partial_md5_of(&mut std::fs::File::open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_short_file_is_hashed_whole() {
        let data = b"hello";
        let want = format!("{:x}", md5::compute(data));
        assert_eq!(partial_md5_of(&mut Cursor::new(data)).unwrap(), want);
    }

    #[test]
    fn only_the_sampled_windows_count() {
        let mut a = vec![7u8; 5000];
        let id = partial_md5_of(&mut Cursor::new(&a)).unwrap();
        a[2000] ^= 1; // between the windows [0,1024) and [1024,2048)? 2000 is inside the second one
        assert_ne!(partial_md5_of(&mut Cursor::new(&a)).unwrap(), id);
        let mut b = vec![7u8; 100_000];
        let id = partial_md5_of(&mut Cursor::new(&b)).unwrap();
        b[3000] ^= 1; // between 2048 and 4096: not sampled
        assert_eq!(partial_md5_of(&mut Cursor::new(&b)).unwrap(), id);
    }
}
