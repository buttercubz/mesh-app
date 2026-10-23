// `createEngineClient(plugin)`: the typed wrapper around the MeshIntercom plugin
// (DESIGN §11.1). Commands are validated, serialised and routed: every ★ command of
// `GESTURE_COMMANDS` goes through `plugin.gestureCommand`, everything else through
// `plugin.send`. Results and events are validated against the contract before any
// component sees them.
import type {
  CommandError,
  ErrorCode,
  UiCommand,
  UiEvent,
} from "./generated/index.js";
import { COMMAND_RESULT_KINDS, ERROR_CODES } from "./generated/index.js";
import { requiresGesture } from "./gesture.js";
import { ContractError, parseCommandResult, parseUiEvent } from "./guards.js";
import { uiCommand } from "./schema/command.js";
import type { ResultPayloads } from "./schema/result.js";
import { resultPayloads } from "./schema/result.js";

/** Listener handle returned by Capacitor's `addListener`. */
export interface PluginListenerHandle {
  remove(): Promise<void>;
}

/**
 * The part of `MeshIntercomPlugin` (@meshapp/capacitor-intercom, contract C6) the client
 * needs. Both command methods take and return JSON text.
 */
export interface EnginePlugin {
  /** Pass-through of non-★ commands; rejects ★ commands with `useGestureCommand`. */
  send(opts: { command: string }): Promise<{ result: string }>;
  /** ★ commands, called inside the user gesture; rejects with `notForeground`. */
  gestureCommand(opts: { command: string }): Promise<{ result: string }>;
  /** UiEvent stream (`{ json }` per event). */
  addListener(
    eventName: "event",
    listener: (event: { json: string }) => void,
  ): Promise<PluginListenerHandle>;
}

/** The successful payload type of command `C`. */
export type OkResultOf<C extends UiCommand> =
  ResultPayloads[(typeof COMMAND_RESULT_KINDS)[C["type"]]];

/** The result type of command `C`: its payload, or `{ ok: false, code }`. */
export type ResultOf<C extends UiCommand> = OkResultOf<C> | CommandError;

/** Removes an event subscription. */
export type Unsubscribe = () => Promise<void>;

export interface EngineClientOptions {
  /**
   * Called for every event the plugin delivers that breaks the contract; the event is
   * dropped. Default: none (the app wires its diagnostics logger here).
   */
  onContractError?: (error: ContractError, raw: string) => void;
}

export interface EngineClient {
  /**
   * Sends `command` and resolves with its typed result. Resolves `{ ok: false, code }`
   * for refusals (also when the plugin rejects with a contract `ErrorCode`, e.g.
   * `notForeground`); rejects with {@link ContractError} when the engine answers with
   * JSON that breaks the contract, and passes any other plugin failure through.
   *
   * The plugin method is called synchronously, so a ★ command issued from a tap
   * handler stays inside the user gesture.
   */
  command<C extends UiCommand>(command: C): Promise<ResultOf<C>>;
  /** Subscribes to every valid UiEvent. */
  onEvent(handler: (event: UiEvent) => void): Promise<Unsubscribe>;
  /** Subscribes to the events of one `type`. */
  on<K extends UiEvent["type"]>(
    type: K,
    handler: (event: Extract<UiEvent, { type: K }>) => void,
  ): Promise<Unsubscribe>;
}

const errorCodes = new Set<string>(ERROR_CODES);

const refusal = (code: ErrorCode): CommandError => ({ ok: false, code });

/** The contract error code carried by a plugin rejection (`code` or `message`), if any. */
export function rejectionCode(reason: unknown): ErrorCode | undefined {
  if (typeof reason !== "object" || reason === null) {
    return typeof reason === "string" && errorCodes.has(reason) ? (reason as ErrorCode) : undefined;
  }
  for (const key of ["code", "message"] as const) {
    const value = (reason as Record<string, unknown>)[key];
    if (typeof value === "string" && errorCodes.has(value)) return value as ErrorCode;
  }
  return undefined;
}

export function createEngineClient(
  plugin: EnginePlugin,
  options: EngineClientOptions = {},
): EngineClient {
  const command = <C extends UiCommand>(cmd: C): Promise<ResultOf<C>> => {
    if (uiCommand.check(cmd, "$") !== undefined) {
      return Promise.resolve(refusal("invalidCommand"));
    }
    const opts = { command: JSON.stringify(cmd) };
    // Synchronous call: no await before this line (user-gesture rule, DESIGN §3.2).
    const pending = requiresGesture(cmd) ? plugin.gestureCommand(opts) : plugin.send(opts);
    return pending.then(
      ({ result }) => {
        const parsed = parseCommandResult(result);
        if (!parsed.ok) return parsed;
        const kind = COMMAND_RESULT_KINDS[cmd.type];
        const error = resultPayloads[kind].check(parsed, "$");
        if (error !== undefined) {
          throw new ContractError(`${cmd.type} result (expected ${kind})`, error);
        }
        return parsed as OkResultOf<C>;
      },
      (reason: unknown) => {
        const code = rejectionCode(reason);
        if (code === undefined) throw reason;
        return refusal(code);
      },
    );
  };

  const onEvent = async (handler: (event: UiEvent) => void): Promise<Unsubscribe> => {
    const handle = await plugin.addListener("event", ({ json }) => {
      let event: UiEvent;
      try {
        event = parseUiEvent(json);
      } catch (e) {
        // Contract violations are reported and the event is dropped; anything else is a bug.
        if (!(e instanceof ContractError)) throw e;
        options.onContractError?.(e, json);
        return;
      }
      handler(event);
    });
    return () => handle.remove();
  };

  const on = <K extends UiEvent["type"]>(
    type: K,
    handler: (event: Extract<UiEvent, { type: K }>) => void,
  ): Promise<Unsubscribe> =>
    onEvent((event) => {
      if (event.type === type) handler(event as Extract<UiEvent, { type: K }>);
    });

  return { command, onEvent, on };
}
