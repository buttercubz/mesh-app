//! The Demo world (DESIGN §3.4) driven the way a runtime drives it: the user's engine
//! joins the bots' group over virtual links, the ride starts, and the bots' voice is
//! played on the user's engine; with rendered lines and with the synthesized fallback.

use mesh_api::{TalkMode, UiCommand};
use mesh_core::effects::{AudioCommand, Effect};
use mesh_core::{BLOCK_SAMPLES, Engine, EngineConfig, Instant, Platform};
use mesh_sim::demo::{DemoError, DemoWorld, Pcm};

struct Run {
    local: Engine,
    demo: DemoWorld,
    now: Instant,
    audio_open: bool,
    loud_blocks: u64,
    render_requests: usize,
}

impl Run {
    fn new(platform: Platform, lines: Option<Vec<Pcm>>) -> Run {
        let mut cfg = EngineConfig::new(platform, [7; 32]);
        cfg.wall_ms = 1_792_746_723_120;
        let mut local = Engine::new(cfg, Instant::ZERO);
        // The app hands its prompt templates at every start (§7.7), demo lines included.
        let templates = mesh_core::media::demo::DEMO_LINES
            .iter()
            .enumerate()
            .map(|(k, key)| (*key, format!("Demo line {}", k + 1)))
            .collect();
        let r = local.handle_command(
            Instant::ZERO,
            UiCommand::SetPrompts {
                lang: mesh_api::Lang::En,
                enabled: true,
                templates,
            },
        );
        assert!(r.is_ok(), "{r:?}");
        let demo = DemoWorld::attach(&mut local, Instant::ZERO, lines).expect("demo attaches");
        Run {
            local,
            demo,
            now: Instant::ZERO,
            audio_open: false,
            loud_blocks: 0,
            render_requests: 0,
        }
    }

    /// The runtime loop, every 5 ms (audio every 10 ms).
    fn run_ms(&mut self, ms: u64) {
        for _ in 0..ms / 5 {
            self.now = self.now.plus_us(5_000);
            let now = self.now;
            self.local.handle_timeout(now);
            if self.audio_open && now.0.is_multiple_of(10_000) {
                self.local.push_capture(now, &[0; BLOCK_SAMPLES]);
                let mut out = [0i16; BLOCK_SAMPLES];
                self.local.pull_playout(now, &mut out);
                if out.iter().any(|s| s.unsigned_abs() > 1_000) {
                    self.loud_blocks += 1;
                }
            }
            self.pump();
            self.demo.drive(&mut self.local, now);
            self.pump();
        }
    }

    fn pump(&mut self) {
        let now = self.now;
        loop {
            let mut any = false;
            while let Some(t) = self.local.poll_transmit(now) {
                any = true;
                assert!(
                    self.demo.on_local_transmit(now, &t),
                    "only demo links exist"
                );
            }
            while let Some(e) = self.local.poll_effect() {
                any = true;
                match e {
                    Effect::Radio(c) => {
                        assert!(self.demo.on_local_radio(now, &c));
                    }
                    Effect::Audio(AudioCommand::OpenSession { .. }) => {
                        self.audio_open = true;
                        self.local.handle_event(
                            now,
                            mesh_core::PlatformEvent::AudioIo(mesh_core::AudioIoState::Running),
                        );
                    }
                    Effect::RenderPrompts(r) => self.render_requests += r.len(),
                    _ => {}
                }
            }
            if !any {
                break;
            }
        }
    }

    fn snapshot_members(&mut self) -> usize {
        let r = self
            .local
            .handle_command(self.now, UiCommand::RequestSnapshot {});
        let _ = r;
        let mut n = 0;
        while let Some(e) = self.local.poll_effect() {
            if let Effect::Ui(mesh_api::UiEvent::Snapshot { s }) = e {
                n = usize::from(s.mesh.reachable);
            }
        }
        n
    }
}

fn tone(ms: usize, hz: f32) -> Pcm {
    (0..ms * 16)
        .map(|i| ((i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 9_000.0) as i16)
        .collect()
}

fn ride(platform: Platform, lines: Option<Vec<Pcm>>) -> Run {
    let mut r = Run::new(platform, lines);
    r.run_ms(8_000);
    assert!(r.demo.ride_started, "the leader bot started the ride");
    let ok = r.local.handle_command(r.now, UiCommand::GoLive {});
    assert!(ok.is_ok(), "go live: {ok:?}");
    let ok = r.local.handle_command(
        r.now,
        UiCommand::SetTalkMode {
            mode: TalkMode::Ptt,
        },
    );
    assert!(ok.is_ok(), "{ok:?}");
    r.run_ms(30_000);
    r
}

#[test]
fn bots_ride_and_talk_with_the_synthesized_fallback() {
    let mut r = ride(Platform::WebSim, None);
    assert!(!r.demo.has_lines());
    assert!(
        r.render_requests >= 8,
        "the user's engine asked for the 8 demo lines"
    );
    assert!(
        r.demo.utterances() >= 3,
        "{} utterances",
        r.demo.utterances()
    );
    assert!(
        r.loud_blocks > 200,
        "heard {} loud blocks of bot speech",
        r.loud_blocks
    );
    assert_eq!(r.snapshot_members(), 5, "everyone reachable");
}

#[test]
fn bots_speak_rendered_lines_encoded_once() {
    let lines: Vec<Pcm> = (0..8)
        .map(|k| tone(2_000, 300.0 + 40.0 * k as f32))
        .collect();
    let r = ride(Platform::Ios, Some(lines));
    assert!(r.demo.has_lines());
    assert!(r.demo.utterances() >= 3);
    assert!(r.loud_blocks > 200, "heard {} loud blocks", r.loud_blocks);
}

#[test]
fn lines_rendered_later_replace_the_fallback_and_busy_engines_are_refused() {
    let mut r = Run::new(Platform::WebSim, None);
    r.run_ms(1_000);
    r.demo.set_lines(vec![tone(1_000, 440.0)]);
    assert!(r.demo.has_lines());
    r.demo.set_lines(vec![Vec::new()]);
    assert!(
        r.demo.has_lines(),
        "an empty render keeps the previous voice"
    );
    // The user's engine is in the demo session now: a second attach is refused.
    assert!(matches!(
        DemoWorld::attach(&mut r.local, r.now, None),
        Err(DemoError::NotIdle)
    ));
    let now = r.now;
    r.demo.detach(&mut r.local, now);
}
