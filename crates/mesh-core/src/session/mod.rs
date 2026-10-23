//! Ride session (DESIGN §1.3 F3-F8, §3.2, §5.8, §7.8, owned by WP-10B): lobby/riding,
//! go-live and the capture gating, talk mode, signed RIDE_CTL / LEAVE, the L0-L3
//! degradation ladder from `AudioIo`, Bluetooth radio state (cue, notice, PRESENCE
//! `radio_off_recently`), the audio-check lifecycle, PRESENCE origination (14 B every
//! 2 s), POSITION while sharing, the reachability view with voice-reach classes and the
//! notices aggregated over 5 s.
//!
//! The session is driven by the group module (`crate::group::core`), which owns the
//! links and the flood; session functions return the mesh packets to originate. What
//! the media module must know (group identity, member names, the gated talk mode,
//! cues, the end of a session) is posted on the engine's inter-module bus
//! (`Ctx::post_media`; see `GroupCore::sync_media`, [`cue`] and [`audio`]).

pub mod audio;
pub mod cue;
pub mod presence;
pub mod ride;
pub mod view;

use std::collections::{BTreeMap, BTreeSet};

use mesh_api::{LadderLevel, RidePhase, TalkMode, Vehicle};
use mesh_routing::{Fix, Reachability};
use mesh_wire::mesh::PacketType;

use crate::{Instant, LinkId};

pub use audio::AudioCheck;
pub use cue::Cue;

/// PRESENCE period (§4.6.1, Appendix A).
pub const PRESENCE_INTERVAL_US: u64 = 2_000_000;
/// POSITION period while moving / stopped (§4.6).
pub const POSITION_MOVING_US: u64 = 5_000_000;
pub const POSITION_STOPPED_US: u64 = 20_000_000;
/// Speed above which a rider counts as moving.
pub const MOVING_MPS: f64 = 1.0;
/// PRESENCE `radio_off_recently`: Bluetooth was off within this window (§4.6.1).
pub const RADIO_OFF_RECENT_US: u64 = 60_000_000;
/// Repeat of the Bluetooth-off cue while the radio stays off (§6.5).
pub const RADIO_CUE_REPEAT_US: u64 = 60_000_000;
/// Aggregation window of member notices (§5.8).
pub const NOTICE_AGGREGATION_US: u64 = 5_000_000;
/// Battery level below which the `batteryLow` cue plays (§1.3 F4).
pub const BATTERY_LOW: f64 = 0.15;
/// Spacing of the `batteryLow` cue.
pub const BATTERY_CUE_INTERVAL_US: u64 = 600_000_000;

/// One mesh packet the session wants originated (flooded on every Up link).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Out {
    pub ptype: PacketType,
    pub payload: Vec<u8>,
}

/// Names gathered for one aggregated notice.
#[derive(Debug, Clone, Default)]
pub struct Aggregate {
    pub names: Vec<String>,
    pub since: Option<Instant>,
}

impl Aggregate {
    pub fn push(&mut self, now: Instant, name: String) {
        self.names.push(name);
        self.since.get_or_insert(now);
    }

    /// When the window of the names gathered so far elapses.
    pub fn deadline(&self) -> Option<Instant> {
        self.since.map(|t| t.plus_us(NOTICE_AGGREGATION_US))
    }

    /// Takes the names once the window elapsed.
    pub fn due(&mut self, now: Instant) -> Option<Vec<String>> {
        let since = self.since?;
        (now.since(since) >= NOTICE_AGGREGATION_US).then(|| {
            self.since = None;
            core::mem::take(&mut self.names)
        })
    }
}

/// State of one running group session.
#[derive(Debug, Clone)]
pub struct Session {
    pub me: u16,
    /// Went live (Go live): capture connected.
    pub live: bool,
    /// Talk mode chosen by the user, if any (otherwise the vehicle default at ride start).
    pub mode: Option<TalkMode>,
    pub vehicle: Vehicle,
    pub ride: RidePhase,
    /// `ride_id` of the current ride (RIDE_CTL start).
    pub ride_id: Option<u32>,
    pub ladder: LadderLevel,
    pub on_call: bool,
    /// Bluetooth availability as the session last saw it.
    pub ble_on: bool,
    /// When Bluetooth came back (PRESENCE `radio_off_recently` for 60 s).
    pub radio_back_at: Option<Instant>,
    /// Last Bluetooth-off cue (repeated every 60 s while off).
    pub radio_cue_at: Option<Instant>,
    /// Talk mode last posted to the media module while live (`None`: not live yet).
    pub media_mode: Option<TalkMode>,
    /// Headset presence as last reported by the audio route.
    pub headset: Option<bool>,
    /// Last battery-low cue.
    pub battery_cue_at: Option<Instant>,
    pub share: bool,
    pub next_presence: Instant,
    pub next_position: Instant,
    pub reach: Reachability,
    /// Latest POSITION fix per member.
    pub positions: BTreeMap<u16, Fix>,
    pub joined: Aggregate,
    pub left: Aggregate,
    /// Roster members already announced (memberJoined diffs).
    pub known: BTreeSet<u16>,
    /// Members that sent a verified LEAVE (until they are heard again).
    pub gone: BTreeSet<u16>,
    /// Direct links counted in PRESENCE `links` (BLE, Aware).
    pub link_kinds: BTreeMap<LinkId, crate::LinkKind>,
}

