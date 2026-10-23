//! `cargo xtask check` (owned by WP-01, DESIGN §2.3, §2.5, §3.1): rustfmt, clippy with
//! `-D warnings`, the determinism grep over the seven sans-IO crates, the pin-list
//! equality with `scripts/msrv-probe/Cargo.toml`, and the wasm32 getrandom tree check.
//! Every step runs; the command fails if any step failed.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{self, DepSpec, fail};

/// The sans-IO crates (DESIGN §3.1): determinism lints denied, grep enforced.
pub const SANS_IO_CRATES: [&str; 7] = [
    "mesh-wire",
    "mesh-crypto",
    "mesh-lab",
    "mesh-voice",
    "mesh-routing",
    "mesh-core",
    "mesh-sim",
];

/// The attribute every sans-IO `lib.rs` must carry.
pub const DENY_ATTR: &str = "#![deny(clippy::disallowed_methods, clippy::disallowed_types)]";

/// Source patterns forbidden in sans-IO `src/` (second line of defence behind clippy).
pub const FORBIDDEN: [(&str, &str); 13] = [
    (
        "std::time",
        "no clocks: time is the caller-supplied Instant(u64 µs)",
    ),
    ("SystemTime", "no wall clock"),
    ("Instant::now", "no clocks: time is injected"),
    ("std::thread", "no threads"),
    ("thread::spawn", "no threads"),
    (
        "getrandom",
        "no OS randomness: use the injected ChaCha20Rng",
    ),
    (
        "thread_rng",
        "no OS randomness: use the injected ChaCha20Rng",
    ),
    ("OsRng", "no OS randomness: use the injected ChaCha20Rng"),
    ("from_os_rng", "no OS randomness: seed from injected bytes"),
    (
        "HashMap",
        "no hash-order iteration: use BTreeMap / sorted Vec",
    ),
    (
        "HashSet",
        "no hash-order iteration: use BTreeSet / sorted Vec",
    ),
    ("RandomState", "no random hashing"),
    ("std::env::", "no environment reads"),
];

/// Entry point for `cargo xtask check`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("check", args, &["--skip-lint"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask check [--skip-lint]".into()));
    }
    let root = util::repo_root();
    let mut failures: Vec<String> = Vec::new();
    let mut step = |name: &str, result: util::Result| match result {
        Ok(()) => eprintln!("xtask check: {name}: ok"),
        Err(e) => {
            eprintln!("xtask check: {name}: FAILED\n{e}");
            failures.push(name.to_owned());
        }
    };

    if flags.contains(&"--skip-lint") {
        eprintln!("xtask check: fmt + clippy skipped (--skip-lint)");
    } else {
        step(
            "cargo fmt --all --check",
            util::run(util::cargo().args(["fmt", "--all", "--check"]), "cargo fmt"),
        );
        step(
            "cargo clippy --workspace --all-targets -- -D warnings",
            util::run(
                util::cargo().args([
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ]),
                "cargo clippy",
            ),
        );
    }
    step("determinism grep", determinism(&root));
    step("pin-list equality", pins(&root));
    step("wasm32 getrandom tree", wasm_getrandom(&root));

    if failures.is_empty() {
        eprintln!("xtask check: all steps passed");
        Ok(())
    } else {
        Err(fail(format!("failed steps: {}", failures.join(", "))))
    }
}

// ---------------------------------------------------------------------------------
// Determinism grep
// ---------------------------------------------------------------------------------

