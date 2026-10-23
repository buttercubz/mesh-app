//! Lab enumerations shared by the catalog, the runner and the `labrun/1` records. The
//! string ids are the ones of the UI contract (mesh-api `LabBench`, `LabAudio`, …) and of
//! the `labrun/1` schema; mesh-core maps between the two crates by these ids.

macro_rules! lab_enum {
    (
        $(#[$m:meta])*
        pub enum $name:ident { $( $(#[$vm:meta])* $v:ident = $s:literal ),+ $(,)? }
    ) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $( $(#[$vm])* $v ),+ }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$v),+];

            /// The string id used in `labrun/1` and the UI contract.
            pub const fn as_str(self) -> &'static str {
                match self { $($name::$v => $s),+ }
            }

            /// Parses a string id.
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($s => Some($name::$v),)+ _ => None }
            }
        }

        impl core::fmt::Display for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

pub(crate) use lab_enum;

lab_enum! {
    /// Field bench (DESIGN §12.2) plus the passive ride recorder (§12.1).
    pub enum Bench {
        T0 = "T0",
        T0b = "T0b",
        T0c = "T0c",
        T1 = "T1",
        T2 = "T2",
        T3 = "T3",
        T4 = "T4",
        T5a = "T5a",
        T5b = "T5b",
        T6 = "T6",
        T8 = "T8",
        VoxCal = "voxCal",
        Ride = "ride",
    }
}

impl Bench {
    /// The `bench` byte of `LAB_CTL` (§4.4). Stable: never renumber.
    pub const fn wire_id(self) -> u8 {
        match self {
            Bench::T0 => 0,
            Bench::T0b => 1,
            Bench::T0c => 2,
            Bench::T1 => 3,
            Bench::T2 => 4,
            Bench::T3 => 5,
            Bench::T4 => 6,
            Bench::T5a => 7,
            Bench::T5b => 8,
            Bench::T6 => 9,
            Bench::T8 => 10,
            Bench::VoxCal => 11,
            Bench::Ride => 12,
        }
    }

    /// Inverse of [`Bench::wire_id`].
    pub fn from_wire_id(id: u8) -> Option<Self> {
        Bench::ALL.iter().copied().find(|b| b.wire_id() == id)
    }
}

lab_enum! {
    /// Audio path of a run (`LabParams.audio`, mandatory, §12.1).
    pub enum Audio {
        /// Radio-only bench, no audio session.
        Off = "off",
        /// Session + VPIO output: keeps the `audio` anchor, plays bench tones, no mic.
        RenderOnly = "renderOnly",
        /// Session + VPIO capture and render; HFP opened when a headset is connected.
        Full = "full",
    }
}

lab_enum! {
    /// Keep-alive anchor of a run (`LabParams.anchor`, mandatory, §12.1).
    pub enum Anchor {
        /// Control variant that measures suspension itself; never a default.
        None = "none",
        /// VPIO running (render-only or full): the product's default anchor.
        Audio = "audio",
        /// `audio` plus `CLBackgroundActivitySession` and live location updates.
        AudioLocation = "audioLocation",
    }
}

lab_enum! {
    /// LPROBE traffic pattern (§12.1, §12.2).
    pub enum Pattern {
        /// Fill the link whenever it is writable.
        Saturate = "saturate",
        /// 1-4 CBR streams padded to exact VOICE frame sizes.
        VoiceLike = "voiceLike",
        /// Echoed probes at `ping_hz`.
        Ping = "ping",
        /// Pings while voice-like streams load the link.
        PingUnderLoad = "pingUnderLoad",
        /// No generated traffic (passive benches: T2 counters, T5a/T5b audio, ride).
        Idle = "idle",
    }
}

lab_enum! {
    /// BLE transport variant.
    pub enum Transport {
        L2cap = "l2cap",
        GattNotify = "gattNotify",
        GattWrite = "gattWrite",
        /// Wi-Fi Aware (T4).
        Aware = "aware",
    }
}

lab_enum! {
    /// BLE role of the local phone.
    pub enum LinkRole {
        Central = "central",
        Peripheral = "peripheral",
    }
}

lab_enum! {
    /// Role of the device in the run.
    pub enum RunRole {
        /// Started the run and sent `LAB_CTL{start}`.
        Controller = "controller",
        /// Joined the run from a received `LAB_CTL{start}`.
        Follower = "follower",
    }
}

lab_enum! {
    /// Link kind as recorded in the `s` line `links[].kind`.
    pub enum LinkKind {
        BleL2cap = "ble-l2cap",
        BleGatt = "ble-gatt",
        Aware = "aware",
        Tunnel = "tunnel",
        /// Simulator link (mesh-sim).
        Sim = "sim",
    }
}

impl LinkKind {
    /// Bytes added around a LinkFrame on air: the L2CAP/tunnel `len u16` prefix,
    /// the GATT chunk header (one chunk assumed), nothing on datagram links.
    pub const fn framing_overhead(self) -> usize {
        match self {
            LinkKind::BleL2cap | LinkKind::Tunnel | LinkKind::Sim => 2,
            LinkKind::BleGatt => 1,
            LinkKind::Aware => 0,
        }
    }
}

lab_enum! {
    /// Go / no-go verdict (mirrors mesh-api `LabVerdict`).
    pub enum Verdict {
        Go = "go",
        NoGo = "noGo",
        /// Not enough valid data, or the bench has no automatic criterion.
        Inconclusive = "inconclusive",
    }
}

lab_enum! {
    /// Bluetooth radio state in the 1 Hz sample (§12.1).
    pub enum BtState {
        On = "on",
        Off = "off",
        Unauthorized = "unauthorized",
        Resetting = "resetting",
        Unsupported = "unsupported",
        Unknown = "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for b in Bench::ALL {
            assert_eq!(Bench::parse(b.as_str()), Some(*b));
            assert_eq!(Bench::from_wire_id(b.wire_id()), Some(*b));
        }
        assert_eq!(Bench::from_wire_id(200), None);
        assert_eq!(Audio::parse("renderOnly"), Some(Audio::RenderOnly));
        assert_eq!(Anchor::parse("audioLocation"), Some(Anchor::AudioLocation));
        assert_eq!(Pattern::parse("bogus"), None);
        assert_eq!(Bench::VoxCal.to_string(), "voxCal");
        assert_eq!(LinkKind::BleL2cap.framing_overhead(), 2);
    }
}
