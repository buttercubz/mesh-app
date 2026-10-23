//! A small QR Code encoder (ISO/IEC 18004) for the invite QR that `getInvite` returns as a
//! natively rendered image (DESIGN §3.7, §8.1: the invite text never reaches JS state).
//!
//! Scope: byte mode, error-correction level M (the design's "ECC M"), versions 1-20
//! (an invite URL is ≈ 280 bytes, version 12-13), automatic mask selection by the
//! standard penalty rules. No allocation-heavy tricks; everything is deterministic.

/// Largest supported version (byte capacity 666 at level M).
pub const MAX_VERSION: u8 = 20;

/// `(ec codewords per block, group-1 blocks, group-1 data codewords, group-2 blocks)`
/// for level M, versions 1-20; group-2 blocks hold one more data codeword.
const M_BLOCKS: [(u8, u8, u8, u8); 20] = [
    (10, 1, 16, 0),
    (16, 1, 28, 0),
    (26, 1, 44, 0),
    (18, 2, 32, 0),
    (24, 2, 43, 0),
    (16, 4, 27, 0),
    (18, 4, 31, 0),
    (22, 2, 38, 2),
    (22, 3, 36, 2),
    (26, 4, 43, 1),
    (30, 1, 50, 4),
    (22, 6, 36, 2),
    (22, 8, 37, 1),
    (24, 4, 40, 5),
    (24, 5, 41, 5),
    (28, 7, 45, 3),
    (28, 10, 46, 1),
    (26, 9, 43, 4),
    (26, 3, 44, 11),
    (26, 3, 41, 13),
];

/// Why a payload could not be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrError {
    /// The payload exceeds the capacity of version 20 at level M.
    TooLong,
}

/// A square module matrix (`true` = dark).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrCode {
    version: u8,
    size: usize,
    modules: Vec<bool>,
    function: Vec<bool>,
    mask: u8,
}

/// Number of data codewords of `version` at level M.
fn data_codewords(version: u8) -> usize {
    let (_, g1, d1, g2) = M_BLOCKS[usize::from(version - 1)];
    usize::from(g1) * usize::from(d1) + usize::from(g2) * (usize::from(d1) + 1)
}

/// Raw data modules (data + EC bits incl. remainder) of `version`.
pub fn raw_data_modules(version: u8) -> usize {
    let v = usize::from(version);
    let mut n = (16 * v + 128) * v + 64;
    if v >= 2 {
        let align = v / 7 + 2;
        n -= (25 * align - 10) * align - 55;
        if v >= 7 {
            n -= 36;
        }
    }
    n
}

/// Total codewords (data + EC) of `version`, from the M table.
pub fn total_codewords(version: u8) -> usize {
    let (ec, g1, _, g2) = M_BLOCKS[usize::from(version - 1)];
    data_codewords(version) + usize::from(ec) * (usize::from(g1) + usize::from(g2))
}

fn char_count_bits(version: u8) -> usize {
    if version <= 9 { 8 } else { 16 }
}

/// Smallest version whose level-M byte capacity holds `len` bytes.
pub fn version_for(len: usize) -> Option<u8> {
    (1..=MAX_VERSION).find(|&v| 4 + char_count_bits(v) + 8 * len <= 8 * data_codewords(v))
}

struct BitBuf(Vec<bool>);

impl BitBuf {
    fn push(&mut self, value: u32, bits: usize) {
        for i in (0..bits).rev() {
            self.0.push((value >> i) & 1 == 1);
        }
    }
}

