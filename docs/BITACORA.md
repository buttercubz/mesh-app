# BITACORA — mesh-app technical log

> **Purpose.** The record of technical decisions as they land: what was chosen, why,
> what was rejected, how it was verified, and what must **not** be reverted.
> `docs/DESIGN.md` is the design; this log is what actually happened.
>
> **How to use it:**
> - Before changing protocol, crypto, routing, floor, audio, packaging or capture
>   gating, read **[Invariants — DO NOT REVERT](#invariants--do-not-revert)**.
> - Every work package adds one entry per delivered milestone at the **top** of the
>   [Chronological log](#chronological-log) (newest first), in the format below.
> - Constants live in the code and DESIGN Appendix A; this log explains the reasoning.
> - Automated checks and physical validation are always reported separately. Anything
>   that needs phones, helmets, vehicles or roads stays **"pending physical — not
>   faked"** until it ran there.
> - Owner: WP-02. Other WPs return their entry text in their report and the owner
>   inserts it (see `AGENTS.md` §12).

## Entry format

```markdown
### YYYY-MM-DD — WP-xx <title> (phase N[, milestone])

**What changed.** Files/areas, behaviour, contracts touched (with versions).
**Decisions.** Choice, rationale, rejected alternatives; deviations from DESIGN.md.
**Automated checks.** Each command run from the repo root and its result (pass/fail).
**Physical validation.** "pending physical — not faked: <what still needs devices>",
or "verified on <devices, date, run ids>", or "not applicable".
**Follow-ups.** Change requests filed, open issues.
```

---

## Invariants — DO NOT REVERT

Hard rules distilled from the design reviews (DESIGN Appendix C, Appendix D). Breaking
one re-opens a problem that was already closed. Changing one requires a DESIGN.md
revision by the design owner and a log entry explaining why.

### Process and repository

- **Never modify `../comvoy-app`.** It is a read-only reference.
- **Exclusive `ownedPaths`, no globs.** Changes elsewhere go through a change request
  to the owner. Generated files only via their xtask command, with drift checks.
- **pnpm only; `pnpm exec cap`, never `npx`.** `npx` can fetch an unpinned CLI.
- **`cargo xtask fetch-opus` precedes every build of `mesh-core`.** `opus` is a
  default feature; skipping the fetch breaks clean builds.
- **Physical honesty.** No device, helmet or road result is ever faked or inferred
  from simulation.

### Architecture and packaging

- **Byte framing lives only in Rust** (`mesh-wire`). Swift and Kotlin move opaque
  bytes. Per-platform deframers were rejected: they allow iPhone↔Android drift and
  escape fuzzing.
- **Sans-IO determinism in the seven core crates** (`mesh-wire|crypto|lab|voice|routing|core|sim`):
  no `std::time`, threads, `getrandom` or HashMap iteration; the `disallowed_*` lints
  are denied per crate in `lib.rs`, allowed workspace-wide (a workspace-wide deny broke
  CI for `mesh-ffi`).
- **`MeshPlatformKit` has no Rust dependency;** `MeshEngineKit` holds the binary target.
  SwiftPM must never resolve a missing xcframework.
- **App-local Capacitor bridges + TS-only plugin.** No SPM path crosses a pnpm symlink.
- **Committed generated bindings with drift checks; link data is `Vec<u8>`** (UniFFI
  `=0.32.2`; `&[u8]` hits the 0.32 Kotlin direct-ByteBuffer break).
- **WASM uses a raw `extern "C"` JSON ABI**, no wasm-bindgen; `mesh-wasm` uses
  `mesh-core`/`mesh-sim` with `default-features = false` (no opus in wasm32); no OS
  randomness in the graph (snow without defaults + custom resolver).
- **libopus 1.6.1 via the `cc` crate with SHA-256-pinned sources and weights and a
  fixed-arity ctl shim.** Rust never calls a variadic C function (Apple arm64 passes
  variadic args on the stack). No system cmake in the default build.
- **RT audio plane is a wait-free C ABI at 16 kHz mono** (`MESH_RT_ABI_VERSION 1`).
  RT callbacks never allocate, lock, log or call UniFFI. A rate change is a v2 ABI.

### Security

- **Per-link Noise `XXpsk1_25519_ChaChaPoly_SHA256` + IDPROOF.** psk1, never psk3 (it
  leaks the responder static key). Mesh headers are never on air in clear.
- **Link AEAD uses raw split keys + `chacha20poly1305` with AAD = `lf_type ‖ kp_ctr`**
  and an in-band key update with a phase bit. snow's `StatelessTransportState` has no
  AD parameter; a 12 h re-handshake conflicted with duplicate-link resolution.
- **No persisted nonce or counter state.** No persist-before-use counters.
- **Replay and dedup windows are updated only after authentication**, keyed by
  `(origin, sess)` with a random per-boot `sess`. No "far behind" restart heuristics.
- **Per-group identities** (Ed25519 + X25519); `node_id` from `id_hash`; a collision
  regenerates that group's identity. No device-wide identity, no leader-assigned ids.
- **Invites carry the current epoch secret; Lock/Remove rekey; QR auto-hides at 60 s;
  members re-share the leader-signed invite verbatim; the invite text never reaches
  JS** (the QR image is rendered natively). No secrets in web storage.
- **Retained epochs (last 4), old-epoch access for non-denied roster members with no
  time limit** until they hold the newest epoch. The 24 h grace silently locked out
  absent members of weekly groups.
- **Monotonic acceptance + link-scoped anti-entropy** for ROSTER/REKEY/GROUP_CTL/JOIN_REQ;
  older signed control state is rejected. Flood-once lost state across partitions.
- **LEAVE and RIDE_CTL are signed;** ROSTER/REKEY/GROUP_CTL are leader-signed.
  FLOOR_REJECT is not signed (a relay signature proves nothing).

### Routing and floor

- **Managed flooding with immediate voice relay; voice is never pruned.** Two-hop
  pruning applies only to ctrl/bulk on fresh, good, uncongested neighbor reports.
  Pruning voice by a rank check was unsafe (per-link stateful admission).
- **Talker cap from the explicit per-link cost model** (`cost(k) = k × r_stream + 3 kbps`,
  `cap_pacing` with the 0.9 factor, 2 s down / 5 s up hysteresis), voice-aware node
  airtime budget 180 kbps, BLE links target 2 / max 3 until T0c says otherwise.
  Proportional node scaling made the cap 1 at the link target.
- **Floor needs no wall clock.** Total rank order: priority, age-estimated start from
  burst-relative VOICE `ts` / TALK `elapsed_10ms`, then hash smear; 300 ms stickiness;
  relays enforce hard per-link caps.
- **Hop promise:** the 300 ms p95 KPI holds for ≤ 2 BLE hops; 3-4 hops are best
  effort and labelled. Never claim KPI latency beyond 2 BLE hops.

### Voice and audio

- **Opus on BLE: 16 kbps WB, 40 ms, no LBRR, `PACKET_LOSS_PERC = 0`, DTX off.** LBRR
  only at 24 kbps with ≥ 3 % loss on Wi-Fi; never LBRR + DRED together.
- **Capture gating:** `goLive`, `resumeMic`, `labStart{audio: full}` and
  `audioCheck{start}` only through `gestureCommand`; the bridge does the OS-level work
  synchronously in the tap; the core's `StartCapture` only connects/unmutes the sink.
  `StartCapture` is never emitted in Idle (property test).
- **iOS: raw VPIO AudioUnit, never stopped during a ride.** Background restart failure
  goes to CaptureLost and the output-only fallback (ladder L2), never a silent mute.
  The session is non-mixable by default.
- **iOS keep-alive = `audio` + opt-in `location`** backed by the visible distance list.
  No `voip`, no `push-to-talk` background modes.
- **Android: FGS `microphone|connectedDevice` (+`location` only when granted and
  sharing) started from a visible tap, `START_NOT_STICKY`, `MODE_IN_COMMUNICATION`,
  Oboe.** Core-Telecom stays behind an off-by-default flag; no default `MEDIA_BUTTON`
  receiver; no `ACCESS_LOCAL_NETWORK` at target 36.
- **No bundled audio assets.** Demo-ride voices are rendered on device by platform TTS
  and encoded by the bots; earcons are synthesized.
- **Talk mode defaults:** VOX for motorcycles, PTT for cars; never VOX on a
  loudspeaker route.

### Product

- **Honest range promise** (DESIGN §0.2) in onboarding and store copy; no Cardo/Sena
  trademarks; no text chat; no text input on the ride screen.
- **Lab is a visible Settings → Field test section**, never a hidden gesture; files
  stay local until the user shares them; GPS consent per run.
- **Groups persist** until forgotten or closed by the leader; no auto-expiry.
- **Leader loss recovery is "start a new group"**; transfer leadership is a signed
  handover while the leader is present. No exportable leader-key backup in v1.

---

## Chronological log

<!-- Newest entry first. Insert new entries directly below this line. -->

### 2026-10-22 — WP-02 README and AGENTS.md conventions (phase 0)

**What changed.** Added `README.md` (product, honest range promise, architecture
summary, repository map, quick start: bootstrap → `pnpm run ci` → `pnpm run dev` web
simulator → iOS device build), `AGENTS.md` (entry point for agents: ownership, phases
/ waves / milestones, contracts C1-C9 and change requests, stub hand-offs, generated
files, verify prerequisites, gesture-command rule, determinism rules, language and
tooling rules, repository boundaries, BITACORA format, definition of done), this
`docs/BITACORA.md` with the invariants seeded from DESIGN Appendix C, and
`docs/conventions/{rust,swift,kotlin,typescript,commits}.md`.
**Decisions.** The Rust channel is documented as `1.99.1` (orchestrator decision that
supersedes the provisional 1.89.0 pin of DESIGN §2.6; `scripts/msrv-probe.sh` probes
1.99.1). Spanish UI locale is neutral `es` (Appendix E working default). The quick
start names commands that WP-01 (`package.json` scripts `dev`/`ci`/`help`,
`scripts/tasks.mjs` groups `ios build|open`, `cap sync`, `android build|test`) and
WP-00 (`scripts/bootstrap.sh`, `doctor.sh`, `env.sh`) deliver; until they land the
README describes the designed interface (DESIGN §2.5).
**Automated checks.** `test -f README.md && test -f AGENTS.md && test -f docs/BITACORA.md`
(pass); `grep -q 'ownedPaths' AGENTS.md && grep -q 'comvoy-app' AGENTS.md && grep -q 'DO NOT REVERT' docs/BITACORA.md`
(pass); `ls docs/conventions/rust.md docs/conventions/swift.md docs/conventions/kotlin.md docs/conventions/typescript.md`
(pass); relative links and in-repo anchors in the new docs resolve (scripted check).
**Physical validation.** Not applicable (documentation only).
**Follow-ups.** Change request to WP-01: the root `dev` script must start the
`@meshapp/mobile` Vite dev server against the WASM simulator, and `tasks.mjs` must
expose `ios build`, `ios open`, `cap sync`, `android build`, `android test` as named in
the README quick start.
