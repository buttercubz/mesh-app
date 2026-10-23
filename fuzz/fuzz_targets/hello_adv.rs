//! HELLO value and advertising payloads (DESIGN §9.3, §9.4): the HELLO parser, the
//! scan-response AD walker and the iPhone local-name parser.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::adv::{LocalNameInfo, ServiceData};
use mesh_wire::hello::Hello;

fuzz_target!(|data: &[u8]| {
    if let Ok(h) = Hello::decode(data) {
        assert_eq!(h.encode().expect("hello encodes").as_slice(), data);
        let _ = h.common_version(1, 1);
        let _ = h.epoch_ahead_by(u32::from(h.epoch_lo).wrapping_mul(3));
    }
    if let Ok(Some(sd)) = ServiceData::find_in(data) {
        assert_eq!(ServiceData::decode(&sd.encode()), Ok(sd));
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = LocalNameInfo::parse(s);
    }
});
