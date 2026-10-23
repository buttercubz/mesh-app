//! Scripted talk schedules (DESIGN §13.2): each talker's microphone hears the
//! deterministic speech-like signal inside its talk spans and silence elsewhere; in PTT
//! the world presses and releases the button at the span edges, in VOX the real capture
//! chain (VAD, talk FSM) decides. Onset detectors on talkers' microphones and listeners'
//! ears give the modeled mouth-to-ear latency of every syllable ([`crate::onset`]).

use crate::onset::OnsetDetector;
use crate::rng::SimRng;
use crate::world::{Ev, World};

/// One scripted talk span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TalkSpan {
    pub node: usize,
    pub from_us: u64,
    pub until_us: u64,
}

impl World {
    /// Node `i` speaks during `[from_us, until_us)`; with `ptt` the button is held over
    /// the span. Times are rounded to the 10 ms block grid.
    pub fn script_talk(&mut self, i: usize, from_us: u64, until_us: u64, ptt: bool) {
        let a = from_us.div_ceil(10_000) * 10_000;
        let b = until_us.div_ceil(10_000) * 10_000;
        if b <= a {
            return;
        }
        self.nodes[i]
            .audio
            .talk
            .get_or_insert_with(Vec::new)
            .push((a, b));
        if ptt {
            self.schedule(
                a,
                Ev::Ptt {
                    node: i,
                    down: true,
                },
            );
            self.schedule(
                b,
                Ev::Ptt {
                    node: i,
                    down: false,
                },
            );
        }
        self.talk_log.push(TalkSpan {
            node: i,
            from_us: a,
            until_us: b,
        });
    }

    /// Continuous speech on node `i` (no breath pauses, so VOX never releases mid-span).
    pub fn steady_voice(&mut self, i: usize) {
        self.nodes[i].audio.mic.breath_prob = 0.0;
    }

    /// Makes node `i`'s microphone silent outside talk spans (no span yet).
    pub fn mute_mic(&mut self, i: usize) {
        self.nodes[i].audio.talk.get_or_insert_with(Vec::new);
    }

    /// Enables the mouth and ear onset detectors on node `i`.
    pub fn track_onsets(&mut self, i: usize) {
        let a = &mut self.nodes[i].audio;
        a.mouth.get_or_insert_with(OnsetDetector::new);
        a.ear.get_or_insert_with(OnsetDetector::new);
    }

    /// Every scripted span so far.
    pub fn talk_spans(&self) -> &[TalkSpan] {
        &self.talk_log
    }
}

/// A conversation: talkers take turns of `turn_ms` (uniform in the range) separated by
/// pauses of `gap_ms`, from `start_us` until `end_us`, the next talker drawn at random
/// among `talkers` (never the same twice in a row when there are several).
pub fn turns(
    rng: &mut SimRng,
    talkers: &[usize],
    start_us: u64,
    end_us: u64,
    turn_ms: (u64, u64),
    gap_ms: (u64, u64),
) -> Vec<TalkSpan> {
    let mut v = Vec::new();
    let mut t = start_us;
    let mut last: Option<usize> = None;
    while t < end_us && !talkers.is_empty() {
        let mut who = talkers[rng.range_u64(0, talkers.len() as u64 - 1) as usize];
        if talkers.len() > 1 && Some(who) == last {
            who =
                talkers[(talkers.iter().position(|x| *x == who).unwrap_or(0) + 1) % talkers.len()];
        }
        let len = rng.range_u64(turn_ms.0, turn_ms.1) * 1_000;
        let until = (t + len).min(end_us);
        v.push(TalkSpan {
            node: who,
            from_us: t,
            until_us: until,
        });
        last = Some(who);
        t = until + rng.range_u64(gap_ms.0, gap_ms.1) * 1_000;
    }
    v
}
