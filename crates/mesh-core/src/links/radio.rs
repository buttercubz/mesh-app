//! Radio decisions for discovery and connection (DESIGN §3.2, §5.7, §9.2-§9.5): HELLO
//! and advertising payloads, which discovered peer to connect (role rules, one
//! outstanding connect per transport, 8 s timeout, per-peer backoff 5 s → 60 s), the
//! HELLO check (`OpenLink` or `Abort`), the BLE link target/max with the `LimitReached`
//! backoff, and Bluetooth on/off handling. Used by Lab mode (WP-10A); group mode reuses
//! it with a group target supplied by WP-10B.
//!
//! §5.7 hooks driven by the mesh glue from its reachability view: candidates connect in
//! class order — an advertising token of an unreachable roster member, then an unknown
//! token, then a reachable member — by RSSI within a class
//! ([`RadioPolicy::set_member_tokens`]).
//!
//! Discovery duty cycle (§5.7, §9.5): scanning shares the controller with every
//! connection event, and a phone that scans continuously while it carries links starves
//! them into supervision timeouts (first field test: three phones at a table split the
//! group). So the policy hunts only when there is something to find — a roster member
//! unreachable ([`RadioPolicy::set_force_forming`]), or under the link target with an
//! unlinked peer of ours heard advertising in the last [`WANTED_FRESH_US`] — and then in
//! bounded windows: a first [`HUNT_FIRST_US`] hunt, then [`HUNT_US`] hunts separated by
//! growing quiet periods ([`QUIET_US`]). Every member reachable is `Stable` whatever
//! the link count. The chosen [`ScanIntensity`] goes to the platform with the mode:
//! `LowLatency` only with no link up, `Balanced` while hunting with links, `LowPower`
//! otherwise.

use std::collections::{BTreeMap, BTreeSet};

pub use mesh_routing::CandidateClass;

use mesh_api::PeerPlatform;
use mesh_wire::adv::{
    AdvFlags, LocalNameInfo, ServiceData, android_pair_initiates, ios_local_name,
    ios_pair_initiates, token_prefix14,
};
use mesh_wire::hello::{Hello, HelloFlags, lab_group_tag};

use super::types::{AdvData, Discovery, LinkDownReason, LinkInfo, RadioAvail, Transport};
use crate::config::{Limits, Platform};
use crate::effects::{
    AdvPayload, DiscoveryMode, EffectQueue, OpenVia, RadioCommand, ScanIntensity,
};
use mesh_lab::ridelog::DiagEvent;

use crate::{Instant, LinkId};

/// What the node advertises and looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioTarget {
    /// Lab peers with this `lab_tag`.
    Lab { lab_tag: u16 },
    /// Group peers whose 4-byte `group_tag` is current (set by the group module).
    Group { group_tag: [u8; 4], epoch: u32 },
}

impl RadioTarget {
    fn group_tag(self) -> [u8; 4] {
        match self {
            RadioTarget::Lab { lab_tag } => lab_group_tag(lab_tag),
            RadioTarget::Group { group_tag, .. } => group_tag,
        }
    }
}

/// Peer identity as far as advertising shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeerKey {
    /// Full `node_token` (Android scan response).
    Token([u8; 4]),
    /// 14-bit token prefix (iPhone foreground local name).
    Prefix(u16),
    /// Background iPhone: only the discovery id is known.
    Disc(u32),
}

/// Preferred data path after the HELLO check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferVia {
    L2cap,
    Gatt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    disc_id: u32,
    key: PeerKey,
    since: Instant,
    /// Last advertisement of the target (at or before the connect, refreshed by
    /// rediscoveries while the connect is outstanding).
    seen: Instant,
    hello_ok: bool,
    /// The data path asked for after the HELLO check.
    via: Option<OpenVia>,
}

