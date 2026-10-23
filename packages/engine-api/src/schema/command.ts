// Runtime validator for `UiCommand` (crates/mesh-api/src/command.rs). The variant table
// is keyed by every generated command type, so a new Rust variant fails `tsc` here.
import type { UiCommand } from "../generated/index.js";
import {
  AUDIO_CHECK_MODES,
  LANGS,
  PROMPT_KEYS,
  PTT_STYLES,
  RIDE_OPS,
  ROUTE_PREFERENCES,
  TALK_MODES,
  VEHICLES,
  VOX_SENSITIVITIES,
} from "../generated/index.js";
import type { Shape, Validator } from "../validate.js";
import { bool, lit, nullable, obj, oneOf, record, str, tagged, u16, u32 } from "../validate.js";
import { labBench, labParams } from "./lab.js";

type Cmd<K extends UiCommand["type"]> = Extract<UiCommand, { type: K }>;

/** An exact command object `{ type: K, ...fields }`. */
const cmd = <K extends UiCommand["type"]>(
  type: K,
  fields: Omit<Shape<Cmd<K>>, "type">,
): Validator<Cmd<K>> => obj<Cmd<K>>({ type: lit(type), ...fields } as unknown as Shape<Cmd<K>>);

const vehicle = oneOf(VEHICLES);

/** `audioCheck` carries a second discriminant, `op` (serde internally tagged newtype). */
type AudioCheck = Cmd<"audioCheck">;
const audioCheck = tagged<AudioCheck, "op">("op", {
  start: obj<Extract<AudioCheck, { op: "start" }>>({
    type: lit("audioCheck"),
    op: lit("start"),
    mode: oneOf(AUDIO_CHECK_MODES),
  }),
  stop: obj<Extract<AudioCheck, { op: "stop" }>>({ type: lit("audioCheck"), op: lit("stop") }),
});

export const uiCommand: Validator<UiCommand> = tagged<UiCommand, "type">("type", {
  createGroup: cmd("createGroup", { name: str(), vehicle }),
  parseInvite: cmd("parseInvite", { url: str() }),
  joinGroup: cmd("joinGroup", { url: str(), displayName: str() }),
  listGroups: cmd("listGroups", {}),
  forgetGroup: cmd("forgetGroup", { groupId: str() }),
  getInvite: cmd("getInvite", { groupId: str() }),
  goLive: cmd("goLive", {}),
  resumeMic: cmd("resumeMic", {}),
  audioCheck,
  setTalkMode: cmd("setTalkMode", { mode: oneOf(TALK_MODES) }),
  ptt: cmd("ptt", { down: bool() }),
  setPttStyle: cmd("setPttStyle", { style: oneOf(PTT_STYLES) }),
  setVoxSensitivity: cmd("setVoxSensitivity", { level: oneOf(VOX_SENSITIVITIES) }),
  rideControl: cmd("rideControl", { op: oneOf(RIDE_OPS) }),
  setDisplayName: cmd("setDisplayName", { name: str() }),
  setVehicle: cmd("setVehicle", { vehicle }),
  setRoutePreference: cmd("setRoutePreference", { pref: oneOf(ROUTE_PREFERENCES) }),
  setShareDistance: cmd("setShareDistance", { enabled: bool() }),
  setPrompts: cmd("setPrompts", {
    lang: oneOf(LANGS),
    enabled: bool(),
    templates: record(str(), oneOf(PROMPT_KEYS)),
  }),
  setBuiltinMicTx: cmd("setBuiltinMicTx", { allowed: bool() }),
  removeMember: cmd("removeMember", { nodeId: u16() }),
  lockGroup: cmd("lockGroup", {}),
  closeGroup: cmd("closeGroup", {}),
  previewAccessLoss: cmd("previewAccessLoss", {}),
  transferLeader: cmd("transferLeader", { nodeId: u16() }),
  setInviteExpiry: cmd("setInviteExpiry", { hours: nullable(u32()) }),
  requestSnapshot: cmd("requestSnapshot", {}),
  labStart: cmd("labStart", { bench: labBench, params: labParams }),
  labStop: cmd("labStop", {}),
  labMark: cmd("labMark", { label: str() }),
  labListRuns: cmd("labListRuns", {}),
  labDeleteRun: cmd("labDeleteRun", { runId: str() }),
});
