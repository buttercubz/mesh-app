#!/usr/bin/env node
// Single task runner (owned by WP-01, DESIGN §2.5; convLink pattern).
//
//   node scripts/tasks.mjs <group> [action] [args...] [--dry-run] [--allow-missing]
//   pnpm run <dev|build|test|lint|ci|help>   (root package.json scripts call this file)
//
// Planning is pure (planTask) and execution never goes through a shell: every step is a
// program plus literal arguments, optionally with a working directory. A step that
// needs the build environment runs after `source scripts/env.sh`, whose exported
// variables are captured once and passed to every later step.
//
// --allow-missing (ci only): skip the steps whose owning work package has not landed
// yet (a required file is missing, or an xtask subcommand is still the WP-01 stub that
// calls `util::not_implemented`). Without it every step runs and a missing owner fails
// loudly.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

const PICK_SIM = "scripts/pick-sim.sh";

const usage = {
  dev: ["dev                          build workspace deps, then run the app dev server (WASM sim)"],
  build: [
    "build                        everything: Rust (host, bindings, wasm, iOS, Android) + packages + cap sync + debug APK",
    "build js                     pnpm -r build only",
  ],
  test: ["test                         pnpm -r test"],
  lint: ["lint                         eslint root tooling + pnpm -r lint"],
  rust: [
    "rust test                    cargo test --workspace (after fetch-opus)",
    "rust check                   cargo xtask check (fmt, clippy, determinism, pins, wasm tree)",
    "rust fetch-opus              cargo xtask fetch-opus",
    "rust build-ios               cargo xtask build-ios (MeshCoreFFI.xcframework)",
    "rust build-android           cargo xtask build-android (jniLibs)",
    "rust build-wasm              cargo xtask build-wasm (packages/engine-web/wasm)",
    "rust gen                     cargo xtask gen-ts + gen-bindings (write generated files)",
    "rust vectors                 cargo xtask vectors (write golden vectors)",
    "rust size                    cargo xtask size (size gate)",
  ],
  ios: [
    "ios build                    build-ios + cap sync ios + xcodebuild App (unsigned)",
    "ios test                     build-ios + xcodebuild test MeshPlatformKit + MeshEngineKit",
    "ios open                     pnpm exec cap open ios",
  ],
  android: [
    "android build                build-android + cap sync android + gradle assembleDebug",
    "android test                 host mesh-ffi + :meshengine unit tests and lint",
  ],
  cap: ["cap sync [ios|android]       build workspace deps + pnpm exec cap sync"],
  lab: ["lab merge [args...]          node tools/lab-analyze/cli.mjs merge (labrun/1 files)"],
  doctor: ["doctor                       bash scripts/doctor.sh (prints what is missing)"],
  ci: ["ci [--allow-missing]         the DESIGN §2.5 CI sequence"],
};

export function help(group) {
  const lines = group ? usage[group] : Object.values(usage).flat();
  return [
    `Usage: node scripts/tasks.mjs ${group ?? "<group>"} [action] [args...] [--dry-run]`,
    "",
    ...lines.map((l) => `  ${l}`),
    "",
    "  help                         this list",
    "",
    "--dry-run        print the command sequence without running it.",
    "--allow-missing  (ci) skip steps whose owning work package has not landed yet.",
    "Root scripts: pnpm run dev|build|test|lint|ci|help.",
    "",
  ].join("\n");
}

// ---------------------------------------------------------------------------------
// Step constructors
// ---------------------------------------------------------------------------------

/** `source scripts/env.sh`: captured once, then passed to every later step. */
const sourceEnv = () => ({ kind: "env" });

/** A program with literal arguments; `cwd` is relative to the repository root. */
function cmd(command, args, { cwd, needs = [] } = {}) {
  return { kind: "run", command, args, cwd, needs };
}

/** Requirement: `file` must exist (owner = WP that creates it). */
const file = (p, owner) => ({ type: "file", path: p, owner });