/// Discovery duty cycle state while there is something to hunt for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunt {
    /// `true` inside a hunt window, `false` in a quiet period.
    hunting: bool,
    /// End of the current window or quiet period.
    phase_end: Instant,
    /// Quiet periods completed (index into [`QUIET_US`]).
    step: usize,
    /// Start of the last hunt window opened by a lost member's advertisement.
    evidence_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Candidate {
    key: PeerKey,
    rssi: i16,
    seen: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Backoff {
    until: Instant,
    next_us: u64,
    /// Failed attempts so far (diagnostics: the attempt number of the next try).
    tries: u16,
}

/// Discovery candidates expire after this long without a new advertisement.
const CANDIDATE_TTL_US: u64 = 10_000_000;

/// A candidate advertised within this window is *fresh*: evidence that the peer is in
/// range now. Scanning in `Forming` reports a peer in range several times per second
/// (iOS `AllowDuplicates`, Android low-latency scan), so an older advertisement most
/// likely belongs to a peer that has left (typically the neighbor whose link just
/// died). Fresh candidates connect before stale ones, and an outstanding connect to a
/// stale peer that has not answered yet yields to a fresh candidate instead of holding
/// the single connect slot for the whole 8 s timeout (§5.7, §9.5): a connect to an
/// out-of-range peer never completes by itself (iOS `connect` does not time out).
pub const CANDIDATE_FRESH_US: u64 = 3_000_000;

/// A peer whose advertisements were absent this long and come back has *reappeared*:
/// its per-peer backoff is reset (§5.7 resets it on a new advertising token; a peer
/// that left and came back is new presence too). Its connects most likely failed only
/// because it was out of range, and a backoff grown to 60 s over a long separation
/// must not delay the rejoin (§9.5).
pub const REAPPEAR_GAP_US: u64 = 6_000_000;

/// The spare slot (see [`RadioPolicy::spare_slot`]) stays open this long after the
/// last advertisement of an unreachable member: the member's own connect may come a
/// few seconds after we heard it (its discovery of us, its connect slot, a backoff).
pub const SPARE_HOLD_US: u64 = CANDIDATE_FRESH_US;

/// A peer of ours that answered "not accepting" (at its link max) is retried after
/// this fixed hold, without growing its per-peer backoff: a full neighbor frees a slot
/// as soon as one of its far links dies or its 2 s selection closes a redundant one,
/// and a doubled backoff (10 s, 20 s) would keep an isolated rider out long after
/// (§9.5 rejoin, mesh-sim S4).
pub const FULL_RETRY_US: u64 = CANDIDATE_FRESH_US;

/// Under the link target, an unlinked peer of ours heard advertising this recently is
/// worth hunting for (§5.7 "under target").
pub const WANTED_FRESH_US: u64 = 30_000_000;
/// The first hunt window after the need appears (most rejoins and formations land in
/// it).
pub const HUNT_FIRST_US: u64 = 30_000_000;
/// Later hunt windows.
pub const HUNT_US: u64 = 10_000_000;
/// Quiet periods between hunt windows (the last one repeats), each stretched by a
/// per-node factor in [0.75, 1.25) so the nodes of one side do not hunt in lockstep.
pub const QUIET_US: [u64; 3] = [10_000_000, 20_000_000, 40_000_000];
/// A lost member heard advertising during a quiet period opens a [`HUNT_US`] window
/// (fast advertising helps it find us), at most once per this period.
pub const EVIDENCE_GAP_US: u64 = 30_000_000;

/// The radio policy state.
#[derive(Debug, Clone)]
pub struct RadioPolicy {
    platform: Platform,
    pub ble: RadioAvail,
    pub aware: RadioAvail,
    target: Option<RadioTarget>,
    token: [u8; 4],
    psm: u16,
    prefer: PreferVia,
    leader: bool,
    pending: Option<Pending>,
    candidates: BTreeMap<u32, Candidate>,
    backoff: BTreeMap<PeerKey, Backoff>,
    ignored: BTreeMap<u32, Instant>,
    limit_until: Vec<Instant>,
    linked: BTreeMap<LinkId, PeerKey>,
    sent_mode: Option<(DiscoveryMode, Option<AdvPayload>, ScanIntensity)>,
    sent_hello: Option<[u8; 16]>,
    /// Advertising tokens of roster members unreachable now / reachable now (§5.7).
    unreachable_tokens: BTreeSet<[u8; 4]>,
    reachable_tokens: BTreeSet<[u8; 4]>,
    /// Discovery stays `Forming` whatever the link count (a member is unreachable).
    force_forming: bool,
    /// Advertising tokens (slot window) of lost roster members: unreachable now but
    /// heard before in this session (a rejoin, not the first meeting).
    lost_tokens: BTreeSet<[u8; 4]>,
    /// Last advertisement of a lost roster member (whichever side initiates).
    missing_seen: Option<Instant>,
    /// Advertising tokens (slot window) of the members I have a BLE link to (§5.7
    /// glue): never a connect candidate, whatever token the link was opened with.
    neighbor_tokens: BTreeSet<[u8; 4]>,
    /// Platform links opened for a connect, waiting for the handshake (link Up).
    opening: BTreeMap<LinkId, PeerKey>,
    /// Last advertisement per discovery id (candidate or not), for reappearance.
    last_adv: BTreeMap<u32, Instant>,
    /// Last advertisement of an unlinked peer of ours (any class, either role).
    wanted_seen: Option<Instant>,
    /// Discovery duty cycle while there is something to hunt for.
    hunt: Option<Hunt>,
    /// Peers (full token) whose L2CAP open failed: the next links use GATT (§9.2
    /// "L2CAP, GATT fallback").
    gatt_peers: BTreeSet<[u8; 4]>,
    /// Voice is flowing through this node (a live floor entry or a voice link).
    voice_active: bool,
    /// Diagnostics events (moved into `Effect::Diag` by the engine).
    pub(crate) diag: crate::diag::DiagBuf,
    /// Last `peerSeen` written per discovery id (at most one per [`PEER_SEEN_US`]).
    diag_seen: BTreeMap<u32, Instant>,
}

/// `peerSeen` diagnostics are written at most once per discovery id per this period.
pub const PEER_SEEN_US: u64 = 10_000_000;

/// Static code of a discovery mode.
pub const fn mode_str(m: DiscoveryMode) -> &'static str {
    match m {
        DiscoveryMode::Off => "off",
        DiscoveryMode::Forming => "forming",
        DiscoveryMode::Stable => "stable",
    }
}

/// Static code and nominal duty cycle of a scan intensity.
pub const fn scan_str(s: ScanIntensity) -> (&'static str, f32) {
    match s {
        ScanIntensity::LowLatency => ("lowLatency", 1.0),
        ScanIntensity::Balanced => ("balanced", 0.25),
        ScanIntensity::LowPower => ("lowPower", 0.1),
    }
}

impl RadioPolicy {
    pub fn new(platform: Platform) -> Self {
        RadioPolicy {
            platform,
            ble: RadioAvail::On,
            aware: RadioAvail::Unsupported,
            target: None,
            token: [0; 4],
            psm: 0,
            prefer: PreferVia::L2cap,
            leader: false,
            pending: None,
            candidates: BTreeMap::new(),
            backoff: BTreeMap::new(),
            ignored: BTreeMap::new(),
            limit_until: Vec::new(),
            linked: BTreeMap::new(),
            sent_mode: None,
            sent_hello: None,
            unreachable_tokens: BTreeSet::new(),
            reachable_tokens: BTreeSet::new(),
            force_forming: false,
            missing_seen: None,
            lost_tokens: BTreeSet::new(),
            neighbor_tokens: BTreeSet::new(),
            opening: BTreeMap::new(),
            last_adv: BTreeMap::new(),
            wanted_seen: None,
            hunt: None,
            gatt_peers: BTreeSet::new(),
            voice_active: false,
            diag: crate::diag::DiagBuf::default(),
            diag_seen: BTreeMap::new(),
        }
    }

