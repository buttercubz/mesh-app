//! `cargo xtask build-wasm` (owned by WP-13, DESIGN §2.3, §11.2): builds mesh-wasm for
//! `wasm32-unknown-unknown` and copies it into `packages/engine-web/wasm/`.
//!
//! Needs nothing beyond the rustup target (no wasm-bindgen, no wasm-opt):
//! 1. `cargo tree -p mesh-wasm --target wasm32-unknown-unknown -i getrandom` must show
//!    getrandom only as mesh-wasm's own `wasm_js` guard (§2.3);
//! 2. `cargo build -p mesh-wasm --release --target wasm32-unknown-unknown`;
//! 3. the module is checked: it imports nothing and exports exactly the §11.2 function
//!    list (plus its linear memory);
//! 4. DWARF custom sections (`.debug_*`) are dropped (the release profile keeps line
//!    tables for native crash reports; the browser gets the `name` section instead);
//! 5. `packages/engine-web/wasm/mesh_wasm.wasm` and `mesh_wasm.json` (sizes and
//!    exports, read by tooling and the package tests) are written and the size printed.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::util::{self, fail};

const TARGET: &str = "wasm32-unknown-unknown";
const OUT_DIR: &str = "packages/engine-web/wasm";
const OUT_WASM: &str = "mesh_wasm.wasm";
const OUT_META: &str = "mesh_wasm.json";

/// The exported functions of DESIGN §11.2, exactly.
pub const ABI_EXPORTS: [&str; 8] = [
    "mw_alloc",
    "mw_command",
    "mw_free",
    "mw_poll_events",
    "mw_poll_lab",
    "mw_step",
    "mw_world_free",
    "mw_world_new",
];

/// Entry point for `cargo xtask build-wasm`.
pub fn run(args: &[String]) -> util::Result {
    let (_, positional) = util::parse_flags("build-wasm", args, &[])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask build-wasm".into()));
    }
    let root = util::repo_root();
    check_getrandom()?;
    let mut build = util::cargo();
    build.args(["build", "-p", "mesh-wasm", "--release", "--target", TARGET]);
    util::run(&mut build, "cargo build -p mesh-wasm (wasm32)").map_err(|e| {
        fail(format!(
            "{e}\n  hint: the only prerequisite is the rustup target: \
             `rustup target add {TARGET}`"
        ))
    })?;
    let built = target_dir(&root)
        .join(TARGET)
        .join("release")
        .join(OUT_WASM);
    let raw = fs::read(&built).map_err(|e| fail(format!("{}: {e}", built.display())))?;
    let module = parse(&raw).map_err(|e| fail(format!("{}: {e}", built.display())))?;
    let problems = check_abi(&module);
    if !problems.is_empty() {
        return Err(fail(format!(
            "{} breaks the DESIGN §11.2 ABI:\n  {}",
            built.display(),
            problems.join("\n  ")
        )));
    }
    let stripped = strip_debug(&raw, &module);
    let out_dir = root.join(OUT_DIR);
    fs::create_dir_all(&out_dir)?;
    fs::write(out_dir.join(OUT_WASM), &stripped)?;
    let meta = json!({
        "comment": "Written by `cargo xtask build-wasm` (WP-13). Do not edit.",
        "file": OUT_WASM,
        "bytes": stripped.len(),
        "rawBytes": raw.len(),
        "exports": module.function_exports,
    });
    let mut text = serde_json::to_string_pretty(&meta).map_err(|e| fail(e.to_string()))?;
    text.push('\n');
    fs::write(out_dir.join(OUT_META), text)?;
    eprintln!(
        "xtask build-wasm: {OUT_DIR}/{OUT_WASM}: {} bytes ({:.2} MiB; {} bytes before \
         dropping DWARF)",
        stripped.len(),
        stripped.len() as f64 / 1_048_576.0,
        raw.len()
    );
    Ok(())
}

