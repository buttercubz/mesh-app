//! `cargo xtask build-android [--no-ndk]` (owned by WP-12, DESIGN §2.3): builds
//! `libmesh_ffi.so` for arm64-v8a and x86_64 (API 31) into
//! `native/android/meshengine/src/main/jniLibs/<abi>/` (gitignored).
//!
//! Default path: `cargo ndk -t arm64-v8a -t x86_64 -P 31 -o <jniLibs> build -p mesh-ffi
//! --release`. Fallback (cargo-ndk missing, or `--no-ndk`): plain `cargo build --target`
//! with the `CC_*`/`AR_*`/linker variables of `scripts/env.sh`, then a copy into jniLibs
//! stripped with the NDK `llvm-strip`. Both paths link with 16 KB page alignment
//! (Android 15+ devices, Play requirement) and keep the unstripped libraries in
//! `target/android-symbols/<abi>/`. Each `.so` is then checked: ELF machine, PT_LOAD
//! alignment and the exported `mesh_rt_*` / UniFFI symbols.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::bindings;
use crate::util::{self, fail};

/// Android API level (minSdk 31).
pub const ANDROID_API: &str = "31";
/// Output directory of the `.so` files.
pub const JNI_LIBS: &str = "native/android/meshengine/src/main/jniLibs";
/// Page size every PT_LOAD segment must be aligned to.
pub const PAGE_ALIGN: u64 = 16 * 1024;

/// One Android ABI.
pub struct Abi {
    pub abi: &'static str,
    pub triple: &'static str,
    /// ELF `e_machine`.
    pub machine: u16,
}

/// arm64-v8a (devices) and x86_64 (emulator); armeabi-v7a is omitted (minSdk 31).
pub const ABIS: [Abi; 2] = [
    Abi {
        abi: "arm64-v8a",
        triple: "aarch64-linux-android",
        machine: 183,
    },
    Abi {
        abi: "x86_64",
        triple: "x86_64-linux-android",
        machine: 62,
    },
];

/// Dynamic symbols every `.so` must export.
const REQUIRED_SYMBOLS: [&str; 6] = [
    "mesh_rt_capture",
    "mesh_rt_render",
    "mesh_rt_set_flags",
    "mesh_rt_stats",
    "uniffi_mesh_ffi_fn_constructor_meshengine_new",
    "uniffi_mesh_ffi_fn_func_api_version",
];

/// Entry point for `cargo xtask build-android [--no-ndk]`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("build-android", args, &["--no-ndk"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage(
            "cargo xtask build-android [--no-ndk]".into(),
        ));
    }
    let root = util::repo_root();
    let ndk = std::env::var_os("ANDROID_NDK_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            fail(
                "ANDROID_NDK_HOME is not set to an existing NDK; run `source scripts/env.sh` \
                 first (NDK 27.1.12297006, DESIGN §2.2)",
            )
        })?;
    let bin = ndk.join("toolchains/llvm/prebuilt/darwin-x86_64/bin");
    let jni = root.join(JNI_LIBS);
    let target = bindings::target_dir(&root);
    let use_ndk = !flags.contains(&"--no-ndk") && cargo_ndk_available();
    if use_ndk {
        let mut cmd = util::cargo();
        cmd.arg("ndk");
        for a in &ABIS {
            cmd.args(["-t", a.abi]);
        }
        cmd.args(["-P", ANDROID_API, "-o"]).arg(&jni).args([
            "build",
            "-p",
            "mesh-ffi",
            "--release",
        ]);
        page_size_flags(&mut cmd);
        util::run(&mut cmd, "cargo ndk build mesh-ffi")?;
    } else {
        eprintln!("build-android: cargo-ndk not used; NDK-linker fallback (scripts/env.sh)");
        for a in &ABIS {
            fallback_build(a)?;
            let dst = jni.join(a.abi).join("libmesh_ffi.so");
            fs::create_dir_all(jni.join(a.abi))?;
            let src = target.join(a.triple).join("release/libmesh_ffi.so");
            let mut strip = Command::new(bin.join("llvm-strip"));
            strip.arg("--strip-debug").arg(&src).arg("-o").arg(&dst);
            util::run(&mut strip, &format!("llvm-strip {}", a.abi))?;
        }
    }
    for a in &ABIS {
        let unstripped = target.join(a.triple).join("release/libmesh_ffi.so");
        let symbols = target.join("android-symbols").join(a.abi);
        fs::create_dir_all(&symbols)?;
        fs::copy(&unstripped, symbols.join("libmesh_ffi.so")).map_err(|e| {
            fail(format!(
                "cannot keep symbols of {}: {e}",
                unstripped.display()
            ))
        })?;
        let so = jni.join(a.abi).join("libmesh_ffi.so");
        verify_so(&so, a, &bin)?;
        println!("build-android: {} ok", so.display());
    }
    Ok(())
}

/// True when `cargo ndk` can be invoked.
fn cargo_ndk_available() -> bool {
    let mut cmd = util::cargo();
    cmd.args(["ndk", "--version"]);
    util::capture(&mut cmd).map(|c| c.success).unwrap_or(false)
}

/// 16 KB ELF segment alignment for both Android targets (NDK r27 defaults to 4 KB).
fn page_size_flags(cmd: &mut Command) {
    for a in &ABIS {
        let key = format!(
            "CARGO_TARGET_{}_RUSTFLAGS",
            a.triple.replace('-', "_").to_ascii_uppercase()
        );
        cmd.env(
            key,
            format!("-C link-arg=-Wl,-z,max-page-size={PAGE_ALIGN}"),
        );
    }
}

