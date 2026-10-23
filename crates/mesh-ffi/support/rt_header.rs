//! Checks the hand-written `include/mesh_rt.h` against `src/rt/abi.rs` with cbindgen used
//! as a library (DESIGN §2.3, contract C5).
//!
//! Shared by `cargo xtask gen-bindings` / `build-ios` (tools/xtask/src/bindings.rs) and
//! the mesh-ffi test `rt_header`, both through `#[path]`. cbindgen renders `abi.rs` as C;
//! both headers are then reduced to the same normal form (function prototypes, the
//! `MeshRtStats` field list, the version and rate `#define`s) and compared.

use std::collections::BTreeMap;
use std::path::Path;

/// The normal form of an RT header.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RtHeader {
    /// `mesh_rt_*` prototypes, whitespace-normalised, sorted by name.
    pub functions: BTreeMap<String, String>,
    /// `MeshRtStats` fields in declaration order: (type, name).
    pub stats_fields: Vec<(String, String)>,
    /// `MESH_RT_ABI_VERSION` and `MESH_RT_SAMPLE_RATE` values (integer suffixes removed).
    pub defines: BTreeMap<String, String>,
}

/// Renders `abi_rs` with cbindgen (C, no includes) and returns the text.
pub fn generate_with_cbindgen(abi_rs: &Path) -> Result<String, String> {
    let bindings = cbindgen::Builder::new()
        .with_src(abi_rs)
        .with_language(cbindgen::Language::C)
        .with_no_includes()
        .with_style(cbindgen::Style::Both)
        .generate()
        .map_err(|e| format!("cbindgen failed on {}: {e}", abi_rs.display()))?;
    let mut out = Vec::new();
    bindings.write(&mut out);
    String::from_utf8(out).map_err(|e| format!("cbindgen output is not UTF-8: {e}"))
}

/// Compares `header` with what cbindgen generates from `abi_rs`; `Err` lists every
/// difference.
pub fn check(abi_rs: &Path, header: &Path) -> Result<(), String> {
    let generated = parse(&generate_with_cbindgen(abi_rs)?);
    let text = std::fs::read_to_string(header)
        .map_err(|e| format!("cannot read {}: {e}", header.display()))?;
    let written = parse(&text);
    let mut diffs = Vec::new();
    if generated.functions.is_empty() || generated.stats_fields.is_empty() {
        diffs.push(format!(
            "cbindgen found no RT items in {}",
            abi_rs.display()
        ));
    }
    for (name, proto) in &generated.functions {
        match written.functions.get(name) {
            None => diffs.push(format!("{name} is missing from mesh_rt.h")),
            Some(p) if p != proto => {
                diffs.push(format!("{name}: header `{p}` != Rust `{proto}`"));
            }
            Some(_) => {}
        }
    }
    for name in written.functions.keys() {
        if !generated.functions.contains_key(name) {
            diffs.push(format!(
                "{name} is declared in mesh_rt.h but not exported by Rust"
            ));
        }
    }
    if generated.stats_fields != written.stats_fields {
        diffs.push(format!(
            "MeshRtStats fields differ: header {:?} != Rust {:?}",
            written.stats_fields, generated.stats_fields
        ));
    }
    for key in ["MESH_RT_ABI_VERSION", "MESH_RT_SAMPLE_RATE"] {
        if generated.defines.get(key) != written.defines.get(key) {
            diffs.push(format!(
                "{key}: header {:?} != Rust {:?}",
                written.defines.get(key),
                generated.defines.get(key)
            ));
        }
    }
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "include/mesh_rt.h does not match src/rt/abi.rs:\n  {}",
            diffs.join("\n  ")
        ))
    }
}

/// Removes `/* */` and `//` comments.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    loop {
        let block = rest.find("/*");
        let line = rest.find("//");
        match (block, line) {
            (Some(b), l) if l.is_none_or(|l| b < l) => {
                out.push_str(&rest[..b]);
                rest = rest[b + 2..].split_once("*/").map_or("", |(_, r)| r);
                out.push(' ');
            }
            (_, Some(l)) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].split_once('\n').map_or("", |(_, r)| r);
                out.push('\n');
            }
            (None, None) => {
                out.push_str(rest);
                return out;
            }
            (Some(_), None) => unreachable!("covered by the first arm"),
        }
    }
}

/// Collapses whitespace and removes it around punctuation.
fn squash(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = joined;
    for p in ["*", "(", ")", ","] {
        out = out
            .replace(&format!(" {p}"), p)
            .replace(&format!("{p} "), p);
    }
    out
}

/// Parses a header into its normal form.
pub fn parse(src: &str) -> RtHeader {
    let clean = strip_comments(src);
    let mut h = RtHeader::default();
    for line in clean.lines() {
        let t = line.trim();
        if let Some(def) = t.strip_prefix("#define ") {
            let mut it = def.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                let v = v.trim_end_matches(['u', 'U', 'l', 'L']);
                h.defines.insert(k.to_owned(), v.to_owned());
            }
        }
    }
    let code: String = clean
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(start) = code.find("struct MeshRtStats") {
        let body = code[start..]
            .split_once('{')
            .and_then(|(_, r)| r.split_once('}'))
            .map_or("", |(b, _)| b);
        for decl in body.split(';').map(str::trim).filter(|d| !d.is_empty()) {
            let mut words = decl.split_whitespace();
            let Some(ty) = words.next() else { continue };
            let names: String = words.collect::<Vec<_>>().join(" ");
            for n in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                h.stats_fields.push((ty.to_owned(), n.to_owned()));
            }
        }
    }
    for stmt in code.split(';') {
        // Drop anything up to an enclosing `extern "C" {` or a previous `}`, and the
        // `struct` keyword of parameter types (cbindgen's `Style::Both` spells it).
        let stmt = stmt.rsplit(['{', '}']).next().unwrap_or(stmt);
        let s = squash(&stmt.replace("struct ", ""));
        let Some(paren) = s.find('(') else { continue };
        let head = &s[..paren];
        let Some(name) = head.rsplit([' ', '*']).next() else {
            continue;
        };
        if name.starts_with("mesh_rt_") {
            h.functions.insert(name.to_owned(), s.clone());
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_expands_multi_name_fields_and_strips_suffixes() {
        let h = parse(
            "#define MESH_RT_ABI_VERSION 1u /* v */\n\
             typedef struct MeshRtStats { uint64_t a, b; /* c */ float x; } MeshRtStats;\n\
             int32_t  mesh_rt_capture( const int16_t * pcm , uint32_t n );\n\
             // void mesh_rt_gone(void);\n",
        );
        assert_eq!(h.defines["MESH_RT_ABI_VERSION"], "1");
        assert_eq!(
            h.stats_fields,
            vec![
                ("uint64_t".into(), "a".into()),
                ("uint64_t".into(), "b".into()),
                ("float".into(), "x".into())
            ]
        );
        assert_eq!(
            h.functions["mesh_rt_capture"],
            "int32_t mesh_rt_capture(const int16_t*pcm,uint32_t n)"
        );
        assert!(!h.functions.contains_key("mesh_rt_gone"));
    }
}
