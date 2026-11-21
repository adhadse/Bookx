//! A small QR code encoder (ISO/IEC 18004) for showing the server address to a phone or e-reader:
//! byte mode only, error correction L, M, Q or H, versions 1 to 40, the eight masks and the
//! standard penalty rules. Pure and `#![forbid(unsafe_code)]`-safe: it takes text and gives a
//! square of dark and light modules, and never touches the network or the disk.
//!
//! The algorithm follows the specification (finder, timing and alignment patterns, Reed-Solomon
//! over GF(256) with the polynomial 0x11D, block interleaving, zig-zag placement, format and
//! version information). The tests compare complete symbols that an independent decoder (zxing-cpp)
//! read back correctly, and check the format and version words against the standard's tables.

/// How much damage a code survives; more correction means a bigger code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecc {
    Low,
    Medium,
    Quartile,
    High,
}

impl Ecc {
    fn ordinal(self) -> usize {
        match self {
            Ecc::Low => 0,
            Ecc::Medium => 1,
            Ecc::Quartile => 2,
            Ecc::High => 3,
        }
    }

    fn format_bits(self) -> u32 {
        match self {
            Ecc::Low => 1,
            Ecc::Medium => 0,
            Ecc::Quartile => 3,
            Ecc::High => 2,
        }
    }
}

// indexed [ecc ordinal][version]; version 0 is unused
const ECC_CODEWORDS_PER_BLOCK: [[u8; 41]; 4] = [
    [
        0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28,
        30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28,
        28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    ],
    [
        0, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30,
        30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        0, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
];

const NUM_ERROR_CORRECTION_BLOCKS: [[u8; 41]; 4] = [
    [
        0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13,
        14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25,
    ],
    [
        0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21,
        23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
    ],
    [
        0, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29,
        34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68,
    ],
    [
        0, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32,
        35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81,
    ],
];

/// A finished symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrCode {
    version: usize,
    size: usize,
    mask: u8,
    modules: Vec<bool>,
}

impl QrCode {
    /// Encodes `text` (as UTF-8 bytes) with the smallest version that fits, at the best error
    /// correction level that does not make it bigger, and the mask with the lowest penalty.
    /// `None` when it does not fit even version 40.
    pub fn encode_text(text: &str, min_ecc: Ecc) -> Option<QrCode> {
        QrCode::encode(text.as_bytes(), min_ecc, None, true)
    }

    /// Like [`encode_text`](Self::encode_text) with a fixed mask (0 to 7) and error correction
    /// level, for tests against other encoders.
    pub fn encode_fixed(text: &str, ecc: Ecc, mask: u8) -> Option<QrCode> {
        QrCode::encode(text.as_bytes(), ecc, Some(mask & 7), false)
    }

    fn encode(data: &[u8], ecc: Ecc, mask: Option<u8>, boost: bool) -> Option<QrCode> {
        let mut version = 0;
        let mut used_bits = 0;
        for v in 1..=40 {
            let count_bits = if v < 10 { 8 } else { 16 };
            let need = 4 + count_bits + data.len() * 8;
            if data.len() >= 1 << count_bits {
                continue;
            }
            if need <= data_codewords(v, ecc) * 8 {
                version = v;
                used_bits = need;
                break;
            }
        }
        if version == 0 {
            return None;
        }
        let mut ecc = ecc;
        if boost {
            for e in [Ecc::Medium, Ecc::Quartile, Ecc::High] {
                if e.ordinal() > ecc.ordinal() && used_bits <= data_codewords(version, e) * 8 {
                    ecc = e;
                }
            }
        }
        let count_bits = if version < 10 { 8 } else { 16 };
        let mut bits = BitBuf::default();
        bits.push(0b0100, 4);
        bits.push(data.len() as u32, count_bits);
        for &b in data {
            bits.push(u32::from(b), 8);
        }
        let capacity = data_codewords(version, ecc) * 8;
        bits.push(0, (capacity - bits.len()).min(4));
        bits.push(0, (8 - bits.len() % 8) % 8);
        let mut pad = 0xEC;
        while bits.len() < capacity {
            bits.push(pad, 8);
            pad ^= 0xEC ^ 0x11;
        }
        let codewords = bits.bytes();
        let all = add_ecc_and_interleave(&codewords, version, ecc);
        Some(QrCode::build(version, ecc, &all, mask))
    }

