//! Spoken prompts (DESIGN §7.7): a bounded bank (≤ 512 KB of 16 kHz PCM) filled through
//! `on_prompt_pcm`, and a player rate-limited to ≤ 1 prompt start per 4 s.
//!
//! Prompts are an addition to earcons, never the only cue. The caller (mesh-core media)
//! renders merged text (e.g. "2 riders joined") before asking for PCM, so the player only
//! de-duplicates identical queued ids.

use core::fmt;

use crate::{BLOCK_SAMPLES, Block};

/// Bank capacity in bytes (§7.7, Appendix A).
pub const PROMPT_BANK_BYTES: usize = 512 * 1024;
/// Bank capacity in samples.
pub const PROMPT_BANK_SAMPLES: usize = PROMPT_BANK_BYTES / 2;
/// Clips held at most.
pub const MAX_PROMPTS: usize = 128;
/// Minimum spacing between two prompt starts.
pub const PROMPT_MIN_INTERVAL_US: u64 = 4_000_000;
/// Pending prompts held by the player.
pub const PROMPT_QUEUE_LEN: usize = 4;

/// Opaque prompt identifier chosen by the caller (key + parameters).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PromptId(pub u32);

/// Bank failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptBankError {
    /// The clip alone exceeds the bank.
    TooLarge,
    /// Pinned clips leave no room (the prompt falls back to its earcon).
    Full,
    /// Empty PCM.
    Empty,
}

impl fmt::Display for PromptBankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PromptBankError::TooLarge => "prompt larger than the bank",
            PromptBankError::Full => "prompt bank full of pinned prompts",
            PromptBankError::Empty => "empty prompt",
        })
    }
}

impl std::error::Error for PromptBankError {}

#[derive(Debug, Clone, Copy)]
struct Entry {
    id: PromptId,
    offset: usize,
    len: usize,
    last_used_us: u64,
    pinned: bool,
}

/// Contiguous arena of prompt clips with LRU eviction of unpinned clips.
pub struct PromptBank {
    arena: Box<[i16]>,
    entries: Vec<Entry>,
    used: usize,
}

impl PromptBank {
    pub fn new() -> Self {
        Self::with_capacity_samples(PROMPT_BANK_SAMPLES)
    }

    /// A smaller bank (tests); never larger than 512 KB.
    pub fn with_capacity_samples(samples: usize) -> Self {
        let cap = samples.min(PROMPT_BANK_SAMPLES);
        Self {
            arena: vec![0; cap].into_boxed_slice(),
            entries: Vec::with_capacity(MAX_PROMPTS),
            used: 0,
        }
    }

    pub fn capacity_bytes(&self) -> usize {
        self.arena.len() * 2
    }

    pub fn used_bytes(&self) -> usize {
        self.used * 2
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, id: PromptId) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    /// Stores a 16 kHz mono clip. `pinned` clips (fixed prompts rendered at Go live) are
    /// never evicted; others are evicted least-recently-used first.
    pub fn insert(
        &mut self,
        id: PromptId,
        pcm: &[i16],
        pinned: bool,
        now_us: u64,
    ) -> Result<(), PromptBankError> {
        if pcm.is_empty() {
            return Err(PromptBankError::Empty);
        }
        if pcm.len() > self.arena.len() {
            return Err(PromptBankError::TooLarge);
        }
        self.remove(id);
        while self.arena.len() - self.used < pcm.len() || self.entries.len() >= MAX_PROMPTS {
            let victim = self
                .entries
                .iter()
                .filter(|e| !e.pinned)
                .min_by_key(|e| (e.last_used_us, e.id))
                .map(|e| e.id);
            match victim {
                Some(v) => {
                    self.remove(v);
                }
                None => return Err(PromptBankError::Full),
            }
        }
        let offset = self.used;
        self.arena[offset..offset + pcm.len()].copy_from_slice(pcm);
        self.used += pcm.len();
        self.entries.push(Entry {
            id,
            offset,
            len: pcm.len(),
            last_used_us: now_us,
            pinned,
        });
        Ok(())
    }

    /// Removes a clip, compacting the arena. Returns true when it existed.
    pub fn remove(&mut self, id: PromptId) -> bool {
        let Some(pos) = self.entries.iter().position(|e| e.id == id) else {
            return false;
        };
        let e = self.entries.remove(pos);
        self.arena
            .copy_within(e.offset + e.len..self.used, e.offset);
        self.used -= e.len;
        for other in &mut self.entries {
            if other.offset > e.offset {
                other.offset -= e.len;
            }
        }
        true
    }

    /// The clip of `id`.
    pub fn get(&self, id: PromptId) -> Option<&[i16]> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| &self.arena[e.offset..e.offset + e.len])
    }

    /// Marks a clip as used now (LRU).
    pub fn touch(&mut self, id: PromptId, now_us: u64) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.last_used_us = now_us;
        }
    }

    /// Drops every unpinned clip.
    pub fn clear_unpinned(&mut self) {
        while let Some(id) = self.entries.iter().find(|e| !e.pinned).map(|e| e.id) {
            self.remove(id);
        }
    }
}

impl Default for PromptBank {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PromptBank {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptBank")
            .field("prompts", &self.entries.len())
            .field("used_bytes", &self.used_bytes())
            .finish()
    }
}

/// Plays queued prompts from a [`PromptBank`], ≤ 1 start per 4 s.
#[derive(Debug, Clone)]
pub struct PromptPlayer {
    queue: [Option<PromptId>; PROMPT_QUEUE_LEN],
    playing: Option<(PromptId, usize)>,
    last_start_us: Option<u64>,
}

impl PromptPlayer {
    pub fn new() -> Self {
        Self {
            queue: [None; PROMPT_QUEUE_LEN],
            playing: None,
            last_start_us: None,
        }
    }

    /// Queues a prompt; identical queued ids merge. False when the queue is full.
    pub fn enqueue(&mut self, id: PromptId) -> bool {
        if self.queue.contains(&Some(id)) || self.playing.is_some_and(|(p, _)| p == id) {
            return true;
        }
        match self.queue.iter_mut().find(|q| q.is_none()) {
            Some(slot) => {
                *slot = Some(id);
                true
            }
            None => false,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Prompt currently audible.
    pub fn current(&self) -> Option<PromptId> {
        self.playing.map(|(id, _)| id)
    }

    pub fn clear(&mut self) {
        self.queue = [None; PROMPT_QUEUE_LEN];
        self.playing = None;
    }

    /// Renders the next 10 ms. Returns true when `out` carries prompt audio.
    pub fn render(&mut self, now_us: u64, bank: &PromptBank, out: &mut Block) -> bool {
        out.fill(0);
        if self.playing.is_none() {
            let may_start = self
                .last_start_us
                .is_none_or(|t| now_us.saturating_sub(t) >= PROMPT_MIN_INTERVAL_US);
            if !may_start {
                return false;
            }
            while let Some(id) = self.queue[0].take() {
                self.queue.rotate_left(1);
                if bank.contains(id) {
                    self.playing = Some((id, 0));
                    self.last_start_us = Some(now_us);
                    break;
                }
            }
        }
        let Some((id, pos)) = self.playing else {
            return false;
        };
        let Some(clip) = bank.get(id) else {
            self.playing = None;
            return false;
        };
        let end = (pos + BLOCK_SAMPLES).min(clip.len());
        out[..end - pos].copy_from_slice(&clip[pos..end]);
        self.playing = if end >= clip.len() {
            None
        } else {
            Some((id, end))
        };
        true
    }
}

impl Default for PromptPlayer {
    fn default() -> Self {
        Self::new()
    }
}
