//! `cargo xtask fetch-opus` and `cargo xtask opus-smoke` (owned by WP-06, DESIGN §2.3).
//!
//! `fetch-opus` downloads the libopus 1.6.1 release tarball, verifies its SHA-256 against
//! `third_party/opus/SHA256SUMS`, extracts it to `third_party/opus/opus-1.6.1/`, makes sure
//! the DNN weight sources (`dnn/*_data.c`) are present (downloading the model tarball whose
//! hash upstream's `autogen.sh` pins when they are not) and writes the `.ok` stamp that
//! `crates/opus-sys/build.rs` requires. It is idempotent: a valid stamp makes it a no-op,
//! and it fails closed (no extraction, no stamp) on any hash mismatch.
//!
//! `opus-smoke` cross-builds `opus-sys` (library and test executables, so every symbol
//! is resolved at link time) for aarch64-apple-ios, aarch64-apple-ios-sim,
//! aarch64-linux-android and x86_64-linux-android, then inspects each `libopus.a`
//! (object format, CPU, Apple platform, SIMD objects present, no CMake involved).
//!
//! Downloads use `curl` and extraction uses `tar` (both ship with macOS); hashing is done
//! in-process by the SHA-256 implementation below, so no `shasum` output is trusted.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{self, fail};

/// libopus release fetched and built by `opus-sys`.
pub const OPUS_VERSION: &str = "1.6.1";
/// SHA-256 of the DNN model tarball, as hard-coded by `autogen.sh` at the upstream
/// `v1.6.1` tag (`dnn/download_model.sh "<hash>"`). `SHA256SUMS` must agree.
pub const MODEL_SHA256: &str = "a5177ec6fb7d15058e99e57029746100121f68e4890b1467d4094aa336b6013e";
/// Release tarball location (xiph redirects to an https mirror).
const RELEASE_URL_BASE: &str = "https://downloads.xiph.org/releases/opus";
/// Model tarball location used by upstream `dnn/download_model.sh`.
const MODEL_URL_BASE: &str = "https://media.xiph.org/opus/models";
/// Lists in `lpcnet_sources.mk` that `opus-sys` compiles (DEEP_PLC, DRED, OSCE).
const DNN_LISTS: [&str; 3] = ["DEEP_PLC_SOURCES", "DRED_SOURCES", "OSCE_SOURCES"];

/// Entry point for `cargo xtask fetch-opus`.
pub fn fetch_opus(args: &[String]) -> util::Result {
    let (_flags, positional) = util::parse_flags("fetch-opus", args, &[])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage("cargo xtask fetch-opus".into()));
    }
    let fetcher = Fetcher::production(&util::repo_root())?;
    match fetcher.fetch()? {
        Outcome::UpToDate => println!(
            "fetch-opus: libopus {OPUS_VERSION} already fetched and verified ({})",
            fetcher.stamp_path().display()
        ),
        Outcome::Fetched { model } => println!(
            "fetch-opus: libopus {OPUS_VERSION} verified and extracted to {} (DNN weights: {})",
            fetcher.src_dir().display(),
            model.as_str()
        ),
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), streaming. Kept in-process so the verification does not depend
// on the output format of an external tool.
// ---------------------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Incremental SHA-256.
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    total_bytes: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            filled: 0,
            total_bytes: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let take = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut out = [0u8; 32];
        let (words, _) = out.as_chunks_mut::<4>();
        for (chunk, word) in words.iter_mut().zip(self.state) {
            *chunk = word.to_be_bytes();
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        let (words, _) = block.as_chunks::<4>();
        for (wi, chunk) in w.iter_mut().zip(words) {
            *wi = u32::from_be_bytes(*chunk);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (k, wi) in K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*wi);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// Lower-case hex of a digest.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a byte slice, as lower-case hex.
#[cfg(test)]
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finish())
}

/// SHA-256 of a file, streamed, as lower-case hex.
pub fn sha256_file(path: &Path) -> util::Result<String> {
    let mut file = fs::File::open(path)
        .map_err(|e| fail(format!("cannot open {} for hashing: {e}", path.display())))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finish()))
}