/// `cargo build --target` with the scripts/env.sh cross variables.
fn fallback_build(a: &Abi) -> util::Result {
    let var = a.triple.replace('-', "_");
    for key in [
        format!("CC_{var}"),
        format!("AR_{var}"),
        format!("CARGO_TARGET_{}_LINKER", var.to_ascii_uppercase()),
    ] {
        if !std::env::var_os(&key).is_some_and(|v| Path::new(&v).is_file()) {
            return Err(fail(format!(
                "{key} is not set to an existing file; run `source scripts/env.sh` first"
            )));
        }
    }
    let mut cmd = util::cargo();
    cmd.args(["build", "-p", "mesh-ffi", "--release", "--target", a.triple]);
    page_size_flags(&mut cmd);
    util::run(&mut cmd, &format!("mesh-ffi for {}", a.triple))
}

/// ELF checks: class 64, machine, every PT_LOAD aligned to 16 KB; exported symbols.
fn verify_so(so: &Path, a: &Abi, ndk_bin: &Path) -> util::Result {
    let bytes = fs::read(so).map_err(|e| fail(format!("cannot read {}: {e}", so.display())))?;
    let elf = Elf64::parse(&bytes).map_err(|e| fail(format!("{}: {e}", so.display())))?;
    if elf.machine != a.machine {
        return Err(fail(format!(
            "{} has ELF machine {} (expected {} for {})",
            so.display(),
            elf.machine,
            a.machine,
            a.abi
        )));
    }
    if let Some(bad) = elf.load_aligns.iter().find(|&&al| al < PAGE_ALIGN) {
        return Err(fail(format!(
            "{} has a PT_LOAD aligned to {bad} bytes (need {PAGE_ALIGN})",
            so.display()
        )));
    }
    let mut nm = Command::new(ndk_bin.join("llvm-nm"));
    nm.args(["-D", "--defined-only", "-j"]).arg(so);
    let out = util::capture(&mut nm)?;
    if !out.success {
        return Err(fail(format!(
            "llvm-nm failed on {}: {}",
            so.display(),
            out.stderr
        )));
    }
    let names: std::collections::BTreeSet<&str> = out.stdout.lines().map(str::trim).collect();
    let missing: Vec<&str> = REQUIRED_SYMBOLS
        .iter()
        .copied()
        .filter(|s| !names.contains(s))
        .collect();
    if !missing.is_empty() {
        return Err(fail(format!(
            "{} does not export {}",
            so.display(),
            missing.join(", ")
        )));
    }
    Ok(())
}

/// The parts of a little-endian ELF64 header the checks need.
#[derive(Debug, PartialEq, Eq)]
struct Elf64 {
    machine: u16,
    load_aligns: Vec<u64>,
}

impl Elf64 {
    fn parse(b: &[u8]) -> Result<Self, String> {
        let u16_at = |o: usize| -> Result<u16, String> {
            b.get(o..o + 2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]))
                .ok_or_else(|| "truncated ELF".to_owned())
        };
        let u32_at = |o: usize| -> Result<u32, String> {
            b.get(o..o + 4)
                .and_then(|s| s.try_into().ok())
                .map(u32::from_le_bytes)
                .ok_or_else(|| "truncated ELF".to_owned())
        };
        let u64_at = |o: usize| -> Result<u64, String> {
            b.get(o..o + 8)
                .and_then(|s| s.try_into().ok())
                .map(u64::from_le_bytes)
                .ok_or_else(|| "truncated ELF".to_owned())
        };
        if b.get(0..4) != Some(b"\x7fELF".as_slice()) {
            return Err("not an ELF file".into());
        }
        if b.get(4) != Some(&2) || b.get(5) != Some(&1) {
            return Err("not a little-endian ELF64 file".into());
        }
        let machine = u16_at(18)?;
        let phoff = usize::try_from(u64_at(32)?).map_err(|e| e.to_string())?;
        let phentsize = usize::from(u16_at(54)?);
        let phnum = usize::from(u16_at(56)?);
        let mut load_aligns = Vec::new();
        for i in 0..phnum {
            let ph = phoff + i * phentsize;
            if u32_at(ph)? == 1 {
                load_aligns.push(u64_at(ph + 48)?);
            }
        }
        Ok(Elf64 {
            machine,
            load_aligns,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF64 header with one PT_LOAD of the given alignment.
    fn elf(machine: u16, align: u64) -> Vec<u8> {
        let mut b = vec![0u8; 64 + 56];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[18..20].copy_from_slice(&machine.to_le_bytes());
        b[32..40].copy_from_slice(&64u64.to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        b[56..58].copy_from_slice(&1u16.to_le_bytes());
        b[64..68].copy_from_slice(&1u32.to_le_bytes());
        b[64 + 48..64 + 56].copy_from_slice(&align.to_le_bytes());
        b
    }

    #[test]
    fn elf_header_parsing_finds_machine_and_load_alignment() {
        let e = Elf64::parse(&elf(183, 16384)).unwrap();
        assert_eq!(e.machine, 183);
        assert_eq!(e.load_aligns, vec![16384]);
        assert!(Elf64::parse(b"nope").is_err());
        assert!(Elf64::parse(&elf(62, 4096)[..70]).is_err());
    }

    #[test]
    fn abis_are_the_two_64_bit_targets() {
        let names: Vec<_> = ABIS.iter().map(|a| a.abi).collect();
        assert_eq!(names, ["arm64-v8a", "x86_64"]);
        assert_eq!(JNI_LIBS, "native/android/meshengine/src/main/jniLibs");
    }
}