impl Session {
    pub fn new(me: u16, vehicle: Vehicle, now: Instant, voice_ttl: u8) -> Self {
        Session {
            me,
            live: false,
            mode: None,
            vehicle,
            ride: RidePhase::Lobby,
            ride_id: None,
            ladder: LadderLevel::L0,
            on_call: false,
            ble_on: true,
            radio_back_at: None,
            radio_cue_at: None,
            media_mode: None,
            headset: None,
            battery_cue_at: None,
            share: false,
            next_presence: now,
            next_position: now,
            reach: Reachability::new(
                me,
                now.routing(),
                voice_ttl,
                mesh_routing::consts::VOICE_MAX_AGE_US,
            ),
            positions: BTreeMap::new(),
            joined: Aggregate::default(),
            left: Aggregate::default(),
            known: BTreeSet::new(),
            gone: BTreeSet::new(),
            link_kinds: BTreeMap::new(),
        }
    }

    /// Carries the rider's ride state over from the session of a replaced identity
    /// (§8.2 regeneration): live capture, talk mode, ride, ladder, call and radio state,
    /// sharing. Reachability, positions and member bookkeeping start afresh.
    pub fn carry_over(&mut self, old: &Session) {
        self.live = old.live;
        self.mode = old.mode;
        self.ride = old.ride;
        self.ride_id = old.ride_id;
        self.ladder = old.ladder;
        self.on_call = old.on_call;
        self.ble_on = old.ble_on;
        self.radio_back_at = old.radio_back_at;
        self.radio_cue_at = old.radio_cue_at;
        self.headset = old.headset;
        self.battery_cue_at = old.battery_cue_at;
        self.share = old.share;
    }

    /// Talk mode in force: muted in the lobby (capture starts muted, §1.3 F3), then the
    /// user's choice or the vehicle default (moto VOX, car PTT, §1.4).
    pub fn effective_mode(&self) -> TalkMode {
        match (self.ride, self.mode) {
            (_, Some(TalkMode::Muted)) => TalkMode::Muted,
            (RidePhase::Lobby, Some(m)) => m,
            (RidePhase::Lobby, None) => self.default_mode(),
            (RidePhase::Riding, Some(m)) => m,
            (RidePhase::Riding, None) => self.default_mode(),
        }
    }

    pub fn default_mode(&self) -> TalkMode {
        match self.vehicle {
            Vehicle::Moto => TalkMode::Vox,
            Vehicle::Car => TalkMode::Ptt,
        }
    }

    /// Talk state for the snapshot and PRESENCE: muted until live and riding, while the
    /// mode is Muted, or while the mic is lost (ladder L2/L3); otherwise idle (the
    /// media module owns talking/blocked).
    pub fn talk_muted(&self) -> bool {
        !self.live
            || self.ride == RidePhase::Lobby
            || self.effective_mode() == TalkMode::Muted
            || matches!(self.ladder, LadderLevel::L2 | LadderLevel::L3)
    }

    /// Bluetooth was off within the last 60 s (or is off now).
    pub fn radio_off_recently(&self, now: Instant) -> bool {
        !self.ble_on
            || self
                .radio_back_at
                .is_some_and(|t| now.since(t) < RADIO_OFF_RECENT_US)
    }

    /// Next instant the session needs a timer: PRESENCE, POSITION, the Bluetooth-off
    /// repeat and the end of the notice aggregation windows are exact deadlines;
    /// reachability (member lost, alone) is evaluated at least every 500 ms.
    pub fn next_deadline(&self, now: Instant) -> Instant {
        let radio = self
            .radio_cue_at
            .filter(|_| !self.ble_on)
            .map(|t| t.plus_us(RADIO_CUE_REPEAT_US));
        let position = self.share.then_some(self.next_position);
        [
            Some(self.next_presence),
            Some(now.plus_ms(500)),
            position,
            radio,
            self.joined.deadline(),
            self.left.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(now)
        .max(now.plus_us(1))
    }
}
