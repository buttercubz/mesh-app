//! Encoder profiles and settings (DESIGN §7.4).
//!
//! Common settings: VOIP application, voice signal, 16 kHz API, constrained VBR, DTX off
//! (the external VAD gates bursts), encoder reset at every burst start, never 10 ms or
//! 60 ms frames. LBRR (in-band FEC) only at 24 kbps with smoothed loss ≥ 3 %; with LBRR
//! off the loss hint is always 0, because a non-zero hint lowers libopus's equivalent
//! rate and can drop the audio bandwidth below WB. DRED and LBRR are never enabled
//! together.

use crate::{BLOCK_SAMPLES, BLOCK_US};

/// Encoder profile; the discriminant is the TALK `profile` id (DESIGN §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Profile {
    /// Default whenever any of my links is BLE: 16 kbps, 40 ms, no LBRR.
    Ble16 = 1,
    /// PATH_HINT heard, own `cap_pacing = 1`, or thermal ≥ serious: 12 kbps, 40 ms.
    Ble12 = 2,
    /// All links Aware/Tunnel and worst-neighbor smoothed loss < 3 %: 20 kbps, 20 ms.
    Wifi20 = 3,
    /// All links Aware/Tunnel and smoothed loss ≥ 3 %: 24 kbps, 20 ms, LBRR.
    Wifi24Fec = 4,
    /// Phase 5 experiment: 20 kbps, 40 ms, DRED (never with LBRR).
    BleDred = 5,
}

impl Profile {
    /// All profiles in TALK id order.
    pub const ALL: [Profile; 5] = [
        Profile::Ble16,
        Profile::Ble12,
        Profile::Wifi20,
        Profile::Wifi24Fec,
        Profile::BleDred,
    ];

    /// TALK `profile` id.
    pub const fn talk_id(self) -> u8 {
        self as u8
    }

    /// Profile from a TALK `profile` id.
    pub const fn from_talk_id(id: u8) -> Option<Self> {
        Some(match id {
            1 => Profile::Ble16,
            2 => Profile::Ble12,
            3 => Profile::Wifi20,
            4 => Profile::Wifi24Fec,
            5 => Profile::BleDred,
            _ => return None,
        })
    }

    /// True for the BLE profiles (40 ms frames).
    pub const fn is_ble(self) -> bool {
        matches!(self, Profile::Ble16 | Profile::Ble12 | Profile::BleDred)
    }
}

/// VOICE frame duration (cfg b7-6). v1 never uses 10 ms or 60 ms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FrameDuration {
    Ms10,
    Ms20,
    Ms40,
    Ms60,
}

impl FrameDuration {
    /// Duration in milliseconds.
    pub const fn ms(self) -> u32 {
        match self {
            FrameDuration::Ms10 => 10,
            FrameDuration::Ms20 => 20,
            FrameDuration::Ms40 => 40,
            FrameDuration::Ms60 => 60,
        }
    }

    /// Duration in 10 ms ticks (the VOICE `ts` unit): a 40 ms frame is 4 ticks.
    pub const fn ticks(self) -> u16 {
        (self.ms() / 10) as u16
    }

    /// Duration in microseconds.
    pub const fn us(self) -> u64 {
        self.ticks() as u64 * BLOCK_US
    }

    /// Samples per frame at 16 kHz.
    pub const fn samples(self) -> usize {
        self.ticks() as usize * BLOCK_SAMPLES
    }

    /// 10 ms blocks per frame.
    pub const fn blocks(self) -> usize {
        self.ticks() as usize
    }

    /// cfg b7-6 value.
    pub const fn cfg_bits(self) -> u8 {
        match self {
            FrameDuration::Ms10 => 0,
            FrameDuration::Ms20 => 1,
            FrameDuration::Ms40 => 2,
            FrameDuration::Ms60 => 3,
        }
    }

    /// Inverse of [`FrameDuration::cfg_bits`] (only the low two bits are used).
    pub const fn from_cfg_bits(bits: u8) -> Self {
        match bits & 3 {
            0 => FrameDuration::Ms10,
            1 => FrameDuration::Ms20,
            2 => FrameDuration::Ms40,
            _ => FrameDuration::Ms60,
        }
    }
}