    fn build(version: usize, ecc: Ecc, all: &[u8], mask: Option<u8>) -> QrCode {
        let size = version * 4 + 17;
        let mut g = Grid {
            size,
            modules: vec![false; size * size],
            function: vec![false; size * size],
        };
        g.draw_function_patterns(version, ecc);
        g.draw_codewords(all);
        let mask = match mask {
            Some(m) => m,
            None => {
                let mut best = (u32::MAX, 0u8);
                for m in 0..8u8 {
                    g.apply_mask(m);
                    g.draw_format(ecc, m);
                    let p = g.penalty();
                    if p < best.0 {
                        best = (p, m);
                    }
                    g.apply_mask(m);
                }
                best.1
            }
        };
        g.apply_mask(mask);
        g.draw_format(ecc, mask);
        QrCode {
            version,
            size,
            mask,
            modules: g.modules,
        }
    }

    /// Modules per side (21 for version 1).
    pub fn size(&self) -> usize {
        self.size
    }

    pub fn version(&self) -> usize {
        self.version
    }

    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// Is the module at column `x`, row `y` dark? Outside the symbol it is light.
    pub fn dark(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    /// One line per row of `#` and `.`, for tests and the dev hook.
    pub fn to_ascii(&self) -> String {
        let mut s = String::new();
        for y in 0..self.size {
            for x in 0..self.size {
                s.push(if self.dark(x, y) { '#' } else { '.' });
            }
            s.push('\n');
        }
        s
    }
}

fn raw_data_modules(ver: usize) -> usize {
    let mut r = (16 * ver + 128) * ver + 64;
    if ver >= 2 {
        let n = ver / 7 + 2;
        r -= (25 * n - 10) * n - 55;
        if ver >= 7 {
            r -= 36;
        }
    }
    r
}

fn data_codewords(ver: usize, ecc: Ecc) -> usize {
    let e = ecc.ordinal();
    raw_data_modules(ver) / 8
        - usize::from(ECC_CODEWORDS_PER_BLOCK[e][ver])
            * usize::from(NUM_ERROR_CORRECTION_BLOCKS[e][ver])
}

#[derive(Default)]
struct BitBuf(Vec<bool>);

impl BitBuf {
    fn push(&mut self, v: u32, n: usize) {
        for i in (0..n).rev() {
            self.0.push((v >> i) & 1 != 0);
        }
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn bytes(&self) -> Vec<u8> {
        self.0
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
            .collect()
    }
}

fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((u32::from(y) >> i) & 1) * u32::from(x);
    }
    z as u8
}

fn rs_divisor(degree: usize) -> Vec<u8> {
    let mut result = vec![0u8; degree];
    result[degree - 1] = 1;
    let mut root = 1u8;
    for _ in 0..degree {
        for j in 0..degree {
            result[j] = gf_mul(result[j], root);
            if j + 1 < degree {
                result[j] ^= result[j + 1];
            }
        }
        root = gf_mul(root, 0x02);
    }
    result
}

fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result.remove(0);
        result.push(0);
        for (r, &d) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(d, factor);
        }
    }
    result
}