/// `CARGO_TARGET_DIR` (relative to the repository root) or `target/`.
fn target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(d) if !d.is_empty() => {
            let p = PathBuf::from(d);
            if p.is_absolute() { p } else { root.join(p) }
        }
        _ => root.join("target"),
    }
}

/// Fails unless getrandom is absent from the wasm32 graph or reached only through
/// mesh-wasm's own guard (the same judgement as `cargo xtask check`).
fn check_getrandom() -> util::Result {
    let out = util::capture(util::cargo().args([
        "tree",
        "-p",
        "mesh-wasm",
        "--target",
        TARGET,
        "-i",
        "getrandom",
        "-e",
        "normal",
        "--prefix",
        "depth",
        "--charset",
        "ascii",
    ]))?;
    if !out.success {
        if out.stderr.contains("did not match any packages") {
            return Ok(());
        }
        return Err(fail(format!("cargo tree failed:\n{}", out.stderr.trim())));
    }
    crate::check::judge_getrandom_tree(&out.stdout).map_err(fail)
}

/// What the build check needs from a module.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Module {
    /// `(module, name)` of every import.
    pub imports: Vec<(String, String)>,
    /// Exported function names, sorted.
    pub function_exports: Vec<String>,
    /// Other exports (memory, globals, tables), sorted.
    pub other_exports: Vec<String>,
    /// Byte ranges `[start, end)` of every section (id byte included) and its custom
    /// name when it is a custom section.
    pub sections: Vec<(usize, usize, Option<String>)>,
}

fn leb_u32(b: &[u8], i: &mut usize) -> Result<u32, String> {
    let mut v: u64 = 0;
    for shift in (0..35).step_by(7) {
        let byte = *b.get(*i).ok_or("truncated LEB128")?;
        *i += 1;
        v |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return u32::try_from(v).map_err(|_| "LEB128 overflow".to_string());
        }
    }
    Err("LEB128 too long".into())
}

fn name(b: &[u8], i: &mut usize) -> Result<String, String> {
    let n = leb_u32(b, i)? as usize;
    let s = b.get(*i..*i + n).ok_or("truncated name")?;
    *i += n;
    String::from_utf8(s.to_vec()).map_err(|_| "name is not UTF-8".to_string())
}

/// Parses the section layout, imports and exports of a wasm module.
pub fn parse(b: &[u8]) -> Result<Module, String> {
    if b.len() < 8 || b[..4] != *b"\0asm" || b[4..8] != [1, 0, 0, 0] {
        return Err("not a wasm (version 1) module".into());
    }
    let mut m = Module::default();
    let mut i = 8;
    while i < b.len() {
        let start = i;
        let id = b[i];
        i += 1;
        let len = leb_u32(b, &mut i)? as usize;
        let body = i;
        let end = body
            .checked_add(len)
            .filter(|e| *e <= b.len())
            .ok_or("truncated section")?;
        let mut custom = None;
        let mut j = body;
        match id {
            0 => custom = Some(name(b, &mut j)?),
            // The module must not import anything, so only the first import is
            // decoded (its kind and type are not needed).
            2 if leb_u32(b, &mut j)? > 0 => {
                let module = name(b, &mut j)?;
                let field = name(b, &mut j)?;
                m.imports.push((module, field));
            }
            7 => {
                for _ in 0..leb_u32(b, &mut j)? {
                    let n = name(b, &mut j)?;
                    let kind = *b.get(j).ok_or("truncated export")?;
                    j += 1;
                    leb_u32(b, &mut j)?;
                    if kind == 0 {
                        m.function_exports.push(n);
                    } else {
                        m.other_exports.push(n);
                    }
                }
            }
            _ => {}
        }
        m.sections.push((start, end, custom));
        i = end;
    }
    m.function_exports.sort();
    m.other_exports.sort();
    Ok(m)
}

