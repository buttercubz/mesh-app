//! `cargo xtask build-ios` (owned by WP-12, DESIGN §2.3): builds
//! `native/ios/MeshEngineKit/Frameworks/MeshCoreFFI.xcframework` (gitignored).
//!
//! 1. Checks `mesh_rt.h` against `abi.rs` (cbindgen).
//! 2. `cargo build -p mesh-ffi --release` for `aarch64-apple-ios` and
//!    `aarch64-apple-ios-sim` (`IPHONEOS_DEPLOYMENT_TARGET=26.0`) → `libmesh_ffi.a`.
//! 3. Host build + uniffi-bindgen Swift for `mesh_ffiFFI.h`; warns when the committed
//!    Swift bindings drifted.
//! 4. Headers in `build/gen/include`: `mesh_ffiFFI.h`, `mesh_rt.h` and
//!    `module.modulemap` (`module MeshCoreFFI { header "mesh_ffiFFI.h" header "mesh_rt.h"
//!    export * }`).
//! 5. Per slice: a symbol check (UniFFI scaffolding and the five `mesh_rt_*` symbols)
//!    and a link probe (a small C program linked by Xcode's clang/ld against the slice),
//!    then `xcodebuild -create-xcframework` with both slices.
//! 6. `swiftc -typecheck` of the committed Swift bindings against the new headers.
//!
//! Toolchain skew: Rust 1.99.1 ships LLVM 23, newer than Xcode's LLVM. The sysroot's
//! `compiler_builtins` objects carry an embedded `__LLVM,__bitcode` section that Xcode's
//! `nm` cannot parse (`Unknown attribute kind`), so the symbol check runs `nm
//! --no-llvm-bc`. The linker ignores those sections (bitcode is deprecated since
//! Xcode 14); the link probe proves it on every build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::bindings::{self, Lang};
use crate::util::{self, fail};

/// iOS deployment target (DESIGN §2.6).
pub const IPHONEOS_DEPLOYMENT_TARGET: &str = "26.0";
/// Device and simulator triples.
pub const IOS_TARGETS: [&str; 2] = ["aarch64-apple-ios", "aarch64-apple-ios-sim"];
/// Output xcframework (SwiftPM binary targets must live under the package root).
pub const XCFRAMEWORK: &str = "native/ios/MeshEngineKit/Frameworks/MeshCoreFFI.xcframework";
/// Header directory handed to `-create-xcframework`.
pub const INCLUDE_DIR: &str = "build/gen/include";
/// Clang module map of the `MeshCoreFFI` module.
pub const MODULEMAP: &str = "module MeshCoreFFI {\n    header \"mesh_ffiFFI.h\"\n    header \"mesh_rt.h\"\n    export *\n}\n";
/// Symbols every slice must export.
pub const REQUIRED_SYMBOLS: [&str; 7] = [
    "_mesh_rt_capture",
    "_mesh_rt_render",
    "_mesh_rt_set_flags",
    "_mesh_rt_report",
    "_mesh_rt_stats",
    "_uniffi_mesh_ffi_fn_constructor_meshengine_new",
    "_uniffi_mesh_ffi_fn_func_ble_constants",
];

/// Entry point for `cargo xtask build-ios`.
pub fn run(args: &[String]) -> util::Result {
    let (_, positional) = util::parse_flags("build-ios", args, &[])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask build-ios".into()));
    }
    let root = util::repo_root();
    bindings::check_rt_header(&root)?;

    for triple in IOS_TARGETS {
        let mut cmd = util::cargo();
        cmd.args(["build", "-p", "mesh-ffi", "--release", "--target", triple]);
        if std::env::var_os("IPHONEOS_DEPLOYMENT_TARGET").is_none() {
            cmd.env("IPHONEOS_DEPLOYMENT_TARGET", IPHONEOS_DEPLOYMENT_TARGET);
        }
        util::run(&mut cmd, &format!("mesh-ffi for {triple}"))?;
    }

    let generated = bindings::generate(&root, &[Lang::Swift])?;
    let committed = fs::read(root.join(bindings::SWIFT_OUT)).ok();
    if committed != fs::read(generated.swift_source()).ok() {
        eprintln!(
            "build-ios: warning: {} differs from the generated bindings; run `cargo xtask \
             gen-bindings`",
            bindings::SWIFT_OUT
        );
    }

    let include = root.join(INCLUDE_DIR);
    if include.exists() {
        fs::remove_dir_all(&include)?;
    }
    fs::create_dir_all(&include)?;
    fs::copy(generated.swift_header(), include.join("mesh_ffiFFI.h"))?;
    fs::copy(
        root.join("crates/mesh-ffi/include/mesh_rt.h"),
        include.join("mesh_rt.h"),
    )?;
    fs::write(include.join("module.modulemap"), MODULEMAP)?;

    let target = bindings::target_dir(&root);
    let libs: Vec<PathBuf> = IOS_TARGETS
        .iter()
        .map(|t| target.join(t).join("release/libmesh_ffi.a"))
        .collect();
    for (lib, slice) in libs.iter().zip(SLICES) {
        check_symbols(lib)?;
        link_probe(&root, lib, &slice)?;
    }

    let out = root.join(XCFRAMEWORK);
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-create-xcframework");
    for lib in &libs {
        cmd.arg("-library").arg(lib).arg("-headers").arg(&include);
    }
    cmd.arg("-output").arg(&out);
    util::run(&mut cmd, "xcodebuild -create-xcframework")?;
    verify_xcframework(&out)?;
    typecheck_bindings(&root)?;
    println!("build-ios: {} (device + simulator)", out.display());
    Ok(())
}

/// An xcframework slice: SDK and clang target of the link probe.
pub struct Slice {
    pub sdk: &'static str,
    pub clang_target: &'static str,
}

