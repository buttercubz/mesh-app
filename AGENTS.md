# AGENTS.md — rules for coding agents (and humans)

This file is the entry point every agent reads first. It is short on purpose: the
authoritative design is [`docs/DESIGN.md`](docs/DESIGN.md) and the machine-readable
work breakdown is [`docs/WORKPLAN.json`](docs/WORKPLAN.json). When this file and
DESIGN.md disagree, DESIGN.md wins and the disagreement is reported (never silently
"fixed" by editing DESIGN.md or WORKPLAN.json; those two files are owned by the design
owner, not by any work package).

## 0. Non-negotiable rules (summary)

1. Edit **only** your work package's `ownedPaths` (§2). Anything else goes through a
   contract change request (§4).
2. Never modify anything in the sibling `../comvoy-app` (convLink). It is a read-only
   reference for patterns and tokens.
3. Generated files are written **only** by their `cargo xtask` command (§6).
4. Run every `verify` command **from the repository root** (§7).
5. Capture-starting commands go **only** through `gestureCommand` (§8).
6. Determinism rules hold in the seven sans-IO crates (§9).
7. Code, comments and docs in **English**; UI strings through i18n in **ES + EN** (§10).
8. **pnpm only**; Capacitor CLI via `pnpm exec cap …`, never `npx` (§10).
9. BITACORA entries separate automated checks from **"pending physical — not faked"** (§12).
10. No text chat, no secrets in web storage, no trademarks in store copy (§10, §11).

## 1. What to read before you start

| You are working on | Read in DESIGN.md |
|---|---|
| anything | §0 (decisions L1-L10, D1-D12), §16 (work breakdown and agent rules), Appendix A (constants), Appendix C (decision log) |
| Rust core crates | §3 (crate graph, determinism, sans-IO API, module map), §4 (wire), §5-§8 (routing, floor, voice, security) |
| build tooling | §2 (layout, bootstrap, per-target builds, packaging, script surface and CI, pinned versions) |
| transports | §9 (link abstraction, BLE roles, GATT, advertising, rejoin, Wi-Fi Aware) |
| iOS / Android | §10.1 / §10.2 and the matching `docs/conventions/swift.md` / `kotlin.md` |
| TS plugin, app, UI | §11 (plugin C6, web sim, app structure, screens, tokens, i18n) |
| Lab and field | §12 (benches, `labrun/1` C7), §14 (phase acceptance) |
| tests | §13 (layers, simulator scenarios S1-S13, where each check runs) |

Then read your WORKPLAN entry in full: `description`, `interfaces`, `acceptance`,
`verify`, `verifyNightly`, `milestones`, `dependsOn`, `softDependsOn`, `needsDevice`.
Print it with:

```bash
node -e 'const w=require("./docs/WORKPLAN.json").find(p=>p.id===process.argv[1]);console.log(JSON.stringify(w,null,2))' WP-10A
```

Coding conventions per language: [`docs/conventions/`](docs/conventions/) —
[`rust.md`](docs/conventions/rust.md), [`swift.md`](docs/conventions/swift.md),
[`kotlin.md`](docs/conventions/kotlin.md), [`typescript.md`](docs/conventions/typescript.md),
[`commits.md`](docs/conventions/commits.md).

## 2. Ownership: `ownedPaths`

Every work package (WP) in `docs/WORKPLAN.json` lists `ownedPaths`. They are
**exclusive** and **do not overlap**.

- A path ending in `/` owns that whole subtree (`crates/mesh-wire/` owns every file
  below it, including new ones you create there).
- Any other path is exactly one file (`tools/xtask/src/ts.rs`).
- There are **no globs**. `crates/mesh-core/src/wire/secure.rs` owned by WP-10B means
  that one file, not its siblings.
- A WP edits only its own `ownedPaths`. Reading anything is always allowed.
- To find the owner of a path, take the longest matching `ownedPaths` entry:

```bash
node -e 'const p=process.argv[1];let best=null;for(const w of require("./docs/WORKPLAN.json"))for(const o of w.ownedPaths)if((o.endsWith("/")?p.startsWith(o):p===o)&&(!best||o.length>best[1].length))best=[w.id,o];console.log(best?best.join("  "):"unowned: ask the design owner")' crates/mesh-core/src/session/ride.rs
```

Allowed without ownership (tool output, never hand-edited):

- Lockfiles: `Cargo.lock`, `pnpm-lock.yaml` (regenerate with `cargo`/`pnpm install`;
  resolve conflicts by re-running the tool, never by hand). They are formally owned by
  WP-01 so that manual edits have one owner.
- Build outputs: `target/`, `build/`, `dist/`, `node_modules/`, the gitignored
  xcframework and `jniLibs/`, and the outputs of your own WP's xtask commands.
- Stub hand-offs listed in your WP description (§5).

