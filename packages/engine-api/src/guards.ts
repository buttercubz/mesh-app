// Hand-written guards over the generated discriminants (DESIGN §11.1): components never
// trust raw plugin JSON, they go through these.
import type {
  CommandResult,
  Snapshot,
  UiCommand,
  UiEvent,
} from "./generated/index.js";
import { commandResult } from "./schema/result.js";
import { uiCommand } from "./schema/command.js";
import { uiEvent } from "./schema/event.js";
import { snapshot } from "./schema/snapshot.js";
import type { Validator } from "./validate.js";

/** A value that does not match the UI contract (`detail` names the JSON path). */
export class ContractError extends Error {
  readonly detail: string;

  constructor(what: string, detail: string) {
    super(`invalid ${what}: ${detail}`);
    this.name = "ContractError";
    this.detail = detail;
  }
}

/** Parses `input` when it is a JSON string; any other value is returned unchanged. */
function decode(what: string, input: unknown): unknown {
  if (typeof input !== "string") return input;
  try {
    return JSON.parse(input) as unknown;
  } catch (e) {
    // Malformed JSON is reported as a contract violation with the parser message.
    throw new ContractError(what, e instanceof Error ? e.message : String(e));
  }
}

function parseWith<T>(what: string, validator: Validator<T>, input: unknown): T {
  const value = decode(what, input);
  const error = validator.check(value, "$");
  if (error !== undefined) throw new ContractError(what, error);
  return value as T;
}

/** The first contract problem of `value` as a UiEvent, or `undefined` if it is valid. */
export const explainUiEvent = (value: unknown): string | undefined => uiEvent.check(value, "$");

export const isUiEvent = (value: unknown): value is UiEvent => uiEvent.check(value, "$") === undefined;
export const isUiCommand = (value: unknown): value is UiCommand =>
  uiCommand.check(value, "$") === undefined;
export const isCommandResult = (value: unknown): value is CommandResult =>
  commandResult.check(value, "$") === undefined;
export const isSnapshot = (value: unknown): value is Snapshot =>
  snapshot.check(value, "$") === undefined;

/** Validates a UiEvent (object or JSON text); throws {@link ContractError}. */
export const parseUiEvent = (input: unknown): UiEvent => parseWith("UiEvent", uiEvent, input);
/** Validates a UiCommand (object or JSON text); throws {@link ContractError}. */
export const parseUiCommand = (input: unknown): UiCommand =>
  parseWith("UiCommand", uiCommand, input);
/** Validates a CommandResult (object or JSON text); throws {@link ContractError}. */
export const parseCommandResult = (input: unknown): CommandResult =>
  parseWith("CommandResult", commandResult, input);
/** Validates a Snapshot (object or JSON text); throws {@link ContractError}. */
export const parseSnapshot = (input: unknown): Snapshot => parseWith("Snapshot", snapshot, input);

/** Narrows an event to one `type`. */
export const isEventOfType = <K extends UiEvent["type"]>(
  event: UiEvent,
  type: K,
): event is Extract<UiEvent, { type: K }> => event.type === type;

/** `true` for `{ ok: true, ... }` results. */
export const isOk = <R extends CommandResult>(result: R): result is Exclude<R, { ok: false }> =>
  result.ok;