/// True for a 64-character lower-case hex string.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parses a `sha256sum`-style file: `<64 hex>  <name>` (or `<hex> *<name>`) per line;
/// blank lines and `#` comments are ignored. Any other line is an error.
pub fn parse_sha256sums(text: &str) -> util::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let hash = parts.next().unwrap_or_default().to_ascii_lowercase();
        let name = parts
            .next()
            .map(|n| n.trim().trim_start_matches('*'))
            .unwrap_or_default();
        if !is_sha256_hex(&hash) || name.is_empty() || name.contains(char::is_whitespace) {
            return Err(fail(format!(
                "SHA256SUMS line {}: expected `<64 hex digits>  <file name>`, got `{line}`",
                i + 1
            )));
        }
        out.push((name.to_owned(), hash));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Makefile fragment lists (`NAME = \` followed by one path per continuation line).
// ---------------------------------------------------------------------------------------

/// Returns the files of list `name` in an upstream `*_sources.mk` file, or `None` when the
/// list is absent.
pub fn mk_list(mk: &str, name: &str) -> Option<Vec<String>> {
    let mut lines = mk.lines();
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
            let continues = current.trim_end().ends_with('\\');
            files.extend(
                current
                    .trim_end()
                    .trim_end_matches('\\')
                    .split_whitespace()
                    .map(str::to_owned),
            );
            if !continues {
                break;
            }
            match lines.next() {
                Some(next) => current = next.to_owned(),
                None => break,
            }
        }
        return Some(files);
    }
    None
}

// ---------------------------------------------------------------------------------------
// fetch-opus
// ---------------------------------------------------------------------------------------

/// Where the DNN weights came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// `dnn/*_data.c` were already in the release tarball.
    Bundled,
    /// They were missing and the verified model tarball was extracted over the sources.
    Downloaded,
}

impl ModelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelSource::Bundled => "bundled",
            ModelSource::Downloaded => "downloaded",
        }
    }
}

/// Result of a successful `fetch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The stamp was valid; nothing was downloaded or extracted.
    UpToDate,
    /// Sources were (re-)extracted and the stamp written.
    Fetched { model: ModelSource },
}

/// One pinned artifact: file name, download URL and expected SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub name: String,
    pub url: String,
    pub sha256: String,
}

/// Everything `fetch` needs; `production` builds the real one, tests build fakes with
/// `file://` URLs.
#[derive(Debug, Clone)]
pub struct Fetcher {
    /// `third_party/opus`.
    pub dir: PathBuf,
    /// Top-level directory inside the release tarball (`opus-1.6.1`).
    pub src_name: String,
    pub release: Pinned,
    pub model: Pinned,
}

impl Fetcher {
    /// The real configuration: pins read from `third_party/opus/SHA256SUMS`, cross-checked
    /// with `MODEL_SHA256`.
    pub fn production(root: &Path) -> util::Result<Self> {
        let dir = root.join("third_party/opus");
        let sums_path = dir.join("SHA256SUMS");
        let text = fs::read_to_string(&sums_path).map_err(|e| {
            fail(format!(
                "cannot read {} ({e}); it is committed by WP-06 and pins the libopus hashes",
                sums_path.display()
            ))
        })?;
        let sums = parse_sha256sums(&text)?;
        let release_name = format!("opus-{OPUS_VERSION}.tar.gz");
        let model_name = format!("opus_data-{MODEL_SHA256}.tar.gz");
        let lookup = |name: &str| {
            sums.iter()
                .find(|(n, _)| n == name)
                .map(|(_, h)| h.clone())
                .ok_or_else(|| fail(format!("{} has no entry for {name}", sums_path.display())))
        };
        let release_sha = lookup(&release_name)?;
        let model_sha = lookup(&model_name)?;
        if model_sha != MODEL_SHA256 {
            return Err(fail(format!(
                "{} pins {model_name} to {model_sha}, but upstream autogen.sh (MODEL_SHA256 in \
                 tools/xtask/src/opus.rs) pins {MODEL_SHA256}; refusing to continue",
                sums_path.display()
            )));
        }
        Ok(Self {
            dir,
            src_name: format!("opus-{OPUS_VERSION}"),
            release: Pinned {
                url: format!("{RELEASE_URL_BASE}/{release_name}"),
                name: release_name,
                sha256: release_sha,
            },
            model: Pinned {
                url: format!("{MODEL_URL_BASE}/{model_name}"),
                name: model_name,
                sha256: model_sha,
            },
        })
    }

    pub fn src_dir(&self) -> PathBuf {
        self.dir.join(&self.src_name)
    }

    pub fn stamp_path(&self) -> PathBuf {
        self.dir.join(".ok")
    }

    /// Stamp contents for a fetch whose DNN weights came from `model`.
    fn stamp_text(&self, model: ModelSource) -> String {
        format!(
            "# written by `cargo xtask fetch-opus` (tools/xtask/src/opus.rs); delete to re-fetch\n\
             {} {}\n{} {} {}\n",
            self.release.name,
            self.release.sha256,
            self.model.name,
            self.model.sha256,
            model.as_str()
        )
    }

    /// True when the stamp names both pinned hashes and the sources it vouches for exist.
    pub fn stamp_is_valid(&self) -> bool {
        let Ok(text) = fs::read_to_string(self.stamp_path()) else {
            return false;
        };
        let has = |p: &Pinned| {
            text.lines().any(|l| {
                let mut it = l.split_whitespace();
                it.next() == Some(p.name.as_str()) && it.next() == Some(p.sha256.as_str())
            })
        };
        has(&self.release)
            && has(&self.model)
            && self.src_dir().join("include/opus.h").is_file()
            && matches!(self.missing_dnn_data(), Ok(m) if m.is_empty())
    }

    /// `dnn/*_data.c` files named by the DEEP_PLC/DRED/OSCE lists that do not exist.
    pub fn missing_dnn_data(&self) -> util::Result<Vec<String>> {
        let src = self.src_dir();
        let mk_path = src.join("lpcnet_sources.mk");
        let mk = fs::read_to_string(&mk_path)
            .map_err(|e| fail(format!("cannot read {}: {e}", mk_path.display())))?;
        let mut missing = Vec::new();
        for list in DNN_LISTS {
            let files = mk_list(&mk, list)
                .ok_or_else(|| fail(format!("{} has no {list} list", mk_path.display())))?;
            missing.extend(
                files
                    .into_iter()
                    .filter(|f| f.ends_with("_data.c") && !src.join(f).is_file()),
            );
        }
        Ok(missing)
    }

    /// Runs the whole fetch. Never writes the stamp unless every hash matched.
    pub fn fetch(&self) -> util::Result<Outcome> {
        if self.stamp_is_valid() {
            return Ok(Outcome::UpToDate);
        }
        fs::create_dir_all(&self.dir)?;
        remove_if_exists(&self.stamp_path())?;

        let tarball = self.dir.join(&self.release.name);
        ensure_verified(&tarball, &self.release)?;
        self.extract_release(&tarball)?;

        let model = if self.missing_dnn_data()?.is_empty() {
            ModelSource::Bundled
        } else {
            let model_tarball = self.dir.join(&self.model.name);
            ensure_verified(&model_tarball, &self.model)?;
            extract(&model_tarball, &self.src_dir())?;
            let still = self.missing_dnn_data()?;
            if !still.is_empty() {
                return Err(fail(format!(
                    "the verified model tarball {} did not provide: {}",
                    self.model.name,
                    still.join(", ")
                )));
            }
            ModelSource::Downloaded
        };
        fs::write(self.stamp_path(), self.stamp_text(model))?;
        Ok(Outcome::Fetched { model })
    }

    /// Extracts into a scratch directory, then swaps it in place of `src_dir()`.
    fn extract_release(&self, tarball: &Path) -> util::Result {
        let scratch = self.dir.join(".extract");
        remove_if_exists(&scratch)?;
        fs::create_dir_all(&scratch)?;
        extract(tarball, &scratch)?;
        let extracted = scratch.join(&self.src_name);
        if !extracted.join("include/opus.h").is_file() {
            remove_if_exists(&scratch)?;
            return Err(fail(format!(
                "{} does not contain {}/include/opus.h",
                tarball.display(),
                self.src_name
            )));
        }
        let dest = self.src_dir();
        remove_if_exists(&dest)?;
        fs::rename(&extracted, &dest)?;
        remove_if_exists(&scratch)?;
        Ok(())
    }
}

/// Removes a file or directory tree if it exists.
fn remove_if_exists(path: &Path) -> util::Result {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Makes sure `dest` holds the pinned artifact: reuses a cached copy whose hash matches,
/// otherwise downloads to `dest.part`, verifies and renames. A mismatching download is
/// deleted and reported (fail closed).
fn ensure_verified(dest: &Path, pin: &Pinned) -> util::Result {
    if dest.is_file() {
        let have = sha256_file(dest)?;
        if have == pin.sha256 {
            return Ok(());
        }
        eprintln!(
            "fetch-opus: cached {} has SHA-256 {have}, expected {}; downloading again",
            dest.display(),
            pin.sha256
        );
        fs::remove_file(dest)?;
    }
    let part = dest.with_extension("part");
    remove_if_exists(&part)?;
    download(&pin.url, &part)?;
    let got = sha256_file(&part)?;
    if got != pin.sha256 {
        remove_if_exists(&part)?;
        return Err(fail(format!(
            "SHA-256 mismatch for {} from {}: got {got}, expected {} (pinned in \
             third_party/opus/SHA256SUMS). Nothing was extracted and no stamp was written.",
            pin.name, pin.url, pin.sha256
        )));
    }
    fs::rename(&part, dest)?;
    Ok(())
}

/// Downloads `url` to `dest` with curl (https, or file:// for tests).
fn download(url: &str, dest: &Path) -> util::Result {
    eprintln!("fetch-opus: downloading {url}");
    let mut cmd = Command::new("curl");
    cmd.args([
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--retry",
        "3",
        "--proto",
        "=https,file",
        "--proto-redir",
        "=https",
        "--output",
    ])
    .arg(dest)
    .arg(url);
    util::run(&mut cmd, &format!("download of {url}"))
}

/// Extracts a `.tar.gz` into `dir`.
fn extract(tarball: &Path, dir: &Path) -> util::Result {
    let mut cmd = Command::new("tar");
    cmd.arg("-xzf").arg(tarball).arg("-C").arg(dir);
    util::run(&mut cmd, &format!("extraction of {}", tarball.display()))
}

// ---------------------------------------------------------------------------------------
// opus-smoke
// ---------------------------------------------------------------------------------------

/// CPU of a smoke target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cpu {
    Aarch64,
    X86_64,
}

/// How a smoke target is built and what its objects must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Apple target; `platform` is the Mach-O `LC_BUILD_VERSION` platform id.
    Apple { platform: u32 },
    /// Android target built through cargo-ndk (`abi`) or the env.sh CC_/AR_ fallback.
    Android { abi: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmokeTarget {
    pub triple: &'static str,
    pub cpu: Cpu,
    pub platform: Platform,
}

/// Mach-O `PLATFORM_IOS` / `PLATFORM_IOSSIMULATOR`.
const MACHO_PLATFORM_IOS: u32 = 2;
const MACHO_PLATFORM_IOS_SIM: u32 = 7;
/// iOS deployment target (DESIGN §2.3, §10.1).
const IPHONEOS_DEPLOYMENT_TARGET: &str = "26.0";
/// Android API level (minSdk 31, DESIGN §2.3).
const ANDROID_API: &str = "31";

/// The Phase 0 gate targets (DESIGN §2.3 step 6, §14).
pub const SMOKE_TARGETS: [SmokeTarget; 4] = [
    SmokeTarget {
        triple: "aarch64-apple-ios",
        cpu: Cpu::Aarch64,
        platform: Platform::Apple {
            platform: MACHO_PLATFORM_IOS,
        },
    },
    SmokeTarget {
        triple: "aarch64-apple-ios-sim",
        cpu: Cpu::Aarch64,
        platform: Platform::Apple {
            platform: MACHO_PLATFORM_IOS_SIM,
        },
    },
    SmokeTarget {
        triple: "aarch64-linux-android",
        cpu: Cpu::Aarch64,
        platform: Platform::Android { abi: "arm64-v8a" },
    },
    SmokeTarget {
        triple: "x86_64-linux-android",
        cpu: Cpu::X86_64,
        platform: Platform::Android { abi: "x86_64" },
    },
];

/// Entry point for `cargo xtask opus-smoke [--no-ndk]`.
///
/// `--no-ndk` skips cargo-ndk and exercises the documented fallback (plain
/// `cargo build --target` with the `CC_*`/`AR_*`/linker variables of `scripts/env.sh`).
pub fn opus_smoke(args: &[String]) -> util::Result {
    let (flags, positional) = util::parse_flags("opus-smoke", args, &["--no-ndk"])?;
    if !positional.is_empty() {
        return Err(util::Error::Usage(
            "cargo xtask opus-smoke [--no-ndk]".into(),
        ));
    }
    let force_fallback = flags.contains(&"--no-ndk");
    let root = util::repo_root();
    let fetcher = Fetcher::production(&root)?;
    if fetcher.fetch()? != Outcome::UpToDate {
        println!("opus-smoke: fetched libopus {OPUS_VERSION}");
    }

    for t in SMOKE_TARGETS
        .iter()
        .filter(|t| matches!(t.platform, Platform::Apple { .. }))
    {
        for sub in [&["build"][..], &["test", "--no-run"][..]] {
            let mut cmd = util::cargo();
            cmd.args(sub)
                .args(["-p", "opus-sys", "--release", "--target", t.triple]);
            if std::env::var_os("IPHONEOS_DEPLOYMENT_TARGET").is_none() {
                cmd.env("IPHONEOS_DEPLOYMENT_TARGET", IPHONEOS_DEPLOYMENT_TARGET);
            }
            util::run(&mut cmd, &format!("opus-sys {} for {}", sub[0], t.triple))?;
        }
    }

    let android: Vec<&SmokeTarget> = SMOKE_TARGETS
        .iter()
        .filter(|t| matches!(t.platform, Platform::Android { .. }))
        .collect();
    build_android(&android, force_fallback)?;

    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    for t in &SMOKE_TARGETS {
        let out_dir = newest_out_dir(&target_dir, t.triple)?;
        let report = inspect_build(&out_dir, t)?;
        println!("opus-smoke: {:<22} ok  {report}", t.triple);
    }
    println!("opus-smoke: libopus {OPUS_VERSION} builds for all four targets (no CMake)");
    Ok(())
}

/// True when `cargo ndk` can be invoked.
fn cargo_ndk_available() -> bool {
    let mut cmd = util::cargo();
    cmd.args(["ndk", "--version"]);
    util::capture(&mut cmd).map(|c| c.success).unwrap_or(false)
}

/// Builds `opus-sys` (library + linked test executables) for the Android targets.
fn build_android(targets: &[&SmokeTarget], force_fallback: bool) -> util::Result {
    let ndk = std::env::var_os("ANDROID_NDK_HOME").map(PathBuf::from);
    let Some(ndk) = ndk.filter(|p| p.is_dir()) else {
        return Err(fail(
            "ANDROID_NDK_HOME is not set to an existing NDK; run `source scripts/env.sh` first \
             (NDK 27.1.12297006, DESIGN §2.2)",
        ));
    };
    if !force_fallback && cargo_ndk_available() {
        eprintln!(
            "opus-smoke: Android through cargo-ndk (NDK {})",
            ndk.display()
        );
        for sub in [&["build"][..], &["test", "--no-run"][..]] {
            let mut cmd = util::cargo();
            cmd.arg("ndk");
            for t in targets {
                if let Platform::Android { abi } = t.platform {
                    cmd.args(["-t", abi]);
                }
            }
            cmd.args(["-P", ANDROID_API])
                .args(sub)
                .args(["-p", "opus-sys", "--release"]);
            util::run(&mut cmd, &format!("cargo ndk {} opus-sys", sub[0]))?;
        }
        return Ok(());
    }
    eprintln!("opus-smoke: Android through the scripts/env.sh CC_/AR_/linker fallback");
    for t in targets {
        let var = t.triple.replace('-', "_");
        for key in [
            format!("CC_{var}"),
            format!("AR_{var}"),
            format!("CARGO_TARGET_{}_LINKER", var.to_ascii_uppercase()),
        ] {
            let ok = std::env::var_os(&key).is_some_and(|v| Path::new(&v).is_file());
            if !ok {
                return Err(fail(format!(
                    "{key} is not set to an existing file; run `source scripts/env.sh` first"
                )));
            }
        }
        for sub in [&["build"][..], &["test", "--no-run"][..]] {
            let mut cmd = util::cargo();
            cmd.args(sub)
                .args(["-p", "opus-sys", "--release", "--target", t.triple]);
            util::run(&mut cmd, &format!("opus-sys {} for {}", sub[0], t.triple))?;
        }
    }
    Ok(())
}

/// The most recently written `opus-sys-*/out` directory holding `libopus.a` for `triple`.
fn newest_out_dir(target_dir: &Path, triple: &str) -> util::Result<PathBuf> {
    let build = target_dir.join(triple).join("release/build");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in
        fs::read_dir(&build).map_err(|e| fail(format!("cannot list {}: {e}", build.display())))?
    {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("opus-sys-") {
            continue;
        }
        let out = entry.path().join("out");
        let Ok(meta) = fs::metadata(out.join("libopus.a")) else {
            continue;
        };
        let modified = meta.modified()?;
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, out));
        }
    }
    best.map(|(_, p)| p).ok_or_else(|| {
        fail(format!(
            "no opus-sys-*/out/libopus.a under {}",
            build.display()
        ))
    })
}

/// Object basenames every build must contain (float SILK, DEEP_PLC, DRED, OSCE, shim).
const REQUIRED_OBJECTS: [&str; 9] = [
    "opus_encoder",
    "celt_decoder",
    "encode_frame_FLP",
    "lpcnet_plc",
    "fargan",
    "dred_encoder",
    "osce",
    "nolace_data",
    "shim",
];

/// Objects required per CPU (presumed SIMD, DESIGN §2.3) and objects that must be absent
/// (runtime CPU detection, DOTPROD, AVX).
fn simd_expectations(cpu: Cpu) -> (&'static [&'static str], &'static [&'static str]) {
    match cpu {
        Cpu::Aarch64 => (
            &[
                "celt_neon_intr",
                "pitch_neon_intr",
                "NSQ_del_dec_neon_intr",
                "nnet_neon",
            ],
            &["armcpu", "arm_celt_map", "nnet_dotprod", "x86cpu"],
        ),
        Cpu::X86_64 => (
            &["pitch_sse", "pitch_sse2", "vq_sse2", "nnet_sse2"],
            &["x86cpu", "x86_celt_map", "nnet_avx2", "pitch_avx", "armcpu"],
        ),
    }
}

/// One member of a static archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArMember {
    pub name: String,
    pub data: Vec<u8>,
}

/// Parses a System V/GNU (`name/`, `/N` long names in `//`) or BSD (`#1/len`) `ar`
/// archive. Symbol tables (`/`, `__.SYMDEF*`) are skipped.
pub fn parse_ar(bytes: &[u8]) -> util::Result<Vec<ArMember>> {
    const MAGIC: &[u8] = b"!<arch>\n";
    let mut pos = MAGIC.len();
    if bytes.get(..pos) != Some(MAGIC) {
        return Err(fail("not an ar archive"));
    }
    let mut gnu_names: &[u8] = &[];
    let mut members = Vec::new();
    while pos + 60 <= bytes.len() {
        let header = &bytes[pos..pos + 60];
        let field = |a: usize, b: usize| String::from_utf8_lossy(&header[a..b]).trim().to_owned();
        let raw_name = field(0, 16);
        let size: usize = field(48, 58)
            .parse()
            .map_err(|_| fail(format!("bad ar member size at offset {pos}")))?;
        let start = pos + 60;
        let end = start
            .checked_add(size)
            .filter(|e| *e <= bytes.len())
            .ok_or_else(|| fail(format!("truncated ar member at offset {pos}")))?;
        let mut data = &bytes[start..end];
        let name = if let Some(len) = raw_name.strip_prefix("#1/") {
            let len: usize = len
                .parse()
                .map_err(|_| fail(format!("bad BSD name length at offset {pos}")))?;
            let len = len.min(data.len());
            let n = String::from_utf8_lossy(&data[..len])
                .trim_end_matches('\0')
                .to_owned();
            data = &data[len..];
            n
        } else if raw_name == "//" {
            gnu_names = data;
            String::new()
        } else if raw_name == "/" || raw_name == "/SYM64/" {
            String::new()
        } else if let Some(off) = raw_name.strip_prefix('/') {
            let off: usize = off
                .parse()
                .map_err(|_| fail(format!("bad GNU long-name offset at {pos}")))?;
            let rest = gnu_names.get(off..).unwrap_or_default();
            let stop = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..stop])
                .trim_end_matches('/')
                .to_owned()
        } else {
            raw_name.trim_end_matches('/').to_owned()
        };
        if !name.is_empty() && !name.starts_with("__.SYMDEF") {
            members.push(ArMember {
                name,
                data: data.to_vec(),
            });
        }
        pos = end + (end % 2);
    }
    Ok(members)
}

