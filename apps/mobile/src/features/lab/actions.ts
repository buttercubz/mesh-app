// Field test actions (WP-21, DESIGN §12.1, §12.5). The only engine calls of the feature:
// startEngine/stopEngine for the Lab code, and the commands labStart (routed through
// gestureCommand by the engine client whenever audio ≠ off), labStop, labMark,
// labListRuns and labDeleteRun. Calls that must stay inside a tap (startEngine,
// labStart) are issued synchronously, before any await.
import type { ErrorCode, LabRunInfo } from "@meshapp/engine-api";
import { rejectionCode } from "@meshapp/engine-api";

import type { AppEngine } from "../../engine/client";
import { useLabStore } from "../../store/useLabStore";
import type { BenchSpec } from "./catalog";
import type { BenchDraft, DraftProblem } from "./config";
import { toLabParams } from "./config";
import { useFieldTestStore } from "./store";

/** The engine surface the Field test uses (the app engine, or a fake in tests). */
export type LabEngine = Pick<
  AppEngine,
  "command" | "startEngine" | "stopEngine" | "requestPermissions" | "checkPermissions" | "shareFile" | "setKeepScreenOn"
>;

/** Lab codes are 4 digits, the same on every phone (§12.1). */
export const LAB_CODE_PATTERN = /^\d{4}$/;

/** Mark labels are user content recorded in the file; the engine accepts ≤ 64 bytes. */
export const MARK_MAX_BYTES = 64;

export type ActionOutcome = { ok: true } | { ok: false; code: ErrorCode };
export type StartOutcome = { ok: true; runId: string } | { ok: false; code?: ErrorCode; problem?: DraftProblem };

const store = () => useFieldTestStore.getState();

function failure(reason: unknown): ErrorCode {
  return rejectionCode(reason) ?? "internal";
}

function refused(code: ErrorCode): { ok: false; code: ErrorCode } {
  store().fail(code);
  return { ok: false, code };
}

/** Joins a Lab code: starts the engine in Lab mode (gesture: call it from the tap). */
export function joinLab(engine: LabEngine, code: string): Promise<ActionOutcome> {
  const trimmed = code.trim();
  if (!LAB_CODE_PATTERN.test(trimmed)) return Promise.resolve(refused("invalidParams"));
  if (store().joining) return Promise.resolve({ ok: false, code: "invalidCommand" });
  // Synchronous plugin call: still inside the tap.
  const pending = engine.startEngine({ lab: { code: trimmed } });
  store().patch({ joining: true, error: null });
  return pending.then(
    () => {
      store().patch({ joining: false, code: trimmed });
      void refreshRuns(engine);
      return { ok: true } as const;
    },
    (reason: unknown) => {
      store().patch({ joining: false });
      return refused(failure(reason));
    },
  );
}

/** Leaves the Lab: stops the engine (an active run is cut short and kept without summary). */
export async function leaveLab(engine: LabEngine): Promise<ActionOutcome> {
  store().patch({ leaving: true, error: null });
  try {
    await engine.stopEngine();
  } catch (e) {
    store().patch({ leaving: false });
    return refused(failure(e));
  }
  store().patch({ leaving: false, code: null, started: null, starting: false, stopping: false });
  // The live ring belongs to the stopped run; the stored runs are listed again.
  useLabStore.setState({ activeRunId: null, samples: [] });
  await refreshRuns(engine);
  return { ok: true };
}

/**
 * Starts a run from the config draft. The draft must carry an audio path and an anchor
 * (§12.1); `labStart` is sent synchronously so a ★ start (audio ≠ off) stays inside
 * the tap that called this.
 */
export function startRun(engine: LabEngine, spec: BenchSpec, draft: BenchDraft): Promise<StartOutcome> {
  const check = toLabParams(spec, draft);
  if (!check.ok) return Promise.resolve({ ok: false, problem: check.problem });
  if (store().starting) return Promise.resolve({ ok: false, code: "labRunActive" });
  const params = check.params;
  const pending = engine.command({ type: "labStart", bench: spec.bench, params });
  store().patch({ starting: true, error: null });
  return pending.then(
    (result) => {
      if (!result.ok) {
        store().patch({ starting: false });
        return refused(result.code);
      }
      store().patch({
        starting: false,
        started: { runId: result.runId, bench: spec.bench, params, at: Date.now() },
      });
      void refreshRuns(engine);
      return { ok: true, runId: result.runId } as const;
    },
    (reason: unknown) => {
      store().patch({ starting: false });
      return refused(failure(reason));
    },
  );
}

/** Stops the running run (the engine answers with labFinished). */
export async function stopRun(engine: LabEngine): Promise<ActionOutcome> {
  store().patch({ stopping: true, error: null });
  try {
    const result = await engine.command({ type: "labStop" });
    if (!result.ok) return refused(result.code);
    return { ok: true };
  } catch (e) {
    return refused(failure(e));
  } finally {
    store().patch({ stopping: false });
  }
}

/** Adds a mark (passengers / stops only; the copy says so next to the button). */
export async function markRun(engine: LabEngine, label: string): Promise<ActionOutcome> {
  const text = label.trim();
  if (text === "" || new TextEncoder().encode(text).length > MARK_MAX_BYTES) return refused("invalidParams");
  try {
    const result = await engine.command({ type: "labMark", label: text });
    return result.ok ? { ok: true } : refused(result.code);
  } catch (e) {
    return refused(failure(e));
  }
}

/** Reloads the stored runs into useLabStore. */
export async function refreshRuns(engine: LabEngine): Promise<LabRunInfo[] | null> {
  try {
    const result = await engine.command({ type: "labListRuns" });
    if (!result.ok) {
      store().fail(result.code);
      return null;
    }
    useLabStore.getState().setRuns(result.runs);
    return result.runs;
  } catch (e) {
    store().fail(failure(e));
    return null;
  }
}

/** Deletes a stored run and its files. */
export async function deleteRun(engine: LabEngine, runId: string): Promise<ActionOutcome> {
  try {
    const result = await engine.command({ type: "labDeleteRun", runId });
    if (!result.ok) return refused(result.code);
    useLabStore.getState().removeRun(runId);
    return { ok: true };
  } catch (e) {
    return refused(failure(e));
  }
}

/** MIME type of a Lab file (labrun/1 JSONL or the summary CSV). */
export function labFileMime(path: string): string {
  if (path.endsWith(".jsonl")) return "application/x-ndjson";
  if (path.endsWith(".csv")) return "text/csv";
  return "application/octet-stream";
}

/** JSONL first (the file the analysis needs), then the CSV. */
export function exportOrder(files: readonly string[]): string[] {
  const rank = (p: string) => (p.endsWith(".jsonl") ? 0 : p.endsWith(".csv") ? 1 : 2);
  return [...files].sort((a, b) => rank(a) - rank(b));
}

/** Shares every file of a run through the share sheet (AirDrop to the Mac, §12.3). */
export async function exportRun(engine: LabEngine, runId: string, files: readonly string[]): Promise<ActionOutcome> {
  if (files.length === 0) return refused("storageFailed");
  store().patch({ exporting: [...store().exporting, runId], error: null });
  try {
    for (const path of exportOrder(files)) await engine.shareFile(path, labFileMime(path));
    return { ok: true };
  } catch (e) {
    return refused(failure(e));
  } finally {
    store().patch({ exporting: store().exporting.filter((id) => id !== runId) });
  }
}
