// Test-only helper (imported by the *.test.ts[x] files of this feature, never by app
// code): absolute path of a repository file, found from the working directory so it
// works in every Vitest environment (jsdom rewrites import.meta.url).
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";

export function repoFile(relative: string): string {
  let dir = process.cwd();
  while (!existsSync(join(dir, "pnpm-workspace.yaml"))) {
    const up = dirname(dir);
    if (up === dir) throw new Error(`repository root not found from ${process.cwd()}`);
    dir = up;
  }
  return join(dir, relative);
}
