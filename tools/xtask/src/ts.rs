//! `cargo xtask gen-ts [--check]` (owned by WP-03, DESIGN §3.7).
//!
//! Runs `cargo test -p mesh-api --features ts-export export_bindings` with
//! `TS_RS_EXPORT_DIR` pointing at a scratch directory under `target/`, then:
//!
//! - default: mirrors the scratch directory into `packages/engine-api/src/generated/`
//!   (writes changed files, deletes stale ones);
//! - `--check`: fails, listing every file, when the committed directory differs.
//!
//! Both modes also enforce the API freeze: the comment-free digest of the generated
//! types must equal `crates/mesh-api/frozen/api-<API_VERSION>.digest`. A contract change
//! without a minor-version bump fails; the first generation of a new version records its
//! digest (write mode only).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{self, fail};

/// Committed output directory, relative to the repository root.
const GENERATED_DIR: &str = "packages/engine-api/src/generated";
/// Frozen API digests, relative to the repository root.
const FROZEN_DIR: &str = "crates/mesh-api/frozen";
/// Scratch export directory, relative to the repository root.
const SCRATCH_DIR: &str = "target/xtask-gen-ts";

/// Entry point for `cargo xtask gen-ts`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("gen-ts", args, &["--check"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask gen-ts [--check]".into()));
    }
    let check = flags.contains(&"--check");
    let root = util::repo_root();

    let scratch = root.join(SCRATCH_DIR);
    export_to(&scratch)?;
    let fresh = read_ts_dir(&scratch)?;
    if fresh.is_empty() {
        return Err(fail(
            "ts-rs exported no files (is the `ts-export` feature wired?)",
        ));
    }

    let generated = root.join(GENERATED_DIR);
    let committed = if generated.is_dir() {
        read_ts_dir(&generated)?
    } else {
        BTreeMap::new()
    };
    let drift = diff(&committed, &fresh);

    freeze_check(&root, &fresh, check)?;

    if check {
        if drift.is_empty() {
            eprintln!(
                "xtask gen-ts: {GENERATED_DIR} is up to date ({} files)",
                fresh.len()
            );
            return Ok(());
        }
        return Err(fail(format!(
            "{GENERATED_DIR} is out of date; run `cargo xtask gen-ts`:\n  {}",
            drift.join("\n  ")
        )));
    }

    fs::create_dir_all(&generated)?;
    for name in committed.keys().filter(|n| !fresh.contains_key(*n)) {
        fs::remove_file(generated.join(name))?;
    }
    for (name, text) in &fresh {
        if committed.get(name) != Some(text) {
            fs::write(generated.join(name), text)?;
        }
    }
    if drift.is_empty() {
        eprintln!(
            "xtask gen-ts: {GENERATED_DIR} unchanged ({} files)",
            fresh.len()
        );
    } else {
        eprintln!(
            "xtask gen-ts: updated {GENERATED_DIR}:\n  {}",
            drift.join("\n  ")
        );
    }
    Ok(())
}

/// Runs the mesh-api export tests into a fresh `dir`.
fn export_to(dir: &Path) -> util::Result {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    let mut cmd = util::cargo();
    cmd.args([
        "test",
        "-p",
        "mesh-api",
        "--features",
        "ts-export",
        "--lib",
        "export_bindings",
        "--",
        "--test-threads=1",
    ])
    .env("TS_RS_EXPORT_DIR", dir)
    // `.js` specifiers resolve in Node ESM, Vite and `tsc --moduleResolution bundler`.
    .env("TS_RS_IMPORT_EXTENSION", "js")
    .env_remove("TS_RS_LARGE_INT");
    util::run(&mut cmd, "mesh-api ts-rs export")
}

/// `*.ts` files of `dir` by file name, with normalised line endings.
fn read_ts_dir(dir: &Path) -> util::Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() {
            return Err(fail(format!(
                "unexpected directory {} (ts-rs output is flat)",
                entry.path().display()
            )));
        }
        if !name.ends_with(".ts") {
            continue;
        }
        let text = fs::read_to_string(entry.path())?.replace("\r\n", "\n");
        files.insert(name, text);
    }
    Ok(files)
}

