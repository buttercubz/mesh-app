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

### 2026-10-24 — WP-10B / WP-09 IDPROOF retransmission on datagram links and floor property oracle fix (phase 4 / phase 1)

**What changed.** `crates/mesh-core/src/secure/admit.rs`: on datagram links (Wi-Fi Aware; Tunnel is a stream unit) IDPROOF is re-sent every 250 ms (`IDPROOF_RESEND_US`, at most `IDPROOF_MAX_RESENDS` = 20) while the link is in IdProof; the 5 s handshake timeout still closes a link that never completes. An Up datagram link answers a repeated, valid IDPROOF of the identity it already admitted (gate `IDPROOF_ANSWER_GAP_US` = 125 ms, at most 20 answers, only until the peer's CAPS arrives); the CAPS after such an answer re-runs the link-up announcements (JOIN_REQ if not yet in the roster, PRESENCE). SYNC/MESH records that arrive while a datagram link is still in IdProof are kept (at most 8 records / 4 KiB, `LinkRt.early` in `group/active.rs`) and replayed once Up; never processed before admission. S16 runs with the default ±2 ms Aware jitter (workaround removed). `crates/mesh-routing/tests/routing/floor_props.rs`: `caps_hold_even_if_talkers_ignore_rejects` now probes just after each transmission instant and needs a strictly later frame to count a burst as streaming; pinned regression `caps_hold_regression_seed_14347263198455711262`. No contract changes.
**Decisions.** Resend instead of buffering the early SEC ciphertext (that would live in the WP-10A link layer). Invalid or mismatching IDPROOFs on Up links are ignored: no reply, no close, no second admission. The router was correct in the flaky seed: a relay forwarding the victim's last frame and evicting it in the same instant is a §6.3 handover; the old oracle counted it as two streams. Loosening the threshold or skipping the seed was rejected.
**Automated checks.** `cargo test --release -p mesh-core --test secure` — new tests (`aware_idproof_overtaking_the_last_handshake_message_still_completes`, `aware_lost_responder_idproof_is_answered_and_the_link_comes_up`, `aware_join_req_overtaking_its_idproof_is_replayed_once_up`, `aware_repeated_idproof_with_shrinking_delay_is_answered_every_period`) fail with each fix reverted, pass with it; `cargo test --release -p mesh-routing -p mesh-core -p mesh-sim` pass; `mesh-sim suite ci --seeds 8 --only S16` 8/8; `PROPTEST_CASES=20000 cargo test --release -p mesh-routing --test routing caps_hold` pass; `cargo clippy --workspace --all-targets -- -D warnings` pass; `cargo fmt --all --check` pass.
**Physical validation.** pending physical — not faked: IDPROOF recovery and early-record replay on real Wi-Fi Aware links between Android phones.
**Follow-ups.** `random_contention_with_per_link_caps` fails at `PROPTEST_CASES=3000` ("top burst rejected", seed 4230068088296886744, n = 11, caps [1], starts [(808, 12), (497, 0)]); not yet investigated.

### 2026-10-24 — WP-10C / WP-10D / WP-09 / WP-11 hybrid Wi-Fi Aware backbone with BLE edges (phase 4)

**What changed.** Per-segment talker caps: the near-talker cap is 4 from the Aware backbone and 2 inside BLE segments. Edge links (BLE hops of a hybrid group) admit purely by rank (`admit_edge` in `crates/mesh-routing/src/floor/admission.rs`): the top `cap(L)` live bursts are carried, refused or displaced bursts wait in a `waiting` set and return by rank, queue-shed bursts stay sticky but keep their rank slot; edges shed locally without `FLOOR_REJECT`, and `on_downgrade` (`mesh/upgrade.rs`) only sheds on edges. `cap_local` (`Media::cap_local`, `Router::cap_local`) ignores edge links and BLE fallback links to a neighbour also reached over Aware (`Router::is_ble_fallback_link`). `media/inputs.rs`: BLE profiles apply whenever any Up voice-egress link is BLE, fallback links included, because relays never transcode. New sim scenarios S16 (hybrid backbone + BLE edge), S16b (BLE edge at 64 kbps, talkers A1/A3/A2, silent leader) and S16c (star on hub A1).
**Decisions.** Literal §7.4 BLE16 rule restored over the narrower "a BLE link carries my voice" rule; consequence: Android groups that keep BLE fallback links stay on BLE16/BLE12. Edge admission drops the `T_STICKY` first-come rule (non-edge links unchanged). S16b uses 64 kbps rather than 60 (no pacer headroom for cap 2 at exactly 60). S16c uses the hub as creator and a second `FORM_LIMIT_MS` window (formation sometimes took ~60.25 s with A0 as leader).
**Automated checks.** `cargo test --release -p mesh-routing -p mesh-core -p mesh-sim` pass (incl. `edge_cap_dip_and_recovery_restores_top_by_rank`, `ble_fallback_to_my_only_aware_peer_does_not_bound_cap_local`); `mesh-sim suite ci --seeds 2` 28/28; `--seeds 8 --only S16b` and `--only S16c` 8/8 each (S16b fails 3/4 seeds with the reviewed profile rule; S16c fails 2/2 without the cap_local fix); clippy `-D warnings` and `cargo fmt --all --check` pass.
**Physical validation.** pending physical — not faked: hybrid Aware/BLE edge behaviour on Android phones.
**Follow-ups.** An origin with only Aware links still encodes WIFI20 and its stream may cross a BLE edge elsewhere; candidate fix (edge relays flood `PATH_HINT{BLE16}` to the origin) needs a design decision. Formation over three BLE hops or around a hub needing a third BLE link can exceed `FORM_LIMIT_MS`.

### 2026-10-24 — WP-03 / WP-07 / WP-10B / app open channels 1-9 (phase 2)

**What changed.** Public open channels 1-9 joined without an invite. `crates/mesh-crypto/src/open_channel.rs`: `group_id` and epoch-0 secret derived by HKDF from a fixed public context and the channel number (group ids pinned by a test). `crates/mesh-core/src/group/open.rs`: an open channel is a `Group` in a restricted mode — no leader (ROSTER/REKEY/GROUP_CTL ignored, nobody can remove/lock/close, a peer `REJECT{5}` never ends the session), fresh identity per session, names from signed JOIN_REQ, voice TTL 2, talker cap 2 (`OPEN_TALKER_CAP`), per-origin packet budget, at most 64 remembered identities, nothing persisted. Local mute/block (never sent; also in private groups). C1 `API_VERSION` 1.1 → **1.2** (`joinOpenChannel`, `setMemberMuted`, `setMemberBlocked`, `session.openChannel`, member flags `muted`/`blocked`; frozen digest `api-1.2.digest`); FFI dispatch, plugin and app (`features/open/`, member actions, i18n ES/EN) updated.
**Decisions.** The channel key is not a secret: it only separates channels and keeps casual observers out; per-link Noise still gives forward-secret transport keys. Pulled forward from v1.x (DESIGN §0 scope table).
**Automated checks.** `cargo test --release -p mesh-core --test group` (open-channel tests) and `-p mesh-crypto` pass as reported by the implementing agents; app unit tests (`OpenChannel.test.tsx`) per the app package gate. Not re-run by WP-02.
**Physical validation.** pending physical — not faked: strangers on the same channel finding each other on the road; mute/block on devices.
**Follow-ups.** None filed.

### 2026-10-24 — WP-09 / WP-10D / WP-14 / WP-16 range at the edge (phase 1)

**What changed.** Android: `setPreferredPhy(LE 1M)` on client and server links (≈3-5 dB better sensitivity than 2M). iOS: the filtered scan is refreshed every 4 s so a peer coming back into range is rediscovered. `crates/mesh-core/src/links/radio.rs`: an established link that dies (supervision timeout, link death, platform error) gets a 3 s settle hold without backoff growth; failed connects keep 5 → 60 s doubling. `crates/mesh-routing/src/pacer.rs`: recovery below the initial rate needs only a period without a congestion signal (`qd ≤ 40 ms`, no `WouldBlock`) instead of `qd < 15 ms`; queue delay uses `min(srtt, sample)`. Mid-burst BLE16 → BLE12 downshift when the link degrades. Opus LBRR enabled on BLE16 at the edge when smoothed loss ≥ 3 % (off after 10 s below 2 %). New mesh tests `tests/mesh/fading.rs`, sim S15.
**Decisions.** A dead established link is a walk-away at the fringe, not a failed connect. Recovery does not wait for `qd < 15 ms`: on a 30 ms iOS connection interval the RTT phase alone keeps `srtt − min_rtt` at 15-30 ms, so a link cut once stayed at the floor for the whole ride.
**Automated checks.** `cargo test --release -p mesh-routing -p mesh-core -p mesh-sim` pass (pacer, rejoin, fading tests); `mesh-sim suite ci --seeds 2` pass incl. S15; clippy and fmt clean (as reported in the fix agents' runs).
**Physical validation.** pending physical — not faked: edge-of-range rejoin time, PHY gain and LBRR benefit on iPhone/Android pairs on the road.
**Follow-ups.** None.

### 2026-10-24 — WP-09 / WP-10C / WP-10D / app false "Network busy" fix and weakSignal banner (phase 1)

**What changed.** Field test 2026-10-24: a lone talker at the edge of range got `FLOOR_REJECT{CONGESTION}` ("Network busy"). `crates/mesh-routing/src/pacer.rs`: the multiplicative decrease never goes below one stream of the lowest profile (`PacerPath::voice_floor_bps`: BLE12 `cost(1)/0.9` = 25 112 bps on BLE, 41 556 bps on Aware/Tunnel). `cap.rs`: a BLE12 grant counts as cap 1. A lone admitted burst is never shed by the queue. A stale reject reason is cleared when the burst ends or is re-admitted. Member quality in the snapshot is now the real link quality (`quality.rs`, `session/view.rs`), and the ride screen shows a `weakSignal` banner (`useRideBanners.ts`, i18n ES/EN) instead of "Network busy" when the cause is fading.
**Decisions.** Fading is not congestion: below one stream our own demand is not what builds the queue, and pacing below it only guarantees silence. Congestion from too many streams is still shed above the floor (cap 1 at the floor).
**Automated checks.** `cargo test --release -p mesh-routing` (pacer, queue, cap tests) pass; `cargo test --release -p mesh-core -p mesh-sim` pass; app tests (`RideScreen.test.tsx`, `model.test.ts`) per the app gate. Results as reported by the implementing agents.
**Physical validation.** pending physical — not faked: lone talker at the edge of range on the road no longer shows "Network busy"; weakSignal banner timing.
**Follow-ups.** None.

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
