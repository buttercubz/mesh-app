// Runtime validator for `UiEvent` (crates/mesh-api/src/event.rs). Events carry i18n keys
// plus params only; params values are strings or numbers (`ParamValue`).
import type { ParamValue, UiEvent } from "../generated/index.js";
import {
  AUDIO_CHECK_PHASES,
  ERROR_CODES,
  GATE_STATES,
  NOTICE_KEYS,
  TALK_DENIED_REASONS,
} from "../generated/index.js";
import type { Shape, Validator } from "../validate.js";
import { anyOf, arr, lit, num, obj, oneOf, record, str, tagged, u32 } from "../validate.js";
import { labLiveSample, labSummary } from "./lab.js";
import { audioRouteKind, snapshot } from "./snapshot.js";

type Ev<K extends UiEvent["type"]> = Extract<UiEvent, { type: K }>;

const ev = <K extends UiEvent["type"]>(
  type: K,
  fields: Omit<Shape<Ev<K>>, "type">,
): Validator<Ev<K>> => obj<Ev<K>>({ type: lit(type), ...fields } as unknown as Shape<Ev<K>>);

export const paramValue: Validator<ParamValue> = anyOf<ParamValue>("ParamValue", [str(), num()]);

/** `Record<string, string | number>` i18n interpolation params. */
export const params = record(paramValue) as Validator<{ [key in string]: ParamValue }>;

export const uiEvent: Validator<UiEvent> = tagged<UiEvent, "type">("type", {
  snapshot: ev("snapshot", { s: snapshot }),
  talkDenied: ev("talkDenied", { reason: oneOf(TALK_DENIED_REASONS) }),
  notice: ev("notice", { key: oneOf(NOTICE_KEYS), params }),
  routeChanged: ev("routeChanged", { route: audioRouteKind }),
  audioCheckSample: ev("audioCheckSample", {
    level: num(),
    vadP: num(),
    gate: oneOf(GATE_STATES),
    route: audioRouteKind,
    inputRate: u32(),
    phase: oneOf(AUDIO_CHECK_PHASES),
  }),
  labProgress: ev("labProgress", { runId: str(), elapsedS: num(), sample: labLiveSample }),
  labFinished: ev("labFinished", { runId: str(), summary: labSummary, files: arr(str()) }),
  error: ev("error", { code: oneOf(ERROR_CODES), params }),
});
