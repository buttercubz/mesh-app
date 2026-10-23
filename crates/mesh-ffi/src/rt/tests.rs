#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use super::*;

/// Leaks a private RT plane so threads can borrow it for `'static`.
fn private_rt() -> &'static RtShared {
    Box::leak(Box::new(RtShared::new()))
}

/// Total samples moved by each stress run (≈ 12 s of audio at 16 kHz).
const STRESS_SAMPLES: usize = 200_000;

#[test]
fn capture_ring_stress_keeps_order_without_loss() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let producer = thread::spawn(move || {
        let mut next: u16 = 0;
        let mut sent = 0usize;
        let mut block = [0i16; 509];
        let mut size = 1usize;
        while sent < STRESS_SAMPLES {
            size = (size * 7 + 3) % block.len() + 1;
            let n = size.min(STRESS_SAMPLES - sent);
            for s in &mut block[..n] {
                *s = next as i16;
                next = next.wrapping_add(1);
            }
            let mut off = 0;
            while off < n {
                off += rt.capture_rt(&block[off..n], sent as u64);
                if off < n {
                    thread::yield_now();
                }
            }
            sent += n;
        }
    });
    let mut expect: u16 = 0;
    let mut got = 0usize;
    let mut buf = [0i16; mesh_core::BLOCK_SAMPLES];
    while got + buf.len() <= STRESS_SAMPLES {
        if claim.pop_capture_exact(&mut buf) {
            for &s in &buf {
                assert_eq!(s as u16, expect, "capture order broken at sample {got}");
                expect = expect.wrapping_add(1);
                got += 1;
            }
        } else {
            thread::yield_now();
        }
    }
    producer.join().unwrap();
    let st = rt.stats();
    assert_eq!(st.captured_frames, STRESS_SAMPLES as u64);
    assert_eq!(st.generation, 1);
}

#[test]
fn render_ring_stress_plays_in_order_and_fills_with_silence() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let done: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let consumer = thread::spawn(move || {
        let mut expect: u16 = 1;
        let mut played_total = 0usize;
        let mut out = [7i16; 333];
        let mut size = 5usize;
        // Samples of the cross-fade still to skip after a dry callback.
        let mut fading = RESUME_FADE_SAMPLES; // a session's first audio fades in
        while played_total < STRESS_SAMPLES {
            size = (size * 13 + 1) % out.len() + 1;
            let played = rt.render_rt(&mut out[..size], 0);
            for &s in &out[..played] {
                if fading > 0 {
                    fading -= 1; // cross-faded from the decaying tail: not the raw sample
                } else {
                    assert_eq!(s as u16, expect, "render order broken");
                }
                expect = expect.wrapping_add(1);
                if expect == 0 {
                    expect = 1; // the engine never sends 0, so silence is unambiguous
                }
            }
            let tail = &out[played..size];
            assert!(
                tail.windows(2)
                    .all(|w| i32::from(w[1]).abs() <= i32::from(w[0]).abs()),
                "tail decays monotonically"
            );
            if tail.len() > 80 {
                assert!(tail[80..].iter().all(|&s| s == 0), "tail reaches silence");
            }
            if played < size {
                fading = RESUME_FADE_SAMPLES;
            }
            played_total += played;
            if played == 0 {
                thread::yield_now();
            }
        }
        done.store(true, Ordering::SeqCst);
    });
    let mut next: u16 = 1;
    let mut block = [0i16; 160];
    // `is_finished` also ends the loop when the consumer panicked (no hang).
    while !done.load(Ordering::SeqCst) && !consumer.is_finished() {
        if claim.render_fill() >= RENDER_TARGET_SAMPLES * 4 {
            thread::yield_now();
            continue;
        }
        for s in &mut block {
            *s = next as i16;
            next = next.wrapping_add(1);
            if next == 0 {
                next = 1;
            }
        }
        let mut off = 0;
        while off < block.len() && !done.load(Ordering::SeqCst) && !consumer.is_finished() {
            off += claim.push_render(&block[off..]);
        }
    }
    consumer.join().unwrap();
    assert!(rt.stats().rendered_frames >= STRESS_SAMPLES as u64);
}

#[test]
fn reset_while_rt_threads_run_is_safe() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    let stop: &'static AtomicBool = Box::leak(Box::new(AtomicBool::new(false)));
    let cap = thread::spawn(move || {
        let block = [1000i16; 160];
        while !stop.load(Ordering::SeqCst) {
            rt.capture_rt(&block, 0);
        }
    });
    let ren = thread::spawn(move || {
        let mut out = [0i16; 160];
        while !stop.load(Ordering::SeqCst) {
            rt.render_rt(&mut out, 0);
        }
    });
    // Both RT threads are running before the resets start.
    while rt.stats().capture_callbacks == 0 || rt.stats().render_callbacks == 0 {
        thread::yield_now();
    }
    let mut buf = [0i16; 160];
    for i in 0..200u32 {
        claim.reset_and_activate();
        assert_eq!(rt.stats().generation, i + 1);
        if i % 50 == 0 {
            thread::sleep(std::time::Duration::from_millis(1));
        }
        claim.push_render(&[5; 100]);
        let _ = claim.pop_capture_exact(&mut buf);
        if i % 3 == 0 {
            claim.deactivate();
        }
    }
    stop.store(true, Ordering::SeqCst);
    cap.join().unwrap();
    ren.join().unwrap();
    assert!(rt.stats().capture_callbacks > 0);
    assert!(rt.stats().render_callbacks > 0);
}