A path that no WP owns is not free real estate: raise it with the design owner.

## 3. Phases, waves, milestones

- `phase` is the **first** phase in which the WP must deliver (0 = Phase 0 field test
  on iPhones … 5 = hardening; DESIGN §14).
- `wave` is the WP's dependency depth: `1 + max(wave of dependsOn)`, 0 when it has no
  dependencies. Every WP of one wave can run in parallel once all earlier waves are
  done (DESIGN §16.3).
- `dependsOn` are hard prerequisites: read their public APIs and build on them, never
  edit their files. `softDependsOn` names WPs whose absence only drops an optional
  variant (e.g. WP-F0 runs without the WP-23 widget; the T3 Live Activity variant
  moves to Phase 2).
- `milestones` (multi-phase WPs) each carry `phase`, `scope`, `acceptance` and
  optional extra dependencies needed only for that milestone. A WP is **"done for
  phase N"** when its milestones up to N pass. Later-phase milestones start when their
  phase starts and their milestone dependencies are met.
- `verify` commands are the per-WP gate for the current phase. `verifyNightly`
  commands (fuzzing, random-seed simulator suites) must fail loudly when run, but are
  not part of the per-WP gate; never fold them into a `|| true` or "skipped" step.
- `needsDevice: true` marks field WPs (WP-F0…F4, WP-25). Their [DEV] acceptance can
  only be satisfied on the user's phones; agents prepare tooling and the report
  skeleton and mark the rest "pending physical — not faked".

DESIGN tags: **[ENV]** buildable and verifiable on this Mac without devices, **[DEV]**
needs the user's phones, **[TUNE]** a default the Lab benches may change (Appendix B).

## 4. Contracts C1-C9 and change requests

Interfaces between WPs are frozen per phase and changed only by their owner
(DESIGN §16.1):

| ID | Contract | Owner | Artifact |
|---|---|---|---|
| C1 | UI command/event/snapshot JSON | WP-03 | `crates/mesh-api/src/lib.rs` + generated `packages/engine-api/src/generated/*.ts` |
| C2 | Wire formats and constants | WP-04 | `crates/mesh-wire` public API + `test-vectors/wire/` + `docs/protocol/WIRE.md` |
| C3 | Sans-IO engine API | WP-10A | `crates/mesh-core/src/lib.rs` |
| C4 | UniFFI surface + `PlatformHooks` | WP-12 | `crates/mesh-ffi/src/api.rs` + committed generated Swift/Kotlin |
| C5 | Real-time C ABI | WP-12 | `crates/mesh-ffi/include/mesh_rt.h` |
| C6 | Capacitor plugin API | WP-17 | `packages/capacitor-intercom/src/definitions.ts` |
| C7 | `labrun/1` schema | WP-05 | `crates/mesh-lab/schema/labrun-1.schema.json` |
| C8 | Crate APIs of mesh-crypto, mesh-voice, mesh-routing, mesh-lab, opus-sys shim | each crate's WP | `src/lib.rs` signatures |
| C9 | Platform link interface, audio sink, Live Activity attributes/intents | WP-14 (Kotlin mirror WP-16) | `MeshLink.swift`, `cmesh_audio.h`, `Sources/MeshActivityShared/` |

**Change request = the only route for any change outside your owned paths**, whether
it touches a frozen contract or just a file another WP owns (a manifest dependency, a
script, an app file). Do not edit the file. File the request in your WP report with:

- target WP and exact path;
- the exact change (signature, field, dependency line, text);
- why your WP needs it and what breaks without it;
- whether it is blocking, and the interim workaround you used (if any).

The owner applies it, regenerates artifacts (no drift), and bumps the contract
version where one exists (e.g. C1 `API_VERSION` minor after its freeze). New external
Rust dependencies for a crate you own go in that crate's `Cargo.toml` with an exact
`=x.y.z` pin and are reported as a deviation; workspace-wide pins belong to WP-01's
`[workspace.dependencies]`.

## 5. Stub hand-off rule

A creating WP may write a **compiling stub** of a file or subtree owned by a later WP
so that the build works from day 1; **ownership passes to the later WP at creation**
(DESIGN §16.2 rule 2). After that the creator never edits it again. Stubs in this plan:

| Creator | Stub | Passes to |
|---|---|---|
| WP-01 | every workspace crate (`Cargo.toml` with the DESIGN §3.1 edges and features + empty `src/lib.rs`, with `#![deny]` in the sans-IO crates) | the crate's WP |
| WP-01 | `tools/xtask/src/{ts,vectors,opus,ios,android,bindings,wasm}.rs`: print "not implemented (owned by WP-xx)" and exit 1, **except** `fetch-opus`, which prints "nothing to fetch" and exits 0 | WP-03, WP-04, WP-06, WP-12, WP-12, WP-12, WP-13 |
| WP-10A | `crates/mesh-core/src/wire/<module>.rs` no-op `register` functions and empty module directories | WP-10B (`secure`, `group`, `session`), WP-10C (`media`), WP-10D (`mesh`) |
| WP-18 | `apps/mobile/ios/App/App/Plugins/MeshIntercomPlugin.swift` and Android `plugin/MeshIntercomPlugin.kt` (answer `getInfo` with `engine: "platform-only"`, reject everything else with `unavailable`) | WP-19 |
| WP-18 | `apps/mobile/ios/IntercomWidget/` bundle compiled against `MeshActivityShared` | WP-23 |

