// Builds @meshapp/capacitor-intercom: clean dist/, then `tsc -p tsconfig.build.json`.
// The package is TS-only (no ios/ or android/ directories, no `capacitor` native
// metadata in package.json): the native bridges are app-local (WP-19, DESIGN §2.4).
import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pkg = join(dirname(fileURLToPath(import.meta.url)), "..");

rmSync(join(pkg, "dist"), { recursive: true, force: true });

const tsc = spawnSync("tsc", ["-p", "tsconfig.build.json"], {
  cwd: pkg,
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (tsc.error) {
  console.error(`capacitor-intercom: could not run tsc: ${tsc.error.message}`);
  process.exit(1);
}
process.exit(tsc.status ?? 1);
