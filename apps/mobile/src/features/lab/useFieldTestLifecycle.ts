// Keeps the Field test state in step with the engine (WP-21): lists the stored runs on
// entry, clears the started run when its labFinished arrives, learns the bench of a run
// another phone started (labProgress carries only the run id) and forgets the Lab code
// when the engine stops underneath the screen.
import { useEffect } from "react";

import { useEngineStore } from "../../store/useEngineStore";
import { useLabStore } from "../../store/useLabStore";
import type { LabEngine } from "./actions";
import { refreshRuns } from "./actions";
import { useFieldTestStore } from "./store";

export function useFieldTestLifecycle(engine: LabEngine): void {
  const finishedId = useLabStore((s) => s.lastFinished?.runId ?? null);
  const activeRunId = useLabStore((s) => s.activeRunId);
  const activeKnown = useLabStore((s) => s.activeRunId === null || s.runs.some((r) => r.runId === s.activeRunId));
  const status = useEngineStore((s) => s.status);

  useEffect(() => {
    void refreshRuns(engine);
  }, [engine]);

  useEffect(() => {
    if (finishedId === null) return;
    const { started, patch } = useFieldTestStore.getState();
    if (started?.runId === finishedId) patch({ started: null });
    void refreshRuns(engine);
  }, [finishedId, engine]);

  useEffect(() => {
    if (activeRunId !== null && !activeKnown) void refreshRuns(engine);
  }, [activeRunId, activeKnown, engine]);

  useEffect(() => {
    if (status !== "idle") return;
    const { code, joining, leaving, patch } = useFieldTestStore.getState();
    if (code !== null && !joining && !leaving) patch({ code: null, started: null, starting: false });
  }, [status]);
}