/// Audio bandwidth (cfg b5-4; 3 = SWB is reserved for a v2 RT ABI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AudioBandwidth {
    /// 4 kHz audio bandwidth (8 kHz capture routes such as CVSD).
    Nb,
    /// 6 kHz audio bandwidth (12 kHz capture).
    Mb,
    /// 8 kHz audio bandwidth: the v1 ceiling.
    Wb,
}

impl AudioBandwidth {
    /// Bandwidth that the capture route supports (`AudioRouteInfo.input_rate`), capped at WB.
    pub const fn from_route_rate(route_rate_hz: u32) -> Self {
        if route_rate_hz <= 8_000 {
            AudioBandwidth::Nb
        } else if route_rate_hz <= 12_000 {
            AudioBandwidth::Mb
        } else {
            AudioBandwidth::Wb
        }
    }

    /// cfg b5-4 value.
    pub const fn cfg_bits(self) -> u8 {
        match self {
            AudioBandwidth::Nb => 0,
            AudioBandwidth::Mb => 1,
            AudioBandwidth::Wb => 2,
        }
    }

    /// Inverse of [`AudioBandwidth::cfg_bits`]; the reserved value 3 maps to `None`.
    pub const fn from_cfg_bits(bits: u8) -> Option<Self> {
        match bits & 3 {
            0 => Some(AudioBandwidth::Nb),
            1 => Some(AudioBandwidth::Mb),
            2 => Some(AudioBandwidth::Wb),
            _ => None,
        }
    }
}

/// Complete encoder settings for one burst (applied after the per-burst reset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncoderConfig {
    pub profile: Profile,
    pub frame: FrameDuration,
    pub bitrate_bps: u32,
    /// `OPUS_SET_MAX_BANDWIDTH`: follows the sender's capture route, WB ceiling.
    pub max_bandwidth: AudioBandwidth,
    /// SILK LBRR (`OPUS_SET_INBAND_FEC`).
    pub inband_fec: bool,
    /// `OPUS_SET_PACKET_LOSS_PERC`.
    pub packet_loss_perc: u8,
    pub complexity: u8,
    /// Always false in v1 (the VAD gates bursts).
    pub dtx: bool,
    /// Always true in v1 (VBR + constrained VBR).
    pub constrained_vbr: bool,
    /// `OPUS_SET_DRED_DURATION` in 10 ms units (0 = off).
    pub dred_duration_10ms: u16,
}

/// DRED look-back for the BLE-DRED experiment: 200 ms, the jitter-buffer ceiling, since DRED
/// only fills gaps shorter than the buffer slack [TUNE, Phase 5 Lab].
pub const BLE_DRED_DURATION_10MS: u16 = 20;

/// Loss threshold (percent) that turns LBRR on for Wi-Fi paths.
pub const FEC_ON_LOSS_PCT: f32 = 3.0;
/// Loss below which LBRR may turn off again (after [`FEC_OFF_HOLD_US`]).
pub const FEC_OFF_LOSS_PCT: f32 = 2.0;
/// Time the loss must stay below [`FEC_OFF_LOSS_PCT`] before LBRR turns off.
pub const FEC_OFF_HOLD_US: u64 = 10_000_000;