    /// Starts discovery for `target` with this node's advertising token.
    pub fn start(&mut self, target: RadioTarget, token: [u8; 4], fx: &mut EffectQueue) {
        self.target = Some(target);
        self.token = token;
        self.pending = None;
        self.candidates.clear();
        self.backoff.clear();
        self.ignored.clear();
        self.last_adv.clear();
        self.missing_seen = None;
        self.wanted_seen = None;
        self.hunt = None;
        self.gatt_peers.clear();
        self.sent_mode = None;
        self.sent_hello = None;
        self.refresh(0, &Limits::V1, Instant::ZERO, fx);
    }

    /// Stops discovery (engine stop).
    pub fn stop(&mut self, fx: &mut EffectQueue) {
        self.target = None;
        self.pending = None;
        self.candidates.clear();
        self.linked.clear();
        self.unreachable_tokens.clear();
        self.reachable_tokens.clear();
        self.force_forming = false;
        self.missing_seen = None;
        self.neighbor_tokens.clear();
        self.lost_tokens.clear();
        self.opening.clear();
        self.wanted_seen = None;
        self.hunt = None;
        self.emit_mode(DiscoveryMode::Off, ScanIntensity::LowPower, fx);
    }

    /// §5.7 candidate classes: the advertising tokens (slot window and retained epochs)
    /// of roster members that are unreachable now and of those that are reachable.
    pub fn set_member_tokens(
        &mut self,
        unreachable: BTreeSet<[u8; 4]>,
        reachable: BTreeSet<[u8; 4]>,
    ) {
        self.unreachable_tokens = unreachable;
        self.reachable_tokens = reachable;
    }

    /// Advertising tokens (slot window) of the members I have a BLE link to: their
    /// advertisements are never connect candidates (§5.7: no second link to a
    /// neighbor; the token a link was opened with rotates every 10-minute slot).
    pub fn set_neighbor_tokens(&mut self, tokens: BTreeSet<[u8; 4]>) {
        self.neighbor_tokens = tokens;
    }

    /// Advertising tokens (slot window) of lost roster members: unreachable now, heard
    /// before in this session. Both sides of a same-OS pair may initiate toward them,
    /// and a node at its BLE max keeps a spare slot while one advertises in range
    /// ([`RadioPolicy::spare_slot`]).
    pub fn set_lost_tokens(&mut self, tokens: BTreeSet<[u8; 4]>) {
        self.lost_tokens = tokens;
    }

    /// `true` when `key` may name a lost roster member.
    fn is_lost(&self, key: PeerKey) -> bool {
        self.lost_tokens
            .iter()
            .any(|t| same_peer(PeerKey::Token(*t), key))
    }

    /// A roster member is unreachable while `on` (§5.7 "or some roster member
    /// unreachable", §5.8): discovery hunts for it, duty-cycled. Returns `true` when it
    /// changed; the caller re-evaluates the mode
    /// ([`crate::engine::Ctx::refresh_radio`]).
    pub fn set_force_forming(&mut self, on: bool) -> bool {
        let changed = self.force_forming != on;
        self.force_forming = on;
        changed
    }

    /// Voice is flowing through this node (or stopped). Returns `true` when it changed;
    /// the caller re-evaluates the HELLO ([`crate::engine::Ctx::refresh_radio`]).
    pub fn set_voice_active(&mut self, on: bool) -> bool {
        let changed = self.voice_active != on;
        self.voice_active = on;
        changed
    }

    /// `true` while a roster member is unreachable.
    pub fn force_forming(&self) -> bool {
        self.force_forming
    }

    /// The §5.7 class of a discovered peer: a token of an unreachable roster member
    /// first, then an unknown token (a background iPhone advertises none), then a token
    /// of a reachable member. A 14-bit prefix (iPhone local name) matches a token with
    /// that prefix.
    pub fn class_of(&self, key: PeerKey) -> CandidateClass {
        let hit = |set: &BTreeSet<[u8; 4]>| match key {
            PeerKey::Token(t) => set.contains(&t),
            PeerKey::Prefix(p) => set.iter().any(|t| token_prefix14(t) == p),
            PeerKey::Disc(_) => false,
        };
        if hit(&self.unreachable_tokens) {
            CandidateClass::UnreachableMember
        } else if hit(&self.reachable_tokens) {
            CandidateClass::ReachableMember
        } else {
            CandidateClass::Unknown
        }
    }

    pub fn target(&self) -> Option<RadioTarget> {
        self.target
    }

    pub fn token(&self) -> [u8; 4] {
        self.token
    }

    /// Prefers L2CAP or GATT for the next links (Lab transport variants).
    pub fn set_prefer(&mut self, prefer: PreferVia) {
        self.prefer = prefer;
    }

    /// HELLO `flags.leader` (group mode).
    pub fn set_leader(&mut self, leader: bool) {
        self.leader = leader;
    }

    /// The local L2CAP PSM was published (0 = GATT only).
    pub fn set_psm(&mut self, psm: u16) {
        self.psm = psm;
        self.sent_hello = None;
    }

