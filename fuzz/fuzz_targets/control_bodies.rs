//! ROSTER / JOIN_REQ / REKEY logical bodies (DESIGN §4.7): bounded parsers; accepted
//! bodies re-encode to the same bytes (names are valid UTF-8 within their limits).
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::mesh::{JoinReq, Rekey, Roster};

fuzz_target!(|data: &[u8]| {
    if let Ok(r) = Roster::decode(data) {
        assert_eq!(r.encode().expect("roster encodes"), data);
    }
    if let Ok(j) = JoinReq::decode(data) {
        assert_eq!(j.encode(), data);
    }
    if let Ok(k) = Rekey::decode(data) {
        assert_eq!(k.encode().expect("rekey encodes"), data);
    }
});
