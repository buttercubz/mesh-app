//! Lab sessions through the runtime: two engines over a virtual link, link
//! backpressure (WouldBlock → Writable) and start/stop cycles.

use std::sync::Weak;

use super::*;
use crate::api::types::{
    FfiLinkDownReason, FfiLinkInfo, FfiLinkKind, FfiLinkRole, FfiLinkUnit, FfiRuntimeStatus,
    FfiSessionStart,
};

fn l2cap(role: FfiLinkRole) -> FfiLinkInfo {
    FfiLinkInfo {
        kind: FfiLinkKind::BleL2cap,
        role,
        max_frame: 1024,
        unit: FfiLinkUnit::Stream,
        peer_platform_hint: Some(crate::api::types::FfiPeerPlatform::Ios),
        att_mtu: None,
        disc_id: None,
    }
}

/// Latest snapshot `mesh.links` seen by `r`'s hooks.
fn links_in_snapshot(r: &Rig) -> Option<u64> {
    r.cmd(r#"{"type":"requestSnapshot"}"#);
    r.hooks
        .ui_events()
        .iter()
        .rev()
        .find(|e| e["type"] == "snapshot")
        .and_then(|e| e["s"]["mesh"]["links"].as_u64())
}

/// Forwards every unit sent by `from` on `link` to `to` (weakly, no cycle).
fn wire(from: &Rig, to: &Rig) {
    let peer: Weak<MeshEngine> = Arc::downgrade(&to.engine);
    from.hooks
        .set_send(move |link, bytes| match peer.upgrade() {
            Some(p) => {
                p.on_link_data(link, bytes.to_vec());
                FfiSendOutcome::Accepted
            }
            None => FfiSendOutcome::NoLink,
        });
}

#[test]
fn two_runtimes_bring_a_lab_link_up_over_a_virtual_link() {
    let a = Rig::new(40);
    let b = Rig::new(41);
    wire(&a, &b);
    wire(&b, &a);
    a.engine.set_ui_attached(true);
    b.engine.set_ui_attached(true);
    a.engine
        .start(FfiSessionStart::Lab { lab_code: 1234 })
        .unwrap();
    b.engine
        .start(FfiSessionStart::Lab { lab_code: 1234 })
        .unwrap();
    b.engine.on_link_up(1, l2cap(FfiLinkRole::Responder));
    a.engine.on_link_up(1, l2cap(FfiLinkRole::Initiator));
    wait_until("both sides see one link", || {
        links_in_snapshot(&a) == Some(1) && links_in_snapshot(&b) == Some(1)
    });
    // Radio decisions reached the platform as UniFFI commands.
    assert!(
        a.hooks.count(|c| matches!(
            c,
            Call::Radio(FfiRadioCommand::SetDiscovery {
                mode: FfiDiscoveryMode::Forming,
                ..
            }) | Call::Radio(FfiRadioCommand::SetDiscovery {
                mode: FfiDiscoveryMode::Stable,
                ..
            })
        )) > 0
    );
    a.engine.on_link_down(1, FfiLinkDownReason::PeerClosed);
    wait_until("link gone on A", || links_in_snapshot(&a) == Some(0));
    a.engine.stop();
    b.engine.stop();
}

#[test]
fn would_block_units_are_resent_first_after_writable() {
    let r = Rig::new(42);
    let refused = Arc::new(AtomicU32::new(0));
    let rf = refused.clone();
    r.hooks.set_send(move |_, _| {
        if rf.fetch_add(1, SeqCst) == 0 {
            FfiSendOutcome::WouldBlock
        } else {
            FfiSendOutcome::Accepted
        }
    });
    r.engine
        .start(FfiSessionStart::Lab { lab_code: 99 })
        .unwrap();
    r.engine.on_link_up(2, l2cap(FfiLinkRole::Initiator));
    wait_until("first send", || {
        r.hooks.count(|c| matches!(c, Call::Send(2, _))) == 1
    });
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(
        r.hooks.count(|c| matches!(c, Call::Send(2, _))),
        1,
        "nothing more is written while the link is blocked"
    );
    r.engine.on_link_writable(2);
    wait_until("resend", || {
        r.hooks.count(|c| matches!(c, Call::Send(2, _))) >= 2
    });
    let sends: Vec<Vec<u8>> = r
        .hooks
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Send(2, b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(sends[0], sends[1], "the refused unit goes out first");
    r.engine.stop();
}

#[test]
fn start_stop_cycles_release_everything() {
    let r = Rig::new(43);
    for i in 0..40u32 {
        r.engine
            .start(FfiSessionStart::Lab {
                lab_code: 1000 + i as u16,
            })
            .unwrap();
        assert_eq!(r.engine.status(), FfiRuntimeStatus::Running);
        assert!(r.rt.is_claimed() && r.rt.is_active());
        r.engine.on_link_up(i, l2cap(FfiLinkRole::Initiator));
        r.engine.on_link_data(i, vec![0, 0]);
        r.engine.stop();
        assert_eq!(r.engine.status(), FfiRuntimeStatus::Idle);
        assert!(!r.rt.is_claimed() && !r.rt.is_active());
    }
    assert_eq!(r.rt.stats().generation, 40);
    // One engine for all cycles: no thread or engine was leaked or rebuilt.
    assert_eq!(r.engine.runtime().shared().generation.load(SeqCst), 1);
    assert_eq!(r.engine.runtime().shared().dropped.load(SeqCst), 0);
    // Every start was matched by a discovery-off on stop.
    let offs = r.hooks.count(|c| {
        matches!(
            c,
            Call::Radio(FfiRadioCommand::SetDiscovery {
                mode: FfiDiscoveryMode::Off,
                ..
            })
        )
    });
    assert!(offs >= 40, "discovery off on every stop ({offs})");
}