impl EncoderConfig {
    /// The §7.4 settings for `profile` given the capture route rate (Hz) and the smoothed
    /// loss (percent, worst neighbor).
    pub fn for_profile(profile: Profile, route_rate_hz: u32, smoothed_loss_pct: f32) -> Self {
        let loss = if smoothed_loss_pct.is_finite() {
            smoothed_loss_pct.max(0.0)
        } else {
            0.0
        };
        let max_bandwidth = AudioBandwidth::from_route_rate(route_rate_hz);
        let base = EncoderConfig {
            profile,
            frame: FrameDuration::Ms40,
            bitrate_bps: 16_000,
            max_bandwidth,
            inband_fec: false,
            packet_loss_perc: 0,
            complexity: 8,
            dtx: false,
            constrained_vbr: true,
            dred_duration_10ms: 0,
        };
        match profile {
            Profile::Ble16 => base,
            Profile::Ble12 => EncoderConfig {
                bitrate_bps: 12_000,
                ..base
            },
            Profile::Wifi20 => EncoderConfig {
                frame: FrameDuration::Ms20,
                bitrate_bps: 20_000,
                complexity: 9,
                ..base
            },
            Profile::Wifi24Fec => EncoderConfig {
                frame: FrameDuration::Ms20,
                bitrate_bps: 24_000,
                complexity: 9,
                inband_fec: true,
                packet_loss_perc: loss.round().clamp(5.0, 25.0) as u8,
                ..base
            },
            // The §7.4 table gives BLE-DRED a measured loss hint: libopus sizes the DRED
            // payload from it. LBRR stays off and DRED is the only redundancy.
            Profile::BleDred => EncoderConfig {
                bitrate_bps: 20_000,
                packet_loss_perc: loss.round().clamp(0.0, 50.0) as u8,
                dred_duration_10ms: BLE_DRED_DURATION_10MS,
                ..base
            },
        }
    }

    /// Thermal state ≥ serious: BLE12 drops to complexity 5 (§7.4); other profiles unchanged.
    pub fn with_thermal(mut self, thermal_serious: bool) -> Self {
        if thermal_serious && self.profile == Profile::Ble12 {
            self.complexity = 5;
        }
        self
    }

    /// Bytes per frame at the nominal bitrate (VBR packets vary around it).
    pub const fn nominal_frame_bytes(&self) -> usize {
        (self.bitrate_bps as usize * self.frame.ms() as usize) / 8000
    }
}

/// Inputs to [`ProfileSelector::select`], evaluated by the media layer at burst start.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProfileInputs {
    /// At least one of my links is BLE (or I have no Aware/Tunnel link at all).
    pub any_ble_link: bool,
    /// A PATH_HINT asking for BLE12 was heard (within its validity).
    pub path_hint_ble12: bool,
    /// My own `cap_pacing` is 1 on some link.
    pub cap_pacing_one: bool,
    /// Thermal state ≥ serious.
    pub thermal_serious: bool,
    /// Worst-neighbor smoothed loss in percent (Wi-Fi decision).
    pub worst_loss_pct: f32,
    /// Every BLE-DRED precondition holds (group `dred_ok`, Lab bursty loss, grants ≥ 64.7 kbps).
    pub dred_allowed: bool,
}

/// Chooses the profile, with the WIFI24FEC hysteresis (on at ≥ 3 %, off after 10 s < 2 %).
#[derive(Debug, Clone, Default)]
pub struct ProfileSelector {
    fec_on: bool,
    below_since_us: Option<u64>,
}

impl ProfileSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the loss hysteresis and returns the profile to use for a burst starting now.
    pub fn select(&mut self, now_us: u64, inputs: &ProfileInputs) -> Profile {
        let loss = inputs.worst_loss_pct;
        if loss >= FEC_ON_LOSS_PCT {
            self.fec_on = true;
            self.below_since_us = None;
        } else if loss < FEC_OFF_LOSS_PCT {
            let since = *self.below_since_us.get_or_insert(now_us);
            if now_us.saturating_sub(since) >= FEC_OFF_HOLD_US {
                self.fec_on = false;
            }
        } else {
            self.below_since_us = None;
        }
        if inputs.any_ble_link {
            if inputs.path_hint_ble12 || inputs.cap_pacing_one || inputs.thermal_serious {
                Profile::Ble12
            } else if inputs.dred_allowed {
                Profile::BleDred
            } else {
                Profile::Ble16
            }
        } else if self.fec_on {
            Profile::Wifi24Fec
        } else {
            Profile::Wifi20
        }
    }

    /// Current LBRR hysteresis state.
    pub fn fec_on(&self) -> bool {
        self.fec_on
    }
}
