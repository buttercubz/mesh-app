// Builds @meshapp/engine-api: clean dist/, then `tsc -p tsconfig.build.json`.
// src/generated/ is committed output of `cargo xtask gen-ts`; the build fails with that
// hint when it is missing instead of emitting a client without its types.
import { spawnSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pkg = join(dirname(fileURLToPath(import.meta.url)), "..");

if (!existsSync(join(pkg, "src/generated/index.ts"))) {
  console.error("engine-api: src/generated/ is missing; run `cargo xtask gen-ts` first.");
  process.exit(1);
}

rmSync(join(pkg, "dist"), { recursive: true, force: true });

const tsc = spawnSync("tsc", ["-p", "tsconfig.build.json"], {
  cwd: pkg,
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (tsc.error) {
  console.error(`engine-api: could not run tsc: ${tsc.error.message}`);
  process.exit(1);
}
process.exit(tsc.status ?? 1);
