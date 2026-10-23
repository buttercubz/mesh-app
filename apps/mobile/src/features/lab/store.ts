// Field test session state (WP-21): the joined Lab code, the run this phone started and
// the in-flight flags of the Lab actions. Not persisted (a Lab session ends with the
// engine). Runs, live samples and the last finished run live in useLabStore (WP-18),
// which the single engine subscription fills from labProgress / labFinished.
import type { ErrorCode, LabBench, LabParams } from "@meshapp/engine-api";
import { create } from "zustand";

export interface StartedRun {
  readonly runId: string;
  readonly bench: LabBench;
  readonly params: LabParams;
  /** Local time (ms) the start was confirmed. */
  readonly at: number;
}

export interface FieldTestError {
  readonly code: ErrorCode;
  readonly at: number;
}

export interface FieldTestData {
  /** Lab code this phone joined (engine running in Lab mode), else `null`. */
  code: string | null;
  joining: boolean;
  leaving: boolean;
  /** A labStart is in flight (the screen stays awake until it resolves). */
  starting: boolean;
  stopping: boolean;
  /** Run started from this phone (controller), until its labFinished arrives. */
  started: StartedRun | null;
  /** Last refused Lab action, shown inline. */
  error: FieldTestError | null;
  /** Run ids whose export is in flight. */
  exporting: readonly string[];
}

export interface FieldTestState extends FieldTestData {
  patch(update: Partial<FieldTestData>): void;
  fail(code: ErrorCode, at?: number): void;
  clearError(): void;
  reset(): void;
}

const initial: FieldTestData = {
  code: null,
  joining: false,
  leaving: false,
  starting: false,
  stopping: false,
  started: null,
  error: null,
  exporting: [],
};

export const useFieldTestStore = create<FieldTestState>()((set) => ({
  ...initial,
  patch: (update) => set(update),
  fail: (code, at = Date.now()) => set({ error: { code, at } }),
  clearError: () => set({ error: null }),
  reset: () => set(initial),
}));
