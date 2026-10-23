//! The sans-IO `ridelog/1` recorder (§3 of the diagnostics spec): stamps `ts`, applies
//! per-kind rate limits and the redaction filter, keeps a RAM ring, rotates segments by
//! size and age, and hands finished lines to the caller in [`Batch`]es (the runtime
//! forwards them to the platform's file store once per second). No clock, no I/O: the
//! caller passes `now` and the session id.

use std::collections::{BTreeMap, VecDeque};

use super::event::{DiagEvent, DiagSrc, MarkLabel, NoteEvent};
use super::header::{EndReason, Header};
use super::redact::redact;
use super::sampler::{FxCounters, SampleInputs, Sampler};
use crate::json::{Json, round};
use crate::time::{Instant, US_PER_S};

/// Recorder limits ([TUNE] defaults of §3.2-§3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderConfig {
    /// Lines kept in RAM for "export last minutes".
    pub ring_lines: usize,
    /// Segment rotation by size (bytes, newline included).
    pub seg_max_bytes: u64,
    /// Segment rotation by age (µs).
    pub seg_max_us: u64,
    /// Events per kind per second (token refill) and burst.
    pub event_rate: u32,
    pub event_burst: u32,
    /// `glitch` events per second (coalescing bound).
    pub glitch_rate: u32,
    /// Lines buffered for the platform before the oldest are dropped (file store stuck).
    pub max_pending_lines: usize,
}

impl Default for RecorderConfig {
    fn default() -> Self {
        RecorderConfig {
            ring_lines: 4096,
            seg_max_bytes: 1 << 20,
            seg_max_us: 600 * US_PER_S,
            event_rate: 20,
            event_burst: 40,
            glitch_rate: 10,
            max_pending_lines: 8192,
        }
    }
}

/// Lines for one segment file (`diag/<sessionId>/seg-NNNN.jsonl`), in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub session_id: String,
    pub seg: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone)]
struct Bucket {
    tokens_milli: u64,
    at: Instant,
    suppressed: u64,
}

#[derive(Debug, Clone)]
struct Session {
    header: Header,
    origin: Instant,
    seg: u32,
    seg_started: Instant,
    seg_bytes: u64,
    seg_lines: u64,
    seg_suppressed: u64,
    seg_redacted: u64,
}

/// The recorder.
#[derive(Debug, Clone)]
pub struct RideRecorder {
    cfg: RecorderConfig,
    enabled: bool,
    session: Option<Session>,
    ring: VecDeque<String>,
    pending: Vec<Batch>,
    pending_lines: usize,
    buckets: BTreeMap<String, Bucket>,
    sampler: Sampler,
    fx: FxCounters,
}

impl RideRecorder {
    pub fn new(cfg: RecorderConfig) -> Self {
        RideRecorder {
            cfg,
            enabled: true,
            session: None,
            ring: VecDeque::new(),
            pending: Vec::new(),
            pending_lines: 0,
            buckets: BTreeMap::new(),
            sampler: Sampler::new(),
            fx: FxCounters::default(),
        }
    }

