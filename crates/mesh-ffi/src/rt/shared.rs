//! Shared state of the real-time audio plane (DESIGN §3.6): two `rtrb` SPSC rings, the
//! RT flags, counters and the claim/activation protocol.
//!
//! Thread roles:
//! - the **capture** side (`capture_rt`) is called by the OS audio input callback;
//! - the **render** side (`render_rt`) is called by the OS audio output callback;
//! - the **engine** side (`pop_capture_block`, `push_render`, `reset_and_activate`, …)
//!   is called by the engine thread or by the runtime while no engine thread exists.
//!
//! Each side guards its ring halves with a try-lock flag taken by `swap(true)`. The RT
//! sides never wait: when their flag is already taken (a reset in progress, or a contract
//! violation such as two capture threads), they discard or output silence and return.
//! Only the engine-side reset spins, and only for the length of one RT callback. This
//! keeps every RT entry point wait-free and the `UnsafeCell` accesses data-race free.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering::SeqCst};

use rtrb::{Consumer, Producer, RingBuffer};

use super::{
    CAPTURE_RING_SAMPLES, FLAG_LATCH_TOGGLE, MAX_RENDER_MARGIN_SAMPLES, MAX_RENDER_TARGET_SAMPLES,
    RENDER_HEADROOM_SAMPLES, RENDER_MARGIN_STEP_SAMPLES, RENDER_RING_SAMPLES,
    RENDER_TARGET_SAMPLES, RESUME_FADE_SAMPLES, RMS_FLOOR_DBFS, RtStats,
};

/// One try-lock flag. `try_lock` never waits; `lock_spin` spins (engine side only).
struct Gate(AtomicBool);

impl Gate {
    const fn new() -> Self {
        Gate(AtomicBool::new(false))
    }

    fn try_lock(&self) -> bool {
        !self.0.swap(true, SeqCst)
    }

