use mesh_wire::mesh::{MeshHeader, MeshPacket, PacketType};

use super::*;

fn voice(origin: u16, seq: u16, at: Instant, age_us: u64) -> QueuedPacket {
    QueuedPacket {
        header: MeshHeader::new(PacketType::Voice, origin, 1, seq),
        payload: vec![0u8; 84],
        enqueued_at: at,
        base_age_us: age_us,
        burst: Some(BurstKey::new(origin, 1, 1)),
    }
}

fn ctrl(ptype: PacketType, seq: u16, at: Instant) -> QueuedPacket {
    QueuedPacket {
        header: MeshHeader::new(ptype, 1, 1, seq),
        payload: vec![0u8; 14],
        enqueued_at: at,
        base_age_us: 0,
        burst: None,
    }
}

fn table_with(origins: &[(u16, u64)], now: Instant) -> FloorTable {
    let mut t = FloorTable::new(1, 0, true);
    for &(o, started_ms_ago) in origins {
        t.on_voice(
            now,
            BurstKey::new(o, 1, 1),
            false,
            (started_ms_ago / 10) as u16,
            0,
        );
    }
    t
}

#[test]
fn priorities_by_type() {
    let t = Instant::ZERO;
    assert_eq!(
        Priority::of_mesh(&ctrl(PacketType::Talk, 0, t).header),
        Priority::P0
    );
    assert_eq!(
        Priority::of_mesh(&ctrl(PacketType::FloorReject, 0, t).header),
        Priority::P0
    );
    assert_eq!(Priority::of_mesh(&voice(1, 0, t, 0).header), Priority::P1);
    assert_eq!(
        Priority::of_mesh(&ctrl(PacketType::Presence, 0, t).header),
        Priority::P2
    );
    assert_eq!(
        Priority::of_mesh(&ctrl(PacketType::Position, 0, t).header),
        Priority::P2
    );
    assert_eq!(
        Priority::of_mesh(&ctrl(PacketType::MeshProbe, 0, t).header),
        Priority::P3
    );
}

#[test]
fn strict_priority_coalescing_and_age_rewrite() {
    let now = Instant::from_secs(1);
    let table = table_with(&[(1, 0)], now);
    let mut q = LinkQueue::new(150_000);
    q.push(
        Priority::P2,
        QueueItem::Mesh(ctrl(PacketType::Presence, 1, now)),
        60_000,
        &table,
    );
    q.push(
        Priority::P1,
        QueueItem::Mesh(voice(1, 2, now, 10_000)),
        60_000,
        &table,
    );
    q.push(
        Priority::P0,
        QueueItem::Record {
            rtype: 0x03,
            body: vec![0; 8],
        },
        60_000,
        &table,
    );
    let mut dropped = Vec::new();
    let later = now.plus_ms(20);
    let out = q.pop_frame(later, 1_000, 60_000, &mut dropped);
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].rtype, 0x03);
    assert_eq!(out[1].mesh.map(|h| h.ptype), Some(PacketType::Voice));
    // Age rewritten at dequeue: 10 ms + 20 ms residence = 30 ms = 15 units.
    let pkt = MeshPacket::decode(&out[1].body).expect("decode");
    assert_eq!(pkt.header.age, 15);
    assert!(q.is_empty());
    // A small budget only takes what fits.
    q.push(
        Priority::P1,
        QueueItem::Mesh(voice(1, 3, later, 0)),
        60_000,
        &table,
    );
    q.push(
        Priority::P1,
        QueueItem::Mesh(voice(2, 3, later, 0)),
        60_000,
        &table,
    );
    let out = q.pop_frame(later, 100, 60_000, &mut dropped);
    assert_eq!(out.len(), 1);
    assert_eq!(q.len(), 1);
    assert_eq!(q.head_len(), Some(95));
}

