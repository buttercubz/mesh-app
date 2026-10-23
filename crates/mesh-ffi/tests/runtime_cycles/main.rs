//! Public-API tests on the process-global RT plane (own test binary, serialised):
//! start/stop cycles leak nothing (counting allocator), and the exported `mesh_rt_*`
//! symbols drive the engine thread end to end.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mesh_ffi::api::outputs::*;
use mesh_ffi::api::types::*;
use mesh_ffi::rt::abi::{
    mesh_rt_capture, mesh_rt_render, mesh_rt_report, mesh_rt_set_flags, mesh_rt_stats,
};
use mesh_ffi::rt::{MeshRtStats, RT};
use mesh_ffi::{MeshEngine, PlatformHooks};

/// Counts live heap bytes.
struct Counting;

static LIVE: AtomicI64 = AtomicI64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        LIVE.fetch_add(l.size() as i64, SeqCst);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size() as i64, SeqCst);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        LIVE.fetch_add(n as i64 - l.size() as i64, SeqCst);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Serialises the tests of this binary (they share the global RT plane).
static SERIAL: Mutex<()> = Mutex::new(());

/// Hooks that only count calls (no growing buffers, so the leak check sees the runtime).
#[derive(Default)]
struct CountingHooks {
    calls: AtomicU32,
    audio: AtomicU32,
}

impl PlatformHooks for CountingHooks {
    fn link_send(&self, _: u32, _: Vec<u8>) -> FfiSendOutcome {
        self.calls.fetch_add(1, SeqCst);
        FfiSendOutcome::Accepted
    }
    fn radio(&self, _: FfiRadioCommand) {
        self.calls.fetch_add(1, SeqCst);
    }
    fn audio(&self, _: FfiAudioCommand) {
        self.audio.fetch_add(1, SeqCst);
    }
    fn ui_event(&self, _: String) {
        self.calls.fetch_add(1, SeqCst);
    }
    fn persist(&self, _: String, _: Option<Vec<u8>>) {}
    fn notify(&self, _: FfiNotification) {}
    fn lab_write(&self, _: String, _: String) {}
    fn lab_write_summary(&self, _: String, _: String) {}
    fn lab_delete(&self, _: String) {}
    fn render_prompts(&self, _: Vec<FfiPromptRequest>) {}
    fn log(&self, _: FfiLogLevel, _: String) {}
    fn diag_write(&self, _: String, _: u32, _: Vec<String>) {}
}

fn config(seed: u8) -> FfiEngineConfig {
    FfiEngineConfig {
        platform: FfiPlatform::Android,
        app_version: 1,
        rng_seed: vec![seed; 32],
        device_model: "Pixel 9".into(),
        device_os: "Android 16".into(),
        app_label: "0.1.0 (1)".into(),
        persisted: vec![],
    }
}

fn link(role: FfiLinkRole) -> FfiLinkInfo {
    FfiLinkInfo {
        kind: FfiLinkKind::BleGatt,
        role,
        max_frame: 512,
        unit: FfiLinkUnit::Message,
        peer_platform_hint: None,
        att_mtu: Some(247),
        disc_id: None,
    }
}

/// One Lab session with link traffic and RT audio through the C symbols.
fn cycle(e: &MeshEngine, i: u32) {
    e.start(FfiSessionStart::Lab {
        lab_code: 4000 + (i % 100) as u16,
    })
    .unwrap();
    e.on_link_up(i, link(FfiLinkRole::Responder));
    e.on_link_data(i, vec![0x01, 0x02, 0x03]);
    let pcm = [100i16; 160];
    let mut out = [0i16; 160];
    for _ in 0..4 {
        // SAFETY: valid buffers of `frames` samples.
        unsafe {
            mesh_rt_capture(pcm.as_ptr(), 160, 0);
            mesh_rt_render(out.as_mut_ptr(), 160, 0);
        }
    }
    e.on_link_down(i, FfiLinkDownReason::PeerClosed);
    e.stop();
    assert_eq!(e.status(), FfiRuntimeStatus::Idle);
    assert!(!RT.is_claimed() && !RT.is_active());
}

#[test]
fn start_stop_cycles_leak_nothing() {
    let _g = SERIAL.lock().unwrap();
    let hooks = Arc::new(CountingHooks::default());
    let before_engine = LIVE.load(SeqCst);
    let e = MeshEngine::new(config(9), hooks.clone()).unwrap();
    // Warm up: engine thread, first rings, lazily grown tables.
    for i in 0..5 {
        cycle(&e, i);
    }
    let warm = LIVE.load(SeqCst);
    for i in 5..105 {
        cycle(&e, i);
    }
    let after = LIVE.load(SeqCst);
    assert!(
        after - warm < 64 * 1024,
        "100 start/stop cycles grew the heap by {} bytes",
        after - warm
    );
    assert!(hooks.calls.load(SeqCst) > 0);
    drop(e);
    // Dropping the handle joins the engine thread and frees the engine; only the RT
    // rings (process-global, reused by the next session) stay allocated.
    let rings = 2 * 4096 * 2 + 1024;
    let left = LIVE.load(SeqCst) - before_engine;
    assert!(left < rings as i64, "{left} bytes left after drop");
}

#[test]
fn exported_symbols_drive_the_engine_thread() {
    let _g = SERIAL.lock().unwrap();
    let hooks = Arc::new(CountingHooks::default());
    let e = MeshEngine::new(config(10), hooks).unwrap();
    // SAFETY (all calls below): null pointers and valid buffers only.
    let mut s0 = MeshRtStats::default();
    unsafe { mesh_rt_stats(&mut s0) };
    e.start(FfiSessionStart::Lab { lab_code: 1 }).unwrap();
    assert!(RT.is_active());
    assert_eq!(unsafe { mesh_rt_capture(std::ptr::null(), 160, 0) }, 0);
    assert_eq!(unsafe { mesh_rt_render(std::ptr::null_mut(), 160, 0) }, 0);
    unsafe { mesh_rt_stats(std::ptr::null_mut()) };
    let pcm = [5i16; 320];
    assert_eq!(unsafe { mesh_rt_capture(pcm.as_ptr(), 320, 11) }, 320);
    // The engine thread drains capture and keeps 20 ms of render queued.
    let t0 = Instant::now();
    let mut out = [1i16; 320];
    loop {
        let mut s = MeshRtStats::default();
        unsafe { mesh_rt_stats(&mut s) };
        let filled = unsafe { mesh_rt_render(out.as_mut_ptr(), 320, 12) };
        assert_eq!(filled, 320, "render always fills the buffer");
        if s.rendered_frames > s0.rendered_frames {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "engine thread not pumping"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    mesh_rt_set_flags(0b1001);
    mesh_rt_report(42);
    let mut s1 = MeshRtStats::default();
    unsafe { mesh_rt_stats(&mut s1) };
    assert_eq!(s1.generation, s0.generation + 1);
    assert!(s1.captured_frames >= s0.captured_frames + 320);
    assert!(s1.capture_callbacks > s0.capture_callbacks);
    assert!(s1.capture_rms_dbfs < -70.0 && s1.capture_rms_dbfs > -90.0);
    e.stop();
    mesh_rt_set_flags(0);
    // Inactive plane: silence and discard.
    let mut out = [3i16; 64];
    assert_eq!(unsafe { mesh_rt_render(out.as_mut_ptr(), 64, 0) }, 64);
    assert!(out.iter().all(|&s| s == 0));
    assert_eq!(unsafe { mesh_rt_capture(pcm.as_ptr(), 64, 0) }, 0);
}