/** Requirement: xtask module `name` must no longer be the WP-01 stub. */
const xtaskImpl = (name, owner) => ({ type: "xtask", path: `tools/xtask/src/${name}.rs`, owner });

/** A dynamic argument: computed right before the step runs, shown as `display`. */
const dynamic = (display, resolve) => ({ dynamic: true, display, resolve });

function pickSim() {
  const out = spawnSync("bash", [PICK_SIM], { cwd: repositoryRoot, encoding: "utf8" });
  if (out.status !== 0 || !out.stdout.trim()) {
    throw new Error(`${PICK_SIM} found no available iPhone simulator: ${out.stderr.trim()}`);
  }
  return out.stdout.trim().split("\n").pop().trim();
}

const simDestination = (depth) =>
  dynamic(`id=$(${"../".repeat(depth)}${PICK_SIM})`, () => `id=${pickSim()}`);

const xtask = (sub, extra = [], needs = []) => cmd("cargo", ["xtask", sub, ...extra], { needs });
const fetchOpus = () => xtask("fetch-opus");

// Owner requirements of the DESIGN §2.5 steps.
const NEEDS = {
  vectors: [xtaskImpl("vectors", "WP-04")],
  genTs: [xtaskImpl("ts", "WP-03")],
  bindings: [xtaskImpl("bindings", "WP-12")],
  ios: [xtaskImpl("ios", "WP-12")],
  android: [xtaskImpl("android", "WP-12")],
  wasm: [xtaskImpl("wasm", "WP-13")],
  size: [xtaskImpl("ios", "WP-12"), xtaskImpl("android", "WP-12")],
  mobile: [file("apps/mobile/package.json", "WP-18")],
  playwright: [file("apps/mobile/playwright.config.ts", "WP-18")],
  layout: [file("apps/mobile/playwright.layout.config.ts", "WP-18")],
  capSync: [file("apps/mobile/capacitor.config.ts", "WP-18")],
  platformKit: [file("native/ios/MeshPlatformKit/Package.swift", "WP-14")],
  engineKit: [file("native/ios/MeshEngineKit/Package.swift", "WP-15"), xtaskImpl("ios", "WP-12")],
  iosApp: [
    file("apps/mobile/ios/App/App.xcodeproj/project.pbxproj", "WP-18"),
    file("apps/mobile/ios/App/App/Plugins/MeshIntercomPlugin.swift", "WP-19"),
    xtaskImpl("ios", "WP-12"),
  ],
  meshengine: [file("native/android/gradlew", "WP-16")],
  androidApp: [file("apps/mobile/android/gradlew", "WP-18"), xtaskImpl("android", "WP-12")],
  lab: [file("tools/lab-analyze/cli.mjs", "WP-05")],
};

const xcodeTest = (pkg, scheme, needs) =>
  cmd("xcodebuild", ["-scheme", scheme, "-destination", simDestination(3), "test"], {
    cwd: `native/ios/${pkg}`,
    needs,
  });
const xcodeApp = () =>
  cmd(
    "xcodebuild",
    [
      "-project",
      "apps/mobile/ios/App/App.xcodeproj",
      "-scheme",
      "App",
      "-destination",
      "generic/platform=iOS",
      "CODE_SIGNING_ALLOWED=NO",
      "build",
    ],
    { needs: NEEDS.iosApp },
  );
const capSync = (...platform) =>
  cmd("pnpm", ["exec", "cap", "sync", ...platform], { cwd: "apps/mobile", needs: NEEDS.capSync });
const buildMobileDeps = () =>
  cmd("pnpm", ["--filter", "@meshapp/mobile...", "build"], { needs: NEEDS.mobile });
const meshengineGradle = (...tasks) =>
  cmd("./gradlew", tasks.map((t) => `:meshengine:${t}`), { cwd: "native/android", needs: NEEDS.meshengine });
const hostMeshFfi = () => cmd("cargo", ["build", "-p", "mesh-ffi", "--release"]);

