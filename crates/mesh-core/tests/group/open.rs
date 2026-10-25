//! Open channels: riders on the same public channel find, admit and hear each other
//! without any invite; another channel never joins; nothing is persisted; mute and
//! block stay local.

use mesh_api::{CommandResult, ErrorCode, MemberFlag, UiCommand};
use mesh_core::Platform;
use mesh_core::effects::Effect;

use crate::harness::World;
use crate::support::*;

fn join_open(w: &mut World, i: usize, channel: u8, name: &str) -> String {
    gid(&w.command(
        i,
        UiCommand::JoinOpenChannel {
            channel,
            display_name: name.into(),
        },
    ))
}

/// Advertising rounds until `done` holds (at most 30 s of simulated time).
fn discover(w: &mut World, done: impl Fn(&World) -> bool) {
    for _ in 0..60 {
        w.advertise();
        w.run_ms(500);
        if done(w) {
            return;
        }
    }
}

fn member_names(w: &World, i: usize) -> Vec<String> {
    let mut v: Vec<String> = snapshot(w, i)
        .map(|s| s.members.into_iter().map(|m| m.name).collect())
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn three_riders_on_channel_3_meet_without_an_invite_and_channel_4_stays_apart() {
    let mut w = World::new(4, Platform::Ios, 91);
    let names = ["Ana", "Ben", "Caro", "Dani"];
    let channels = [3u8, 3, 3, 4];
    let mut ids = Vec::new();
    for (i, (c, n)) in channels.iter().zip(names).enumerate() {
        ids.push(join_open(&mut w, i, *c, n));
    }
    // Same channel → same group id (derived), the other channel differs.
    assert_eq!(ids[0], ids[1]);
    assert_eq!(ids[0], ids[2]);
    assert_ne!(ids[0], ids[3]);
    for (i, id) in ids.iter().enumerate() {
        start(&mut w, i, id);
    }
    w.in_range = vec![(0, 1), (0, 2), (1, 2), (0, 3), (1, 3), (2, 3)];
    discover(&mut w, |w| (0..3).all(|i| up_peers(w, i).len() == 2));
    for i in 0..3 {
        assert_eq!(
            up_peers(&w, i).len(),
            2,
            "node {i} links both channel-3 peers"
        );
    }
    assert!(
        up_peers(&w, 3).is_empty(),
        "channel 4 never joins channel 3"
    );
    w.run_ms(3_000);
    w.command(0, UiCommand::RequestSnapshot {});
    w.collect(0);
    let s = snapshot(&w, 0).expect("snapshot");
    let info = s.session.as_ref().expect("session");
    assert_eq!(info.open_channel, Some(3));
    assert_eq!(
        info.ride,
        mesh_api::RidePhase::Riding,
        "no leader to start it"
    );
    assert_eq!(info.role, mesh_api::MemberRole::Member, "nobody leads");
    assert_eq!(info.mode, mesh_api::TalkMode::Ptt, "PTT by default");
    let names0 = member_names(&w, 0);
    assert!(names0.contains(&"Ben".to_owned()), "{names0:?}");
    assert!(names0.contains(&"Caro".to_owned()), "{names0:?}");
    assert!(!names0.contains(&"Dani".to_owned()));
    // Leader operations do not exist on an open channel.
    let ben = s
        .members
        .iter()
        .find(|m| m.name == "Ben")
        .map(|m| m.node_id)
        .expect("Ben listed");
    assert!(matches!(
        w.command(0, UiCommand::RemoveMember { node_id: ben }),
        CommandResult::Error(e) if e.code == ErrorCode::NotLeader
    ));
    // Nothing about the channel is persisted, nor listed.
    for i in 0..4 {
        assert!(
            !w.nodes[i]
                .effects
                .iter()
                .any(|e| matches!(e, Effect::Persist { key, .. } if key.starts_with("group"))),
            "node {i} persisted an open channel"
        );
        match w.command(i, UiCommand::ListGroups {}) {
            CommandResult::Groups(g) => assert!(g.groups.is_empty()),
            other => panic!("{other:?}"),
        }
    }
    // Local mute and block: flags in my snapshot only; a blocked rider is refused as a
    // direct neighbor.
    assert!(
        w.command(
            0,
            UiCommand::SetMemberMuted {
                node_id: ben,
                muted: true
            }
        )
        .is_ok()
    );
    w.run_ms(1_500);
    let m = snapshot(&w, 0).expect("snapshot");
    let flags = &m
        .members
        .iter()
        .find(|m| m.node_id == ben)
        .expect("Ben")
        .flags;
    assert!(flags.contains(&MemberFlag::Muted));
    assert!(
        w.command(
            0,
            UiCommand::SetMemberBlocked {
                node_id: ben,
                blocked: true
            }
        )
        .is_ok()
    );
    w.run_ms(500);
    discover(&mut w, |_| false);
    assert!(
        !up_peers(&w, 0).contains(&ben),
        "blocked rider stays unlinked"
    );
    assert!(up_peers(&w, 1).len() == 1, "Ben still talks to Caro");
    // Leaving drops the channel; it cannot be restarted without joining again.
    stop(&mut w, 0);
    assert!(matches!(
        w.command(
            0,
            UiCommand::SetMemberMuted {
                node_id: ben,
                muted: false
            }
        ),
        CommandResult::Error(_)
    ));
}

#[test]
fn channel_out_of_range_is_refused() {
    let mut w = World::new(1, Platform::Android, 92);
    for c in [0u8, 10] {
        assert!(matches!(
            w.command(0, UiCommand::JoinOpenChannel { channel: c, display_name: String::new() }),
            CommandResult::Error(e) if e.code == ErrorCode::InvalidParams
        ));
    }
}
