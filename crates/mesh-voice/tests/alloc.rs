//! Counting allocator: the 10 ms capture and playout paths allocate nothing in steady
//! state (DESIGN §3.1, §7.5), including burst start/stop, with SimCodec and (feature
//! `opus`) libopus.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{FakeEnv, blocks, read_wav};
use mesh_voice::{
    AudioCheck, AudioCheckMode, BLOCK_SAMPLES, BLOCK_US, Block, CaptureChain, Codec, Earcon,
    Earcons, EncoderConfig, Mixer, Packetizer, Profile, PromptBank, PromptId, PromptPlayer,
    ReceivePath, RnnVad, SimCodec, StreamKey, TalkFsm, TalkInput, TalkMode, TalkOutput, VadEvent,
    VadSensitivity, VoiceFrame,
};

struct Counting;

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

fn bump() {
    let _ = ACTIVE.try_with(|a| {
        if a.get() {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn counted<R>(f: impl FnOnce() -> R) -> (R, usize) {
    COUNT.with(|c| c.set(0));
    ACTIVE.with(|a| a.set(true));
    let r = f();
    ACTIVE.with(|a| a.set(false));
    (r, COUNT.with(Cell::get))
}

#[test]
fn counting_allocator_sees_allocations() {
    let (_, n) = counted(|| vec![1u8; 32]);
    assert!(n >= 1, "the harness itself works");
}

/// The engine's per-tick media work, minus I/O.
struct Pipeline {
    chain: CaptureChain,
    fsm: TalkFsm,
    env: FakeEnv,
    tx: Packetizer,
    rx: ReceivePath,
    mixer: Mixer,
    earcons: Earcons,
    prompts: PromptPlayer,
    bank: PromptBank,
    check: AudioCheck,
    cfg: EncoderConfig,
    sent: [Option<VoiceFrame>; 4],
    n_sent: usize,
}

impl Pipeline {
    fn new(codec: &dyn Codec, profile: Profile) -> Self {
        let mut bank = PromptBank::new();
        bank.insert(PromptId(1), &vec![1000; 8_000], true, 0)
            .unwrap();
        Self {
            chain: CaptureChain::new(Box::new(RnnVad::new()), VadSensitivity::Medium),
            fsm: TalkFsm::new(TalkMode::Vox, 0),
            env: FakeEnv::admitting(),
            tx: Packetizer::new(codec.new_encoder().unwrap()),
            rx: ReceivePath::new(codec).unwrap(),
            mixer: Mixer::new(),
            earcons: Earcons::new(),
            prompts: PromptPlayer::new(),
            bank,
            check: AudioCheck::new(AudioCheckMode::Echo),
            cfg: EncoderConfig::for_profile(profile, 16_000, 5.0),
            sent: [None, None, None, None],
            n_sent: 0,
        }
    }

    /// One 10 ms tick: capture → VAD → FSM → packetizer; loop the frames back into the
    /// receive path as another origin; playout mix of streams + earcons + prompts.
    fn tick(&mut self, t: u64, captured: &Block, out: &mut Block) {
        let now = t * BLOCK_US;
        let mut b = *captured;
        let f = self.chain.process(&mut b);
        let mut outs = [None, None];
        if let Some(ev) = f.event {
            let input = if ev == VadEvent::On {
                TalkInput::VadOn
            } else {
                TalkInput::VadOff
            };
            outs[0] = Some(self.fsm.handle(now, input, &mut self.env));
        }
        outs[1] = Some(
            self.fsm
                .handle(now, TalkInput::Block { vad_p: f.vad_p }, &mut self.env),
        );
        self.n_sent = 0;
        let (sent, n_sent) = (&mut self.sent, &mut self.n_sent);
        let mut emit = |fr: &VoiceFrame| {
            if *n_sent < sent.len() {
                match &mut sent[*n_sent] {
                    Some(slot) => slot.clone_from(fr),
                    slot @ None => *slot = Some(fr.clone()),
                }
                *n_sent += 1;
            }
        };
        for o in outs.iter().flatten() {
            for x in o.iter() {
                match *x {
                    TalkOutput::StartBurst { burst, .. } => {
                        self.tx
                            .start_burst(burst, &self.cfg, self.chain.preroll(), &mut emit)
                            .unwrap();
                        self.earcons.trigger(now, Earcon::GoAhead);
                        self.prompts.enqueue(PromptId(1));
                    }
                    TalkOutput::StopBurst { flush, .. } => {
                        if flush {
                            self.tx.finish_burst(&mut emit).unwrap();
                        } else {
                            self.tx.abort_burst();
                        }
                    }
                    _ => {}
                }
            }
        }
        if !outs
            .iter()
            .flatten()
            .any(|o| o.contains(|x| matches!(x, TalkOutput::StartBurst { .. })))
        {
            self.tx.push_block(&b, &mut emit).unwrap();
        }
        let burst = self.tx.burst();
        for fr in self.sent.iter().take(self.n_sent).flatten() {
            for origin in 0..3u16 {
                self.rx.push(now, StreamKey { origin, sess: 1 }, fr);
            }
        }
        if !self.tx.is_active() {
            for origin in 0..3u16 {
                self.rx.talk_stop(StreamKey { origin, sess: 1 }, burst);
            }
        }
        self.rx.pull(now);
        let mut cue = [0i16; BLOCK_SAMPLES];
        let mut prompt = [0i16; BLOCK_SAMPLES];
        let mut echo = [0i16; BLOCK_SAMPLES];
        self.earcons.render(&mut cue);
        self.prompts.render(now, &self.bank, &mut prompt);
        let _ = self.check.process(&b, &f, &mut echo);
        self.rx
            .mix_into(&mut self.mixer, &[&cue, &prompt, &echo], out);
    }
}

fn run_steady_state(codec: &dyn Codec, profile: Profile) {
    let speech = blocks(&read_wav("speech.wav"));
    let mut p = Pipeline::new(codec, profile);
    let mut out = [0i16; BLOCK_SAMPLES];
    // Warm-up: first burst, first decodes, lazy library tables.
    for (t, b) in speech.iter().enumerate().take(500) {
        p.tick(t as u64, b, &mut out);
    }
    let ((), n) = counted(|| {
        for (t, b) in speech.iter().enumerate().skip(500) {
            p.tick(t as u64, b, &mut out);
        }
    });
    assert_eq!(n, 0, "{}: {n} allocations on the 10 ms paths", codec.name());
    let st =
        p.rx.jitter(StreamKey { origin: 0, sess: 1 })
            .unwrap()
            .stats();
    assert!(
        st.bursts >= 3 && st.decoded > 100,
        "the counted span covered real bursts: {st:?}"
    );
}

#[test]
fn no_allocation_in_steady_state_sim_codec() {
    run_steady_state(&SimCodec::new(), Profile::Ble16);
    run_steady_state(&SimCodec::new(), Profile::Wifi24Fec);
}

#[cfg(feature = "opus")]
#[test]
fn no_allocation_in_steady_state_opus() {
    run_steady_state(&mesh_voice::OpusCodec::new(), Profile::Ble16);
    run_steady_state(&mesh_voice::OpusCodec::new(), Profile::Wifi24Fec);
}