    fn lock_spin(&self) {
        while self.0.swap(true, SeqCst) {
            std::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.0.store(false, SeqCst);
    }
}

/// `[min, max]` callback size (samples) seen in a window, `None` when none ran.
pub type CbWindow = Option<(u32, u32)>;

/// The RT plane state. One process-global instance backs the C ABI (`super::RT`); tests
/// may create private instances.
pub struct RtShared {
    /// Rings are live: capture is queued and render plays queued audio.
    active: AtomicBool,
    /// Owned by one runtime (`RtClaim`).
    claimed: AtomicBool,
    capture_gate: Gate,
    render_gate: Gate,
    engine_gate: Gate,
    /// Producer half of the capture ring (capture side).
    capture_tx: UnsafeCell<Option<Producer<i16>>>,
    /// Consumer half of the capture ring (engine side).
    capture_rx: UnsafeCell<Option<Consumer<i16>>>,
    /// Producer half of the render ring (engine side).
    render_tx: UnsafeCell<Option<Producer<i16>>>,
    /// Consumer half of the render ring (render side).
    render_rx: UnsafeCell<Option<Consumer<i16>>>,
    /// Level bits 0-2 of the flags.
    flags: AtomicU32,
    /// Unconsumed latch-toggle edge (bit 3).
    latch: AtomicBool,
    generation: AtomicU32,
    captured_frames: AtomicU64,
    rendered_frames: AtomicU64,
    capture_overruns: AtomicU32,
    render_underruns: AtomicU32,
    capture_callbacks: AtomicU32,
    render_callbacks: AtomicU32,
    capture_rms_bits: AtomicU32,
    render_rms_bits: AtomicU32,
    reports: AtomicU32,
    last_report: AtomicU32,
    last_capture_host_ns: AtomicU64,
    last_render_host_ns: AtomicU64,
    // Render-side glitch handling and diagnostics (first device test: crackling).
    /// Samples zero/decay-filled because the ring ran dry.
    render_underrun_samples: AtomicU64,
    /// Samples refused because the capture ring was full.
    capture_overrun_samples: AtomicU64,
    /// Largest callback sizes seen since the last activation.
    max_render_frames: AtomicU32,
    max_capture_frames: AtomicU32,
    /// Extra refill margin learnt from underruns since the last activation.
    render_margin: AtomicU32,
    /// The render side played queued audio since the last activation (an underrun
    /// before that is the start-up race, not engine starvation).
    render_primed: AtomicBool,
    /// Last sample written by the render side (render thread only).
    render_tail: AtomicI32,
    /// Callback sizes seen since the diagnostics recorder last read them
    /// ([`RtShared::take_cb_window`]); min starts at `u32::MAX`, max at 0.
    win_capture_min: AtomicU32,
    win_capture_max: AtomicU32,
    win_render_min: AtomicU32,
    win_render_max: AtomicU32,
    /// The previous render callback ran dry: cross-fade into the next queued audio.
    render_resume: AtomicBool,
}

// SAFETY: every `UnsafeCell` is only touched while holding the gate of its side
// (capture_tx: capture_gate; render_rx: render_gate; capture_rx and render_tx:
// engine_gate). A reset holds all three gates. `rtrb` halves are `Send`.
unsafe impl Sync for RtShared {}

impl Default for RtShared {
    fn default() -> Self {
        Self::new()
    }
}

impl RtShared {
    /// An inactive, unclaimed RT plane without rings.
    pub const fn new() -> Self {
        RtShared {
            active: AtomicBool::new(false),
            claimed: AtomicBool::new(false),
            capture_gate: Gate::new(),
            render_gate: Gate::new(),
            engine_gate: Gate::new(),
            capture_tx: UnsafeCell::new(None),
            capture_rx: UnsafeCell::new(None),
            render_tx: UnsafeCell::new(None),
            render_rx: UnsafeCell::new(None),
            flags: AtomicU32::new(0),
            latch: AtomicBool::new(false),
            generation: AtomicU32::new(0),
            captured_frames: AtomicU64::new(0),
            rendered_frames: AtomicU64::new(0),
            capture_overruns: AtomicU32::new(0),
            render_underruns: AtomicU32::new(0),
            capture_callbacks: AtomicU32::new(0),
            render_callbacks: AtomicU32::new(0),
            capture_rms_bits: AtomicU32::new(RMS_FLOOR_DBFS.to_bits()),
            render_rms_bits: AtomicU32::new(RMS_FLOOR_DBFS.to_bits()),
            reports: AtomicU32::new(0),
            last_report: AtomicU32::new(0),
            last_capture_host_ns: AtomicU64::new(0),
            last_render_host_ns: AtomicU64::new(0),
            render_underrun_samples: AtomicU64::new(0),
            capture_overrun_samples: AtomicU64::new(0),
            max_render_frames: AtomicU32::new(0),
            max_capture_frames: AtomicU32::new(0),
            render_margin: AtomicU32::new(0),
            render_primed: AtomicBool::new(false),
            render_tail: AtomicI32::new(0),
            render_resume: AtomicBool::new(false),
            win_capture_min: AtomicU32::new(u32::MAX),
            win_capture_max: AtomicU32::new(0),
            win_render_min: AtomicU32::new(u32::MAX),
            win_render_max: AtomicU32::new(0),
        }
    }

    // ---------------------------------------------------------------- RT sides

    /// Capture callback: queues `pcm` for the engine. Returns the samples accepted.
    /// Wait-free; never allocates.
    pub fn capture_rt(&self, pcm: &[i16], host_time_ns: u64) -> usize {
        self.capture_callbacks.fetch_add(1, SeqCst);
        self.last_capture_host_ns.store(host_time_ns, SeqCst);
        let n = u32::try_from(pcm.len()).unwrap_or(u32::MAX);
        self.max_capture_frames.fetch_max(n, SeqCst);
        self.win_capture_min.fetch_min(n, SeqCst);
        self.win_capture_max.fetch_max(n, SeqCst);
        self.capture_rms_bits.store(rms_dbfs(pcm).to_bits(), SeqCst);
        if !self.capture_gate.try_lock() {
            return 0;
        }
        let mut accepted = 0;
        if self.active.load(SeqCst) {
            // SAFETY: capture_gate is held.
            if let Some(tx) = unsafe { &mut *self.capture_tx.get() } {
                let (pushed, rest) = tx.push_partial_slice(pcm);
                accepted = pushed.len();
                if !rest.is_empty() {
                    self.capture_overruns.fetch_add(1, SeqCst);
                    self.capture_overrun_samples
                        .fetch_add(rest.len() as u64, SeqCst);
                }
            }
        }
        self.capture_gate.unlock();
        self.captured_frames.fetch_add(accepted as u64, SeqCst);
        accepted
    }