/// Every way `m` departs from the §11.2 ABI (empty when it matches).
pub fn check_abi(m: &Module) -> Vec<String> {
    let mut p = Vec::new();
    if let Some((module, field)) = m.imports.first() {
        p.push(format!(
            "imports `{module}.{field}`: the module must be self-contained (no wasm-bindgen, \
             no JS randomness)"
        ));
    }
    let want: Vec<String> = ABI_EXPORTS.iter().map(|s| s.to_string()).collect();
    if m.function_exports != want {
        p.push(format!(
            "exported functions {:?}, expected exactly {:?}",
            m.function_exports, want
        ));
    }
    if !m.other_exports.iter().any(|e| e == "memory") {
        p.push("does not export its linear memory as `memory`".into());
    }
    p
}

/// The module without its `.debug_*` custom sections.
pub fn strip_debug(b: &[u8], m: &Module) -> Vec<u8> {
    let mut out = b[..8].to_vec();
    for (start, end, custom) in &m.sections {
        if custom.as_deref().is_some_and(|n| n.starts_with(".debug")) {
            continue;
        }
        out.extend_from_slice(&b[*start..*end]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leb(mut v: usize, out: &mut Vec<u8>) {
        loop {
            let byte = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
        out.push(id);
        leb(body.len(), out);
        out.extend_from_slice(body);
    }

    fn named(n: &str, out: &mut Vec<u8>) {
        leb(n.len(), out);
        out.extend_from_slice(n.as_bytes());
    }

    /// A synthetic module: optional import, the given function exports + memory, a
    /// DWARF section and a `name` section.
    fn module(exports: &[&str], import: bool) -> Vec<u8> {
        let mut b = b"\0asm\x01\0\0\0".to_vec();
        if import {
            let mut s = vec![1];
            named("env", &mut s);
            named("rand", &mut s);
            s.extend_from_slice(&[0, 0]);
            section(2, &s, &mut b);
        }
        let mut s = Vec::new();
        leb(exports.len() + 1, &mut s);
        named("memory", &mut s);
        s.extend_from_slice(&[2, 0]);
        for (k, e) in exports.iter().enumerate() {
            named(e, &mut s);
            s.push(0);
            leb(k, &mut s);
        }
        section(7, &s, &mut b);
        let mut d = Vec::new();
        named(".debug_info", &mut d);
        d.extend_from_slice(&[0xAA; 300]);
        section(0, &d, &mut b);
        let mut n = Vec::new();
        named("name", &mut n);
        n.extend_from_slice(&[1, 2, 3]);
        section(0, &n, &mut b);
        b
    }

    #[test]
    fn exact_abi_passes_and_debug_is_dropped() {
        let b = module(&ABI_EXPORTS, false);
        let m = parse(&b).unwrap();
        assert!(check_abi(&m).is_empty(), "{:?}", check_abi(&m));
        let s = strip_debug(&b, &m);
        let ms = parse(&s).unwrap();
        assert_eq!(ms.function_exports, m.function_exports);
        assert!(
            ms.sections
                .iter()
                .all(|x| x.2.as_deref() != Some(".debug_info"))
        );
        assert!(ms.sections.iter().any(|x| x.2.as_deref() == Some("name")));
        assert!(s.len() + 300 < b.len());
    }

    #[test]
    fn imports_missing_or_extra_exports_fail() {
        let m = parse(&module(&ABI_EXPORTS, true)).unwrap();
        assert_eq!(check_abi(&m).len(), 1);
        let m = parse(&module(&ABI_EXPORTS[..7], false)).unwrap();
        assert_eq!(check_abi(&m).len(), 1);
        let mut more = ABI_EXPORTS.to_vec();
        more.push("mw_debug");
        let m = parse(&module(&more, false)).unwrap();
        assert_eq!(check_abi(&m).len(), 1);
        assert!(parse(b"\0asm\x02\0\0\0").is_err());
        assert!(parse(b"\0asm\x01\0\0\0\x07\x05\x01").is_err());
    }
}