impl QrCode {
    /// Encodes `data` in byte mode at level M with the smallest fitting version.
    pub fn encode(data: &[u8]) -> Result<QrCode, QrError> {
        let version = version_for(data.len()).ok_or(QrError::TooLong)?;
        let capacity_bits = 8 * data_codewords(version);
        let mut bb = BitBuf(Vec::with_capacity(capacity_bits));
        bb.push(0b0100, 4);
        bb.push(data.len() as u32, char_count_bits(version));
        for &b in data {
            bb.push(u32::from(b), 8);
        }
        let term = (capacity_bits - bb.0.len()).min(4);
        bb.push(0, term);
        let pad = (8 - bb.0.len() % 8) % 8;
        bb.push(0, pad);
        let mut pad_byte = 0xEC;
        while bb.0.len() < capacity_bits {
            bb.push(pad_byte, 8);
            pad_byte ^= 0xEC ^ 0x11;
        }
        let codewords: Vec<u8> =
            bb.0.chunks(8)
                .map(|c| c.iter().fold(0u8, |acc, &b| (acc << 1) | u8::from(b)))
                .collect();
        let all = add_ecc_and_interleave(version, &codewords);
        let size = usize::from(version) * 4 + 17;
        let mut qr = QrCode {
            version,
            size,
            modules: vec![false; size * size],
            function: vec![false; size * size],
            mask: 0,
        };
        qr.draw_function_patterns();
        qr.draw_codewords(&all);
        let mut best: Option<(u32, u8)> = None;
        for mask in 0..8u8 {
            qr.apply_mask(mask);
            qr.draw_format_bits(mask);
            let p = qr.penalty();
            if best.is_none_or(|(bp, _)| p < bp) {
                best = Some((p, mask));
            }
            qr.apply_mask(mask);
        }
        let mask = best.map_or(0, |(_, m)| m);
        qr.apply_mask(mask);
        qr.draw_format_bits(mask);
        qr.mask = mask;
        Ok(qr)
    }

    pub fn version(&self) -> u8 {
        self.version
    }

    /// Modules per side (without quiet zone).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Mask pattern chosen (0-7).
    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// `true` when the module at column `x`, row `y` is dark (outside → light).
    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.modules[y * self.size + x]
    }

    fn set_fn(&mut self, x: usize, y: usize, dark: bool) {
        let i = y * self.size + x;
        self.modules[i] = dark;
        self.function[i] = true;
    }

    fn draw_function_patterns(&mut self) {
        let size = self.size;
        for i in 0..size {
            self.set_fn(6, i, i % 2 == 0);
            self.set_fn(i, 6, i % 2 == 0);
        }
        self.draw_finder(3, 3);
        self.draw_finder(size as i32 - 4, 3);
        self.draw_finder(3, size as i32 - 4);
        let pos = self.alignment_positions();
        let n = pos.len();
        for i in 0..n {
            for j in 0..n {
                let corner = (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0);
                if !corner {
                    self.draw_alignment(pos[i], pos[j]);
                }
            }
        }
        self.draw_format_bits(0);
        self.draw_version();
    }

    fn alignment_positions(&self) -> Vec<usize> {
        let v = usize::from(self.version);
        if v == 1 {
            return Vec::new();
        }
        let num = v / 7 + 2;
        let step = (v * 8 + num * 3 + 5) / (num * 4 - 4) * 2;
        let mut out = vec![6];
        let mut pos = self.size - 7;
        let mut rest = Vec::new();
        for _ in 0..num - 1 {
            rest.push(pos);
            pos -= step;
        }
        rest.reverse();
        out.extend(rest);
        out
    }

    fn draw_finder(&mut self, cx: i32, cy: i32) {
        for dy in -4..=4i32 {
            for dx in -4..=4i32 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
                    continue;
                }
                let d = dx.abs().max(dy.abs());
                self.set_fn(x as usize, y as usize, d != 2 && d != 4);
            }
        }
    }

    fn draw_alignment(&mut self, cx: usize, cy: usize) {
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let d = dx.abs().max(dy.abs());
                self.set_fn((cx as i32 + dx) as usize, (cy as i32 + dy) as usize, d != 1);
            }
        }
    }
}