    /// BLE link max now: the configured max lowered by 1 per active `LimitReached`
    /// backoff (floor 1, §3.2).
    pub fn ble_max(&self, limits: &Limits, now: Instant) -> usize {
        let active = self.limit_until.iter().filter(|t| **t > now).count();
        usize::from(limits.ble_max()).saturating_sub(active).max(1)
    }

    fn hello(&self, accepting: bool) -> Option<[u8; 16]> {
        let flags = HelloFlags::new(
            self.platform == Platform::Android,
            false,
            accepting,
            self.aware == RadioAvail::On,
            self.leader,
        );
        let h = match self.target? {
            RadioTarget::Lab { lab_tag } => Hello::lab(flags, self.psm, lab_tag, self.token),
            RadioTarget::Group { group_tag, epoch } => {
                Hello::group(flags, self.psm, group_tag, self.token, epoch)
            }
        };
        h.encode().ok()
    }

    fn adv(&self) -> Option<AdvPayload> {
        let target = self.target?;
        let group_tag = target.group_tag();
        let sd = ServiceData {
            flags: AdvFlags::new(
                self.platform == Platform::Android,
                matches!(target, RadioTarget::Lab { .. }),
            ),
            group_tag,
            node_token: self.token,
        };
        Some(AdvPayload {
            service_data: sd.encode(),
            local_name: ios_local_name(&group_tag, &self.token),
        })
    }

    fn emit_mode(&mut self, mode: DiscoveryMode, scan: ScanIntensity, fx: &mut EffectQueue) {
        let adv = if mode == DiscoveryMode::Off {
            None
        } else {
            self.adv()
        };
        let next = (mode, adv, scan);
        if self.sent_mode.as_ref() != Some(&next) {
            if self.sent_mode.as_ref().map(|m| (m.0, m.2)) != Some((mode, scan)) {
                let (code, duty) = scan_str(scan);
                self.diag.push(DiagEvent::DiscoveryMode {
                    mode: mode_str(mode),
                    scan: Some(code),
                    adv: Some(if next.1.is_some() { "on" } else { "off" }),
                    duty: Some(if mode == DiscoveryMode::Off { 0.0 } else { duty }),
                });
            }
            fx.radio(RadioCommand::SetDiscovery {
                mode: next.0,
                adv: next.1.clone(),
                scan,
            });
            self.sent_mode = Some(next);
        }
    }

    /// `true` when there is something to hunt for (module docs).
    fn need_hunt(&self, ble_links: usize, limits: &Limits, now: Instant) -> bool {
        let under = ble_links < usize::from(limits.ble_target());
        match self.target {
            Some(RadioTarget::Lab { .. }) => under,
            Some(RadioTarget::Group { .. }) => {
                self.force_forming
                    || (under
                        && self
                            .wanted_seen
                            .is_some_and(|t| now.since(t) < WANTED_FRESH_US))
            }
            None => false,
        }
    }

    /// Moves the duty cycle to `now` (starting or ending it with the need).
    fn advance_hunt(&mut self, need: bool, now: Instant) {
        if !need {
            self.hunt = None;
            return;
        }
        let h = self.hunt.get_or_insert(Hunt {
            hunting: true,
            phase_end: now.plus_us(HUNT_FIRST_US),
            step: 0,
            evidence_at: None,
        });
        while h.phase_end <= now {
            if h.hunting {
                let base = QUIET_US[h.step.min(QUIET_US.len() - 1)];
                h.hunting = false;
                h.phase_end = h.phase_end.plus_us(jitter(&self.token, h.step, base));
                h.step += 1;
            } else {
                h.hunting = true;
                h.phase_end = h.phase_end.plus_us(HUNT_US);
            }
        }
    }

    /// A lost member advertises while the duty cycle is quiet: hunt now (bounded by
    /// [`EVIDENCE_GAP_US`]).
    fn evidence(&mut self, now: Instant) {
        if let Some(h) = self.hunt.as_mut()
            && !h.hunting
            && h.evidence_at
                .is_none_or(|t| now.since(t) >= EVIDENCE_GAP_US)
        {
            h.hunting = true;
            h.phase_end = now.plus_us(HUNT_US);
            h.evidence_at = Some(now);
        }
    }

    /// The scan intensity now published (`None` before the first `SetDiscovery`).
    pub fn scan_intensity(&self) -> Option<ScanIntensity> {
        self.sent_mode.as_ref().map(|m| m.2)
    }

    /// Re-evaluates HELLO and discovery mode for `ble_links` active BLE links.
    pub fn refresh(
        &mut self,
        ble_links: usize,
        limits: &Limits,
        now: Instant,
        fx: &mut EffectQueue,
    ) {
        if self.target.is_none() {
            return;
        }
        let max = self.ble_max(limits, now);
        // The spare slot takes one link: an outstanding connect of mine at the max
        // already uses it.
        let accepting =
            ble_links < max || (ble_links == max && self.spare_slot(now) && self.pending.is_none());
        if let Some(h) = self.hello(accepting)
            && self.sent_hello != Some(h)
        {
            fx.radio(RadioCommand::SetHello { bytes: h });
            self.sent_hello = Some(h);
        }
        let need = self.need_hunt(ble_links, limits, now);
        self.advance_hunt(need, now);
        let hunting = self.hunt.is_some_and(|h| h.hunting);
        let (mode, scan) = if self.ble != RadioAvail::On {
            (DiscoveryMode::Off, ScanIntensity::LowPower)
        } else if need && ble_links == 0 {
            // Nothing to starve: scan as fast as the platform can.
            (DiscoveryMode::Forming, ScanIntensity::LowLatency)
        } else if need && hunting {
            (DiscoveryMode::Forming, ScanIntensity::Balanced)
        } else {
            (DiscoveryMode::Stable, ScanIntensity::LowPower)
        };
        self.emit_mode(mode, scan, fx);
    }
}