/// Object basename without the cc-rs hash prefix and the `.o` suffix
/// (`0123abcd-celt_neon_intr.o` → `celt_neon_intr`).
fn object_stem(member: &str) -> &str {
    let base = member.rsplit('/').next().unwrap_or(member);
    let base = base.strip_suffix(".o").unwrap_or(base);
    match base.split_once('-') {
        Some((prefix, rest)) if prefix.bytes().all(|b| b.is_ascii_hexdigit()) => rest,
        _ => base,
    }
}

/// Format facts read from one object file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// 64-bit Mach-O: CPU type and `LC_BUILD_VERSION` platform (if present).
    MachO { cputype: u32, platform: Option<u32> },
    /// 64-bit ELF: `e_machine`.
    Elf { machine: u16 },
}

const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const EM_AARCH64: u16 = 183;
const EM_X86_64: u16 = 62;

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Identifies a little-endian 64-bit Mach-O or ELF object.
pub fn object_kind(obj: &[u8]) -> util::Result<ObjectKind> {
    if le_u32(obj, 0) == Some(0xfeed_facf) {
        let cputype = le_u32(obj, 4).unwrap_or_default();
        let ncmds = le_u32(obj, 16).unwrap_or_default();
        let mut at = 32usize;
        let mut platform = None;
        for _ in 0..ncmds {
            let (Some(cmd), Some(size)) = (le_u32(obj, at), le_u32(obj, at + 4)) else {
                break;
            };
            // LC_BUILD_VERSION = 0x32; LC_VERSION_MIN_IPHONEOS = 0x25 (older objects).
            if cmd == 0x32 {
                platform = le_u32(obj, at + 8);
            } else if cmd == 0x25 && platform.is_none() {
                platform = Some(MACHO_PLATFORM_IOS);
            }
            if size == 0 {
                break;
            }
            at += size as usize;
        }
        return Ok(ObjectKind::MachO { cputype, platform });
    }
    if obj.get(..4) == Some(b"\x7fELF") && obj.get(4) == Some(&2) && obj.get(5) == Some(&1) {
        let machine = obj
            .get(18..20)
            .map(|s| u16::from_le_bytes([s[0], s[1]]))
            .unwrap_or_default();
        return Ok(ObjectKind::Elf { machine });
    }
    Err(fail(
        "unrecognised object format (expected 64-bit LE Mach-O or ELF)",
    ))
}

