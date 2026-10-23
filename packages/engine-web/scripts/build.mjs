// Builds @meshapp/engine-web: checks that `cargo xtask build-wasm` wrote the module,
// cleans dist/, then `tsc -p tsconfig.build.json`. dist/ loads ../wasm/mesh_wasm.wasm
// relative to itself, so the module is not copied.
import { spawnSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const pkg = join(dirname(fileURLToPath(import.meta.url)), "..");

if (!existsSync(join(pkg, "wasm/mesh_wasm.wasm"))) {
  console.error("engine-web: wasm/mesh_wasm.wasm is missing; run `cargo xtask build-wasm` first.");
  process.exit(1);
}

rmSync(join(pkg, "dist"), { recursive: true, force: true });

const tsc = spawnSync("tsc", ["-p", "tsconfig.build.json"], {
  cwd: pkg,
  stdio: "inherit",
  shell: process.platform === "win32",
});
if (tsc.error) {
  console.error(`engine-web: could not run tsc: ${tsc.error.message}`);
  process.exit(1);
}
process.exit(tsc.status ?? 1);