fn add_ecc_and_interleave(data: &[u8], ver: usize, ecc: Ecc) -> Vec<u8> {
    let e = ecc.ordinal();
    let num_blocks = usize::from(NUM_ERROR_CORRECTION_BLOCKS[e][ver]);
    let block_ecc_len = usize::from(ECC_CODEWORDS_PER_BLOCK[e][ver]);
    let raw_codewords = raw_data_modules(ver) / 8;
    let num_short = num_blocks - raw_codewords % num_blocks;
    let short_len = raw_codewords / num_blocks;
    let divisor = rs_divisor(block_ecc_len);
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    let mut k = 0;
    for i in 0..num_blocks {
        let dat_len = short_len - block_ecc_len + usize::from(i >= num_short);
        let dat = &data[k..k + dat_len];
        k += dat_len;
        let mut block = dat.to_vec();
        let ecc_bytes = rs_remainder(dat, &divisor);
        if i < num_short {
            block.push(0);
        }
        block.extend(ecc_bytes);
        blocks.push(block);
    }
    let mut result = Vec::with_capacity(raw_codewords);
    for i in 0..blocks[0].len() {
        for (j, b) in blocks.iter().enumerate() {
            if i != short_len - block_ecc_len || j >= num_short {
                result.push(b[i]);
            }
        }
    }
    result
}

struct Grid {
    size: usize,
    modules: Vec<bool>,
    function: Vec<bool>,
}

impl Grid {
    fn set(&mut self, x: i32, y: i32, dark: bool) {
        let i = y as usize * self.size + x as usize;
        self.modules[i] = dark;
        self.function[i] = true;
    }

    fn draw_function_patterns(&mut self, version: usize, ecc: Ecc) {
        let size = self.size as i32;
        for i in 0..size {
            self.set(6, i, i % 2 == 0);
            self.set(i, 6, i % 2 == 0);
        }
        for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
            for dy in -4i32..=4 {
                for dx in -4i32..=4 {
                    let (x, y) = (cx + dx, cy + dy);
                    if (0..size).contains(&x) && (0..size).contains(&y) {
                        let d = dx.abs().max(dy.abs());
                        self.set(x, y, d != 2 && d != 4);
                    }
                }
            }
        }
        let pos = alignment_positions(version, self.size);
        let n = pos.len();
        for (i, &cx) in pos.iter().enumerate() {
            for (j, &cy) in pos.iter().enumerate() {
                if (i == 0 && (j == 0 || j == n - 1)) || (i == n - 1 && j == 0) {
                    continue;
                }
                for dy in -2i32..=2 {
                    for dx in -2i32..=2 {
                        self.set(cx + dx, cy + dy, dx.abs().max(dy.abs()) != 1);
                    }
                }
            }
        }
        self.draw_format(ecc, 0);
        self.draw_version(version);
    }

    fn draw_format(&mut self, ecc: Ecc, mask: u8) {
        let data = (ecc.format_bits() << 3) | u32::from(mask);
        let bits = format_word(data);
        let bit = |i: u32| (bits >> i) & 1 != 0;
        let size = self.size as i32;
        for i in 0..6 {
            self.set(8, i, bit(i as u32));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i, 8, bit(i as u32));
        }
        for i in 0..8 {
            self.set(size - 1 - i, 8, bit(i as u32));
        }
        for i in 8..15 {
            self.set(8, size - 15 + i, bit(i as u32));
        }
        self.set(8, size - 8, true);
    }

    fn draw_version(&mut self, version: usize) {
        if version < 7 {
            return;
        }
        let bits = version_word(version as u32);
        let size = self.size as i32;
        for i in 0..18 {
            let b = (bits >> i) & 1 != 0;
            let a = size - 11 + i % 3;
            let c = i / 3;
            self.set(a, c, b);
            self.set(c, a, b);
        }
    }

    fn draw_codewords(&mut self, data: &[u8]) {
        let size = self.size as i32;
        let mut i = 0usize;
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = right - j;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert };
                    let idx = y as usize * self.size + x as usize;
                    if !self.function[idx] && i < data.len() * 8 {
                        self.modules[idx] = (data[i >> 3] >> (7 - (i & 7))) & 1 != 0;
                        i += 1;
                    }
                }
            }
            right -= 2;
        }
    }

    fn apply_mask(&mut self, mask: u8) {
        for y in 0..self.size {
            for x in 0..self.size {
                let i = y * self.size + x;
                if self.function[i] {
                    continue;
                }
                let (xi, yi) = (x, y);
                let invert = match mask {
                    0 => (xi + yi) % 2 == 0,
                    1 => yi % 2 == 0,
                    2 => xi % 3 == 0,
                    3 => (xi + yi) % 3 == 0,
                    4 => (xi / 3 + yi / 2) % 2 == 0,
                    5 => xi * yi % 2 + xi * yi % 3 == 0,
                    6 => (xi * yi % 2 + xi * yi % 3) % 2 == 0,
                    _ => ((xi + yi) % 2 + xi * yi % 3) % 2 == 0,
                };
                self.modules[i] ^= invert;
            }
        }
    }

    fn at(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }

    /// The four penalty rules of the standard.
    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut total = 0u32;
        // rule 1: runs of five or more of one colour, rows and columns
        for transpose in [false, true] {
            for a in 0..n {
                let mut run = 1u32;
                for b in 1..n {
                    let (cur, prev) = if transpose {
                        (self.at(a, b), self.at(a, b - 1))
                    } else {
                        (self.at(b, a), self.at(b - 1, a))
                    };
                    if cur == prev {
                        run += 1;
                        if run == 5 {
                            total += 3;
                        } else if run > 5 {
                            total += 1;
                        }
                    } else {
                        run = 1;
                    }
                }
            }
        }
        // rule 2: 2x2 blocks of one colour
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = self.at(x, y);
                if c == self.at(x + 1, y) && c == self.at(x, y + 1) && c == self.at(x + 1, y + 1) {
                    total += 3;
                }
            }
        }
        // rule 3: 1:1:3:1:1 finder-like patterns with four light modules on one side
        const A: [bool; 11] = [
            true, false, true, true, true, false, true, false, false, false, false,
        ];
        let b: Vec<bool> = A.iter().rev().copied().collect();
        for transpose in [false, true] {
            for a in 0..n {
                for s in 0..=n.saturating_sub(11) {
                    let line = |k: usize| {
                        if transpose {
                            self.at(a, s + k)
                        } else {
                            self.at(s + k, a)
                        }
                    };
                    if (0..11).all(|k| line(k) == A[k]) {
                        total += 40;
                    }
                    if (0..11).all(|k| line(k) == b[k]) {
                        total += 40;
                    }
                }
            }
        }
        // rule 4: balance of dark modules
        let dark = self.modules.iter().filter(|&&d| d).count();
        let percent = dark * 100 / self.modules.len();
        let k = (percent as i32 - 50).unsigned_abs() / 5;
        total + k * 10
    }
}