    /// Render callback: fills all of `out` (queued audio, then a short decay of the last
    /// sample down to silence). Returns the samples that came from the ring. Wait-free;
    /// never allocates.
    ///
    /// Glitch handling (first device test, "crackling"): when the ring runs dry the tail
    /// is not a hard step to zero but the last sample decaying over ~3 ms, and the first
    /// queued audio after a dry callback is cross-faded in over
    /// [`RESUME_FADE_SAMPLES`](super::RESUME_FADE_SAMPLES). Underruns after the first
    /// played audio also widen the engine's refill target ([`RtClaim::render_target`]).
    pub fn render_rt(&self, out: &mut [i16], host_time_ns: u64) -> usize {
        self.render_callbacks.fetch_add(1, SeqCst);
        self.last_render_host_ns.store(host_time_ns, SeqCst);
        let n = u32::try_from(out.len()).unwrap_or(u32::MAX);
        self.max_render_frames.fetch_max(n, SeqCst);
        self.win_render_min.fetch_min(n, SeqCst);
        self.win_render_max.fetch_max(n, SeqCst);
        let mut played = 0;
        if self.render_gate.try_lock() {
            if self.active.load(SeqCst) {
                // SAFETY: render_gate is held.
                if let Some(rx) = unsafe { &mut *self.render_rx.get() } {
                    let (filled, rest) = rx.pop_partial_slice(out);
                    played = filled.len();
                    if !rest.is_empty() {
                        self.note_underrun(rest.len());
                    }
                }
            }
            self.render_gate.unlock();
        }
        let mut tail = self.render_tail.load(SeqCst);
        if played > 0 {
            if let Some(head) = out.get_mut(..played) {
                if self.render_resume.swap(false, SeqCst) {
                    crossfade_in(head, tail);
                }
                tail = head.last().map_or(tail, |&s| i32::from(s));
            }
            self.render_primed.store(true, SeqCst);
        }
        if let Some(rest) = out.get_mut(played..)
            && !rest.is_empty()
        {
            tail = decay_fill(rest, tail);
            self.render_resume.store(true, SeqCst);
        }
        self.render_tail.store(tail, SeqCst);
        self.render_rms_bits.store(rms_dbfs(out).to_bits(), SeqCst);
        self.rendered_frames.fetch_add(played as u64, SeqCst);
        played
    }

    /// Counts one dry render callback missing `missing` samples; once audio has played
    /// since the last activation it also widens the refill margin by one step.
    fn note_underrun(&self, missing: usize) {
        self.render_underruns.fetch_add(1, SeqCst);
        self.render_underrun_samples
            .fetch_add(missing as u64, SeqCst);
        if self.render_primed.load(SeqCst) {
            self.render_margin
                .fetch_add(RENDER_MARGIN_STEP_SAMPLES, SeqCst);
            self.render_margin
                .fetch_min(MAX_RENDER_MARGIN_SAMPLES, SeqCst);
        }
    }

    /// Render ring fill the engine should keep: the larger of the 20 ms baseline and the
    /// largest render callback plus [`RENDER_HEADROOM_SAMPLES`], plus the margin learnt
    /// from underruns, capped at [`MAX_RENDER_TARGET_SAMPLES`]. A device that pulls 20 ms
    /// or more per callback (Android VoIP paths, iOS with the screen locked) would
    /// otherwise drain the 20 ms ring in one callback and play zeros until the next.
    pub fn render_target(&self) -> usize {
        let cb = self.max_render_frames.load(SeqCst) as usize;
        let margin = self.render_margin.load(SeqCst) as usize;
        RENDER_TARGET_SAMPLES
            .max(cb.saturating_add(RENDER_HEADROOM_SAMPLES))
            .saturating_add(margin)
            .min(MAX_RENDER_TARGET_SAMPLES)
    }

    /// `mesh_rt_set_flags`: bits 0-2 are levels and replace the previous value; bit 3
    /// (latch toggle) is an edge kept in its own flag until the engine consumes it, so a
    /// following call without bit 3 cannot erase an unconsumed toggle. Wait-free.
    pub fn set_flags(&self, flags: u32) {
        self.flags.store(flags & !FLAG_LATCH_TOGGLE, SeqCst);
        if flags & FLAG_LATCH_TOGGLE != 0 {
            self.latch.store(true, SeqCst);
        }
    }

    /// `mesh_rt_report`.
    pub fn report(&self, event_code: u32) {
        self.last_report.store(event_code, SeqCst);
        self.reports.fetch_add(1, SeqCst);
    }

