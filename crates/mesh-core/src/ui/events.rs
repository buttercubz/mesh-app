//! Base handling of platform events (DESIGN §3.2): links, discovery and HELLO,
//! radio state, audio route/IO and platform samples. Modules see every event after
//! this through `CoreModule::on_event`.

use mesh_api::{NoticeKey, Params, UiEvent};
use mesh_wire::frame::RejectReason;

use crate::engine::{Engine, EngineMode};
use crate::links::{LinkDownReason, LinkSecurity, Plain, RadioAvail};
use crate::{LinkId, PlatformEvent};

/// Applies one platform event to the engine's own state.
pub(crate) fn on_platform_event(e: &mut Engine, ev: &PlatformEvent<'_>) {
    let now = e.ctx.now;
    let limits = e.ctx.cfg.limits;
    match ev {
        PlatformEvent::Up { link, info } => link_up(e, *link, *info),
        PlatformEvent::Down { link, reason } => {
            e.ctx.links.on_down(*link, *reason);
            e.process_link_events();
        }
        PlatformEvent::Writable { link } => e.ctx.links.on_writable(*link),
        PlatformEvent::WouldBlock { link, unit } => e.ctx.links.on_would_block(*link, unit),
        PlatformEvent::Metrics {
            link,
            rssi,
            att_mtu,
        } => {
            e.ctx.links.on_metrics(*link, *rssi, *att_mtu);
            if e.ctx.is_lab() {
                e.lab.on_metrics(*link, *rssi, *att_mtu);
            }
        }
        PlatformEvent::Discovered(d) => {
            let n = e.ctx.ble_links();
            e.ctx.radio.on_discovered(now, d, n, &limits, &mut e.ctx.fx);
        }
        PlatformEvent::HelloRead { disc_id, bytes } => {
            let handled = if matches!(e.ctx.mode, EngineMode::Session { .. }) {
                let ctx = &mut e.ctx;
                e.modules
                    .first(|m| m.on_hello_read(ctx, *disc_id, bytes).then_some(()))
                    .is_some()
            } else {
                false
            };
            if !handled {
                e.ctx
                    .radio
                    .on_hello_read(now, *disc_id, bytes, &limits, &mut e.ctx.fx);
            }
        }
        PlatformEvent::ConnectFailed { disc_id, reason } => {
            e.ctx
                .radio
                .on_connect_failed(now, *disc_id, *reason, &limits);
            if *reason == LinkDownReason::LimitReached && !e.ctx.is_lab() {
                e.ctx.notice(NoticeKey::LinkLimitReached, Params::new());
            }
        }
        PlatformEvent::L2capPublished { psm } => {
            e.ctx.radio.set_psm(*psm);
            e.ctx.refresh_radio();
        }
        PlatformEvent::AudioRoute(r) => {
            let changed = e.ctx.device.route.kind != r.kind;
            e.ctx.device.route = *r;
            if changed {
                e.ctx.ui_event(UiEvent::RouteChanged { route: r.kind });
            }
            e.ctx.mark_dirty();
            if e.ctx.is_lab() {
                e.lab.on_route(&mut e.ctx);
            }
        }
        PlatformEvent::AudioIo(s) => {
            e.ctx.device.io = *s;
            e.ctx.mark_dirty();
            if e.ctx.is_lab() {
                e.lab.on_audio_io(&mut e.ctx, *s);
            }
        }
        PlatformEvent::Platform(s) => {
            e.ctx.device.on_sample(now, s.clone());
            e.ctx.mark_dirty();
            if e.ctx.is_lab() {
                e.lab.on_platform(&mut e.ctx);
            }
        }
        PlatformEvent::RadioState { ble, aware } => radio_state(e, *ble, *aware),
        PlatformEvent::PromptPcm { .. } => {}
    }
}

fn link_up(e: &mut Engine, link: LinkId, info: crate::LinkInfo) {
    e.ctx.radio.on_link_opened(link, &info);
    let security: Option<Box<dyn LinkSecurity>> = match e.ctx.mode {
        EngineMode::Lab { .. } => Some(Box::new(Plain::new(e.lab.lab_tag()))),
        EngineMode::Session { .. } => {
            let ctx = &mut e.ctx;
            e.modules.first(|m| m.security_for(ctx, link, &info))
        }
        EngineMode::Idle => None,
    };
    match security {
        Some(sec) => {
            e.ctx.links.add(e.ctx.now, link, info, sec, &mut e.ctx.fx);
        }
        None => {
            // No radios in Idle, and no secure module in this build: refuse at once.
            e.ctx.fx.radio(crate::effects::RadioCommand::Close {
                link,
                reason: Some(RejectReason::HandshakeFailure),
            });
        }
    }
    e.process_link_events();
}

fn radio_state(e: &mut Engine, ble: RadioAvail, aware: RadioAvail) {
    let now = e.ctx.now;
    let limits = e.ctx.cfg.limits;
    let before = e.ctx.radio.ble;
    let n = e.ctx.ble_links();
    let went_off = e
        .ctx
        .radio
        .on_radio_state(ble, aware, n, &limits, now, &mut e.ctx.fx);
    if went_off {
        // The platform's BLE links are dead: mark them down with RadioOff (§3.2).
        e.ctx.links.drop_ble(LinkDownReason::RadioOff);
        e.process_link_events();
    }
    e.ctx.mark_dirty();
    if before == ble {
        return;
    }
    if e.ctx.is_lab() {
        e.lab.on_radio_state(&mut e.ctx, ble);
    }
    // In a group session the session module raises the notices with its cue and
    // PRESENCE bit (WP-10B); elsewhere the base engine tells the UI.
    if matches!(e.ctx.mode, EngineMode::Session { .. }) {
        return;
    }
    match ble {
        RadioAvail::Off => e.ctx.notice(NoticeKey::BluetoothOff, Params::new()),
        RadioAvail::Unauthorized => e.ctx.notice(NoticeKey::BluetoothDenied, Params::new()),
        RadioAvail::On if before != RadioAvail::On => {
            e.ctx.notice(NoticeKey::BluetoothOn, Params::new());
        }
        _ => {}
    }
}
