//! LinkFrame parser (DESIGN §4.3): every accepted frame re-encodes to the same bytes;
//! SEC frames expose a consistent kp_ctr / AAD / nonce.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::frame::{LinkFrame, TAG_LEN};

fuzz_target!(|data: &[u8]| {
    if let Ok(f) = LinkFrame::decode(data) {
        assert_eq!(f.encode().expect("decoded frame encodes"), data);
        if let LinkFrame::Sec { kp_ctr, sealed } = f {
            assert!(sealed.len() >= TAG_LEN);
            assert_eq!(kp_ctr.aad()[1..], data[1..5]);
            assert_eq!(kp_ctr.nonce()[4..8], data[1..5]);
        }
    }
});