    /// `mesh_rt_stats`.
    pub fn stats(&self) -> RtStats {
        RtStats {
            captured_frames: self.captured_frames.load(SeqCst),
            rendered_frames: self.rendered_frames.load(SeqCst),
            capture_overruns: self.capture_overruns.load(SeqCst),
            render_underruns: self.render_underruns.load(SeqCst),
            capture_callbacks: self.capture_callbacks.load(SeqCst),
            render_callbacks: self.render_callbacks.load(SeqCst),
            capture_rms_dbfs: f32::from_bits(self.capture_rms_bits.load(SeqCst)),
            render_rms_dbfs: f32::from_bits(self.render_rms_bits.load(SeqCst)),
            generation: self.generation.load(SeqCst),
            render_underrun_samples: self.render_underrun_samples.load(SeqCst),
            capture_overrun_samples: self.capture_overrun_samples.load(SeqCst),
            max_render_frames: self.max_render_frames.load(SeqCst),
            max_capture_frames: self.max_capture_frames.load(SeqCst),
            render_target: u32::try_from(self.render_target()).unwrap_or(u32::MAX),
        }
    }

    /// `[min, max]` capture and render callback sizes (samples) seen since the previous
    /// call, then restarts the window (diagnostics recorder, 1 Hz; `None` = no
    /// callback ran). Wait-free; RT sides only do `fetch_min`/`fetch_max`.
    pub fn take_cb_window(&self) -> (CbWindow, CbWindow) {
        let take = |min: &AtomicU32, max: &AtomicU32| {
            let hi = max.swap(0, SeqCst);
            let lo = min.swap(u32::MAX, SeqCst);
            (hi > 0 && lo != u32::MAX).then_some((lo.min(hi), hi))
        };
        (
            take(&self.win_capture_min, &self.win_capture_max),
            take(&self.win_render_min, &self.win_render_max),
        )
    }

    /// Reports received through `mesh_rt_report`: (count, last code).
    pub fn reports(&self) -> (u32, u32) {
        (self.reports.load(SeqCst), self.last_report.load(SeqCst))
    }

    /// Host time of the last capture and render callbacks (ns).
    pub fn last_host_times(&self) -> (u64, u64) {
        (
            self.last_capture_host_ns.load(SeqCst),
            self.last_render_host_ns.load(SeqCst),
        )
    }

    /// Current flags without consuming the latch edge.
    pub fn flags(&self) -> u32 {
        let latch = if self.latch.load(SeqCst) {
            FLAG_LATCH_TOGGLE
        } else {
            0
        };
        self.flags.load(SeqCst) | latch
    }

    /// Whether the rings are live.
    pub fn is_active(&self) -> bool {
        self.active.load(SeqCst)
    }
}

/// RMS level of a block in dBFS (`RMS_FLOOR_DBFS` for silence or an empty block).
pub(crate) fn rms_dbfs(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return RMS_FLOOR_DBFS;
    }
    let sum: f64 = pcm.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let mean = sum / pcm.len() as f64;
    if mean <= 0.0 {
        return RMS_FLOOR_DBFS;
    }
    let db = 10.0 * (mean / (32_768.0 * 32_768.0)).log10();
    (db as f32).max(RMS_FLOOR_DBFS)
}

/// Per-sample decay of the held tail when the ring runs dry (−37 dB after 2 ms).
const TAIL_DECAY_NUM: i32 = 7;
const TAIL_DECAY_DEN: i32 = 8;

/// Fills `rest` with `tail` decaying to zero (no step to silence). Returns the new tail.
fn decay_fill(rest: &mut [i16], mut tail: i32) -> i32 {
    for s in rest.iter_mut() {
        tail = tail * TAIL_DECAY_NUM / TAIL_DECAY_DEN;
        *s = clamp_i16(tail);
    }
    tail
}

/// Cross-fades from the decaying `tail` into `head` over the first
/// [`RESUME_FADE_SAMPLES`] samples (linear gain on the new audio).
fn crossfade_in(head: &mut [i16], mut tail: i32) {
    let n = RESUME_FADE_SAMPLES as i32;
    for (i, s) in head.iter_mut().take(RESUME_FADE_SAMPLES).enumerate() {
        tail = tail * TAIL_DECAY_NUM / TAIL_DECAY_DEN;
        let g = i as i32 + 1;
        *s = clamp_i16((i32::from(*s) * g + tail * (n - g)) / n);
    }
}

fn clamp_i16(v: i32) -> i16 {
    v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

/// Exclusive engine-side access to an [`RtShared`], held by one runtime at a time.
/// Dropping it deactivates the rings and releases the claim.
pub struct RtClaim<'a> {
    rt: &'a RtShared,
}

