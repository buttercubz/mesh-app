//! Links (DESIGN §3.3, owned by WP-10A): the link registry and per-link state machine
//! over the real mesh-wire framing, the `LinkSecurity` trait with its `Plain` (Lab)
//! implementation, PING/PONG, LSTAT, CAPS, the per-link priority queues and transmit
//! scheduler with pacing, and the radio policy (HELLO, advertising, connect decisions).

pub mod framing;
pub mod link;
pub mod queue;
pub mod radio;
pub mod security;
pub mod table;
mod transmit;
pub mod types;

pub use link::{Link, LinkCounters, LinkState, Pace};
pub use queue::{OutItem, Prio};
pub use radio::{CandidateClass, PeerKey, PreferVia, RadioPolicy, RadioTarget};
pub use security::{LinkSecurity, PLAIN_OVERHEAD, Plain, SecurityError, SecurityStep};
pub use table::{CLOSE_DRAIN_US, CLOSE_FLUSH_US, LinkEvent, LinkTable};
pub use types::{
    AdvData, Discovery, LinkDownReason, LinkInfo, LinkKind, LinkRole, LinkUnit, RadioAvail,
    Transport,
};

use mesh_wire::record::Caps;

use crate::config::{Limits, Platform};

/// The CAPS record of this node (§4.4). Lab links carry a zero anti-entropy summary;
/// group links get `(epoch, roster_version, ctl_version)` from the group module.
pub fn own_caps(
    platform: Platform,
    app_version: u16,
    limits: &Limits,
    leader: bool,
    summary: (u32, u32, u32),
) -> Caps {
    let mut codec = 0u16;
    if cfg!(feature = "opus") {
        codec |= Caps::CODEC_OPUS_161 | Caps::CODEC_OSCE;
    }
    Caps {
        proto: mesh_wire::PROTOCOL_VERSION,
        platform: platform.caps_byte(),
        app_ver: app_version,
        codec,
        transports: limits.transports,
        max_links: limits.ble_max(),
        role: u8::from(leader),
        epoch: summary.0,
        roster_version: summary.1,
        ctl_version: summary.2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_wire::record::Record;

    #[test]
    fn caps_is_21_bytes_with_zero_lab_summary() {
        let l = Limits::for_platform(Platform::Ios);
        let c = own_caps(Platform::Ios, 7, &l, false, (0, 0, 0));
        let body = Record::Caps(c).encode_body().unwrap();
        assert_eq!(body.len(), Caps::LEN);
        assert_eq!(&body[body.len() - 12..], &[0u8; 12]);
        assert_eq!(c.max_links, 3);
        assert_eq!(c.app_ver, 7);
    }
}
