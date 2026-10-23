//! Minimal PNG writer for the invite QR (`getInvite` → `qrPng` data URL): 1-bit
//! grayscale, a 4-module quiet zone, zlib "stored" blocks (no compression library, no
//! decompression anywhere), CRC-32 and Adler-32 computed here.

use super::qr::QrCode;

/// Light quiet zone around the symbol, in modules (ISO/IEC 18004 minimum).
pub const QUIET_ZONE: usize = 4;

/// Renders `qr` at `scale` pixels per module as a PNG file.
pub fn render_png(qr: &QrCode, scale: usize) -> Vec<u8> {
    let scale = scale.max(1);
    let modules = qr.size() + 2 * QUIET_ZONE;
    let side = modules * scale;
    let row_bytes = side.div_ceil(8);
    let mut raw = Vec::with_capacity((row_bytes + 1) * side);
    for py in 0..side {
        raw.push(0); // filter: none
        let my = py / scale;
        let mut byte = 0u8;
        for px in 0..side {
            let mx = px / scale;
            let dark =
                mx >= QUIET_ZONE && my >= QUIET_ZONE && qr.get(mx - QUIET_ZONE, my - QUIET_ZONE);
            // Grayscale 1 bit: 1 = white.
            byte = (byte << 1) | u8::from(!dark);
            if px % 8 == 7 {
                raw.push(byte);
                byte = 0;
            }
        }
        if !side.is_multiple_of(8) {
            byte <<= 8 - side % 8;
            raw.push(byte);
        }
    }
    let mut png = Vec::with_capacity(raw.len() + 128);
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(side as u32).to_be_bytes());
    ihdr.extend_from_slice(&(side as u32).to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // depth 1, grayscale, deflate, no filter, no interlace
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib stream of uncompressed deflate blocks.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65_535 * 5 + 16);
    out.extend_from_slice(&[0x78, 0x01]);
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        let last = chunks.peek().is_none();
        out.push(u8::from(last));
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// CRC-32 (ISO-HDLC, as used by PNG).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Adler-32 (zlib trailer).
pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Standard base64 with padding (data URLs).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        s.push(char::from(T[(n >> 18) as usize & 63]));
        s.push(char::from(T[(n >> 12) as usize & 63]));
        s.push(if c.len() > 1 {
            char::from(T[(n >> 6) as usize & 63])
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            char::from(T[n as usize & 63])
        } else {
            '='
        });
    }
    s
}

/// `data:image/png;base64,…` of `text` as a QR code (`None` when it does not fit).
pub fn qr_data_url(text: &str, scale: usize) -> Option<String> {
    let qr = QrCode::encode(text.as_bytes()).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64(&render_png(&qr, scale))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_and_base64_match_known_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b""), "");
    }

    #[test]
    fn png_has_a_valid_structure() {
        let qr = QrCode::encode(b"hello").unwrap();
        let png = render_png(&qr, 2);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let side = (qr.size() + 8) * 2;
        assert_eq!(&png[16..20], &(side as u32).to_be_bytes());
        // IHDR CRC.
        let crc = crc32(&png[12..29]);
        assert_eq!(&png[29..33], &crc.to_be_bytes());
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
        let url = qr_data_url("hello", 4).unwrap();
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
    }

    #[test]
    fn zlib_splits_large_inputs() {
        let data = vec![7u8; 70_000];
        let z = zlib_stored(&data);
        assert_eq!(z.len(), 2 + 5 + 65_535 + 5 + (70_000 - 65_535) + 4);
        assert_eq!(z[2], 0, "first block is not final");
        assert_eq!(z[2 + 5 + 65_535], 1, "last block is final");
    }
}
