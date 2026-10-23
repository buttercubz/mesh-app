//! PRESENCE payload (DESIGN §4.6.1), 14 bytes, every 2 s.

use crate::codec::{PutLe, Reader};
use crate::error::Result;
use crate::serial::truncate16;

/// `state`: b7-6 ride (0 lobby, 1 riding, 2 ending) | b5-4 talk (0 idle, 1 talking,
/// 2 blocked, 3 muted) | b3 leader | b2 on_call | b1 capture_ok | b0 live.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenceState(pub u8);

impl PresenceState {
    pub const RIDE_LOBBY: u8 = 0;
    pub const RIDE_RIDING: u8 = 1;
    pub const RIDE_ENDING: u8 = 2;
    pub const TALK_IDLE: u8 = 0;
    pub const TALK_TALKING: u8 = 1;
    pub const TALK_BLOCKED: u8 = 2;
    pub const TALK_MUTED: u8 = 3;

    pub const fn new(
        ride: u8,
        talk: u8,
        leader: bool,
        on_call: bool,
        capture_ok: bool,
        live: bool,
    ) -> Self {
        PresenceState(
            ((ride & 3) << 6)
                | ((talk & 3) << 4)
                | ((leader as u8) << 3)
                | ((on_call as u8) << 2)
                | ((capture_ok as u8) << 1)
                | (live as u8),
        )
    }
    pub const fn ride(self) -> u8 {
        self.0 >> 6
    }
    pub const fn talk(self) -> u8 {
        (self.0 >> 4) & 3
    }
    pub const fn leader(self) -> bool {
        self.0 & 0x08 != 0
    }
    pub const fn on_call(self) -> bool {
        self.0 & 0x04 != 0
    }
    pub const fn capture_ok(self) -> bool {
        self.0 & 0x02 != 0
    }
    pub const fn live(self) -> bool {
        self.0 & 0x01 != 0
    }
}

/// `audio`: b7-6 ladder (L0-L3) | b5-3 route (0 none, 1 builtin, 2 hfp-nb, 3 hfp-wb,
/// 4 le-audio, 5 car-hfp, 6 wired) | b2 tx_muted_no_headset | b1 render_ok |
/// b0 sharing_distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenceAudio(pub u8);

impl PresenceAudio {
    pub const ROUTE_NONE: u8 = 0;
    pub const ROUTE_BUILTIN: u8 = 1;
    pub const ROUTE_HFP_NB: u8 = 2;
    pub const ROUTE_HFP_WB: u8 = 3;
    pub const ROUTE_LE_AUDIO: u8 = 4;
    pub const ROUTE_CAR_HFP: u8 = 5;
    pub const ROUTE_WIRED: u8 = 6;

    pub const fn new(
        ladder: u8,
        route: u8,
        tx_muted_no_headset: bool,
        render_ok: bool,
        sharing_distance: bool,
    ) -> Self {
        PresenceAudio(
            ((ladder & 3) << 6)
                | ((route & 7) << 3)
                | ((tx_muted_no_headset as u8) << 2)
                | ((render_ok as u8) << 1)
                | (sharing_distance as u8),
        )
    }
    pub const fn ladder(self) -> u8 {
        self.0 >> 6
    }
    pub const fn route(self) -> u8 {
        (self.0 >> 3) & 7
    }
    pub const fn tx_muted_no_headset(self) -> bool {
        self.0 & 0x04 != 0
    }
    pub const fn render_ok(self) -> bool {
        self.0 & 0x02 != 0
    }
    pub const fn sharing_distance(self) -> bool {
        self.0 & 0x01 != 0
    }
}

/// `links`: b7-4 BLE link count, b3-0 Aware link count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenceLinks(pub u8);

impl PresenceLinks {
    pub const fn new(ble: u8, aware: u8) -> Self {
        let ble = if ble > 15 { 15 } else { ble };
        let aware = if aware > 15 { 15 } else { aware };
        PresenceLinks((ble << 4) | aware)
    }
    pub const fn ble(self) -> u8 {
        self.0 >> 4
    }
    pub const fn aware(self) -> u8 {
        self.0 & 0x0F
    }
}

/// `caps`: b7-6 platform | b5 dred_ok | b4 radio_off_recently (BT off within 60 s) |
/// b3-0 reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresenceCaps(pub u8);

impl PresenceCaps {
    pub const fn new(platform: u8, dred_ok: bool, radio_off_recently: bool) -> Self {
        PresenceCaps(
            ((platform & 3) << 6) | ((dred_ok as u8) << 5) | ((radio_off_recently as u8) << 4),
        )
    }
    pub const fn platform(self) -> u8 {
        self.0 >> 6
    }
    pub const fn dred_ok(self) -> bool {
        self.0 & 0x20 != 0
    }
    pub const fn radio_off_recently(self) -> bool {
        self.0 & 0x10 != 0
    }
}

/// 0x04 PRESENCE (14 B): `roster_ver16 u16 · epoch16 u16 · state · audio · battery ·
/// links · caps · interval (100 ms units) · heard u32` (bitmap by roster index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Presence {
    pub roster_ver16: u16,
    pub epoch16: u16,
    pub state: PresenceState,
    pub audio: PresenceAudio,
    /// 0-100 %, 255 unknown.
    pub battery: u8,
    pub links: PresenceLinks,
    pub caps: PresenceCaps,
    pub interval: u8,
    pub heard: u32,
}

impl Presence {
    pub const LEN: usize = 14;
    /// Battery unknown.
    pub const BATTERY_UNKNOWN: u8 = 255;

    /// Fills the truncated version fields from authoritative u32 values.
    pub fn with_versions(mut self, roster_version: u32, epoch: u32) -> Self {
        self.roster_ver16 = truncate16(roster_version);
        self.epoch16 = truncate16(epoch);
        self
    }

    /// `true` if roster member `index` (0-31) was heard.
    pub fn heard_index(&self, index: u8) -> bool {
        index < 32 && self.heard & (1u32 << index) != 0
    }

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let v = Presence {
            roster_ver16: r.u16("roster_ver16")?,
            epoch16: r.u16("epoch16")?,
            state: PresenceState(r.u8("state")?),
            audio: PresenceAudio(r.u8("audio")?),
            battery: r.u8("battery")?,
            links: PresenceLinks(r.u8("links")?),
            caps: PresenceCaps(r.u8("caps")?),
            interval: r.u8("interval")?,
            heard: r.u32("heard")?,
        };
        r.finish("PRESENCE")?;
        Ok(v)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u16(self.roster_ver16);
        o.put_u16(self.epoch16);
        o.put_u8(self.state.0);
        o.put_u8(self.audio.0);
        o.put_u8(self.battery);
        o.put_u8(self.links.0);
        o.put_u8(self.caps.0);
        o.put_u8(self.interval);
        o.put_u32(self.heard);
        o
    }
}
