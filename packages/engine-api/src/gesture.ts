// ★ capture-gated commands (DESIGN §3.2, §3.7): evaluated from the generated
// `GESTURE_COMMANDS` table, so Rust (`UiCommand::requires_gesture`), the client, the
// plugin web gating and the app lint rule share one definition.
import type { UiCommand } from "./generated/index.js";
import { GESTURE_COMMANDS } from "./generated/index.js";

type Rule = (typeof GESTURE_COMMANDS)[number];

/** The command types that can be ★ (some only under a condition). */
export const GESTURE_COMMAND_TYPES: readonly UiCommand["type"][] = [
  ...new Set(GESTURE_COMMANDS.map((r) => r.type)),
];

function field(root: unknown, path: readonly string[]): unknown {
  let cur: unknown = root;
  for (const key of path) {
    if (typeof cur !== "object" || cur === null || !Object.hasOwn(cur, key)) return undefined;
    cur = (cur as Record<string, unknown>)[key];
  }
  return cur;
}

function matches(rule: Rule, command: Readonly<Record<string, unknown>>): boolean {
  if (rule.type !== command["type"]) return false;
  const when = rule.when;
  if (when === null) return true;
  const value = field(command, when.path);
  if (typeof value !== "string") return false;
  return when.op === "equals" ? value === when.value : value !== when.value;
}

/**
 * `true` when `command` must go through the plugin's `gestureCommand` (inside the user
 * gesture): `goLive`, `resumeMic`, `labStart` with `params.audio` ≠ `off` and
 * `audioCheck` with `op: "start"`. `send` rejects these with `useGestureCommand`.
 */
export function requiresGesture(command: UiCommand): boolean {
  const record = command as unknown as Readonly<Record<string, unknown>>;
  return GESTURE_COMMANDS.some((rule) => matches(rule, record));
}