/// Device and simulator slices, in `IOS_TARGETS` order.
pub const SLICES: [Slice; 2] = [
    Slice {
        sdk: "iphoneos",
        clang_target: "arm64-apple-ios26.0",
    },
    Slice {
        sdk: "iphonesimulator",
        clang_target: "arm64-apple-ios26.0-simulator",
    },
];

/// C program of the link probe: every RT symbol plus UniFFI entry points.
pub const LINK_PROBE_C: &str = r#"/* Generated by `cargo xtask build-ios`: link probe for libmesh_ffi.a. */
#include <stdint.h>
#include "mesh_rt.h"
#include "mesh_ffiFFI.h"
int main(void) {
    MeshRtStats s;
    int16_t pcm[160] = {0};
    RustCallStatus status = {0};
    mesh_rt_set_flags(MESH_RT_FLAG_PTT_DOWN);
    mesh_rt_capture(pcm, 160, 0);
    mesh_rt_render(pcm, 160, 0);
    mesh_rt_report(0);
    mesh_rt_stats(&s);
    return (int)uniffi_mesh_ffi_fn_func_api_version(&status) + (int)s.generation
        + (int)uniffi_mesh_ffi_checksum_constructor_meshengine_new();
}
"#;

/// Links [`LINK_PROBE_C`] against `lib` with Xcode's clang and linker: proves the
/// archive (built by Rust's newer LLVM) is accepted by Apple's toolchain.
fn link_probe(root: &Path, lib: &Path, slice: &Slice) -> util::Result {
    let dir = root.join("build/gen/link-probe").join(slice.sdk);
    fs::create_dir_all(&dir)?;
    let src = dir.join("probe.c");
    fs::write(&src, LINK_PROBE_C)?;
    let mut cc = Command::new("xcrun");
    cc.args(["-sdk", slice.sdk, "clang", "-target", slice.clang_target])
        .arg("-I")
        .arg(root.join(INCLUDE_DIR))
        .arg(&src)
        .arg(lib)
        .arg("-o")
        .arg(dir.join("probe"));
    util::run(&mut cc, &format!("link probe ({})", slice.sdk))
}

/// Type-checks the committed Swift bindings (Swift 5 language mode, as the
/// `MeshCoreBindings` target of MeshEngineKit) against the `MeshCoreFFI` module map.
fn typecheck_bindings(root: &Path) -> util::Result {
    let sim = &SLICES[1];
    let mut swiftc = Command::new("xcrun");
    swiftc
        .args([
            "-sdk",
            sim.sdk,
            "swiftc",
            "-typecheck",
            "-swift-version",
            "5",
        ])
        .args(["-target", sim.clang_target])
        .arg("-I")
        .arg(root.join(INCLUDE_DIR))
        .arg(root.join(bindings::SWIFT_OUT));
    util::run(&mut swiftc, "swiftc -typecheck MeshCoreBindings")
}

/// Fails unless `lib` exports every required symbol.
fn check_symbols(lib: &Path) -> util::Result {
    if !lib.is_file() {
        return Err(fail(format!("{} was not produced", lib.display())));
    }
    let mut nm = Command::new("nm");
    nm.args(["-gU", "-j", "--no-llvm-bc"]).arg(lib);
    let out = util::capture(&mut nm)?;
    if !out.success {
        return Err(fail(format!(
            "nm failed on {}: {}",
            lib.display(),
            out.stderr
        )));
    }
    let names: std::collections::BTreeSet<&str> = out.stdout.lines().map(str::trim).collect();
    let missing: Vec<&str> = REQUIRED_SYMBOLS
        .iter()
        .copied()
        .filter(|s| !names.contains(s))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(fail(format!(
            "{} does not export {}",
            lib.display(),
            missing.join(", ")
        )))
    }
}

/// Checks the xcframework layout: two library slices, each with the module map and
/// both headers.
fn verify_xcframework(out: &Path) -> util::Result {
    let plist = out.join("Info.plist");
    let text = fs::read_to_string(&plist)
        .map_err(|e| fail(format!("cannot read {}: {e}", plist.display())))?;
    for needle in ["ios-arm64", "ios-arm64-simulator", "libmesh_ffi.a"] {
        if !text.contains(needle) {
            return Err(fail(format!("{} lacks `{needle}`", plist.display())));
        }
    }
    for slice in ["ios-arm64", "ios-arm64-simulator"] {
        for f in [
            "libmesh_ffi.a",
            "Headers/module.modulemap",
            "Headers/mesh_ffiFFI.h",
            "Headers/mesh_rt.h",
        ] {
            let p = out.join(slice).join(f);
            if !p.is_file() {
                return Err(fail(format!("xcframework is missing {}", p.display())));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulemap_exposes_both_headers_as_meshcoreffi() {
        assert!(MODULEMAP.starts_with("module MeshCoreFFI {"));
        assert!(MODULEMAP.contains("header \"mesh_ffiFFI.h\""));
        assert!(MODULEMAP.contains("header \"mesh_rt.h\""));
        assert!(MODULEMAP.contains("export *"));
        assert!(XCFRAMEWORK.starts_with("native/ios/MeshEngineKit/Frameworks/"));
    }

    #[test]
    fn link_probe_covers_every_rt_symbol_and_both_slices() {
        for sym in REQUIRED_SYMBOLS
            .iter()
            .filter(|s| s.starts_with("_mesh_rt_"))
        {
            assert!(LINK_PROBE_C.contains(&format!("{}(", &sym[1..])), "{sym}");
        }
        assert_eq!(SLICES.len(), IOS_TARGETS.len());
        assert!(SLICES[1].clang_target.ends_with("-simulator"));
        assert!(IOS_TARGETS[1].ends_with("-sim"));
    }
}