A stub must compile and must not pretend to work: it returns an explicit "not
implemented / unavailable" error. Contract signatures may be stubbed with `todo!()`
only by the contract owner and only until the WP that implements them lands. Outside
listed stubs, no `todo!()`, `unimplemented!()`, empty test bodies or skipped tests are
left behind.

## 6. Generated files

Written **only** by their xtask command and checked for drift in CI (`--check`):

| Artifact | Command | Owner |
|---|---|---|
| `packages/engine-api/src/generated/*.ts` | `cargo xtask gen-ts` | WP-03 |
| `test-vectors/wire/`, `test-vectors/crypto/` | `cargo xtask vectors` | WP-04, WP-07 |
| `native/ios/MeshEngineKit/Sources/MeshCoreBindings/mesh_ffi.swift`, `native/android/meshengine/src/main/kotlin/com/convlink/voice/core/mesh_ffi.kt` | `cargo xtask gen-bindings` | WP-12 |
| `native/ios/MeshEngineKit/Frameworks/MeshCoreFFI.xcframework` (gitignored) | `cargo xtask build-ios` | WP-12 |
| `native/android/meshengine/src/main/jniLibs/` (gitignored) | `cargo xtask build-android` | WP-12 |
| `packages/engine-web/wasm/` | `cargo xtask build-wasm` | WP-13 |
| `third_party/opus/opus-1.6.1/` (gitignored, SHA-256 verified) | `cargo xtask fetch-opus` | WP-06 |

Never hand-edit a generated file, never "fix" drift by editing output: change the
source (Rust types, vector generator) and re-run the command.

## 7. Verify commands and their prerequisites

- Every `verify` command runs **from the repository root** (`cd` into a subdirectory
  only inside the command, e.g. `(cd native/android && ./gradlew …)`).
- Native builds: `source scripts/env.sh` first (ANDROID_HOME, NDK 27.1.12297006, SDK
  cmake 3.22.1 on PATH, JAVA_HOME 21, `CC_*`/`AR_*` and Android linkers).
- **`cargo xtask fetch-opus` before anything that builds `mesh-core`** directly or
  through `mesh-sim`, `mesh-ffi` or `--workspace` (the `opus` feature is on by
  default). It is idempotent; before WP-06 lands it prints "nothing to fetch".
- **Build workspace dependencies before app tests**:
  `pnpm --filter "@meshapp/mobile..." build` (packages are consumed from `dist/`).
  The same applies to any package: `pnpm --filter "<pkg>..." build`.
- **Capacitor CLI**: `pnpm exec cap …` (or `node scripts/tasks.mjs cap sync`), never
  `npx`, which could fetch an unpinned CLI.
- **Kotlin JVM tests**: build the host library first, `cargo build -p mesh-ffi --release`,
  because the binding tests load `libmesh_ffi.dylib` through JNA.
- iOS simulator destinations come from `scripts/pick-sim.sh` (never hard-code a
  device name).
- A verify that cannot pass only because another WP is missing or broken is reported
  precisely (which WP, which command, which error); everything else must still pass.

## 8. Gesture-command rule (capture gating)

Four UI commands may start microphone capture and are accepted **only** through the
plugin's `gestureCommand` (★ in DESIGN §3.7), which refuses unless the app is
foreground-active:

- `goLive`, `resumeMic`;
- `labStart` with `params.audio` `"full"` (and `"renderOnly"`, which activates the
  session but never starts capture);
- `audioCheck` with `op: "start"`.

The plain `send` pass-through rejects them with `useGestureCommand`, natively **and**
in the web implementation. Inside the tap the bridge synchronously performs the
OS-level work (iOS: activate the session, start VPIO with the sink muted, location
anchor, Live Activity; Android: `startForegroundService` with the `microphone` type,
open Oboe streams) and only then forwards the command. The core's `StartCapture` only
connects the RT sink and unmutes. UI code calls `client.gestureCommand(...)` (or the
`goLive()` / `resumeMic()` sugar) directly from the tap handler, never after an
`await` of something else or from a timer. A lint rule and tests enforce this (DESIGN
§3.2, §11.1, §13.1).

## 9. Determinism rules (Rust)

