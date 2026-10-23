//! `cargo xtask gen-bindings [--check]` (owned by WP-12, DESIGN §2.3).
//!
//! 1. Checks the hand-written `crates/mesh-ffi/include/mesh_rt.h` against
//!    `src/rt/abi.rs` with cbindgen used as a library (contract C5).
//! 2. Builds the host `libmesh_ffi.dylib` (`cargo build -p mesh-ffi --release`).
//! 3. Runs the workspace `uniffi-bindgen` in library mode for Swift and Kotlin into
//!    `build/gen/{swift,kotlin}` (`--no-format`, so the output does not depend on
//!    swiftformat/ktlint being installed). UniFFI 0.32's `--config` only accepts a
//!    *global* config (`[defaults]`, `[crates.<name>]`), so the flat root `uniffi.toml`
//!    is rewritten under `[crates.mesh_ffi.*]` into `build/gen/uniffi.global.toml`.
//! 4. Copies the two sources to their committed homes, or with `--check` fails when
//!    they differ (drift).
//!
//! `build-ios` reuses steps 1-3 for the `mesh_ffiFFI.h` header of the xcframework.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{self, fail};

#[path = "../../../crates/mesh-ffi/support/rt_header.rs"]
pub mod rt_header;

/// Committed Swift bindings (target `MeshCoreBindings` of MeshEngineKit).
pub const SWIFT_OUT: &str = "native/ios/MeshEngineKit/Sources/MeshCoreBindings/mesh_ffi.swift";
/// Committed Kotlin bindings (package `com.convlink.voice.core`).
pub const KOTLIN_OUT: &str =
    "native/android/meshengine/src/main/kotlin/com/convlink/voice/core/mesh_ffi.kt";
/// Generated output root (gitignored `build/`).
pub const GEN_DIR: &str = "build/gen";

/// Entry point for `cargo xtask gen-bindings [--check]`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("gen-bindings", args, &["--check"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage(
            "cargo xtask gen-bindings [--check]".into(),
        ));
    }
    let check = flags.contains(&"--check");
    let root = util::repo_root();
    check_rt_header(&root)?;
    let generated = generate(&root, &[Lang::Swift, Lang::Kotlin])?;
    let pairs = [
        (generated.swift_source(), root.join(SWIFT_OUT)),
        (generated.kotlin_source(), root.join(KOTLIN_OUT)),
    ];
    let mut drift = Vec::new();
    for (src, dst) in &pairs {
        let new = fs::read(src).map_err(|e| fail(format!("cannot read {}: {e}", src.display())))?;
        let old = fs::read(dst).ok();
        if old.as_deref() == Some(new.as_slice()) {
            continue;
        }
        if check {
            drift.push(dst.strip_prefix(&root).unwrap_or(dst).display().to_string());
        } else {
            if let Some(dir) = dst.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(dst, &new)?;
            println!("gen-bindings: wrote {}", dst.display());
        }
    }
    if !drift.is_empty() {
        return Err(fail(format!(
            "generated bindings drifted from the committed files (run `cargo xtask \
             gen-bindings` and commit):\n  {}",
            drift.join("\n  ")
        )));
    }
    println!(
        "gen-bindings: Swift + Kotlin bindings {} (uniffi 0.32.2, mesh_rt.h checked)",
        if check { "up to date" } else { "in place" }
    );
    Ok(())
}

/// Step 1: `mesh_rt.h` vs `abi.rs` (cbindgen as a library).
pub fn check_rt_header(root: &Path) -> util::Result {
    let krate = root.join("crates/mesh-ffi");
    rt_header::check(
        &krate.join("src/rt/abi.rs"),
        &krate.join("include/mesh_rt.h"),
    )
    .map_err(fail)?;
    println!("gen-bindings: include/mesh_rt.h matches src/rt/abi.rs (cbindgen)");
    Ok(())
}

/// A binding language of uniffi-bindgen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Swift,
    Kotlin,
}

impl Lang {
    fn arg(self) -> &'static str {
        match self {
            Lang::Swift => "swift",
            Lang::Kotlin => "kotlin",
        }
    }
}

/// Where `generate` wrote its output.
pub struct Generated {
    pub dir: PathBuf,
}

impl Generated {
    pub fn swift_dir(&self) -> PathBuf {
        self.dir.join("swift")
    }
    /// UniFFI names the Swift file after `module_name`; it is committed as `mesh_ffi.swift`.
    pub fn swift_source(&self) -> PathBuf {
        self.swift_dir().join("MeshCoreBindings.swift")
    }
    /// The UniFFI C header for the `MeshCoreFFI` module.
    pub fn swift_header(&self) -> PathBuf {
        self.swift_dir().join("mesh_ffiFFI.h")
    }
    pub fn kotlin_source(&self) -> PathBuf {
        self.dir.join("kotlin/com/convlink/voice/core/mesh_ffi.kt")
    }
}

