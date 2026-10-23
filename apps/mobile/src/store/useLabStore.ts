// Lab store (WP-18, DESIGN §11.3, §12.5): the Field test runs list and a bounded ring of
// live samples for the running bench (600 samples = 5 min at the 2 Hz labProgress rate).
// Not persisted: runs live in the native Lab file store and are listed with labListRuns.
import type { LabLiveSample, LabRunInfo, LabSummary } from "@meshapp/engine-api";
import { create } from "zustand";

/** Live samples kept for the sparklines (DESIGN §11.3: bounded 600). */
export const LAB_SAMPLE_RING = 600;

export interface LabSamplePoint {
  elapsedS: number;
  sample: LabLiveSample;
}

export interface LabFinishedRun {
  runId: string;
  summary: LabSummary;
  files: string[];
}

export interface LabData {
  runs: LabRunInfo[];
  /** Run currently reporting progress, if any. */
  activeRunId: string | null;
  samples: LabSamplePoint[];
  lastFinished: LabFinishedRun | null;
}

export interface LabState extends LabData {
  setRuns(runs: LabRunInfo[]): void;
  /** labProgress event: a new run id restarts the ring. */
  progress(runId: string, elapsedS: number, sample: LabLiveSample): void;
  /** labFinished event. */
  finished(runId: string, summary: LabSummary, files: string[]): void;
  removeRun(runId: string): void;
  reset(): void;
}

const initial: LabData = { runs: [], activeRunId: null, samples: [], lastFinished: null };

/** Appends to a ring buffer of at most `limit` entries (returns a new array). */
export function pushBounded<T>(ring: readonly T[], item: T, limit: number = LAB_SAMPLE_RING): T[] {
  const next = ring.length >= limit ? ring.slice(ring.length - limit + 1) : ring.slice();
  next.push(item);
  return next;
}

export const useLabStore = create<LabState>()((set) => ({
  ...initial,
  setRuns: (runs) => set({ runs: [...runs].sort((a, b) => b.startedAt.localeCompare(a.startedAt)) }),
  progress: (runId, elapsedS, sample) =>
    set((state) =>
      state.activeRunId === runId
        ? { samples: pushBounded(state.samples, { elapsedS, sample }) }
        : { activeRunId: runId, samples: [{ elapsedS, sample }] },
    ),
  finished: (runId, summary, files) =>
    set((state) => ({
      activeRunId: state.activeRunId === runId ? null : state.activeRunId,
      lastFinished: { runId, summary, files: [...files] },
      runs: state.runs.map((r) =>
        r.runId === runId ? { ...r, durationS: summary.durationS, verdict: summary.verdict, files: [...files] } : r,
      ),
    })),
  removeRun: (runId) =>
    set((state) => ({
      runs: state.runs.filter((r) => r.runId !== runId),
      lastFinished: state.lastFinished?.runId === runId ? null : state.lastFinished,
    })),
  reset: () => set(initial),
}));
