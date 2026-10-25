// Contract fixtures (mesh-api C1 shapes), validated by @meshapp/engine-api guards in the
// tests that use them, so a contract change breaks the fixtures loudly.
import type { LabLiveSample, LabSummary, Snapshot } from "@meshapp/engine-api";

export const GROUP_ID = "0123456789abcdef0123456789abcdef";

export function snapshotFixture(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    engine: "running",
    session: {
      groupId: GROUP_ID,
      groupName: "Sunday ride",
      role: "leader",
      myNodeId: 7,
      live: true,
      mode: "vox",
      talk: "idle",
      ride: "riding",
      ladder: "L0",
      shareDistance: false,
      openChannel: null,
    },
    audio: { route: "hfpWideband", captureOk: true, renderOk: true, inputLevel: 0.2, txMutedNoHeadset: false },
    talkers: [{ nodeId: 9, name: "Ana", hops: 1, level: 0.6, sinceMs: 1200 }],
    members: [
      {
        nodeId: 9,
        name: "Ana",
        role: "member",
        platform: "ios",
        status: "direct",
        hops: 1,
        quality: 80,
        linkKinds: ["ble"],
        hearsMe: true,
        voice: "kpi",
        lastHeardMs: 200,
        battery: 64,
        ready: { live: true, mic: true, route: "hfpWideband" },
        flags: [],
        distanceM: null,
        bearingDeg: null,
      },
    ],
    mesh: { links: 1, neighbors: 1, islands: 1, reachable: 2, total: 2, capBle: 2, capWifi: 4, profile: "ble" },
    device: { battery: 80, thermal: "nominal", background: false, bluetooth: "on" },
    latency: null,
    ...overrides,
  };
}

export const labSampleFixture: LabLiveSample = {
  links: 1,
  peers: 1,
  txKbps: 32,
  rxKbps: 31.5,
  loss: 0.01,
  rttP50Ms: 40,
  rttP95Ms: 80,
  rssi: -60,
  distM: null,
  stalls: 0,
  anchorOk: true,
};

export const labSummaryFixture: LabSummary = {
  durationS: 60,
  txKbps: { mean: 32 },
  loss: 0.01,
  owdMs: null,
  rttMs: { p50: 40, p95: 80, p99: 120 },
  stalls: 0,
  verdict: "go",
};