/// Cargo target directory (honours `CARGO_TARGET_DIR`).
pub fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("target"))
}

/// The host library uniffi-bindgen reads its metadata from.
pub fn host_library(root: &Path) -> PathBuf {
    let name = if cfg!(target_os = "macos") {
        "libmesh_ffi.dylib"
    } else if cfg!(windows) {
        "mesh_ffi.dll"
    } else {
        "libmesh_ffi.so"
    };
    target_dir(root).join("release").join(name)
}

/// Steps 2-3: host build, then uniffi-bindgen for each language into `build/gen`.
pub fn generate(root: &Path, langs: &[Lang]) -> util::Result<Generated> {
    let mut build = util::cargo();
    build.args(["build", "-p", "mesh-ffi", "--release"]);
    util::run(&mut build, "host build of mesh-ffi")?;
    let lib = host_library(root);
    if !lib.is_file() {
        return Err(fail(format!(
            "host library {} was not produced",
            lib.display()
        )));
    }
    let out = Generated {
        dir: root.join(GEN_DIR),
    };
    fs::create_dir_all(&out.dir)?;
    let flat = fs::read_to_string(root.join("uniffi.toml"))
        .map_err(|e| fail(format!("cannot read uniffi.toml: {e}")))?;
    let global = out.dir.join("uniffi.global.toml");
    fs::write(&global, to_global_config(&flat, "mesh_ffi"))?;
    for &lang in langs {
        let dir = out.dir.join(lang.arg());
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        let mut cmd = util::cargo();
        cmd.args([
            "run",
            "-q",
            "-p",
            "uniffi-bindgen",
            "--",
            "generate",
            "--library",
        ])
        .arg(&lib)
        .args([
            "--crate",
            "mesh_ffi",
            "--language",
            lang.arg(),
            "--no-format",
        ])
        .arg("--config")
        .arg(&global)
        .arg("--out-dir")
        .arg(&dir);
        util::run(&mut cmd, &format!("uniffi-bindgen {}", lang.arg()))?;
    }
    if langs.contains(&Lang::Swift) && !out.swift_source().is_file() {
        return Err(fail(
            "uniffi-bindgen wrote no MeshCoreBindings.swift (check the Swift module_name in uniffi.toml)",
        ));
    }
    if langs.contains(&Lang::Kotlin) && !out.kotlin_source().is_file() {
        return Err(fail(format!(
            "uniffi-bindgen wrote no {} (check the Kotlin package in uniffi.toml)",
            out.kotlin_source().display()
        )));
    }
    Ok(out)
}

/// Rewrites a flat crate config (`[bindings.swift]` …) as a UniFFI 0.32 global config
/// for `krate` (`[crates.<krate>.bindings.swift]` …).
pub fn to_global_config(flat: &str, krate: &str) -> String {
    let mut out = String::from("# Generated by `cargo xtask gen-bindings` from uniffi.toml.\n");
    for line in flat.lines() {
        let t = line.trim_start();
        match t.strip_prefix('[').filter(|r| !r.starts_with('[')) {
            Some(rest) => out.push_str(&format!("[crates.{krate}.{rest}")),
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_paths_follow_the_design() {
        assert!(SWIFT_OUT.ends_with("MeshCoreBindings/mesh_ffi.swift"));
        assert!(KOTLIN_OUT.contains("com/convlink/voice/core/"));
        let g = Generated {
            dir: PathBuf::from("/x"),
        };
        assert_eq!(g.swift_header(), PathBuf::from("/x/swift/mesh_ffiFFI.h"));
        assert!(
            host_library(Path::new("/r")).starts_with("/r")
                || std::env::var_os("CARGO_TARGET_DIR").is_some()
        );
    }

    #[test]
    fn flat_config_becomes_a_crate_section() {
        let g = to_global_config(
            "# c\n[bindings.kotlin]\npackage_name = \"a.b\"\n",
            "mesh_ffi",
        );
        assert!(g.contains("[crates.mesh_ffi.bindings.kotlin]\npackage_name = \"a.b\""));
        assert!(g.contains("# c"));
    }

    #[test]
    fn the_committed_rt_header_matches_abi_rs() {
        check_rt_header(&util::repo_root()).unwrap();
    }
}