/// The 15-bit format word: 5 data bits, 10 BCH bits, masked with 0x5412.
pub fn format_word(data: u32) -> u32 {
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | rem) ^ 0x5412
}

/// The 18-bit version word of versions 7 to 40: 6 data bits and 12 BCH bits.
pub fn version_word(version: u32) -> u32 {
    let mut rem = version;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    (version << 12) | rem
}

fn alignment_positions(version: usize, size: usize) -> Vec<i32> {
    if version == 1 {
        return Vec::new();
    }
    let n = version / 7 + 2;
    let step = if version == 32 {
        26
    } else {
        (version * 4 + n * 2 + 1) / (n * 2 - 2) * 2
    } as i32;
    let mut result = vec![6];
    let mut pos = size as i32 - 7;
    while result.len() < n {
        result.insert(1, pos);
        pos -= step;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_words_match_the_standard() {
        // ISO 18004 Annex C: M, mask 0 is 101010000010010; L, mask 0 is 111011111000100
        assert_eq!(format_word(0b00_000), 0b101010000010010);
        assert_eq!(format_word(0b01_000), 0b111011111000100);
    }

    #[test]
    fn version_words_match_the_standard() {
        // ISO 18004 Annex D
        assert_eq!(version_word(7), 0b000111110010010100);
        assert_eq!(version_word(8), 0b001000010110111100);
        assert_eq!(version_word(40), 0b101000110001101001);
    }

    #[test]
    fn data_capacity_matches_the_standard_table() {
        // byte-mode capacities of the well known table (L, M, Q, H)
        for (v, caps) in [
            (1, [17, 14, 11, 7]),
            (2, [32, 26, 20, 14]),
            (5, [106, 84, 60, 44]),
            (10, [271, 213, 151, 119]),
            (40, [2953, 2331, 1663, 1273]),
        ] {
            for (e, ecc) in [Ecc::Low, Ecc::Medium, Ecc::Quartile, Ecc::High]
                .into_iter()
                .enumerate()
            {
                let count_bits = if v < 10 { 8 } else { 16 };
                let bytes = (data_codewords(v, ecc) * 8 - 4 - count_bits) / 8;
                assert_eq!(bytes, caps[e], "version {v} ecc {e}");
            }
        }
    }

    #[test]
    fn reed_solomon_known_vector() {
        // the worked example of the standard (1-M "01234567"): divisor of degree 10 starts 1,216,194
        let d = rs_divisor(10);
        assert_eq!(d.len(), 10);
        // the 16 data codewords of "01234567" at 1-M and their 10 error words (ISO 18004 Annex I)
        let data = [
            0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11,
            0xEC, 0x11,
        ];
        assert_eq!(
            rs_remainder(&data, &d),
            [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55]
        );
    }

    #[test]
    fn sizes_follow_the_version() {
        let q = QrCode::encode_text("http://192.168.1.5:8080/opds", Ecc::Low).unwrap();
        assert_eq!(q.size(), q.version() * 4 + 17);
        assert!(q.version() <= 3);
        assert!(!q.dark(1000, 0));
        // the finder patterns are always where they belong
        for (x, y) in [(0, 0), (q.size() - 7, 0), (0, q.size() - 7)] {
            assert!(
                q.dark(x, y)
                    && q.dark(x + 6, y + 6)
                    && !q.dark(x + 1, y + 1)
                    && q.dark(x + 3, y + 3)
            );
        }
        assert!(q.dark(8, q.size() - 8), "the dark module");
    }

    #[test]
    fn too_long_text_does_not_fit() {
        assert!(QrCode::encode_text(&"x".repeat(2954), Ecc::Low).is_none());
        assert!(QrCode::encode_text(&"x".repeat(2953), Ecc::Low).is_some());
    }

    #[test]
    fn auto_mask_is_deterministic_and_valid() {
        let text = "https://books.example.com/opds";
        let q = QrCode::encode_text(text, Ecc::Medium).unwrap();
        assert!(q.mask() < 8);
        assert_eq!(q, QrCode::encode_text(text, Ecc::Medium).unwrap());
    }

    /// Complete symbols, rows as hex, most significant bit first (decoded by an independent decoder
    /// when recorded, see the table). Covers a single block (1), several
    /// blocks with uneven lengths (5-Q), alignment patterns and the version word (7-L, 10-M).
    #[test]
    fn symbols_stay_as_recorded() {
        for (text, ecc, mask, version, rows) in REFERENCE {
            let q = QrCode::encode_fixed(text, *ecc, *mask).unwrap();
            assert_eq!(q.version(), *version, "{text}");
            let mut got = Vec::new();
            for y in 0..q.size() {
                let mut hex = String::new();
                for chunk in (0..q.size()).step_by(4) {
                    let mut n = 0u8;
                    for k in 0..4 {
                        n = (n << 1) | u8::from(q.dark(chunk + k, y));
                    }
                    hex.push(char::from_digit(u32::from(n), 16).unwrap());
                }
                got.push(hex);
            }
            assert_eq!(got.join(","), *rows, "{text} ecc {ecc:?} mask {mask}");
        }
    }

    include!("qr_reference.in");
}
