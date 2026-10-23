//! `cargo xtask size [--update-baseline]` (owned by WP-01, DESIGN §2.3): the binary
//! size gate. Fails when the iOS device `libmesh_ffi.a` grows more than the allowed
//! percentage over `tools/xtask/size-baseline.json`, or when the stripped arm64
//! Android `libmesh_ffi.so` exceeds the absolute limit. Run after
//! `cargo xtask build-ios` and `cargo xtask build-android` (WP-12).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use crate::util::{self, fail};

const BASELINE: &str = "tools/xtask/size-baseline.json";
const IOS_DEVICE_LIB: &str = "target/aarch64-apple-ios/release/libmesh_ffi.a";
const IOS_SIM_LIB: &str = "target/aarch64-apple-ios-sim/release/libmesh_ffi.a";
const JNI_ARM64: &str = "native/android/meshengine/src/main/jniLibs/arm64-v8a/libmesh_ffi.so";
const CARGO_ARM64: &str = "target/aarch64-linux-android/release/libmesh_ffi.so";
const JNI_X86_64: &str = "native/android/meshengine/src/main/jniLibs/x86_64/libmesh_ffi.so";

/// Limits and the recorded baseline, as stored in `size-baseline.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    /// Recorded size of the iOS device staticlib; `None` until first recorded.
    pub ios_staticlib_bytes: Option<u64>,
    /// Allowed growth over the recorded size, in percent (DESIGN §2.3: 10).
    pub max_ios_growth_percent: u64,
    /// Absolute limit for the stripped arm64 `.so` (DESIGN §2.3: 9 MB).
    pub max_android_arm64_stripped_bytes: u64,
}

impl Baseline {
    fn from_json(v: &Value) -> util::Result<Self> {
        let num = |key: &str| {
            v.get(key).and_then(Value::as_u64).ok_or_else(|| {
                fail(format!(
                    "{BASELINE}: `{key}` must be a non-negative integer"
                ))
            })
        };
        Ok(Self {
            ios_staticlib_bytes: v.get("ios_staticlib_bytes").and_then(Value::as_u64),
            max_ios_growth_percent: num("max_ios_growth_percent")?,
            max_android_arm64_stripped_bytes: num("max_android_arm64_stripped_bytes")?,
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "comment": "Size gate of DESIGN §2.3, read by `cargo xtask size`. Record \
                        ios_staticlib_bytes with `cargo xtask size --update-baseline` \
                        after `cargo xtask build-ios`.",
            "ios_staticlib": IOS_DEVICE_LIB,
            "ios_staticlib_bytes": self.ios_staticlib_bytes,
            "max_ios_growth_percent": self.max_ios_growth_percent,
            "android_arm64_so": JNI_ARM64,
            "max_android_arm64_stripped_bytes": self.max_android_arm64_stripped_bytes,
        })
    }
}

/// Measured sizes that the gate judges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Measured {
    pub ios_staticlib_bytes: u64,
    pub android_arm64_stripped_bytes: u64,
}

/// Returns every gate violation (empty when the gate passes).
pub fn evaluate(baseline: &Baseline, m: &Measured) -> Vec<String> {
    let mut problems = Vec::new();
    match baseline.ios_staticlib_bytes {
        None => problems.push(format!(
            "no iOS baseline recorded in {BASELINE}; run `cargo xtask size --update-baseline` \
             once after the first `cargo xtask build-ios`"
        )),
        Some(base) => {
            // limit = base × (100 + pct) / 100, in integer bytes (u128: no overflow).
            let limit =
                (u128::from(base) * u128::from(100 + baseline.max_ios_growth_percent)) / 100;
            if u128::from(m.ios_staticlib_bytes) > limit {
                problems.push(format!(
                    "iOS libmesh_ffi.a is {} bytes, more than {} % over the {} byte baseline \
                     (limit {limit})",
                    m.ios_staticlib_bytes, baseline.max_ios_growth_percent, base
                ));
            }
        }
    }
    if m.android_arm64_stripped_bytes > baseline.max_android_arm64_stripped_bytes {
        problems.push(format!(
            "stripped arm64 libmesh_ffi.so is {} bytes, over the {} byte limit",
            m.android_arm64_stripped_bytes, baseline.max_android_arm64_stripped_bytes
        ));
    }
    problems
}

fn file_size(path: &Path) -> util::Result<u64> {
    fs::metadata(path).map(|m| m.len()).map_err(|e| {
        fail(format!(
            "{}: {e} (build it first: `cargo xtask build-ios` / `cargo xtask build-android`)",
            path.display()
        ))
    })
}

