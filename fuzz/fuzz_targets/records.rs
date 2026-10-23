//! Record iterator and every record body decoder (DESIGN §4.4): bounded, at most 32
//! records; a decoded record re-encodes to bytes that decode to the same record (PAD and
//! LPROBE padding is re-emitted as zeros, so byte equality is not required there).
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::record::{MAX_RECORDS_PER_FRAME, Record, RecordIter};

fuzz_target!(|data: &[u8]| {
    let mut n = 0usize;
    let mut it = RecordIter::new(data);
    while let Some(raw) = it.next_raw() {
        let Ok(raw) = raw else { break };
        n += 1;
        assert!(n <= MAX_RECORDS_PER_FRAME);
        if let Ok(rec) = raw.parse() {
            assert_eq!(rec.rtype(), raw.rtype);
            check(raw.rtype, &rec);
        }
    }
    if let Some((&t, body)) = data.split_first()
        && let Ok(rec) = Record::decode(t & 0x1F, body)
    {
        check(t & 0x1F, &rec);
    }
});

fn check(rtype: u8, rec: &Record<'_>) {
    let body = rec.encode_body().expect("decoded body encodes");
    assert_eq!(Record::decode(rtype, &body).as_ref(), Ok(rec));
}
