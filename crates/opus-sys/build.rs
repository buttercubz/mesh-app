//! Builds libopus 1.6.1 with the `cc` crate (no CMake, no autotools; DESIGN §2.3).
//!
//! * Sources come from `third_party/opus/opus-1.6.1/`, fetched and SHA-256 verified by
//!   `cargo xtask fetch-opus`; without its `.ok` stamp the build stops and says so.
//! * The file list is parsed from upstream `celt_sources.mk`, `silk_sources.mk`,
//!   `opus_sources.mk` and `lpcnet_sources.mk`, so it tracks upstream.
//! * Float build with DEEP_PLC, DRED and OSCE. aarch64 presumes NEON intrinsics (no
//!   runtime CPU detection, no DOTPROD); x86_64 (Android emulator) presumes SSE/SSE2.
//! * wasm32: nothing is built (the crate is empty there, DESIGN §3.1).
//! * On the host (HOST == TARGET) the upstream `test_opus_api`, `test_opus_decode` and
//!   `test_opus_encode` programs are linked against the same `libopus.a`; their paths are
//!   exported to `tests/upstream.rs` through `cargo:rustc-env`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Directory inside `third_party/opus` (keep in sync with tools/xtask/src/opus.rs).
const OPUS_DIR: &str = "opus-1.6.1";

/// Lists always compiled (float build).
const BASE_LISTS: [(&str, &str); 8] = [
    ("celt_sources.mk", "CELT_SOURCES"),
    ("silk_sources.mk", "SILK_SOURCES"),
    ("silk_sources.mk", "SILK_SOURCES_FLOAT"),
    ("opus_sources.mk", "OPUS_SOURCES"),
    ("opus_sources.mk", "OPUS_SOURCES_FLOAT"),
    ("lpcnet_sources.mk", "DEEP_PLC_SOURCES"),
    ("lpcnet_sources.mk", "DRED_SOURCES"),
    ("lpcnet_sources.mk", "OSCE_SOURCES"),
];

/// Presumed-NEON intrinsics lists (aarch64).
const NEON_LISTS: [(&str, &str); 3] = [
    ("celt_sources.mk", "CELT_SOURCES_ARM_NEON_INTR"),
    ("silk_sources.mk", "SILK_SOURCES_ARM_NEON_INTR"),
    ("lpcnet_sources.mk", "DNN_SOURCES_NEON"),
];

/// Presumed SSE/SSE2 lists (x86_64).
const SSE2_LISTS: [(&str, &str); 3] = [
    ("celt_sources.mk", "CELT_SOURCES_SSE"),
    ("celt_sources.mk", "CELT_SOURCES_SSE2"),
    ("lpcnet_sources.mk", "DNN_SOURCES_SSE2"),
];

/// Defines of every build (DESIGN §2.3). ENABLE_OSCE_BWE matches upstream
/// `--enable-osce`, which always defines it together with ENABLE_OSCE.
const COMMON_DEFINES: [&str; 9] = [
    "OPUS_BUILD",
    "VAR_ARRAYS",
    "HAVE_LRINTF",
    "HAVE_LRINT",
    "ENABLE_DEEP_PLC",
    "ENABLE_DRED",
    "ENABLE_OSCE",
    "ENABLE_OSCE_BWE",
    "DISABLE_DEBUG_FLOAT",
];

/// Upstream test programs run by `cargo test -p opus-sys` (name, sources).
const UPSTREAM_TESTS: [(&str, &[&str]); 3] = [
    ("api", &["tests/test_opus_api.c"]),
    ("decode", &["tests/test_opus_decode.c"]),
    (
        "encode",
        &[
            "tests/test_opus_encode.c",
            "tests/opus_encode_regressions.c",
        ],
    ),
];

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/shim.c");

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if target_arch == "wasm32" {
        return;
    }

    let third_party = manifest_dir.join("../../third_party/opus");
    let stamp = third_party.join(".ok");
    let sums = third_party.join("SHA256SUMS");
    println!("cargo:rerun-if-changed={}", stamp.display());
    println!("cargo:rerun-if-changed={}", sums.display());
    check_stamp(&stamp, &sums);

    let src = third_party.join(OPUS_DIR);
    let mut files = Vec::new();
    for (mk, list) in BASE_LISTS {
        files.extend(read_list(&src, mk, list));
    }
    let mut defines: Vec<&str> = COMMON_DEFINES.to_vec();
    // "." is the source root: upstream headers include e.g. "celt/arm/armcpu.h".
    let mut includes = vec![".", "include", "celt", "silk", "silk/float", "dnn", "src"];
    match target_arch.as_str() {
        "aarch64" => {
            for (mk, list) in NEON_LISTS {
                files.extend(read_list(&src, mk, list));
            }
            defines.extend([
                "OPUS_ARM_MAY_HAVE_NEON_INTR",
                "OPUS_ARM_PRESUME_NEON_INTR",
                "OPUS_ARM_PRESUME_NEON",
            ]);
            // Upstream: "silk arm neon depends on main_Fix.h".
            includes.push("silk/fixed");
        }
        "x86_64" => {
            for (mk, list) in SSE2_LISTS {
                files.extend(read_list(&src, mk, list));
            }
            defines.extend([
                "OPUS_X86_MAY_HAVE_SSE",
                "OPUS_X86_PRESUME_SSE",
                "OPUS_X86_MAY_HAVE_SSE2",
                "OPUS_X86_PRESUME_SSE2",
            ]);
        }
        _ => {}
    }

    let version = package_version(&src);
    let version_define = format!("PACKAGE_VERSION=\"{version}\"");

    let mut build = cc::Build::new();
    for inc in &includes {
        build.include(src.join(inc));
    }
    for d in &defines {
        build.define(d, None);
    }
    // `opus_get_version_string()` reports this ("libopus 1.6.1"); upstream's configure
    // and CMake take it from the `package_version` file of the release tarball.
    build.define("PACKAGE_VERSION", Some(format!("\"{version}\"").as_str()));
    // libopus is unusably slow unoptimised (the upstream tests take minutes at -O0), so
    // debug Rust builds still get at least -O2 for the C code.
    let opt = env::var("OPT_LEVEL").unwrap_or_default();
    if matches!(opt.as_str(), "0" | "1") {
        build.opt_level(2);
    }
    build
        .warnings(false)
        .flag_if_supported("-fno-math-errno")
        .files(files.iter().map(|f| src.join(f)))
        .file(manifest_dir.join("src/shim.c"));
    build.compile("opus");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let needs_libm = matches!(target_os.as_str(), "android" | "linux");
    if needs_libm {
        println!("cargo:rustc-link-lib=m");
    }
    println!("cargo:include={}", src.join("include").display());

    let host = env::var("HOST").unwrap_or_default();
    let target = env::var("TARGET").unwrap_or_default();
    if host == target {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        defines.push(&version_define);
        build_upstream_tests(&build, &src, &includes, &defines, &out_dir, needs_libm);
    }
}