/// `llvm-strip` from the pinned NDK (scripts/env.sh), else from PATH.
fn llvm_strip() -> PathBuf {
    if let Some(ndk) = std::env::var_os("ANDROID_NDK_HOME") {
        for host in ["darwin-x86_64", "linux-x86_64"] {
            let p = Path::new(&ndk)
                .join("toolchains/llvm/prebuilt")
                .join(host)
                .join("bin/llvm-strip");
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from("llvm-strip")
}

/// Copies the `.so` into target/xtask-size/ and strips the copy (the artifact itself
/// keeps its symbols for target/android-symbols/).
fn stripped_size(root: &Path, so: &Path) -> util::Result<u64> {
    let dir = root.join("target/xtask-size");
    fs::create_dir_all(&dir)?;
    let copy = dir.join("libmesh_ffi.arm64.stripped.so");
    fs::copy(so, &copy).map_err(|e| fail(format!("copy {}: {e}", so.display())))?;
    util::run(
        Command::new(llvm_strip()).arg("--strip-all").arg(&copy),
        "llvm-strip (source scripts/env.sh so the NDK llvm-strip is found)",
    )?;
    file_size(&copy)
}

fn first_existing(root: &Path, candidates: &[&str]) -> PathBuf {
    candidates
        .iter()
        .map(|c| root.join(c))
        .find(|p| p.is_file())
        .unwrap_or_else(|| root.join(candidates[0]))
}

/// Entry point for `cargo xtask size`.
pub fn run(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("size", args, &["--update-baseline"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage(
            "cargo xtask size [--update-baseline]".into(),
        ));
    }
    let root = util::repo_root();
    let baseline_path = root.join(BASELINE);
    let text = fs::read_to_string(&baseline_path)
        .map_err(|e| fail(format!("{}: {e}", baseline_path.display())))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| fail(format!("{}: invalid JSON: {e}", baseline_path.display())))?;
    let mut baseline = Baseline::from_json(&value)?;

    let ios = file_size(&root.join(IOS_DEVICE_LIB))?;
    eprintln!("xtask size: {IOS_DEVICE_LIB}: {ios} bytes");
    if let Ok(sim) = file_size(&root.join(IOS_SIM_LIB)) {
        eprintln!("xtask size: {IOS_SIM_LIB}: {sim} bytes (informational)");
    }

    if flags.contains(&"--update-baseline") {
        baseline.ios_staticlib_bytes = Some(ios);
        let mut out = serde_json::to_string_pretty(&baseline.to_json())
            .map_err(|e| fail(format!("serialize baseline: {e}")))?;
        out.push('\n');
        fs::write(&baseline_path, out)?;
        eprintln!("xtask size: recorded iOS baseline {ios} bytes in {BASELINE}");
        return Ok(());
    }

    let so = first_existing(&root, &[JNI_ARM64, CARGO_ARM64]);
    let so_stripped = stripped_size(&root, &so)?;
    eprintln!("xtask size: {} stripped: {so_stripped} bytes", so.display());
    if let Ok(x86) = file_size(&root.join(JNI_X86_64)) {
        eprintln!("xtask size: {JNI_X86_64}: {x86} bytes unstripped (informational)");
    }

    let problems = evaluate(
        &baseline,
        &Measured {
            ios_staticlib_bytes: ios,
            android_arm64_stripped_bytes: so_stripped,
        },
    );
    if problems.is_empty() {
        eprintln!("xtask size: gate passed");
        Ok(())
    } else {
        Err(fail(format!(
            "size gate failed (DESIGN §2.3):\n  {}",
            problems.join("\n  ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(ios: Option<u64>) -> Baseline {
        Baseline {
            ios_staticlib_bytes: ios,
            max_ios_growth_percent: 10,
            max_android_arm64_stripped_bytes: 9_000_000,
        }
    }

    #[test]
    fn growth_up_to_ten_percent_passes() {
        let m = Measured {
            ios_staticlib_bytes: 1_100,
            android_arm64_stripped_bytes: 9_000_000,
        };
        assert!(evaluate(&baseline(Some(1_000)), &m).is_empty());
    }

    #[test]
    fn growth_over_ten_percent_and_oversize_so_fail() {
        let m = Measured {
            ios_staticlib_bytes: 1_101,
            android_arm64_stripped_bytes: 9_000_001,
        };
        let problems = evaluate(&baseline(Some(1_000)), &m);
        assert_eq!(problems.len(), 2, "{problems:#?}");
        assert!(problems[0].contains("iOS"));
        assert!(problems[1].contains("arm64"));
    }

    #[test]
    fn missing_baseline_fails_with_the_fix() {
        let m = Measured {
            ios_staticlib_bytes: 1,
            android_arm64_stripped_bytes: 1,
        };
        let problems = evaluate(&baseline(None), &m);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("--update-baseline"));
    }

    #[test]
    fn committed_baseline_file_parses_and_round_trips() {
        let text = fs::read_to_string(util::repo_root().join(BASELINE)).expect("baseline file");
        let parsed =
            Baseline::from_json(&serde_json::from_str(&text).expect("json")).expect("fields");
        assert_eq!(parsed.max_ios_growth_percent, 10);
        assert_eq!(parsed.max_android_arm64_stripped_bytes, 9_000_000);
        assert_eq!(Baseline::from_json(&parsed.to_json()), Ok(parsed));
    }
}
