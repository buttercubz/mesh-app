//! One simulated phone: its real [`Engine`], the platform state the engine drives
//! (HELLO, discovery, audio session) and the record of everything it emitted.

use std::collections::BTreeMap;

use mesh_api::{
    CommandResult, NoticeKey, Snapshot, TalkDeniedReason, TalkState, UiCommand, UiEvent,
};
use mesh_core::effects::{
    AdvPayload, AudioCommand, AudioSessionProfile, DiscoveryMode, Effect, LabFile, RadioCommand,
};
use mesh_core::{BLOCK_SAMPLES, Engine, Instant, LinkId};

use crate::config::NodeSpec;
use crate::onset::OnsetDetector;
use crate::synth::SpeechSynth;
use crate::tap::TapHandle;

/// Audio IO the simulated platform runs for this node (§3.2, §10.1.2).
#[derive(Debug, Clone)]
pub struct AudioPlatform {
    /// The open session, if any.
    pub session: Option<AudioSessionProfile>,
    /// The RT capture sink is connected and unmuted.
    pub capture: bool,
    /// A 10 ms audio tick is scheduled.
    pub ticking: bool,
    /// Capture / render blocks since the session opened.
    pub capture_blocks: u64,
    pub render_blocks: u64,
    /// Blocks counted at the previous platform sample (for fps).
    pub(crate) last_sample: (u64, u64, Instant),
    /// Peak absolute playout sample since the session opened.
    pub playout_peak: i16,
    pub(crate) mic: SpeechSynth,
    /// Scripted talk spans `[from, until)` (µs): when set, the microphone hears speech
    /// only inside a span and silence elsewhere; `None` = speech all the time (Lab).
    pub talk: Option<Vec<(u64, u64)>>,
    /// The mic was inside a span at the previous block.
    pub(crate) in_span: bool,
    /// Onsets of what this node's microphone captured (enabled by the scenario).
    pub mouth: Option<OnsetDetector>,
    /// Onsets of what this node played (enabled by the scenario).
    pub ear: Option<OnsetDetector>,
    /// Optional PCM to play into the microphone instead of the synth (Demo bots).
    pub(crate) pcm: Option<PcmSource>,
}

/// A queue of PCM clips the microphone plays, each from a start instant.
#[derive(Debug, Clone, Default)]
pub struct PcmSource {
    /// `(start µs, samples)`; consumed in order.
    pub clips: std::collections::VecDeque<(u64, Vec<i16>)>,
    pub pos: usize,
}

impl AudioPlatform {
    /// The microphone block captured in `[t − 10 ms, t)`: PCM clips first (Demo bots),
    /// else the synth, gated by the talk spans when they are set.
    pub(crate) fn fill_mic(&mut self, t: u64, out: &mut [i16; BLOCK_SAMPLES]) {
        if let Some(src) = self.pcm.as_mut() {
            out.fill(0);
            while let Some((start, clip)) = src.clips.front() {
                if *start > t {
                    break;
                }
                if src.pos >= clip.len() {
                    src.clips.pop_front();
                    src.pos = 0;
                    continue;
                }
                let n = (clip.len() - src.pos).min(BLOCK_SAMPLES);
                out[..n].copy_from_slice(&clip[src.pos..src.pos + n]);
                src.pos += n;
                break;
            }
            return;
        }
        let Some(spans) = self.talk.as_ref() else {
            self.mic.fill_block(out);
            return;
        };
        // The block's capture window starts 10 ms before the push.
        let start = t.saturating_sub(10_000);
        let inside = spans.iter().any(|(a, b)| start >= *a && start < *b);
        if inside && !self.in_span {
            self.mic.restart();
        }
        self.in_span = inside;
        if inside {
            self.mic.fill_block(out);
        } else {
            out.fill(0);
        }
    }
}

/// One command handled by a node, with the range of effects it produced.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub at: Instant,
    pub cmd: UiCommand,
    pub result: CommandResult,
    /// `effects[start..end]` were emitted while handling it.
    pub start: usize,
    pub end: usize,
}

/// A connect the engine asked for (`RadioCommand::Connect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingConnect {
    pub target: usize,
    pub generation: u64,
    pub hello_read: bool,
    /// The target was out of reach when the connect first reached for it; the
    /// connect stays outstanding (iOS `connect` never times out by itself, Android
    /// `connectGatt` waits too) and completes once the target is back in reach.
    pub waiting: bool,
}