impl QrCode {
    /// Format bits (level M = 0b00, `mask`), drawn in both copies.
    fn draw_format_bits(&mut self, mask: u8) {
        let bits = format_bits(mask);
        let size = self.size;
        let bit = |i: u32| (bits >> i) & 1 == 1;
        for i in 0..=5 {
            self.set_fn(8, i, bit(i as u32));
        }
        self.set_fn(8, 7, bit(6));
        self.set_fn(8, 8, bit(7));
        self.set_fn(7, 8, bit(8));
        for i in 9..15 {
            self.set_fn(14 - i, 8, bit(i as u32));
        }
        for i in 0..8 {
            self.set_fn(size - 1 - i, 8, bit(i as u32));
        }
        for i in 8..15 {
            self.set_fn(8, size - 15 + i, bit(i as u32));
        }
        self.set_fn(8, size - 8, true);
    }

    fn draw_version(&mut self) {
        if self.version < 7 {
            return;
        }
        let bits = version_bits(self.version);
        for i in 0..18 {
            let dark = (bits >> i) & 1 == 1;
            let a = self.size - 11 + i % 3;
            let b = i / 3;
            self.set_fn(a, b, dark);
            self.set_fn(b, a, dark);
        }
    }

    fn draw_codewords(&mut self, data: &[u8]) {
        let size = self.size as i32;
        let mut i = 0usize;
        let total_bits = data.len() * 8;
        let mut right = size - 1;
        while right >= 1 {
            if right == 6 {
                right = 5;
            }
            for vert in 0..size {
                for j in 0..2 {
                    let x = (right - j) as usize;
                    let upward = (right + 1) & 2 == 0;
                    let y = if upward { size - 1 - vert } else { vert } as usize;
                    let idx = y * self.size + x;
                    if !self.function[idx] && i < total_bits {
                        self.modules[idx] = (data[i >> 3] >> (7 - (i & 7))) & 1 == 1;
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
                let idx = y * self.size + x;
                if self.function[idx] {
                    continue;
                }
                let invert = match mask {
                    0 => (x + y) % 2 == 0,
                    1 => y % 2 == 0,
                    2 => x % 3 == 0,
                    3 => (x + y) % 3 == 0,
                    4 => (x / 3 + y / 2) % 2 == 0,
                    5 => x * y % 2 + x * y % 3 == 0,
                    6 => (x * y % 2 + x * y % 3) % 2 == 0,
                    _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
                };
                if invert {
                    self.modules[idx] = !self.modules[idx];
                }
            }
        }
    }

    /// The four penalty rules of ISO/IEC 18004 §7.8.3.
    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut p = 0u32;
        let line = |get: &dyn Fn(usize) -> bool| -> u32 {
            let mut score = 0u32;
            let mut run = 0usize;
            let mut prev = false;
            let mut seq: Vec<bool> = Vec::with_capacity(n + 8);
            seq.extend([false; 4]);
            for k in 0..n {
                let c = get(k);
                if k > 0 && c == prev {
                    run += 1;
                } else {
                    if run >= 5 {
                        score += 3 + (run - 5) as u32;
                    }
                    run = 1;
                    prev = c;
                }
                seq.push(c);
            }
            if run >= 5 {
                score += 3 + (run - 5) as u32;
            }
            seq.extend([false; 4]);
            let pat = [true, false, true, true, true, false, true];
            for s in 0..=seq.len() - 11 {
                let w = &seq[s..s + 11];
                let core = w[2..9] == pat;
                if w[..4] == [false; 4] && w[4..11] == pat
                    || core && w[7..] == [false; 4] && w[..7] == pat
                {
                    score += 40;
                }
            }
            score
        };
        for y in 0..n {
            p += line(&|x| self.get(x, y));
        }
        for x in 0..n {
            p += line(&|y| self.get(x, y));
        }
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = self.get(x, y);
                if c == self.get(x + 1, y) && c == self.get(x, y + 1) && c == self.get(x + 1, y + 1)
                {
                    p += 3;
                }
            }
        }
        let dark = self.modules.iter().filter(|m| **m).count();
        let total = n * n;
        let k = ((dark * 20).abs_diff(total * 10))
            .div_ceil(total)
            .saturating_sub(1);
        p + k as u32 * 10
    }
}

