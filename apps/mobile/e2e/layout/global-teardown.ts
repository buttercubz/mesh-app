// Layout audit global teardown: merges every device×locale part into report.json and
// summary.md (also after a failing CI subset, so the failures come with screenshots).
import { mergeReport } from "./report";

export default function globalTeardown(): void {
  const merged = mergeReport();
  if (merged !== null) console.log(`[layout] report: ${merged.report}\n[layout] summary: ${merged.summary}`);
}