#[test]
fn inactive_plane_discards_capture_and_renders_silence() {
    let rt = private_rt();
    assert_eq!(rt.capture_rt(&[3; 160], 1), 0);
    let mut out = [9i16; 64];
    assert_eq!(rt.render_rt(&mut out, 2), 0);
    assert!(out.iter().all(|&s| s == 0));
    let st = rt.stats();
    assert_eq!((st.capture_callbacks, st.render_callbacks), (1, 1));
    assert_eq!(st.captured_frames, 0);
    assert_eq!(st.capture_rms_dbfs, rms_db(3));
    assert_eq!(st.render_rms_dbfs, RMS_FLOOR_DBFS);
    assert_eq!(rt.last_host_times(), (1, 2));
}

fn rms_db(v: i16) -> f32 {
    shared_rms(&[v; 160])
}

fn shared_rms(pcm: &[i16]) -> f32 {
    super::shared::rms_dbfs(pcm)
}

#[test]
fn rms_is_full_scale_for_a_square_wave_and_floored_for_silence() {
    let sq: Vec<i16> = (0..160)
        .map(|i| if i % 2 == 0 { i16::MAX } else { -i16::MAX })
        .collect();
    assert!(shared_rms(&sq).abs() < 0.01);
    assert_eq!(shared_rms(&[0; 10]), RMS_FLOOR_DBFS);
    assert_eq!(shared_rms(&[]), RMS_FLOOR_DBFS);
}

#[test]
fn overrun_and_underrun_are_counted() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let big = vec![1i16; CAPTURE_RING_SAMPLES + 100];
    assert_eq!(rt.capture_rt(&big, 0), CAPTURE_RING_SAMPLES);
    assert_eq!(rt.stats().capture_overruns, 1);
    assert_eq!(claim.capture_available(), CAPTURE_RING_SAMPLES);
    // The first queued audio of a session is faded in over RESUME_FADE_SAMPLES.
    claim.push_render(&[4; RESUME_FADE_SAMPLES + 10]);
    let mut fade = [0i16; RESUME_FADE_SAMPLES];
    assert_eq!(rt.render_rt(&mut fade, 0), RESUME_FADE_SAMPLES);
    assert!(fade[0] < 4 && fade[RESUME_FADE_SAMPLES - 1] == 4);
    assert_eq!(claim.render_fill(), 10);
    let mut out = [0i16; 16];
    assert_eq!(rt.render_rt(&mut out, 0), 10);
    let st = rt.stats();
    assert_eq!(st.render_underruns, 1);
    assert_eq!(st.xruns(), 2);
    // The dry tail decays from the last sample (no step to zero).
    assert_eq!(&out, &[4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 2, 1, 0, 0, 0]);
    assert_eq!(st.render_underrun_samples, 6);
    assert_eq!(st.capture_overrun_samples, 100);
    assert_eq!(st.max_capture_frames, (CAPTURE_RING_SAMPLES + 100) as u32);
    assert_eq!(st.max_render_frames, RESUME_FADE_SAMPLES as u32);
}

#[test]
fn latch_edge_survives_a_level_update_and_is_consumed_once() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    rt.set_flags(FLAG_PTT_DOWN | FLAG_LATCH_TOGGLE);
    rt.set_flags(FLAG_HARD_MUTE);
    assert_eq!(rt.flags(), FLAG_HARD_MUTE | FLAG_LATCH_TOGGLE);
    assert_eq!(claim.take_flags(), FLAG_HARD_MUTE | FLAG_LATCH_TOGGLE);
    assert_eq!(claim.take_flags(), FLAG_HARD_MUTE);
}

#[test]
fn only_one_claim_at_a_time() {
    let rt = private_rt();
    let mut a = rt.claim().unwrap();
    assert!(rt.claim().is_none());
    a.reset_and_activate();
    assert!(rt.is_active());
    drop(a);
    assert!(!rt.is_active() && !rt.is_claimed());
    assert!(rt.claim().is_some());
}

#[test]
fn reports_are_counted() {
    let rt = private_rt();
    rt.report(7);
    rt.report(9);
    assert_eq!(rt.reports(), (2, 9));
}

