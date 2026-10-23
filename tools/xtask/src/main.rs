//! `cargo xtask <command> [args]` — the workspace task runner (DESIGN §2.3, §2.5).
//!
//! main.rs, util.rs, check.rs and size.rs are owned by WP-01. Every other module was
//! created by WP-01 as a stub and belongs to the WP named in the dispatch table
//! (DESIGN §16.2 stub hand-off); owners replace the stub body without touching main.rs.

mod android;
mod bindings;
mod check;
mod ios;
mod opus;
mod size;
mod ts;
mod util;
mod vectors;
mod wasm;

use std::process::ExitCode;

/// A subcommand: name, owning WP, one-line help, entry point.
struct Task {
    name: &'static str,
    owner: &'static str,
    help: &'static str,
    run: fn(&[String]) -> util::Result,
}

/// Dispatch table, in the order of the DESIGN §2.5 CI sequence.
const TASKS: &[Task] = &[
    Task {
        name: "fetch-opus",
        owner: "WP-06",
        help: "download + SHA-256 verify libopus 1.6.1 and its DNN weights (idempotent)",
        run: opus::fetch_opus,
    },
    Task {
        name: "opus-smoke",
        owner: "WP-06",
        help: "cross-build opus-sys for the iOS and Android targets",
        run: opus::opus_smoke,
    },
    Task {
        name: "check",
        owner: "WP-01",
        help: "fmt + clippy + determinism grep + pin-list equality + wasm getrandom tree",
        run: check::run,
    },
    Task {
        name: "vectors",
        owner: "WP-04",
        help: "write golden wire/crypto vectors to test-vectors/ [--check: fail on drift]",
        run: vectors::run,
    },
    Task {
        name: "gen-ts",
        owner: "WP-03",
        help: "generate packages/engine-api/src/generated/ [--check: fail on drift]",
        run: ts::run,
    },
    Task {
        name: "gen-bindings",
        owner: "WP-12",
        help: "generate committed Swift + Kotlin UniFFI bindings [--check: fail on drift]",
        run: bindings::run,
    },
    Task {
        name: "build-ios",
        owner: "WP-12",
        help: "build MeshCoreFFI.xcframework (device + simulator staticlib)",
        run: ios::run,
    },
    Task {
        name: "build-android",
        owner: "WP-12",
        help: "build libmesh_ffi.so for arm64-v8a + x86_64 into jniLibs",
        run: android::run,
    },
    Task {
        name: "build-wasm",
        owner: "WP-13",
        help: "build mesh-wasm and copy it to packages/engine-web/wasm/",
        run: wasm::run,
    },
    Task {
        name: "size",
        owner: "WP-01",
        help: "size gate: iOS .a <= baseline + 10 %, stripped arm64 .so <= 9 MB [--update-baseline]",
        run: size::run,
    },
];

fn usage() -> String {
    let mut out = String::from("usage: cargo xtask <command> [args]\n\ncommands:\n");
    for t in TASKS {
        out.push_str(&format!("  {:<14} {} ({})\n", t.name, t.help, t.owner));
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((name, rest)) = args.split_first() else {
        eprint!("{}", usage());
        return ExitCode::FAILURE;
    };
    if matches!(name.as_str(), "help" | "--help" | "-h") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let Some(task) = TASKS.iter().find(|t| t.name == name) else {
        eprintln!("xtask: unknown command `{name}`\n");
        eprint!("{}", usage());
        return ExitCode::FAILURE;
    };
    match (task.run)(rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask {}: {e}", task.name);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TASKS;

    #[test]
    fn dispatch_table_matches_the_design_interface() {
        let names: Vec<_> = TASKS.iter().map(|t| t.name).collect();
        for expected in [
            "fetch-opus",
            "opus-smoke",
            "build-ios",
            "build-android",
            "build-wasm",
            "gen-bindings",
            "gen-ts",
            "vectors",
            "check",
            "size",
        ] {
            assert!(names.contains(&expected), "missing subcommand {expected}");
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate subcommand names");
    }

    /// Normalises a shell line for comparison: drops a trailing `# comment`, quotes and
    /// repeated whitespace.
    fn normalise(line: &str) -> String {
        let code = line.split(" #").next().unwrap_or(line);
        code.replace(['"', '\''], "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `pnpm run ci` must run DESIGN §2.5 exactly: compare the dry run of
    /// scripts/tasks.mjs with the code block of the design document.
    #[test]
    fn tasks_mjs_ci_matches_design_section_2_5() {
        let root = crate::util::repo_root();
        let design = std::fs::read_to_string(root.join("docs/DESIGN.md")).expect("DESIGN.md");
        let section = design
            .split("### 2.5 Script surface and CI")
            .nth(1)
            .expect("§2.5 heading");
        let block = section
            .split("```bash\n")
            .nth(1)
            .and_then(|b| b.split("```").next())
            .expect("§2.5 code block");
        let expected: Vec<String> = block
            .lines()
            .take_while(|l| !l.starts_with("# nightly"))
            .map(normalise)
            .filter(|l| !l.is_empty())
            .flat_map(|l| {
                // `a && b && c` on one design line is three CI steps; `(cd x && y)` is one.
                if l.starts_with('(') {
                    vec![l]
                } else {
                    l.split(" && ").map(str::to_owned).collect()
                }
            })
            .collect();

        let out = std::process::Command::new("node")
            .args(["scripts/tasks.mjs", "ci", "--dry-run"])
            .current_dir(&root)
            .output()
            .expect("node runs scripts/tasks.mjs");
        assert!(out.status.success(), "tasks.mjs ci --dry-run failed");
        let actual: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("[dry-run] "))
            .map(normalise)
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            actual.first().map(String::as_str),
            Some("source scripts/env.sh")
        );
        assert_eq!(
            actual.get(1).map(String::as_str),
            Some("cargo xtask fetch-opus")
        );
        assert!(actual.iter().any(|l| l == "cargo xtask size"));
    }
}
