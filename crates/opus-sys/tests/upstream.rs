//! Runs upstream libopus `test_opus_api`, `test_opus_decode` and `test_opus_encode`
//! (DESIGN §2.3 step 5). `build.rs` compiles them on the host against the same
//! `libopus.a` this crate links and exports their paths; on cross targets they are not
//! built, so these tests are only meaningful (and only run) on the host.

use std::process::Command;

/// Runs one upstream program. A failed check calls `test_failed()`, which aborts, so the
/// exit status is the verdict; `done` is the last line upstream prints on success and
/// proves the program ran to completion.
fn run_upstream(name: &str, path: Option<&str>, done: &str) {
    let path = path.unwrap_or_else(|| {
        panic!(
            "upstream test_opus_{name} was not built: build.rs only links it when HOST == TARGET"
        )
    });
    let out = Command::new(path)
        // Fixed seed: the upstream programs randomise inputs; a fixed seed keeps runs
        // reproducible (upstream reads SEED from the environment).
        .env("SEED", "42")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {path}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "upstream test_opus_{name} failed ({}):\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        out.status
    );
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains(done),
        "test_opus_{name} exited 0 without printing {done:?}:\n{all}"
    );
}

#[test]
fn upstream_test_opus_api() {
    run_upstream(
        "api",
        option_env!("OPUS_SYS_UPSTREAM_TEST_API"),
        "All API tests passed.",
    );
}

#[test]
fn upstream_test_opus_decode() {
    run_upstream(
        "decode",
        option_env!("OPUS_SYS_UPSTREAM_TEST_DECODE"),
        "Testing opus_pcm_soft_clip... OK.",
    );
}

#[test]
fn upstream_test_opus_encode() {
    run_upstream(
        "encode",
        option_env!("OPUS_SYS_UPSTREAM_TEST_ENCODE"),
        "Tests completed successfully.",
    );
}
