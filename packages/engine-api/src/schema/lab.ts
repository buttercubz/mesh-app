// Runtime validators for the Lab types (crates/mesh-api/src/lab.rs). `audio`, `anchor`
// and `gps` of `LabParams` are mandatory, exactly as in Rust (DESIGN §3.7).
import type {
  LabLiveSample,
  LabParams,
  LabPercentiles,
  LabRate,
  LabRunInfo,
  LabSummary,
} from "../generated/index.js";
import {
  LAB_ANCHORS,
  LAB_AUDIOS,
  LAB_BENCHES,
  LAB_LINK_ROLES,
  LAB_PATTERNS,
  LAB_TRANSPORTS,
  LAB_VERDICTS,
} from "../generated/index.js";
import { arr, bool, i16, nullable, num, obj, oneOf, opt, str, u16, u32, u8 } from "../validate.js";

export const labBench = oneOf(LAB_BENCHES);
export const labAudio = oneOf(LAB_AUDIOS);
export const labAnchor = oneOf(LAB_ANCHORS);
export const labVerdict = oneOf(LAB_VERDICTS);

export const labParams = obj<LabParams>({
  audio: labAudio,
  anchor: labAnchor,
  gps: bool(),
  durationS: opt(u32()),
  pattern: opt(oneOf(LAB_PATTERNS)),
  streams: opt(u8()),
  payloadB: opt(u16()),
  intervalMs: opt(u16()),
  pingHz: opt(u8()),
  transport: opt(oneOf(LAB_TRANSPORTS)),
  role: opt(oneOf(LAB_LINK_ROLES)),
  headset: opt(bool()),
  mixable: opt(bool()),
  variant: opt(str()),
});

export const labLiveSample = obj<LabLiveSample>({
  links: u8(),
  peers: u8(),
  txKbps: num(),
  rxKbps: num(),
  loss: num(),
  rttP50Ms: nullable(num()),
  rttP95Ms: nullable(num()),
  rssi: nullable(i16()),
  distM: nullable(num()),
  stalls: u32(),
  anchorOk: bool(),
});

export const labRate = obj<LabRate>({ mean: num() });

export const labPercentiles = obj<LabPercentiles>({ p50: num(), p95: num(), p99: num() });

export const labSummary = obj<LabSummary>({
  durationS: num(),
  txKbps: labRate,
  loss: num(),
  owdMs: nullable(labPercentiles),
  rttMs: nullable(labPercentiles),
  stalls: u32(),
  verdict: labVerdict,
});

export const labRunInfo = obj<LabRunInfo>({
  runId: str(),
  bench: labBench,
  audio: labAudio,
  anchor: labAnchor,
  startedAt: str(),
  durationS: nullable(num()),
  verdict: nullable(labVerdict),
  files: arr(str()),
});
