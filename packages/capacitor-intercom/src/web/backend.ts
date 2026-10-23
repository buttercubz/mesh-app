// The engine behind the web implementation. Phase 0: `FakeLabEngine`, a scripted Lab
// peer so the Field test UI can be built in a browser. Phase 1 (WP-13): a backend over
// @meshapp/engine-web (WASM core + simulated convoy) implements the same interface, and
// `MeshIntercomWeb` (gating, visibility, wake lock, permissions, share) stays as is.
import type { CommandResult, UiCommand, UiEvent } from "@meshapp/engine-api";

import type { StartEngineOptions } from "../definitions.js";

/** A file the engine wrote (Lab exports), readable for `shareFile`. */
export interface BackendFile {
  /** File name without directories (used for downloads). */
  name: string;
  data: string;
  mime: string;
}

/** The backend's ridelog/1 recorder (engine-web's `RideLogRecorder`, structurally). */
export interface DiagBackend {
  readonly enabled: boolean;
  setEnabled(on: boolean): void;
  status(): {
    enabled: boolean;
    sessionId: string | null;
    rideKey: string | null;
    node: number | null;
    bytes: number;
    sessions: number;
    oldestMs: number | null;
  };
  list(): { sessionId: string; bytes: number; segments: number; modifiedMs: number }[];
  files(scope: "last" | "all"): BackendFile[];
  clear(): void;
  mark(label: string): boolean;
  note(ev: string, fields: Record<string, string | number | boolean | null | number[]>): boolean;
}

export interface EngineBackend {
  /** `full` when the Rust core runs (WASM, Phase 1); `platform-only` for the fake. */
  readonly engineKind: "platform-only" | "full";
  /** Delivers every engine event; set once by the plugin before `start`. */
  setEventSink(sink: (event: UiEvent) => void): void;
  /** Starts the engine (restarts it when running). Rejects with `IntercomError`. */
  start(opts: StartEngineOptions): Promise<void>;
  /** Stops the engine; an active Lab run is cut short (no summary). Idempotent. */
  stop(): Promise<void>;
  /** Runs one validated command. ★ gating is done by the plugin before this call. */
  command(command: UiCommand): CommandResult | Promise<CommandResult>;
  /** PTT fast path (native: `mesh_rt_set_flags`), stamped for T8 (§12.2). */
  pttFast(down: boolean, tUiMs: number | undefined, receivedAtMs: number): void;
  /** Snapshots are emitted only while attached (§3.5 `set_ui_attached`). */
  setUiAttached(attached: boolean): void;
  status(): { engine: "idle" | "running"; live: boolean };
  /** Reads a file the engine wrote, by the path it reported (`labFinished.files`). */
  readFile(path: string): BackendFile | undefined;
  /** In-app invite scanner stand-in. */
  scanInviteQr(): { url?: string; cancelled: boolean };
  /** Diagnostics recorder (absent in the scripted Phase 0 fake). */
  readonly diag?: DiagBackend;
}