/// Checks one object against the target's expected format.
fn check_object(kind: ObjectKind, t: &SmokeTarget) -> util::Result {
    let ok = match (kind, t.platform, t.cpu) {
        (ObjectKind::MachO { cputype, platform }, Platform::Apple { platform: want }, cpu) => {
            let want_cpu = match cpu {
                Cpu::Aarch64 => CPU_TYPE_ARM64,
                Cpu::X86_64 => CPU_TYPE_X86_64,
            };
            cputype == want_cpu && platform == Some(want)
        }
        (ObjectKind::Elf { machine }, Platform::Android { .. }, cpu) => {
            machine
                == match cpu {
                    Cpu::Aarch64 => EM_AARCH64,
                    Cpu::X86_64 => EM_X86_64,
                }
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(fail(format!(
            "{}: object built as {kind:?}, which does not match the target",
            t.triple
        )))
    }
}

/// Verifies one target's `OUT_DIR`: no CMake artefacts, every required object present,
/// presumed SIMD objects present, runtime-detection objects absent, objects built for
/// the right CPU and Apple platform. Returns a one-line summary.
pub fn inspect_build(out_dir: &Path, t: &SmokeTarget) -> util::Result<String> {
    if out_dir.join("CMakeCache.txt").exists() || out_dir.join("build/CMakeCache.txt").exists() {
        return Err(fail(format!(
            "{}: a CMake build was found in {}",
            t.triple,
            out_dir.display()
        )));
    }
    let lib = out_dir.join("libopus.a");
    let bytes = fs::read(&lib).map_err(|e| fail(format!("cannot read {}: {e}", lib.display())))?;
    let members = parse_ar(&bytes)?;
    let stems: Vec<&str> = members.iter().map(|m| object_stem(&m.name)).collect();
    let (simd, forbidden) = simd_expectations(t.cpu);
    let missing: Vec<&str> = REQUIRED_OBJECTS
        .iter()
        .chain(simd)
        .copied()
        .filter(|want| !stems.contains(want))
        .collect();
    if !missing.is_empty() {
        return Err(fail(format!(
            "{}: {} lacks objects {}",
            t.triple,
            lib.display(),
            missing.join(", ")
        )));
    }
    let unexpected: Vec<&str> = forbidden
        .iter()
        .copied()
        .filter(|bad| stems.contains(bad))
        .collect();
    if !unexpected.is_empty() {
        return Err(fail(format!(
            "{}: {} contains runtime-detection/unsupported objects {}",
            t.triple,
            lib.display(),
            unexpected.join(", ")
        )));
    }
    for m in &members {
        check_object(object_kind(&m.data)?, t)
            .map_err(|e| fail(format!("{} ({}): {e}", lib.display(), m.name)))?;
    }
    Ok(format!(
        "{} objects, {} KiB, SIMD {}",
        members.len(),
        bytes.len() / 1024,
        simd.join("+")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_fips_180_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // One million 'a', fed in uneven chunks to exercise the block buffer.
        let mut h = Sha256::new();
        let chunk = [b'a'; 997];
        let mut left = 1_000_000usize;
        while left > 0 {
            let n = left.min(chunk.len());
            h.update(&chunk[..n]);
            left -= n;
        }
        assert_eq!(
            hex(&h.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn sha256sums_parsing_accepts_standard_lines_and_rejects_garbage() {
        let h = "a".repeat(64);
        let text = format!("# comment\n\n{h}  opus-1.6.1.tar.gz\n{h} *binary.tar.gz\n");
        let sums = parse_sha256sums(&text).expect("parses");
        assert_eq!(sums.len(), 2);
        assert_eq!(sums[0], ("opus-1.6.1.tar.gz".to_owned(), h.clone()));
        assert_eq!(sums[1].0, "binary.tar.gz");
        assert!(parse_sha256sums("deadbeef  short.tar.gz\n").is_err());
        assert!(parse_sha256sums(&format!("{h}\n")).is_err());
        assert!(parse_sha256sums(&format!("{}  x.tar.gz\n", "g".repeat(64))).is_err());
    }

    #[test]
    fn committed_sha256sums_agree_with_the_pins() {
        let f = Fetcher::production(&util::repo_root()).expect("committed SHA256SUMS is valid");
        assert_eq!(f.release.name, "opus-1.6.1.tar.gz");
        assert_eq!(
            f.release.sha256,
            "6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1"
        );
        assert_eq!(f.model.sha256, MODEL_SHA256);
        assert!(f.model.name.contains(MODEL_SHA256));
        assert!(f.release.url.starts_with("https://downloads.xiph.org/"));
        assert!(
            f.model
                .url
                .starts_with("https://media.xiph.org/opus/models/")
        );
    }

    #[test]
    fn mk_lists_follow_line_continuations() {
        let mk = "A = \\\nx/a.c \\\nx/b.c\n\nB = y/c.c\nC =\n";
        assert_eq!(mk_list(mk, "A").unwrap(), ["x/a.c", "x/b.c"]);
        assert_eq!(mk_list(mk, "B").unwrap(), ["y/c.c"]);
        assert!(mk_list(mk, "C").unwrap().is_empty());
        assert!(mk_list(mk, "D").is_none());
    }

    #[test]
    fn object_stems_drop_the_cc_hash_prefix() {
        assert_eq!(
            object_stem("0123abcdef012345-celt_neon_intr.o"),
            "celt_neon_intr"
        );
        assert_eq!(object_stem("shim.o"), "shim");
        assert_eq!(object_stem("dir/9f-nnet_sse2.o"), "nnet_sse2");
    }

    /// Builds an ar archive with BSD and GNU long names plus a symbol table.
    fn ar_bytes(gnu: bool) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        let mut push = |name: &str, data: &[u8]| {
            out.extend(
                format!(
                    "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    0,
                    0,
                    0,
                    644,
                    data.len()
                )
                .as_bytes(),
            );
            out.extend(data);
            if out.len() % 2 == 1 {
                out.push(b'\n');
            }
        };
        let long = "0123456789abcdef-NSQ_del_dec_neon_intr.o";
        if gnu {
            push("/", b"\0\0\0\0");
            let table = format!("{long}/\n");
            push("//", table.as_bytes());
            push("/0", b"OBJ1");
            push("shim.o/", b"OBJ2x");
        } else {
            let mut d = format!("{:\0<20}", "__.SYMDEF SORTED").into_bytes();
            d.extend(b"SYM");
            push("#1/20", &d);
            let mut d = long.as_bytes().to_vec();
            d.extend(b"OBJ1");
            push(&format!("#1/{}", long.len()), &d);
            push("shim.o", b"OBJ2x");
        }
        out
    }

    #[test]
    fn ar_parser_reads_bsd_and_gnu_archives() {
        for gnu in [false, true] {
            let members = parse_ar(&ar_bytes(gnu)).expect("parses");
            let names: Vec<_> = members.iter().map(|m| object_stem(&m.name)).collect();
            assert_eq!(names, ["NSQ_del_dec_neon_intr", "shim"], "gnu={gnu}");
            assert_eq!(members[0].data, b"OBJ1");
            assert_eq!(members[1].data, b"OBJ2x");
        }
        assert!(parse_ar(b"not an archive").is_err());
    }

    #[test]
    fn object_kinds_are_recognised() {
        // Minimal Mach-O header with one LC_BUILD_VERSION (platform 7 = iOS simulator).
        let mut macho = Vec::new();
        for w in [0xfeed_facfu32, CPU_TYPE_ARM64, 0, 1, 1, 24, 0, 0] {
            macho.extend(w.to_le_bytes());
        }
        for w in [0x32u32, 24, MACHO_PLATFORM_IOS_SIM, 0, 0, 0] {
            macho.extend(w.to_le_bytes());
        }
        let kind = object_kind(&macho).expect("mach-o");
        assert_eq!(
            kind,
            ObjectKind::MachO {
                cputype: CPU_TYPE_ARM64,
                platform: Some(MACHO_PLATFORM_IOS_SIM)
            }
        );
        assert!(check_object(kind, &SMOKE_TARGETS[1]).is_ok());
        assert!(
            check_object(kind, &SMOKE_TARGETS[0]).is_err(),
            "device vs simulator"
        );

        let mut elf = vec![0u8; 64];
        elf[..6].copy_from_slice(b"\x7fELF\x02\x01");
        elf[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        let kind = object_kind(&elf).expect("elf");
        assert!(check_object(kind, &SMOKE_TARGETS[3]).is_ok());
        assert!(
            check_object(kind, &SMOKE_TARGETS[2]).is_err(),
            "x86_64 vs aarch64"
        );
        assert!(object_kind(b"garbage!").is_err());
    }
}

/// End-to-end `fetch` behaviour against fake tarballs served over `file://` (needs the
/// `curl` and `tar` that ship with macOS, like the real command).
#[cfg(test)]
mod fetch_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A scratch tree: `serve/` holds the "remote" tarballs, `third_party/` the fetch dir.
    struct Scratch {
        root: PathBuf,
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "xtask-opus-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("serve")).unwrap();
            Self { root }
        }

        /// Writes `files` under `stage/<name>/` and packs `stage/` into `serve/<tarball>`.
        fn tarball(&self, tarball: &str, files: &[(&str, &str)]) -> (String, String) {
            let stage = self.root.join(format!("stage-{tarball}"));
            let _ = fs::remove_dir_all(&stage);
            for (path, body) in files {
                let p = stage.join(path);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, body).unwrap();
            }
            let out = self.root.join("serve").join(tarball);
            let status = Command::new("tar")
                .arg("-czf")
                .arg(&out)
                .arg("-C")
                .arg(&stage)
                .arg(".")
                .status()
                .unwrap();
            assert!(status.success());
            (
                format!("file://{}", out.display()),
                sha256_file(&out).unwrap(),
            )
        }

        fn fetcher(&self, release: (String, String), model: (String, String)) -> Fetcher {
            Fetcher {
                dir: self.root.join("third_party"),
                src_name: "opus-9.9".into(),
                release: Pinned {
                    name: "opus-9.9.tar.gz".into(),
                    url: release.0,
                    sha256: release.1,
                },
                model: Pinned {
                    name: "opus_data-x.tar.gz".into(),
                    url: model.0,
                    sha256: model.1,
                },
            }
        }
    }

    const MK: &str = "DEEP_PLC_SOURCES = \\\ndnn/plc.c \\\ndnn/plc_data.c\n\n\
                      DRED_SOURCES = \\\ndnn/dred_rdovae_enc_data.c\n\nOSCE_SOURCES = \\\ndnn/osce.c\n";

    fn release_files(with_data: bool) -> Vec<(&'static str, &'static str)> {
        let mut files = vec![
            ("opus-9.9/include/opus.h", "/* opus */"),
            ("opus-9.9/lpcnet_sources.mk", MK),
            ("opus-9.9/dnn/plc.c", "int plc;"),
            ("opus-9.9/dnn/osce.c", "int osce;"),
        ];
        if with_data {
            files.push(("opus-9.9/dnn/plc_data.c", "int plc_data;"));
            files.push(("opus-9.9/dnn/dred_rdovae_enc_data.c", "int enc_data;"));
        }
        files
    }

    fn model_files() -> Vec<(&'static str, &'static str)> {
        vec![
            ("dnn/plc_data.c", "int plc_data_from_model;"),
            ("dnn/dred_rdovae_enc_data.c", "int enc_data_from_model;"),
        ]
    }

    #[test]
    fn fetch_is_idempotent_and_works_offline_once_stamped() {
        let s = Scratch::new();
        let release = s.tarball("release.tar.gz", &release_files(true));
        let model = s.tarball("model.tar.gz", &model_files());
        let f = s.fetcher(release, model);
        assert_eq!(
            f.fetch().unwrap(),
            Outcome::Fetched {
                model: ModelSource::Bundled
            }
        );
        assert!(f.stamp_is_valid());
        let stamp = fs::read_to_string(f.stamp_path()).unwrap();
        assert!(stamp.contains(&f.release.sha256) && stamp.contains(&f.model.sha256));
        // Second run: no download at all (the "remote" is gone).
        fs::remove_dir_all(s.root.join("serve")).unwrap();
        assert_eq!(f.fetch().unwrap(), Outcome::UpToDate);
        assert_eq!(f.fetch().unwrap(), Outcome::UpToDate);
    }

    #[test]
    fn fetch_fails_closed_on_release_hash_mismatch() {
        let s = Scratch::new();
        let (url, _) = s.tarball("release.tar.gz", &release_files(true));
        let model = s.tarball("model.tar.gz", &model_files());
        let f = s.fetcher((url, "0".repeat(64)), model);
        let err = f.fetch().expect_err("mismatch must fail");
        assert!(err.to_string().contains("SHA-256 mismatch"), "{err}");
        assert!(!f.stamp_path().exists(), "no stamp on mismatch");
        assert!(!f.src_dir().exists(), "nothing extracted on mismatch");
        assert!(
            !f.dir.join(&f.release.name).exists(),
            "bad download removed"
        );
        assert!(!f.stamp_is_valid());
    }

    #[test]
    fn a_corrupted_cache_is_replaced_and_a_bad_stamp_forces_a_refetch() {
        let s = Scratch::new();
        let release = s.tarball("release.tar.gz", &release_files(true));
        let model = s.tarball("model.tar.gz", &model_files());
        let f = s.fetcher(release, model);
        fs::create_dir_all(&f.dir).unwrap();
        fs::write(f.dir.join(&f.release.name), b"corrupted").unwrap();
        assert!(matches!(f.fetch().unwrap(), Outcome::Fetched { .. }));
        assert_eq!(
            sha256_file(&f.dir.join(&f.release.name)).unwrap(),
            f.release.sha256
        );
        // A stamp naming another hash is not trusted.
        fs::write(f.stamp_path(), "opus-9.9.tar.gz 1234\n").unwrap();
        assert!(!f.stamp_is_valid());
        assert!(matches!(f.fetch().unwrap(), Outcome::Fetched { .. }));
        // Deleting a weight file invalidates the stamp too.
        fs::remove_file(f.src_dir().join("dnn/plc_data.c")).unwrap();
        assert!(!f.stamp_is_valid());
        assert!(matches!(f.fetch().unwrap(), Outcome::Fetched { .. }));
        assert!(f.src_dir().join("dnn/plc_data.c").is_file());
    }

    #[test]
    fn missing_weights_come_from_the_verified_model_tarball() {
        let s = Scratch::new();
        let release = s.tarball("release.tar.gz", &release_files(false));
        let model = s.tarball("model.tar.gz", &model_files());
        let f = s.fetcher(release, model);
        assert_eq!(
            f.fetch().unwrap(),
            Outcome::Fetched {
                model: ModelSource::Downloaded
            }
        );
        let data = fs::read_to_string(f.src_dir().join("dnn/plc_data.c")).unwrap();
        assert!(data.contains("from_model"));
        assert!(
            fs::read_to_string(f.stamp_path())
                .unwrap()
                .contains("downloaded")
        );
        assert_eq!(f.fetch().unwrap(), Outcome::UpToDate);
    }

    #[test]
    fn a_model_hash_mismatch_fails_closed() {
        let s = Scratch::new();
        let release = s.tarball("release.tar.gz", &release_files(false));
        let (url, _) = s.tarball("model.tar.gz", &model_files());
        let f = s.fetcher(release, (url, "f".repeat(64)));
        let err = f.fetch().expect_err("model mismatch must fail");
        assert!(err.to_string().contains("SHA-256 mismatch"), "{err}");
        assert!(!f.stamp_path().exists());
        assert!(!f.src_dir().join("dnn/plc_data.c").exists());
    }
}