impl RtShared {
    /// Claims the engine side, or `None` when another runtime holds it.
    pub fn claim(&self) -> Option<RtClaim<'_>> {
        self.claimed
            .compare_exchange(false, true, SeqCst, SeqCst)
            .ok()
            .map(|_| RtClaim { rt: self })
    }

    /// Whether a runtime holds the engine side.
    pub fn is_claimed(&self) -> bool {
        self.claimed.load(SeqCst)
    }
}

impl RtClaim<'_> {
    /// Recreates both rings empty, bumps `generation` and activates them (session start).
    /// Allocates; never called on an RT thread.
    pub fn reset_and_activate(&mut self) {
        let rt = self.rt;
        rt.engine_gate.lock_spin();
        rt.capture_gate.lock_spin();
        rt.render_gate.lock_spin();
        let (ctx, crx) = RingBuffer::<i16>::new(CAPTURE_RING_SAMPLES);
        let (rtx, rrx) = RingBuffer::<i16>::new(RENDER_RING_SAMPLES);
        // SAFETY: all three gates are held, so no other access to the cells exists.
        unsafe {
            *rt.capture_tx.get() = Some(ctx);
            *rt.capture_rx.get() = Some(crx);
            *rt.render_tx.get() = Some(rtx);
            *rt.render_rx.get() = Some(rrx);
        }
        rt.max_render_frames.store(0, SeqCst);
        rt.max_capture_frames.store(0, SeqCst);
        rt.render_margin.store(0, SeqCst);
        rt.render_primed.store(false, SeqCst);
        rt.render_resume.store(true, SeqCst); // fade the first queued audio in
        rt.generation.fetch_add(1, SeqCst);
        rt.active.store(true, SeqCst);
        rt.render_gate.unlock();
        rt.capture_gate.unlock();
        rt.engine_gate.unlock();
    }

    /// Stops queueing: capture is discarded and render outputs silence. The rings are
    /// kept until the next `reset_and_activate` (no deallocation near RT threads).
    pub fn deactivate(&mut self) {
        self.rt.active.store(false, SeqCst);
    }

    /// Whether the rings are live.
    pub fn is_active(&self) -> bool {
        self.rt.is_active()
    }

    /// Samples waiting in the capture ring.
    pub fn capture_available(&mut self) -> usize {
        self.with_engine(|crx, _| crx.map_or(0, |c| c.slots()))
    }

    /// Pops exactly `out.len()` samples when that many are queued.
    pub fn pop_capture_exact(&mut self, out: &mut [i16]) -> bool {
        self.with_engine(|crx, _| crx.is_some_and(|c| c.pop_entire_slice(out).is_ok()))
    }

    /// Samples queued in the render ring (not yet played).
    pub fn render_fill(&mut self) -> usize {
        self.with_engine(|_, rtx| {
            rtx.map_or(0, |p| p.buffer().capacity().saturating_sub(p.slots()))
        })
    }

    /// Render ring fill to keep (see [`RtShared::render_target`]).
    pub fn render_target(&self) -> usize {
        self.rt.render_target()
    }

    /// Queues playout; returns the samples accepted.
    pub fn push_render(&mut self, pcm: &[i16]) -> usize {
        self.with_engine(|_, rtx| rtx.map_or(0, |p| p.push_partial_slice(pcm).0.len()))
    }

    /// Reads the flags and consumes the latch-toggle edge (bit 3).
    pub fn take_flags(&mut self) -> u32 {
        let latch = if self.rt.latch.swap(false, SeqCst) {
            FLAG_LATCH_TOGGLE
        } else {
            0
        };
        self.rt.flags.load(SeqCst) | latch
    }

    /// The shared state.
    pub fn shared(&self) -> &RtShared {
        self.rt
    }

    fn with_engine<T>(
        &mut self,
        f: impl FnOnce(Option<&mut Consumer<i16>>, Option<&mut Producer<i16>>) -> T,
    ) -> T {
        let rt = self.rt;
        rt.engine_gate.lock_spin();
        // SAFETY: engine_gate is held; capture_rx and render_tx are engine-side cells.
        let r = unsafe {
            f(
                (*rt.capture_rx.get()).as_mut(),
                (*rt.render_tx.get()).as_mut(),
            )
        };
        rt.engine_gate.unlock();
        r
    }
}

impl Drop for RtClaim<'_> {
    fn drop(&mut self) {
        self.rt.active.store(false, SeqCst);
        self.rt.claimed.store(false, SeqCst);
    }
}
