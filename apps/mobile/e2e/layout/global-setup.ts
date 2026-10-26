// Layout audit global setup: drops the parts of a previous run in the same output
// directory, so report.json only merges this run (screenshots are overwritten in place).
import { rmSync } from "node:fs";
import { join } from "node:path";

import { outDir } from "./report";

export default function globalSetup(): void {
  rmSync(join(outDir(), "parts"), { recursive: true, force: true });
}
