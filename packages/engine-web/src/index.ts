// @meshapp/engine-web (WP-13, DESIGN §11.2): the browser simulator engine.
//
// - `createSimConvoy(opts)` loads mesh_wasm.wasm (mesh-core + mesh-sim, SimCodec, raw
//   `extern "C"` JSON ABI) and returns a convoy: `command(json)`, `onEvent(cb)`,
//   `start()` / `stop()` (the 20 ms clock), `labFiles()`.
// - `createWebEngineBackend(opts)` wraps a convoy as the MeshIntercom web backend.
// - `scenarioFromQuery(location.search)` reads `?sim=chain5&loss=0.1&roam=1&audio=1`.
export { ABI_EXPORTS, MeshWasm, MeshWasmError } from "./abi.js";
export type { AudioContextLike } from "./audio.js";
export { TalkerNoisePlayer, audibleTalkers } from "./audio.js";
export type { BackendFile, PttStamp, StartEngineOptions, WebEngineBackendOptions } from "./backend.js";
export { EngineWebError, WebEngineBackend, createWebEngineBackend } from "./backend.js";
export type { EventListener, SimConvoyOptions, Timers } from "./convoy.js";
export { MAX_STEP_MS, SimConvoy, TICK_MS, createSimConvoy } from "./convoy.js";
export type { DiagDevice, DiagFields, DiagState, MarkLabel } from "./diag-files.js";
export {
  MARK_LABELS,
  RIDELOG_MIME,
  RideLogRecorder,
  groupRideKey,
  labRideKey,
  sessionIdFor,
} from "./diag-files.js";
export type { LabFile } from "./lab-files.js";
export { CSV_MIME, JSONL_MIME, LabFileStore, csvName, jsonlName } from "./lab-files.js";
export type { LoadOptions } from "./loader.js";
export { compileMeshWasm, defaultWasmUrl, loadMeshWasm } from "./loader.js";
export type { PageOptions, Preset, Scenario } from "./scenario.js";
export { PRESETS, scenarioFromQuery, scenarioJson } from "./scenario.js";
