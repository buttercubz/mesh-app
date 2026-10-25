// Runtime validators for `Snapshot` (crates/mesh-api/src/snapshot.rs). Integer ranges
// follow the Rust field types; every struct is exact (serde `deny_unknown_fields`).
import type {
  AudioInfo,
  DeviceInfo,
  LatencyInfo,
  Member,
  MemberReady,
  MeshInfo,
  SessionInfo,
  Snapshot,
  Talker,
} from "../generated/index.js";
import {
  AUDIO_ROUTE_KINDS,
  BLUETOOTH_STATES,
  ENGINE_STATES,
  LADDER_LEVELS,
  LINK_KINDS,
  MEMBER_FLAGS,
  MEMBER_ROLES,
  MEMBER_STATUSES,
  MESH_PROFILES,
  PEER_PLATFORMS,
  RIDE_PHASES,
  TALK_MODES,
  TALK_STATES,
  THERMAL_STATES,
  VOICE_REACHES,
} from "../generated/index.js";
import { arr, bool, nullable, num, obj, oneOf, str, u16, u32, u8 } from "../validate.js";

export const audioRouteKind = oneOf(AUDIO_ROUTE_KINDS);

export const sessionInfo = obj<SessionInfo>({
  groupId: str(),
  groupName: str(),
  role: oneOf(MEMBER_ROLES),
  myNodeId: u16(),
  live: bool(),
  mode: oneOf(TALK_MODES),
  talk: oneOf(TALK_STATES),
  ride: oneOf(RIDE_PHASES),
  ladder: oneOf(LADDER_LEVELS),
  shareDistance: bool(),
  openChannel: nullable(u8()),
});

export const audioInfo = obj<AudioInfo>({
  route: audioRouteKind,
  captureOk: bool(),
  renderOk: bool(),
  inputLevel: num(),
  txMutedNoHeadset: bool(),
});

export const talker = obj<Talker>({
  nodeId: u16(),
  name: str(),
  hops: u8(),
  level: num(),
  sinceMs: u32(),
});

export const memberReady = obj<MemberReady>({
  live: bool(),
  mic: bool(),
  route: nullable(audioRouteKind),
});

export const member = obj<Member>({
  nodeId: u16(),
  name: str(),
  role: oneOf(MEMBER_ROLES),
  platform: oneOf(PEER_PLATFORMS),
  status: oneOf(MEMBER_STATUSES),
  hops: u8(),
  quality: u8(),
  linkKinds: arr(oneOf(LINK_KINDS)),
  hearsMe: bool(),
  voice: oneOf(VOICE_REACHES),
  lastHeardMs: u32(),
  battery: nullable(num()),
  ready: memberReady,
  flags: arr(oneOf(MEMBER_FLAGS)),
  distanceM: nullable(num()),
  bearingDeg: nullable(num()),
});

export const meshInfo = obj<MeshInfo>({
  links: u8(),
  neighbors: u8(),
  islands: u8(),
  reachable: u8(),
  total: u8(),
  capBle: u8(),
  capWifi: u8(),
  profile: oneOf(MESH_PROFILES),
});

export const deviceInfo = obj<DeviceInfo>({
  battery: nullable(num()),
  thermal: oneOf(THERMAL_STATES),
  background: bool(),
  bluetooth: oneOf(BLUETOOTH_STATES),
});

export const latencyInfo = obj<LatencyInfo>({
  p50Ms: num(),
  p95Ms: num(),
  byHops: arr(num()),
});

export const snapshot = obj<Snapshot>({
  engine: oneOf(ENGINE_STATES),
  session: nullable(sessionInfo),
  audio: audioInfo,
  talkers: arr(talker),
  members: arr(member),
  mesh: meshInfo,
  device: deviceInfo,
  latency: nullable(latencyInfo),
});
