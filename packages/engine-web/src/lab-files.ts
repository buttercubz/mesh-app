// The Lab files the simulated phone writes: labrun/1 JSONL (streamed by `mw_poll_lab`)
// and the summary CSV (read with `sim.labCsv` once the run finished). File names match
// the engine's `labFinished.files`: `<runId>.jsonl` and `<runId>-summary.csv`.
// Kept in memory only (never in web storage).

/** A file the engine wrote, ready for `shareFile` / download. */
export interface LabFile {
  /** File name without directories (the path `labFinished.files` reports). */
  name: string;
  data: string;
  mime: string;
}

export const JSONL_MIME = "application/x-ndjson";
export const CSV_MIME = "text/csv";

export function jsonlName(runId: string): string {
  return `${runId}.jsonl`;
}

export function csvName(runId: string): string {
  return `${runId}-summary.csv`;
}

/** Last path segment (`labFinished.files` may carry directories on a phone). */
function baseName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i >= 0 ? path.slice(i + 1) : path;
}

export class LabFileStore {
  /** runId → JSONL lines, in run order. */
  readonly #jsonl = new Map<string, string[]>();
  readonly #csv = new Map<string, string>();
  #current: string | undefined;

  /**
   * Appends a `mw_poll_lab` chunk. Lines belong to the run opened by the last
   * `{"t":"run",…}` line; the core emits each run's lines in order.
   */
  append(chunk: string): void {
    for (const line of chunk.split("\n")) {
      if (line.trim() === "") continue;
      const runId = runIdOf(line);
      if (runId !== undefined) {
        this.#current = runId;
        if (!this.#jsonl.has(runId)) this.#jsonl.set(runId, []);
      }
      if (this.#current === undefined) continue;
      this.#jsonl.get(this.#current)?.push(line);
    }
  }

  /** Stores a run's summary CSV. */
  setCsv(runId: string, csv: string): void {
    this.#csv.set(runId, csv);
  }

  hasCsv(runId: string): boolean {
    return this.#csv.has(runId);
  }

  /** Forgets a run (`labDeleteRun`). */
  delete(runId: string): void {
    this.#jsonl.delete(runId);
    this.#csv.delete(runId);
    if (this.#current === runId) this.#current = undefined;
  }

  /** Every file, JSONL then CSV per run, in run order. */
  files(): LabFile[] {
    const out: LabFile[] = [];
    for (const [runId, lines] of this.#jsonl) {
      out.push({ name: jsonlName(runId), data: `${lines.join("\n")}\n`, mime: JSONL_MIME });
      const csv = this.#csv.get(runId);
      if (csv !== undefined) out.push({ name: csvName(runId), data: csv, mime: CSV_MIME });
    }
    return out;
  }

  /** A file by the path the engine reported. */
  read(path: string): LabFile | undefined {
    const name = baseName(path);
    return this.files().find((f) => f.name === name);
  }
}

function runIdOf(line: string): string | undefined {
  if (!line.includes('"run"')) return undefined;
  try {
    const v = JSON.parse(line) as { t?: unknown; runId?: unknown };
    return v.t === "run" && typeof v.runId === "string" ? v.runId : undefined;
  } catch (_e) {
    // Not JSON: the core only writes JSON lines; keep it as a line of the current run.
    return undefined;
  }
}