/// Version from the release's `package_version` file (`PACKAGE_VERSION="1.6.1"`).
fn package_version(src: &Path) -> String {
    let path = src.join("package_version");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}; run `cargo xtask fetch-opus`",
            path.display()
        )
    });
    let version = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("PACKAGE_VERSION="))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("no PACKAGE_VERSION in {}", path.display()));
    assert!(
        !version.is_empty()
            && version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "unexpected PACKAGE_VERSION {version:?} in {}",
        path.display()
    );
    assert!(
        OPUS_DIR.ends_with(&version),
        "{} says {version}, but build.rs expects {OPUS_DIR}",
        path.display()
    );
    version
}

/// Stops the build unless `cargo xtask fetch-opus` stamped sources matching every hash
/// pinned in SHA256SUMS.
fn check_stamp(stamp: &Path, sums: &Path) {
    let Ok(stamp_text) = fs::read_to_string(stamp) else {
        panic!(
            "libopus sources are missing ({} not found): run `cargo xtask fetch-opus` from the \
             repository root (DESIGN §2.3)",
            stamp.display()
        );
    };
    let sums_text =
        fs::read_to_string(sums).unwrap_or_else(|e| panic!("cannot read {}: {e}", sums.display()));
    for line in sums_text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let mut it = line.split_whitespace();
        let (Some(hash), Some(name)) = (it.next(), it.next()) else {
            panic!("malformed line in {}: {line}", sums.display());
        };
        let name = name.trim_start_matches('*');
        let stamped = stamp_text.lines().any(|l| {
            let mut s = l.split_whitespace();
            s.next() == Some(name) && s.next() == Some(hash)
        });
        assert!(
            stamped,
            "{} does not vouch for {name} {hash} (pins changed?): run `cargo xtask fetch-opus`",
            stamp.display()
        );
    }
}

/// Files of list `name` in `src/<mk>` (`NAME = \` plus continuation lines).
fn read_list(src: &Path, mk: &str, name: &str) -> Vec<String> {
    let path = src.join(mk);
    println!("cargo:rerun-if-changed={}", path.display());
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}; run `cargo xtask fetch-opus`",
            path.display()
        )
    });
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if lhs.trim() != name {
            continue;
        }
        let mut files = Vec::new();
        let mut current = rhs.to_owned();
        loop {
            let trimmed = current.trim_end();
            let continues = trimmed.ends_with('\\');
            files.extend(
                trimmed
                    .trim_end_matches('\\')
                    .split_whitespace()
                    .map(str::to_owned),
            );
            match (continues, lines.next()) {
                (true, Some(next)) => current = next.to_owned(),
                _ => break,
            }
        }
        assert!(!files.is_empty(), "{name} in {} is empty", path.display());
        for f in &files {
            assert!(
                src.join(f).is_file(),
                "{f} (from {name}) is missing; run `cargo xtask fetch-opus`"
            );
        }
        return files;
    }
    panic!("{name} not found in {}", path.display());
}

/// Links each upstream test program against `libopus.a` in OUT_DIR.
fn build_upstream_tests(
    build: &cc::Build,
    src: &Path,
    includes: &[&str],
    defines: &[&str],
    out_dir: &Path,
    needs_libm: bool,
) {
    let compiler = build.get_compiler();
    for (name, sources) in UPSTREAM_TESTS {
        let exe = out_dir.join(format!("upstream_test_opus_{name}"));
        let mut cmd: Command = compiler.to_command();
        for inc in includes {
            cmd.arg("-I").arg(src.join(inc));
        }
        for d in defines {
            cmd.arg(format!("-D{d}"));
        }
        cmd.arg("-w");
        for s in sources {
            println!("cargo:rerun-if-changed={}", src.join(s).display());
            cmd.arg(src.join(s));
        }
        cmd.arg(out_dir.join("libopus.a"));
        if needs_libm {
            cmd.arg("-lm");
        }
        cmd.arg("-o").arg(&exe);
        let status = cmd
            .status()
            .unwrap_or_else(|e| panic!("cannot run the C compiler for test_opus_{name}: {e}"));
        assert!(
            status.success(),
            "linking upstream test_opus_{name} failed: {cmd:?}"
        );
        println!(
            "cargo:rustc-env=OPUS_SYS_UPSTREAM_TEST_{}={}",
            name.to_ascii_uppercase(),
            exe.display()
        );
    }
}
