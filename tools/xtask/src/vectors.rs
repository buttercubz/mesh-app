//! `cargo xtask vectors [--check]` (owned by WP-04, DESIGN §4.10, §16.2 rule 3).
//!
//! Generates the golden vectors into `target/xtask-vectors/<kind>/` by running each
//! generator example, then either syncs them into `test-vectors/<kind>/` (default) or
//! compares them byte for byte (`--check`, fails on any added, removed or changed file).
//!
//! Generators (each an example taking the output directory as its only argument):
//! - `wire`: `cargo run -p mesh-wire --features vectors --example gen_vectors` (WP-04).
//! - `crypto`: the same convention in mesh-crypto (WP-07). It runs once
//!   `crates/mesh-crypto/examples/gen_vectors.rs` exists and the crate declares a
//!   `vectors` feature; until then it is reported as skipped.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{self, fail};

struct Generator {
    kind: &'static str,
    package: &'static str,
    owner: &'static str,
}

const GENERATORS: [Generator; 2] = [
    Generator {
        kind: "wire",
        package: "mesh-wire",
        owner: "WP-04",
    },
    Generator {
        kind: "crypto",
        package: "mesh-crypto",
        owner: "WP-07",
    },
];

/// Entry point for `cargo xtask vectors`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("vectors", args, &["--check"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask vectors [--check]".into()));
    }
    let check = flags.contains(&"--check");
    let root = util::repo_root();
    let mut drift = Vec::new();
    for g in &GENERATORS {
        if !generator_available(&root, g) {
            eprintln!(
                "xtask vectors: {} skipped ({} has no `vectors` feature + examples/gen_vectors.rs yet; owned by {})",
                g.kind, g.package, g.owner
            );
            continue;
        }
        let fresh = root.join("target/xtask-vectors").join(g.kind);
        if fresh.exists() {
            fs::remove_dir_all(&fresh)?;
        }
        fs::create_dir_all(&fresh)?;
        let mut cmd = util::cargo();
        cmd.args([
            "run",
            "--quiet",
            "-p",
            g.package,
            "--features",
            "vectors",
            "--example",
            "gen_vectors",
            "--",
        ])
        .arg(&fresh);
        util::run(&mut cmd, &format!("{} vector generation", g.kind))?;
        let committed = root.join("test-vectors").join(g.kind);
        if check {
            let d = compare(&fresh, &committed)?;
            if d.is_empty() {
                eprintln!(
                    "xtask vectors: {} vectors match test-vectors/{}/",
                    g.kind, g.kind
                );
            }
            drift.extend(d.into_iter().map(|m| format!("{}: {m}", g.kind)));
        } else {
            sync(&fresh, &committed)?;
            eprintln!("xtask vectors: wrote test-vectors/{}/", g.kind);
        }
    }
    if drift.is_empty() {
        Ok(())
    } else {
        for d in &drift {
            eprintln!("xtask vectors: drift: {d}");
        }
        Err(fail(format!(
            "{} golden vector file(s) differ from the generators; if the change is intended, run `cargo xtask vectors` and commit the result",
            drift.len()
        )))
    }
}

fn generator_available(root: &Path, g: &Generator) -> bool {
    let krate = root.join("crates").join(g.package);
    let example = krate.join("examples/gen_vectors.rs").is_file();
    let feature = fs::read_to_string(krate.join("Cargo.toml"))
        .map(|m| m.lines().any(|l| l.trim_start().starts_with("vectors =")))
        .unwrap_or(false);
    example && feature
}

/// Sorted `*.json` file names in `dir` (empty when the directory does not exist).
fn json_files(dir: &Path) -> util::Result<Vec<String>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".json") && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Lists every difference between the generated and the committed directory.
fn compare(fresh: &Path, committed: &Path) -> util::Result<Vec<String>> {
    let new = json_files(fresh)?;
    let old = json_files(committed)?;
    let mut out = Vec::new();
    for n in &new {
        if !old.contains(n) {
            out.push(format!("{n} is missing from {}", committed.display()));
            continue;
        }
        let a = fs::read(fresh.join(n))?;
        let b = fs::read(committed.join(n))?;
        if a != b {
            out.push(format!("{n} changed{}", first_difference(&a, &b)));
        }
    }
    for n in old.iter().filter(|n| !new.contains(n)) {
        out.push(format!("{n} is no longer generated"));
    }
    Ok(out)
}

fn first_difference(a: &[u8], b: &[u8]) -> String {
    let line = |s: &[u8], i: usize| s[..i.min(s.len())].iter().filter(|&&c| c == b'\n').count() + 1;
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!(" (first difference at line {})", line(a, i)),
        None => format!(" (length {} vs {})", a.len(), b.len()),
    }
}

/// Makes `committed` contain exactly the generated `*.json` files.
fn sync(fresh: &Path, committed: &Path) -> util::Result {
    fs::create_dir_all(committed)?;
    let new = json_files(fresh)?;
    for stale in json_files(committed)?.iter().filter(|n| !new.contains(n)) {
        fs::remove_file(committed.join(stale))?;
    }
    for n in &new {
        let src: PathBuf = fresh.join(n);
        let bytes = fs::read(&src)?;
        let dst = committed.join(n);
        if fs::read(&dst).ok().as_deref() != Some(bytes.as_slice()) {
            fs::write(&dst, bytes)?;
        }
    }
    Ok(())
}
