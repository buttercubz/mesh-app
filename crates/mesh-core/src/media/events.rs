//! Platform events seen by the media module: rendered prompt PCM, and — while no session
//! module drives the cues (standalone build, provisional identity) — the cues the
//! platform state implies (§6.5): Bluetooth off (repeated every 60 s while off), mic lost
//! / back, headset lost, battery low. With the session module present these cues arrive
//! as [`MediaRequest::Cue`](super::MediaRequest) instead, so they never play twice.

use mesh_api::PromptKey;
use mesh_voice::Earcon;

use super::bus::Cue;
use super::state::Media;
use super::{BATTERY_CUE_INTERVAL_US, BATTERY_LOW, BLUETOOTH_CUE_REPEAT_US};
use crate::engine::Ctx;
use crate::links::RadioAvail;
use crate::{AudioIoState, PlatformEvent};

impl Media {
    /// The module plays platform-derived cues itself.
    fn standalone_cues(&self) -> bool {
        self.live && (self.identity.is_none() || self.provisional)
    }

    pub(super) fn on_event(&mut self, ctx: &mut Ctx, ev: &PlatformEvent<'_>) {
        let now = ctx.now;
        match ev {
            PlatformEvent::PromptPcm { key, pcm } => self.on_prompt_pcm(ctx, *key, pcm),
            PlatformEvent::AudioRoute(r) => {
                let was = self.cues.headset.replace(r.headset_present);
                if was == Some(true) && !r.headset_present && self.standalone_cues() {
                    self.play_cue(
                        ctx,
                        &Cue::with_prompt(Some(Earcon::HeadsetLost), PromptKey::HeadsetLost),
                    );
                }
            }
            PlatformEvent::AudioIo(s) => match s {
                AudioIoState::CaptureLost => {
                    if !self.cues.capture_lost && self.standalone_cues() {
                        self.play_cue(
                            ctx,
                            &Cue::with_prompt(Some(Earcon::MicLost), PromptKey::MicPaused),
                        );
                    }
                    self.cues.capture_lost = true;
                }
                AudioIoState::Restored | AudioIoState::Running if self.cues.capture_lost => {
                    self.cues.capture_lost = false;
                    if self.standalone_cues() {
                        self.play_cue(ctx, &Cue::with_prompt(None, PromptKey::MicBack));
                    }
                    self.chain.reset();
                }
                _ => {}
            },
            PlatformEvent::RadioState { ble, .. } => {
                let off = *ble == RadioAvail::Off;
                if off && !self.cues.radio_off {
                    self.cues.next_bt_cue = Some(now);
                } else if !off {
                    self.cues.next_bt_cue = None;
                }
                self.cues.radio_off = off;
                self.platform_timer(ctx);
            }
            PlatformEvent::Platform(s) => {
                let low = s.battery.is_some_and(|b| b < BATTERY_LOW) && s.charging != Some(true);
                let due = self
                    .cues
                    .last_battery_cue
                    .is_none_or(|t| now.since(t) >= BATTERY_CUE_INTERVAL_US);
                if low && due && self.standalone_cues() {
                    self.cues.last_battery_cue = Some(now);
                    self.play_cue(ctx, &Cue::with_prompt(None, PromptKey::BatteryLow));
                }
            }
            _ => {}
        }
    }

    /// Repeats the Bluetooth-off cue every 60 s while the radio stays off.
    pub(super) fn platform_timer(&mut self, ctx: &mut Ctx) {
        let now = ctx.now;
        if let Some(t) = self.cues.next_bt_cue
            && now >= t
        {
            self.cues.next_bt_cue = Some(now.plus_us(BLUETOOTH_CUE_REPEAT_US));
            if self.standalone_cues() {
                self.play_cue(
                    ctx,
                    &Cue::with_prompt(Some(Earcon::BluetoothOff), PromptKey::BluetoothOff),
                );
            }
        }
    }
}