/// One simulated phone.
pub struct SimNode {
    pub idx: usize,
    pub spec: NodeSpec,
    pub engine: Engine,
    /// Restarts so far (S8 app restart: a new engine, a new `sess`).
    pub generation: u32,
    /// Every effect `(time, effect)`, when the world records effects.
    pub effects: Vec<(Instant, Effect)>,
    pub commands: Vec<CommandRecord>,
    /// labrun/1 and summary CSV lines per run id.
    pub lab_files: BTreeMap<(String, bool), Vec<String>>,
    /// Count of every effect kind (kept even when effects are not recorded).
    pub counters: EffectCounters,
    /// The receive tap registered first in the engine (replaced at app restart).
    pub tap: TapHandle,
    /// The latest UI snapshot (kept even when effects are not recorded).
    pub snapshot: Option<Snapshot>,
    /// When the latest snapshot was emitted (µs).
    pub snapshot_at: u64,
    /// Every notice `(time µs, key)` (kept even when effects are not recorded).
    pub notices: Vec<(u64, NoticeKey)>,
    /// Changes of the snapshot talk state `(time µs, state)`.
    pub talk_hist: Vec<(u64, TalkState)>,
    /// Changes of the snapshot talker list `(time µs, sorted node ids)`.
    pub talkers_hist: Vec<(u64, Vec<u16>)>,
    /// Changes of the snapshot BLE cap `(time µs, cap_ble)`.
    pub cap_hist: Vec<(u64, u8)>,
    /// `talkDenied` events `(time µs, reason)`.
    pub denied: Vec<(u64, TalkDeniedReason)>,
    /// Secure storage of the device (`Effect::Persist`), survives app restarts.
    pub store: BTreeMap<String, Vec<u8>>,
    pub hello: Option<[u8; 16]>,
    pub discovery: (DiscoveryMode, Option<AdvPayload>),
    /// Scan intensity of the last `SetDiscovery` (the radio cost model reads it).
    pub scan: mesh_core::ScanIntensity,
    /// Published L2CAP PSM.
    pub psm: u16,
    pub audio: AudioPlatform,
    pub pending: BTreeMap<u32, PendingConnect>,
    /// Engine deadline cache (the engine's own deadline or the next 5 ms tick).
    pub next_timeout: Option<Instant>,
    /// Phase of this phone's 5 ms engine loop (µs in `[0, 5000)`).
    pub tick_phase_us: u64,
    /// Node BLE airtime budget (§5.6, `NODE_BLE_BUDGET`): a leaky bucket of airtime.
    pub air: AirBucket,
    /// Airtime consumed (µs) and bytes sent over BLE.
    pub air_used_us: u64,
    pub ble_tx_bytes: u64,
}

/// The node airtime budget as a leaky bucket: `level_us` of airtime (at the budget
/// rate) is outstanding and drains at 1 µs per µs; a new transmission waits while the
/// level is above the burst tolerance. Retransmissions consume their airtime over
/// the connection events they occupy (one attempt per CI), not up front, and stop
/// consuming when their link drops.
#[derive(Debug, Clone, Default)]
pub struct AirBucket {
    pub level_us: f64,
    pub last_us: u64,
    /// `(link, start, end, total airtime, charged so far)`.
    pub retx: Vec<(usize, u64, u64, f64, f64)>,
}

impl AirBucket {
    /// Brings the level to `now`: drain, then the retransmission airtime accrued since.
    pub fn update(&mut self, now: u64) {
        let dt = now.saturating_sub(self.last_us) as f64;
        self.level_us = (self.level_us - dt).max(0.0);
        self.last_us = self.last_us.max(now);
        let mut add = 0.0;
        for r in &mut self.retx {
            let (_, a, b, total, charged) = *r;
            let frac = if now >= b {
                1.0
            } else if now <= a {
                0.0
            } else {
                (now - a) as f64 / (b - a).max(1) as f64
            };
            let target = total * frac;
            add += target - charged;
            r.4 = target;
        }
        self.retx.retain(|r| r.2 > now);
        self.level_us += add;
    }

    /// Earliest instant ≥ `now` a transmission may start (level ≤ `depth_us`).
    pub fn ready_at(&self, now: u64, depth_us: f64) -> u64 {
        if self.level_us <= depth_us {
            now
        } else {
            now + (self.level_us - depth_us).ceil() as u64
        }
    }

    /// Forgets the retransmissions of a link that went down.
    pub fn drop_link(&mut self, link: usize) {
        self.retx.retain(|r| r.0 != link);
    }
}

/// Counters of emitted effects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectCounters {
    pub radio: u64,
    pub audio: u64,
    pub ui: u64,
    pub lab_lines: u64,
    pub start_capture: u64,
    pub open_session: u64,
    pub logs: u64,
}

impl SimNode {
    pub(crate) fn new(
        idx: usize,
        spec: NodeSpec,
        engine: Engine,
        tap: TapHandle,
        seed: u64,
    ) -> Self {
        SimNode {
            tap,
            snapshot: None,
            snapshot_at: 0,
            notices: Vec::new(),
            talk_hist: Vec::new(),
            talkers_hist: Vec::new(),
            cap_hist: Vec::new(),
            denied: Vec::new(),
            idx,
            spec,
            engine,
            generation: 0,
            effects: Vec::new(),
            commands: Vec::new(),
            lab_files: BTreeMap::new(),
            counters: EffectCounters::default(),
            store: BTreeMap::new(),
            hello: None,
            discovery: (DiscoveryMode::Off, None),
            scan: mesh_core::ScanIntensity::LowPower,
            psm: 0,
            audio: AudioPlatform {
                session: None,
                capture: false,
                ticking: false,
                capture_blocks: 0,
                render_blocks: 0,
                last_sample: (0, 0, Instant::ZERO),
                playout_peak: 0,
                mic: SpeechSynth::new(seed, idx as u64),
                talk: None,
                in_span: false,
                mouth: None,
                ear: None,
                pcm: None,
            },
            pending: BTreeMap::new(),
            next_timeout: None,
            tick_phase_us: crate::rng::mix(&[seed, 0x71C4, idx as u64]) % 5_000,
            air: AirBucket::default(),
            air_used_us: 0,
            ble_tx_bytes: 0,
        }
    }