/// `base` stretched by a per-node factor in [0.75, 1.25), stable for a token and step.
fn jitter(token: &[u8; 4], step: usize, base: u64) -> u64 {
    let mut h: u32 = 0x811c_9dc5;
    for b in token.iter().copied().chain((step as u32).to_le_bytes()) {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    let frac = u64::from(h % 1_000);
    base * (750 + frac / 2) / 1_000
}

/// `true` when two advertising keys may name the same peer (a full token and its
/// 14-bit prefix match).
fn same_peer(a: PeerKey, b: PeerKey) -> bool {
    match (a, b) {
        (PeerKey::Token(t), PeerKey::Prefix(p)) | (PeerKey::Prefix(p), PeerKey::Token(t)) => {
            token_prefix14(&t) == p
        }
        _ => a == b,
    }
}

impl RadioPolicy {
    /// Parses the advertising data into a peer key, if it is one of ours, with the §9.2
    /// role (`Some(true)` I initiate, `Some(false)` the peer does, `None` either side)
    /// and whether that role comes from the same-OS token order (as opposed to the
    /// platform rule "Android never initiates toward an iPhone").
    fn classify(&self, d: &Discovery) -> Option<(PeerKey, Option<bool>, bool)> {
        let target = self.target?;
        let tag = target.group_tag();
        let lab = matches!(target, RadioTarget::Lab { .. });
        let me_android = self.platform == Platform::Android;
        match &d.adv {
            AdvData::ServiceData(b) => {
                let sd = if b.len() == mesh_wire::adv::SERVICE_DATA_LEN {
                    ServiceData::decode(b).ok()
                } else {
                    ServiceData::find_in(b).ok().flatten()
                }?;
                if sd.flags.lab() != lab || (lab && sd.group_tag != tag) {
                    return None;
                }
                let same_os = sd.flags.android() == me_android;
                let initiate = if sd.flags.android() && !me_android {
                    Some(true)
                } else if !sd.flags.android() && me_android {
                    // Android never initiates toward an iPhone (§9.2).
                    Some(false)
                } else {
                    android_pair_initiates(&self.token, &sd.node_token)
                };
                Some((PeerKey::Token(sd.node_token), initiate, same_os))
            }
            AdvData::LocalName(name) => {
                let info = LocalNameInfo::parse(name).ok()?;
                if !info.matches_group_tag(&tag) {
                    return None;
                }
                if me_android {
                    return Some((PeerKey::Prefix(info.token_prefix14), Some(false), false));
                }
                let mine = token_prefix14(&self.token);
                if mine == info.token_prefix14 && d.platform_hint == Some(PeerPlatform::Android) {
                    return None;
                }
                Some((
                    PeerKey::Prefix(info.token_prefix14),
                    ios_pair_initiates(&self.token, info.token_prefix14),
                    true,
                ))
            }
            AdvData::UuidOnly => {
                if me_android {
                    None
                } else {
                    Some((PeerKey::Disc(d.disc_id), None, true))
                }
            }
        }
    }

    fn key_linked(&self, key: PeerKey) -> bool {
        self.linked
            .values()
            .chain(self.opening.values())
            .any(|k| same_peer(*k, key))
            || self
                .neighbor_tokens
                .iter()
                .any(|t| same_peer(PeerKey::Token(*t), key))
    }

    /// A peer was discovered.
    pub fn on_discovered(
        &mut self,
        now: Instant,
        d: &Discovery,
        ble_links: usize,
        limits: &Limits,
        fx: &mut EffectQueue,
    ) {
        if d.transport != Transport::Ble || self.ble != RadioAvail::On {
            return;
        }
        let Some((key, initiate, same_os)) = self.classify(d) else {
            return;
        };
        if self.diag.on() {
            self.diag_seen.retain(|_, t| now.since(*t) < PEER_SEEN_US);
            if !self.diag_seen.contains_key(&d.disc_id) {
                self.diag_seen.insert(d.disc_id, now);
                self.diag.push(DiagEvent::PeerSeen {
                    disc: d.disc_id,
                    rssi: d.rssi,
                    group_match: true,
                    epoch_behind: false,
                });
            }
        }
        let back = self
            .last_adv
            .insert(d.disc_id, now)
            .is_none_or(|t| now.since(t) >= REAPPEAR_GAP_US);
        if back {
            self.backoff.retain(|k, _| !same_peer(*k, key));
            self.last_adv
                .retain(|_, t| now.since(*t) < CANDIDATE_TTL_US.max(REAPPEAR_GAP_US));
        }
        let unlinked = !self.key_linked(key)
            && self
                .ignored
                .get(&d.disc_id)
                .is_none_or(|until| *until <= now);
        if unlinked {
            self.wanted_seen = Some(now);
        }
        let missing = self.is_lost(key);
        if missing {
            self.missing_seen = Some(now);
            if unlinked {
                self.evidence(now);
            }
        }
        if missing || unlinked {
            self.refresh(ble_links, limits, now, fx);
        }
        // §9.2 lets one side of a same-OS pair initiate (lower token). Toward a lost
        // roster member (a rejoin) both sides may, as for equal prefixes and
        // backgrounded iPhones: the rejoin then waits for the first of two
        // discoveries, not for one in particular, and a duplicate is resolved after
        // IDPROOF (lower `node_id` initiator wins).
        if initiate == Some(false) && !(missing && same_os) {
            return;
        }
        if let Some(p) = self.pending.as_mut().filter(|p| p.disc_id == d.disc_id) {
            p.seen = now;
        }
        self.candidates.insert(
            d.disc_id,
            Candidate {
                key,
                rssi: d.rssi.unwrap_or(-127),
                seen: now,
            },
        );
        self.try_connect(now, ble_links, limits, fx);
    }

    fn try_connect(
        &mut self,
        now: Instant,
        ble_links: usize,
        limits: &Limits,
        fx: &mut EffectQueue,
    ) {
        if self.ble != RadioAvail::On || self.target.is_none() {
            return;
        }
        let max = self.ble_max(limits, now);
        if ble_links > max || (ble_links == max && !self.spare_slot(now)) {
            return;
        }
        self.candidates
            .retain(|_, c| now.since(c.seen) < CANDIDATE_TTL_US);
        self.ignored.retain(|_, until| *until > now);
        let Some((disc_id, key, seen)) = self.best_candidate(now) else {
            return;
        };
        // At the max, only the spare slot's purpose: a lost member advertising now.
        if ble_links >= max && (!self.is_lost(key) || now.since(seen) >= CANDIDATE_FRESH_US) {
            return;
        }
        if let Some(p) = self.pending {
            // One outstanding connect per transport: a fresh candidate takes the slot
            // only from a connect to a stale peer that has not answered yet.
            let fresh = now.since(seen) < CANDIDATE_FRESH_US;
            if p.hello_ok || !fresh || !self.pending_stale(now) {
                return;
            }
            self.pending = None;
            self.diag.push(DiagEvent::Abort {
                disc: p.disc_id,
                ignore_s: 0,
                why: "stale",
            });
            fx.radio(RadioCommand::Abort {
                disc_id: p.disc_id,
                ignore_s: 0,
            });
        }
        self.candidates.remove(&disc_id);
        let attempt = self.attempt(key);
        self.diag.push(DiagEvent::ConnectTry {
            disc: disc_id,
            via: "hello",
            attempt,
        });
        self.pending = Some(Pending {
            disc_id,
            key,
            since: now,
            seen,
            hello_ok: false,
            via: None,
        });
        fx.radio(RadioCommand::Connect { disc_id });
        if ble_links >= max {
            // The spare slot is taken by this connect: stop accepting.
            self.refresh(ble_links, limits, now, fx);
        }
    }

    /// The candidate to connect next: §5.7 order (class, then the strongest RSSI, then
    /// the lowest disc id) among fresh candidates first, then among stale ones.
    fn best_candidate(&self, now: Instant) -> Option<(u32, PeerKey, Instant)> {
        let pending = self.pending.map(|p| p.disc_id);
        self.candidates
            .iter()
            .filter(|(id, c)| {
                Some(**id) != pending
                    && !self.ignored.contains_key(id)
                    && !self.key_linked(c.key)
                    && !self.backed_off(c.key, now)
            })
            .min_by_key(|(id, c)| {
                (
                    now.since(c.seen) >= CANDIDATE_FRESH_US,
                    self.class_of(c.key),
                    core::cmp::Reverse(c.rssi),
                    **id,
                )
            })
            .map(|(id, c)| (*id, c.key, c.seen))
    }

    /// `true` while a per-peer backoff holds `key` back, under any key form of the peer
    /// (an iPhone is tried by its 14-bit prefix and fails by its full token).
    fn backed_off(&self, key: PeerKey, now: Instant) -> bool {
        self.backoff
            .iter()
            .any(|(k, b)| b.until > now && same_peer(*k, key))
    }

    /// `true` while the outstanding connect has not read the HELLO and its target has
    /// not advertised within [`CANDIDATE_FRESH_US`].
    fn pending_stale(&self, now: Instant) -> bool {
        self.pending
            .is_some_and(|p| !p.hello_ok && now.since(p.seen) >= CANDIDATE_FRESH_US)
    }

    /// Holds `key` back for [`FULL_RETRY_US`] without growing its backoff.
    fn hold(&mut self, now: Instant, key: PeerKey, limits: &Limits) {
        let b = self.backoff.entry(key).or_insert(Backoff {
            until: now,
            next_us: limits.backoff_initial_us,
            tries: 0,
        });
        b.until = b.until.max(now.plus_us(FULL_RETRY_US));
    }

    fn fail(&mut self, now: Instant, key: PeerKey, limits: &Limits) {
        let b = self.backoff.entry(key).or_insert(Backoff {
            until: now,
            next_us: limits.backoff_initial_us,
            tries: 0,
        });
        b.until = now.plus_us(b.next_us);
        b.next_us = (b.next_us * 2).min(limits.backoff_max_us);
        b.tries = b.tries.saturating_add(1);
    }

    /// The attempt number of the next try to `key` (1 = first), for diagnostics.
    fn attempt(&self, key: PeerKey) -> u16 {
        self.backoff
            .iter()
            .filter(|(k, _)| same_peer(**k, key))
            .map(|(_, b)| b.tries)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Records a failed attempt to `key` (after its backoff was updated).
    fn diag_fail(&mut self, now: Instant, disc: u32, key: PeerKey, reason: &'static str) {
        if !self.diag.on() {
            return;
        }
        let (tries, until) = self
            .backoff
            .iter()
            .filter(|(k, _)| same_peer(**k, key))
            .map(|(_, b)| (b.tries, b.until))
            .max()
            .unwrap_or((0, now));
        let backoff_s = until.0.saturating_sub(now.0).div_ceil(1_000_000);
        self.diag.push(DiagEvent::ConnectFail {
            disc,
            reason,
            attempt: tries.max(1),
            backoff_s: u32::try_from(backoff_s).unwrap_or(u32::MAX),
        });
    }

    /// The HELLO of the pending connect was read: `OpenLink` or `Abort` (§9.3).
    pub fn on_hello_read(
        &mut self,
        now: Instant,
        disc_id: u32,
        bytes: &[u8],
        limits: &Limits,
        fx: &mut EffectQueue,
    ) {
        let Some(p) = self.pending.filter(|p| p.disc_id == disc_id) else {
            self.diag.push(DiagEvent::Abort {
                disc: disc_id,
                ignore_s: 0,
                why: "unexpected",
            });
            fx.radio(RadioCommand::Abort {
                disc_id,
                ignore_s: 0,
            });
            return;
        };
        // `ours`: a peer of this ride/bench at a compatible version. `ok`: it can take a
        // link now (accepting, not already linked to us).
        let (ours, ok) = match (Hello::decode(bytes), self.target) {
            (Ok(h), Some(t)) => {
                let lab = matches!(t, RadioTarget::Lab { .. });
                let ours = h.flags.lab() == lab
                    && h.group_tag == t.group_tag()
                    && h.proto_min <= mesh_wire::PROTOCOL_VERSION
                    && mesh_wire::PROTOCOL_VERSION <= h.proto_max;
                let free =
                    h.flags.accepting() && (lab || !self.key_linked(PeerKey::Token(h.node_token)));
                (ours, ours && free)
            }
            _ => (false, false),
        };
        if ok {
            let h = Hello::decode(bytes).ok();
            let psm = h.map_or(0, |h| h.psm);
            // §9.2 "L2CAP, GATT fallback": a peer whose L2CAP open failed gets GATT.
            let l2cap_ok = h.is_some_and(|h| !self.gatt_peers.contains(&h.node_token));
            let via = if psm != 0 && self.prefer == PreferVia::L2cap && l2cap_ok {
                OpenVia::L2cap { psm }
            } else {
                OpenVia::Gatt
            };
            let code = match via {
                OpenVia::L2cap { .. } => "l2cap",
                OpenVia::Gatt if psm != 0 && self.prefer == PreferVia::L2cap => "gattFallback",
                OpenVia::Gatt => "gatt",
            };
            let attempt = self.attempt(p.key);
            self.diag.push(DiagEvent::ConnectTry {
                disc: disc_id,
                via: code,
                attempt,
            });
            if let (Some(h), Some(pp)) = (h, self.pending.as_mut()) {
                pp.hello_ok = true;
                pp.key = PeerKey::Token(h.node_token);
                pp.via = Some(via);
            }
            fx.radio(RadioCommand::OpenLink { disc_id, via });
        } else if ours {
            // A peer of ours that is full (or already linked to us) is a transient
            // condition: retried after [`FULL_RETRY_US`], instead of the 60 s ignore
            // meant for peers that will never match.
            self.pending = None;
            self.hold(now, p.key, limits);
            self.diag.push(DiagEvent::Abort {
                disc: disc_id,
                ignore_s: 0,
                why: "full",
            });
            fx.radio(RadioCommand::Abort {
                disc_id,
                ignore_s: 0,
            });
        } else {
            self.pending = None;
            self.ignored.insert(
                disc_id,
                now.plus_us(u64::from(limits.abort_ignore_s) * 1_000_000),
            );
            self.fail(now, p.key, limits);
            self.diag_fail(now, disc_id, p.key, "notOurs");
            self.diag.push(DiagEvent::Abort {
                disc: disc_id,
                ignore_s: u32::from(limits.abort_ignore_s),
                why: "notOurs",
            });
            fx.radio(RadioCommand::Abort {
                disc_id,
                ignore_s: limits.abort_ignore_s,
            });
        }
    }

    /// The platform opened a link (either role), before its handshake. The connect it
    /// answers is over: the single connect slot moves on while the handshake runs.
    pub fn on_link_opened(&mut self, link: LinkId, info: &LinkInfo) {
        if let (Some(p), Some(d)) = (self.pending, info.disc_id)
            && p.disc_id == d
        {
            self.pending = None;
            self.opening.insert(link, p.key);
        }
    }

    /// A link came up (either role).
    pub fn on_link_up(&mut self, link: LinkId, info: &LinkInfo) {
        let key = if let Some(k) = self.opening.remove(&link) {
            k
        } else {
            match (self.pending, info.disc_id) {
                (Some(p), Some(d)) if p.disc_id == d => {
                    self.pending = None;
                    p.key
                }
                _ => PeerKey::Disc(u32::MAX - link.0),
            }
        };
        // The peer is back: forget its backoff under every key it was tried by (the
        // 14-bit prefix of an iPhone local name, then its full token after the HELLO).
        self.backoff.retain(|k, _| !same_peer(*k, key));
        self.linked.insert(link, key);
    }

    /// A link went down.
    pub fn on_link_down(
        &mut self,
        now: Instant,
        link: LinkId,
        reason: LinkDownReason,
        limits: &Limits,
    ) {
        // A link opened for a connect that died before its handshake completed (a
        // duplicate of an existing link, a refused or failed handshake) is a failed
        // attempt: per-peer backoff.
        if let Some(key) = self.opening.remove(&link) {
            self.fail(now, key, limits);
            let disc = match key {
                PeerKey::Disc(d) => d,
                _ => u32::MAX,
            };
            self.diag_fail(now, disc, key, "handshake");
        }
        if let Some(key) = self.linked.remove(&link)
            && matches!(reason, LinkDownReason::Timeout | LinkDownReason::Error(_))
        {
            self.fail(now, key, limits);
        }
        if reason == LinkDownReason::LimitReached {
            self.limit_until.push(now.plus_us(limits.limit_backoff_us));
        }
    }

    /// A connect attempt failed before a link existed.
    pub fn on_connect_failed(
        &mut self,
        now: Instant,
        disc_id: u32,
        reason: LinkDownReason,
        limits: &Limits,
    ) {
        if let Some(p) = self.pending.filter(|p| p.disc_id == disc_id) {
            self.pending = None;
            // The HELLO was fine and the L2CAP open failed (a dead PSM, a channel the
            // peer's stack refuses): the next attempt to this peer uses GATT.
            let mut code = crate::diag::down_str(reason);
            if let (Some(OpenVia::L2cap { .. }), PeerKey::Token(t)) = (p.via, p.key) {
                self.gatt_peers.insert(t);
                code = "l2capOpen";
            }
            self.fail(now, p.key, limits);
            self.diag_fail(now, disc_id, p.key, code);
        }
        if reason == LinkDownReason::LimitReached {
            self.limit_until.push(now.plus_us(limits.limit_backoff_us));
        }
    }

    /// Connect timeout, backoff expiry and retries.
    pub fn on_timer(
        &mut self,
        now: Instant,
        ble_links: usize,
        limits: &Limits,
        fx: &mut EffectQueue,
    ) {
        if let Some(p) = self.pending
            && now.since(p.since) >= limits.connect_timeout_us
        {
            self.pending = None;
            self.fail(now, p.key, limits);
            self.diag_fail(now, p.disc_id, p.key, "timeout");
            fx.radio(RadioCommand::Abort {
                disc_id: p.disc_id,
                ignore_s: 0,
            });
        }
        self.limit_until.retain(|t| *t > now);
        self.try_connect(now, ble_links, limits, fx);
        self.refresh(ble_links, limits, now, fx);
    }

    /// Next instant (after `now`) the policy needs a timer: the connect timeout, the end
    /// of a backoff that holds back a waiting candidate, a `LimitReached` expiry.
    pub fn next_deadline(&self, limits: &Limits, now: Instant) -> Option<Instant> {
        let mut t = self
            .pending
            .map(|p| p.since.plus_us(limits.connect_timeout_us));
        let mut take = |x: Instant| t = Some(t.map_or(x, |y: Instant| y.min(x)));
        // An unanswered connect turns stale (a fresh candidate may then take its slot).
        if let Some(p) = self
            .pending
            .filter(|p| !p.hello_ok && !self.candidates.is_empty())
        {
            let stale_at = p.seen.plus_us(CANDIDATE_FRESH_US);
            if stale_at > now {
                take(stale_at);
            }
        }
        for c in self.candidates.values() {
            for (_, b) in self
                .backoff
                .iter()
                .filter(|(k, b)| b.until > now && same_peer(**k, c.key))
            {
                take(b.until);
            }
        }
        for u in self.limit_until.iter().filter(|u| **u > now) {
            take(*u);
        }
        // The duty cycle moves on; an unlinked peer's advertisement turns old.
        if let Some(h) = self.hunt {
            take(h.phase_end.max(now.plus_us(1)));
        }
        if let Some(w) = self
            .wanted_seen
            .map(|w| w.plus_us(WANTED_FRESH_US))
            .filter(|w| *w > now)
        {
            take(w);
        }
        // The spare slot closes (HELLO `accepting` is re-evaluated).
        if let Some(t) = self
            .missing_seen
            .map(|t| t.plus_us(SPARE_HOLD_US))
            .filter(|t| *t > now && self.force_forming)
        {
            take(t);
        }
        t
    }

    /// Bluetooth availability changed. Returns `true` when BLE just went unavailable.
    pub fn on_radio_state(
        &mut self,
        ble: RadioAvail,
        aware: RadioAvail,
        ble_links: usize,
        limits: &Limits,
        now: Instant,
        fx: &mut EffectQueue,
    ) -> bool {
        let was_on = self.ble == RadioAvail::On;
        if (self.ble, self.aware) != (ble, aware) {
            self.diag.push(DiagEvent::RadioState {
                ble: ble.lab().as_str(),
                aware: aware.lab().as_str(),
            });
        }
        self.ble = ble;
        self.aware = aware;
        if ble != RadioAvail::On {
            self.pending = None;
            self.candidates.clear();
        }
        self.sent_hello = None;
        self.refresh(ble_links, limits, now, fx);
        was_on && ble != RadioAvail::On
    }

    /// A node at its BLE max still accepts one more link, or connects to that member if
    /// it is the initiator, while a lost roster member advertises in range
    /// (fresh): that member may have nobody else in reach, and a node whose links all
    /// go to one tight cluster would otherwise shut it out for as long as the cluster
    /// holds. Over the max, the 2 s selection closes a redundant link, never the only
    /// path to a reachable member (§5.7).
    ///
    /// An Android phone never sees a backgrounded iPhone advertise (overflow area,
    /// §9.2), and that iPhone can only rejoin through an Android peripheral: an Android
    /// phone keeps the spare slot open for as long as a lost member is missing.
    ///
    /// Never while voice flows through this node: going over the max mid-burst makes
    /// the 2 s selection close a redundant link, which may be the inbound path of an
    /// admitted talker; its stream is then re-routed and the floor views split (mesh-sim
    /// S5). The lost member gets the slot when the burst ends, or a node under its max.
    fn spare_slot(&self, now: Instant) -> bool {
        !self.voice_active
            && self.force_forming
            && (self
                .missing_seen
                .is_some_and(|t| now.since(t) < SPARE_HOLD_US)
                || (self.platform == Platform::Android && !self.lost_tokens.is_empty()))
    }

    /// `true` while a connect is outstanding.
    pub fn connecting(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
#[path = "radio_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rejoin_tests.rs"]
mod rejoin_tests;

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod discovery_tests;
