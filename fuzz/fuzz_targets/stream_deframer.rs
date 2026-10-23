//! L2CAP/tunnel stream deframer at arbitrary chunk boundaries (DESIGN §4.2): the first
//! input byte drives the chunk size, the rest is the stream. Frames are bounded, and a
//! re-encoded frame sequence must deframe to the same frames.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mesh_wire::link::{MAX_STREAM_FRAME, StreamDeframer, encode_stream};

fuzz_target!(|data: &[u8]| {
    let Some((&step, stream)) = data.split_first() else {
        return;
    };
    let step = usize::from(step).max(1);
    let mut d = StreamDeframer::new();
    let mut frames = Vec::new();
    for chunk in stream.chunks(step) {
        match d.feed(chunk) {
            Ok(f) => frames.extend(f),
            Err(_) => {
                assert!(d.is_poisoned());
                break;
            }
        }
    }
    let mut again = Vec::new();
    for f in &frames {
        assert!(!f.is_empty() && f.len() <= MAX_STREAM_FRAME);
        again.extend(encode_stream(f).expect("a deframed frame re-encodes"));
    }
    let mut d2 = StreamDeframer::new();
    assert_eq!(d2.feed(&again).expect("re-encoded stream is valid"), frames);
});
