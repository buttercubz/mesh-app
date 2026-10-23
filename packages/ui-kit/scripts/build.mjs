#!/usr/bin/env node
// Builds @meshapp/ui-kit into dist/ (WP-20 acceptance "builds to dist"):
//   1. clean dist/
//   2. tsc -p tsconfig.build.json  (ESM .js + .d.ts + maps)
//   3. copy src/styles/*.css → dist/*.css (the package's CSS entry points)
//   4. self-check: every package.json export resolves to a file, and the JS entry
//      imports in plain Node with the expected public names.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const pkgDir = fileURLToPath(new URL("../", import.meta.url));
const dist = path.join(pkgDir, "dist");
const styles = path.join(pkgDir, "src", "styles");

function fail(message) {
  console.error(`ui-kit build: ${message}`);
  process.exit(1);
}

rmSync(dist, { recursive: true, force: true });

const require = createRequire(import.meta.url);
const tsc = require.resolve("typescript/bin/tsc");
const result = spawnSync(process.execPath, [tsc, "-p", "tsconfig.build.json"], { cwd: pkgDir, stdio: "inherit" });
if (result.status !== 0) fail(`tsc exited with ${result.status ?? result.signal}`);

mkdirSync(dist, { recursive: true });
const cssFiles = readdirSync(styles).filter((f) => f.endsWith(".css"));
for (const f of cssFiles) copyFileSync(path.join(styles, f), path.join(dist, f));

// Self-check 1: every export target exists.
const pkg = JSON.parse(readFileSync(path.join(pkgDir, "package.json"), "utf8"));
for (const [entry, target] of Object.entries(pkg.exports)) {
  const files = typeof target === "string" ? [target] : Object.values(target);
  for (const f of files) {
    if (!existsSync(path.join(pkgDir, f))) fail(`export "${entry}" points to missing ${f}`);
  }
}

// Self-check 2: CSS @imports inside dist resolve.
for (const f of cssFiles) {
  const css = readFileSync(path.join(dist, f), "utf8");
  for (const m of css.matchAll(/@import\s+"(\.\/[^"]+)"/g)) {
    if (!existsSync(path.join(dist, m[1]))) fail(`${f} imports missing ${m[1]}`);
  }
}

// Self-check 3: the ESM entry loads in Node and exposes the primitives.
const api = await import(pathToFileURL(path.join(dist, "index.js")).href);
const expected = [
  "Button",
  "BigPttButton",
  "SegmentedControl",
  "ListRow",
  "ListSection",
  "Badge",
  "QualityBars",
  "Sheet",
  "Banner",
  "Toast",
  "Toaster",
  "Sparkline",
  "applyTheme",
  "createPttGesture",
];
const missing = expected.filter((name) => typeof api[name] !== "function");
if (missing.length) fail(`dist/index.js is missing exports: ${missing.join(", ")}`);

console.log(`ui-kit build: dist/ ready (${cssFiles.length} CSS entry points, ${Object.keys(api).length} JS exports)`);
