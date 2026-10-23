// Per-bench run configuration (WP-21, DESIGN §12.1, §12.5): the draft the config sheet
// edits and its translation into `labStart` parameters. The audio path and the
// keep-alive anchor are mandatory: a draft without either (or with an anchor the bench
// does not allow) never becomes a LabParams, so a run cannot start without them.
import type { LabAnchor, LabAudio, LabBench, LabLinkRole, LabParams, LabTransport } from "@meshapp/engine-api";
import { LAB_ANCHORS, LAB_AUDIOS, LAB_LINK_ROLES, LAB_TRANSPORTS } from "@meshapp/engine-api";

import type { BenchSpec } from "./catalog";
import { PARAM_AXES, audioFor, patternAxis } from "./catalog";

/** Longest run the config accepts (24 h; the T2 long run is 8 h+). */
export const MAX_DURATION_S = 86_400;
/** `variant` label limit (UTF-8 bytes), as the engine checks it. */
export const VARIANT_MAX_BYTES = 64;

export interface BenchDraft {
  readonly bench: LabBench;
  /** Selected value per variant axis. */
  readonly axes: Readonly<Record<string, string>>;
  /** `null` = not chosen (the sheet pre-selects the bench default). */
  readonly audio: LabAudio | null;
  readonly anchor: LabAnchor | null;
  /** Per-run GPS consent (off until the user opts in). */
  readonly gps: boolean;
  readonly durationS: number;
  /** Free note (helmet model, mount…), appended to the variant label. */
  readonly note: string;
}

export type DraftProblem =
  | "audioMissing"
  | "anchorMissing"
  | "anchorNotAllowed"
  | "headsetNeedsFull"
  | "durationInvalid"
  | "variantTooLong";

export type DraftCheck = { ok: true; params: LabParams } | { ok: false; problem: DraftProblem };

const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length;

const headsetOn = (draft: Pick<BenchDraft, "axes">): boolean => {
  const v = draft.axes["headset"];
  return v !== undefined && v !== "off";
};

/** Run length implied by the selected traffic pattern, else the bench default. */
function durationFor(spec: BenchSpec, axes: Readonly<Record<string, string>>): number {
  const pattern = axes["pattern"];
  return (pattern === undefined ? undefined : patternAxis(pattern)?.durationS) ?? spec.defaults.durationS;
}

/** The bench's default draft: first value of every axis, catalog audio path and anchor. */
export function defaultDraft(spec: BenchSpec): BenchDraft {
  const axes: Record<string, string> = {};
  for (const axis of spec.axes) {
    const first = axis.values[0];
    if (first !== undefined) axes[axis.key] = axis.key === "anchor" ? spec.anchor : first;
  }
  return {
    bench: spec.bench,
    axes,
    audio: audioFor(spec, headsetOn({ axes })),
    anchor: spec.anchor,
    gps: false,
    durationS: durationFor(spec, axes),
    note: "",
  };
}

/** Selects one axis value; headset, anchor and pattern also update their linked fields. */
export function withAxis(spec: BenchSpec, draft: BenchDraft, key: string, value: string): BenchDraft {
  const axis = spec.axes.find((a) => a.key === key);
  if (axis === undefined || !axis.values.includes(value)) return draft;
  const axes = { ...draft.axes, [key]: value };
  let next: BenchDraft = { ...draft, axes };
  if (key === "headset") next = { ...next, audio: audioFor(spec, headsetOn(next)) };
  if (key === "anchor" && (LAB_ANCHORS as readonly string[]).includes(value)) {
    next = { ...next, anchor: value as LabAnchor };
  }
  if (key === "pattern") next = { ...next, durationS: durationFor(spec, axes) };
  return next;
}

/** Selects the keep-alive anchor (keeps an `anchor` variant axis in sync). */
export function withAnchor(spec: BenchSpec, draft: BenchDraft, anchor: LabAnchor | null): BenchDraft {
  const hasAxis = spec.axes.some((a) => a.key === "anchor" && anchor !== null && a.values.includes(anchor));
  return {
    ...draft,
    anchor,
    axes: hasAxis && anchor !== null ? { ...draft.axes, anchor } : draft.axes,
  };
}

/** Free-text label of the variant: the non-parameter axes plus the note. */
export function variantLabel(spec: BenchSpec, draft: BenchDraft): string | null {
  const parts = spec.axes
    .filter((a) => !PARAM_AXES.has(a.key))
    .map((a) => `${a.key}:${draft.axes[a.key] ?? ""}`);
  const note = draft.note.trim().replace(/\s+/g, " ");
  if (note !== "") parts.push(note);
  return parts.length === 0 ? null : parts.join(";");
}

/**
 * Validates a draft and builds the `labStart` parameters. Fields the draft does not set
 * are left out so the engine applies the WP-05 bench defaults.
 */
export function toLabParams(spec: BenchSpec, draft: BenchDraft): DraftCheck {
  if (draft.audio === null || !(LAB_AUDIOS as readonly string[]).includes(draft.audio)) {
    return { ok: false, problem: "audioMissing" };
  }
  if (draft.anchor === null || !(LAB_ANCHORS as readonly string[]).includes(draft.anchor)) {
    return { ok: false, problem: "anchorMissing" };
  }
  if (!spec.anchors.includes(draft.anchor)) return { ok: false, problem: "anchorNotAllowed" };
  const headset = headsetOn(draft);
  if (headset && draft.audio !== "full") return { ok: false, problem: "headsetNeedsFull" };
  if (!Number.isInteger(draft.durationS) || draft.durationS < 1 || draft.durationS > MAX_DURATION_S) {
    return { ok: false, problem: "durationInvalid" };
  }
  const variant = variantLabel(spec, draft);
  if (variant !== null && utf8Bytes(variant) > VARIANT_MAX_BYTES) return { ok: false, problem: "variantTooLong" };

  const params: LabParams = { audio: draft.audio, anchor: draft.anchor, gps: draft.gps, durationS: draft.durationS };
  const pattern = draft.axes["pattern"];
  const traffic = pattern === undefined ? undefined : patternAxis(pattern);
  if (traffic !== undefined) {
    params.pattern = traffic.pattern;
    if (traffic.streams > 0) params.streams = traffic.streams;
  }
  const transport = draft.axes["transport"];
  if (transport !== undefined && (LAB_TRANSPORTS as readonly string[]).includes(transport)) {
    params.transport = transport as LabTransport;
  }
  const role = draft.axes["role"];
  if (role !== undefined && (LAB_LINK_ROLES as readonly string[]).includes(role)) params.role = role as LabLinkRole;
  if (draft.axes["headset"] !== undefined) params.headset = headset;
  const session = draft.axes["session"];
  if (session !== undefined) params.mixable = session === "mixable";
  if (variant !== null) params.variant = variant;
  return { ok: true, params };
}

/** `true` when the run uses location (the anchor or the GPS consent): ask in the foreground. */
export function needsLocation(draft: Pick<BenchDraft, "anchor" | "gps">): boolean {
  return draft.gps || draft.anchor === "audioLocation";
}
