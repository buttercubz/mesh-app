//! Spoken prompts (DESIGN §7.7): templates from `setPrompts`, `RenderPrompts` requests
//! (fixed prompts and known names at go-live, names learned mid-ride on demand), the PCM
//! that comes back through `PlatformEvent::PromptPcm` into the bounded prompt bank, and
//! cue playback with the earcon as the guaranteed cue (a prompt that is off, missing or
//! unrenderable falls back to its earcon).
//!
//! `PromptPcm` carries only the key, so one key rendered with several parameters (a name
//! per member) is matched to the oldest pending request of that key: the platform
//! renders a request batch in order.

use std::collections::{BTreeMap, VecDeque};

use mesh_api::{Lang, PromptKey};
use mesh_voice::{Earcon, PromptId};

use super::bus::Cue;
use super::state::Media;
use crate::Instant;
use crate::effects::{Effect, PromptRequest};
use crate::engine::Ctx;

/// Requests older than this are forgotten (rendering failed or is impossible now).
const PENDING_TIMEOUT_US: u64 = 30_000_000;
/// Pending requests kept at most.
const PENDING_MAX: usize = 96;

/// Replaces every `{{ … }}` placeholder of an i18next template with `param`.
pub fn fill_template(template: &str, param: Option<&str>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        match rest[start..].find("}}") {
            Some(end) => {
                out.push_str(param.unwrap_or(""));
                rest = &rest[start + end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Stable bank id of a prompt key with its parameter (FNV-1a).
pub fn prompt_id(key: PromptKey, param: Option<&str>) -> PromptId {
    let mut h: u32 = 0x811C_9DC5;
    let mut feed = |b: u8| {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    };
    key.as_str().bytes().for_each(&mut feed);
    feed(0);
    if let Some(p) = param {
        p.bytes().for_each(&mut feed);
    }
    PromptId(h)
}

/// The earcon a prompt falls back to when it cannot be spoken.
pub(super) fn fallback_earcon(key: PromptKey) -> Option<Earcon> {
    Some(match key {
        PromptKey::MemberJoined | PromptKey::LeaderChanged => Earcon::MemberJoined,
        PromptKey::MemberLeft | PromptKey::RidersOutOfRange => Earcon::MemberLost,
        PromptKey::Alone | PromptKey::BatteryLow => Earcon::Alone,
        PromptKey::MicPaused => Earcon::MicLost,
        PromptKey::HeadsetLost => Earcon::HeadsetLost,
        PromptKey::BluetoothOff => Earcon::BluetoothOff,
        PromptKey::TalkLimit | PromptKey::VoxLessSensitive => Earcon::Busy,
        PromptKey::RideStarted => Earcon::RideStarted,
        PromptKey::IntercomLive | PromptKey::MicBack => Earcon::GoAhead,
        _ => return None,
    })
}

/// `true` for the Demo-ride bot lines (rendered for the runtime, never banked here).
pub(super) fn is_demo(key: PromptKey) -> bool {
    key.as_str().starts_with("demo.")
}

/// Keys whose template takes a `{{name}}`.
pub(super) fn takes_name(key: PromptKey) -> bool {
    matches!(
        key,
        PromptKey::MemberJoined | PromptKey::MemberLeft | PromptKey::LeaderChanged
    )
}

/// Keys whose template takes a count `{{n}}`.
pub(super) fn takes_count(key: PromptKey) -> bool {
    matches!(key, PromptKey::IntercomLive | PromptKey::RidersOutOfRange)
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    key: PromptKey,
    id: PromptId,
    pinned: bool,
    at: Instant,
}

/// Templates and outstanding render requests.
#[derive(Debug, Clone)]
pub(super) struct PromptState {
    pub lang: Lang,
    pub enabled: bool,
    pub templates: BTreeMap<PromptKey, String>,
    pending: VecDeque<Pending>,
}

impl PromptState {
    pub fn new() -> Self {
        PromptState {
            lang: Lang::Es,
            enabled: true,
            templates: BTreeMap::new(),
            pending: VecDeque::new(),
        }
    }

    fn is_pending(&self, id: PromptId) -> bool {
        self.pending.iter().any(|p| p.id == id)
    }

    /// Requests for the PCM-carrying pending entry of `key` (oldest first).
    fn take(&mut self, key: PromptKey) -> Option<Pending> {
        let i = self.pending.iter().position(|p| p.key == key)?;
        self.pending.remove(i)
    }

    fn expire(&mut self, now: Instant) {
        self.pending
            .retain(|p| now.since(p.at) < PENDING_TIMEOUT_US);
    }
}

impl Media {
    /// Asks the platform to render `items` (key, param, pinned); already banked or
    /// pending prompts and keys without a template are skipped.
    pub(super) fn render_prompts(
        &mut self,
        ctx: &mut Ctx,
        items: &[(PromptKey, Option<String>, bool)],
    ) {
        let now = ctx.now;
        self.prompts.expire(now);
        let mut reqs = Vec::new();
        for (key, param, pinned) in items {
            let Some(t) = self.prompts.templates.get(key) else {
                continue;
            };
            let id = prompt_id(*key, param.as_deref());
            if self.bank.contains(id) || self.prompts.is_pending(id) {
                continue;
            }
            if self.prompts.pending.len() >= PENDING_MAX {
                break;
            }
            self.prompts.pending.push_back(Pending {
                key: *key,
                id,
                pinned: *pinned,
                at: now,
            });
            reqs.push(PromptRequest {
                key: *key,
                text: fill_template(t, param.as_deref()),
                lang: self.prompts.lang,
            });
        }
        if !reqs.is_empty() {
            ctx.fx.push(Effect::RenderPrompts(reqs));
        }
    }

    /// Go live (§7.7): every fixed prompt, `intercomLive{n}` and the names known now.
    pub(super) fn render_at_go_live(&mut self, ctx: &mut Ctx, riders: u8) {
        if !self.prompts.enabled {
            return;
        }
        let mut items: Vec<(PromptKey, Option<String>, bool)> = PromptKey::ALL
            .iter()
            .filter(|k| !is_demo(**k) && !takes_name(**k) && !takes_count(**k))
            .map(|k| (*k, None, true))
            .collect();
        items.push((PromptKey::IntercomLive, Some(riders.to_string()), true));
        let names: Vec<String> = self.names.values().cloned().collect();
        for n in names {
            for k in [PromptKey::MemberJoined, PromptKey::MemberLeft] {
                items.push((k, Some(n.clone()), false));
            }
        }
        self.render_prompts(ctx, &items);
    }

    /// Demo-ride lines for the bots (§3.4): rendered by platform TTS, consumed by the
    /// runtime's demo world (not banked here).
    pub(super) fn render_demo_lines(&mut self, ctx: &mut Ctx) {
        let reqs = super::demo::demo_prompt_requests(&self.prompts.templates, self.prompts.lang);
        if !reqs.is_empty() {
            ctx.fx.push(Effect::RenderPrompts(reqs));
        }
    }

    /// Rendered PCM for `key` (16 kHz mono).
    pub(super) fn on_prompt_pcm(&mut self, ctx: &Ctx, key: PromptKey, pcm: &[i16]) {
        if is_demo(key) {
            return;
        }
        let (id, pinned) = match self.prompts.take(key) {
            Some(p) => (p.id, p.pinned),
            None if !takes_name(key) && !takes_count(key) => (prompt_id(key, None), true),
            None => return,
        };
        let _ = self.bank.insert(id, pcm, pinned, ctx.now.us());
    }

    /// Plays a cue: its earcon, plus its prompt when enabled and rendered; otherwise the
    /// prompt's fallback earcon (and an on-demand render for next time).
    pub(super) fn play_cue(&mut self, ctx: &mut Ctx, cue: &Cue) {
        if let Some(e) = cue.earcon {
            self.earcons.trigger(ctx.now.us(), e);
        }
        if let Some(key) = cue.prompt {
            self.speak(ctx, key, cue.param.as_deref(), cue.earcon.is_none());
        }
    }

    /// Speaks `key` when enabled and rendered; otherwise plays its fallback earcon (when
    /// `fallback`) and asks for a render so the next occurrence can be spoken.
    pub(super) fn speak(
        &mut self,
        ctx: &mut Ctx,
        key: PromptKey,
        param: Option<&str>,
        fallback: bool,
    ) {
        let now = ctx.now.us();
        let id = prompt_id(key, param);
        if self.prompts.enabled && self.bank.contains(id) && self.player.enqueue(id) {
            self.bank.touch(id, now);
            return;
        }
        if fallback && let Some(fb) = fallback_earcon(key) {
            self.earcons.trigger(now, fb);
        }
        if self.prompts.enabled && !self.bank.contains(id) {
            let pinned = param.is_none();
            self.render_prompts(ctx, &[(key, param.map(str::to_string), pinned)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_ids() {
        assert_eq!(fill_template("{{name}} joined", Some("Ana")), "Ana joined");
        assert_eq!(
            fill_template("Intercom live, {{ n }} riders", Some("3")),
            "Intercom live, 3 riders"
        );
        assert_eq!(fill_template("no param", Some("x")), "no param");
        assert_eq!(fill_template("broken {{n", Some("x")), "broken {{n");
        let a = prompt_id(PromptKey::MemberJoined, Some("Ana"));
        assert_eq!(a, prompt_id(PromptKey::MemberJoined, Some("Ana")));
        assert_ne!(a, prompt_id(PromptKey::MemberJoined, Some("Bob")));
        assert_ne!(a, prompt_id(PromptKey::MemberLeft, Some("Ana")));
        assert!(is_demo(PromptKey::DemoLine3));
        assert!(!is_demo(PromptKey::Alone));
        for k in PromptKey::ALL {
            if !is_demo(*k) {
                assert!(fallback_earcon(*k).is_some(), "{k:?} has no fallback");
            }
        }
    }
}