/// Strips `//` comments and `/* */` blocks (tracked across lines by `in_block`);
/// string literals are kept so that `"//"` inside a string does not hide code.
fn strip_comments(line: &str, in_block: &mut bool) -> String {
    let mut out = String::new();
    let bytes = line.as_bytes();
    let (mut i, mut in_str) = (0usize, false);
    while i < bytes.len() {
        if *in_block {
            if bytes[i..].starts_with(b"*/") {
                *in_block = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        let c = bytes[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else if bytes[i..].starts_with(b"'\"'") {
            // A `'"'` char literal does not open a string.
            out.push_str("'\"'");
            i += 3;
            continue;
        } else if c == b'"' {
            in_str = true;
        } else if bytes[i..].starts_with(b"//") {
            break;
        } else if bytes[i..].starts_with(b"/*") {
            *in_block = true;
            i += 2;
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

/// Violations in one source text, as `(line number, pattern, reason)`.
pub fn scan_source(text: &str) -> Vec<(usize, &'static str, &'static str)> {
    let mut hits = Vec::new();
    let mut in_block = false;
    for (n, line) in text.lines().enumerate() {
        let code = strip_comments(line, &mut in_block);
        for (pat, why) in FORBIDDEN {
            if code.contains(pat) {
                hits.push((n + 1, pat, why));
            }
        }
    }
    hits
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> util::Result {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

fn determinism(root: &Path) -> util::Result {
    let mut problems = Vec::new();
    for krate in SANS_IO_CRATES {
        let src = root.join("crates").join(krate).join("src");
        let lib = src.join("lib.rs");
        let lib_text =
            fs::read_to_string(&lib).map_err(|e| fail(format!("{}: {e}", lib.display())))?;
        if !lib_text.lines().any(|l| l.trim() == DENY_ATTR) {
            problems.push(format!("{}: missing `{DENY_ATTR}`", lib.display()));
        }
        let mut files = Vec::new();
        rust_files(&src, &mut files)?;
        for file in files {
            let text = fs::read_to_string(&file)?;
            for (line, pat, why) in scan_source(&text) {
                let rel = file.strip_prefix(root).unwrap_or(&file);
                problems.push(format!("{}:{line}: `{pat}` ({why})", rel.display()));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(fail(format!(
            "determinism rules violated (DESIGN §3.1):\n  {}",
            problems.join("\n  ")
        )))
    }
}

// ---------------------------------------------------------------------------------
// Pin-list equality (DESIGN §2.6)
// ---------------------------------------------------------------------------------

fn same_spec(a: &DepSpec, b: &DepSpec) -> bool {
    a.version == b.version
        && a.default_features.unwrap_or(true) == b.default_features.unwrap_or(true)
}

/// Compares the probe's `[dependencies]` with the root `[workspace.dependencies]`:
/// identical names, identical exact (`=x.y.z`) versions, identical `default-features`.
pub fn compare_pins(probe: &[DepSpec], workspace: &[DepSpec]) -> Vec<String> {
    let mut problems = Vec::new();
    let external: Vec<&DepSpec> = workspace.iter().filter(|d| !d.path).collect();
    for p in probe {
        match p.version.as_deref() {
            Some(v) if v.starts_with('=') => {}
            other => problems.push(format!(
                "msrv-probe `{}` is not an exact `=x.y.z` pin ({other:?})",
                p.name
            )),
        }
        match external.iter().find(|w| w.name == p.name) {
            None => problems.push(format!(
                "`{}` {} is pinned in scripts/msrv-probe/Cargo.toml but missing from \
                 [workspace.dependencies]",
                p.name,
                p.version.as_deref().unwrap_or("?")
            )),
            Some(w) if !same_spec(p, w) => problems.push(format!(
                "`{}` differs: msrv-probe version {:?} default-features {}, workspace \
                 version {:?} default-features {}",
                p.name,
                p.version,
                p.default_features.unwrap_or(true),
                w.version,
                w.default_features.unwrap_or(true)
            )),
            Some(_) => {}
        }
    }
    for w in &external {
        if !probe.iter().any(|p| p.name == w.name) {
            problems.push(format!(
                "`{}` is in [workspace.dependencies] but not in scripts/msrv-probe/Cargo.toml \
                 (add it to the probe and run `bash scripts/msrv-probe.sh` first)",
                w.name
            ));
        }
    }
    problems
}

/// `channel = "x.y.z"` of rust-toolchain.toml / `rust-version = "x.y.z"` of a manifest.
fn quoted_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        let v = v.split('#').next()?.trim();
        v.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
    })
}

fn pins(root: &Path) -> util::Result {
    let probe_path = root.join("scripts/msrv-probe/Cargo.toml");
    let probe_text = fs::read_to_string(&probe_path)
        .map_err(|e| fail(format!("{}: {e}", probe_path.display())))?;
    let ws_text = fs::read_to_string(root.join("Cargo.toml"))?;
    let probe = util::read_dep_table(&probe_text, "dependencies")?;
    let workspace = util::read_dep_table(&ws_text, "workspace.dependencies")?;
    let mut problems = compare_pins(&probe, &workspace);

    let toolchain = fs::read_to_string(root.join("rust-toolchain.toml"))?;
    let channel = quoted_value(&toolchain, "channel");
    for (what, rv) in [
        (
            "Cargo.toml [workspace.package]",
            quoted_value(&ws_text, "rust-version"),
        ),
        (
            "scripts/msrv-probe/Cargo.toml",
            quoted_value(&probe_text, "rust-version"),
        ),
    ] {
        if rv != channel {
            problems.push(format!(
                "{what} rust-version {rv:?} != rust-toolchain.toml channel {channel:?}"
            ));
        }
    }
    if problems.is_empty() {
        eprintln!(
            "xtask check: {} pins equal to scripts/msrv-probe (toolchain {})",
            probe.len(),
            channel.unwrap_or_default()
        );
        Ok(())
    } else {
        Err(fail(format!(
            "pin lists differ (DESIGN §2.6):\n  {}",
            problems.join("\n  ")
        )))
    }
}

// ---------------------------------------------------------------------------------
// wasm32 getrandom tree (DESIGN §2.3)
// ---------------------------------------------------------------------------------

/// Checks `cargo tree --prefix depth -i getrandom` output: the only crate allowed to
/// depend on getrandom is mesh-wasm itself (the `wasm_js` guard), directly.
pub fn judge_getrandom_tree(tree: &str) -> Result<(), String> {
    let mut bad = Vec::new();
    for line in tree.lines().filter(|l| !l.trim().is_empty()) {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        let depth: usize = line[..digits]
            .parse()
            .map_err(|_| format!("unparsable line `{line}`"))?;
        let name = line[digits..].split_whitespace().next().unwrap_or_default();
        let allowed = match depth {
            0 => name == "getrandom",
            1 => name == "mesh-wasm",
            _ => false,
        };
        if !allowed {
            bad.push(line.to_owned());
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "getrandom is reachable in the wasm32 graph through a path other than the \
             mesh-wasm `wasm_js` guard:\n  {}\n(full tree:\n{tree})",
            bad.join("\n  ")
        ))
    }
}

fn wasm_getrandom(root: &Path) -> util::Result {
    if !root.join("crates/mesh-wasm/Cargo.toml").is_file() {
        eprintln!("xtask check: crates/mesh-wasm does not exist yet; tree check skipped");
        return Ok(());
    }
    let out = util::capture(util::cargo().args([
        "tree",
        "-p",
        "mesh-wasm",
        "--target",
        "wasm32-unknown-unknown",
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
            eprintln!("xtask check: getrandom is not in the wasm32 graph of mesh-wasm");
            return Ok(());
        }
        return Err(fail(format!("cargo tree failed:\n{}", out.stderr.trim())));
    }
    judge_getrandom_tree(&out.stdout).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_flags_forbidden_code_but_not_comments() {
        let src = "use std::collections::BTreeMap;\n\
                   // HashMap in a comment is fine\n\
                   /* std::time in a block\n still a comment HashSet */ let x = 1;\n\
                   let t = std::time::Instant::now(); // and a trailing note\n\
                   let q = '\"'; let m: HashMap<u8, u8> = todo;\n\
                   let s = \"// not a comment\"; std::thread::spawn(f);\n";
        let hits: Vec<_> = scan_source(src)
            .into_iter()
            .map(|(l, p, _)| (l, p))
            .collect();
        assert_eq!(
            hits,
            [
                (5, "std::time"),
                (5, "Instant::now"),
                (6, "HashMap"),
                (7, "std::thread"),
                (7, "thread::spawn"),
            ]
        );
    }

    #[test]
    fn the_sans_io_stubs_are_clean() {
        let root = util::repo_root();
        assert_eq!(determinism(&root), Ok(()));
    }

    fn dep(name: &str, version: &str, default_features: Option<bool>) -> DepSpec {
        DepSpec {
            name: name.into(),
            version: Some(version.into()),
            default_features,
            path: false,
        }
    }

    #[test]
    fn pin_comparison_reports_every_kind_of_drift() {
        let probe = [
            dep("a", "=1.0.0", None),
            dep("b", "=2.0.0", Some(false)),
            dep("c", "=3.0.0", None),
            dep("d", "^4", None),
        ];
        let ws = [
            dep("a", "=1.0.0", Some(true)),
            dep("b", "=2.0.0", None),
            dep("d", "^4", None),
            dep("e", "=5.0.0", None),
            DepSpec {
                name: "local".into(),
                version: None,
                default_features: None,
                path: true,
            },
        ];
        let problems = compare_pins(&probe, &ws);
        assert_eq!(problems.len(), 4, "{problems:#?}");
        assert!(problems[0].contains("`b` differs"));
        assert!(problems[1].contains("`c` =3.0.0 is pinned"));
        assert!(problems[2].contains("`d` is not an exact"));
        assert!(problems[3].contains("`e` is in [workspace.dependencies]"));
        assert!(compare_pins(&probe[..1], &ws[..1]).is_empty());
    }

    #[test]
    fn the_repository_pins_match_the_probe() {
        assert_eq!(pins(&util::repo_root()), Ok(()));
    }

    #[test]
    fn getrandom_tree_allows_only_the_direct_guard() {
        assert_eq!(
            judge_getrandom_tree("0getrandom v0.3.4\n1mesh-wasm v0.1.0 (/x)\n"),
            Ok(())
        );
        let err = judge_getrandom_tree(
            "0getrandom v0.3.4\n1rand_core v0.9.5\n2mesh-crypto v0.1.0 (/x)\n1mesh-wasm v0.1.0\n",
        )
        .expect_err("indirect path");
        assert!(err.contains("1rand_core") && err.contains("2mesh-crypto"));
    }
}
