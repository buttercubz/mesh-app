//! VOICE, TALK and FLOOR_REJECT payloads (DESIGN §4.6).

use crate::codec::{PutLe, Reader};
use crate::error::{Result, WireError};

macro_rules! u8_enum {
    ($(#[$m:meta])* $name:ident, $what:literal { $($(#[$vm:meta])* $var:ident = $val:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        pub enum $name { $($(#[$vm])* $var = $val),+ }

        impl $name {
            /// Parses the byte value.
            pub fn from_u8(v: u8) -> Result<Self> {
                match v {
                    $($val => Ok(Self::$var),)+
                    _ => Err(WireError::BadValue { what: $what, value: u32::from(v) }),
                }
            }
        }
    };
}
pub(crate) use u8_enum;

u8_enum!(
    /// Encoder profile (TALK `profile`, PATH_HINT `profile`, §7.4).
    EncoderProfile, "profile" {
        /// 16 kbps WB, 40 ms, no LBRR.
        Ble16 = 1,
        /// 12 kbps, 40 ms.
        Ble12 = 2,
        /// 20 kbps, 20 ms.
        Wifi20 = 3,
        /// 24 kbps, 20 ms + LBRR.
        Wifi24Fec = 4,
        /// 20 kbps, 40 ms with DRED.
        BleDred = 5,
    }
);

u8_enum!(
    /// TALK `state`.
    TalkState, "TALK state" { Start = 1, Stop = 2, Keep = 3 }
);
u8_enum!(
    /// TALK `prio`.
    TalkPrio, "TALK prio" { Member = 1, Leader = 2 }
);
u8_enum!(
    /// TALK `mode`.
    TalkMode, "TALK mode" { Vox = 1, Ptt = 2 }
);
u8_enum!(
    /// FLOOR_REJECT `reason`.
    FloorRejectReason, "FLOOR_REJECT reason" { Cap = 1, Preempted = 2, Congestion = 3, Outranked = 4 }
);

/// VOICE frame duration (cfg b7-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDur {
    Ms10 = 0,
    Ms20 = 1,
    Ms40 = 2,
    Ms60 = 3,
}

impl FrameDur {
    /// Milliseconds.
    pub const fn ms(self) -> u16 {
        match self {
            Self::Ms10 => 10,
            Self::Ms20 => 20,
            Self::Ms40 => 40,
            Self::Ms60 => 60,
        }
    }
    /// 10 ms ticks per frame (VOICE `ts` step).
    pub const fn ticks(self) -> u16 {
        self.ms() / 10
    }
}

/// VOICE audio bandwidth (cfg b5-4; 3 reserved).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bandwidth {
    Narrow = 0,
    Medium = 1,
    Wide = 2,
    Reserved = 3,
}

/// VOICE `cfg`: b7-6 dur, b5-4 bw, b3 LBRR, b2 DRED, b1 FIRST, b0 PREROLL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VoiceCfg(pub u8);

impl VoiceCfg {
    /// Builds a cfg byte.
    pub const fn new(
        dur: FrameDur,
        bw: Bandwidth,
        lbrr: bool,
        dred: bool,
        first: bool,
        preroll: bool,
    ) -> Self {
        VoiceCfg(
            ((dur as u8) << 6)
                | ((bw as u8) << 4)
                | ((lbrr as u8) << 3)
                | ((dred as u8) << 2)
                | ((first as u8) << 1)
                | (preroll as u8),
        )
    }
    pub const fn dur(self) -> FrameDur {
        match self.0 >> 6 {
            0 => FrameDur::Ms10,
            1 => FrameDur::Ms20,
            2 => FrameDur::Ms40,
            _ => FrameDur::Ms60,
        }
    }
    pub const fn bw(self) -> Bandwidth {
        match (self.0 >> 4) & 0x03 {
            0 => Bandwidth::Narrow,
            1 => Bandwidth::Medium,
            2 => Bandwidth::Wide,
            _ => Bandwidth::Reserved,
        }
    }
    pub const fn lbrr(self) -> bool {
        self.0 & 0x08 != 0
    }
    pub const fn dred(self) -> bool {
        self.0 & 0x04 != 0
    }
    pub const fn first(self) -> bool {
        self.0 & 0x02 != 0
    }
    pub const fn preroll(self) -> bool {
        self.0 & 0x01 != 0
    }
}

