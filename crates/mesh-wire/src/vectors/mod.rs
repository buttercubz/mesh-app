//! Golden wire vectors (DESIGN §4.10): `generate()` returns every file of
//! `test-vectors/wire/`; `cargo xtask vectors` writes them and `--check` fails on any
//! byte change. Each vector is `{name, inputs, expected}`: `inputs` holds the fields
//! (byte strings as lowercase hex), `expected` the encoded bytes as lowercase hex, or an
//! array of hex strings when one input yields several units (GATT chunks, segments).
//!
//! Everything is fixed: byte strings come from [`pattern`], signatures and sealed boxes
//! are placeholders of the right length (real signatures and keys are pinned by the
//! crypto vectors of mesh-crypto in `test-vectors/crypto/`).

mod ble;
mod control;
mod frame;
mod link;
mod mesh;
mod records;

use serde_json::{Map, Value, json};

use crate::codec::to_hex;
use crate::error::Result;

/// Version of the vector file format.
pub const FORMAT_VERSION: u32 = 1;

/// One golden vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    pub name: String,
    pub inputs: Value,
    pub expected: Value,
}

/// One file of `test-vectors/wire/`.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorFile {
    pub file_name: String,
    pub description: String,
    pub vectors: Vec<Vector>,
}

impl VectorFile {
    fn new(file_name: &str, description: &str) -> Self {
        VectorFile {
            file_name: file_name.to_owned(),
            description: description.to_owned(),
            vectors: Vec::new(),
        }
    }

    /// Adds a vector whose expected output is one byte string.
    fn push(&mut self, name: &str, inputs: Value, expected: &[u8]) {
        self.vectors.push(Vector {
            name: name.to_owned(),
            inputs,
            expected: Value::String(to_hex(expected)),
        });
    }

    /// Adds a vector whose expected output is a list of byte strings.
    fn push_parts(&mut self, name: &str, inputs: Value, parts: &[Vec<u8>]) {
        let list = parts.iter().map(|p| Value::String(to_hex(p))).collect();
        self.vectors.push(Vector {
            name: name.to_owned(),
            inputs,
            expected: Value::Array(list),
        });
    }

    /// The JSON value of the file (object keys are sorted by serde_json's default map).
    pub fn to_json(&self) -> Value {
        let vectors = self
            .vectors
            .iter()
            .map(|v| {
                let mut m = Map::new();
                m.insert("name".into(), Value::String(v.name.clone()));
                m.insert("inputs".into(), v.inputs.clone());
                m.insert("expected".into(), v.expected.clone());
                Value::Object(m)
            })
            .collect();
        json!({
            "format": FORMAT_VERSION,
            "protocol": crate::PROTOCOL_VERSION,
            "description": self.description,
            "vectors": Value::Array(vectors),
        })
    }

    /// Byte-stable rendering: pretty JSON with a trailing newline.
    pub fn render(&self) -> String {
        let mut s = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// Every wire vector file, in a fixed order with fixed contents.
pub fn generate() -> Result<Vec<VectorFile>> {
    let files = vec![
        link::file()?,
        frame::file()?,
        records::file()?,
        mesh::file()?,
        mesh::segmentation_file()?,
        control::file()?,
        ble::hello_file()?,
        ble::adv_file()?,
        ble::names_file(),
        ble::sizes_file(),
    ];
    Ok(files)
}

/// Deterministic filler: `bytes[i] = seed + 31·i (mod 256)`.
pub fn pattern(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| seed.wrapping_add((i as u8).wrapping_mul(31)))
        .collect()
}

/// [`pattern`] as a fixed array.
pub fn pattern_array<const N: usize>(seed: u8) -> [u8; N] {
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = seed.wrapping_add((i as u8).wrapping_mul(31));
    }
    out
}

/// Hex string value.
fn hx(b: &[u8]) -> Value {
    Value::String(to_hex(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_byte_stable() {
        let a: Vec<String> = generate().unwrap().iter().map(VectorFile::render).collect();
        let b: Vec<String> = generate().unwrap().iter().map(VectorFile::render).collect();
        assert_eq!(a, b);
        let names: Vec<String> = generate()
            .unwrap()
            .into_iter()
            .map(|f| f.file_name)
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate file names");
    }

    #[test]
    fn vector_names_are_unique_per_file() {
        for f in generate().unwrap() {
            let mut names: Vec<&str> = f.vectors.iter().map(|v| v.name.as_str()).collect();
            let n = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), n, "duplicate vector name in {}", f.file_name);
            assert!(n > 0, "{} is empty", f.file_name);
        }
    }

    fn expected_bytes(f: &VectorFile) -> Vec<(String, Vec<u8>)> {
        f.vectors
            .iter()
            .filter_map(|v| {
                v.expected
                    .as_str()
                    .map(|e| (v.name.clone(), crate::codec::from_hex(e).unwrap()))
            })
            .collect()
    }

    /// Every vector's bytes parse back with the crate's own decoders.
    #[test]
    fn vectors_decode() {
        use crate::frame::LinkFrame;
        use crate::hello::Hello;
        use crate::mesh::{MeshPacket, MeshPayload};
        use crate::record::RecordIter;
        for f in generate().unwrap() {
            for (name, b) in expected_bytes(&f) {
                let ok = match f.file_name.as_str() {
                    "frame.json"
                        if name.starts_with("hs/")
                            || name.starts_with("reject/")
                            || name.starts_with("lab/") =>
                    {
                        LinkFrame::decode(&b)
                            .map(|fr| fr.encode().unwrap() == b)
                            .unwrap_or(false)
                    }
                    "frame.json"
                        if name.starts_with("sec/")
                            && !name.contains("/aad")
                            && !name.contains("/nonce") =>
                    {
                        LinkFrame::decode(&b).is_ok()
                    }
                    "records.json" if !name.contains("signed-message") => {
                        RecordIter::collect_all(&b).is_ok()
                    }
                    "mesh.json"
                        if !name.contains("signed-message")
                            && !name.starts_with("header/")
                            && !name.contains("payload-14") =>
                    {
                        let pkt = MeshPacket::decode(&b).unwrap();
                        MeshPayload::decode(&pkt).is_ok()
                    }
                    "hello.json" => Hello::decode(&b).is_ok(),
                    _ => true,
                };
                assert!(ok, "{}: {name} does not decode", f.file_name);
            }
        }
    }

    /// The committed files must equal the generator output (the same check as
    /// `cargo xtask vectors --check`, run here so `cargo test` catches drift too).
    #[test]
    fn committed_vectors_match() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/wire");
        for f in generate().unwrap() {
            let path = dir.join(&f.file_name);
            let committed = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e} (run `cargo xtask vectors`)", path.display()));
            assert!(
                committed == f.render(),
                "{} drifted (run `cargo xtask vectors`)",
                f.file_name
            );
        }
    }
}