#[test]
fn render_target_follows_the_largest_callback() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    assert_eq!(claim.render_target(), RENDER_TARGET_SAMPLES);
    let mut out = vec![0i16; 4096];
    rt.render_rt(&mut out[..64], 0);
    rt.render_rt(&mut out[..160], 0);
    assert_eq!(
        claim.render_target(),
        RENDER_TARGET_SAMPLES,
        "10 ms callbacks: 20 ms"
    );
    // iOS with the screen locked: ~93 ms IO buffer at 44.1 kHz → 1486 samples at 16 kHz.
    rt.render_rt(&mut out[..1486], 0);
    assert_eq!(claim.render_target(), 1486 + RENDER_HEADROOM_SAMPLES);
    assert_eq!(
        rt.stats().render_target,
        (1486 + RENDER_HEADROOM_SAMPLES) as u32
    );
    rt.render_rt(&mut out[..4096], 0);
    assert_eq!(claim.render_target(), MAX_RENDER_TARGET_SAMPLES, "capped");
    // A new session forgets the device's callback sizes.
    claim.reset_and_activate();
    assert_eq!(claim.render_target(), RENDER_TARGET_SAMPLES);
}

#[test]
fn underruns_after_playout_started_widen_the_margin() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let mut out = [0i16; 160];
    // Dry callbacks before any audio played: the start-up race, no margin.
    rt.render_rt(&mut out, 0);
    rt.render_rt(&mut out, 0);
    assert_eq!(claim.render_target(), RENDER_TARGET_SAMPLES);
    claim.push_render(&[100; 200]);
    rt.render_rt(&mut out, 0);
    rt.render_rt(&mut out, 0); // 40 samples left: dry
    assert_eq!(
        claim.render_target(),
        RENDER_TARGET_SAMPLES + RENDER_MARGIN_STEP_SAMPLES as usize
    );
    for _ in 0..100 {
        rt.render_rt(&mut out, 0);
    }
    assert_eq!(
        claim.render_target(),
        RENDER_TARGET_SAMPLES + MAX_RENDER_MARGIN_SAMPLES as usize,
        "margin capped"
    );
}

/// Largest sample-to-sample step of a sequence.
fn max_step(prev: &mut i16, pcm: &[i16]) -> i32 {
    let mut m = 0;
    for &s in pcm {
        m = m.max((i32::from(s) - i32::from(*prev)).abs());
        *prev = s;
    }
    m
}

#[test]
fn a_dry_ring_decays_and_resumes_without_a_step() {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let level = 12_000i16;
    let mut prev = 0i16;
    let mut worst = 0;
    let mut out = [0i16; 160];
    // Start (fade-in from silence), steady playout, a dry callback mid-burst, resume.
    for round in 0..6 {
        let queued = if round == 3 { 100 } else { 160 };
        claim.push_render(&vec![level; queued]);
        rt.render_rt(&mut out, 0);
        worst = worst.max(max_step(&mut prev, &out));
    }
    assert_eq!(rt.stats().render_underruns, 1);
    // A hard step would be 12 000; decay (×7/8) and the 32-sample cross-fade keep every
    // step below 1/8 of the level plus one fade increment.
    assert!(
        worst <= i32::from(level) / 8 + i32::from(level) / 32 + 1,
        "worst step {worst}"
    );
}

/// Device and engine interleaved on one simulated clock: the device pulls `cb` samples
/// every `cb / 16` ms; the engine refills to `render_target()` every 5 ms, sometimes 15 ms
/// late. Returns the underruns after the first second.
fn simulate_device(cb: usize, seconds: u64) -> u32 {
    let rt = private_rt();
    let mut claim = rt.claim().unwrap();
    claim.reset_and_activate();
    let mut out = vec![0i16; cb];
    let block = [1000i16; 160];
    let cb_us = cb as u64 * 1_000_000 / 16_000;
    let (mut t_dev, mut t_eng, mut k) = (0u64, 0u64, 0u64);
    let mut base = None;
    while t_dev < seconds * 1_000_000 {
        if t_eng <= t_dev {
            let target = claim.render_target();
            while claim.render_fill() < target {
                claim.push_render(&block);
            }
            k += 1;
            t_eng += if k % 37 == 0 { 20_000 } else { 5_000 };
        } else {
            rt.render_rt(&mut out, t_dev);
            t_dev += cb_us;
            if base.is_none() && t_dev >= 1_000_000 {
                base = Some(rt.stats().render_underruns);
            }
        }
    }
    rt.stats().render_underruns - base.unwrap_or(0)
}

#[test]
fn odd_device_callback_sizes_do_not_starve_the_ring() {
    // Android Oboe at 48 kHz resampled to 16 kHz: 96/192/480/960-frame bursts → 32, 64,
    // 160, 320 samples; 341 (≈ 1024 frames at 48 kHz); iOS 10 ms and screen-locked 1486.
    for cb in [32usize, 64, 107, 160, 320, 341, 1486] {
        assert_eq!(
            simulate_device(cb, 10),
            0,
            "callback of {cb} samples underran"
        );
    }
}