    /// File writing on/off. When off, lines only reach the RAM ring. Turning it back on
    /// during a session starts a new segment, so the file store gets a `hdr` first
    /// (lines of a segment whose header never reached a file are dropped there).
    pub fn set_enabled(&mut self, now: Instant, on: bool) {
        let was = self.enabled;
        self.enabled = on;
        if !on {
            self.pending.clear();
            self.pending_lines = 0;
        } else if !was && self.session.is_some() {
            self.close_segment(now, EndReason::Rotate);
            self.open_segment(now);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// The current session id.
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_ref().map(|s| s.header.session_id.as_str())
    }

    /// The current header (identity fields as known now).
    pub fn header(&self) -> Option<&Header> {
        self.session.as_ref().map(|s| &s.header)
    }

    /// Seconds since the session start (the `ts` of a line written now).
    pub fn ts(&self, now: Instant) -> f64 {
        self.session
            .as_ref()
            .map_or(0.0, |s| now.since(s.origin) as f64 / US_PER_S as f64)
    }

    /// Ends the current session (if any) and starts a new one with segment 1.
    pub fn begin_session(&mut self, now: Instant, header: Header) {
        self.end_session(now, EndReason::SessionEnd);
        self.ring.clear();
        self.sampler.reset();
        self.buckets.clear();
        self.session = Some(Session {
            header,
            origin: now,
            seg: 0,
            seg_started: now,
            seg_bytes: 0,
            seg_lines: 0,
            seg_suppressed: 0,
            seg_redacted: 0,
        });
        self.open_segment(now);
    }

    /// Writes the `end` line of the current segment and closes the session.
    pub fn end_session(&mut self, now: Instant, reason: EndReason) {
        if self.session.is_some() {
            self.flush_suppressed(now);
            self.close_segment(now, reason);
            self.session = None;
        }
    }

    /// Updates the identity of the session (later segments' `hdr` carry it).
    pub fn set_identity(
        &mut self,
        ride_key: Option<String>,
        node: Option<u16>,
        role: Option<String>,
    ) {
        if let Some(s) = self.session.as_mut() {
            if ride_key.is_some() {
                s.header.ride_key = ride_key;
            }
            if node.is_some() {
                s.header.node = node;
            }
            if role.is_some() {
                s.header.role = role;
            }
        }
    }

    /// A typed event (core or runtime).
    pub fn event(&mut self, now: Instant, e: &DiagEvent) {
        if let DiagEvent::Session {
            ride_key,
            node,
            role,
        } = e
        {
            self.set_identity(
                ride_key.as_ref().map(super::event_fields::hex4),
                *node,
                role.map(str::to_string),
            );
        }
        if !self.admit(now, e.name()) {
            return;
        }
        let mut j = self.event_head(now, e.src(), e.name());
        for (k, v) in e.fields() {
            j.set(k, v);
        }
        self.write(now, j);
    }

    /// An event injected by platform or UI code (already validated by
    /// [`NoteEvent::new`]).
    pub fn note(&mut self, now: Instant, n: &NoteEvent) {
        if !self.admit(now, &n.ev) {
            return;
        }
        let mut j = self.event_head(now, n.src, &n.ev);
        for (k, v) in &n.fields {
            j.set(k, v.clone());
        }
        self.write(now, j);
    }

    /// A user marker (never rate-limited).
    pub fn mark(&mut self, now: Instant, label: MarkLabel, src: DiagSrc) {
        let j = Json::obj()
            .with("t", "mark")
            .with("ts", round(self.ts(now), 3))
            .with("src", src.as_str())
            .with("label", label.as_str());
        self.write(now, j);
    }

    /// The 1 Hz sample. Also emits the `suppressed` summary of the last second.
    pub fn sample(&mut self, now: Instant, input: SampleInputs<'_>) {
        self.flush_suppressed(now);
        let ts = self.ts(now);
        let j = self.sampler.sample(now, ts, input, self.fx);
        self.write(now, j);
    }

    /// One `e suppressed {of, n}` per rate-limited kind with drops since the last call.
    pub fn flush_suppressed(&mut self, now: Instant) {
        let drops: Vec<(String, u64)> = self
            .buckets
            .iter_mut()
            .filter(|(_, b)| b.suppressed > 0)
            .map(|(k, b)| (k.clone(), std::mem::take(&mut b.suppressed)))
            .collect();
        for (ev, n) in drops {
            let j = self
                .event_head(now, DiagSrc::Rt, "suppressed")
                .with("of", ev)
                .with("n", n);
            self.write(now, j);
        }
    }

    /// Batches ready for the file store (oldest first), emptied.
    pub fn take_batches(&mut self) -> Vec<Batch> {
        self.pending_lines = 0;
        std::mem::take(&mut self.pending)
    }

    /// Lines waiting for the file store.
    pub fn pending_lines(&self) -> usize {
        self.pending_lines
    }

    /// The current session as a self-contained file: its `hdr` and the RAM ring.
    pub fn ring_snapshot(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.ring.len() + 1);
        if let Some(s) = &self.session
            && self
                .ring
                .front()
                .is_none_or(|l| !l.starts_with("{\"t\":\"hdr\""))
        {
            out.push(s.header.to_json(s.seg.max(1)).to_json_string());
        }
        out.extend(self.ring.iter().cloned());
        out
    }