/**
 * `pnpm run build`: every Rust artifact the app consumes, then every package, then
 * `cap sync` for both platforms and the Android debug APK. Generated TS types and
 * bindings are rewritten by their xtask commands (no-ops when up to date). The iOS
 * xcframework needs macOS.
 */
export function buildAllSteps(platform = process.platform) {
  return [
    sourceEnv(),
    fetchOpus(),
    cmd("cargo", ["build", "--workspace"]),
    xtask("gen-ts", [], NEEDS.genTs),
    xtask("gen-bindings", [], NEEDS.bindings),
    xtask("build-wasm", [], NEEDS.wasm),
    ...(platform === "darwin" ? [xtask("build-ios", [], NEEDS.ios)] : []),
    xtask("build-android", [], NEEDS.android),
    cmd("pnpm", ["install", "--frozen-lockfile"]),
    cmd("pnpm", ["-r", "build"]),
    capSync(),
    cmd("./gradlew", ["assembleDebug"], { cwd: "apps/mobile/android", needs: NEEDS.androidApp }),
  ];
}

/** DESIGN §2.5, exactly and in order. */
export function ciSteps() {
  return [
    sourceEnv(),
    fetchOpus(),
    cmd("cargo", ["fmt", "--all", "--check"]),
    cmd("cargo", ["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    xtask("check"),
    cmd("cargo", ["test", "--workspace"]),
    xtask("vectors", ["--check"], NEEDS.vectors),
    xtask("gen-ts", ["--check"], NEEDS.genTs),
    xtask("gen-bindings", ["--check"], NEEDS.bindings),
    xtask("build-ios", [], NEEDS.ios),
    xtask("build-android", [], NEEDS.android),
    xtask("build-wasm", [], NEEDS.wasm),
    xtask("size", [], NEEDS.size),
    cmd("pnpm", ["install", "--frozen-lockfile"]),
    cmd("pnpm", ["-r", "build"]),
    cmd("pnpm", ["-r", "test"]),
    cmd("pnpm", ["-r", "lint"]),
    cmd("pnpm", ["--filter", "@meshapp/mobile", "exec", "playwright", "test"], { needs: NEEDS.playwright }),
    // Layout regression subset (no scroll, no clipping, safe areas) on small and big phones.
    cmd("pnpm", ["--filter", "@meshapp/mobile", "exec", "playwright", "test", "-c", "playwright.layout.config.ts"], {
      needs: NEEDS.layout,
    }),
    xcodeTest("MeshPlatformKit", "MeshPlatformKit-Package", NEEDS.platformKit),
    xcodeTest("MeshEngineKit", "MeshEngineKit-Package", NEEDS.engineKit),
    capSync(),
    xcodeApp(),
    hostMeshFfi(),
    meshengineGradle("testDebugUnitTest", "lintDebug", "assembleRelease"),
    cmd("./gradlew", ["assembleDebug"], { cwd: "apps/mobile/android", needs: NEEDS.androidApp }),
  ];
}

// ---------------------------------------------------------------------------------
// Planning (pure: invalid input is rejected before any process starts)
// ---------------------------------------------------------------------------------

function noArgs(args, what) {
  if (args.length) throw new Error(`${what} does not accept extra arguments: ${args.join(" ")}`);
}

function rustSteps(action) {
  const steps = {
    test: [fetchOpus(), cmd("cargo", ["test", "--workspace"])],
    check: [fetchOpus(), xtask("check")],
    "fetch-opus": [fetchOpus()],
    "build-ios": [fetchOpus(), xtask("build-ios")],
    "build-android": [fetchOpus(), xtask("build-android")],
    "build-wasm": [xtask("build-wasm")],
    gen: [fetchOpus(), xtask("gen-ts"), xtask("gen-bindings")],
    vectors: [xtask("vectors")],
    size: [xtask("size")],
  }[action];
  if (!steps) throw new Error(`Unknown rust action: ${action}`);
  return [sourceEnv(), ...steps];
}

function iosSteps(action) {
  if (action === "build") {
    return [sourceEnv(), fetchOpus(), xtask("build-ios"), buildMobileDeps(), capSync("ios"), xcodeApp()];
  }
  if (action === "test") {
    return [
      sourceEnv(),
      fetchOpus(),
      xtask("build-ios"),
      xcodeTest("MeshPlatformKit", "MeshPlatformKit-Package", NEEDS.platformKit),
      xcodeTest("MeshEngineKit", "MeshEngineKit-Package", NEEDS.engineKit),
    ];
  }
  if (action === "open") return [cmd("pnpm", ["exec", "cap", "open", "ios"], { cwd: "apps/mobile", needs: NEEDS.capSync })];
  throw new Error(`Unknown ios action: ${action}`);
}

function androidSteps(action) {
  if (action === "build") {
    return [
      sourceEnv(),
      fetchOpus(),
      xtask("build-android"),
      buildMobileDeps(),
      capSync("android"),
      cmd("./gradlew", ["assembleDebug"], { cwd: "apps/mobile/android", needs: NEEDS.androidApp }),
    ];
  }
  if (action === "test") {
    return [sourceEnv(), fetchOpus(), hostMeshFfi(), meshengineGradle("testDebugUnitTest", "lintDebug")];
  }
  throw new Error(`Unknown android action: ${action}`);
}

/**
 * argv → { steps, dryRun, allowMissing, help? }. Flags may appear anywhere; `lab merge`
 * forwards its remaining arguments literally, every other task rejects extra ones.
 */
export function planTask(argv) {
  const dryRun = argv.includes("--dry-run");
  const allowMissing = argv.includes("--allow-missing");
  const args = argv.filter((a) => a !== "--dry-run" && a !== "--allow-missing");
  const group = args.shift();
  const base = { steps: [], dryRun, allowMissing };
  if (!group || ["help", "--help", "-h"].includes(group)) return { ...base, help: help() };
  if (!Object.hasOwn(usage, group)) throw new Error(`Unknown task group: ${group}\n\n${help()}`);
  if (args.includes("--help") || args.includes("-h") || args[0] === "help") return { ...base, help: help(group) };
  if (allowMissing && group !== "ci") throw new Error("--allow-missing is only accepted by `ci`.");

  const single = ["dev", "test", "lint", "doctor", "ci"];
  const optionalAction = ["build"];
  const action = single.includes(group) ? undefined : args.shift();
  if (!single.includes(group) && !optionalAction.includes(group) && !action) return { ...base, help: help(group) };
  const platform = group === "cap" && ["ios", "android"].includes(args[0]) ? args.shift() : undefined;
  if (group !== "lab") noArgs(args, `${group}${action ? ` ${action}` : ""}`);

  let steps;
  switch (group) {
    case "dev":
      steps = [
        cmd("pnpm", ["--filter", "@meshapp/mobile^...", "build"], { needs: NEEDS.mobile }),
        cmd("pnpm", ["--filter", "@meshapp/mobile", "dev"], { needs: NEEDS.mobile }),
      ];
      break;
    case "build":
      if (action === "js") steps = [cmd("pnpm", ["-r", "build"])];
      else if (action === undefined) steps = buildAllSteps();
      else throw new Error(`Unknown build action: ${action}`);
      break;
    case "test":
      steps = [cmd("pnpm", ["-r", "test"])];
      break;
    case "lint":
      steps = [cmd("pnpm", ["exec", "eslint", "scripts", "eslint.config.js"]), cmd("pnpm", ["-r", "lint"])];
      break;
    case "doctor":
      steps = [cmd("bash", ["scripts/doctor.sh"])];
      break;
    case "ci":
      steps = ciSteps();
      break;
    case "rust":
      steps = rustSteps(action);
      break;
    case "ios":
      steps = iosSteps(action);
      break;
    case "android":
      steps = androidSteps(action);
      break;
    case "cap":
      if (action !== "sync") throw new Error(`Unknown cap action: ${action}`);
      steps = [buildMobileDeps(), capSync(...(platform ? [platform] : []))];
      break;
    case "lab":
      if (action !== "merge") throw new Error(`Unknown lab action: ${action}`);
      steps = [cmd(process.execPath, ["tools/lab-analyze/cli.mjs", "merge", ...args], { needs: NEEDS.lab })];
      break;
  }
  return { ...base, steps };
}

// ---------------------------------------------------------------------------------
// Formatting, readiness, execution
// ---------------------------------------------------------------------------------

const SAFE = /^[A-Za-z0-9_./:=@+^-]+$/;

function showArg(arg) {
  if (typeof arg === "object") return `"${arg.display}"`;
  return SAFE.test(arg) ? arg : JSON.stringify(arg);
}

/** Renders a step the way DESIGN §2.5 writes it. */
export function formatStep(step) {
  if (step.kind === "env") return "source scripts/env.sh";
  const program = step.command === process.execPath ? "node" : step.command;
  const line = [program, ...step.args.map(showArg)].join(" ");
  return step.cwd ? `(cd ${step.cwd} && ${line})` : line;
}

/** Why a step cannot run yet (owner not landed), or null when it is ready. */
export function missingOwner(step, { root = repositoryRoot, exists = existsSync, read = readFileSync } = {}) {
  for (const need of step.needs ?? []) {
    const full = path.join(root, need.path);
    if (!exists(full)) return `${need.owner} has not landed (${need.path} is missing)`;
    if (need.type === "xtask" && read(full, "utf8").includes("not_implemented(")) {
      return `${need.owner} has not landed (${need.path} is still the WP-01 stub)`;
    }
  }
  return null;
}

/** `source scripts/env.sh` in bash, returning the resulting environment. */
function loadEnv(spawn, env) {
  const out = spawn("bash", ["-c", "source scripts/env.sh && env -0"], {
    cwd: repositoryRoot,
    env,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (out.error || out.status !== 0) {
    throw new Error(`source scripts/env.sh failed: ${out.error?.message ?? out.stderr}`);
  }
  const next = {};
  for (const entry of out.stdout.split("\0")) {
    const eq = entry.indexOf("=");
    if (eq > 0) next[entry.slice(0, eq)] = entry.slice(eq + 1);
  }
  return next;
}

export function executePlan(plan, { spawn = spawnSync, env = process.env, log = console.log, error = console.error } = {}) {
  if (plan.help) {
    log(plan.help);
    return 0;
  }
  let stepEnv = env;
  const skipped = [];
  for (const step of plan.steps) {
    const shown = formatStep(step);
    const missing = plan.allowMissing ? missingOwner(step) : null;
    if (missing) {
      skipped.push(`${shown}  # ${missing}`);
      log(`${plan.dryRun ? "[dry-run] " : ""}[skip] ${shown}  # ${missing}`);
      continue;
    }
    log(`${plan.dryRun ? "[dry-run] " : "$ "}${shown}`);
    if (plan.dryRun) continue;
    if (step.kind === "env") {
      stepEnv = loadEnv(spawn, env);
      continue;
    }
    const args = step.args.map((a) => (typeof a === "object" ? a.resolve() : a));
    const cwd = step.cwd ? path.join(repositoryRoot, step.cwd) : repositoryRoot;
    const result = spawn(step.command, args, { cwd, env: stepEnv, stdio: "inherit", shell: false });
    if (result.error) {
      error(`Could not start ${step.command}: ${result.error.message}`);
      return 1;
    }
    if (result.signal) {
      error(`${step.command} stopped by ${result.signal}.`);
      return 1;
    }
    if (result.status !== 0) {
      error(`Failed (exit ${result.status}): ${shown}`);
      return result.status ?? 1;
    }
  }
  if (skipped.length) {
    log(`\n${skipped.length} step(s) skipped with --allow-missing (owners not landed):\n  ${skipped.join("\n  ")}`);
  }
  return 0;
}

export function runTask(argv, options = {}) {
  try {
    return executePlan(planTask(argv), options);
  } catch (err) {
    (options.error ?? console.error)(err instanceof Error ? err.message : String(err));
    return 1;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = runTask(process.argv.slice(2));
}
