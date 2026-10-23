//! Field lists of the typed [`DiagEvent`]s (the JSON names of §2.4).

use super::event::{DiagEvent, r32};
use crate::json::Json;

fn ids(v: &[u16]) -> Json {
    Json::Arr(v.iter().map(|&i| Json::from(i)).collect())
}

/// Lower-case hex of a ride key.
pub fn hex4(k: &[u8; 4]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(8);
    for b in k {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn fields(e: &DiagEvent) -> Vec<(&'static str, Json)> {
    use DiagEvent as E;
    match e {
        E::Session {
            ride_key,
            node,
            role,
        } => vec![
            ("rideKey", ride_key.as_ref().map(hex4).into()),
            ("node", (*node).into()),
            ("role", (*role).into()),
        ],
        E::RadioState { ble, aware } => vec![("ble", (*ble).into()), ("aware", (*aware).into())],
        E::DiscoveryMode {
            mode,
            scan,
            adv,
            duty,
        } => vec![
            ("mode", (*mode).into()),
            ("scan", (*scan).into()),
            ("adv", (*adv).into()),
            ("duty", duty.map_or(Json::Null, |d| r32(d, 3))),
        ],
        E::PeerSeen {
            disc,
            rssi,
            group_match,
            epoch_behind,
        } => vec![
            ("disc", (*disc).into()),
            ("rssi", (*rssi).into()),
            ("groupMatch", (*group_match).into()),
            ("epochBehind", (*epoch_behind).into()),
        ],
        E::ConnectTry { disc, via, attempt } => vec![
            ("disc", (*disc).into()),
            ("via", (*via).into()),
            ("attempt", (*attempt).into()),
        ],
        E::ConnectFail {
            disc,
            reason,
            attempt,
            backoff_s,
        } => vec![
            ("disc", (*disc).into()),
            ("reason", (*reason).into()),
            ("attempt", (*attempt).into()),
            ("backoffS", (*backoff_s).into()),
        ],
        E::Abort {
            disc,
            ignore_s,
            why,
        } => vec![
            ("disc", (*disc).into()),
            ("ignoreS", (*ignore_s).into()),
            ("why", (*why).into()),
        ],
        E::LinkUp {
            link,
            kind,
            role,
            mtu,
            psm,
        } => vec![
            ("link", (*link).into()),
            ("kind", kind.as_str().into()),
            ("role", role.map(|r| r.as_str()).into()),
            ("mtu", (*mtu).into()),
            ("psm", (*psm).into()),
        ],
        E::Handshake {
            link,
            peer,
            ok,
            ms,
            err,
        } => vec![
            ("link", (*link).into()),
            ("peer", (*peer).into()),
            ("ok", (*ok).into()),
            ("ms", (*ms).into()),
            ("err", (*err).into()),
        ],
        E::LinkDown {
            link,
            peer,
            reason,
            up_s,
            tx_b,
            rx_b,
        } => vec![
            ("link", (*link).into()),
            ("peer", (*peer).into()),
            ("reason", (*reason).into()),
            ("upS", (*up_s).into()),
            ("txB", (*tx_b).into()),
            ("rxB", (*rx_b).into()),
        ],
        E::LinkParams {
            link,
            prio,
            grant_kbps,
        } => vec![
            ("link", (*link).into()),
            ("prio", (*prio).into()),
            ("grantKbps", r32(*grant_kbps, 1)),
        ],
        E::WouldBlockBurst { link, count, ms } => vec![
            ("link", (*link).into()),
            ("count", (*count).into()),
            ("ms", (*ms).into()),
        ],
        E::ReconnectArmed { peer, until_s } => {
            vec![("peer", (*peer).into()), ("untilS", (*until_s).into())]
        }
        E::ClkOffset {
            link,
            peer,
            offset_us,
            rtt_us,
        } => vec![
            ("link", (*link).into()),
            ("peer", (*peer).into()),
            ("offsetUs", (*offset_us).into()),
            ("rttUs", (*rtt_us).into()),
        ],
        E::NbrChange {
            peer,
            q,
            good,
            congested,
            kind,
        } => vec![
            ("peer", (*peer).into()),
            ("q", r32(*q, 3)),
            ("good", (*good).into()),
            ("congested", (*congested).into()),
            ("kind", kind.as_str().into()),
        ],
        E::Reach {
            nodes,
            added,
            removed,
        } => vec![
            ("nodes", (*nodes).into()),
            ("added", ids(added)),
            ("removed", ids(removed)),
        ],
        E::Partition {
            islands,
            mine,
            others,
        } => vec![
            ("islands", (*islands).into()),
            ("mine", (*mine).into()),
            ("others", ids(others)),
        ],
        E::Heard {
            peer,
            last_heard_s,
            lost,
        } => vec![
            ("peer", (*peer).into()),
            ("lastHeardS", (*last_heard_s).into()),
            ("lost", (*lost).into()),
        ],
        E::Floor {
            kind: _,
            origin,
            reason,
            streams,
        } => vec![
            ("origin", (*origin).into()),
            ("reason", (*reason).into()),
            ("streams", (*streams).into()),
        ],
        E::Rekey { epoch } => vec![("epoch", (*epoch).into())],
        E::MemberRemoved { peer, epoch } | E::LeaderChange { peer, epoch } => {
            vec![("peer", (*peer).into()), ("epoch", (*epoch).into())]
        }
        E::BudgetCap { cap_kbps, level } => {
            vec![("capKbps", r32(*cap_kbps, 1)), ("level", (*level).into())]
        }
        E::CaptureStart { why } | E::CaptureStop { why } => vec![("why", (*why).into())],
        E::CaptureLost { reason, detail } => {
            vec![("reason", (*reason).into()), ("detail", (*detail).into())]
        }
        E::MicState { state, cause } => {
            vec![("state", (*state).into()), ("cause", (*cause).into())]
        }
        E::Route {
            route,
            prev,
            reason,
            hfp_wb,
        } => vec![
            ("route", (*route).into()),
            ("prev", (*prev).into()),
            ("reason", (*reason).into()),
            ("hfpWb", (*hfp_wb).into()),
        ],
        E::JbRetarget {
            origin,
            from_ms,
            to_ms,
            why,
        } => vec![
            ("origin", (*origin).into()),
            ("fromMs", (*from_ms).into()),
            ("toMs", (*to_ms).into()),
            ("why", (*why).into()),
        ],
        E::Vad { on, p } => vec![("on", (*on).into()), ("p", r32(*p, 2))],
        E::Ptt { on } => vec![("on", (*on).into())],
        E::PromptPlay { key } => vec![("key", (*key).into())],
        E::Glitch { kind, n, ms } => vec![
            ("kind", kind.as_str().into()),
            ("n", (*n).into()),
            ("ms", (*ms).into()),
        ],
        E::EngineStart { mode } => vec![("mode", (*mode).into())],
        E::EngineStop => Vec::new(),
        E::EngineCrash { at } => vec![("at", (*at).into())],
        E::EffectOverflow { dropped } => vec![("dropped", (*dropped).into())],
        E::Logs {
            debug,
            info,
            warn,
            error,
        } => vec![
            ("debug", (*debug).into()),
            ("info", (*info).into()),
            ("warn", (*warn).into()),
            ("error", (*error).into()),
        ],
        E::ClkAnchor { wall_ms, mono_us } => {
            vec![("wallMs", (*wall_ms).into()), ("monoUs", (*mono_us).into())]
        }
    }
}
