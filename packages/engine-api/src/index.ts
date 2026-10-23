// @meshapp/engine-api (WP-03, DESIGN §3.7, §11.1): the ts-rs generated UI contract
// (types + constants from crates/mesh-api), runtime guards and the typed client.
export * from "./generated/index.js";
export { createEngineClient, rejectionCode } from "./client.js";
export type {
  EngineClient,
  EngineClientOptions,
  EnginePlugin,
  OkResultOf,
  PluginListenerHandle,
  ResultOf,
  Unsubscribe,
} from "./client.js";
export { GESTURE_COMMAND_TYPES, requiresGesture } from "./gesture.js";
export {
  ContractError,
  explainUiEvent,
  isCommandResult,
  isEventOfType,
  isOk,
  isSnapshot,
  isUiCommand,
  isUiEvent,
  parseCommandResult,
  parseSnapshot,
  parseUiCommand,
  parseUiEvent,
} from "./guards.js";
export type { ResultPayloads } from "./schema/result.js";