/// Human-readable differences between the committed and the fresh file sets.
fn diff(committed: &BTreeMap<String, String>, fresh: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for (name, text) in fresh {
        match committed.get(name) {
            None => out.push(format!("added    {name}")),
            Some(old) if old != text => out.push(format!("changed  {name}")),
            Some(_) => {}
        }
    }
    for name in committed.keys().filter(|n| !fresh.contains_key(*n)) {
        out.push(format!("removed  {name}"));
    }
    out
}

/// `API_VERSION` as written in the generated `constants.ts`.
fn api_version(files: &BTreeMap<String, String>) -> util::Result<String> {
    let constants = files
        .get("constants.ts")
        .ok_or_else(|| fail("generated constants.ts is missing"))?;
    constants
        .lines()
        .find_map(|l| l.strip_prefix("export const API_VERSION = \""))
        .and_then(|rest| rest.split('"').next())
        .map(str::to_owned)
        .ok_or_else(|| fail("constants.ts has no `export const API_VERSION = \"x.y\"` line"))
}

/// Contract digest: FNV-1a 64 over every generated file with comment lines and blank
/// lines removed, so documentation edits never count as contract changes. It detects
/// drift; it is not a security hash.
pub(crate) fn contract_digest(files: &BTreeMap<String, String>) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for (name, text) in files {
        feed(name.as_bytes());
        feed(b"\0");
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                continue;
            }
            feed(t.as_bytes());
            feed(b"\n");
        }
    }
    format!("{h:016x}")
}

/// Enforces the API freeze (see the module docs).
fn freeze_check(root: &Path, fresh: &BTreeMap<String, String>, check: bool) -> util::Result {
    let version = api_version(fresh)?;
    let digest = contract_digest(fresh);
    let path: PathBuf = root.join(FROZEN_DIR).join(format!("api-{version}.digest"));
    match fs::read_to_string(&path) {
        Ok(frozen) if frozen.trim() == digest => Ok(()),
        Ok(frozen) => Err(fail(format!(
            "the UI contract changed but API {version} is frozen (digest {} != {digest} in {}). \
             Bump API_VERSION_MINOR in crates/mesh-api/src/lib.rs (DESIGN §3.7: any change \
             after the freeze bumps the minor version), then run `cargo xtask gen-ts`.",
            frozen.trim(),
            path.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !check => {
            fs::create_dir_all(root.join(FROZEN_DIR))?;
            fs::write(&path, format!("{digest}\n"))?;
            eprintln!("xtask gen-ts: froze API {version} (digest {digest})");
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(fail(format!(
            "API {version} has no frozen digest ({}); run `cargo xtask gen-ts`",
            path.display()
        ))),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(n, t)| ((*n).to_owned(), (*t).to_owned()))
            .collect()
    }

    #[test]
    fn digest_ignores_comments_and_blank_lines_only() {
        let a = files(&[("A.ts", "// header\n/**\n * doc\n */\nexport type A = 1;\n")]);
        let b = files(&[("A.ts", "// other header\n\nexport type A = 1;\n")]);
        let c = files(&[("A.ts", "export type A = 2;\n")]);
        let d = files(&[("B.ts", "export type A = 1;\n")]);
        assert_eq!(contract_digest(&a), contract_digest(&b));
        assert_ne!(contract_digest(&a), contract_digest(&c));
        assert_ne!(contract_digest(&a), contract_digest(&d));
        assert_eq!(contract_digest(&a).len(), 16);
    }

    #[test]
    fn api_version_is_read_from_constants() {
        let f = files(&[(
            "constants.ts",
            "x\nexport const API_VERSION = \"1.1\" as const;\n",
        )]);
        assert_eq!(api_version(&f).expect("found"), "1.1");
        assert!(api_version(&files(&[("constants.ts", "nothing")])).is_err());
        assert!(api_version(&BTreeMap::new()).is_err());
    }

    #[test]
    fn diff_lists_added_changed_and_removed_files() {
        let old = files(&[("A.ts", "1"), ("B.ts", "2")]);
        let new = files(&[("A.ts", "1"), ("B.ts", "3"), ("C.ts", "4")]);
        assert_eq!(diff(&old, &new), ["changed  B.ts", "added    C.ts"]);
        assert_eq!(diff(&new, &old), ["changed  B.ts", "removed  C.ts"]);
        assert!(diff(&old, &old).is_empty());
    }
}
