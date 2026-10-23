//! Contract C5: `include/mesh_rt.h` matches the Rust exports (cbindgen as a library) and
//! the C compiler agrees with Rust on the `MeshRtStats` layout and every prototype.

#[path = "../../support/rt_header.rs"]
mod rt_header;

use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};
use std::process::Command;

use mesh_ffi::rt::MeshRtStats;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn header_matches_cbindgen_output_of_abi_rs() {
    let dir = crate_dir();
    rt_header::check(&dir.join("src/rt/abi.rs"), &dir.join("include/mesh_rt.h"))
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn a_drifted_header_is_rejected() {
    let dir = crate_dir();
    let header = std::fs::read_to_string(dir.join("include/mesh_rt.h")).unwrap();
    let tmp = std::env::temp_dir().join(format!("mesh_rt_drift_{}.h", std::process::id()));
    std::fs::write(
        &tmp,
        header.replace("uint32_t frames, uint64_t", "uint16_t frames, uint64_t"),
    )
    .unwrap();
    let err = rt_header::check(&dir.join("src/rt/abi.rs"), &tmp).unwrap_err();
    assert!(err.contains("mesh_rt_capture"), "{err}");
    std::fs::write(&tmp, header.replace("uint32_t generation;", "")).unwrap();
    let err = rt_header::check(&dir.join("src/rt/abi.rs"), &tmp).unwrap_err();
    assert!(err.contains("MeshRtStats"), "{err}");
    let _ = std::fs::remove_file(&tmp);
}

/// Compiles a C program against the header: prototypes are assigned to function
/// pointers of the exact types (any mismatch is an error under -Werror) and the struct
/// layout is printed for comparison with Rust.
#[test]
fn c_compiler_agrees_on_prototypes_and_layout() {
    let dir = crate_dir();
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let c = out_dir.join("rt_layout.c");
    let exe = out_dir.join("rt_layout");
    std::fs::write(
        &c,
        r#"#include <stdio.h>
#include <stddef.h>
#include "mesh_rt.h"
int32_t (*p_capture)(const int16_t *, uint32_t, uint64_t) = mesh_rt_capture;
int32_t (*p_render)(int16_t *, uint32_t, uint64_t) = mesh_rt_render;
void (*p_flags)(uint32_t) = mesh_rt_set_flags;
void (*p_report)(uint32_t) = mesh_rt_report;
void (*p_stats)(MeshRtStats *) = mesh_rt_stats;
int main(void) {
  printf("%zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %u %u %u\n", sizeof(MeshRtStats),
    offsetof(MeshRtStats, captured_frames), offsetof(MeshRtStats, rendered_frames),
    offsetof(MeshRtStats, capture_overruns), offsetof(MeshRtStats, render_underruns),
    offsetof(MeshRtStats, capture_callbacks), offsetof(MeshRtStats, render_callbacks),
    offsetof(MeshRtStats, capture_rms_dbfs), offsetof(MeshRtStats, render_rms_dbfs),
    offsetof(MeshRtStats, generation), MESH_RT_ABI_VERSION, MESH_RT_SAMPLE_RATE,
    MESH_RT_FLAG_LATCH_TOGGLE);
  return 0;
}
"#,
    )
    .unwrap();
    // Link against nothing: the pointers are resolved by a stub object below.
    let stub = out_dir.join("rt_stub.c");
    std::fs::write(
        &stub,
        r#"#include "mesh_rt.h"
int32_t mesh_rt_capture(const int16_t *p, uint32_t f, uint64_t t) { (void)p; (void)t; return (int32_t)f; }
int32_t mesh_rt_render(int16_t *p, uint32_t f, uint64_t t) { (void)p; (void)t; return (int32_t)f; }
void mesh_rt_set_flags(uint32_t f) { (void)f; }
void mesh_rt_report(uint32_t c) { (void)c; }
void mesh_rt_stats(MeshRtStats *o) { (void)o; }
"#,
    )
    .unwrap();
    let include = dir.join("include");
    let status = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(&include)
        .arg(&c)
        .arg(&stub)
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("a C compiler (cc) is available on the host");
    assert!(status.success(), "C compile against mesh_rt.h failed");
    let out = Command::new(&exe).output().unwrap();
    let got = String::from_utf8(out.stdout).unwrap();
    let expect = format!(
        "{} {} {} {} {} {} {} {} {} {} 1 16000 8\n",
        size_of::<MeshRtStats>(),
        offset_of!(MeshRtStats, captured_frames),
        offset_of!(MeshRtStats, rendered_frames),
        offset_of!(MeshRtStats, capture_overruns),
        offset_of!(MeshRtStats, render_underruns),
        offset_of!(MeshRtStats, capture_callbacks),
        offset_of!(MeshRtStats, render_callbacks),
        offset_of!(MeshRtStats, capture_rms_dbfs),
        offset_of!(MeshRtStats, render_rms_dbfs),
        offset_of!(MeshRtStats, generation),
    );
    assert_eq!(got, expect);
    assert!(Path::new(&exe).exists());
}