/// 0x01 VOICE: `burst u8 · ts u16 · cfg u8 · opus[1..=400]`. `ts` counts 10 ms ticks
/// since the burst's first VOICE frame (frame n of a 40 ms profile has `ts = 4n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Voice<'a> {
    pub burst: u8,
    pub ts: u16,
    pub cfg: VoiceCfg,
    pub opus: &'a [u8],
}

impl<'a> Voice<'a> {
    /// Voice header bytes before the Opus packet.
    pub const HEADER: usize = 4;
    /// Largest Opus packet.
    pub const MAX_OPUS: usize = 400;

    pub fn decode(p: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let v = Voice {
            burst: r.u8("VOICE burst")?,
            ts: r.u16("VOICE ts")?,
            cfg: VoiceCfg(r.u8("VOICE cfg")?),
            opus: r.rest(),
        };
        check_opus(v.opus.len())?;
        Ok(v)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        check_opus(self.opus.len())?;
        let mut o = Vec::with_capacity(Self::HEADER + self.opus.len());
        o.put_u8(self.burst);
        o.put_u16(self.ts);
        o.put_u8(self.cfg.0);
        o.put(self.opus);
        Ok(o)
    }

    /// `ts` of frame `n` of a burst with frame duration `dur` (wrapping u16).
    pub fn ts_for_frame(n: u32, dur: FrameDur) -> u16 {
        (n.wrapping_mul(u32::from(dur.ticks())) & 0xFFFF) as u16
    }
}

fn check_opus(len: usize) -> Result<()> {
    if len == 0 {
        return Err(WireError::TooShort {
            what: "VOICE opus",
            len,
            min: 1,
        });
    }
    crate::codec::check_max("VOICE opus", len, Voice::MAX_OPUS)
}

/// 0x02 TALK (7 B): `burst, state, prio, mode, profile, elapsed_10ms u16` (0 on START).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Talk {
    pub burst: u8,
    pub state: TalkState,
    pub prio: TalkPrio,
    pub mode: TalkMode,
    pub profile: EncoderProfile,
    pub elapsed_10ms: u16,
}

impl Talk {
    pub const LEN: usize = 7;

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let t = Talk {
            burst: r.u8("TALK burst")?,
            state: TalkState::from_u8(r.u8("TALK state")?)?,
            prio: TalkPrio::from_u8(r.u8("TALK prio")?)?,
            mode: TalkMode::from_u8(r.u8("TALK mode")?)?,
            profile: EncoderProfile::from_u8(r.u8("TALK profile")?)?,
            elapsed_10ms: r.u16("TALK elapsed")?,
        };
        r.finish("TALK")?;
        Ok(t)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u8(self.burst);
        o.put_u8(self.state as u8);
        o.put_u8(self.prio as u8);
        o.put_u8(self.mode as u8);
        o.put_u8(self.profile as u8);
        o.put_u16(self.elapsed_10ms);
        o
    }
}

/// 0x03 FLOOR_REJECT (7 B): `target u16, target_sess u8, burst u8, reason u8, at u16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FloorReject {
    pub target: u16,
    pub target_sess: u8,
    pub burst: u8,
    pub reason: FloorRejectReason,
    pub at: u16,
}

impl FloorReject {
    pub const LEN: usize = 7;

    pub fn decode(p: &[u8]) -> Result<Self> {
        let mut r = Reader::new(p);
        let f = FloorReject {
            target: r.u16("REJECT target")?,
            target_sess: r.u8("REJECT target_sess")?,
            burst: r.u8("REJECT burst")?,
            reason: FloorRejectReason::from_u8(r.u8("REJECT reason")?)?,
            at: r.u16("REJECT at")?,
        };
        r.finish("FLOOR_REJECT")?;
        Ok(f)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(Self::LEN);
        o.put_u16(self.target);
        o.put_u8(self.target_sess);
        o.put_u8(self.burst);
        o.put_u8(self.reason as u8);
        o.put_u16(self.at);
        o
    }
}
