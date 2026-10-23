//! Receive path (DESIGN §6.3, §7.5): a pool of 6 preallocated decoders, one jitter buffer
//! per `(origin, sess)` stream, and selection of the ≤ 4 best-ranked streams for the mixer.

use crate::codec::{Codec, CodecError, VoiceDecoder};
use crate::jitter::{JitterBuffer, JitterStats, PullStatus, PushResult};
use crate::mixer::{MAX_MIX_STREAMS, Mixer};
use crate::packetizer::VoiceFrame;
use crate::{BLOCK_SAMPLES, Block};

/// Decoders (and stream slots) held by the pool (§7.5: 6, ≤ 4 active in the mix).
pub const POOL_SIZE: usize = 6;

/// Jitter-buffer key: never the previous hop (§7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamKey {
    pub origin: u16,
    pub sess: u8,
}

/// Six decoders created up front.
pub struct DecoderPool {
    decoders: Vec<Box<dyn VoiceDecoder>>,
}

impl DecoderPool {
    pub fn new(codec: &dyn Codec) -> Result<Self, CodecError> {
        let mut decoders = Vec::with_capacity(POOL_SIZE);
        for _ in 0..POOL_SIZE {
            decoders.push(codec.new_decoder()?);
        }
        Ok(Self { decoders })
    }

    pub fn len(&self) -> usize {
        self.decoders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decoders.is_empty()
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut (dyn VoiceDecoder + 'static)> {
        self.decoders.get_mut(i).map(|d| &mut **d)
    }
}

impl core::fmt::Debug for DecoderPool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecoderPool")
            .field("len", &self.decoders.len())
            .finish()
    }
}

/// Outcome of [`ReceivePath::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveResult {
    Buffered(PushResult),
    /// All 6 slots carry other live streams.
    NoSlot,
}

struct StreamSlot {
    key: Option<StreamKey>,
    jb: JitterBuffer,
    rank: u32,
    status: PullStatus,
    pcm: Block,
}

/// All receive streams of a session.
pub struct ReceivePath {
    pool: DecoderPool,
    slots: Box<[StreamSlot]>,
    selected: [usize; MAX_MIX_STREAMS],
    n_selected: usize,
    /// Counters of streams whose slot was reused (kept for [`ReceivePath::totals`]).
    retired: JitterStats,
}

impl ReceivePath {
    pub fn new(codec: &dyn Codec) -> Result<Self, CodecError> {
        let pool = DecoderPool::new(codec)?;
        let slots = (0..POOL_SIZE)
            .map(|_| StreamSlot {
                key: None,
                jb: JitterBuffer::new(),
                rank: u32::MAX,
                status: PullStatus::Idle,
                pcm: [0; BLOCK_SAMPLES],
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            pool,
            slots,
            selected: [0; MAX_MIX_STREAMS],
            n_selected: 0,
            retired: JitterStats::default(),
        })
    }

    fn slot_of(&self, key: StreamKey) -> Option<usize> {
        self.slots.iter().position(|s| s.key == Some(key))
    }

    /// Delivers one VOICE frame of `key` received at `now_us`.
    pub fn push(&mut self, now_us: u64, key: StreamKey, frame: &VoiceFrame) -> ReceiveResult {
        let i = match self.slot_of(key) {
            Some(i) => i,
            None => match self
                .slots
                .iter()
                .position(|s| s.key.is_none() || s.jb.is_idle())
            {
                Some(i) => {
                    let s = &mut self.slots[i];
                    self.retired.accumulate(&s.jb.stats());
                    s.key = Some(key);
                    s.jb.reset();
                    s.rank = u32::MAX;
                    i
                }
                None => return ReceiveResult::NoSlot,
            },
        };
        ReceiveResult::Buffered(self.slots[i].jb.push(now_us, frame))
    }

    /// TALK STOP from `key` for `burst`.
    pub fn talk_stop(&mut self, key: StreamKey, burst: u8) {
        if let Some(i) = self.slot_of(key) {
            self.slots[i].jb.end_burst(burst);
        }
    }

    /// Floor rank of a stream (0 = best; from the floor table). Unranked streams sort last.
    pub fn set_rank(&mut self, key: StreamKey, rank: u32) {
        if let Some(i) = self.slot_of(key) {
            self.slots[i].rank = rank;
        }
    }

    /// Pulls 10 ms from every stream and selects the ≤ 4 best-ranked audible ones.
    pub fn pull(&mut self, now_us: u64) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if s.key.is_none() {
                s.status = PullStatus::Idle;
                continue;
            }
            let dec = match self.pool.get_mut(i) {
                Some(d) => d,
                None => continue,
            };
            s.status = s.jb.pull(now_us, dec, &mut s.pcm);
        }
        self.n_selected = 0;
        let mut taken = [false; POOL_SIZE];
        while self.n_selected < MAX_MIX_STREAMS {
            let best = (0..self.slots.len())
                .filter(|&i| !taken[i] && self.slots[i].status.is_audio())
                .min_by_key(|&i| (self.slots[i].rank, self.slots[i].key));
            match best {
                Some(i) => {
                    taken[i] = true;
                    self.selected[self.n_selected] = i;
                    self.n_selected += 1;
                }
                None => break,
            }
        }
    }

    /// Streams selected by the last [`ReceivePath::pull`], best first.
    pub fn selected(&self) -> impl Iterator<Item = (StreamKey, &Block)> {
        self.selected[..self.n_selected]
            .iter()
            .filter_map(|&i| self.slots[i].key.map(|k| (k, &self.slots[i].pcm)))
    }

    /// Mixes the selected streams plus `cues` into `out` through `mixer`.
    pub fn mix_into(&self, mixer: &mut Mixer, cues: &[&Block], out: &mut Block) {
        let mut refs: [&Block; MAX_MIX_STREAMS] = [&self.slots[0].pcm; MAX_MIX_STREAMS];
        let mut n = 0;
        for (_, pcm) in self.selected() {
            refs[n] = pcm;
            n += 1;
        }
        mixer.mix(&refs[..n], cues, out);
    }

    /// Last pull status of `key`.
    pub fn status(&self, key: StreamKey) -> Option<PullStatus> {
        self.slot_of(key).map(|i| self.slots[i].status)
    }

    /// The jitter buffer of `key` (diagnostics).
    pub fn jitter(&self, key: StreamKey) -> Option<&JitterBuffer> {
        self.slot_of(key).map(|i| &self.slots[i].jb)
    }

    /// Jitter-buffer counters summed over every stream of this session, including
    /// streams whose slot was reused (late frames, concealment, stretch, drops; for the
    /// diagnostics recorder).
    pub fn totals(&self) -> JitterStats {
        let mut t = self.retired;
        for s in self.slots.iter() {
            t.accumulate(&s.jb.stats());
        }
        t
    }

    /// Streams with an active burst.
    pub fn active_streams(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.key.is_some() && !s.jb.is_idle())
            .count()
    }
}

impl core::fmt::Debug for ReceivePath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReceivePath")
            .field("active", &self.active_streams())
            .finish_non_exhaustive()
    }
}
