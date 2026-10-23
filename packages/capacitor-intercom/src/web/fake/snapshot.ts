// Snapshot of the fake engine (DESIGN §3.7). Lab mode has no group session, so
// `session` is null and Lab peers count as mesh links/neighbours, not as members.
import type { AudioRouteKind, Snapshot } from "@meshapp/engine-api";

export interface FakeSnapshotState {
  running: boolean;
  uiAttached: boolean;
  peers: number;
  /** Audio path in use: a Lab run with audio ≠ off, or an audio check. */
  render: boolean;
  capture: boolean;
  inputLevel: number;
}

/** BLE / Wi-Fi talker caps (Appendix A: S_MAX_BLE 2, S_MAX_WIFI 4). */
const CAP_BLE = 2;
const CAP_WIFI = 4;

export function fakeSnapshot(st: FakeSnapshotState): Snapshot {
  const route: AudioRouteKind = st.render || st.capture ? "builtinSpeaker" : "none";
  return {
    engine: st.running ? "running" : "idle",
    session: null,
    audio: {
      route,
      captureOk: st.capture,
      renderOk: st.render,
      inputLevel: st.capture ? Math.round(st.inputLevel * 100) / 100 : 0,
      txMutedNoHeadset: false,
    },
    talkers: [],
    members: [],
    mesh: {
      links: st.peers,
      neighbors: st.peers,
      islands: 0,
      reachable: 0,
      total: 0,
      capBle: CAP_BLE,
      capWifi: CAP_WIFI,
      profile: "ble",
    },
    device: {
      battery: null,
      thermal: "nominal",
      background: !st.uiAttached,
      bluetooth: "on",
    },
    latency: null,
  };
}