    /// Counts (and, when `keep`, records) one effect.
    pub(crate) fn record(&mut self, now: Instant, e: Effect, keep: bool) {
        let c = &mut self.counters;
        match &e {
            Effect::Radio(_) => c.radio += 1,
            Effect::Audio(a) => {
                c.audio += 1;
                match a {
                    AudioCommand::StartCapture => c.start_capture += 1,
                    AudioCommand::OpenSession { .. } => c.open_session += 1,
                    _ => {}
                }
            }
            Effect::Ui(_) => c.ui += 1,
            Effect::LabWrite { .. } => c.lab_lines += 1,
            Effect::Log { .. } => c.logs += 1,
            _ => {}
        }
        if let Effect::LabWrite { run_id, file, line } = &e {
            self.lab_files
                .entry((run_id.clone(), *file == LabFile::Jsonl))
                .or_default()
                .push(line.clone());
        }
        match &e {
            Effect::Ui(UiEvent::Snapshot { s }) => {
                if let Some(sess) = s.session.as_ref()
                    && self.talk_hist.last().map(|x| x.1) != Some(sess.talk)
                {
                    self.talk_hist.push((now.0, sess.talk));
                }
                if self.cap_hist.last().map(|x| x.1) != Some(s.mesh.cap_ble) {
                    self.cap_hist.push((now.0, s.mesh.cap_ble));
                }
                let mut ids: Vec<u16> = s.talkers.iter().map(|t| t.node_id).collect();
                ids.sort_unstable();
                if self.talkers_hist.last().map(|x| &x.1) != Some(&ids) {
                    self.talkers_hist.push((now.0, ids));
                }
                self.snapshot = Some(s.clone());
                self.snapshot_at = now.0;
            }
            Effect::Ui(UiEvent::TalkDenied { reason }) => self.denied.push((now.0, *reason)),
            Effect::Ui(UiEvent::Notice { key, .. }) => self.notices.push((now.0, *key)),
            _ => {}
        }
        if let Effect::Persist { key, value } = &e {
            match value {
                Some(v) => {
                    self.store.insert(key.clone(), v.clone());
                }
                None => {
                    self.store.remove(key);
                }
            }
        }
        if let Effect::LabDelete { run_id } = &e {
            self.lab_files.retain(|(r, _), _| r != run_id);
        }
        if keep {
            self.effects.push((now, e));
        }
    }

    /// labrun/1 lines of `run_id` (JSONL file).
    pub fn lab_lines(&self, run_id: &str) -> &[String] {
        self.lab_files
            .get(&(run_id.to_string(), true))
            .map_or(&[], Vec::as_slice)
    }

    /// Summary CSV lines of `run_id`.
    pub fn lab_csv(&self, run_id: &str) -> &[String] {
        self.lab_files
            .get(&(run_id.to_string(), false))
            .map_or(&[], Vec::as_slice)
    }

    /// Run ids with a JSONL file.
    pub fn lab_runs(&self) -> Vec<String> {
        self.lab_files
            .keys()
            .filter(|(_, jsonl)| *jsonl)
            .map(|(r, _)| r.clone())
            .collect()
    }

    /// UI events emitted so far.
    pub fn ui_events(&self) -> impl Iterator<Item = &UiEvent> {
        self.effects.iter().filter_map(|(_, e)| match e {
            Effect::Ui(ev) => Some(ev),
            _ => None,
        })
    }

    /// Audio commands emitted so far, with their time.
    pub fn audio_commands(&self) -> Vec<(Instant, AudioCommand)> {
        self.effects
            .iter()
            .filter_map(|(t, e)| match e {
                Effect::Audio(a) => Some((*t, *a)),
                _ => None,
            })
            .collect()
    }

    /// Radio commands emitted so far.
    pub fn radio_commands(&self) -> impl Iterator<Item = &RadioCommand> {
        self.effects.iter().filter_map(|(_, e)| match e {
            Effect::Radio(c) => Some(c),
            _ => None,
        })
    }

    /// The Lab node id, in Lab mode.
    pub fn lab_node_id(&self) -> Option<u16> {
        self.engine.lab_node_id()
    }

    /// Up links of the engine (BLE only when `ble`).
    pub fn active_links(&self, ble: bool) -> usize {
        self.engine.ctx().links.active_count(ble)
    }

    /// Engine link ids in the Up state.
    pub fn up_links(&self) -> Vec<LinkId> {
        self.engine.ctx().links.up_ids()
    }
}