    /// Recorder counters (cumulative).
    pub fn counters(&self) -> FxCounters {
        self.fx
    }

    fn event_head(&self, now: Instant, src: DiagSrc, ev: &str) -> Json {
        Json::obj()
            .with("t", "e")
            .with("ts", round(self.ts(now), 3))
            .with("src", src.as_str())
            .with("ev", ev)
    }

    /// Token bucket per `ev` (`glitch` has its own, tighter rate).
    fn admit(&mut self, now: Instant, ev: &str) -> bool {
        let (rate, burst) = if ev == "glitch" {
            (self.cfg.glitch_rate, self.cfg.glitch_rate)
        } else {
            (self.cfg.event_rate, self.cfg.event_burst)
        };
        let cap = u64::from(burst) * 1000;
        let b = self.buckets.entry(ev.to_string()).or_insert(Bucket {
            tokens_milli: cap,
            at: now,
            suppressed: 0,
        });
        let refill = now.since(b.at) * u64::from(rate) / 1000;
        b.tokens_milli = (b.tokens_milli + refill).min(cap);
        b.at = now;
        if b.tokens_milli >= 1000 {
            b.tokens_milli -= 1000;
            true
        } else {
            b.suppressed += 1;
            self.fx.suppressed += 1;
            if let Some(s) = self.session.as_mut() {
                s.seg_suppressed += 1;
            }
            false
        }
    }

    fn write(&mut self, now: Instant, mut j: Json) {
        let red = u64::from(redact(&mut j));
        self.fx.redacted += red;
        if let Some(s) = self.session.as_mut() {
            s.seg_redacted += red;
        }
        if let Some(s) = &self.session
            && s.seg_lines > 0
            && (s.seg_bytes >= self.cfg.seg_max_bytes
                || now.since(s.seg_started) >= self.cfg.seg_max_us)
        {
            self.close_segment(now, EndReason::Rotate);
            self.open_segment(now);
        }
        self.push(j.to_json_string(), true);
    }

    fn open_segment(&mut self, now: Instant) {
        let Some(s) = self.session.as_mut() else {
            return;
        };
        s.seg += 1;
        s.seg_started = now;
        s.seg_bytes = 0;
        s.seg_lines = 0;
        s.seg_suppressed = 0;
        s.seg_redacted = 0;
        let line = s.header.to_json(s.seg).to_json_string();
        self.push(line, false);
    }

    fn close_segment(&mut self, now: Instant, reason: EndReason) {
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let j = Json::obj()
            .with("t", "end")
            .with("ts", round(self.ts(now), 3))
            .with("reason", reason.as_str())
            .with("lines", s.seg_lines + 1)
            .with("suppressed", s.seg_suppressed)
            .with("redacted", s.seg_redacted);
        self.push(j.to_json_string(), false);
    }

    /// Appends a finished line to the ring and (when enabled) to the pending batch of
    /// the current segment.
    fn push(&mut self, line: String, counted: bool) {
        if self.ring.len() >= self.cfg.ring_lines {
            self.ring.pop_front();
        }
        self.ring.push_back(line.clone());
        let Some(s) = self.session.as_mut() else {
            return;
        };
        s.seg_bytes += line.len() as u64 + 1;
        s.seg_lines += 1;
        if !self.enabled {
            return;
        }
        if self.pending_lines >= self.cfg.max_pending_lines && counted {
            // The file store is not draining: drop new data lines (never a `hdr` or
            // `end`, so every segment file stays valid); the ring still has them.
            self.fx.dropped_lines += 1;
            return;
        }
        let (id, seg) = (s.header.session_id.clone(), s.seg);
        match self.pending.last_mut() {
            Some(b) if b.session_id == id && b.seg == seg => b.lines.push(line),
            _ => self.pending.push(Batch {
                session_id: id,
                seg,
                lines: vec![line],
            }),
        }
        self.pending_lines += 1;
    }
}