/// 15-bit format word for level M and `mask`, BCH-protected and XOR-masked.
pub fn format_bits(mask: u8) -> u32 {
    let data = u32::from(mask & 7);
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    ((data << 10) | (rem & 0x3FF)) ^ 0x5412
}

/// 18-bit version word (versions ≥ 7).
pub fn version_bits(version: u8) -> u32 {
    let v = u32::from(version);
    let mut rem = v;
    for _ in 0..12 {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
    }
    (v << 12) | (rem & 0xFFF)
}

fn gf_mul(a: u8, b: u8) -> u8 {
    let mut z: u16 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((u16::from(b) >> i) & 1) * u16::from(a);
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

/// Reed-Solomon remainder of `data` by the generator of `divisor`.
pub fn rs_remainder(data: &[u8], divisor: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; divisor.len()];
    for &b in data {
        let factor = b ^ result[0];
        result.remove(0);
        result.push(0);
        for (r, &d) in result.iter_mut().zip(divisor) {
            *r ^= gf_mul(d, factor);
        }
    }
    result
}

fn add_ecc_and_interleave(version: u8, data: &[u8]) -> Vec<u8> {
    let (ec, g1, d1, g2) = M_BLOCKS[usize::from(version - 1)];
    let (ec, d1) = (usize::from(ec), usize::from(d1));
    let nblocks = usize::from(g1) + usize::from(g2);
    let divisor = rs_divisor(ec);
    let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(nblocks);
    let mut k = 0;
    for b in 0..nblocks {
        let len = if b < usize::from(g1) { d1 } else { d1 + 1 };
        let dat = data[k..k + len].to_vec();
        k += len;
        let e = rs_remainder(&dat, &divisor);
        blocks.push((dat, e));
    }
    let mut out = Vec::with_capacity(total_codewords(version));
    for i in 0..=d1 {
        for (dat, _) in &blocks {
            if i < dat.len() {
                out.push(dat[i]);
            }
        }
    }
    for i in 0..ec {
        for (_, e) in &blocks {
            out.push(e[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_match_the_module_count() {
        for v in 1..=MAX_VERSION {
            assert_eq!(total_codewords(v), raw_data_modules(v) / 8, "version {v}");
        }
        assert_eq!(version_for(14), Some(1));
        assert_eq!(version_for(15), Some(2));
        assert_eq!(version_for(276), Some(12));
        assert_eq!(version_for(1000), None);
    }

    #[test]
    fn bch_words_match_the_standard() {
        // ISO/IEC 18004 Annex C: level M mask 0 → 101010000010010; version 7 → 0x07C94.
        assert_eq!(format_bits(0), 0b101_0100_0001_0010);
        assert_eq!(version_bits(7), 0x07C94);
        // The generator of degree 10 (level M version 1).
        assert_eq!(rs_divisor(2), vec![3, 2]);
    }

    #[test]
    fn alignment_positions_follow_the_table() {
        let mut q = QrCode::encode(&[0u8; 300]).unwrap();
        assert_eq!(q.version(), 13);
        assert_eq!(q.alignment_positions(), vec![6, 34, 62]);
        q.version = 20;
        q.size = 97;
        assert_eq!(q.alignment_positions(), vec![6, 34, 62, 90]);
        q.version = 14;
        q.size = 73;
        assert_eq!(q.alignment_positions(), vec![6, 26, 46, 66]);
    }

    #[test]
    fn finder_and_timing_patterns_are_drawn() {
        let q = QrCode::encode(b"convlinkvoice://join?v=1#AAAA").unwrap();
        assert_eq!(q.size(), usize::from(q.version()) * 4 + 17);
        for (x, y) in [(0, 0), (q.size() - 7, 0), (0, q.size() - 7)] {
            assert!(q.get(x, y) && q.get(x + 6, y + 6) && q.get(x + 3, y + 3));
            assert!(!q.get(x + 1, y + 1));
        }
        assert!(q.get(8, q.size() - 8), "dark module");
        assert!(q.get(6, 8) && !q.get(6, 9));
    }
}
