//! Golden crypto vectors (DESIGN §4.10): `generate()` returns every file of
//! `test-vectors/crypto/`; `cargo xtask vectors` writes them and `--check` fails on any
//! byte change. Each vector is `{name, inputs, expected}` with byte strings as lowercase
//! hex. Every key, ephemeral and secret is fixed (derived from [`pattern`]), and Ed25519
//! is deterministic, so the files are byte-stable.
//!
//! Files: `identity.json` (per-group identities), `schedule.json` (HKDF schedule, tags,
//! tokens), `invite.json` (invite blob, `invite_sig`, URL), `link.json` (Noise HS msg1-3
//! with fixed keys, handshake hash, raw-split keys, first SEC frame each way with nonce
//! and AAD, IDPROOF, key update), `membership.json` (ROSTER, JOIN_REQ, REKEY,
//! GROUP_CTL close/transfer, LEAVE, RIDE_CTL).

mod identity;
mod link;
mod membership;

use serde_json::{Map, Value, json};

use mesh_wire::codec::to_hex;

use crate::error::Result;
use crate::identity::GroupIdentity;
use crate::prim::Secret32;

/// Version of the vector file format (same as the wire vectors).
pub const FORMAT_VERSION: u32 = 1;

/// One golden vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    pub name: String,
    pub inputs: Value,
    pub expected: Value,
}

/// One file of `test-vectors/crypto/`.
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

    /// Adds a vector whose expected output is an object of named byte strings/values.
    fn push(&mut self, name: &str, inputs: Value, expected: Value) {
        self.vectors.push(Vector {
            name: name.to_owned(),
            inputs,
            expected,
        });
    }

    /// The JSON value of the file (object keys sorted by serde_json's default map).
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
            "protocol": mesh_wire::PROTOCOL_VERSION,
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

/// Every crypto vector file, in a fixed order with fixed contents.
pub fn generate() -> Result<Vec<VectorFile>> {
    Ok(vec![
        identity::identity_file(),
        identity::schedule_file(),
        identity::invite_file()?,
        link::file()?,
        membership::file()?,
    ])
}

/// Deterministic filler: `bytes[i] = seed + 31·i (mod 256)` (same as the wire vectors).
pub fn pattern<const N: usize>(seed: u8) -> [u8; N] {
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

/// Fixed fixtures shared by the vector files.
pub(crate) struct Fixture {
    pub group_id: [u8; 16],
    pub epoch: u32,
    pub epoch_secret: [u8; 32],
    pub leader: GroupIdentity,
    pub member: GroupIdentity,
    pub member2: GroupIdentity,
    pub seeds: [([u8; 32], [u8; 32]); 3],
}

pub(crate) fn fixture() -> Fixture {
    let seeds = [
        (pattern(0x30), pattern(0x31)),
        (pattern(0x40), pattern(0x41)),
        (pattern(0x48), pattern(0x49)),
    ];
    let id = |(a, b): ([u8; 32], [u8; 32])| {
        GroupIdentity::from_secrets(Secret32::new(a), Secret32::new(b))
    };
    Fixture {
        group_id: pattern(0x10),
        epoch: 7,
        epoch_secret: pattern(0x20),
        leader: id(seeds[0]),
        member: id(seeds[1]),
        member2: id(seeds[2]),
        seeds,
    }
}

#[cfg(test)]
mod tests;