The seven sans-IO crates — `mesh-wire`, `mesh-crypto`, `mesh-lab`, `mesh-voice`,
`mesh-routing`, `mesh-core`, `mesh-sim` — put
`#![deny(clippy::disallowed_methods, clippy::disallowed_types)]` in `lib.rs`. The
workspace sets those lints to `allow`, so the deny applies **only** there. In those
crates: no `std::time`, no threads, no `getrandom` / OS randomness, no
`HashMap`/`HashSet` iteration (use `BTreeMap`, `Vec`, fixed arrays); RNG is an
injected `rand_chacha::ChaCha20Rng`; time is a caller-supplied `Instant(u64 µs)`; no
allocation in `push_capture`/`pull_playout`. `mesh-ffi`, `mesh-wasm`, `xtask`,
`uniffi-bindgen`, `opus-sys` and integration-test targets may use threads and clocks.
`cargo xtask check` adds a grep over the seven crates as a second line of defence.
Details: [`docs/conventions/rust.md`](docs/conventions/rust.md), DESIGN §3.1.

## 10. Language, tooling and product rules

- **English** for code, identifiers, comments, commit messages and docs. The UI ships
  **Spanish (`es`, neutral) and English**; every user-facing string goes through
  i18n catalogs with compile-time key parity. The engine emits i18n keys plus
  parameters, never English sentences.
- **pnpm only** (`packageManager: pnpm@11.5.3`). Never npm, yarn, bun or `npx`.
- Rust toolchain: `rust-toolchain.toml` (channel `1.99.1`, edition 2024). Do not
  change the user's default toolchain.
- **convLink TypeScript rules** (details in
  [`docs/conventions/typescript.md`](docs/conventions/typescript.md)): TS `strict` +
  `verbatimModuleSyntax` + `erasableSyntaxOnly`; React Compiler on, no `StrictMode`;
  one Zustand store per domain, non-React code uses `getState()`; every `catch {}`
  carries a comment saying why the error is ignored; **no secrets in web storage**
  (no keys, invite text or epoch secrets in localStorage/IndexedDB/persisted stores);
  **no text chat**, ever.
- Product copy: the honest range promise (DESIGN §0.2); no Cardo/Sena trademarks; no
  text input on the ride screen; Lab markers are for passengers or stops.
- Working product defaults (DESIGN Appendix E): app name "convLink Voice", bundle id /
  applicationId `com.convlink.voice`, URL scheme `convlinkvoice`. Demo-ride bot lines
  may stay as clearly marked placeholders in the `prompts` catalog.

## 11. Repository boundaries

- **Never modify `../comvoy-app`** (convLink). Read it for patterns (task runner,
  i18n `defineCatalog`, tokens, SceneDelegate fix, vault pattern), copy what you need
  into your owned paths, and leave it untouched.
- Never modify anything outside this repository.
- `mesh-app` currently sits inside the parent `loRa` git repository; the user decides
  when it becomes its own repo. Agents run **read-only git only** (`status`, `log`,
  `diff`, `show`); no `init`, `add`, `commit`, `checkout`, `switch`, `reset`, `stash`,
  `rm`, `clean` or `merge` unless the user asks. Commit message format, once commits
  happen: [`docs/conventions/commits.md`](docs/conventions/commits.md).
- Network downloads are limited to crates.io, npm, Maven/Google, xiph.org and GitHub
  releases, always with exact version pins (and SHA-256 for tarballs).

## 12. BITACORA (technical log)

[`docs/BITACORA.md`](docs/BITACORA.md) is owned by WP-02. Other WPs **return** their
entry text in their report; the BITACORA owner inserts it at the top of the log
(newest first). Entry format:

```markdown
### YYYY-MM-DD — WP-xx <title> (phase N[, milestone])

**What changed.** Files/areas, behaviour, contracts touched (with versions).
**Decisions.** Choice, rationale, rejected alternatives; deviations from DESIGN.md.
**Automated checks.** Each command run from the repo root and its result (pass/fail).
**Physical validation.** "pending physical — not faked: <what still needs devices>",
or "verified on <devices, date, run ids>", or "not applicable".
**Follow-ups.** Change requests filed, open issues.
```

Automated checks and physical validation are always separate. Nothing that needs a
phone, helmet, vehicle or road is reported as verified until it ran there. A finding
that must never regress goes into the **Invariants — DO NOT REVERT** section.

## 13. Definition of done (every WP)

1. Owned-path code and tests; `cargo fmt` + `clippy -D warnings`, or `pnpm lint`,
   clean.
2. The WP's `verify` commands green from the repo root on a clean checkout (after
   `scripts/bootstrap.sh`) for the milestones of the current phase.
3. Contract artifacts regenerated with no drift.
4. A BITACORA entry that separates automated checks from physical validation.
5. The WP report lists deviations from DESIGN.md, change requests to other owners,
   and what remains device-only.