#[test]
fn per_origin_limit_and_deadline() {
    let now = Instant::from_secs(1);
    let table = table_with(&[(1, 0)], now);
    let mut q = LinkQueue::new(150_000);
    let mut ev = Vec::new();
    for s in 0..3 {
        ev.extend(q.push(
            Priority::P1,
            QueueItem::Mesh(voice(1, s, now, 0)),
            1_000_000,
            &table,
        ));
    }
    assert_eq!(ev, vec![ShedEvent::Frame(Some(BurstKey::new(1, 1, 1)))]);
    assert_eq!(q.len(), 2);
    let mut dropped = Vec::new();
    // Past the 150 ms deadline both frames are dropped at dequeue.
    let out = q.pop_frame(now.plus_ms(151), 1_000, 1_000_000, &mut dropped);
    assert!(out.is_empty());
    assert_eq!(dropped.len(), 2);
}

#[test]
fn backlog_sheds_lowest_ranked_burst_entirely() {
    let now = Instant::from_secs(10);
    // Origin 1 started 5 s ago, origin 2 1 s ago, origin 3 now (lowest rank).
    let table = table_with(&[(1, 5_000), (2, 1_000), (3, 0)], now);
    let mut q = LinkQueue::new(150_000);
    // 60 kbps × 60 ms = 450 B of voice backlog: 4 records of 95 B fit, the 5th does not.
    let mut ev = Vec::new();
    for (o, s) in [(3u16, 0u16), (1, 0), (2, 0), (3, 1), (1, 1)] {
        ev.extend(q.push(
            Priority::P1,
            QueueItem::Mesh(voice(o, s, now, 0)),
            60_000,
            &table,
        ));
    }
    assert_eq!(ev, vec![ShedEvent::Burst(BurstKey::new(3, 1, 1))]);
    assert_eq!(q.len(), 3);
    assert!(q.voice_bytes() <= 450);
}

#[test]
fn p2_limited_while_voice_waits_and_p3_last() {
    let now = Instant::from_secs(2);
    let table = table_with(&[(1, 0)], now);
    let mut q = LinkQueue::new(150_000);
    let mut dropped = Vec::new();
    // The P2 bucket is full (1100 B) after 2 s; the first frame uses 800 B of it.
    for s in 0..60 {
        q.push(
            Priority::P2,
            QueueItem::Mesh(ctrl(PacketType::Presence, s, now)),
            60_000,
            &table,
        );
    }
    q.push(
        Priority::P3,
        QueueItem::Mesh(ctrl(PacketType::MeshProbe, 0, now)),
        60_000,
        &table,
    );
    // With no voice waiting P2 flows freely (budget-limited only).
    let out = q.pop_frame(now, 10_000, 60_000, &mut dropped);
    assert_eq!(out.len(), 32);
    // Once the link carries voice, P2 is held to its 10 % share (300 B of tokens left).
    for s in 0..2 {
        q.push(
            Priority::P1,
            QueueItem::Mesh(voice(1, s, now, 0)),
            60_000,
            &table,
        );
    }
    let out = q.pop_frame(now, 10_000, 60_000, &mut dropped);
    let p2 = out
        .iter()
        .filter(|r| r.mesh.map(|h| h.ptype) == Some(PacketType::Presence))
        .count();
    assert_eq!(out.len() - p2, 2);
    assert_eq!(p2, 12);
    // P3 never jumps ahead of rate-limited P2.
    assert!(
        out.iter()
            .all(|r| r.mesh.map(|h| h.ptype) != Some(PacketType::MeshProbe))
    );
    // P3 only once everything above is drained.
    let mut later = now;
    let mut saw_probe = false;
    for _ in 0..200 {
        later = later.plus_ms(100);
        let out = q.pop_frame(later, 10_000, 60_000, &mut dropped);
        if out
            .iter()
            .any(|r| r.mesh.map(|h| h.ptype) == Some(PacketType::MeshProbe))
        {
            saw_probe = true;
            break;
        }
    }
    assert!(saw_probe);
    assert!(q.is_empty());
}
