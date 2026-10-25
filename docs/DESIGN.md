# mesh-app — Final Architecture and Design (v1)

- **Status:** authoritative design, 2026-10-22. Nothing is implemented yet. `docs/WORKPLAN.json` holds the machine-readable work packages derived from §16.
- **Basis:** proposal A ("delivery-first, vertical slices", highest overall score) with grafts from proposals B (protocol rigor) and C (rider experience and platform reality). Every must-fix from the three reviews is resolved; Appendix D maps each one to the section that fixes it.
- **Revision 2 (2026-10-22):** resolves the adversarial architecture review (2 blockers, 12 majors, 10 minors, 12 missing items, 19 work-plan issues). Appendix D.2 maps each finding to its fix; where the review was only partly adopted, Appendix C says why.
- **Inputs:** `docs/research/INVESTIGACION.md`, where verification corrections override the original claims, the raw and verified research JSON, and a read-only map of the sibling `comvoy-app` (convLink). **Never modify `comvoy-app`.**
- **Language rule:** code, comments and docs are in English. The UI ships ES and EN.
- **Tag legend:** **[ENV]** can be built and verified on this Mac (no devices). **[DEV]** needs the user's phones. **[TUNE]** is a default the Lab benches may change (Appendix B).

### 0.1 Locked user decisions (constraints, not options)

| # | Decision |
|---|---|
| L1 | Phone-only: no ESP32/LoRa relays, no other external devices. iPhones and Android phones must interoperate in the same group. |
| L2 | Voice is VOX (motorcycle default) plus PTT. Relays enforce a hard simultaneous-talker cap: 2 on BLE paths, 4 on Wi-Fi paths. Talkers over the cap hear a busy tone. No all-open-mic in v1. |
| L3 | Shared core in Rust, sans-IO, deterministic and simulatable. UniFFI is the control plane (Kotlin and Swift). A plain C ABI with wait-free SPSC rings is the real-time audio plane. libopus 1.6.1 (float, deep PLC, OSCE) is built inside the core. |
| L4 | UI is Capacitor 8, React 19, TypeScript, Zustand and Tailwind v4, with pnpm only. The UI is a thin client over a UI-independent native engine. A WASM build of the core backs the web implementation, so the UI can be developed in a browser against a simulated convoy. |
| L5 | Native platform layer per OS: a Swift package (iOS) and a Kotlin Android library. They cover BLE central + peripheral + L2CAP CoC, optional Wi-Fi Aware, audio I/O, and background service hosting. |
| L6 | Audience: motorcycles with Bluetooth helmet headsets first; cars too. |
| L7 | Group formation: a pre-ride QR lobby with phones unlocked, plus optional Wi-Fi Aware PIN pairing between iPhones. |
| L8 | v1 is 100 % offline. Leave an explicit v2 cellular transport hook but do not build it. |
| L9 | English code and docs; ES + EN UI. Group size 2-15, with protocol headroom of at least 31. |
| L10 | Test devices are several iPhones (12+, iOS 26/27) and no Android device. Android must build and pass unit tests. iPhone↔iPhone is the first field path, and the cross-platform protocol must not be compromised by that. |

### 0.2 Honest product promise (shown in onboarding and in the store listing)

> "A compact group intercom that uses only your phones. Riders reach each other directly over tens of metres, and the voice is relayed through the other riders' phones. It is not a 1.6 km dedicated mesh headset."

Store metadata never uses Cardo or Sena trademarks. The ride screen has no text input. The copy says riders must not operate the phone while moving, and Lab markers are for passengers or stops.

### 0.3 Architecture overview

```
 ┌──────────────────────────────── apps/mobile (Capacitor 8.5 WebView) ───────────────────────────────┐
 │ React 19 + Zustand + Tailwind v4 + i18next (ES/EN)                                                   │
 │ screens: Onboarding · Home · Group(create/join) · Lobby · Ride · Members · Settings · Field test(Lab)│
 │        │  typed client (@meshapp/engine-api, ts-rs generated)                                         │
 │        ▼                                                                                              │
 │ @meshapp/capacitor-intercom (TS-only plugin: definitions + registerPlugin + web.ts)                   │
 └────────┬──────────────────────────────────────────┬─────────────────────────────────┬───────────────┘
          │ native bridge (app-local)                 │ native bridge (app-local)        │ web (browser)
          ▼                                           ▼                                  ▼
 iOS: MeshIntercomPlugin.swift            Android: MeshIntercomPlugin.kt        @meshapp/engine-web
   │                                        │                                     raw extern"C" WASM ABI
   ▼                                        ▼                                     mesh-wasm = core + sim
 MeshEngineKit (Swift, Rust bridge)       meshengine (Kotlin library)            (SimCodec, no libopus)
   EngineHost implements PlatformHooks      EngineHost implements PlatformHooks
   │        ▲ UniFFI (control plane)        │        ▲ UniFFI/JNA (control plane)
   │        │ Vec<u8> link data, JSON UI    │        │
   ▼        │                               ▼        │
 MeshPlatformKit (Swift, NO Rust)          ble/ audio/ service/ (Kotlin)
   BLE central+peripheral, L2CAP, GATT       BLE scanner/advertiser/GATT/L2CAP
   CMeshAudio: raw VPIO AudioUnit ───┐       Oboe C++ shim ───────────────┐
   keep-alive (audio+location),      │       FGS microphone|connectedDevice│
   Live Activity, Lab file store     │       (+location when permitted)    │
                                     │  C ABI mesh_rt_* (wait-free rtrb SPSC rings, no UniFFI on RT threads)
                                     ▼                                     ▼
 ┌────────────────────────── libmesh_ffi (Rust: staticlib iOS / cdylib Android / host dylib) ─────────┐
 │ mesh-ffi runtime: engine thread (5 ms tick) · MPSC inbound · RT rings · demo world · panic guard     │
 │   └─ mesh-core (sans-IO Engine: links, group, session, media, mesh, lab, ui)                         │
 │        ├─ mesh-wire   framing, records, mesh header, golden vectors                                   │
 │        ├─ mesh-crypto Noise XXpsk1 per link, IDPROOF, invite, key schedule, roster, REKEY             │
 │        ├─ mesh-routing neighbor table, managed flooding + 2-hop pruning, pacing, floor, presence      │
 │        ├─ mesh-voice  HPF, VAD, talk FSM, Opus (opus-sys, cc-built 1.6.1), jitter buffer, mixer, tones│
 │        └─ mesh-lab    benches, stats, clock estimator, labrun/1 records                               │
 │ mesh-sim (deterministic N-node world) · mesh-api (UI JSON contract → TS)                              │
 └──────────────────────────────────────────────────────────────────────────────────────────────────────┘
 Radios: BLE L2CAP CoC (all platform pairs, iPhone central toward Android) · GATT fallback ·
         Wi-Fi Aware upgrade (iPhone↔iPhone PIN-paired, Android↔Android) · Tunnel (v2 hook only)
```

### 0.4 Decisions at a glance

The full rationale and the rejected alternatives are in Appendix C.

| # | Area | Choice |
|---|---|---|
| D1 | First field path | iPhone↔iPhone over BLE L2CAP CoC. The Phase 0 Lab runs on the production Rust link framing and the production Swift drivers. |
| D2 | Link security | Per-link Noise `XXpsk1_25519_ChaChaPoly_SHA256` with IDPROOF channel binding. Mesh headers are never on air in clear. |
| D3 | Identity | Per-group Ed25519 + X25519 keys. `node_id` is u16 from the per-group `id_hash`. A collision is fixed by regenerating that per-group identity, with no contradiction because identities are per group. |
| D4 | Routing | Stateless managed flooding, immediate voice relay, dedup per `(origin, sess, seq)` after authentication. Voice is **never pruned** (its fan-out is bounded by the BLE link max); 2-hop pruning applies only to ctrl/bulk classes and only on fresh "good, uncongested" neighbor reports. Signed control state (ROSTER, REKEY, GROUP_CTL, JOIN_REQ) is kept consistent by link-scoped anti-entropy (§8.9). |
| D5 | Floor | Optimistic: talk immediately. Strict total rank order: priority, estimated origin start from age + burst-relative time (no wall clock), then hash smear. Per-link hard caps at relays, a 300 ms stickiness window, FLOOR_REJECT and busy tone. BLE cap 2 is sized from the per-link budget (§5.6), with hysteresis. |
| D6 | Codec | libopus 1.6.1 float built with the `cc` crate (no CMake). BLE: 16 kbps WB, 40 ms, no LBRR, `PACKET_LOSS_PERC=0`, DTX off. Wi-Fi: 20 ms at 20 kbps, or 24 kbps with LBRR at ≥3 % loss. DRED compiled in but off. |
| D7 | iOS audio | Raw VPIO AudioUnit, never stopped during a ride. The native bridge performs the OS-level, gesture-bound start (session activation, VPIO start muted, FGS microphone) synchronously inside the tap; the core only connects and unmutes the RT sink (§3.2). Restart failures in the background go to CaptureLost, then an output-only fallback. Degradation ladder L0-L3. |
| D8 | iOS keep-alive | Two anchors: `audio`, plus `location` (CLBackgroundActivitySession) when the rider opts into "Share my distance". The location anchor is justified by the user-visible distance list. |
| D9 | Android audio | FGS `microphone\|connectedDevice` (+`location` only when granted and sharing), started from the visible Go-live tap, `START_NOT_STICKY`, `MODE_IN_COMMUNICATION` + `setCommunicationDevice`, Oboe VoiceCommunication. Core-Telecom stays behind an off-by-default strategy flag. |
| D10 | Packaging | A Rust-free `MeshPlatformKit` plus the `MeshEngineKit` bridge, so SwiftPM never resolves a missing xcframework. App-local Capacitor bridges and a TS-only plugin, so no SPM path crosses a pnpm symlink. |
| D11 | Tooling | Everything builds here: `cc` for libopus, a workspace `uniffi-bindgen` binary, a raw `extern "C"` WASM ABI (no wasm-bindgen), cargo-ndk (installed) with an NDK-linker fallback. Generated bindings are committed with drift checks. |
| D12 | Scope | v1 = private groups, VOX/PTT, multi-hop, honest reachability, distance list (opt-in), earcons + pre-rendered spoken prompts, Lab, Demo ride, remove/lock + rekey, signed leader handover. No co-leaders, member policies, CRDT roster, Telecom default or Control Center widgets in v1. |

## 1. v1 product scope, user flows, default modes

Design principle (from C): **a rider is never mute or deaf without knowing it, and after the lobby needs one tap and then the pocket.** Every failure produces three things: a helmet cue (an earcon, plus a spoken prompt where enabled), a lock-screen state (Live Activity or the FGS notification), and a PRESENCE flag the other riders see.

### 1.1 In scope (v1)

| Area | v1 feature |
|---|---|
| Groups | Private groups of 2-15 riders (protocol roster ≤ 32 entries, bitmap u32). One active group per device and many saved groups. Groups persist until the user forgets them or the leader closes them. There is no auto-expiry. The leader can set an optional invite expiry. |
| Joining | The leader creates the group, and members scan the QR (or open the `convlinkvoice://join#…` link) in the lobby with phones unlocked. Any member can show the current-epoch QR; members re-share the **leader-signed invite blob verbatim** (§8.3), and joiners cross-check the invite's leader key against the first verified ROSTER. The QR auto-hides after 60 s and warns that a screenshot is membership. |
| Voice | VOX (motorcycle default), PTT (car default) with hold or latch (in-app button or the Live Activity latch), and Mute. Talker cap 2 on BLE paths and 4 on all-Wi-Fi paths, enforced at relays. Leader priority (preempts the lowest-ranked burst). Busy, preempt, go-ahead and ride-started earcons. |
| Mesh | Multi-hop managed flooding. Voice meets the 300 ms p95 target (MCPTT KPI3) at **≤ 2 BLE hops**; 3-4 hops are relayed **best effort** inside a 150 ms network-age cap (usually 3 hops at most on BLE) and the UI labels them so. Automatic rejoin. Per-member states: direct, relayed n hops, one-way, lost ("last heard 40 s"), unconfirmed. "Heard by k of n", islands, "You are alone" cue, aggregated lost/back cues. |
| Audio routes | Helmet HFP (CVSD NB, mSBC WB; Android LE Audio when present), car HFP, speaker (car profile), wired. The motorcycle profile mutes TX when the only mic is the phone's built-in mic (wind); the rider can override this in Settings. |
| Interruptions | Phone calls, Siri, other apps: auto-resume when the OS allows. Peers see "on a call" or "mic paused". Output-only fallback keeps the rider hearing the group. |
| Lock screen | iOS Live Activity (status, speakers, in-range count, Mute, **Talk (PTT latch)**, Resume mic). Android ongoing notification (Mute, Talk latch, Leave, Resume). The screen stays awake in the lobby and on the ride screen in PTT mode (car profile: "keep screen on while mounted", opt-out). |
| Distances | Opt-in "Share my distance with my group": a member distance and bearing list ("Ana · 140 m behind") from POSITION packets. The same opt-in is the iOS `location` keep-alive anchor. No map in v1. |
| Field test (Lab) | A visible Settings → Field test section with benches T0, T0b, T0c, T1, T2, T3, T5a/T5b, T6, T8 and VOX calibration (§12.2), each with an explicit audio path and keep-alive anchor, a passive ride recorder, a latency estimator, and JSONL export through the share sheet. |
| Demo | Single-device Demo ride (4 bot riders speak ES/EN lines rendered on device by the platform TTS at Demo start and Opus-encoded by the bots, so no audio assets are bundled; they go through the real audio, floor and background path) and an Echo test (hear yourself after 1 s, `audioCheck{echo}`). Used for onboarding and App Review 2.5.4. |
| Security lifecycle | Remove member, Lock group (rekey that invalidates old QRs), Close group, Transfer leadership (signed handover). These ship in Phase 2, not deferred to hardening. If the leader's phone is lost, nobody can remove or lock: the documented recovery is "start a new group" from any member's phone (one lobby scan). |
| Wi-Fi Aware | Optional "Faster link (Wi-Fi)" for PIN-paired iPhones and for Android↔Android (Phase 4, if the entitlement is granted). |
| i18n | UI ES + EN with compile-time key parity. The engine emits i18n keys plus parameters, never English strings. Spoken prompts are localized. |

### 1.2 Out of scope (v1)

| Item | Where it goes |
|---|---|
| Group map | v1.1 (reuses POSITION data; convLink MapLibre code is the reference) |
| Cellular fallback (Supabase/WebRTC) | v2 via `LinkKind::Tunnel` (§9.8) |
| Open-mic for all | Never on BLE; maybe small all-Wi-Fi groups later |
| Co-leaders, member policies (force-PTT, remote mute), CRDT roster, leader-approval admission | v1.x, only if field use asks for them |
| Helmet-button / handlebar PTT, iOS PushToTalk framework, Control Center widget, AudioRecordingIntent, Action-button latch | v1.x evaluation |
| Android Core-Telecom "ride call" default, MEDIA_BUTTON restart receiver | Behind an off-by-default strategy flag until device-validated (§10.2) |
| Public/open channels | **Pulled into v1** (2026-10-24): channels 1-9, public HKDF-derived key, no leader, cap 2, TTL 2 (Appendix C) |
| Per-burst Ed25519 voice signatures, MLS | v1.x/v2 |
| Android-hosted hotspot topology | Not planned unless T1/T4 demand it |
| convLink accounts / shared identity | v2 integration |
| Text chat | Never (convLink product rule) |

### 1.3 User flows (screen → engine command)

**F0. First launch (once, about 90 s, foreground).**
1. Language, display name (≤ 24 UTF-8 bytes, truncated at a grapheme boundary), vehicle profile (Motorcycle / Car).
2. Permission walkthrough, each with a one-line reason, all requested in the foreground (iOS silently denies background prompts). iOS: Microphone → Bluetooth → Notifications → Camera (when the QR scanner opens) → Location (only if the rider turns on "Share my distance"). Android: Nearby devices → Microphone → Notifications → Location (only with sharing). The battery-optimisation exemption is requested **after the first successful ride**, with OEM-specific guidance.
3. **Headset check** (skippable): "Turn on and pair your helmet now." The tap calls `gestureCommand({type:"audioCheck", op:"start", mode:"tone"})`: the bridge activates a temporary session inside the gesture, the engine plays a tone, records 2 s, and streams `audioCheckSample{level, vadP, gate, route, inputRate}` so the screen shows the level and detected route ("HFP 16 kHz wideband" or "HFP 8 kHz narrowband"), plus headset tips (Sena: turn off "Audio Multitasking" to keep HD Voice). Offers the Echo test (`mode:"echo"`). `audioCheck{op:"stop"}` closes the temporary session.
4. "Do not swipe the app away during a ride: it cannot restart the mic by itself."

**F1. Create group (leader).** Home → Create → name + vehicle → `createGroup{name, vehicle}`. The engine creates `group_id` (16 B), `epoch_secret` (32 B) for epoch 0 and the leader's per-group identity, stores them in Keychain/Keystore, and returns. Then the lobby opens.

**F2. Join (member).** The system camera opens `convlinkvoice://join?v=1#<invite>`, or Home → Join → in-app scanner (`scanInviteQr`) → `parseInvite{url}` → confirm sheet ("Join 'Sunday ride'?") → tap Join → `joinGroup{url, displayName}` → lobby.

**F3. Lobby (all phones unlocked, 1-3 min).**
- Opening the lobby is a tap, so it calls `startEngine({groupId})` (audio stays off until Go live). Radios and the engine thread start (Android: FGS typed `connectedDevice`). The lobby screen calls `setKeepScreenOn({on:true})` (iOS `isIdleTimerDisabled`, Android `FLAG_KEEP_SCREEN_ON`) so phones do not lock and suspend before Go live, when no audio anchor exists yet. Links form while screens are on, which is the reliable iOS↔iOS moment.
- The member list fills in with per-member **ready checks** from PRESENCE: link ✓, mic ✓ (after Go live), headset route, battery %, OS. The leader also sees "voice reaches N of M", which differs from merely reachable.
- The QR (leader or any member) shows full-width, auto-hides after 60 s, and has a "Show again" button.
- Optional "Pair Wi-Fi" (Phase 4, iPhones).
- Each rider taps **Go live**: `goLive()`. This is the explicit consent for App Review 2.5.14 and the product's start of capture (iOS VPIO / Android FGS `microphone`); the only other capture starts are the user-tapped `resumeMic`, Field-test runs with audio, and the onboarding/Settings audio check (§3.2). Capture starts **muted** and the screen says so: "Mic ready, muted until the ride starts". The Live Activity and the FGS notification show "muted".
- **Unconfirmed members:** a member whose JOIN_REQ has not reached the leader yet shows "unconfirmed". JOIN_REQ is retransmitted on every new link and every 30 s (§8.9), so this clears as soon as a path to the leader exists.
- The leader taps **Start ride** → `rideControl{op:"start"}` floods RIDE_CTL. Every live rider switches to its default talk mode and hears the "ride started" cue plus the prompt "Intercom live, 5 riders". A member who joins late, or whose leader is out of range, can tap "Start" locally.

**F4. Ride (phone locked or mounted).** VOX: just talk. PTT: hold or latch the 160 px button, or toggle the **Talk** latch on the Live Activity / Android notification (`LatchTalkIntent`, §10.1.5). On the ride screen in PTT mode the screen stays awake (car profile default "keep screen on while mounted", opt-out in Settings); in VOX mode the phone may lock. When the channel is full the rider hears busy; when a slot frees while PTT is still held, the go-ahead chirp plays. Helmet cues cover member lost/back (aggregated over 5 s), alone, mic paused/back, headset lost (TX muted), Bluetooth off / airplane mode, battery 15 % and thermal throttling.

**F5. Interruptions.** On a phone call the intercom pauses and peers see `ON_CALL`; it resumes when the OS allows. If capture cannot restart in the background, the rider stays at ladder level L2: still hearing the group, a Live Activity "Resume mic" button, a time-sensitive notification, and the helmet prompt "Mic paused, tap the phone to resume" (§10.1.4).

**F6. Partition and rejoin (automatic).** Links drop, members turn "lost" with the last-heard time, and discovery switches to Forming. When riders are back in range, links re-form and the "back" cue plays. Nothing to tap.

**F7. Process death.** On the next launch the last active group is restored and the app offers a one-tap **Resume ride**. A gesture is required because neither OS starts the mic from the background.

**F8. Leave / end.** "Leave" (confirm) floods LEAVE → `stopEngine()`; iOS deactivates with `.notifyOthersOnDeactivation`, Android stops the FGS and calls `clearCommunicationDevice()`. The group stays saved for the next ride. The leader's "End ride" floods `RIDE_CTL{end}`: members get a prompt and are never auto-stopped. The leader's "Close group" floods a signed CLOSE, and members mark the group closed.

**F9. Remove member / Lock group / Transfer leadership (leader, Phase 2).** Members → Remove → new signed ROSTER with a deny entry + REKEY (§8.6). "Lock group" rekeys without removing anyone, so photographed or old QRs stop working. Before either runs, the leader sees the **unconfirmed members who would lose access** (no roster entry, so no sealed REKEY) and can wait for them ("Confirm reachable members first") or proceed. Roster members who are absent keep access across any number of later rides until they pick up the new epoch (§8.6). The removed member sees "You were removed from <group>". "Make leader" hands leadership to a present member with a signed GROUP_CTL (§8.6).

### 1.4 Default modes per vehicle profile

| Setting | Motorcycle (helmet) | Car |
|---|---|---|
| Talk mode after Start ride | **VOX**, sensitivity Medium | **PTT** (hold). VOX is offered only when the route is car HFP or a headset **and** the onboarding/Echo AEC check passed. Never VOX on a loudspeaker route. |
| PTT style | Hold, latch selectable (gloves) | Hold |
| TX with built-in mic only | **Muted** ("Phone mic off: wind"); can be overridden | Allowed (VPIO/platform AEC) |
| Output without headset | Speaker allowed (useful when stopped) | Car HFP > speaker (`defaultToSpeaker`) |
| Route preference | BT headset (LE Audio > HFP) | Car BT (HFP) > speaker |
| Share my distance | Offered in onboarding with the keep-alive explanation; the rider chooses; can be turned off at any time | Same |
| Spoken prompts | On (names, mic/headset state) | On |
| Leader priority | On (group policy) | On |
| Screen during the ride | Awake in PTT mode only; may lock in VOX | **Keep screen on while mounted** (on by default, opt-out); PTT also reachable from the Live Activity latch |

Rationale: VOX with loudspeaker playback in a cabin is the worst AEC case (research §3.4). The motorcycle built-in-mic mute avoids wind-triggered VOX hogging the channel.

## 2. Monorepo layout and build tooling

### 2.1 Repository

`mesh-app` becomes its own git repository, a sibling of `comvoy-app`. The parent `loRa` repo currently shows it as untracked, so WP-01 asks the user before `git init`. Package manager: **pnpm only** (`packageManager: "pnpm@11.5.3"`, `engines.node: ">=22.18"`). Rust is pinned by `rust-toolchain.toml`.

```
mesh-app/
├─ package.json                 # root scripts only (dev, build, test, lint, ci, help) → scripts/tasks.mjs
├─ pnpm-workspace.yaml          # packages: apps/*, packages/*, tools/* ; minimumReleaseAge + overrides (convLink style)
├─ tsconfig.base.json  eslint.config.js   # shared TS/ESLint base (WP-01); each package/app extends it
├─ Cargo.toml                   # [workspace] members = ["crates/*", "tools/xtask", "tools/uniffi-bindgen"], resolver "3",
│                               #   [workspace.dependencies] exact pins (§2.6), [profile.*]
├─ Cargo.lock
├─ rust-toolchain.toml          # channel pinned by WP-00 after the MSRV probe (§2.6; "1.99.1"); targets
│                               #   aarch64-apple-darwin, aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-linux-android,
│                               #   x86_64-linux-android, wasm32-unknown-unknown; rustfmt, clippy
├─ .cargo/config.toml           # alias xtask; NDK linker fallbacks (§2.3); wasm32 getrandom cfg
├─ clippy.toml                  # disallowed-methods/types list (determinism); the lint is denied only in sans-IO crates (§3.1)
├─ uniffi.toml                  # Kotlin package com.convlink.voice.core, Swift module MeshCoreBindings / ffi MeshCoreFFI
├─ crates/
│  ├─ mesh-api/                 # UiCommand/UiEvent/Snapshot/Lab* serde types → ts-rs (no logic)
│  ├─ mesh-wire/                # link framing, GATT chunking, LinkFrame, records, mesh header, packet codecs, BLE constants,
│  │                            #   golden vectors (vectors::generate)
│  ├─ mesh-crypto/              # identities, invite codec, key schedule, Noise wrapper, link AEAD, replay window,
│  │                            #   IDPROOF, roster/JOIN/REKEY signing and sealing
│  ├─ mesh-lab/                 # bench generators, stats, clock-offset estimator, loopback correlator, labrun/1 records
│  ├─ opus-sys/                 # libopus 1.6.1 built with `cc` + fixed-arity ctl shim (no CMake, no variadic FFI)
│  ├─ mesh-voice/               # Codec trait (Opus | SimCodec), HPF, VAD, talk FSM, packetizer, jitter buffer, mixer,
│  │                            #   limiter, earcons, prompt bank
│  ├─ mesh-routing/             # neighbor table, link quality, pacing, airtime budget, flood/dedup/TTL/age/pruning,
│  │                            #   floor table, presence, reachability
│  ├─ mesh-core/                # sans-IO Engine: links, group, session, media, mesh glue, lab, ui
│  ├─ mesh-sim/                 # deterministic discrete-event N-node simulator, radio/mobility models, scenarios, demo world
│  ├─ mesh-ffi/                 # runtime (engine thread, MPSC, RT rings), UniFFI exports, C ABI (include/mesh_rt.h)
│  └─ mesh-wasm/                # raw extern "C" JSON ABI over mesh-core + mesh-sim (SimCodec)
├─ tools/
│  ├─ xtask/                    # cargo xtask: fetch-opus | opus-smoke | build-ios | build-android | build-wasm |
│  │                            #   gen-bindings | gen-ts | vectors | check | size
│  ├─ uniffi-bindgen/           # fn main() { uniffi::uniffi_bindgen_main() } — no global install
│  └─ lab-analyze/              # Node CLI: merge per-device labrun files by runId, verdicts vs thresholds.json, CSV
├─ third_party/opus/            # SHA256SUMS (committed); opus-1.6.1/ sources (gitignored, fetched by xtask)
├─ test-vectors/                # golden wire/crypto vectors (JSON, generated by `cargo xtask vectors`, committed)
├─ fuzz/                        # cargo-fuzz targets (excluded from workspace; nightly)
├─ native/
│  ├─ ios/MeshPlatformKit/      # Swift package, NO Rust dependency: BLE, CMeshAudio (VPIO), keep-alive, Lab store,
│  │                            #   notifications, QR, Live Activity attributes (MeshActivityShared)
│  ├─ ios/MeshEngineKit/        # Swift package: binaryTarget MeshCoreFFI.xcframework (gitignored) + committed
│  │                            #   generated bindings + EngineHost bridge; depends on ../MeshPlatformKit
│  └─ android/                  # standalone Gradle build: :meshengine Android library (Kotlin + src/main/cpp Oboe shim)
├─ packages/
│  ├─ engine-api/               # @meshapp/engine-api: src/generated/*.ts (ts-rs) + typed client + guards
│  ├─ engine-web/               # @meshapp/engine-web: WASM loader + JS wrapper over the raw ABI + sim driver
│  ├─ capacitor-intercom/       # @meshapp/capacitor-intercom: TS-only plugin (definitions.ts, index.ts, web.ts)
│  └─ ui-kit/                   # @meshapp/ui-kit: CSS tokens + primitives (convLink look, own accent)
├─ apps/
│  └─ mobile/                   # @meshapp/mobile: Capacitor 8.5 app (React 19, Vite 8, Tailwind 4, Zustand 5, i18next)
│     ├─ src/ tests/ e2e/ public/  eslint.config.js
│     ├─ ios/App/               # Xcode project; App/Plugins/MeshIntercomPlugin.swift (app-local bridge);
│     │                         #   IntercomWidget extension; local package refs to native/ios/*
│     └─ android/               # Gradle app; app-local plugin; includeBuild("../../../native/android")
├─ docs/
│  ├─ DESIGN.md  WORKPLAN.json  BITACORA.md
│  ├─ research/INVESTIGACION.md
│  ├─ protocol/WIRE.md          # normative byte layouts (generated sections checked against vectors)
│  ├─ lab/LAB_PROTOCOL.md       # bench procedures + labrun/1 schema reference
│  ├─ conventions/              # coding conventions referenced by AGENTS.md
│  └─ field/results/            # dated field reports ("pending physical" vs "verified" kept separate)
├─ scripts/
│  ├─ bootstrap.sh              # idempotent toolchain setup (§2.2)
│  ├─ doctor.sh                 # prints what is missing, never installs
│  ├─ env.sh                    # exports ANDROID_HOME, NDK, SDK cmake PATH, JAVA_HOME 21
│  ├─ pick-sim.sh               # prints an available iPhone simulator UDID (never hard-code names)
│  └─ tasks.mjs                 # single task runner (convLink pattern, --dry-run)
├─ README.md  AGENTS.md
└─ .github/workflows/ci.yml     # mirrors `pnpm run ci` on a macOS arm64 runner (best effort)
```

### 2.2 Toolchain bootstrap (`scripts/bootstrap.sh`, idempotent, first work package)

Local state: Rust 1.89 (host target only), Xcode 27.1 (iOS 26.0-27.0 simulators), Swift 6.4, Android SDK 33-36, build-tools 36, NDK 27.1/28.2/29.0, **SDK cmake 3.22.1** (`$ANDROID_HOME/cmake/3.22.1`, includes ninja), **cargo-ndk already present at `~/.cargo/bin/cargo-ndk`**, Java 17 default (21 available), Node 26, pnpm 11.5, `jq`, Homebrew. There is no system cmake or ninja, and no wasm-pack, wasm-bindgen-cli or uniffi-bindgen.

```bash
set -euo pipefail
rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android x86_64-linux-android wasm32-unknown-unknown
command -v cargo-ndk >/dev/null || cargo install cargo-ndk --version ^4 --locked   # present today; kept for fresh machines
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/27.1.12297006"                         # pinned (convLink pitfall)
export PATH="$ANDROID_HOME/cmake/3.22.1/bin:$PATH"                                # cmake+ninja for the Oboe shim only
export JAVA_HOME="$(/usr/libexec/java_home -v 21)"                                # AGP 8.13 / Capacitor 8 Android
corepack enable >/dev/null 2>&1 || true; pnpm --version
rustup toolchain install "$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)" --profile minimal   # the pinned name is
                                                                                 #   a separate toolchain from "stable" even at the same version
bash scripts/msrv-probe.sh                                                       # fails if a pinned dep needs a newer Rust
[ -f tools/xtask/Cargo.toml ] && cargo xtask fetch-opus                          # libopus 1.6.1 + DNN weights, SHA-256 verified
[ -f apps/mobile/package.json ] && pnpm install --frozen-lockfile \
  && pnpm --filter @meshapp/mobile exec playwright install chromium             # e2e browsers (WP-18/21/22 verify)
# optional: rustup toolchain install nightly && cargo install cargo-fuzz --locked   (fuzzing only)
# optional: brew install cmake ninja  (NOT required; nothing in the default build needs system cmake)
```

`scripts/env.sh` exports the same variables for every task, plus the `cc`-crate cross variables for the Android targets, so every crate with a C build (opus-sys) cross-compiles with plain `cargo build --target` as well as through cargo-ndk:

```bash
NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"
export CC_aarch64_linux_android="$NDK_BIN/aarch64-linux-android31-clang"  AR_aarch64_linux_android="$NDK_BIN/llvm-ar"
export CC_x86_64_linux_android="$NDK_BIN/x86_64-linux-android31-clang"    AR_x86_64_linux_android="$NDK_BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$CC_x86_64_linux_android"
```

`scripts/doctor.sh` checks each item and prints a fix-it line, including the Playwright Chromium install and the result of `scripts/msrv-probe.sh` (the highest `rust-version` declared by any locked dependency, and the dependencies that declare none). No step needs a global install of wasm-bindgen, wasm-pack or uniffi-bindgen.

### 2.3 Rust build per target (`cargo xtask …`)

- **Host / tests:** `cargo test --workspace` on aarch64-apple-darwin. `opus-sys` compiles libopus with `cc`.
- **libopus (`opus-sys`, default build path):**
  1. `cargo xtask fetch-opus` downloads `https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz` and verifies its SHA-256 against `third_party/opus/SHA256SUMS`. The value is pinned at WP start from xiph's published `SHA256SUMS`. It extracts to `third_party/opus/opus-1.6.1/`. If the DNN weight files (`dnn/*_data.c`) are missing, it downloads the model tarball whose SHA-256 is hard-coded in that release's `autogen.sh`, verifies it and extracts it. It writes a `.ok` stamp with both hashes.
  2. `build.rs` fails with "run `cargo xtask fetch-opus`" when the stamp is missing. Because `mesh-core` enables `opus` by default (§3.1), **every Rust build of `mesh-core`, `mesh-sim`, `mesh-ffi` or the workspace runs `cargo xtask fetch-opus` first** (CI, `tasks.mjs rust *`, and every WORKPLAN verify that builds those crates). The command is idempotent and a no-op when the stamp is valid; until WP-06 lands, its stub prints "nothing to fetch" and exits 0. It parses upstream `celt_sources.mk`, `silk_sources.mk`, `opus_sources.mk` and `lpcnet_sources.mk` (DEEP_PLC, DRED, OSCE lists) so the file list tracks upstream. Float build, defines `OPUS_BUILD VAR_ARRAYS HAVE_LRINTF HAVE_LRINT ENABLE_DEEP_PLC ENABLE_DRED ENABLE_OSCE`. aarch64 uses NEON intrinsics presumed (no runtime detection, no DOTPROD). x86_64 (emulator) uses plain C + SSE2.
  3. `src/shim.c` exposes **fixed-arity** wrappers for every ctl we use (`ms_enc_set_bitrate`, `…_complexity`, `…_vbr`, `…_vbr_constraint`, `…_inband_fec`, `…_packet_loss_perc`, `…_dtx`, `…_max_bandwidth`, `…_signal`, `…_dred_duration`, `ms_dec_set_complexity`, `ms_get_lookahead`). Rust never calls a variadic C function, because the Apple arm64 ABI passes variadic arguments on the stack.
  4. Hand-written `extern "C"` declarations (about 25 symbols): no bindgen, no libclang.
  5. Upstream `tests/test_opus_api.c`, `test_opus_decode.c` and `test_opus_encode.c` are compiled and run on the host by `cargo test -p opus-sys`.
  6. `cargo xtask opus-smoke` cross-builds `opus-sys` for the four device/emulator targets (Phase 0 gate). The Android targets go through `cargo ndk` (which sets `CC_*`/`AR_*`) and fall back to the `scripts/env.sh` variables when cargo-ndk is absent.
  7. Fallback (documented, not default): `opusic-sys =0.7.5` with features `dred, osce` using the SDK cmake 3.22.1 on `PATH`.
- **iOS xcframework** (`cargo xtask build-ios`):
  1. `IPHONEOS_DEPLOYMENT_TARGET=26.0 cargo build -p mesh-ffi --release --target aarch64-apple-ios` and `--target aarch64-apple-ios-sim` → `libmesh_ffi.a` each (crate-type `["staticlib","cdylib","lib"]`).
  2. Bindings: `cargo build -p mesh-ffi --release` on the host → `cargo run -p uniffi-bindgen -- generate --library target/release/libmesh_ffi.dylib --language swift --out-dir build/gen/swift` (and `--language kotlin`). The output is committed to `native/ios/MeshEngineKit/Sources/MeshCoreBindings/mesh_ffi.swift` and `native/android/meshengine/src/main/kotlin/com/convlink/voice/core/mesh_ffi.kt`. `cargo xtask gen-bindings --check` fails CI on drift. This lets Swift and Kotlin agents compile without cross-building Rust first.
  3. Headers: `mesh_ffiFFI.h` (UniFFI) + `mesh_rt.h` (hand-written, §3.6, checked by the `cbindgen` crate used as a library in xtask) + `module.modulemap` (`module MeshCoreFFI { header "mesh_ffiFFI.h" header "mesh_rt.h" export * }`).
  4. `xcodebuild -create-xcframework -library <ios>/libmesh_ffi.a -headers build/gen/include -library <sim>/libmesh_ffi.a -headers build/gen/include -output native/ios/MeshEngineKit/Frameworks/MeshCoreFFI.xcframework` (gitignored; SwiftPM needs local binary targets under the package root).
- **Android `.so`** (`cargo xtask build-android`): `cargo ndk -t arm64-v8a -t x86_64 -P 31 -o native/android/meshengine/src/main/jniLibs build -p mesh-ffi --release`. armeabi-v7a is omitted (minSdk 31, Play 64-bit). If `cargo-ndk` is missing, xtask sets `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android31-clang` plus `CC_*` and `AR_*` (`llvm-ar`) and calls `cargo build --target` directly. Debug symbols are kept in `target/android-symbols/`.
- **WASM** (`cargo xtask build-wasm`): `cargo build -p mesh-wasm --release --target wasm32-unknown-unknown`. `mesh-wasm` depends on `mesh-core` and `mesh-sim` with `default-features = false`, so the `opus` feature (and `opus-sys`) is never unified into the wasm32 graph; `SimCodec` is used. The output is copied to `packages/engine-web/wasm/`. There is no wasm-bindgen: the module exports the raw ABI of §11.2. **No OS randomness in the graph:** `snow` is built with `default-features = false` plus our own `CryptoResolver` whose RNG is the engine's injected `ChaCha20Rng` (this also enforces the determinism rule on native), and the dalek crates are used without their `rand_core`/`getrandom` features (keys are generated from bytes the engine RNG supplies). As a guard for any remaining transitive `getrandom` 0.3, `mesh-wasm` declares `[target.'cfg(target_arch = "wasm32")'.dependencies] getrandom = { version = "0.3", features = ["wasm_js"] }` and `.cargo/config.toml` sets `rustflags = ['--cfg','getrandom_backend="wasm_js"']`; `cargo xtask check` fails if `cargo tree -p mesh-wasm --target wasm32-unknown-unknown -i getrandom` shows any path other than that guard.
- **Release profile:** `lto = "fat"`, `codegen-units = 1`, `opt-level = 3`, `debug = "line-tables-only"`, `panic = "unwind"` (UniFFI converts panics to foreign errors; the runtime catches engine-thread panics). RT entry points are `#![deny(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]`. Size gate: `cargo xtask size` fails if the iOS `.a` grows more than 10 % over `tools/xtask/size-baseline.json`, or the arm64 stripped `.so` exceeds 9 MB.

### 2.4 Native packaging and wiring

- **`native/ios/MeshPlatformKit/Package.swift`** (swift-tools 6.0, `platforms: [.iOS("26.0")]`): targets `CMeshAudio` (C: VPIO callbacks, atomics, function-pointer sink), `MeshActivityShared` (ActivityAttributes, used by the widget), `MeshPlatform` (Swift). Products `MeshPlatform`, `MeshActivityShared`. **No binary target**, so it resolves and builds in week 1 before any Rust artifact exists.
- **`native/ios/MeshEngineKit/Package.swift`**: `binaryTarget MeshCoreFFI` (path `Frameworks/MeshCoreFFI.xcframework`), `MeshCoreBindings` (generated, Swift 5 language mode), `MeshEngineKit` (EngineHost, adapters) depending on `.package(path: "../MeshPlatformKit")`. That is an in-repo relative path, never through `node_modules`.
- **Capacitor bridge (iOS):** the plugin's native code lives **in the app** (`apps/mobile/ios/App/App/Plugins/MeshIntercomPlugin.swift`), registered in a `BridgeViewController.capacitorDidLoad()` subclass (`bridge?.registerPluginInstance(MeshIntercomPlugin())`), the convLink pattern. The Xcode project references the two local packages by project-relative path. `@meshapp/capacitor-intercom` is TS-only, so `pnpm exec cap sync` never adds an SPM package for it (never `npx`, which could fetch an unpinned CLI).
- **Who creates what in the app project (stub hand-off, §16.2):** WP-18 creates the Xcode and Gradle projects, including stub `Plugins/MeshIntercomPlugin.swift`, stub Android `plugin/MeshIntercomPlugin.kt` (both answer `getInfo` with `engine: "platform-only"` and reject everything else with `unavailable`) and a stub `IntercomWidget/` bundle compiled against `MeshActivityShared`; ownership of those stubs passes to WP-19 and WP-23. Because the App target links MeshEngineKit (whose binary target exists only after `cargo xtask build-ios`) and the Android app links `:meshengine`, **native app builds are verified by WP-19** (which depends on WP-12, WP-15 and WP-16), not by WP-18.
- **Capacitor bridge (Android):** `apps/mobile/android/app/src/main/java/com/convlink/voice/plugin/MeshIntercomPlugin.kt`, registered with `registerPlugin(MeshIntercomPlugin::class.java)` before `super.onCreate`. `apps/mobile/android/settings.gradle` uses `includeBuild("../../../native/android")`, and the app depends on `com.convlink.voice:meshengine` (substituted by the included build).
- **`native/android`:** AGP 8.13.2, Gradle 8.14.3, Kotlin 2.3.x, JDK 21, compileSdk 36, targetSdk 36, minSdk 31, NDK 27.1.12297006, CMake 3.22.1 from the SDK, Oboe prefab `com.google.oboe:oboe:1.9.x` (exact pin), JNA `net.java.dev.jna:jna:5.17.0@aar`, kotlinx-coroutines. R8 keep rules: `-keep class com.sun.jna.** { *; }`, `-keep class com.convlink.voice.core.** { *; }`, `-keep class com.convlink.voice.engine.** { *; }`.
- **Audio host independence (from C):** `CMeshAudio` and the Oboe shim call the core through **function pointers registered at runtime** (`cmesh_set_sink(push_fn, pull_fn, flags_fn, ctx)` on iOS; the Oboe shim `dlopen("libmesh_ffi.so")` + `dlsym("mesh_rt_capture")` on Android, falling back to an internal tone/loopback sink). The audio hosts therefore build and run their own benches (meters, loopback, T5a) before the Rust library is linked.
- **Web:** `web.ts` in the plugin imports `@meshapp/engine-web` (lazily, only on the web platform). Workspace packages are consumed from `dist/` (convLink pitfall): `pnpm -r build` runs before the app type-check.
- **Plugin `getInfo()`** reports `engine: "platform-only" | "full"`, so the same UI serves a build where MeshEngineKit is not linked (emergency Swift-only debugging) and the full engine.

### 2.5 Script surface and CI

Root `package.json` scripts: `dev`, `build`, `test`, `lint`, `ci`, `help`. Everything else goes through `node scripts/tasks.mjs <group> <action> [--dry-run]` (`rust test|build-ios|build-android|build-wasm|gen|vectors`, `ios build|test|open`, `android build|test`, `cap sync`, `lab merge`, `doctor`). `pnpm run ci` runs exactly:

```bash
source scripts/env.sh
cargo xtask fetch-opus                                 # must precede anything that builds mesh-core (opus is a default feature)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings  # disallowed-* lints are denied only inside sans-IO crates (§3.1)
cargo xtask check                                      # determinism grep + wasm getrandom tree check
cargo test --workspace                                 # includes mesh-sim scenarios S1-S13 on fixed seeds
cargo xtask vectors --check && cargo xtask gen-ts --check && cargo xtask gen-bindings --check
cargo xtask build-ios && cargo xtask build-android && cargo xtask build-wasm
cargo xtask size                                       # size gate (§2.3)
pnpm install --frozen-lockfile && pnpm -r build && pnpm -r test && pnpm -r lint
pnpm --filter @meshapp/mobile exec playwright test     # e2e against the WASM sim (browsers installed by bootstrap)
(cd native/ios/MeshPlatformKit && xcodebuild -scheme MeshPlatformKit-Package -destination "id=$(../../../scripts/pick-sim.sh)" test)
(cd native/ios/MeshEngineKit && xcodebuild -scheme MeshEngineKit-Package -destination "id=$(../../../scripts/pick-sim.sh)" test)
(cd apps/mobile && pnpm exec cap sync)
xcodebuild -project apps/mobile/ios/App/App.xcodeproj -scheme App -destination 'generic/platform=iOS' CODE_SIGNING_ALLOWED=NO build
cargo build -p mesh-ffi --release                      # host dylib for the Kotlin JNA binding tests
(cd native/android && ./gradlew :meshengine:testDebugUnitTest :meshengine:lintDebug :meshengine:assembleRelease)
(cd apps/mobile/android && ./gradlew assembleDebug)
# nightly (separate job, fails loudly; never folded into a "|| skipped" step):
#   (cd fuzz && cargo +nightly fuzz build) && for t in $(cargo +nightly fuzz list); do cargo +nightly fuzz run $t -- -max_total_time=300; done
#   cargo run -p mesh-sim --release -- suite nightly --seeds 64
```

### 2.6 Pinned versions

| Component | Pin |
|---|---|
| Rust | **1.99.1** (installed 2026-10-22 as a separate rustup toolchain; the default `stable` 1.89 stays untouched), edition 2024. `rust-toolchain.toml` pins `channel = "1.99.1"`; `scripts/msrv-probe.sh` checks every pinned dependency against it and WP-01 copies the same pins into `[workspace.dependencies]`, with `cargo xtask check` asserting the two lists match. This supersedes the earlier provisional 1.89.0 pin (Appendix E). |
| uniffi | `=0.32.2` (link data as `Vec<u8>`; see §3.5) |
| rtrb | `=0.4.x` exact at WP start |
| snow (Noise rev 34), chacha20poly1305, x25519-dalek, ed25519-dalek, sha2, hkdf, hmac, zeroize, rand_chacha, plus crossbeam-channel, cc and getrandom | Exact pins (set by WP-00 in `scripts/msrv-probe/Cargo.toml` and mirrored in `[workspace.dependencies]`): `snow =0.10.0` with `default-features = false` and no features (its `std` feature enables getrandom) and a custom `CryptoResolver`; `chacha20poly1305 =0.10.1`; `x25519-dalek =2.0.1` and `ed25519-dalek =2.2.0` without `rand_core` features; `sha2 =0.10.9`, `hkdf =0.12.4`, `hmac =0.12.1` (the digest-0.10 generation that matches dalek 2); `zeroize =1.9.1`; `rand_chacha =0.9.0` with `rand_core =0.9.5`; `crossbeam-channel =0.5.17` (§3.1); `cc =1.9.0` (§2.3 opus-sys); `getrandom =0.3.4` with `wasm_js` (§2.3 wasm guard only). With Rust 1.99.1 the highest declared `rust-version` in the locked graph is 1.91 (cargo-platform 0.3.3 via uniffi `cli`). |
| nnnoiseless, serde, serde_json, ts-rs 12, proptest, cbindgen (xtask only) | exact pins |
| libopus | 1.6.1 via `opus-sys` (`cc`) |
| Capacitor | 8.5.x (SPM only, no CocoaPods); migrate to 9 after GA (adapter-only change, §11) |
| React / Vite / TS / Tailwind / Zustand / i18next / Vitest / Playwright | 19.2.x / 8.x / 5.9.x / 4.x / 5.x / 26.x + react-i18next / 4.x / latest, matching convLink |
| Android | AGP 8.13.2, Gradle 8.14.3, Kotlin 2.3.x, JDK 21, compile/target 36, minSdk 31, NDK 27.1.12297006, CMake 3.22.1 (SDK), Oboe 1.9.x, JNA 5.17.0 |
| iOS | Xcode 27.1, Swift 6.4 toolchain (tools-version 6.0), deployment target iOS 26.0, iPhone 12+ |

## 3. Rust core design

### 3.1 Crate graph and rules

```
mesh-api      (serde + ts-rs types only)
mesh-wire     (no deps beyond core/alloc + serde for vectors)
mesh-crypto ─► mesh-wire
mesh-lab ────► mesh-wire
opus-sys      (cc build; not compiled for wasm32)
mesh-voice ──► opus-sys (optional, feature "opus"; NOT default), nnnoiseless
mesh-routing ► mesh-wire
mesh-core ───► mesh-api, mesh-wire, mesh-crypto, mesh-lab, mesh-voice, mesh-routing
               features: default = ["opus"], opus = ["mesh-voice/opus"]
mesh-sim ────► mesh-core (default-features = false); features: default = ["opus"], opus = ["mesh-core/opus"]
mesh-ffi ────► mesh-core, mesh-sim (demo world)  (+ uniffi =0.32.2, rtrb, crossbeam-channel) → staticlib/cdylib
mesh-wasm ───► mesh-core + mesh-sim, both default-features = false (no opus)           → cdylib (wasm)
```

**Crate stubs:** WP-01 creates every workspace crate as a compiling stub (`Cargo.toml` with the dependency edges and features above, empty `src/lib.rs`), and ownership of each stub passes to the crate's WP (§16.2 stub hand-off). `mesh-core` can therefore declare all of its dependencies on day 1, and no later WP edits another WP's `Cargo.toml` to add a workspace edge. New external dependencies go through a contract change request to the manifest's owner.

**Determinism rules** apply to `mesh-wire|crypto|lab|voice|routing|core|sim`. `clippy.toml` holds the `disallowed-methods`/`disallowed-types` list; the workspace sets `[workspace.lints.clippy] disallowed_methods = "allow"` and `disallowed_types = "allow"`, and **only those seven crates** put `#![deny(clippy::disallowed_methods, clippy::disallowed_types)]` in `lib.rs` (a source attribute overrides the workspace level). `mesh-ffi`, `mesh-wasm`, `xtask`, `uniffi-bindgen`, `opus-sys` and all integration-test targets (`tests/*/main.rs`) may use threads and clocks. `cargo xtask check` adds a grep over the seven crates' `src/` as a second line of defence. WP-01 proves the setup: `cargo clippy --workspace --all-targets -- -D warnings` passes with a stub `mesh-ffi` that spawns a thread, and a scratch call to `std::time::Instant::now()` inside a sans-IO stub fails it.
- no `std::time`, no threads, no `getrandom`, no `HashMap`/`HashSet` iteration (use `BTreeMap`, `Vec`, fixed arrays);
- RNG is an injected `rand_chacha::ChaCha20Rng`; time is `Instant(u64 µs, monotonic)` supplied by the caller;
- no allocation on `push_capture`/`pull_playout`. This is proven by a counting-allocator test.

### 3.2 Sans-IO engine API (`mesh-core`, contract C3)

```rust
pub struct Instant(pub u64);            // µs, monotonic, supplied by caller
pub struct LinkId(pub u32);             // assigned by the platform
pub struct NodeId(pub u16);

pub struct EngineConfig {
    pub platform: Platform,             // Ios | Android | WebSim
    pub app_version: u16,
    pub stored_groups: Vec<StoredGroup>,// per-group identity + epoch secrets, loaded by the runtime from secure storage
    pub rng_seed: [u8; 32],             // CSPRNG on device, fixed in sim
    pub limits: Limits,                 // link targets, caps, timers (per-platform defaults, Appendix A)
}

pub enum SessionStart {
    Group { group_id: [u8; 16] },        // lobby: radios on, audio off until GoLive
    Lab { lab_code: u16 },               // Lab mode (§12)
    Demo,                                // handled by the runtime (§3.4), not by Engine
}

pub enum PlatformEvent<'a> {
    Up { link: LinkId, info: LinkInfo }, // kind, role, max_frame, unit (Stream|Message|Datagram), peer_platform_hint
    Down { link: LinkId, reason: LinkDownReason },   // Timeout | PeerClosed | LocalClose | RadioOff | LimitReached | Error(code)
    Writable { link: LinkId },
    Metrics { link: LinkId, rssi: Option<i16>, att_mtu: Option<u16> },
    Discovered(Discovery),               // disc_id, transport, rssi, adv tokens (Option), platform hint
    HelloRead { disc_id: u32, bytes: &'a [u8] },
    AudioRoute(AudioRouteInfo),          // kind, input_rate (8k/16k/32k/48k), output kind, headset_present
    AudioIo(AudioIoState),               // Running | Interrupted{reason} | Restarting | CaptureLost | RenderLost | Restored
    Platform(PlatformSample<'a>),        // gps fix, battery, thermal, app visibility, device locked, call state
    RadioState { ble: RadioAvail, aware: RadioAvail }, // On | Off (BT off / airplane) | Unauthorized | Unsupported
    PromptPcm { key: PromptKey, pcm: &'a [i16] }, // rendered spoken prompt (16 kHz mono), §7.7
}

impl Engine {
    pub fn new(cfg: EngineConfig, now: Instant) -> Engine;
    pub fn handle_command(&mut self, now: Instant, cmd: mesh_api::UiCommand) -> mesh_api::CommandResult;
    pub fn handle_event(&mut self, now: Instant, ev: PlatformEvent<'_>);
    pub fn handle_packet(&mut self, now: Instant, link: LinkId, bytes: &[u8]); // stream chunk / ATT value / datagram
    pub fn handle_timeout(&mut self, now: Instant);
    pub fn poll_timeout(&self) -> Option<Instant>;
    pub fn poll_transmit(&mut self, now: Instant) -> Option<Transmit>;  // { link, bytes } framed for that link kind
    pub fn poll_effect(&mut self) -> Option<Effect>;                    // Radio | Audio | Ui | Persist | Notify | LabWrite | RenderPrompts | Log
    pub fn push_capture(&mut self, now: Instant, pcm: &[i16; 160]);     // 10 ms @ 16 kHz mono
    pub fn pull_playout(&mut self, now: Instant, out: &mut [i16; 160]); // 10 ms @ 16 kHz mono
    pub fn set_rt_flags(&mut self, flags: u32);                         // PTT down / hard mute fast path (§3.6)
}
```

- **All byte-level framing lives in Rust** (`mesh-wire::link`): L2CAP length-prefix deframing, GATT chunk reassembly, records and mesh packets. Swift and Kotlin move opaque bytes only, so iPhone↔Android drift at the byte level is structurally impossible, and the fuzz targets cover the code that ships. For stream links `handle_packet` accepts any chunk boundary; for GATT each call is one ATT value; for datagram links each call is one datagram. Each `poll_transmit` result is written as one unit.
- **Pacing is in the core:** a `Transmit` is emitted only when the link's token bucket and the node airtime budget allow it and the link is writable. Voice frames carry deadlines and are dropped in the core, never in an OS buffer (§5.6).
- **Radio decisions are in the core:** discovery mode, which peer to connect, which link to close, upgrades. The platform executes `RadioCommand`s: `SetDiscovery{mode, adv}`, `SetHello{bytes}`, `Connect{disc_id}`, `OpenLink{disc_id, via: L2cap{psm}|Gatt}`, `Abort{disc_id, ignore_s}`, `Close{link, reason}`, `SetLinkParams{link, priority}`, `ArmReconnect{peer_ref, until_s}` (T3 variant), `AwareUpgrade{peer, op}` (Phase 4), `TunnelOpen{..}` (v2).
- **Audio decisions are in the core:** `AudioCommand::{OpenSession{profile}, StartCapture, StopCapture, SetRoutePreference, OutputOnlyFallback(bool), CloseSession}`.
- **Capture gating (normative, single owner per step).** Four commands may start capture, and each must arrive through the plugin's `gestureCommand` (§11.1), which refuses unless the app is foreground-active (iOS `applicationState == .active`; Android activity resumed): `goLive`, `resumeMic`, `labStart` with `audio: "full"`, and `audioCheck{op:"start"}`. The plain `send` pass-through rejects these four types with `useGestureCommand`. Inside the tap, **the bridge synchronously performs every OS-level, gesture-bound action** (iOS: activate the session, initialise and start VPIO with the RT sink muted, start the location anchor when requested, start or update the Live Activity; Android: `startForegroundService` with the `microphone` type, open Oboe streams) and only then forwards the command. The core then emits `StartCapture`, which on both platforms only **connects the RT sink and unmutes**; it never activates a session or starts a service, so a late or duplicated `StartCapture` is harmless. Property test (WP-10B): over random command/event sequences, `StartCapture` is emitted only while handling one of those four commands, and never in Idle. `labStart` with `audio: "renderOnly"` also arrives through `gestureCommand` (the session is activated in the foreground) but emits only `OpenSession` + render, never `StartCapture`; `audio: "off"` emits no audio command and may use plain `send`.
- **Radio state:** `RadioState{ble: Off}` (Bluetooth off, airplane mode; iOS `CBManagerState.poweredOff`, Android `ACTION_STATE_CHANGED`) puts discovery on hold, marks all BLE links down with `RadioOff`, sets PRESENCE `radio_off` once the radio returns (peers saw it as lost meanwhile), plays the `radioOff` earcon + prompt and raises notice `bluetoothOff`. `Unauthorized` raises `bluetoothDenied` with an "open Settings" action. `LinkDownReason::LimitReached` (iOS `CBError.connectionLimitReached`, Android GATT 133/257 on connect, `BluetoothStatusCodes.ERROR_…` connection-limit codes) lowers that device's BLE link max by 1 for 10 min (floor: 1).
- **Modes:** `EngineMode::Idle | Lab{code} | Session{group_id}`. Group management commands work in Idle (no threads or radios). `audioCheck` works without a session (the runtime starts the engine thread with radios off for the duration of the check, then returns to Idle) and in the lobby before Go live; it is refused while live or during a Lab run.

### 3.3 `mesh-core` module map (sub-ownership for parallel agents)

| Module | Owns | WP |
|---|---|---|
| `Cargo.toml`, `lib.rs`, `engine.rs`, `effects.rs`, `config.rs`, `wire/mod.rs` | Engine struct, dispatch, effect queues, the `CoreModule` trait and registry, the fixed module list; the manifest declares **all** workspace dependencies and features from day 1 (§3.1) | WP-10A |
| `links/` | link registry; per-link state machine `Hello → Handshake → IdProof → Up → Closing`; framing; PING/PONG; LSTAT; CAPS; per-link scheduler; the `LinkSecurity` trait (`Plain` for Lab, implemented by `secure/` for groups) | WP-10A |
| `lab/` | lab sessions, LAB_CTL, bench runners (from mesh-lab), Lab audio paths and anchors, minimal TTL+dedup flood for MESH_PROBE, platform samples, labrun records | WP-10A |
| `ui/` | command dispatch, snapshot builder (dirty flag, ≤ 10 Hz, suppressed when UI detached), discrete events with i18n keys | WP-10A |
| `secure/` | `LinkSecurity` implementation: Noise sessions, SEC framing and key update, IDPROOF exchange, deny-list and collision checks, duplicate-link resolution | WP-10B |
| `group/` | stored groups, invite create/parse, per-group identities, roster cache/verification, JOIN_REQ, epochs, rekey, deny list, token rotation, anti-entropy (§8.9), leader handover | WP-10B |
| `session/` | ride state, go-live, talk mode, ladder level, presence origination, reachability view, partition alerts, positions, radio state, audio check | WP-10B |
| `media/` | capture chain and receive chain, encoder profile selection, earcons, prompt bank, latency estimator, demo voice lines | WP-10C |
| `mesh/` | glue to mesh-routing: flood, pruning inputs (ctrl/bulk only), floor bridge, PATH_HINT, link selection policy | WP-10D |
| `wire/<module>.rs` (`secure`, `group`, `session`, `media`, `mesh`) | one `pub fn register(reg: &mut ModuleRegistry)` per module; WP-10A creates each as a no-op stub and ownership passes to the module's WP | the module's WP |

`engine.rs` calls `wire::register_all()`, which `wire/mod.rs` implements as a fixed list (`secure::register(reg); group::register(reg); …`). Sub-WPs plug in by editing only their own `wire/<module>.rs` and module directory, never `engine.rs` or `Cargo.toml`. Integration tests use one directory per owner (`tests/<area>/main.rs`, which Cargo discovers as test target `<area>`), so test ownership is also by subtree.

### 3.4 Runtime and threading (`mesh-ffi`)

| Thread | Owner | Does | Must never |
|---|---|---|---|
| Audio RT (CoreAudio IO / Oboe callback) | OS | `mesh_rt_capture` / `mesh_rt_render` on preallocated rings; reads atomic flags | allocate, lock, log, call UniFFI |
| Engine (`mesh-engine`) | mesh-ffi | owns `Engine`. Loop: drain inbound MPSC → drain capture ring (10 ms blocks) → refill render ring to 20 ms → apply RT flags → `handle_timeout` → flush transmits/effects to `PlatformHooks` → `park_timeout(min(next_deadline, 5 ms))` | block on platform calls |
| Radio threads | platform (CB queue, NSStream run-loop thread, Android binder/socket threads) | call `on_link_*` → bounded MPSC (`crossbeam-channel`, 2048) + `unpark` | touch `Engine` directly |
| UI | Capacitor main thread | `ui_command(json)`: through the MPSC with a one-shot reply (≤ 5 ms) while running; inline under the runtime mutex in Idle | be in any RT path |

Engine thread priority: iOS `pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0)`; Android `setpriority(PRIO_PROCESS, gettid(), -19)`. Engine-thread panics are caught with `catch_unwind` and reported as `engine: crashed` plus a local notification; the runtime stops cleanly.

**Demo ride** belongs to the runtime, not the core: on `startEngine({demo:true})` the runtime creates a local demo group and attaches a `mesh-sim` world of 4 bot engines through virtual links. **No audio assets are bundled.** At Demo start (foreground, inside the tap) the core requests the `demo.*` lines of the `prompts` i18n namespace (owned by WP-18, ES/EN, about 5 s each) through the existing `render_prompts` path (platform TTS, on device); each bot Opus-encodes its PCM once with the real encoder (the PCM is then dropped and never counts against the 512 KB prompt bank; ≈ 8 lines × 5 s × 16 kbps ≈ 80 KB of Opus) and replays the frames on a talk schedule, so the real audio, floor and background path runs on one phone and in the iOS Simulator. If TTS is unavailable, bots fall back to a synthesized speech-like tone pattern generated in `mesh-sim` (WP-11). This avoids any licensing question about recorded or TTS-generated clips shipped in the binary.

### 3.5 UniFFI surface (control plane, contract C4)

```rust
#[derive(uniffi::Object)]
pub struct MeshEngine { /* runtime handle */ }

#[uniffi::export]
impl MeshEngine {
    #[uniffi::constructor]
    pub fn new(cfg: FfiEngineConfig, hooks: Arc<dyn PlatformHooks>) -> Result<Arc<Self>, FfiError>; // cheap, no threads
    pub fn ui_command(&self, json: String) -> String;                     // works in Idle too
    pub fn start(&self, start: FfiSessionStart) -> Result<(), FfiError>;  // engine thread + radios; audio still off
    pub fn stop(&self);
    pub fn on_link_up(&self, link: u32, info: FfiLinkInfo);
    pub fn on_link_down(&self, link: u32, reason: FfiLinkDownReason);
    pub fn on_link_data(&self, link: u32, data: Vec<u8>);                // Vec<u8>, never &[u8] (0.32 Kotlin direct-ByteBuffer break)
    pub fn on_link_writable(&self, link: u32);
    pub fn on_link_metrics(&self, link: u32, rssi: Option<i16>, att_mtu: Option<u16>);
    pub fn on_discovered(&self, d: FfiDiscovery);
    pub fn on_hello_read(&self, disc_id: u32, bytes: Vec<u8>);
    pub fn on_audio_route(&self, r: FfiAudioRoute);
    pub fn on_audio_io(&self, s: FfiAudioIoState);
    pub fn on_platform_sample(&self, s: FfiPlatformSample);
    pub fn on_prompt_pcm(&self, key: String, pcm: Vec<i16>);
    pub fn set_ui_attached(&self, attached: bool);                        // gates snapshot emission
}

#[uniffi::export] pub fn ble_constants() -> FfiBleConstants;  // single source of truth for UUIDs and timings
#[uniffi::export] pub fn protocol_version() -> u8;
#[uniffi::export] pub fn api_version() -> u16;                // UI contract major.minor; plugin refuses a different major

#[uniffi::export(with_foreign)]
pub trait PlatformHooks: Send + Sync {
    fn link_send(&self, link: u32, bytes: Vec<u8>) -> FfiSendOutcome;   // Accepted | WouldBlock | NoLink; non-blocking
    fn radio(&self, cmd: FfiRadioCommand);
    fn audio(&self, cmd: FfiAudioCommand);
    fn ui_event(&self, json: String);                                   // forwarded verbatim to the plugin
    fn persist(&self, key: String, value: Option<Vec<u8>>);             // Keychain / Keystore-backed
    fn notify(&self, n: FfiNotification);                               // local notification, Live Activity, FGS notification
    fn lab_write(&self, run_id: String, line: String);                  // append one labrun/1 JSONL line
    fn render_prompts(&self, reqs: Vec<FfiPromptRequest>);              // {key, text, lang}; platform TTS → on_prompt_pcm
    fn log(&self, level: FfiLogLevel, msg: String);
}
```

`uniffi.toml`: Kotlin `package_name = "com.convlink.voice.core"`, `cdylib_name = "mesh_ffi"`; Swift `module_name = "MeshCoreBindings"`, `ffi_module_name = "MeshCoreFFI"`. Call rates are link data 50-400/s, sends 50-400/s and UI ≤ 10/s, so UniFFI copying is negligible. The audio path never touches UniFFI.

### 3.6 Real-time C ABI (`crates/mesh-ffi/include/mesh_rt.h`, contract C5)

```c
#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#define MESH_RT_ABI_VERSION 1u
#define MESH_RT_SAMPLE_RATE 16000u   /* mono, int16. v1 ceiling is wideband (16 kHz); a v2 ABI may negotiate 32 kHz */
/* All functions: wait-free, no allocation, no locks, no syscalls, never panic. */
int32_t mesh_rt_capture(const int16_t *pcm, uint32_t frames, uint64_t host_time_ns); /* frames accepted */
int32_t mesh_rt_render(int16_t *pcm, uint32_t frames, uint64_t host_time_ns);        /* fills exactly `frames` */
void    mesh_rt_set_flags(uint32_t flags);  /* bit0 PTT down, bit1 hard mute, bit2 motorcycle-builtin-mic mute,
                                               bit3 PTT latch toggled (edge; set by LatchTalkIntent / notification action) */
void    mesh_rt_report(uint32_t event_code);/* OS-detected xrun/route glitch counter */
typedef struct MeshRtStats {
    uint64_t captured_frames, rendered_frames;
    uint32_t capture_overruns, render_underruns;
    uint32_t capture_callbacks, render_callbacks;   /* read by the native MuteWatchdog */
    float    capture_rms_dbfs, render_rms_dbfs;
    uint32_t generation;
} MeshRtStats;
void    mesh_rt_stats(MeshRtStats *out);
#ifdef __cplusplus
}
#endif
```

Implementation: one process-global `static RT: RtShared` (leaked once) holds `rtrb` producer/consumer halves in `UnsafeCell`s, each touched by exactly one thread by contract, plus `AtomicBool active`, `AtomicU32` counters, an `AtomicU32 flags` and `AtomicU32 generation`. `start()` resets the rings; while inactive, render outputs zeros and capture discards. Capture ring 4096 samples (256 ms), render ring 4096, render target fill 320 samples (20 ms). **PTT fast path:** the plugin writes `mesh_rt_set_flags` directly on the button event (no core round trip). The engine reads the flags every tick, so PTT-down to the first encoded frame is ≤ 5 ms + pre-roll. Bench T8 measures the remaining leg, touch → WebView → bridge → `mesh_rt_set_flags` (target < 50 ms p95). **Wideband ceiling:** the RT ABI is fixed at 16 kHz in v1, so v1 is wideband end to end even on LE Audio / LC3-SWB routes (the OS resamples). CAPS no longer advertises SWB capture (§4.4); raising the rate is a v2 ABI change (`MESH_RT_ABI_VERSION 2`, rate chosen at `OpenSession`). The same symbols are exported from `libmesh_ffi` (static on iOS, `.so` on Android) and reached through the function-pointer sink (§2.4).

### 3.7 UI contract and TS generation (`mesh-api`, contract C1)

All types `#[derive(Serialize, Deserialize, TS)]`, `#[serde(tag = "type", rename_all = "camelCase")]`, exported to `packages/engine-api/src/generated/`. `cargo xtask gen-ts` runs `cargo test -p mesh-api --features ts-export export_bindings`; `--check` diffs the directory. `API_VERSION` is `1.1` (1.0 was the pre-review draft and never froze; 1.1 adds `audioCheck`, `transferLeader`, Lab audio/anchor params and the radio/leader notices; its Lab subset freezes in Phase 0 and the full contract in week 1 of Phase 1, and any later change bumps the minor version). The engine emits **i18n keys plus params**, never English text. Commands marked ★ are accepted only through the plugin's `gestureCommand` (§3.2).

```ts
// UiCommand (JSON, discriminated by `type`) → CommandResult { ok: true, ... } | { ok: false, code: ErrorCode }
| { type: "createGroup"; name: string; vehicle: "moto" | "car" }              → { groupId }
| { type: "parseInvite"; url: string }                                        → { valid; groupName; groupId; epoch; expired; alreadyMember;
                                                                                   leaderSigOk }   // invite blob signature by its leader key
| { type: "joinGroup"; url: string; displayName: string }                     → { groupId }
| { type: "listGroups" } | { type: "forgetGroup"; groupId: string }
| { type: "getInvite"; groupId: string }                                      → { qrPng: string /* data URL rendered natively */;
                                                                                   hideAfterMs: 60000 }  // the invite text never reaches JS
| { type: "goLive" }★ | { type: "resumeMic" }★
| { type: "audioCheck"; op: "start"; mode: "tone" | "echo" | "voxMeter" }★ | { type: "audioCheck"; op: "stop" }
| { type: "setTalkMode"; mode: "vox" | "ptt" | "muted" }
| { type: "ptt"; down: boolean }                                              // also mirrored to mesh_rt_set_flags
| { type: "setPttStyle"; style: "hold" | "latch" }
| { type: "setVoxSensitivity"; level: "low" | "medium" | "high" }
| { type: "rideControl"; op: "start" | "end" }
| { type: "setDisplayName"; name: string } | { type: "setVehicle"; vehicle: "moto" | "car" }
| { type: "setRoutePreference"; pref: "auto" | "headset" | "speaker" }
| { type: "setShareDistance"; enabled: boolean }
| { type: "setPrompts"; lang: "es" | "en"; enabled: boolean; templates: Record<PromptKey, string> }
| { type: "setBuiltinMicTx"; allowed: boolean }
| { type: "removeMember"; nodeId: number } | { type: "lockGroup" } | { type: "closeGroup" }
| { type: "previewAccessLoss" }                                              → { unconfirmed: { nodeId; name }[] }  // shown before lock/remove
| { type: "transferLeader"; nodeId: number }
| { type: "setInviteExpiry"; hours: number | null }
| { type: "requestSnapshot" }
| { type: "labStart"; bench: LabBench; params: LabParams }★ (★ when params.audio ≠ "off") | { type: "labStop" }
| { type: "labMark"; label: string } | { type: "labListRuns" } | { type: "labDeleteRun"; runId: string }

interface LabParams {                       // bench-specific fields (durationS, streams, pattern, transport, role …) plus:
  audio: "off" | "renderOnly" | "full";     // mandatory; benches needing capture or HFP (T0 headset, T2, T5a/b, T6, VOX-cal) use "full"
  anchor: "none" | "audio" | "audioLocation"; // mandatory keep-alive anchor (§12.1); "none" only for the suspension-control variant
  gps: boolean;                             // per-run consent
}

// UiEvent
| { type: "snapshot"; s: Snapshot }                                   // ≤10 Hz, only when changed and UI attached
| { type: "talkDenied"; reason: "cap" | "preempted" | "congestion" }
| { type: "notice"; key: NoticeKey; params: Record<string, string | number> }  // memberJoined, memberLost(aggregated),
                                                                      // memberBack, alone, partition, captureLost, captureRestored,
                                                                      // headsetLost, onCall, removed, groupLocked, rideStarted,
                                                                      // bluetoothOff, bluetoothDenied, linkLimitReached, leaderChanged,
                                                                      // inviteLeaderMismatch, liveActivityRenewed …
| { type: "routeChanged"; route: AudioRouteKind }
| { type: "audioCheckSample"; level: number; vadP: number; gate: "open" | "closed";
    route: AudioRouteKind; inputRate: number; phase: "tone" | "record" | "echo" | "meter" }   // 20 Hz while an audioCheck runs
| { type: "labProgress"; runId: string; elapsedS: number; sample: LabLiveSample }  // 2 Hz
| { type: "labFinished"; runId: string; summary: LabSummary; files: string[] }
| { type: "error"; code: ErrorCode; params: Record<string, string | number> }

interface Snapshot {
  engine: "idle" | "starting" | "running" | "degraded" | "crashed";
  session: null | { groupId: string; groupName: string; role: "leader" | "member"; myNodeId: number;
                    live: boolean; mode: "vox" | "ptt" | "muted"; talk: "idle" | "talking" | "blocked" | "muted";
                    ride: "lobby" | "riding"; ladder: "L0" | "L1" | "L2" | "L3"; shareDistance: boolean };
  audio: { route: AudioRouteKind; captureOk: boolean; renderOk: boolean; inputLevel: number; txMutedNoHeadset: boolean };
  talkers: { nodeId: number; name: string; hops: number; level: number; sinceMs: number }[];   // ≤4
  members: { nodeId: number; name: string; role: "leader" | "member"; platform: "ios" | "android";
             status: "me" | "direct" | "relayed" | "oneWay" | "lost" | "unconfirmed";
             hops: number; quality: number; linkKinds: ("ble" | "aware" | "tunnel")[];
             hearsMe: boolean; voice: "kpi" | "bestEffort" | "no";   // ≤ 2 BLE hops / 3-4 hops in age cap / out
             lastHeardMs: number; battery: number | null;
             ready: { live: boolean; mic: boolean; route: AudioRouteKind | null };
             flags: ("onCall" | "micLost" | "noHeadset" | "radioOff")[];
             distanceM: number | null; bearingDeg: number | null }[];
  mesh: { links: number; neighbors: number; islands: number; reachable: number; total: number;
          capBle: number; capWifi: number; profile: "ble" | "bleDegraded" | "wifi" };
  device: { battery: number | null; thermal: "nominal" | "fair" | "serious" | "critical"; background: boolean;
            bluetooth: "on" | "off" | "unauthorized" };
  latency: { p50Ms: number; p95Ms: number; byHops: number[] } | null;  // §12.6 estimator, excludes headset legs
}
type AudioRouteKind = "hfpWideband" | "hfpNarrowband" | "leAudio" | "builtinSpeaker" | "builtinReceiver" | "wired" | "carBluetooth" | "none";
```

## 4. Wire protocol specification (v1)

All multi-byte integers are **little-endian**. Protocol version **1**. Every layer is length-delimited and every parser is bounded: there is no compression anywhere (Bridgefy's decompression bomb) and no recursion. Unknown record types are skipped. Unknown mesh packet types are **still relayed**, so old relays carry new traffic, but they are not delivered. Normative examples are the golden vectors in `test-vectors/` (§4.10). `docs/protocol/WIRE.md` is the human copy of this section.

### 4.1 Layering

```
transport bytes ─► [4.2 link framing] ─► LinkFrame ─► [4.3 lf_type] ─► Noise HS | SEC (AEAD) | LAB (plain, Lab only) | REJECT
                                                        SEC/LAB plaintext ─► [4.4 records]* ─► MESH record ─► [4.5 MeshPacket]
```

Nothing above the link framing is ever parsed outside `mesh-wire` (Rust). Mesh-level dedup, floor and delivery happen **only after** the SEC frame's AEAD tag verified (§5.3).

### 4.2 Link framing per transport

| Transport | Unit given to the core | Framing | Max LinkFrame |
|---|---|---|---|
| BLE L2CAP CoC (stream API on both OSes) | arbitrary stream chunks | `len u16` + `frame[len]`, `len ∈ [1, 1024]`; an invalid length closes the link (no resync) | 1024 B |
| BLE GATT data path (fallback) | one ATT value per call | per value: `hdr u8` + `chunk`; `hdr` bit7 START, bit6 END, bits5-0 frame counter mod 64 (same for all chunks of one frame); chunk ≤ `ATT_MTU − 3 − 1` | 1024 B reassembled |
| Wi-Fi Aware (UDP over iOS `NetworkConnection` / Android `Network` socket) | one datagram | none (datagram = one frame) | 1200 B |
| Tunnel (v2) | stream chunks | as L2CAP | 1024 B |

GATT reassembly: a START discards any partial frame; a chunk whose counter differs from the current partial frame discards it; the buffer is bounded at 1024 B; END completes the frame. One frame in flight per direction per link (GATT is ordered).

### 4.3 LinkFrame

```
offset size field
0      1    lf_type   0x00 HS (Noise handshake) | 0x01 SEC (encrypted records) | 0x02 LAB (plaintext records, Lab links only)
                      | 0x03 REJECT (plaintext; the sender then closes)
1      …    body
```

- **HS body:** `hs_step u8 (1|2|3)` · `proto_ver u8` (initiator picks the highest common version from HELLO) · `noise_msg[..]`. Pattern `Noise_XXpsk1_25519_ChaChaPoly_SHA256`, prologue `"cvic/v1" ‖ group_id(16) ‖ epoch u32`, PSK `psk_e` (§8.3). Sizes with empty payloads: msg1 `e, psk` = 32 + 16 = 48 B; msg2 `e, ee, s, es` = 32 + 48 + 16 = 96 B; msg3 `s, se` = 48 + 16 = 64 B. Handshake timeout 5 s.
- **SEC body:** `kp_ctr u32` (bit 31 = key phase, bits 30-0 = `ctr`) · `ciphertext` · `tag[16]`. The two direction keys come from `snow`'s `HandshakeState::dangerously_get_raw_split()` at the end of the handshake (initiator→responder, responder→initiator); `snow`'s transport states are **not** used, because their API takes no associated data. Each frame is sealed with the `chacha20poly1305` crate (IETF ChaCha20-Poly1305): key = current phase key of that direction, nonce = `0x00000000 ‖ u64(kp_ctr)` (12 B, little-endian), **AAD = `lf_type (0x01) ‖ kp_ctr (4 B LE)`**. Overhead 21 B per frame (incl. lf_type). `ctr` starts at 0 per direction per link and per phase. The receiver keeps a 1024-bit sliding replay window per direction and phase and updates it **only after** the tag verifies.
- **In-band key update (no second link):** each direction updates its own key after 12 h or when `ctr` reaches 2³⁰: `k_next = HKDF-Expand(PRK = k_cur, "cvic/v1/keyupdate", 32)`, flip the phase bit, restart `ctr` at 0, zeroize `k_cur`. A receiver that sees the other phase bit derives `k_next` lazily, and commits to it only if the tag verifies; it keeps the previous key for 10 s for reordered datagrams, then zeroizes it. The handshake hash, link id and duplicate-link state are unchanged, so no second link to the same peer ever exists.
- **LAB body:** `lab_tag u16` (= `SHA-256("cvic/lab" ‖ u16 lab_code)[0..2]`) · records. The sender appends a `PAD` record so the on-air size equals the SEC frame for the same records. LAB frames are accepted only on links whose HELLO had `flags.lab = 1`, and never carry group traffic.
- **REJECT body:** `reason u8`: 1 version, 2 group/epoch mismatch, 3 at max links, 4 duplicate link, 5 member removed/denied, 6 lab code mismatch, 7 handshake failure, 8 node_id collision.

### 4.4 Records (inside SEC/LAB plaintext)

`rtype u8` · `len` LEB128 (1-2 B, ≤ 16383) · `body[len]`. At most 32 records per frame. The sender coalesces whatever is queued for that link at send time and never waits to coalesce.

| rtype | Name | Body | Size | Scope |
|---|---|---|---|---|
| 0x01 | MESH | MeshPacket (§4.5) | 9 + payload | relayable |
| 0x02 | PING | `id u16, t_tx u32` (sender µs clock, truncated) | 6 | link |
| 0x03 | PONG | `id u16, t_tx u32` (echo), `hold_us u16` (rx→tx delay at responder) | 8 | link |
| 0x04 | NBR | `ver u8=1, nseq u8, count u8, count × (node_id u16, q u8, flags u8, vcap u8)`; flags b0 good (q ≥ Q_GOOD), b1 congested, b3-2 kind (0 BLE, 1 Aware, 2 Tunnel); count ≤ 8 | 3 + 5n | link: on change (≤ 4 Hz) and every 1 s |
| 0x05 | LSTAT | `rx_frames u16, rx_bytes u32, gaps u16, pace_kbps u16, queue_ms u8` | 11 | link (1 s) |
| 0x06 | CAPS | `proto u8, platform u8 (0 ios, 1 android, 2 websim), app_ver u16, codec u16 (b0 opus 1.6.1, b1 DRED v-current, b2 OSCE, b3 reserved: v1 is wideband-only, §3.6), transports u8 (b0 l2cap, b1 gatt, b2 aware-ios, b3 aware-android, b4 tunnel), max_links u8, role u8 (0 member, 1 leader), epoch u32, roster_version u32, ctl_version u32` (the anti-entropy summary, §8.9; zeros on Lab links) | 21 | link (once) |
| 0x07 | LPROBE | `probe_id u32, t_tx u64, flags u8, pad[..]` | 13 + pad | lab link |
| 0x08 | LECHO | `probe_id u32, t_tx u64 (echo), hold_us u32, rx_count u32` | 20 | lab link |
| 0x09 | UPGRADE | `op u8 (1 offer, 2 accept, 3 reject, 4 ready, 5 retire), transport u8 (1 aware-ios, 2 aware-android, 3 tunnel), nonce u32, params[..]` | 6+ | link |
| 0x0A | CLOSE | `reason u8` (REJECT reasons, plus 9 redundant, 10 leaving) | 1 | link |
| 0x0B | LAB_CTL | `op u8 (1 start, 2 stop, 3 mark, 4 pattern, 5 announce), run_id u32, bench u8, params[..]`; `announce` params = `lab_node_id u16, nonce u64` | 6+ | lab link |
| 0x0C | IDPROOF | `ed_pk[32], x_pk[32], sig[64]`, sig = Ed25519 over `"cvic/v1/idproof" ‖ role u8 (0 initiator, 1 responder) ‖ handshake_hash(32)` | 128 | link (once, first SEC record each way) |
| 0x0D | LAB_GPS | `t_ms u32, lat_e7 i32, lon_e7 i32, speed_cms u16, acc_dm u16, heading_cdeg u16` | 18 | lab link (1 Hz) |
| 0x0E | PAD | zero bytes | n | any (ignored) |
| 0x0F | SYNC | `kind u8 (1 ROSTER, 2 REKEY, 3 GROUP_CTL, 4 JOIN_REQ), msg_id u16, seg u8 (idx<<4 \| count−1), chunk[≤ 900]`: a chunk of the **same signed logical body** as §4.7, reassembled per link and `msg_id` (≤ 2 concurrent per link, 5 s timeout) and then processed exactly like the flooded message (same acceptance rules) | 4 + chunk | link (anti-entropy, §8.9; P2 priority) |
| 0x10-0x7F | reserved (skipped) | | | |

### 4.5 MeshPacket header (9 bytes, relayable)

```
offset size field
0      1    vt        version (bits 7-5, = 1) | ptype (bits 4-0)
1      1    ttl_hops  ttl_remaining (bits 7-4, 0-15) | hops (bits 3-0, link traversals so far, saturating 15)
2      2    origin    node_id (u16)
4      1    sess      origin session tag: random non-zero u8 drawn at every engine start (never persisted)
5      2    seq       per-(origin, sess) sequence (u16, serial-number arithmetic); random initial value
7      1    age       time since creation at the origin, 2 ms units, saturating at 255 (= 510 ms)
8      1    flags     bits 7-6 class (0 ctrl, 1 voice, 2 bulk, 3 lab) | bit5 LEADER | bit4 SEG | bits 3-0 reserved (0)
9      …    payload
```

Relay rewrite (inside the link encryption, so headers are never visible on air): `ttl_remaining −= 1`, `hops += 1`, `age += local residence + incoming-link one-way estimate` (RTT/2 from PING/PONG). The origin sets `age` to its queueing delay before the first transmit. The dedup key is `(origin, sess, seq)` (§5.3).

### 4.6 Mesh packet types

| ptype | Name | Payload | Bytes | TTL | Class |
|---|---|---|---|---|---|
| 0x01 | VOICE | `burst u8` · `ts u16` (**10 ms ticks since the burst's first VOICE frame**: frame *n* of a 40 ms profile has `ts = 4n`; TALK START is sent in the same instant as frame 0; u16 covers 655 s ≫ `MAX_BURST`) · `cfg u8` (b7-6 dur 0=10/1=20/2=40/3=60 ms; b5-4 bw 0 NB/1 MB/2 WB/3 reserved; b3 LBRR; b2 DRED; b1 FIRST; b0 PREROLL) · `opus[1..=400]` | 4 + opus | 4 (policy 3-6) | voice |
| 0x02 | TALK | `burst u8, state u8 (1 START, 2 STOP, 3 KEEP every 1 s), prio u8 (1 member, 2 leader), mode u8 (1 vox, 2 ptt), profile u8 (1 BLE16, 2 BLE12, 3 WIFI20, 4 WIFI24FEC, 5 BLE-DRED), elapsed_10ms u16` (time since burst start at the origin when sent; 0 on START) | 7 | 6 | ctrl |
| 0x03 | FLOOR_REJECT | `target u16, target_sess u8, burst u8, reason u8 (1 cap, 2 preempted, 3 congestion, 4 outranked), at u16` | 7 | 6 | ctrl |
| 0x04 | PRESENCE | §4.6.1 | 14 | 15 | ctrl |
| 0x05 | ROSTER | segmented (§4.7) | ≤ 16 seg | 15 | ctrl |
| 0x06 | JOIN_REQ | segmented (§4.7) | 1 seg | 15 | ctrl |
| 0x07 | REKEY | segmented (§4.7) | ≤ 16 seg | 15 | ctrl |
| 0x08 | LEAVE | `reason u8 (1 user, 2 removed, 3 app-exit), sig[64]` (origin's per-group Ed25519 over `"cvic/v1/leave" ‖ group_id ‖ origin ‖ sess ‖ reason`; ignored if the signer key is unknown or the signature fails) | 65 | 15 | ctrl |
| 0x09 | RIDE_CTL | `op u8 (1 start, 2 end), ride_id u32 (random per Start ride), sig[64]` (origin's per-group Ed25519 over `"cvic/v1/ridectl" ‖ group_id ‖ epoch ‖ ride_id ‖ op`; `end` applies only to the current `ride_id`; a member's local "Start" is never flooded) | 69 | 15 | ctrl |
| 0x0A | MESH_PROBE | `probe_id u32, t_origin u64, n u8, trace[n] u16` (relays append their node_id, n ≤ 8) | 13 + 2n | 8 | lab |
| 0x0B | PATH_HINT | `profile u8 (1 BLE16, 2 BLE12), at u16, target u16 (0 = all)` | 5 | 3 | ctrl |
| 0x0C | GROUP_CTL | `op u8 (1 close-group, 2 transfer-leader), ctl_version u32, epoch u32, [new_leader_ed_pk[32], new_leader_node u16 if op 2], sig[64]` (current leader's Ed25519 over `"cvic/v1/groupctl" ‖ group_id ‖ preceding bytes`); accepted only if `ctl_version` > the stored one and `epoch` ≥ the current epoch | 73 / 107 | 15 | ctrl |
| 0x10 | POSITION | `lat_e7 i32, lon_e7 i32, speed_dms u16 (0.1 m/s), heading_2deg u8, acc_m u8, fix_age_100ms u8, rsv u8`; sent every 5 s moving / 20 s stopped, only while "Share my distance" is on | 14 | 15 | bulk |
| 0x11 | ALERT | reserved (convLink `ALT:` types) | — | — | ctrl |
| 0x1F | EXT | `ext_type u8` + body (future) | — | — | — |

#### 4.6.1 PRESENCE (every 2 s)

```
roster_ver16 u16 · epoch16 u16
state  u8  b7-6 ride (0 lobby, 1 riding, 2 ending) | b5-4 talk (0 idle, 1 talking, 2 blocked, 3 muted)
           | b3 leader | b2 on_call | b1 capture_ok | b0 live (went live)
audio  u8  b7-6 ladder (L0-L3, §7.8) | b5-3 route (0 none, 1 builtin, 2 hfp-nb, 3 hfp-wb, 4 le-audio, 5 car-hfp, 6 wired)
           | b2 tx_muted_no_headset | b1 render_ok | b0 sharing_distance
battery u8 (255 unknown) · links u8 (b7-4 BLE, b3-0 Aware)
caps   u8  b7-6 platform | b5 dred_ok (same libopus DRED version) | b4 radio_off_recently (BT was off within 60 s) | b3-0 reserved
interval u8 (100 ms units) · heard u32 (bitmap by roster index: members whose PRESENCE I received within 6 s)
```

**Truncated version fields (one width per concept).** `roster_version` and `epoch` are u32 wherever a message is authoritative (ROSTER, REKEY, GROUP_CTL, CAPS, invite). Advertising/presence fields carry the low 16 bits: PRESENCE `roster_ver16`, `epoch16` and HELLO `epoch_lo` are all `value mod 2¹⁶`, compared with serial-number arithmetic (a difference ≥ 2¹⁵ never occurs: one group changes epoch or roster a few times per ride). Names are **UTF-8 bytes**, `name_len` counts bytes, and encoders truncate at the last grapheme-cluster boundary that fits (display name ≤ 24 B, group name ≤ 32 B).

**Segmentation (`SEG` flag):** the payload starts with `msg_id u16` · `seg u8 = idx<<4 | (count−1)` (≤ 16 segments). Each segment is its own MeshPacket (own seq, deduped individually). Relays forward segments without reassembly. Receivers reassemble by `(origin, sess, msg_id)`, with ≤ 4 concurrent assemblies per origin, a 5 s timeout and ≤ 16 × 900 B.

### 4.7 Control message bodies (logical, before segmentation)

- **ROSTER:** `ver u8=1 · group_id[16] · epoch u32 · roster_version u32 · invite_sig[64] · n u8 · n × (node_id u16 · id_hash[16] · ed_pk[32] · role u8 (0 member, 1 leader, 0xFF denied) · name_len u8 · name[≤ 24 B]) · sig[64]`, Ed25519 by the leader's per-group key over `"cvic/v1/roster" ‖ preceding bytes` (≤ 2.6 KB at 32 entries, 3 segments). `ed_pk` lets every member verify the signed LEAVE/RIDE_CTL of any roster member (`id_hash` is checked against it plus the JOIN_REQ `x_pk`). `invite_sig` is the leader's signature for this epoch's invite (§8.3), so any member can re-share a leader-signed invite. The roster index is the entry position among non-denied entries (0-31). Denied entries stay listed so every node can refuse their IDPROOF.
- **JOIN_REQ:** `ver u8=1 · ed_pk[32] · x_pk[32] · platform u8 · name_len u8 · name[≤ 24 B] · sig[64]` (Ed25519 over `"cvic/v1/join" ‖ group_id ‖ preceding`).
- **REKEY:** `new_epoch u32 · roster_version u32 · leader_eph_pk[32] · n u8 · n × (node_id u16 · sealed[48]) · sig[64]`, with `sealed = ChaCha20-Poly1305(k_i, nonce = 0, epoch_secret_{new_epoch})`, `k_i = HKDF(X25519(leader_eph, x_pk_i), "cvic/v1/rekey" ‖ new_epoch ‖ node_id)`. Each `k_i` is used exactly once, because `leader_eph` is fresh per REKEY, so nonce 0 is safe. A REKEY seals the **newest** secret to every current roster member, so a member several epochs behind needs only the latest REKEY.
- **Acceptance and monotonicity (normative).** A node accepts a ROSTER only if its signature verifies against the current leader key and `roster_version` is **strictly greater** than the stored one (so a replayed older roster without a deny entry is ignored); a REKEY only if `new_epoch > epoch` and it contains a sealed entry for this node that opens (older or equal epochs are ignored); a GROUP_CTL only if `ctl_version` is strictly greater and `epoch ≥` the current epoch; a JOIN_REQ is idempotent (the leader keys it by `id_hash`). Each node stores the latest accepted ROSTER, REKEY and GROUP_CTL bodies verbatim for anti-entropy (§8.9).

### 4.8 Sizes and per-link load (design budget)

| Item | Bytes |
|---|---|
| BLE VOICE @ 16 kbps, 40 ms (Opus ≈ 80 B) | MeshPacket 93 → record 95 → SEC frame 116 → L2CAP 118 B ⇒ **23.6 kbps per stream** |
| Two BLE streams | 47.2 kbps uncoalesced (the design value: relays never wait to coalesce, and two origins' frames are not phase-aligned); 42.6 kbps when coalesced in one frame |
| BLE-DRED VOICE @ 20 kbps, 40 ms (Opus ≈ 100 B) | 113 → 115 → 136 → 138 B ⇒ 27.6 kbps per stream |
| Wi-Fi VOICE @ 20 kbps, 20 ms (Opus ≈ 50 B) | 63 → 65 → 86 B datagram ⇒ 34.4 kbps per stream |
| PRESENCE (15 members, 2 s) | 25 B record each ⇒ ≈ 1.5 kbps per link |
| POSITION (15 members, 5 s) | 25 B record each ⇒ ≈ 0.6 kbps per link |
| NBR (1 s, n = 4) + LSTAT + PING/PONG (2 Hz) | ≈ 0.9 kbps |
| **Control reserve `C_CTRL`** | **3 kbps per link** (sum of the three rows above) |
| **BLE link at S = 2** | `cost(2) = 2 × 23.6 + 3 = 50.2 kbps` ≤ 0.9 × 60 kbps = 54 kbps (the per-link budget of the research: 50 % of ~120 kbps) |

**Cost model used by the pacer and the cap (normative):** `cost(k, profile) = k × r_stream(profile) + C_CTRL` with the uncoalesced per-stream rates above. The research budgets **per link** (60 kbps) and notes that a relay pays `r × degree`; §5.6 therefore budgets node airtime from the number of links that actually carry voice, not from a fixed node total split evenly.

### 4.9 Versioning and capability negotiation

1. **Pre-connection:** HELLO (§9.3) carries `proto_min/proto_max`. The initiator picks the highest common version and writes it in HS `proto_ver`; no common version ⇒ `REJECT{1}`.
2. **Post-handshake:** the CAPS record. A feature is used only with capability: the DRED profile only when every roster member's PRESENCE `caps.dred_ok` is set (same libopus DRED bitstream version); an Aware upgrade only when both CAPS have the matching transport bit.
3. **Mesh layer:** the `vt` version bits change only for incompatible header changes. New ptypes and records are additive (skip and relay rules).
4. **Group policy** (voice TTL, cap overrides, BLE frame size, leader priority) travels in the invite `policy` byte and the roster. Defaults are compiled in.
5. **UI contract:** `api_version()` major must match the plugin's; otherwise the UI shows "update required".

### 4.10 Golden vectors (protocol freeze)

`mesh-wire::vectors::generate()` and `mesh-crypto::vectors::generate()` write `test-vectors/{wire,crypto}/*.json` via `cargo xtask vectors`; `--check` fails CI on any byte change. Each vector has `name`, `inputs` (hex keys, fixed ephemerals, fields) and `expected` (hex). Coverage: every LinkFrame type, L2CAP and GATT chunking (including MTU 23 and 517), every record, every mesh type, segmentation, HS msg1-3 with fixed keys, raw-split keys, SEC frames in both phases (incl. the AAD bytes and a key update), IDPROOF, invite blob + `invite_sig`, HELLO value, adv tokens, roster/JOIN/REKEY/GROUP_CTL (close and transfer), signed LEAVE/RIDE_CTL, SYNC chunking, TALK with `elapsed_10ms`, 14-byte PRESENCE. Swift and Kotlin tests replay the HELLO and advertising vectors, the only bytes they build themselves (from values the core gives them).

## 5. Mesh routing

Model: **stateless managed flooding with immediate voice relay.** OLSR-class proactive routing converges more slowly than convoy churn (HELLO 2 s / hold 6 s). In a chain every interior node must relay anyway, and BLE links are unicast, so overhearing-based suppression does not apply.

### 5.1 Neighbor table

Keyed by `node_id` after IDPROOF. Each neighbor has a **link bundle** (≤ 1 link per kind: BLE, Aware, Tunnel) and:

| Field | Source |
|---|---|
| `links[kind] → {link_id, role, max_frame, srtt, rttvar, min_rtt, loss_ewma, rssi, pace_rate, q, congested_until}` | PING/PONG, LSTAT, platform metrics, scheduler |
| `nbr: {nseq, at, entries: [(node_id, q, good, congested, kind, vcap)]}` | NBR record (on change ≤ 4 Hz, plus 1 s heartbeat); `vcap` is informational (diagnostics and the origin's `cap_local`), never a pruning input for voice |
| `platform, caps, role` | CAPS |
| `last_rx` | any inbound frame (liveness is any frame, never only PING; convLink lesson) |

**Link quality:** `q = 100 · (1 − loss)² · f_rtt · f_rssi`, where `f_rtt = 1` if `srtt − min_rtt < 30 ms`, falling linearly to 0.5 at 150 ms; `f_rssi = 1` if RSSI ≥ −75 dBm, falling linearly to 0.6 at −95 dBm (BLE only). `loss` is an EWMA (α = 0.2 per second) of SEC counter gaps (datagram links) and PING loss. `Q_GOOD = 60`. UI bars: ≥ 80 → 4, ≥ 60 → 3, ≥ 40 → 2, > 0 → 1.

**Congested** (per outgoing link): set for 2 s whenever a voice frame was dropped on that link (queue limit or deadline) or `WouldBlock` was returned. A cap below `S_MAX` is **not** by itself congestion: it is reported through `vcap`, so a link whose pacer sits just under two streams does not flap the congested bit.

**NBR freshness:** a report is fresh if received < 1.5 s ago. An NBR is sent immediately (rate limit 4 Hz) whenever the sender's set of good links, any congested flag or any vcap changes, and at least every 1 s. A link going down triggers an NBR at once. That bounds staleness to about 250 ms plus one hop.

**Link death:** no inbound frame for 4 s closes the link. PING every 500 ms while any burst is active or in Lab, otherwise every 2 s (this also keeps Wi-Fi Aware from idling out).

### 5.2 Forwarding (normative)

```
on authenticated MESH record P from neighbor N over link L:
  if P.version != 1 or malformed            → drop; N.penalty += 1
  if P.hops == 0 and P.origin != N.node_id  → drop; N.penalty += 5       # first-hop origin check
  if dedup.seen(P.origin, P.sess, P.seq)    → stats.dup += 1; return
  dedup.mark(P.origin, P.sess, P.seq)
  age_now = P.age*2 + owd(L) + residence
  deliver_locally(P)                         # unless ptype unknown
  if P.ttl_remaining <= 1 → return
  if P.class == voice and age_now > VOICE_MAX_AGE → stats.stale += 1; return
  for each neighbor M ∉ {N, P.origin}, using M's best link for P.class:
      if P.class != voice and pruned(N, M, P) → continue                # §5.4: voice is never pruned
      if P.class == voice and !floor.admits(link(M), burst(P)) → maybe FLOOR_REJECT (§6.3); continue
      enqueue(link(M), P with ttl−1, hops+1; age rewritten at dequeue)
```

- `N.penalty` ≥ 20 per minute ⇒ disconnect and ban that identity for 60 s. Per-neighbor ingress caps: 300 records/s and 64 KB/s.
- **Origination** sends to **all** neighbors (no pruning at the origin). That anchors the pruning induction.
- **Relay jitter:** voice 0 ms (unicast BLE has no collision to avoid). Control 5-25 ms random (lets records coalesce). Control packets also use counter suppression: if the same `(origin, sess, seq)` arrived from ≥ 3 distinct neighbors before the jittered send, drop the pending copy for neighbors that already sent it to us.

### 5.3 Dedup window (after authentication only)

Dedup state is updated only for records inside a SEC frame whose tag verified (or a LAB frame on a Lab link). A key is `(origin, sess)`: highest seq `H`, 1024-bit bitmap behind `H`, last-update time. For `d = (seq − H) mod 2¹⁶`: `1 ≤ d < 32768` ⇒ newer (shift the window); otherwise, if within 1024 behind ⇒ check and set the bit; otherwise ⇒ drop as too old. There is **no "far behind means restart" heuristic**: a restarted origin draws a new random `sess`, which starts a fresh window, and PRESENCE from the new `sess` immediately evicts the old window for that origin. Windows expire after 5 s of silence. A same-`sess` collision after a restart (probability 1/255) can only drop frames whose seq falls inside the old window, and a restart needs a user tap (≫ 5 s), so the old window has normally expired. Memory ≤ 2 windows × 31 origins × 140 B.

### 5.4 Two-hop pruning (neighbor elimination), ctrl and bulk only

**Voice is never pruned.** A safe voice skip would require X to know whether N actually forwarded burst P to M, but admission is per link and stateful (`T_STICKY`, arrival order, per-link `cap_pacing`, a `vcap` up to 1.5 s stale): N's floor table can hold bursts X never saw, so "same total order" does not imply "same admitted set" (review finding, accepted). Voice fan-out is instead bounded by the BLE link max (3 by default, §5.7): a relay forwards each voice frame on at most 2 links, so a 15-node cluster costs ≤ 14 × 2 + 3 = 31 transmissions per originated voice frame, and each link still carries ≤ cap streams (§6.3). In chains pruning never triggered anyway.

For **ctrl and bulk** classes (PRESENCE, POSITION, ROSTER/REKEY segments, LEAVE, RIDE_CTL), relay X, having received P from N, skips neighbor M only if **both** hold:

1. N's NBR is fresh (< 1.5 s) and lists M with `good = 1` and `congested = 0`;
2. P is not a segment of a message whose reassembly X saw time out from N (N may be shedding it).

These classes are not floor-admitted, so condition 1 covers every case in which N could have dropped P (a degraded or congested N→M link); a residual loss is repaired by the next 2 s PRESENCE and by anti-entropy (§8.9). By induction back to the origin, which never prunes, every skipped M was served by an earlier node. A property test checks **pruning equivalence** for ctrl/bulk on random geometric graphs (pruned flooding delivers to exactly the nodes plain flooding reaches within the TTL), a second property test runs **random floor contention with per-link caps** and asserts that every node whose links admit a burst receives it (i.e. voice delivery never depends on pruning), and simulator scenario S11 measures loss with stale tables (a link that broke less than one NBR period ago).

Effects: in chains (degree ≤ 2) pruning never triggers and every interior node relays deterministically. In clusters (stops, traffic lights) pruning removes most control duplicates; S3 asserts ≤ 2.25 × (N−1) transmissions per originated voice frame in a 15-node cluster (the degree bound) and that pruned control floods use fewer transmissions than unpruned ones (the saving is recorded as a metric, not a threshold).

### 5.5 TTL and latency bounds

| Class | Initial TTL | Extra bound |
|---|---|---|
| VOICE | 4 (policy 3-6) | `VOICE_MAX_AGE = 150 ms` (policy 100-250): relays drop older frames; receivers count them as late |
| TALK / FLOOR_REJECT | 6 | — |
| PRESENCE / POSITION / ROSTER / JOIN / LEAVE / RIDE_CTL / REKEY / GROUP_CTL | 15 | — |
| MESH_PROBE | 8 | — |
| PATH_HINT | 3 | — |

Why 150 ms and not the research's 250 ms: on the BLE profile the non-network parts of the mouth-to-ear path already cost about 150 ms (§7.9), so network transit plus jitter buffer must fit in about 150 ms to keep p95 ≤ 300 ms. Dropping a stale frame is better than delaying the whole stream. **Consistent hop statement (used in §1.1, §7.9 and the UI):** KPI3 (p95 < 300 ms incl. headset legs) is promised for **≤ 2 BLE hops**. Hops 3-4 are relayed best effort while the age cap allows, which on BLE usually means 3 hops at most; the snapshot labels such members `voice: "bestEffort"`. The research's "voice TTL ≤ 3 (max 4)" is kept as the TTL policy range.

### 5.6 Per-link queues, pacing and node airtime

- **Strict priority per link:** P0 link control (HS, PONG, IDPROOF, CLOSE, NBR on change) and TALK/FLOOR_REJECT → P1 voice → P2 presence/position/roster/LSTAT (≤ 10 % of the pace rate) → P3 lab/bulk. Voice queue: ≤ 2 frames per origin per link (drop oldest); whole-link voice backlog ≤ 60 ms of pace-rate drain; deadline = `VOICE_MAX_AGE`. Records are coalesced up to `max_frame`.
- **Stream-level shedding:** when a link must shed voice, it drops the **lowest-ranked burst entirely** (no further frames of that burst on that link until it ends), marks the link congested, and the next NBR advertises the lower vcap. It never drops random packets across bursts.
- **Delay-based pacing:** a token bucket at rate `R` per link (initial L2CAP 60 kbps, GATT 40 kbps, Aware 2 Mbps, Tunnel 64 kbps; bounds [16 kbps, 4 Mbps]). Every 500 ms: `qd = srtt − min_rtt` (min over 30 s). If `qd > 40 ms` or a `WouldBlock` happened ⇒ `R ← 0.85 R`. If `qd < 15 ms` and the queue was non-empty ⇒ `R ← R + 4 kbps` (BLE) or `×1.1` (Aware). BLE is reliable at the link layer, so overload shows up as **delay**, not loss; the controller keeps OS buffers short.
- **Node airtime budget (voice-aware):** all BLE links of a node share one radio (and the helmet's eSCO). A BLE link is a **voice link** if it carried voice in the last 2 s, otherwise **idle**. Idle links are capped at a control-only floor `R_IDLE = 8 kbps` (their `R` estimate is kept, only the grant is limited). Voice links share the rest: `Σ grant(voice links) ≤ NODE_BLE_BUDGET − Σ grant(idle links)`, with `NODE_BLE_BUDGET = 180 kbps` [TUNE: T0c]; when the sum of their `R` would exceed it, **only the voice links** are scaled, proportionally. Sizing: 3 voice links × 60 kbps = 180 kbps, i.e. the research's 60 kbps per-link budget at the link max; at the default target of 2 links no scaling ever occurs. If T0c shows the radio cannot sustain it with HFP active, Appendix B lowers the budget **and** the link max together so that `links_max × cost(2) ≤ 0.9 × NODE_BLE_BUDGET` keeps holding.
- **Caps from pacing (with hysteresis):** `cap_pacing(L) = max{ k ∈ 1..S_MAX[kind] : cost(k, profile) ≤ 0.9 × grant(L) }`, with `cost` from §4.8 (uncoalesced, plus `C_CTRL`). For BLE16 this gives cap 2 whenever `grant ≥ 55.8 kbps` and cap 1 down to 29.6 kbps. The published cap **drops only after the raw value has been lower for 2 s** and **rises only after it has been higher for 5 s**, so a single multiplicative decrease (60 → 51 kbps, recovered by the additive increase within ~1.5 s when the queue is not building) never flaps the cap. **Voice floor (2026-10-24):** the multiplicative decrease never takes the rate below one stream of the lowest profile of the link kind (BLE12 `cost(1)/0.9` = 25 112 bps on BLE; 41 556 bps on Aware/Tunnel), a BLE12 grant counts as cap 1, and a lone admitted burst is never shed (fading is not congestion). Recovery below the initial rate needs only a period without a congestion signal (`qd ≤ 40 ms`, no `WouldBlock`); queue delay uses `min(srtt, sample)`. If `grant` stays below `cost(1)` for 3 s, the relay floods `PATH_HINT{BLE12}`; origins that hear it use the 12 kbps profile for 30 s. **BLE-DRED** (27.6 kbps per stream) needs `grant ≥ 64.7 kbps` for cap 2, so it is enabled only on groups whose links' pacers sustain that (Phase 5 experiment, §7.4).
- **Steady-state guarantee (property test, WP-09):** with BLE16, default `R`, no loss and no queueing delay, a chain interior node (2 voice links), a 3-link hub with 3 voice links, and a 3-link node with 2 voice links + 1 idle link all keep `cap = 2` on every link for a 10 min simulated run; one injected `WouldBlock` lowers the cap for at most 2 s + recovery.

### 5.7 Link selection policy (every 2 s)

- **Targets:** BLE links **target 2, max 3** per node [TUNE: T0c may raise them to 3/4 together with `NODE_BLE_BUDGET`]. The per-device max is lowered by 1 for 10 min after a `LimitReached` link failure (§3.2). Aware max `min(4, available data paths)`. Distinct neighbors max 6.
- **Under target, or some roster member unreachable:** discovery mode `Forming`. Connect to candidates ordered by (a) an advertising `node_token` that maps to an unreachable member, (b) an unknown token (e.g. a backgrounded iPhone), (c) RSSI. One outstanding connect per transport, 8 s attempt timeout, per-peer backoff 5 s → 60 s (doubling), reset on a new adv token. Role rules of §9.2 apply.
- **Over max:** close the link to a neighbor that appears `good` in another neighbor's fresh NBR (redundant) with the worst `q`. Never close a link that is the only path to a reachable member (articulation check on the 2-hop graph).
- **Duplicate links** to the same node and kind (simultaneous same-OS connects): after IDPROOF, keep the link whose initiator has the lower `node_id`; close the other with `CLOSE{4}`. Key updates happen in band (§4.3), so a long-lived link never needs a second link to the same peer.

### 5.8 Presence, "who hears me", partitions

- Each node floods PRESENCE every **2 s**. A member is **reachable** if heard within **6 s**. Hops and path age come from the latest PRESENCE copy.
- `heard` bitmap ⇒ any node computes **who hears me** (members whose bitmap has my bit), **one-way** members (I hear them, they do not hear me), **voice reach** per member from PRESENCE `hops` and `age`: `kpi` (≤ 2 BLE hops or all-Aware path within the age cap), `bestEffort` (3-4 hops within TTL and age cap), `no`; and **islands** (connected components of the "hears" graph).
- PRESENCE `roster_ver16`/`epoch16` from a **direct neighbor** (hops = 0) that are lower than mine trigger the anti-entropy push of §8.9.
- Events (i18n keys): `memberLost` after 6 s of silence, `memberBack` on the first PRESENCE after a loss, both aggregated over 5 s ("3 riders out of range"); `alone` when no member has been reachable for 10 s; `partition` when the island count changes and stays changed for 6 s. Discovery stays `Forming` while any member is missing.
- Unconfirmed members (not yet in the roster) are tracked by `node_id` only (no bitmap bit).
- **Positions:** when a member's POSITION is newer than 30 s and my own fix is valid, the snapshot carries distance (haversine) and bearing, and "ahead/behind" relative to my heading.

## 6. Floor control

Model: **optimistic floor.** A talker who sees room in its local view starts sending at once, so there is no request/grant round trip and PTT/VOX access time is local processing only (MCPTT KPI1 < 300 ms). Correctness comes from three things. Every node ranks bursts with the same **total order**. Every relay **hard-enforces** the cap per outgoing link. Nodes near a refused talker tell it to stop. This follows the Cardo precedent "the first N that speak are heard".

### 6.1 Constants

| Name | Value | Note |
|---|---|---|
| `S_MAX_BLE` | 2 | per outgoing BLE link |
| `S_MAX_AWARE` | 4 | per outgoing Aware link |
| `S_MAX_TUNNEL` | 4 | v2 |
| `T_TIE` | 150 ms [TUNE] | hash smear that makes the rank order total |
| `T_STICKY` | 300 ms | after this, an admitted burst can only be evicted by higher priority |
| `FLOOR_IDLE_RELEASE` | 600 ms | slot freed when no voice/TALK from an admitted burst |
| `MAX_BURST` | 60 s PTT / 45 s VOX | the origin ends the burst with a tone; the next onset gets a fresh rank |
| `TALK KEEP` | every 1 s | refreshes relays and late joiners during long bursts |
| Priority | leader 2, member 1 | leader priority is a group policy (default on) |

### 6.2 Floor table and ranking

Every node keeps `FloorTable { active: ArrayVec<ActiveBurst, 8> }`. An entry is created by the first TALK START **or** VOICE packet of a burst and removed on TALK STOP, on FLOOR_REJECT for it, or `FLOOR_IDLE_RELEASE` after its last packet.

```
BurstKey   = (origin, sess, burst)
t_origin   = local_rx(P) − age(P) − rel(P)          # P = first packet of the burst THIS node saw; no wall clock
             rel(P) = ts × 10 ms for VOICE, elapsed_10ms × 10 ms for TALK (0 for START)
h          = SHA-256("cvic/v1/floor" ‖ epoch ‖ origin ‖ sess ‖ burst)[0..4] as u32
eff_start  = t_origin + (h / 2^32) × T_TIE
rank order = (prio DESC, eff_start ASC, origin ASC)                   # strict total order
```

Because VOICE `ts` and TALK `elapsed_10ms` are burst-relative, a node that missed TALK START (late join, relink, partition) computes the same `t_origin` as everyone else, up to the age-estimation error, so a 20 s-old burst ranks as old everywhere. A floor test covers a node that links up mid-burst and must agree on the admitted set within one TALK KEEP period. The smear makes the order total and consistent while still acting first-come-first-served for starts more than 150 ms apart. Two nodes disagree only when two effective starts are within the age-estimation error (tens of ms), and FLOOR_REJECT resolves that, because rejection only shrinks the talking set.

### 6.3 Relay admission (per outgoing link L, hard enforcement)

```
cap(L)    = min(S_MAX[kind(L)], cap_pacing(L), group_cap_override?)   # cap_pacing with hysteresis (§5.6)
vcap(L)   = cap(L)                                             # advertised in NBR (§4.4); informational, not a voice-pruning input
admitted(L) = entries currently forwarded on L

admits(L, b):
  if b ∈ admitted(L): refresh; return true
  if |admitted(L)| < cap(L): admit; return true
  victim = lowest-ranked entry of admitted(L)
  if b.prio > victim.prio
     or (b ranks above victim and now − victim.admitted_at < T_STICKY):   # near-simultaneous starts converge on rank
       evict victim (stop forwarding it on L);
       if victim.prio < b.prio: flood FLOOR_REJECT{victim, PREEMPTED} else FLOOR_REJECT{victim, OUTRANKED}
       admit b; return true
  if (b not yet rejected by me): flood FLOOR_REJECT{b, CAP (or CONGESTION when cap_pacing < S_MAX), at = me}
  return false
```

- Equal-priority bursts never evict each other after `T_STICKY`, so a sentence is never cut mid-way by a peer.
- Relays never forward voice of a burst outside `admits(L, b)` on L. Even if every talker ignored FLOOR_REJECT, no link would carry more than its cap.
- **Near-talker rejection (from B):** a node that receives VOICE with `hops ≤ 1` for a burst outside the global top `group_cap` of its table (2 if any BLE link exists in the group, else 4) sends one FLOOR_REJECT{OUTRANKED} (TTL 4) per burst, so far-side conflicts are reported within 2 hops.
- **Per-segment caps and edge links (2026-10-24, hybrid groups):** the near-talker `group_cap` is evaluated per segment: 4 for bursts heard from the Aware backbone, 2 inside BLE segments. An **edge link** (a BLE hop leaving the Aware backbone) admits purely by rank: it carries the top `cap(L)` live bursts known on it, refused or displaced bursts wait and return by rank, queue-shed bursts stay sticky but keep their rank slot. There is no `T_STICKY` first-come rule on edges and edges **shed locally without FLOOR_REJECT**. Non-edge links keep the pseudocode above.
- Receivers mix at most 4 streams, chosen by rank.

### 6.4 Origin talk state machine (VOX and PTT)

```
States: MUTED | IDLE | TALKING | BLOCKED
Inputs: vad_on / vad_off | ptt_down / ptt_up (RT flag) | floor_reject(me) | slot_freed | mode change | MAX_BURST

IDLE --(VOX vad_on | PTT ptt_down)--> local_admit?
      local_admit = my prospective burst (prio, eff_start = now + h·T_TIE) ranks inside cap_local
                    (cap of my most constrained link kind) given my table;
                    cap_local ignores edge links and BLE fallback links to a neighbour I also reach over Aware
      yes → TALKING: new burst id, TALK{START, profile}, encode pre-roll (40 ms) + live, FIRST on first VOICE
      no  → BLOCKED (PTT: busy tone now; VOX: busy tone only after 600 ms of sustained speech)
TALKING --VOX vad_off--> TALK{STOP} → IDLE      # vad_off is emitted by the VAD only after its 300 ms hangover (§7.3);
                                                 # there is no second FSM hangover (single 300 ms hangover in total)
TALKING --PTT ptt_up | MAX_BURST--> flush encoder, TALK{STOP} (sent twice, 60 ms apart) → IDLE
TALKING --floor_reject(me, burst)--> stop TX at once, TALK{STOP}, busy/preempt tone → BLOCKED
BLOCKED --slot_freed and (VOX still vad_on | PTT still down)--> go-ahead chirp → TALKING (new burst)
BLOCKED --VOX: vad_off  | PTT: ptt_up--> IDLE ; VOX holdoff 1.5 s + rand(0..500 ms) before re-evaluating
any --mode = muted, hard-mute flag, or built-in-mic mute (moto, no headset)--> MUTED (capture keeps running; nothing sent)
```

### 6.5 Cues and VOX hygiene

| Cue | Sound (synthesized in mesh-voice, mixed inside the AEC reference) | Rate limit |
|---|---|---|
| Busy | 2 × (480 Hz, 150 ms on, 100 ms off), −18 dBFS | PTT: per press, repeated every 2 s while held; VOX: once per 10 s |
| Preempted | 660/550/440 Hz × 80 ms | per event |
| Go-ahead (PTT) | 880 Hz, 40 ms | per event |
| Ride started | 440→660→880 Hz × 80 ms + prompt | once |
| Member joined/back | 660→880 Hz, 2 × 80 ms | aggregated per 5 s |
| Member lost | 880→660 Hz, 2 × 80 ms | aggregated per 5 s |
| Alone | 2 low pulses (300 Hz) | once per 60 s |
| Mic lost (listen-only) | 3 × 300 Hz, 150 ms + prompt | once per 60 s |
| Headset lost (TX muted) | 2 × 400 Hz + prompt | per event |
| Bluetooth off / airplane mode | 3 descending 600→300 Hz × 100 ms + prompt "Bluetooth is off: intercom paused" | per event, repeated every 60 s while off |

- **Wind-hog protection (from C):** if my VOX is denied ≥ 3 times in 30 s, or a burst's mean voice probability is < 0.5 after 1.5 s, the burst is released and the VOX threshold rises one level for 2 min (UI and prompt: "VOX less sensitive: wind").
- **PTT robustness:** the button uses pointer capture, and `ptt{down:false}` is sent on `pointerup`, `pointercancel` and `visibilitychange`. Latch mode toggles on tap. The lock-screen **Talk** latch (`LatchTalkIntent` / Android notification action) sets RT flag bit3, which the engine treats exactly like a latch tap; the Live Activity shows the latched state. MAX_BURST guards against a stuck PTT or a forgotten latch.
- Snapshot exposes `talkers[]`, my talk state and `talkDenied` events for the UI banner.

## 7. Voice pipeline

### 7.1 Signal path

```
capture (OS: HFP mic / built-in)                                       playout (OS: HFP speaker / speaker)
  │ iOS raw VPIO AU (AEC+NS+AGC, 16 kHz Int16 client format)              ▲ iOS VPIO output bus (= AEC reference)
  │ Android Oboe VoiceCommunication preset (platform AEC/NS/AGC)          │ Android Oboe VoiceCommunication usage
  ▼                                                                       │
mesh_rt_capture ─► capture ring (256 ms) ─► engine tick (5 ms) ─────►     render ring (target 20 ms) ◄─ engine
  ▼ 10 ms blocks @ 16 kHz                                                 ▲ 10 ms blocks
HPF 80 Hz ─► wind guard ─► VAD (nnnoiseless p + noise-floor gate) ─► talk FSM ─► pre-roll 40 ms ─► Opus enc ─► VOICE
                                                                          ▲
per-origin jitter buffer ─► Opus dec (LBRR / deep PLC / NoLACE) ─► gain ─► mixer (≤ 4 + earcons + prompts) ─► limiter
```

### 7.2 Capture front end

- **Rate:** 16 kHz mono Int16 at the C ABI on both OSes (VPIO client-format conversion on iOS; Oboe resampler `Medium` on Android). Narrow-band routes (CVSD 8 kHz) are upsampled by the OS. The core knows the real capture bandwidth from `AudioRouteInfo.input_rate` and, on 8 kHz routes, sets Opus `MAX_BANDWIDTH = NB`: **encoder bandwidth follows the sender's capture route**, never a fixed WB. **v1 ceiling is wideband:** 32/48 kHz routes (LE Audio LC3-SWB, wired) are captured at 16 kHz through the OS resampler; SWB is a v2 RT-ABI change (§3.6), and VOICE `cfg.bw = 3` is reserved.
- **Platform APM first:** iOS VPIO (AEC, NS, AGC on; other-audio ducking `{advanced: true, level: min}` so navigation prompts stay audible). Android `VOICE_COMMUNICATION` preset; `AcousticEchoCanceler.isAvailable/getEnabled()` is reported in diagnostics. No ML noise suppression in v1, because stacking three NS stages causes artifacts. A software APM slot (`trait Apm`) is reserved for v1.x speakerphone-in-car cases.
- **HPF:** 2nd-order Butterworth at 80 Hz (wind rumble), in Rust.
- **Built-in-mic mute (motorcycle):** when the route has no headset mic, the RT flag bit2 is set and nothing is transmitted unless the rider overrides it (`setBuiltinMicTx`). PRESENCE `tx_muted_no_headset` tells the group.

### 7.3 VAD / VOX

- **v1 detector:** `nnnoiseless` (pure-Rust RNNoise), used **only** for its per-frame voice probability `p` (the denoised output is discarded). Our 10 ms / 16 kHz block is upsampled ×3 to 480 samples @ 48 kHz by a 48-tap polyphase FIR (about 0.2 % of a core).
- **Decision:** onset when `p ≥ θ_on` for 2 consecutive blocks **and** block RMS ≥ noise floor + margin; release when `p < θ_off` for the hangover. The noise floor is a slow minimum tracker (rise 3 dB/s, instant fall).

| Sensitivity | θ_on | θ_off | margin | hangover |
|---|---|---|---|---|
| High | 0.50 | 0.30 | 4 dB | 300 ms |
| Medium (default) | 0.65 | 0.40 | 6 dB | 300 ms |
| Low | 0.80 | 0.50 | 9 dB | 300 ms |

- **Wind guard:** on the pre-HPF signal, if `E(<200 Hz) / E(300-3400 Hz) > 6 dB`, onset is suppressed unless `p ≥ 0.9`.
- **VOX test meter** in Settings (live `p` and gate state) via `audioCheck{start, mode:"voxMeter"}` and its `audioCheckSample{vadP, gate}` events. The VOX-calibration bench (§12.2) records decisions against PTT ground truth.
- **Upgrade path:** `trait Vad { fn process(&mut self, block: &[i16; 160]) -> f32; }`. Silero v5 sits behind cargo feature `vad-silero` (`tract-onnx`, pure Rust, 32 ms windows). It is switched on only if Lab A/B data on real wind recordings (opt-in, local) shows a gain.

### 7.4 Encoder profiles (per origin, chosen at burst start from its own links and PATH_HINT)

Common settings: `OPUS_APPLICATION_VOIP`, `OPUS_SIGNAL_VOICE`, 16 kHz API, VBR + constrained VBR, **DTX off** (the external VAD gates bursts), `DRED_DURATION = 0`, and the encoder is reset at each burst start. Never 10 ms or 60 ms frames.

| Profile (TALK id) | When | Frame | Bitrate | Bandwidth | LBRR | `PACKET_LOSS_PERC` | Complexity |
|---|---|---|---|---|---|---|---|
| **BLE16** (1, default) | any of my Up voice-egress links is BLE (BLE fallback links included) | **40 ms** | 16 kbps | WB (NB on 8 kHz capture) | **off**; **on** at the edge when smoothed loss ≥ 3 % (off after 10 s below 2 %) | 0 (LBRR off) / `clamp(smoothed_loss, 5, 25)` (LBRR on) | 8 |
| **BLE12** (2) | PATH_HINT heard, own `cap_pacing = 1`, or thermal ≥ serious | 40 ms | 12 kbps | WB (NoLACE helps at the decoder); NB on 8 kHz capture | off | 0 | 8 (5 when thermal) |
| **WIFI20** (3) | all my links are Aware/Tunnel and worst-neighbor smoothed loss < 3 % | 20 ms | 20 kbps | WB | off | 0 | 9 |
| **WIFI24FEC** (4) | all Aware/Tunnel and smoothed loss ≥ 3 % (hysteresis: back below 2 % for 10 s) | 20 ms | 24 kbps | WB | **on** | `clamp(smoothed_loss, 5, 25)` | 9 |
| BLE-DRED (5, experiment, Phase 5) | every roster member `dred_ok`, Lab shows bursty loss, LBRR off, and every own BLE link's grant ≥ 64.7 kbps (cap 2 at 27.6 kbps/stream, §5.6) | 40 ms | 20 kbps | WB | **off** | measured | 8 |

Rules from the verified FEC analysis: with LBRR off the loss hint is **always 0**, because a non-zero hint lowers libopus's equivalent rate and can drop the audio bandwidth below WB. LBRR is used at 24 kbps with ≥ 3 % smoothed loss and, since 2026-10-24, on BLE16 at the edge of range under the same 3 % / 2 % for 10 s hysteresis (Appendix C). Relays never transcode, so a Wi-Fi-profile stream may still be relayed onto a BLE edge (open issue: Aware-only origins). A burst that starts on BLE16 may downshift to BLE12 mid-burst when its link degrades. DRED and LBRR are never enabled together, and DRED requires the group-wide `dred_ok` capability. A **BLE-fast** profile (20 ms frames, −20 ms latency) is enabled by group policy only if T0 shows ≥ 150 kbps sustained with the headset active. One identical libopus binary runs on iOS and Android, and its capabilities travel in CAPS and PRESENCE.

### 7.5 Receive side: per-origin jitter buffer (NetEq-lite)

- Keyed by `(origin, sess)`, never by previous hop; a new `burst` id resets the burst-relative timeline. Duplicates are already removed by routing. Placement is by `ts` (10 ms ticks since the burst's first frame); jitter statistics use the first copy's arrival time.
- **Target delay `D`:** initial 60 ms (BLE) / 40 ms (Wi-Fi). Updated per packet from the p95 of `(arrival − expected)` over the last 50 packets, clamped to `[frame + 20 ms (+ 1 frame when LBRR), 200 ms]`.
- **Burst-friendly adaptation:** each burst starts at the current `D`. Within a burst, if buffered audio exceeds `D + 30 ms`, drop one 10 ms sub-block whose energy is < −40 dBFS or below the stream's noise floor + 3 dB (max one drop per 100 ms). This also absorbs the 40 ms pre-roll. Underrun ⇒ PLC.
- **Loss:** if a frame is missing and the next packet has LBRR ⇒ `decode_fec`; otherwise deep PLC (decoder complexity 7, which also enables NoLACE). After 100 ms of consecutive loss, fade to silence over 20 ms. A burst ends on TALK STOP or 600 ms without frames ⇒ flush and reset the decoder.
- Decoder pool: 6 preallocated Opus decoders (≤ 4 active). No allocation on the receive path.

### 7.6 Mixer, limiter, earcons

The mixer sums ≤ 4 decoded streams chosen by floor rank, each attenuated −3 dB when ≥ 2 are active. Earcons and prompts are summed **before** the limiter, so they are inside the VPIO / platform AEC reference. The limiter is feed-forward (threshold −3 dBFS, attack 1 ms, release 50 ms, no look-ahead). Per-stream slow AGC is v1.x.

### 7.7 Spoken prompts

- Every audible state has an earcon (§6.5), which is the **guaranteed** cue. Spoken prompts are an addition, on by default and rate-limited to ≤ 1 per 4 s (queued prompts merge).
- Templates come from the UI's `prompts` i18n namespace via `setPrompts`. The core asks the platform to render them with `render_prompts` (iOS `AVSpeechSynthesizer.write(_:toBufferCallback:)`, Android `TextToSpeech.synthesizeToFile`), and the PCM returns through `on_prompt_pcm` into a bounded prompt bank (≤ 512 KB).
- **Pre-rendering:** all fixed prompts and the names of members known in the lobby are rendered at Go live, while the app is in the foreground. Names learned mid-ride are rendered on demand. If rendering fails or is not possible in the background (verified in T2), that prompt falls back to its earcon. Prompts never become the only cue until T2 verifies background rendering.
- Keys: `intercomLive{n}`, `memberJoined{name}`, `memberLeft{name}`, `ridersOutOfRange{n}`, `alone`, `micPaused`, `micBack`, `headsetLost`, `bluetoothOff`, `batteryLow`, `talkLimit`, `voxLessSensitive`, `rideStarted`, `leaderChanged{name}`, plus `demo.line1…demo.line8` (Demo-ride bot lines, §3.4). Default templates live in the app's `prompts` catalog (WP-18), and the app sends them with `setPrompts` on every `startEngine`.

### 7.8 Routes, interruptions and the degradation ladder

The voice route is opened **once per session** at Go live and held. SCO setup can take seconds and Android device-change callbacks up to 30 s, so it is never opened per PTT press. The platform reports every route change; the core switches the encoder bandwidth and the snapshot route chip.

| Level | Meaning | Rider hears | Peers see | Exit |
|---|---|---|---|---|
| **L0** | Full: capture and render running | group + cues | normal | — |
| **L1** | Restarting (interruption ended, route change, watchdog) | cues as available | `capture_ok = 0` briefly | success → L0; failure → L2 |
| **L2** | Listen-only: capture lost, output-only fallback | group + "mic paused" prompt | "Ana: mic paused" | foreground tap / Resume mic → L0 |
| **L3** | Relay-only: audio dead entirely, process kept alive by the location anchor (iOS) or the FGS (Android) | nothing; notification | "Ana: no audio (relaying)" | foreground tap → L0 |

- **Phone calls:** iOS interrupts the session; Android loses SCO to the dialer. PRESENCE `on_call`; TX off. Resume after the call (iOS `shouldResume` or iOS 27 `resumptionRecommendation`; Android re-`setCommunicationDevice`).
- **Music:** while HFP is open, other apps' music drops to HFP mono. This is documented in onboarding (R13) and not solvable in v1.

### 7.9 Latency budget (one way, mouth to ear, design targets in ms)

| Stage | BLE 1 hop | BLE 2 hops | BLE 3 hops | Aware 1 hop (20 ms) |
|---|---|---|---|---|
| Headset mic + eSCO up (T5a measures) | 25 | 25 | 25 | 25 |
| OS capture IO (10 ms) + VPIO/APM | 15 | 15 | 15 | 15 |
| Engine tick alignment (5 ms tick, avg) | 3 | 3 | 3 | 3 |
| Opus framing + look-ahead | 46.5 | 46.5 | 46.5 | 26.5 |
| Encode + packetize | 1 | 1 | 1 | 1 |
| Radio per hop (CI wait + airtime + stacks) | 20 | 40 | 60 | 6 |
| Relay processing | 0 | 1 | 2 | 0 |
| Jitter buffer (adaptive, typical) | 60 | 70 | 80 | 40 |
| Decode + mix (10 ms tick avg) | 5 | 5 | 5 | 5 |
| Render ring target | 20 | 20 | 20 | 20 |
| OS render IO | 10 | 10 | 10 | 10 |
| eSCO down + headset (T5a measures) | 25 | 25 | 25 | 25 |
| **Total** | **≈ 231** | **≈ 262** | **≈ 293** (best effort) | **≈ 177** |

The acceptance target is MCPTT KPI3 (p95 < 300 ms), **promised for ≤ 2 BLE hops only**. The 3-hop column is a *typical* value already at the limit, so its p95 will exceed 300 ms; 3-4 hops are best effort (§5.5) and labelled so in the UI and store copy. Simulator thresholds (§13.2) are expressed excluding headset legs and add the 50 ms T5a leg budget to reach the 300 ms target. The jitter-buffer row is adaptive, not constant; the in-app estimator (§12.6) measures every stage except the two headset legs, which T5a measures per headset model. If T0/T5a come in worse, Appendix B lists the knobs (BLE-fast profile, `VOICE_MAX_AGE`, voice TTL 3).

## 8. Security and key schedule

### 8.1 Threat model and controls

| Threat | v1 control |
|---|---|
| Eavesdropping by nearby outsiders | All group traffic inside per-link Noise transport (ChaCha20-Poly1305). Mesh headers (origin, sess, seq, talk state, presence) are never on air in clear. |
| Injection / impersonation by outsiders | Noise XXpsk1: no epoch PSK, no link. IDPROOF binds the per-group identity to this handshake. |
| Replay | Per-direction AEAD counters + 1024-bit window, updated only after the tag verifies; fresh keys per link. |
| Nonce reuse after crash, restart or reinstall | Impossible by construction: every link derives fresh keys from fresh Noise ephemerals; no counter is persisted and nothing is persisted before use. REKEY seals use fresh ephemeral X25519 per message. |
| Tracking riders by radio | No static identifiers on air: rotating group tag and node token (10 min), OS MAC rotation, HELLO content rotates; no names or manufacturer data in advertisements; per-group identities are unlinkable across groups. |
| Counting a convoy | The fixed service UUID still reveals "an app user is here" (accepted; rotating UUIDs are v2). |
| Malicious member forging another member's origin | First-hop origin check (a direct neighbor cannot claim another origin at `hops = 0`). The cheap, rare control messages that could silence or misrepresent others are **signed by their origin's per-group Ed25519 key**: LEAVE, RIDE_CTL (plus leader-signed ROSTER, REKEY, GROUP_CTL). Members are still trusted for high-rate traffic: VOICE, TALK, PRESENCE, POSITION and FLOOR_REJECT stay unsigned in v1 (a forged FLOOR_REJECT can only silence a burst for its remaining duration; per-burst signatures are v1.x). |
| Malicious member handing out a fake invite | Members re-share the leader-signed invite verbatim (`invite_sig` from the ROSTER, §8.3); `parseInvite` reports `leaderSigOk`. After joining, the first verified ROSTER's key must equal the invite's `leader_ed_pk`, otherwise notice `inviteLeaderMismatch` and the group is marked "unverified leader" in the UI. |
| Invite link hijack | The `convlinkvoice://` custom scheme can be claimed by another installed app. The in-app scanner is the default path in the UI copy; the system-camera deep link is a convenience. Universal Links are a v1.x option. |
| Leaked or photographed QR | The QR carries only the **current** epoch secret. "Lock group" or "Remove member" rekeys, so old QRs stop working. QR auto-hides after 60 s; optional invite expiry. |
| Removed member | Signed roster deny entry + REKEY sealed only to remaining members (§8.6). |
| Malformed packets / DoS | Bounded parsers, fuzzed; per-neighbor penalties and bans; ingress caps (300 records/s, 64 KB/s); no compression. |
| Secret leakage through the WebView | Identity keys and epoch secrets live only in Keychain/Keystore. `getInvite` returns a natively rendered QR image (`qrPng`), so the invite text never enters JS state; the image itself still encodes the secret (residual risk accepted: the WebView loads only bundled content under a `default-src 'self'` CSP). An inbound deep link passes through JS (`appUrlOpen`) once and is handed straight to `parseInvite`; nothing secret is stored in web storage. |

### 8.2 Identities (per group)

At group create or join the engine generates a **per-group identity**: an Ed25519 signing key + an independent X25519 static key. `id_hash = SHA-256(ed_pk ‖ x_pk)[0..16]`; `node_id = u16::from_le_bytes(id_hash[0..2])`. Two groups never share an identity, so membership in two groups is not linkable. Storage: one Keychain item per group (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, not synchronized, excluded from backups) / Android AES-256-GCM blob wrapped by a non-exportable Keystore key (convLink vault pattern), excluded from backup and data-extraction rules.

**`node_id` collision** (P ≈ 0.7 % at 31 members): a node that sees two distinct `id_hash` values with the same `node_id` (JOIN_REQ, IDPROOF or ROSTER) rejects the second link with `REJECT{8}`. The member whose `id_hash` is lexicographically larger regenerates its per-group identity and rejoins with the same invite; this is deterministic and needs no leader. The leader never assigns a roster entry to a colliding `node_id`.

**Lab identity:** each Lab start draws a random `lab_node_id` (u16, CSPRNG) and a random 64-bit nonce, announced with `LAB_CTL{announce}`. If two Lab peers announce the same id, the one with the larger nonce redraws and re-announces. There are no user-chosen letters, so sender ids, sessions and dedup windows cannot collide.

### 8.3 Invite and key schedule

**Invite (QR / link):** `convlinkvoice://join?v=1#<base64url(invite)>`. It lives in the fragment, so it is never sent anywhere. About 170 characters → QR version ≈ 10, ECC M.

```
invite v1: ver u8=1 · flags u8 (b0 aware-pairing suggested, b1 has-expiry) · group_id[16] · epoch u32
           · epoch_secret[32] (CURRENT epoch) · leader_ed_pk[32] (leader's per-group key)
           · policy u8 (b7-5 voice_ttl−1, b4 ble_frame 0=40/1=20 ms, b3 leader priority, b2-0 reserved)
           · [expires_at u32 (unix minutes) if flags.b1] · name_len u8 · name[≤ 32 B] · invite_sig[64]
invite_sig = Ed25519(leader, "cvic/v1/invite" ‖ group_id ‖ epoch ‖ policy ‖ name_len ‖ name ‖ leader_ed_pk
                     ‖ SHA-256(epoch_secret))      # flags and expires_at are not signed (soft, leader-local settings)
```

The invite is about 255 base64url characters (QR version ≈ 13, ECC M). Because `invite_sig` commits to the epoch secret only through its hash, the leader can publish it in every ROSTER (§4.7), and any member assembles a byte-identical, leader-signed invite for the current epoch; members cannot change the group name, policy or leader key. `parseInvite` reports `expired` against the phone's clock with ±10 min tolerance (soft check: clocks are usually network-synced even offline) and `leaderSigOk`. Epoch 0's `epoch_secret` is 32 random bytes from the leader's CSPRNG.

**Schedule** (HKDF-SHA256):

```
PRK_e       = HKDF-Extract(salt = group_id, IKM = epoch_secret_e)
psk_e       = HKDF-Expand(PRK_e, "cvic/v1/noise-psk", 32)                 # Noise XXpsk1 PSK
disc_key_e  = HKDF-Expand(PRK_e, "cvic/v1/discovery", 32)
t10         = floor(unix_seconds / 600)                                    # accept t10−1, t10, t10+1 (clock skew), for every
                                                                           #   retained epoch (≤ 4, §8.6), newest first by epoch_lo
group_tag   = HMAC-SHA256(disc_key_e, "gt" ‖ u32 t10)[0..4]
node_token  = HMAC-SHA256(disc_key_e, "nt" ‖ u32 t10 ‖ id_hash)[0..4]     # receivers map token→member with ≤ 31 HMACs
aware_pmk   = HKDF-Expand(PRK_e, "cvic/v1/aware-pmk" ‖ ble_handshake_hash, 32)   # Android↔Android Aware (§9.6)
tunnel_rdv  = HKDF-Expand(PRK_e, "cvic/v1/tunnel-rendezvous", 16)          # v2 hook only
lab_tag     = SHA-256("cvic/lab" ‖ u16 lab_code)[0..2]                     # Lab links: not a secret, avoids strangers
```

### 8.4 Per-link Noise (decision and justification)

- **Pattern:** `Noise_XXpsk1_25519_ChaChaPoly_SHA256` via `snow` (spec rev 34; unaudited, mitigated by fuzzing the handshake path and keeping our code minimal). The prologue binds `group_id` and `epoch`. Initiator = connection initiator (BLE central, Aware subscriber). `snow` is built without default features and driven by our own `CryptoResolver` (X25519, ChaChaPoly, SHA-256 from the pinned RustCrypto/dalek crates; randomness from the engine's injected `ChaCha20Rng`). After the handshake, `dangerously_get_raw_split()` yields the two direction keys and SEC frames are sealed with the `chacha20poly1305` crate so the AAD of §4.3 can be bound (snow's `StatelessTransportState` has no associated-data parameter). Golden vectors pin the raw-split keys and the first SEC frame of each direction.
- **Why psk1 and not psk3:** with psk3 the PSK is mixed only after msg3, so any stranger who sends msg1 receives the responder's static X25519 key in msg2, which is a linkable identifier. With psk1 the PSK is mixed at the end of msg1, so a stranger's msg1 fails authentication and the responder rejects it before revealing anything. Neither static key is ever exposed to non-members.
- **IDPROOF** (first SEC record each way): `Ed25519(sk, "cvic/v1/idproof" ‖ role ‖ handshake_hash)` + both public keys. This binds the per-group Ed25519 identity to the Noise-authenticated X25519 key and to this exact handshake (channel binding). Other records are rejected until both proofs verify; then the deny-list, collision and duplicate-link checks run.
- **IDPROOF on datagram links (2026-10-24):** on Wi-Fi Aware (the only datagram link kind; Tunnel is a stream) IDPROOF may be lost or overtake msg3. While a link is in IdProof the node re-sends its IDPROOF every 250 ms (`IDPROOF_RESEND_US`, at most `IDPROOF_MAX_RESENDS` = 20; the 5 s handshake timeout still applies). An Up datagram link answers a repeated IDPROOF only if it verifies against this link's handshake hash and remote static and shows the identity already admitted, at most once per 125 ms (`IDPROOF_ANSWER_GAP_US`), at most 20 times, and only until the peer's CAPS arrives; anything else is ignored (no reply, no close, no second admission). SYNC/MESH records that arrive while the link is in IdProof are buffered (≤ 8 records / 4 KiB) and replayed after admission, never processed before it; the peer's CAPS after an answer re-runs the link-up announcements (JOIN_REQ if not in the roster, PRESENCE).
- **Why per-link Noise instead of end-to-end group AEAD:** (1) nonce safety without persisted state; (2) all mesh metadata hidden from outsiders, which gives header protection for free; (3) membership checked per link (needed for removal); (4) per-link replay protection; (5) the same mechanism secures Wi-Fi Aware and the v2 tunnel; (6) SFrame-style group keys would not give per-sender authenticity either. **Cost:** decrypt/re-encrypt per hop (ChaCha20 on ~120 B ≈ µs) and 21 B per frame, amortized by coalescing; the §4.8 budget includes it. A handshake costs about 1.5 RTT per link (≈ 50-100 ms on BLE), negligible next to BLE connection setup. If a future end-to-end format is added (v2 tunnel optimisation), it must use per-boot random session keys and QUIC-style header protection, never persisted counters.
- **Key lifetime:** in-band key update per direction after 12 h or 2³⁰ frames (§4.3), on the same link. There is no re-handshake and never a second link to the same peer.

### 8.5 Roster and admission

Possession of the current-epoch PSK is admission (Cardo-like remote grouping). The leader's signed ROSTER confirms names and roles and fixes roster indices; members verify it with `leader_ed_pk` from the invite (updated only by a verified GROUP_CTL transfer-leader). The leader adds every valid JOIN_REQ it receives (no approval step) and issues the next `roster_version`. Nodes accept traffic from unconfirmed members (the UI marks them) so a group works even when the leader is out of range; JOIN_REQ retransmission (§8.9) turns them into roster members as soon as a path to the leader exists. Leader-approval admission is v1.x.

### 8.6 Remove member, Lock group, rekey (Phase 2)

1. The leader floods a new signed ROSTER (version + 1). Remove adds a deny entry (role 0xFF) for the removed `id_hash`; Lock changes nothing in the entries.
2. Every node closes links whose peer `id_hash` is denied (`CLOSE{5}`) and refuses future IDPROOFs from it.
3. The leader floods REKEY with `epoch + 1`, sealing `epoch_secret_{e+1}` individually to each **roster** member (X25519 to `x_pk_i`; O(N), ≤ 31 entries, ≤ 16 segments). A removed member who still receives the flood cannot open it.
4. Nodes switch new handshakes to `psk_{e+1}`. They **retain the last 4 epoch secrets** and keep accepting handshakes under any retained older epoch, **only** from identities listed (non-denied) in the current roster that are **not yet known to hold the newest epoch** (no PRESENCE/CAPS with that epoch and no newest-epoch handshake seen from them). There is **no time limit**: groups persist and are reused weekly, so a roster member who skipped several rides still rejoins and is brought up to date at once by anti-entropy (§8.9: CAPS shows the old epoch → the peer pushes ROSTER + the latest REKEY, which is sealed to every current member). Lock's goal is still met: holders of an old QR who never joined are not in the roster, and removed members are denied by `id_hash`, so both are refused. The responder tries the newest psk first, then retained older ones; HELLO `epoch_lo` lets the initiator pick first. Existing authenticated links stay up.
5. **Before** `lockGroup`/`removeMember` runs, the UI calls `previewAccessLoss`; the leader sees the unconfirmed members (seen in PRESENCE but not in the roster, hence no sealed REKEY) who will lose access, and may wait for their JOIN_REQs ("Confirm reachable members first") or proceed.
6. New invites carry the new epoch and its `invite_sig` (published in the new ROSTER). The leader's "Close group" floods GROUP_CTL{close}, signed; members mark the group closed and delete its secrets.
7. **Transfer leadership** (`transferLeader{nodeId}`, target must be a present roster member): the leader floods GROUP_CTL{transfer-leader, new_leader_ed_pk} and then a ROSTER (version + 1) with the roles swapped, signed by the **old** key; members switch their trust anchor to the new key after both verify. From then on the new leader signs ROSTER/REKEY/GROUP_CTL. **Leader loss** (phone lost or group forgotten by the leader): no member can remove, lock or close. The documented recovery is that any member creates a new group and the riders rescan in the lobby (about one minute); "any member may Lock" is rejected because it would let any member evict others.

MLS is a later option; conflicting commits fit a partitionable mesh poorly.

### 8.7 Rotating on-air identifiers

Android restarts its advertising set at every `t10` boundary with new tokens and requests a new random address where the API allows. The iOS foreground local name changes with `t10`. The HELLO value changes with `t10` (tokens) and state (`psm`, `accepting`). No member identity, name or roster data is ever placed in advertisements or unauthenticated characteristics.

### 8.8 Lab links

Lab links use LAB frames (plaintext records, padded to SEC size) gated by the shared 4-digit Lab code. They never carry group traffic or secrets. GPS in Lab is opt-in per run and stays on the device until the user shares the file.

### 8.9 Control-state anti-entropy (normative)

Flooding alone loses signed state for nodes that were out of range, in another partition, or joined late. Every node therefore stores the latest accepted ROSTER, REKEY and GROUP_CTL bodies verbatim (they are self-authenticating), and repairs neighbors over links:

1. **Summary exchange:** CAPS carries `(epoch, roster_version, ctl_version)` (§4.4); PRESENCE carries their low 16 bits.
2. **Push on link-up:** after CAPS, if the peer's summary is behind mine in any component, I send the missing bodies as link-scoped `SYNC` records (§4.4) in the order REKEY → ROSTER → GROUP_CTL (a REKEY first, so the peer can then verify a ROSTER of the new epoch). SYNC records are P2 traffic, never flooded.
3. **Push on mismatch:** when a PRESENCE from a direct neighbor (`hops = 0`) shows `epoch16` or `roster_ver16` behind mine, I push the same way. Rate limit: one push per (link, kind) per 10 s; a push is cancelled if the neighbor's next PRESENCE shows it caught up.
4. **Pull is implicit:** a node that is behind does nothing; its own PRESENCE/CAPS makes its neighbors push.
5. **JOIN_REQ retransmission:** a member that is not in a verified ROSTER sends its JOIN_REQ as a `SYNC{4}` on every new link and re-floods it every 30 s until it appears in a verified ROSTER. Relays and the leader treat duplicates idempotently (by `id_hash`).
6. **Acceptance** uses exactly the monotonic rules of §4.7, so anti-entropy can never roll state back; a received SYNC that is accepted becomes my stored state and is offered onward by rules 2-3 (no flood needed).
7. **Effects of newly accepted state:** a ROSTER with new deny entries immediately closes links to denied peers (`CLOSE{5}`); a REKEY switches new handshakes to the new epoch; a GROUP_CTL{close} closes the group; a transfer-leader GROUP_CTL updates the trust anchor.

Scenario S13 (§13.2) checks convergence: a member partitioned during Remove + Lock rejoins later, converges to the newest epoch and roster, and closes its link to the removed member.

## 9. Transport layer

### 9.1 Link abstraction

The core sees links as **data + events**, not objects: `LinkInfo{id, kind: BleL2cap|BleGatt|WifiAware|Tunnel, role: Initiator|Responder, max_frame, unit: Stream|Message|Datagram, peer_platform_hint}`. Each native layer implements the same small interface and owns all OS objects:

```swift
// Swift (MeshPlatformKit)                                // Kotlin (meshengine)
public protocol MeshLink: AnyObject {                       interface MeshLink {
  var id: UInt32 { get }                                      val id: Int
  var kind: LinkKind { get }                                  val kind: LinkKind
  func send(_ bytes: Data) -> SendOutcome   // non-blocking    fun send(bytes: ByteArray): SendOutcome
  func close(reason: UInt8)                                   fun close(reason: Int)
}                                                           }
public protocol LinkDriver { func apply(_ cmd: RadioCommand) }   interface LinkDriver { fun apply(cmd: RadioCommand) }
public protocol LinkSink: AnyObject {                       interface LinkSink {          // implemented by EngineHost
  func linkUp(_ l: MeshLink, info: LinkInfo); func linkDown(_ id: UInt32, reason: LinkDownReason)
  func linkData(_ id: UInt32, _ bytes: Data); func linkWritable(_ id: UInt32)
  func discovered(_ d: Discovery); func helloRead(_ discId: UInt32, _ bytes: Data) }
```

Drivers: `BleCentralDriver`, `BlePeripheralDriver` (both produce L2CAP or GATT links), `AwareDriver` (Phase 4), `TunnelDriver` (v2 interface, no implementation). `LinkRegistry` assigns ids and fans data in. Writable signals: `NSStream.hasSpaceAvailable`, `peripheralManagerIsReady(toUpdateSubscribers:)`, `peripheralIsReady(toSendWriteWithoutResponse:)`, Android `onNotificationSent` / writer-thread completion. The platform layer **never** parses or frames bytes; the GATT chunk sizes come from the core's `Transmit` units.

### 9.2 BLE role rules (only pre-connection information is used before connecting)

| Pair | Scans | Advertises | Initiates | Data path | Duplicates |
|---|---|---|---|---|---|
| iPhone ↔ Android | iPhone (filter: service UUID) | Android (always, from the FGS) | **iPhone (central)** | L2CAP (Android `listenUsingInsecureL2capChannel`), GATT fallback | n/a (single direction) |
| iPhone ↔ iPhone | both | both (fg: UUID + name; bg: overflow area) | fg: the side with the lower **14-bit** node-token prefix carried in the local name (§9.4); on equal prefixes, or in bg (no name visible), either side may initiate | L2CAP (`publishL2CAPChannel(withEncryption: false)`) | after IDPROOF: lower `node_id` initiator wins |
| Android ↔ Android | both | both | the side with the lower `node_token` (both visible in scan responses) | L2CAP, GATT fallback | after IDPROOF: lower `node_id` initiator wins |
| Android → iPhone | — | — | **never**: Android connects only to adverts whose scan response carries our service data with `flags.android = 1` | — | — |

Rules: no mesh data in advertisements; every hop is a connected unicast link; no OS pairing or bonding (unencrypted attributes and insecure L2CAP only; an encrypted CCCD pops the iOS PIN dialog, convLink lesson). Equal 14-bit prefixes (probability 2⁻¹⁴ per pair) simply let both sides try; the resulting duplicate is resolved after IDPROOF like the background case. The Android overflow-bitmask detection of backgrounded iPhones is compiled in but **off** by default; it is the T3 variant "Android detects a backgrounded iPhone", a Phase 3 [DEV] bench (§12.2), and is switched on only if it passes there.

**Open gate:** iPhone↔iPhone T0 does not measure the iPhone-central → Android-peripheral link, whose connection interval neither app can set and whose throughput is role-asymmetric. This stays a **[DEV] gate for Phase 3**. Android is not labelled "supported" until that T0 variant passes.

### 9.3 GATT service (UUIDs fixed, exported by `ble_constants()`)

| Item | UUID | Properties | Content |
|---|---|---|---|
| Service `MESH_SVC` | `31B8CD85-9D3A-4622-A465-3CDCA2C32DC5` | primary | — |
| `HELLO` | `86841842-A7C3-4844-84D2-5B281A3F616C` | read (unencrypted) | 16 B: `magic u8 = 0xC1 · proto_min u8 · proto_max u8 · flags u8 (b0 android, b1 lab, b2 accepting, b3 aware-capable, b4 leader) · psm u16 (0 ⇒ GATT only) · group_tag[4] (Lab: lab_tag ‖ 0x0000) · node_token[4] · epoch_lo u16` |
| `RX` | `14E91943-7673-440A-979A-F85B4DBD4DBF` | write without response | GATT data path central → peripheral |
| `TX` | `A76E3168-B4F6-4BE1-8252-35AB8AF17302` | notify | GATT data path peripheral → central |

**Connection sequence (initiator)**, with every decision in the core so no secret leaves Rust: `RadioCommand::Connect{disc_id}` → native connects → discovers `MESH_SVC` → reads `HELLO` → `on_hello_read` → the core checks `group_tag` (t10 ± 1) against every **retained** epoch's `disc_key` (newest first, ordered by HELLO `epoch_lo`), version range and `accepting` → `OpenLink{disc_id, via: L2cap{psm} | Gatt}` or `Abort{disc_id, ignore_s: 60}` → native opens L2CAP or subscribes `TX` → `on_link_up` → Noise HS → IDPROOF → CAPS → anti-entropy SYNC if either side is behind (§8.9) → NBR → link up. **Epoch catch-up:** a node that is behind cannot verify tags of a newer epoch. If HELLO `epoch_lo` is 1-4 epochs ahead of mine and the tag does not verify, the core still opens a **speculative link** (lowest connect priority, at most one per 30 s, per-peer backoff 60 s on failure) and handshakes with its newest psk; the ahead peer accepts it under the retained-epoch rule of §8.6 and pushes the REKEY. `node_token` mapping uses the same retained-epoch list. This covers an iPhone (central) that is behind an Android peripheral. Responder side: the core keeps the peripheral's HELLO current with `SetHello{bytes}` on every `t10` rollover and state change. iOS `connect` never times out by itself, so native cancels after 8 s unless the core armed a pending reconnect (§9.5).

### 9.4 Advertising payloads

| Platform / state | Primary ADV | Scan response | Visible to |
|---|---|---|---|
| Android (FGS) | **legacy, connectable, scannable `ADV_IND`** (`AdvertisingSetParameters.Builder().setLegacyMode(true).setConnectable(true).setScannable(true)`): Flags (3 B) + Complete List of 128-bit UUIDs `MESH_SVC` (18 B) = 21 B ≤ 31 | Service Data 128-bit (AD 0x21): `MESH_SVC` + `flags u8 (b0 android, b1 lab) · group_tag[4] · node_token[4]` = 27 B ≤ 31 | iOS fg/bg (bg ignores the scan response; the iPhone still connects by UUID and reads HELLO), Android |
| iPhone foreground | `CBAdvertisementDataServiceUUIDsKey: [MESH_SVC]` | `CBAdvertisementDataLocalNameKey` = **6 chars**, base32 of `group_tag[0..2] ‖ node_token` top 14 bits (8 B AD incl. header, inside the 10 B name allowance) | iOS, Android |
| iPhone background | UUID in the overflow area only | — | iPhones scanning explicitly for `MESH_SVC` (possibly only with the receiver's screen lit; T3 measures) |

- Android advertising: `ADVERTISE_MODE_BALANCED` (≈ 250 ms) while `Forming`, `LOW_POWER` (≈ 1 s) when `Stable`; `TX_POWER_HIGH`.
- Android scanning: `ScanFilter(serviceUuid = MESH_SVC)` always (`neverForLocation`); `SCAN_MODE_LOW_LATENCY` while Forming, `BALANCED` when Stable; restart filtered scans every 9 min; never more than 5 starts per 30 s.
- iOS scanning: `scanForPeripherals(withServices: [MESH_SVC], options: [AllowDuplicates: Forming && foreground])`, refreshed every 4 s. A Live Activity is started before backgrounding to keep foreground-class Bluetooth privileges (iOS 26; T3 measures whether this helps discovery).

### 9.5 Connection management and rejoin

| Constant | Value |
|---|---|
| BLE links target / max per node | 2 / 3 [TUNE: T0c may raise to 3 / 4] |
| Neighbors max (all kinds) | 6 |
| Node BLE airtime budget | 180 kbps, voice-aware (idle links 8 kbps) [TUNE: T0c] |
| Connection-limit backoff | `LimitReached` ⇒ that device's BLE max −1 for 10 min (floor 1) |
| Concurrent connect attempts | 1 per transport |
| Connect / handshake timeout | 8 s / 5 s |
| Per-peer backoff | 5 s → 60 s (×2), reset on new token; an **established** link that dies gets a 3 s settle hold without backoff growth |
| iOS filtered-scan refresh | every 4 s (rediscovers a peer coming back into range) |
| Android `requestConnectionPriority` | HIGH while any burst is active or in Lab; BALANCED otherwise (GATT client only) |
| Android PHY / MTU | `setPreferredPhy(1M)` on client and server links (1M has ~3-5 dB better sensitivity than 2M: more range; field test 2026-10-24); MTU 517 requested |
| iOS L2CAP stream thread | dedicated `Thread` with its own `RunLoop` (NSStream needs one) |
| Link death | 4 s without inbound frames |

**Rejoin strategy:** links formed in the lobby with screens on are kept alive (PING, 4 s death) for the whole ride. After a loss, discovery returns to Forming. Two T3 variants decide the rest: (a) **pending reconnect**: iOS keeps an outstanding `CBCentralManager.connect(peripheral)` (and tests `CBConnectPeripheralOptionEnableAutoReconnect`) to each lost iPhone peer for up to 15 min, bounded by the peer's resolvable-address rotation; (b) Live Activity-backed scanning. Whichever T3 shows working becomes `RadioCommand::ArmReconnect` policy. If both fail for "both phones locked", the rejoin path for iPhones is Wi-Fi Aware (Phase 4) and the product copy says so. **Aware grants no runtime by itself** (verified): an Aware rejoin works only while the L0-L3 anchors (audio, or location in L3) keep the process alive, exactly like BLE relaying.

### 9.6 Wi-Fi Aware (Phase 4, optional upgrade)

**iOS (iPhone↔iPhone):**
- Entitlement `com.apple.developer.wifi-aware = ["Publish", "Subscribe"]`. **Request it in Phase 0** (approval uncertain). Info.plist `WiFiAwareServices` declares `_cvic._udp` (≤ 15 chars; an invalid name crashes the app, so a static contract test checks it).
- **Single role per phone via 2-colouring:** the lobby proposes N−1 PIN pairings via DeviceDiscoveryUI (persisted in iOS Settings) in one of two bipartite shapes, chosen by the leader: **riding order** (a chain in the order the riders say they will ride, so Aware neighbors match road neighbors; even positions publish, odd subscribe) or **star** (the leader publishes, every member subscribes; best for small groups that ride bunched). Lobby join order is *not* used, because it rarely matches the riding order. Both shapes avoid the reported both-roles-on-one-service failure (FB21527009). The role is stored per group. Realtime mode costs roughly 4× BLE power (research), so "Faster link" is opt-in and T4 includes a battery variant.
- Connections: `WAPublisherListener` / `WASubscriberBrowser` + `NetworkConnection` (UDP), `WAPerformanceMode.realtime`, access category `.interactiveVoice`. Keepalive PING 500 ms; rebuild on `publisherTimeout`/`subscriberTimeout`. The Local Network prompt (if NetworkConnection over Aware triggers it) is requested in the foreground during pairing.

**Android (Android↔Android):** `WifiAwareManager.attach`, publish/subscribe service `cvic`; data path with `WifiAwareNetworkSpecifier.Builder(session, peerHandle).setPmk(aware_pmk)` (salted with the BLE link's Noise handshake hash) + `ConnectivityManager.requestNetwork`; a UDP socket bound to that `Network`. Handle `ACTION_WIFI_AWARE_STATE_CHANGED` by discarding sessions; check `getAvailableAwareResources()`; never run Aware together with hotspot or Wi-Fi Direct. The same 2-colouring is used.

**iPhone↔Android Aware:** not in v1. T4 builds an empirical per-model allow-list (Android 17 pairing APIs); `isAwarePairingSupported()` alone is not trusted. The allow-list is empty in v1.

**Upgrade protocol (over the existing BLE link):** `UPGRADE{offer, transport, nonce}` → `UPGRADE{accept}` → the publisher-side peer connects Aware → fresh Noise HS + IDPROOF on the Aware link (IDPROOF retransmitted on this datagram link, §8.4) → both sides add it to the neighbor's link bundle → `UPGRADE{ready}`.

### 9.7 Upgrade and downgrade without dropping audio

- The scheduler chooses the link **per packet** from the neighbor's bundle: Aware if `q ≥ 60` and `srtt < 50 ms`, else BLE.
- **Overlap:** for 1 s after a switch in either direction, voice is sent on both links; end-to-end dedup removes the copy, so the receiver sees no gap.
- **Downgrade triggers:** Aware has no RX for 600 ms, or `q < 40`. The BLE link is always kept (PING 2 s) as the fallback and control path.
- **Cap change on downgrade:** admission is re-evaluated with the BLE cap; bursts beyond it (lowest rank first) get `FLOOR_REJECT{congestion}`. On an **edge link** of a hybrid group the downgrade only sheds locally (by rank, no FLOOR_REJECT; §6.3).

### 9.8 v2 cellular hook (designed, not built)

- `LinkKind::Tunnel` exists in v1 enums, `CAPS.transports.b4`, `UPGRADE.transport = 3`, and the scheduler has a link cost: a Tunnel is used only for neighbors with no radio link for > 5 s, i.e. to bridge islands.
- A tunnel is a byte pipe between **two members** (relay server or WebRTC data channel). Noise runs end-to-end over it, so the server never sees plaintext. Rendezvous id = `tunnel_rdv` (§8.3).
- Native interface `TunnelProvider` (Swift protocol / Kotlin interface: `open(rendezvous: Data, peerHint: Data) -> MeshLink?`) ships with no implementation. Supabase Realtime/WebRTC is a v2 implementation of that interface.

## 10. Platform layers

### 10.1 iOS (Swift 6, iOS 26.0+, iPhone 12+)

#### 10.1.1 Packages and source layout

```
native/ios/MeshPlatformKit/            (no Rust; builds in week 1)
  Sources/CMeshAudio/                  C: VPIO AudioUnit setup + input/render callbacks, atomics,
                                       cmesh_set_sink(push_fn, pull_fn, flags_fn, stats_fn, ctx), built-in sinks
                                       (meter, loopback, tone, T5a click train/correlator capture)
  Sources/MeshActivityShared/          IntercomAttributes (ActivityAttributes + ContentState) + the LiveActivityIntent
                                       types (ToggleMute, LatchTalk, ResumeMic) for app + widget (WP-14, contract C9)
  Sources/MeshPlatform/
    BLE/   BleCentralDriver, BlePeripheralDriver, L2capLink, GattLink, LinkIOThread, LinkRegistry, AdvBuilder
    Audio/ AudioSessionController (category/mode/options, interruptions incl. iOS 27, route changes, media reset),
           VoiceIOUnit (Swift wrapper over CMeshAudio), RouteMonitor, MuteWatchdog, OutputOnlyFallback, PromptRenderer
    Host/  KeepAliveLocation (CLBackgroundActivitySession + CLLocationUpdate.liveUpdates), LifecycleMonitor,
           DeviceSampler (battery, thermal, protected data), ResumeNotifier, IntercomActivityController (ActivityKit,
           8 h renewal), RadioStateMonitor (CBManagerState → RadioState), IdleTimerController
    Keys/  SecureStore (Keychain, AfterFirstUnlockThisDeviceOnly)
    Lab/   LabFileStore (Documents/lab/*.jsonl, share export)
    QR/    InviteScanner (VisionKit DataScannerViewController, .barcode([.qr]))
    WiFiAware/                         (Phase 4)
  Tests/MeshPlatformTests/             adv/HELLO vector replay, session-config logic with a mocked session wrapper,
                                       LinkRegistry, SecureStore, LabFileStore, watchdog FSM
native/ios/MeshEngineKit/              (needs the xcframework)
  Frameworks/MeshCoreFFI.xcframework   (gitignored, `cargo xtask build-ios`)
  Sources/MeshCoreBindings/mesh_ffi.swift   (generated, committed)
  Sources/MeshEngineKit/  EngineHost (owns MeshEngine, implements PlatformHooks + LinkSink),
                          RadioAdapter, AudioAdapter (registers mesh_rt_* into cmesh_set_sink), NotifyAdapter
  Tests/MeshEngineKitTests/            bindings smoke, EngineHost with fake drivers, Demo ride in the simulator
```

#### 10.1.2 Audio session and VPIO (exact configuration)

```swift
let s = AVAudioSession.sharedInstance()
try s.setCategory(.playAndRecord, mode: .voiceChat,
                  options: profile == .car ? [.allowBluetoothHFP, .defaultToSpeaker] : [.allowBluetoothHFP]) // never .mixWithOthers by default
try s.setPreferredSampleRate(16_000)           // hint; the VPIO client format guarantees 16 kHz to the core
try s.setPreferredIOBufferDuration(0.010)
try s.setPrefersNoInterruptionsFromSystemAlerts(true)
try s.setActive(true)
if let hfp = s.availableInputs?.first(where: { $0.portType == .bluetoothHFP }) { try s.setPreferredInput(hfp) }
// CMeshAudio: kAudioUnitType_Output / kAudioUnitSubType_VoiceProcessingIO; enable input (bus 1) and output (bus 0);
// client format on bus1-output and bus0-input scopes: 16 kHz, mono, Int16, packed; re-read formats after enabling VP;
// kAUVoiceIOProperty_BypassVoiceProcessing = 0 (1 only in the T5a bench); VoiceProcessingEnableAGC = 1;
// kAUVoiceIOProperty_OtherAudioDuckingConfiguration = { mEnableAdvancedDucking = true, mDuckingLevel = Min };
// input callback: AudioUnitRender into a preallocated AudioBufferList → push_fn(ptr, frames, hostTimeNs)
// render callback: pull_fn(ioData, frames, hostTimeNs)
```

`bluetoothHighQualityRecording` is never used (not for real-time). The session is non-mixable because mixable recording sessions do not (re)activate in the background (DTS). A Lab-only toggle makes it mixable so T2 can compare against navigation apps.

#### 10.1.3 Keep-alive anchors and lifecycle

| Anchor | Started | Keeps alive | Notes |
|---|---|---|---|
| `audio` + running VPIO | Go live (foreground tap) | process + capture/playout | may lose capture on some route changes (R2) |
| `location` + `CLBackgroundActivitySession` + `CLLocationUpdate.liveUpdates(.otherNavigation)` | Go live, **only if "Share my distance" is on** | process; relaying continues even if audio dies (L3) | blue indicator; GPS power. Justified by the visible distance list (App Review 2.5.4). |
| `bluetooth-central` / `bluetooth-peripheral` | always | short wakes when suspended | not a keep-alive by itself |
| Live Activity | Go live, **before backgrounding** | foreground-class BT privileges in the background (iOS 26) | effect on discovery measured in T3. The system ends a Live Activity after **8 h**; `IntercomActivityController` ends and restarts it whenever the app is foregrounded after 7.5 h, and the 8 h limit is noted in the T2 report and in long-ride copy ("open the app once every few hours"). |
| Idle-timer control | Lobby screen visible; Ride screen in PTT mode; car "keep screen on while mounted" | screen stays on, so the process is foreground | `UIApplication.isIdleTimerDisabled` toggled by the plugin's `setKeepScreenOn`, always reset when the screen is left or the app backgrounds |

When distance sharing is off, the UI states it once: "Without location, your phone may stop relaying during a phone call." CoreBluetooth restore identifiers (`cvic.central`, `cvic.peripheral`) are set but not relied upon. After force-quit there is no relaunch (TN3115); onboarding says so and the next launch offers Resume ride.

#### 10.1.4 Interruption and mic-loss state machine (R2 mitigation)

```
RUNNING ──interruption began──► INTERRUPTED (PRESENCE on_call; TX off)                       [ladder L1]
INTERRUPTED ──ended + shouldResume / iOS 27 resumptionRecommendation(.resume)──► restart ──ok──► RUNNING (L0, prompt "mic back")
                                                                                  └─fail ('!rec')──► CAPTURE_LOST
INTERRUPTED ──no "ended" 5 s after the call──► retry every 5 s for 2 min, then every 30 s
RUNNING ──route change, format unchanged──► nothing (the AU keeps running)
RUNNING ──route change, format changed / IsRunning false──► uninit, reconfigure, init, start ──fail──► CAPTURE_LOST
RUNNING ──MuteWatchdog: capture_callbacks stalled > 1.5 s (mesh_rt_stats)──► restart ──fail──► CAPTURE_LOST
RUNNING ──mediaServicesWereReset──► rebuild session + VPIO ──fail──► CAPTURE_LOST
CAPTURE_LOST (L2): (1) AudioIo::CaptureLost → PRESENCE capture_ok = 0 (peers see "mic paused");
                   (2) OutputOnlyFallback: RemoteIO output-only unit so the rider still hears the group [verified in T2];
                   (3) Live Activity "Mic paused — Resume" + time-sensitive local notification "Tap to resume intercom";
                   (4) helmet earcon + prompt;
                   (5) app foreground or Resume button (LiveActivityIntent opens the app) → restart → RUNNING.
render also lost → L3 (relay-only) while the location anchor keeps the process alive.
```

iOS 27 uses `didBecomeInactive`/`resumptionRecommendation`; earlier versions use `interruptionNotification` (`if #available(iOS 27, *)`). The engine never stops VPIO during a ride: talk modes gate transmission after capture.

#### 10.1.5 Info.plist, entitlements, privacy manifest (app target `apps/mobile/ios/App/App/`)

| Key | Value |
|---|---|
| `CFBundleIdentifier` | `com.convlink.voice` (working name "convLink Voice", team V6X9QP25Z6; renameable, Appendix E) |
| `NSMicrophoneUsageDescription` | "Your voice is sent to the riders in your group." (ES/EN via InfoPlist.strings) |
| `NSBluetoothAlwaysUsageDescription` | "Connects directly to nearby riders' phones to carry the intercom, without internet." |
| `NSCameraUsageDescription` | "Scan your group's invite QR code." |
| `NSLocationWhenInUseUsageDescription` | "Shows how far each rider is and keeps the intercom running while the screen is locked." |
| `UIBackgroundModes` | `audio`, `bluetooth-central`, `bluetooth-peripheral`, `location` (backed by the distance feature). **No** `voip`, **no** `push-to-talk`. |
| `NSSupportsLiveActivities` | `true` |
| `CFBundleURLTypes` | scheme `convlinkvoice` |
| `UIApplicationSceneManifest` | single scene, `SceneDelegate`, `UISceneStoryboardFile = Main` (iOS 27 requirement; copy convLink's fix and its contract test) |
| `UIRequiredDeviceCapabilities` | `arm64`, `bluetooth-le`, `microphone` |
| `ITSAppUsesNonExemptEncryption` | `true` (Noise/ChaCha20; standard algorithms; export documentation in Phase 5) |
| `WiFiAwareServices` (Phase 4) | `{"_cvic._udp": {Publishable, Subscribable}}` |
| `NSLocalNetworkUsageDescription` (Phase 4, only if NetworkConnection over Aware triggers it) | "Faster Wi-Fi link to riders you paired with." |
| Entitlements | `com.apple.developer.usernotifications.time-sensitive`; Phase 4: `com.apple.developer.wifi-aware = [Publish, Subscribe]` (requested in Phase 0) |
| `PrivacyInfo.xcprivacy` | required-reason APIs: UserDefaults `CA92.1`, system boot time `35F9.1`; no data collected by the developer; privacy label and review notes disclose peer-to-peer location sharing with group members when enabled |

Widget extension target `IntercomWidget` renders `IntercomAttributes.ContentState { live, muted, talk, pttLatched, speakers[≤2], inRange, total, ladder, onCall, alone, bluetoothOff, labStatus? }` on the lock screen and Dynamic Island (updates ≤ 1/s and on change). Buttons: `ToggleMuteIntent: LiveActivityIntent` and **`LatchTalkIntent: LiveActivityIntent`** (both run in the app process, which already captures, and only write `mesh_rt_set_flags` bits 1/3, so they are legal from the lock screen), and `ResumeMicIntent` (opens the app, because capture cannot start in the background). `IntercomAttributes` lives in `MeshActivityShared` (owned by WP-14 as part of contract C9); the widget UI is WP-23. Phase 0 ships a minimal version (Lab status only) for the T3 Live Activity variant.

**App Review notes:** explicit Go live with a visible mic state, a Demo ride that keeps talking when locked (2.5.4), a review video, and the distance feature that justifies `location`.

### 10.2 Android (`native/android/meshengine`, Kotlin 2.3, minSdk 31, target/compile 36)

No Android device exists yet. Everything here must **build, lint and pass JVM unit tests [ENV]** and launch in the emulator. Field behaviour is a Phase 3 [DEV] gate.

#### 10.2.1 Source layout

```
src/main/kotlin/com/convlink/voice/engine/
  MeshEngineService.kt      FGS host: startForeground(types per §10.2.3), wake lock, ongoing notification
  EngineHost.kt             owns MeshEngine (UniFFI/JNA), implements PlatformHooks + LinkSink, binder for the bridge
  LinkRegistry.kt
  ble/  BleScanner.kt BleAdvertiser.kt GattServer.kt GattClientLink.kt L2capServer.kt L2capClient.kt
        L2capLink.kt (reader + writer thread per socket) AdvBuilder.kt
  audio/ AudioRouteController.kt   MODE_IN_COMMUNICATION, setCommunicationDevice, listeners, focus
         AudioSessionStrategy.kt   interface; DefaultCommunicationStrategy (on) | TelecomStrategy (flag, off)
         OboeBridge.kt             JNI to meshaudio.cpp (start/stop/state callbacks, dlsym of mesh_rt_*)
         MuteWatchdog.kt PromptRenderer.kt
  sensors/ LocationSampler.kt DeviceSampler.kt
  keys/  SecureStore.kt            Keystore-wrapped AES-GCM (convLink vault pattern), backup-excluded
  notify/ IntercomNotifications.kt ongoing notification (Mute/Talk latch/Leave/Resume), "Tap to rejoin"
  radio/  RadioStateMonitor.kt     BluetoothAdapter.ACTION_STATE_CHANGED + airplane mode → on_platform RadioState
  lab/   LabFileStore.kt
  qr/    InviteScanner.kt          Google code scanner (play-services-code-scanner): no CAMERA permission
  aware/                           Phase 4
src/main/cpp/meshaudio.cpp + CMakeLists.txt   Oboe streams → mesh_rt_capture / mesh_rt_render (via dlsym; tone fallback)
src/main/kotlin/com/convlink/voice/core/mesh_ffi.kt   generated, committed
src/test/kotlin/…                JVM tests; bindings run against the host libmesh_ffi.dylib via jna.library.path
```

#### 10.2.2 Audio (default strategy)

`AudioManager.mode = MODE_IN_COMMUNICATION`; `setCommunicationDevice` preferring `TYPE_BLE_HEADSET` > `TYPE_BLUETOOTH_SCO` > `TYPE_WIRED_HEADSET`/`TYPE_USB_HEADSET` > `TYPE_BUILTIN_SPEAKER` (car profile); `OnCommunicationDeviceChangedListener` (proceed on the default route while waiting up to 30 s); audio focus `AUDIOFOCUS_GAIN` with `USAGE_VOICE_COMMUNICATION`. Oboe input and output: `Usage::VoiceCommunication`, `InputPreset::VoiceCommunication`, `PerformanceMode::LowLatency`, `SharingMode::Shared`, 16 kHz mono I16 with Oboe resampling; `onErrorAfterClose` ⇒ reopen streams (allowed because the FGS was started while visible and has while-in-use). On exit `clearCommunicationDevice()`. A real call takes SCO (the dialer has priority) ⇒ PRESENCE `on_call`, then re-`setCommunicationDevice` after the call. MuteWatchdog uses `mesh_rt_stats` exactly as on iOS.

**`TelecomStrategy` (off by default):** Jetpack Core-Telecom self-managed call, `phoneCall` FGS type, `MANAGE_OWN_CALLS`, CallStyle notification. It is compiled only into builds with Gradle property `meshengine.telecom=true`, which also adds its permissions and FGS type through a separate manifest overlay (`src/telecom/AndroidManifest.xml`). It is evaluated once an Android device exists, including head-unit/Android Auto behaviour. No static `MEDIA_BUTTON` receiver ships by default, because it would take helmet media keys from music apps. The helmet-button restart path is a flag-gated experiment.

#### 10.2.3 Service rules (Android 14-17)

- The bridge calls `startForegroundService` **inside the visible tap** that opens the lobby (types `CONNECTED_DEVICE`), and again inside the Go-live tap with `MICROPHONE | CONNECTED_DEVICE`, plus `LOCATION` **only when** fine location is granted and "Share my distance" is on. `startForeground` follows within 5 s. A location-typed `startForeground` without a granted location permission throws on Android 14+, so the type set is computed from the current grants every time.
- `START_NOT_STICKY`: a mic FGS restarted from the background would throw. After process death the next launch offers Resume ride, and `ApplicationExitInfo` explains why. The notification's Resume action is a while-in-use exemption path.
- Screen: the bridge's `setKeepScreenOn` toggles `FLAG_KEEP_SCREEN_ON` on the activity window (lobby, PTT ride screen, car "keep screen on while mounted").
- `PARTIAL_WAKE_LOCK` for the session. `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` is requested after the first successful ride (justified: offline intercom, FCM unusable), with OEM pages (Samsung, Xiaomi, Huawei, OnePlus).
- Android 17 background-audio hardening applies to all apps on Android 17: audio runs only while the non-short-service FGS is up. Validated with `adb shell cmd audio set-enable-hardening throw` in Phase 3 [DEV]. Android 17 memory limits: jitter buffers, codec state and caches stay bounded (decoder pool, prompt bank ≤ 512 KB).
- Play declarations: each FGS type needs a declaration and video. If Play rejects `connectedDevice` for phone-to-phone use, the fallback is `microphone` (+`location`) only; this is decided at Phase 5 submission.

#### 10.2.4 Manifest (library manifest merged into the app; target 36)

```xml
<uses-feature android:name="android.hardware.bluetooth_le" android:required="true"/>
<uses-feature android:name="android.hardware.microphone" android:required="true"/>
<uses-feature android:name="android.hardware.wifi.aware" android:required="false"/>
<uses-permission android:name="android.permission.RECORD_AUDIO"/>
<uses-permission android:name="android.permission.MODIFY_AUDIO_SETTINGS"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_MICROPHONE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE"/>
<uses-permission android:name="android.permission.FOREGROUND_SERVICE_LOCATION"/>
<uses-permission android:name="android.permission.POST_NOTIFICATIONS"/>
<uses-permission android:name="android.permission.BLUETOOTH_SCAN" android:usesPermissionFlags="neverForLocation"/>
<uses-permission android:name="android.permission.BLUETOOTH_ADVERTISE"/>
<uses-permission android:name="android.permission.BLUETOOTH_CONNECT"/>
<uses-permission android:name="android.permission.NEARBY_WIFI_DEVICES" android:usesPermissionFlags="neverForLocation"/>
<uses-permission android:name="android.permission.ACCESS_WIFI_STATE"/>
<uses-permission android:name="android.permission.CHANGE_WIFI_STATE"/>
<uses-permission android:name="android.permission.ACCESS_NETWORK_STATE"/>
<uses-permission android:name="android.permission.CHANGE_NETWORK_STATE"/>
<uses-permission android:name="android.permission.ACCESS_FINE_LOCATION"/>   <!-- distance sharing, Lab GPS, Aware on API 31-32 -->
<uses-permission android:name="android.permission.ACCESS_COARSE_LOCATION"/>
<uses-permission android:name="android.permission.WAKE_LOCK"/>
<uses-permission android:name="android.permission.REQUEST_IGNORE_BATTERY_OPTIMIZATIONS"/>
<!-- NOT declared at target 36: ACCESS_LOCAL_NETWORK (added only when targeting 37, for Aware sockets).
     NOT declared by default: MANAGE_OWN_CALLS, FOREGROUND_SERVICE_PHONE_CALL (telecom overlay only). -->
<application>
  <service android:name="com.convlink.voice.engine.MeshEngineService" android:exported="false"
           android:foregroundServiceType="microphone|connectedDevice|location"/>
</application>
```

App manifest (WP-18): `MainActivity` `launchMode="singleTask"` with an intent filter for `convlinkvoice://join`; backup and data-extraction rules exclude the secure store. Runtime permissions requested in onboarding: `BLUETOOTH_SCAN/ADVERTISE/CONNECT` (+ `NEARBY_WIFI_DEVICES` in Phase 4, same "Nearby devices" group), `RECORD_AUDIO`, `POST_NOTIFICATIONS`, and `ACCESS_FINE_LOCATION` only when the rider enables distance sharing or a Lab GPS run.

## 11. Capacitor plugin and React UI

### 11.1 Plugin `@meshapp/capacitor-intercom` (jsName `MeshIntercom`, contract C6)

The package is **TS-only**: `definitions.ts`, `index.ts` (`registerPlugin('MeshIntercom', { web: () => import('./web') })`) and `web.ts`. Native code lives in the app-local bridges (§2.4), each about 300 LOC.

```ts
export interface MeshIntercomPlugin {
  getInfo(): Promise<{ platform: "ios" | "android" | "web"; engine: "platform-only" | "full";
                       apiVersion: string; protocolVersion: number; appVersion: string }>;
  getStatus(): Promise<{ engine: "idle" | "running"; live: boolean; uiAttached: boolean }>;
  checkPermissions(): Promise<IntercomPermissionStatus>;
  requestPermissions(opts?: { permissions?: IntercomPermission[] }): Promise<IntercomPermissionStatus>;
  /** Gesture required. Radios + engine thread; Android FGS (connectedDevice). Audio stays off. */
  startEngine(opts: { groupId?: string; lab?: { code: string }; demo?: boolean }): Promise<void>;
  /** Gesture required; rejects with "notForeground" unless the app is foreground-active. For the ★ commands of §3.7
   *  (goLive, resumeMic, labStart with audio "full"/"renderOnly", audioCheck start). The bridge synchronously does the
   *  OS-level work (iOS: activate session, start VPIO with the RT sink muted, location anchor if requested, Live Activity;
   *  Android: startForegroundService with microphone [+location], open Oboe streams), then forwards the command; the
   *  core's StartCapture only connects and unmutes the sink (§3.2). */
  gestureCommand(opts: { command: string }): Promise<{ result: string }>;
  /** Sugar for gestureCommand({type:"goLive"}) / ({type:"resumeMic"}). */
  goLive(): Promise<void>;
  resumeMic(): Promise<void>;
  stopEngine(): Promise<void>;
  /** JSON pass-through of mesh-api UiCommand → CommandResult; rejects ★ commands with "useGestureCommand". */
  send(opts: { command: string }): Promise<{ result: string }>;
  /** Fast path: writes mesh_rt_set_flags directly and also sends {type:"ptt"}. Stamps the native receive time for T8. */
  ptt(opts: { down: boolean; tUiMs?: number }): Promise<void>;
  /** iOS isIdleTimerDisabled / Android FLAG_KEEP_SCREEN_ON; reset automatically on background. */
  setKeepScreenOn(opts: { on: boolean }): Promise<void>;
  scanInviteQr(): Promise<{ url?: string; cancelled: boolean }>;
  shareFile(opts: { path: string; mime: string }): Promise<void>;     // Lab export via share sheet
  openAppSettings(): Promise<void>;
  requestBatteryExemption(): Promise<{ granted: boolean }>;           // Android only; no-op elsewhere
  addListener(event: "event", cb: (e: { json: string }) => void): Promise<PluginListenerHandle>;  // UiEvent stream
}
type IntercomPermission = "microphone" | "bluetooth" | "notifications" | "camera" | "location" | "nearbyWifi";
```

- Deep links (`convlinkvoice://join…`) arrive through `@capacitor/app` `appUrlOpen` (convLink's SceneDelegate path), not through this plugin; the handler passes the URL straight to `parseInvite` and never stores it. The in-app scanner is the recommended path (§8.1, custom-scheme hijack).
- Bridge contract tests (WP-19): `send` rejects every ★ command; `gestureCommand` rejects when the app is not foreground-active; `goLive` performs session/FGS work before the command reaches the engine; `setKeepScreenOn(false)` runs on every background transition.
- The bridges call `set_ui_attached(true/false)` on resume/pause and request a full snapshot on foreground return (WebView timers are not trusted while backgrounded; convLink lesson). They forward `ui_event` JSON via `notifyListeners("event", ["json": …])`. They refuse to start if `api_version()` major differs from the plugin's.
- `@meshapp/engine-api` wraps the plugin: `const client = createEngineClient(MeshIntercom)` gives typed `client.command<C extends UiCommand>()` and `client.onEvent(handler)`. Hand-written guards over the generated discriminants live there, not in components.

### 11.2 Web implementation (browser simulator)

`web.ts` lazily loads `@meshapp/engine-web`, which instantiates `mesh_wasm.wasm` with a **raw `extern "C"` JSON ABI** (no wasm-bindgen):

```c
uint32_t mw_alloc(uint32_t len);  void mw_free(uint32_t ptr, uint32_t len);
uint32_t mw_world_new(uint32_t cfg_ptr, uint32_t cfg_len);          // JSON scenario {preset, nodes, loss, seed}
uint32_t mw_command(uint32_t world, uint32_t ptr, uint32_t len);    // returns ptr to [u32 len][JSON CommandResult]
void     mw_step(uint32_t world, uint64_t dt_us);                   // advance simulated time
uint32_t mw_poll_events(uint32_t world);                            // ptr to [u32 len][JSON UiEvent[]]
uint32_t mw_poll_lab(uint32_t world);                               // ptr to [u32 len][labrun/1 JSONL]
void     mw_world_free(uint32_t world);
```

The web implementation mirrors the native gating: `gestureCommand` accepts ★ commands (and checks `document.visibilityState === "visible"`), and `send` rejects them, so UI code that would fail on a phone also fails in the browser. `startEngine` creates a world: local node 0 + 6 bots on a looping road scenario. `setInterval(20 ms)` advances time. Bots talk on a schedule with SimCodec frames (optional WebAudio playback of synthesized speech-like noise). PTT generates synthetic frames, so floor and busy logic run for real. `scanInviteQr` returns a generated sim invite. Lab benches run against simulated peers. Query parameters tune the scenario (`?sim=chain5&loss=0.1`). The web build is **off the Phase 0 field critical path** (§14).

### 11.3 App structure (`apps/mobile`)

```
src/
  app/App.tsx, app/router.ts      tiny screen-stack store; Android back via convLink useBackButton pattern
  engine/client.ts                engine-api instance
  engine/useEngineEvents.ts       single subscription: events → stores; notices → i18n → toast/banner
  store/useEngineStore.ts         last Snapshot + engine status (not persisted)
  store/useGroupsStore.ts         group metadata only (id, name, role, lastUsed) — persisted, NO secrets
  store/useSettingsStore.ts       language, theme, vehicle, default mode, PTT style, VOX sensitivity, prompts,
                                  share distance, haptics — persisted (name/version/migrate/merge)
  store/useLabStore.ts            runs list, live samples ring (bounded 600)
  (public/                        static assets: app icon sources for the web build, fonts; no audio assets)
  features/{onboarding,home,group,lobby,ride,members,settings,lab}/
  i18n/                           convLink infra: defineCatalog(en, es) with compile-time parity; catalogs per feature;
                                  `notices` and `prompts` namespaces map engine keys → text
  components/                     app compositions; primitives from @meshapp/ui-kit
tests/                            static contract tests: Info.plist, entitlements, PrivacyInfo, AndroidManifest
                                  (incl. no ACCESS_LOCAL_NETWORK at target 36, no MANAGE_OWN_CALLS by default),
                                  capacitor.config, WiFiAwareServices name rules
e2e/                              Playwright against vite dev + WASM sim
```

convLink rules carried over: TS strict + `verbatimModuleSyntax` + `erasableSyntaxOnly`; React Compiler on, no StrictMode; one Zustand store per domain, and non-React code uses `getState()`; every `catch {}` has a comment; no secrets in web storage; no text chat.

### 11.4 Screens

| Screen | Content | Commands / events |
|---|---|---|
| Onboarding | language, name, vehicle, permissions walkthrough, "Share my distance" choice (explains the keep-alive), headset check + Echo test, "don't swipe-kill", honest range promise | `requestPermissions`, `setDisplayName`, `setVehicle`, `setShareDistance`, `gestureCommand(audioCheck start tone/echo)`, `audioCheck stop`, `audioCheckSample` |
| Home | active/last group card (**Resume ride** after process death), Create, Join, Demo ride, Settings | `listGroups`, `startEngine` |
| Create group | name, vehicle, advanced: voice range (TTL 3-6), leader priority, invite expiry | `createGroup`, `setInviteExpiry` |
| Join confirm | group name, leader, display name, expired warning, Join | `parseInvite`, `joinGroup`, `startEngine` |
| Lobby | QR (natively rendered `qrPng`, auto-hide 60 s, "a screenshot of this code lets anyone join"), member list with ready checks and "voice reaches N of M" (best-effort members marked), unconfirmed badges, **Go live** (shows "Mic ready, muted until the ride starts"), Start ride (leader), Pair Wi-Fi (Phase 4); screen kept awake | `getInvite`, `goLive`, `rideControl`, `setKeepScreenOn`, snapshot |
| Ride | top: group name, "heard by k/n", island/alone/Bluetooth-off banner, ladder banner (L1-L3 with Resume); centre: speaking chips (≤ 4, name + level); bottom: **PTT/Mute 160 px** (hold or latch in PTT; tap toggles mute in VOX; ring shows input level), mode segmented control, route chip, busy banner ("Channel full — 2 talking"); screen kept awake in PTT mode | `ptt`, `setTalkMode`, `resumeMic`, `setKeepScreenOn`, snapshot, `talkDenied`, notices |
| Members | status, hops, voice reach (kpi / best effort / no), quality, hears-me, one-way, flags (on call, mic paused, no headset, radio off), battery, platform, distance/bearing; leader: Remove, Lock group (with the access-loss preview), Close group, Make leader | snapshot, `previewAccessLoss`, `removeMember`, `lockGroup`, `closeGroup`, `transferLeader` |
| Settings | language, theme, vehicle, default mode, PTT style, VOX sensitivity + VOX test meter (`audioCheck voxMeter`), spoken prompts, built-in mic TX, share distance, route preference, keep screen on while mounted (car), **Field test** (Lab, visible, plain copy), diagnostics export, about | settings commands, `gestureCommand(audioCheck)`, `audioCheckSample` |
| Field test | §12.5 | `lab*` |

Driving rules: minimum target 44 px (PTT 160 px), dark theme default, no text input on the ride screen, haptics on PTT press/release (opt-in), every state also announced by earcons so riders never need to look.

### 11.5 Design tokens and i18n

`@meshapp/ui-kit` ports convLink's Apple-like tokens (light `#f2f2f7/#1c1c1e/#ffffff`, dark `#08080a/#f5f5f7/#1c1c1e`, `--radius 1rem`, capsules, `--duration-ui 220ms`, `.glass` without blur on Android, reduced-motion/transparency fallbacks) plus intercom tokens: `--talk-active` (pulsing ring), `--talk-blocked` (amber), `--status-relayed` (blue), `--status-out` (zinc-500), `--ladder-warn`, `--danger`. The brand accent differs from convLink's green (final colour is the user's call). i18n: English source catalogs, Spanish parity enforced at compile time. Namespaces: `common`, `errors`, `notices`, `prompts` (WP-18); `lab` (WP-21); `onboarding`, `group`, `ride`, `settings` (WP-22). Intl locale `es` (or `es-DO` like convLink if the user prefers).

### 11.6 Capacitor 9

Capacitor 9 GA is forecast for end of November 2026. Because the plugin is TS-only and the bridges are two app-local files, the migration touches only those files and the generated `CapApp-SPM`. It is scheduled after GA (Phase 5 at the latest).

## 12. Lab mode (field benches)

Purpose: answer the research go/no-go questions (T0 throughput with headset, T1 range between vehicles, T2 background survival, T3 rediscovery) **on the user's iPhones in Phase 0**, using the code that will ship. The research rule is "do not build the product until T0, T1 and T2 say go".

### 12.1 Mechanics

- **Field test section:** Settings → Field test (visible, plain copy, no hidden gesture). Every phone enters the same 4-digit **Lab code**; no group is needed. HELLO `flags.lab = 1`, `group_tag = lab_tag`. Lab links use LAB frames over the **production** Swift BLE drivers and the **production Rust** link framing, records and scheduler, padded to the SEC frame size.
- **Identity:** a random `lab_node_id` per run with collision redraw (§8.2). The simulator asserts unique ids and, for every secured mode, global `(key, nonce)` uniqueness.
- **Controller:** one phone starts a run and sends `LAB_CTL{start, run_id, bench, params}`. Every phone records its own JSONL file for the same `run_id`.
- **Traffic is generated by `mesh-lab` inside the core**, so Android gets the same benches for free and the simulator can run them: `LPROBE` patterns (saturate, voice-like CBR padded to exact VOICE frame sizes, ping), `MESH_PROBE` for multi-hop over the **minimal Lab flood** (TTL + dedup only, in `mesh-core/lab`), `LAB_GPS` at 1 Hz.
- **Clocks:** per-link offset and RTT from PING/PONG (NTP-style four timestamps, min-RTT filter) give one-way estimates. Raw timestamps are exported for offline re-analysis.
- **Audio path per run (`LabParams.audio`, mandatory):** `off` (radio-only benches), `renderOnly` (session + VPIO output: keeps the `audio` anchor, plays bench tones, no mic), or `full` (session + VPIO capture and render, HFP opened when a headset is connected). `full` and `renderOnly` are started with `gestureCommand` inside the Start tap, exactly like Go live (§3.2), so Lab audio follows the same consent and OS rules as the product. Lab capture is never transmitted as voice; it feeds only meters, the VOX-calibration recorder and the T5a correlator.
- **Keep-alive anchor per run (`LabParams.anchor`, mandatory, recorded in the `run` line):** `audio` (VPIO running, render-only or full; the product's default anchor), `audioLocation` (plus `CLBackgroundActivitySession` + live location updates; needs the Location permission, requested in the foreground at run setup), or `none` (control variant that measures suspension itself; never the default). Benches with pocket, locked or background variants (T1, T2, T3, T6) default to `audio` (VPIO running with the capture sink muted when `audio = full`, or `renderOnly`), with `audioLocation` as a variant, which mirrors the product's L0/L3 anchors. On Android the FGS (`connectedDevice` [+`microphone`, +`location`]) started in the tap is the anchor.
- **Platform samples** at 1 Hz: GPS fix (per-run consent), battery and charging, thermal state, app state, device locked (protected-data notifications), audio IO counters (`mesh_rt_stats`), route, ladder level, anchor state (session active, location session active, Live Activity active), Bluetooth radio state.
- **Ride recorder (Phase 2):** during real rides, if enabled, 1 Hz samples of all links (kind, q, RTT, loss, RSSI), reachability and islands, floor events, audio counters and GPS (if permitted), with `bench: "ride"`. Every ride becomes T1/T2 data.

### 12.2 Benches

| Bench | Setup | Variants (each a separate run) | Captured per second | Go / no-go (thresholds in `tools/lab-analyze/thresholds.json`) |
|---|---|---|---|---|
| **T0** BLE throughput/latency/loss (audio `off` for headset-off variants, `full` for headset-on; anchor `audio`) | 2 iPhones at 1-5 m, then 20 m | transport {L2CAP, GATT notify, GATT write-w/o-resp} × **role {A central/B peripheral, swapped}** × headset {off, HFP active with the Lab session running, `audio = full`} × pattern {saturate 20 s, voice-like 1-4 streams of 118 B / 40 ms for 60 s, ping 64 B @ 10 Hz, ping under load} | tx/rx kbps, loss, reorder, stalls > 100 ms, RTT p50/p95/p99, one-way estimate, ATT max write, RSSI, inferred connection interval (RTT quantization histogram) | **≥ 2 voice-like streams (≈ 47 kbps) at < 5 % loss and p95 one-way < 60 ms with the headset active** ⇒ `S_MAX_BLE = 2` confirmed; ≥ 150 kbps ⇒ BLE-fast allowed |
| **T0b** relay chain (audio `full` on B when headset on) | 3 iPhones A–B–C, A and C out of direct range (or forced by `LAB_CTL` blacklist) | B with headset {off, on}; 1-2 voice-like streams; B's node airtime measured | as T0 + per-hop latency via MESH_PROBE trace | 2 streams relayed at < 5 % loss, added p95 latency per hop ≤ 40 ms |
| **T0c** multi-link hub (audio `full` on H; anchor `audio`) | 4-5 iPhones: hub H with 3, then 4 simultaneous BLE links (mixed central/peripheral roles) + an active HFP headset on H. With only 3 iPhones, H runs 2 links (+ headset) and the target/max stay 2 / 3 | voice-like streams relayed through H: 2 streams out on 2, 3, then 4 links; H's pacer free-running vs capped at the §5.6 budget | per-link and **node-total** tx kbps, loss, p95 one-way, H's `WouldBlock`/drop counters, HFP audio glitches (`mesh_rt_stats` underruns), thermal | sets `NODE_BLE_BUDGET` and the BLE link target/max: target 3 / max 4 only if H sustains 3 links × cost(2) (≈ 150 kbps) at < 5 % loss with HFP glitch-free; otherwise keep 2 / 3 |
| **T1** range between vehicles (audio `renderOnly`, anchor `audio`; variant `audioLocation`) | 2-3 vehicles, phones mounted / pocket / tank bag (marked), 30-60 min mixed road | continuous: 2 voice-like streams + ping 5 Hz; mark button for passengers or stops | own + peer GPS, distance, speed, RSSI, loss, RTT p95, link up/down, reconnect duration, anchor state | useful separation (≥ 95 % of 1 s windows < 5 % loss) ≥ the user's typical convoy gap (Appendix E); runs whose anchor dropped are excluded, not counted as radio loss |
| **T2** background survival (audio `full`) | 1 iPhone (+ 1 peer for link continuity), session live, phone locked | 4 h baseline; anchors {`audio`, `audioLocation`, `none` (control)}; session {non-mixable, mixable (Lab toggle)}; scripted: helmet off/on (HFP reconnect), incoming call, Siri, music app, navigation prompts, Low Power Mode, Bluetooth toggled off/on; **TTS prompt rendering while locked**; output-only fallback; Live Activity at the 8 h limit (long run) | capture/render fps, VPIO running, interruptions (type/reason/resume), route changes, ladder level, BLE links up, thermal, battery %, app state | **0 silent mutes** (every capture stop produced CaptureLost + notification), automatic recovery or one guided tap; relaying continues during a call with the location anchor |
| **T3** rediscovery (audio `renderOnly`, anchor `audio`; `none` as control) | 2 iPhones (3 for relay rejoin) | {both fg, one bg, both bg screen off, both bg + Live Activity, pending `connect()` / EnableAutoReconnect, behind-epoch peer (speculative link, §9.3)}; break by `LAB_CTL` blacklist for 30 s and by walking apart; 5 trials each. **Phase 3 [DEV] variant:** Android scanner detects a backgrounded iPhone via the overflow bitmask | time to adv seen, connect, HELLO, handshake, first probe; app, screen and anchor state | rejoin ≤ 10 s in the variants the product uses; if "both bg" fails ⇒ lobby-formed links + Live Activity + Aware rejoin (Phase 4) and honest copy; the Android variant switches the overflow-bitmask scan on only if it rejoins ≤ 10 s |
| **T5a** headset loopback latency (audio `full`) | 1 iPhone + helmet; helmet speaker against helmet mic in a quiet room | per headset model; HFP WB/NB as negotiated; **VPIO with `BypassVoiceProcessing = 1`** so AEC does not cancel the loop | core plays an MLS-style click train into the mixer; a correlator finds it in capture ⇒ round trip (OS + both HFP legs); 50 repetitions | per-leg estimate fits the 25 ms/leg budget; otherwise retune JB/profile (Appendix B) |
| **T5b** headset behaviour (Phase 1, audio `full`; scripted checklist + recorded counters) | 1-2 iPhones + each helmet model the user owns | NREC/AEC: does the helmet's own echo canceller fight VPIO (echo-return measured with the click train, VPIO on); music ducking with navigation + music apps; helmet buttons (which act on the call, which on media, which do nothing); interference with the helmet's own mesh/intercom running at the same time | echo return loss, audible artefacts (rider-marked), route changes, `mesh_rt_stats` glitches, button events seen by the OS | a per-model note in the Phase 1 report; "disable the helmet's own intercom while using the app" copy if interference is seen |
| **T6** battery and thermal (audio `full`, anchor per variant) | 1-2 iPhones on a real ride, 2-4 h | {screen off, screen off + navigation app, screen on (car mount)} × {charging, not charging} | battery %/h, thermal state, throttling events, links, ladder | records %/h; proposed target < 5 %/h screen off (research T6). **Phase 0 runs ≥ 1 h per variant to set the Phase 5 battery target**; Phase 5 repeats 4 h |
| **T8** UI and PTT input latency (Phase 1, audio `full`) | 1 iPhone (Android later), Demo world with 15 simulated peers | members screen at 15 Hz updates; PTT press/release 100 × | touch timestamp in JS (`tUiMs`), native receipt in the bridge, `mesh_rt_set_flags` time, frame times of the WebView | touch → `mesh_rt_set_flags` p95 < 50 ms and no dropped frames > 50 ms (research T8); otherwise move the PTT button to a native overlay (v1.x) |
| **VOX calibration** (Phase 1, audio `full`) | 1 rider with helmet riding; PTT held while actually speaking (passenger or rider at stops) | sensitivity {low, medium, high}; wind conditions marked | VAD decisions vs PTT ground truth, p, noise floor; optional consented 60 s raw clips (local only) | false triggers ≤ 1/min at 80 km/h and onset delay ≤ 150 ms at Medium |
| **T4** Wi-Fi Aware (Phase 4) | 2-3 paired iPhones | realtime vs bulk; single-role (riding-order chain or star) vs both-roles test; **battery %/h realtime vs BLE-only** | as T0 + max connectable devices + battery | upgrade is worth it if ≥ 4 streams at < 2 % loss; battery cost recorded for the opt-in copy |

### 12.3 Export format `labrun/1` (JSON Lines, one file per device per run; contract C7)

```json
{"t":"run","v":1,"runId":"r-20261023-0912-ab12","bench":"T0","variant":{"transport":"l2cap","role":"central","headset":"hfp-wb","pattern":"voice4"},"audio":"full","anchor":"audio","startedAt":"2026-10-23T09:12:03.120Z","device":{"model":"iPhone15,2","os":"iOS 27.0","app":"0.1.0 (12)","platform":"ios"},"labNode":4711,"role":"controller","peers":[{"labNode":918,"model":"iPhone13,1"}],"params":{"durationS":60,"streams":4,"payloadB":118,"intervalMs":40}}
{"t":"s","ts":1.000,"links":[{"id":3,"kind":"ble-l2cap","role":"central","rssi":-61,"txKbps":93.4,"rxKbps":93.1,"loss":0.004,"rttP50":38.0,"rttP95":61.5,"owdP95":31.2,"stalls":0,"maxWrite":null}],"gps":{"lat":18.4861,"lon":-69.9312,"spd":0.0,"acc":4.8},"peerGps":[{"labNode":918,"lat":18.4862,"lon":-69.9311}],"distM":12.4,"audio":{"capFps":100,"renFps":100,"running":true,"route":"hfpWideband","ladder":"L0","xruns":0},"dev":{"bat":0.81,"chg":false,"therm":"nominal","app":"background","locked":true}}
{"t":"e","ts":13.104,"ev":"linkDown","link":3,"reason":"timeout"}
{"t":"mark","ts":20.0,"label":"pocket"}
{"t":"raw","ts":21.0,"pings":[[1203,1.000000,1.038112,0.000210]]}
{"t":"summary","durationS":60.0,"txKbps":{"mean":92.8},"loss":0.006,"owdMs":{"p50":19.1,"p95":31.2,"p99":44.0},"stalls":0,"verdict":"go"}
```

The JSON Schema lives at `crates/mesh-lab/schema/labrun-1.schema.json`. A `…-summary.csv` (one row per run) is written next to it. Files live in Documents/lab (iOS, visible in Files) or app-specific external files (Android) and are exported with the share sheet (AirDrop to the Mac). `tools/lab-analyze` merges devices by `runId`, emits CSV and prints verdicts against `thresholds.json`.

### 12.4 Phase 0 exit artifact

`docs/field/results/phase0/memo.md` is a decision memo that applies Appendix B: `S_MAX_BLE` confirmed or changed, BLE-fast yes/no, L2CAP vs GATT default, preferred role on iPhone↔iPhone links, `NODE_BLE_BUDGET` and the BLE link target/max (T0c), `VOICE_MAX_AGE` and initial JB values, the battery target (T6), which keep-alive anchors are needed, mixable or not, the rediscovery strategy, and a **go / no-go** per research §9. Automated results and "pending physical" items are kept separate; nothing physical is faked.

### 12.5 Field test UI

Bench list with the go/no-go thresholds shown; per-bench config sheet with GPS consent, the audio path and the keep-alive anchor (defaults per §12.2; `none` labelled "control: the phone may suspend"); the Start button calls `gestureCommand(labStart)` whenever audio ≠ off, and the screen is kept awake until the run starts; live view (big numbers + 60 s sparklines: kbps, loss, RTT p95, RSSI, distance); mark button (with the passenger/stop safety note); results list with verdict chips; export/share; the procedure text from `docs/lab/LAB_PROTOCOL.md` (ES/EN).

### 12.6 Built-in latency estimator (Phase 1+)

No clock synchronisation is needed. The MeshPacket `age` field already accumulates origin queueing, per-hop residence and per-link one-way estimates. Each receiver computes, per voice frame, `capture→ear (excluding headset legs) = frame_dur + 6.5 ms look-ahead + encode + tick alignment + age_at_arrival + jitter-buffer wait + mix/render buffering (from ring fill)`, and keeps p50/p95 per origin and per hop count. It is shown in diagnostics and the snapshot and exported in ride-recorder files. Combined with T5a (headset legs) it gives full mouth-to-ear estimates without acoustic rigs. Accuracy is bounded by the 2 ms age resolution and the RTT/2 symmetry assumption.

## 13. Testing strategy

### 13.1 Layers

| Layer | Tooling | What is asserted |
|---|---|---|
| Wire codecs | `cargo test`, `proptest`, golden vectors | round trip for every record and ptype; vectors byte-identical (`cargo xtask vectors --check`); malformed input never panics; the §4.8 size table; GATT chunking at MTU 23/185/517 |
| Crypto | `cargo test` + KATs + vectors | HKDF/HMAC KATs; Noise XXpsk1 between in-memory peers incl. wrong PSK, wrong epoch, replayed msg1, stranger msg1 (no static key revealed); raw-split keys + `chacha20poly1305` SEC with AAD (a flipped lf_type or kp_ctr byte fails the tag); in-band key update incl. reordered old-phase frames; replay window (proptest, updated only after auth); invite codec incl. expiry and `invite_sig`; roster/JOIN/REKEY/GROUP_CTL/LEAVE/RIDE_CTL signatures; **monotonic acceptance** (older roster/REKEY/GROUP_CTL rejected); IDPROOF binding; per-group identity unlinkability (distinct keys per group); no `getrandom` call (custom resolver) |
| opus-sys | `cargo test -p opus-sys` | upstream libopus API/encode/decode tests compiled with `cc`; ctl shims round-trip each setting |
| Routing | unit + proptest | dedup windows keyed by `(origin, sess)` incl. wrap and new-session; **pruning equivalence** for ctrl/bulk on random geometric graphs; **voice never pruned** + delivery under random floor contention with per-link caps; TTL/age drops; pacing step responses; voice-aware node airtime scaling (idle links at 8 kbps); **cap steady state** (chain node, 3-link hub, 2 voice + 1 idle: cap stays 2 with BLE16; hysteresis 2 s down / 5 s up); articulation rule; `LimitReached` backoff |
| Floor | unit + proptest | **convergence** (all nodes reach the same admitted set within one TALK propagation), **safety** (no link ever carries more than its cap), **liveness** (a freed slot is taken by the next-ranked waiting talker); stickiness; leader preemption; **mid-burst join** (a node that missed TALK START ranks the burst by `ts`/`elapsed` and agrees within one KEEP) |
| Voice | unit + golden audio | Opus WB/NB round trip (spectral-distance thresholds); `PACKET_LOSS_PERC = 0` whenever LBRR is off; LBRR recovery path; deep PLC on loss; JB ordering and adaptation (proptest on arrival permutations); limiter bounds; VAD precision/recall floors on fixture clips (speech, wind, engine; 16 kHz WAVs, ≤ 2 MB in `crates/mesh-voice/fixtures`); **no allocation** in `push_capture`/`pull_playout` (counting allocator) |
| Core | unit + scenario | link state machine (HS → IDPROOF → CAPS → SYNC → up), collision REJECT, duplicate-link resolution, group flows (create/join/roster/remove/lock/close/transfer), anti-entropy (§8.9) and JOIN_REQ retransmission, retained-epoch acceptance and speculative links, capture gating (`StartCapture` only on `goLive` / `resumeMic` / `labStart{audio: full}` / `audioCheck{start}`; property test over random sequences), Lab audio paths and anchors, `RadioState` handling, ladder transitions, command/result contract, snapshot throttling, i18n-key-only events |
| Simulator | `mesh-sim`, fixed seeds per PR, random seeds nightly | §13.2 |
| Fuzzing | `cargo +nightly fuzz` | targets: stream deframer, GATT reassembly, LinkFrame, records, mesh header, each ptype, segmentation, invite URL, HELLO, Noise handshake input; 5 min per target nightly |
| FFI | `cargo test -p mesh-ffi` | RT rings under concurrent producer/consumer stress; start/stop cycles; RT flags; stats; panic → `crashed`; `Vec<u8>` link-data path |
| Swift | `xcodebuild test` on an iOS Simulator (`scripts/pick-sim.sh`) | adv/HELLO vector replay, session-configuration logic (mocked session wrapper), watchdog FSM, SecureStore, LabFileStore, LinkRegistry; BLE drivers behind protocols with fakes (CoreBluetooth does not work in the simulator); MeshEngineKit binding smoke and Demo ride with audio |
| Kotlin | `./gradlew :meshengine:testDebugUnitTest` (JVM) | bindings against the host `libmesh_ffi.dylib` via JNA; FGS type computation from grants (no location type without permission); scan/adv policy (quota, 9-min restart, legacy connectable scannable params); notification builder; permission matrix |
| TS | Vitest (node + jsdom) | engine-api guards, stores (persist migrate/merge), i18n parity incl. `notices`/`prompts` keys for every engine key, components (ride screen states and ladder banners from fixture snapshots), PTT pointercancel/visibility handling, ★ commands only via `gestureCommand` (lint rule + test), lobby and PTT ride screens call `setKeepScreenOn(true)` on mount and `false` on unmount |
| E2E web | Playwright against `vite dev` + WASM sim | create → QR → join (sim) → Go live → Start ride; PTT shows talking; 3rd talker gets the busy banner; partition banner appears and clears; Field test T0 sim run exports a schema-valid file |
| Static contracts | `apps/mobile/tests/*.test.mjs` | Info.plist keys, background modes (exact set, no voip/push-to-talk), URL scheme, scene manifest, PrivacyInfo, `WiFiAwareServices` name rules; Android manifest permissions incl. `neverForLocation`, FGS types, absence of `ACCESS_LOCAL_NETWORK` at target 36 and of telecom permissions by default; capacitor.config |
| Device / field | Lab benches + scripted checklists | radios, HFP, background, rediscovery, battery, review demo — **devices only** |

### 13.2 Simulator design and scenario suite (`mesh-sim`, research bench T9)

- **Model:** discrete-event, µs clock, seeded `ChaCha20Rng`, N real `Engine`s. Radio per pair: BLE PDR(d) logistic with `R50` = 50 m (pocket) / 80 m (mount), log-normal shadowing σ = 6 dB per 5 s; link drop after 3 s below PDR 0.5. Per-hop delay = CI phase (30 ms for iOS-central links, 15 ms Android↔Android) + serialization at 120 kbps per link and a per-node airtime limit **equal to `NODE_BLE_BUDGET`** (read from `Limits`, 180 kbps by default, so the sim fails exactly where the design's budget fails; a headset eSCO reservation is modeled inside it) + retransmissions (geometric in PDR, one CI each). Aware: R50 120 m, 5 ± 2 ms, 20 Mbps. Discovery: exponential time to discover (foreground 1.5 s mean; iOS bg↔iOS bg configurable including "never"); connect 1-3 s; handshake 2 RTT. The role constraints of §9.2 are enforced per node platform. Mobility: a 1-D road with curves, speed profiles 0-33 m/s, stops, gap distributions. Audio: scripted talk schedules with SimCodec frames carrying the capture timestamp, so modeled mouth-to-ear (excluding headset legs) is exact.
- **Global assertions in every scenario:** no `(key, nonce)` pair is ever used twice (all modes, Lab included); no Lab or node id collision survives; no link exceeds its cap; no panic.

| ID | Scenario | Assertions |
|---|---|---|
| S1 | pair-ble: 2 nodes, 10 m | modeled m2e p95 ≤ 190 ms; loss ≤ 1 %; delivered duplicates = 0 |
| S2 | chain5-ble: 5 nodes, 45 m spacing | listeners at 1/2 hops p95 ≤ 210/250 ms excl. headset legs (≤ 260/300 ms with the 50 ms T5a leg budget = KPI3); 3-hop p95 **reported** (best effort, expected ≈ 265-300 ms excl. legs); loss ≤ 3 %; 4-hop listener gets frames only if age ≤ 150 ms; cap stays 2 on every link |
| S3 | cluster15: 15 nodes within 30 m, 2 talkers | voice transmissions per originated frame ≤ 2.25 × 14 (degree bound, no voice pruning); pruned control floods cheaper than unpruned (metric); per-link load ≤ 60 % of modeled capacity; node airtime ≤ `NODE_BLE_BUDGET`; cap 2 on every voice link at steady state |
| S4 | convoy15-highway: 15 nodes, 90-110 km/h, log-normal gaps (median 40 m, 20-500 m), 30 min, VOX schedules | partition detected ≤ 6.5 s after a physical split; rejoin ≤ 10 s after back in range (fg discovery model); ≥ 95 % of frames delivered to listeners reachable within TTL; p95 m2e excl. headset legs ≤ 250 ms at ≤ 2 hops (= 300 ms with legs); 3-hop p95 reported, not asserted |
| S5 | floor-contention: 6 nodes, 3-4 near-simultaneous VOX starts, leader among them, plus one node that links up mid-burst | ≤ 2 streams per BLE link at all times; losers busy within 300 ms; leader always heard; no equal-priority eviction after `T_STICKY`; all nodes (incl. the late one) agree on the admitted set within 300 ms of hearing a TALK KEEP or VOICE |
| S6 | mixed-roles: 4 iOS + 3 Android, iOS backgrounded | connected whenever geometry allows, using iPhone-central links only toward Android; no Android-initiated link to an iPhone |
| S7 | aware-upgrade: iOS pair with BLE + Aware, Aware fails mid-burst | ≤ 1 lost frame at the switch; cap re-evaluated |
| S8 | restart: a node's app restarts mid-ride (new `sess`) | zero frames of the new session dropped as duplicates; presence back ≤ 4 s after relink |
| S9 | adversarial neighbor: malformed records, replays, forged origin at hops 0, stranger msg1 | all dropped, no panic, neighbor banned after the threshold, no static key revealed |
| S10 | lab-bench-sim: T0/T0b/T0c/T3 against simulated peers, each audio path and anchor | labrun/1 files validate against the schema (incl. `audio`/`anchor`); Lab id collision is resolved; `StartCapture` appears only for `audio = full` |
| S11 | stale-pruning: cluster + chain mix with links breaking every 5-20 s and congested links | pruned flooding loses ≤ 1 % more than plain flooding; never prunes a congested or refused link |
| S12 | membership: remove member + lock group during a ride; a roster member on the old epoch rejoins (as BLE central behind an ahead peripheral, and vice versa); transfer leadership | denied node isolated within 2 s of hearing the roster; old-QR stranger refused; old-epoch roster member connects via the retained-epoch rule / speculative link and receives the REKEY; tags of retained epochs accepted; after transfer, only the new leader's ROSTER is accepted |
| S13 | anti-entropy: a member partitioned during Remove + Lock (and a late joiner whose JOIN_REQ was sent while the leader was unreachable) reconnect after 5-30 min; replayed older ROSTER injected | both converge to the newest epoch, roster and ctl versions within 10 s of the first link; the partitioned member closes its link to the removed member; the late joiner appears in a verified ROSTER; the replayed roster is ignored |

### 13.3 Where each check runs

| Check | This Mac | iOS Simulator | Android emulator | Devices |
|---|---|---|---|---|
| Rust tests, simulator, fuzz, vectors | ✓ | | | |
| Rust cross-builds (iOS, iOS-sim, Android arm64/x86_64, wasm32), bindings drift | ✓ | | | |
| Swift packages build + unit tests | | ✓ | | |
| App build unsigned (device + simulator), Demo ride audio via the Mac mic | | ✓ | | |
| Kotlin JVM tests, lint, AAR/APK builds | ✓ | | | |
| App launch, onboarding, permissions UI, Demo ride audio | | | ✓ (no BLE) | |
| TS unit, Playwright e2e against the WASM sim, static contracts | ✓ | | | |
| BLE links/throughput, HFP behaviour and latency, background survival, rediscovery, battery/thermal, Wi-Fi Aware, App Review behaviour | | | | ✓ iPhones now; Android later |

## 14. Phased roadmap

Durations assume one developer orchestrating parallel coding agents. WP ids refer to `docs/WORKPLAN.json` (§16).

### Phase 0 — "Field test on iPhones" (target 2-3 weeks)

**Goal:** run T0, T0b, T0c, T1, T2, T3, T5a and a ≥ 1 h T6 on the user's iPhones (3+; T0c's 3-link hub needs 4) using the production Swift BLE/audio drivers and the production Rust link framing, each run with an explicit audio path and keep-alive anchor (§12.1), and prove the whole pipeline (Rust → xcframework → SPM → Capacitor app → device).

**Critical path:** WP-00 toolchain (incl. MSRV probe) → WP-01 scaffold (crate stubs, lint config) → WP-04 wire (link framing, records, Lab records, vectors) + WP-05 mesh-lab + WP-03 (Lab subset of the API) → WP-10A core (Idle + Lab modes, Lab audio paths/anchors, minimal Lab flood) → WP-12 FFI + xcframework → WP-14 MeshPlatformKit (BLE, VPIO via CMeshAudio, keep-alive, samplers, Lab store, MeshActivityShared) + WP-15 EngineHost → WP-18 app shell + WP-19 bridges (incl. `gestureCommand`, `setKeepScreenOn`, native app builds) → WP-21 Field test UI → device install → WP-F0.
**Parallel, off the critical path:** WP-02 conventions, WP-06 libopus `cc` build + cross-target smoke build (retires the libopus risk now; once it lands, every Rust verify runs `cargo xtask fetch-opus` first), WP-07 crypto start, WP-11 (Phase 0 milestone: S10 only), WP-17 plugin TS, WP-20 ui-kit, WP-23 minimal Live Activity (for the T3 variant; if late, that variant moves to Phase 2: encoded as a soft dependency of WP-F0), WP-16 Android skeleton build (after WP-12, because it compiles the committed Kotlin bindings).
**Not in Phase 0:** WASM/web simulator (WP-13), full product UI, voice codec in the app.
**Process:** request the Wi-Fi Aware entitlement now; register bundle id `com.convlink.voice` with the background modes.

Acceptance:
- [ENV] `cargo xtask fetch-opus && cargo test --workspace` green incl. S10; clippy `-D warnings` green with the per-crate lint split; vectors checked; `cargo xtask opus-smoke` builds `opus-sys` for `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android` and `x86_64-linux-android` (NDK linker env from `scripts/env.sh`); `cargo xtask build-ios` produces the xcframework; MeshPlatformKit and MeshEngineKit tests pass on a simulator; the app builds unsigned for device and simulator; static contract tests pass.
- [DEV] the app installs on ≥ 3 iPhones via Xcode. T0 (all transport × role × headset variants) and T0b complete and export. T0c with as many links as the available phones allow. T1 ≥ 30 min drive logged at ≥ 1 sample/s with GPS, anchor `audio` (pocket variant included). T2 ≥ 60 min locked log with the scripted events, each anchor variant. T3 5 trials × each variant. T5a for each helmet model the user owns. T6 ≥ 1 h per variant (battery %/h baseline for the Phase 5 target).
- **Exit artifact:** the Phase 0 memo (§12.4) with go / no-go.

### Phase 1 — "Two-iPhone intercom, 1 hop" (3-4 weeks)

Deliverables: WP-07 crypto (per-group identities, invite, Noise XXpsk1, link AEAD, IDPROOF, roster/JOIN); WP-08 voice (profiles, VAD + wind guard, talk FSM, JB, mixer, earcons); WP-09 routing (neighbor table, direct delivery, floor table + admission, presence); WP-10B/10C session, go-live, ladder, media; WP-11 sim S1, S5, S8, S9; WP-12 Demo ride world; WP-13 WASM sim + web plugin; WP-14/15 full audio path, Keychain, QR scanner, notifications, mic-loss FSM and output-only fallback, radio-state monitor, idle-timer control; WP-22 onboarding (headset check, Echo test), home, group, lobby, ride, settings (VOX meter); VOX-calibration, T5b and T8 benches.

Acceptance:
- [ENV] all unit/property tests; S1, S5 (2-node subset), S8, S9; Opus round trip and VAD fixture thresholds; Playwright create → join → Go live → talk against the WASM sim; Demo ride runs in the iOS Simulator with audio.
- [DEV] 2 iPhones + helmets: QR create → join → linked ≤ 30 s; VOX and PTT both directions; latency estimator p95 ≤ 200 ms (excluding headset legs) at 1 hop; 60 min locked ride with no silent mute (every capture loss produced a notification and the L2 fallback kept audio playing); automatic relink after 30 s apart; busy tone verified with a 3rd phone talking (cap 2); T8 touch → `mesh_rt_set_flags` p95 < 50 ms; T5b notes per helmet; Bluetooth toggled off/on mid-ride produces the cue and recovers.

### Phase 2 — "iPhone multi-hop mesh, 3-6 phones" (3-4 weeks)

Deliverables: flooding (voice unpruned, ctrl/bulk pruning), TTL/age, pacing + voice-aware node airtime budget + cap hysteresis, stream-level shedding, per-link admission + FLOOR_REJECT + near-talker rejection + PATH_HINT, link selection policy, islands / who-hears-me / one-way UI, **Remove member / Lock group / Close group / Transfer leadership + REKEY + anti-entropy**, signed LEAVE/RIDE_CTL, distance sharing (POSITION + location anchor), pre-rendered spoken prompts, full Live Activity (WP-23, incl. LatchTalk), ride recorder, full simulator suite S1-S13.

Acceptance:
- [ENV] S1-S13 green on fixed seeds; nightly random seeds ≥ 99 % pass; property tests for ctrl pruning equivalence, voice delivery under random per-link floor contention, cap steady state, and floor convergence/safety/liveness (incl. mid-burst join).
- [DEV] 3 iPhones A–B–C with A and C out of direct range (UI shows "relayed, 1 hop"): voice both ways with estimator p95 ≤ 235 ms (excl. headset legs); 3 simultaneous talkers ⇒ the third gets busy within 300 ms; partition shown ≤ 7 s; rejoin ≤ 10 s with the Phase 0 strategy; lock group ⇒ an old QR on a 4th phone is refused, and a roster member who was away during the lock rejoins and converges; locked-screen PTT via the Live Activity latch works in the car profile; a 2 h group ride recorded; T2 confirms prompt rendering while locked (or prompts stay earcon-backed).

### Phase 3 — "Android parity, build-verified" (parallel lane from the end of Phase 1, 3-5 weeks)

Deliverables: WP-16 Kotlin library (FGS with grant-computed types, Oboe shim, route controller, BLE scanner/advertiser/GATT server/L2CAP, SecureStore, notifications, TelecomStrategy behind the flag), WP-19 Android bridge, WP-18 Android app project + manifest contract tests; S6 green.

Acceptance:
- [ENV] `./gradlew :meshengine:assembleRelease testDebugUnitTest lintDebug` and the app's `assembleDebug` green; Kotlin binding tests run against the host dylib; the emulator launches the app with onboarding, permissions UI and Demo ride audio; S6 green.
- [DEV, deferred until an Android phone exists] iPhone (central) ↔ Android (peripheral) link and voice; **T0 iPhone↔Android with role and headset variants** (the open gate of §9.2); **T3 variant "Android detects a backgrounded iPhone via the overflow bitmask"** (decides whether that scan is enabled); FGS 60 min locked; Android 17 hardening `throw` mode clean; OEM checklist on the first Samsung/Xiaomi device. Android is labelled "supported" only after this.

### Phase 4 — "Wi-Fi Aware upgrade" (2-3 weeks, after the entitlement)

Deliverables: WP-24 iOS Aware (riding-order chain or star pairing UX with 2-colouring, realtime/interactiveVoice links, upgrade protocol, bundles), Android↔Android Aware (PMK), T4, S7.
Acceptance: [ENV] S7 green, builds; [DEV] 3 paired iPhones switch BLE↔Aware with ≤ 1 lost frame; 4 simultaneous talkers on an all-Aware path; T4 report incl. the single-role result.

### Phase 5 — "Hardening and store readiness" (3-4 weeks)

Leader-priority tuning, thermal/battery adaptation, OEM onboarding, App Review notes + video with Demo ride, Play FGS declaration videos, privacy labels / Data safety (peer-to-peer location), export-compliance docs, accessibility pass, diagnostics export, Capacitor 9 migration (if GA), optional BLE-DRED experiment behind caps.
Acceptance: [ENV] full CI green; [DEV] TestFlight external build approved; Play internal track accepted (when an Android device exists); a 4 h T6 ride meets the battery target set in the Phase 0 memo from the T6 baseline. WP-25 changes app files only through contract change requests to their owners (WP-18, WP-19, WP-22).

### Phase 6 — v1 release; v1.x backlog

Group map, helmet/handlebar PTT evaluation, Silero VAD if Lab data justifies it, per-burst signatures, leader-approval admission, co-leaders, public channels, Telecom strategy if validated, v2 tunnel (Supabase/WebRTC) and convLink integration.

## 15. Risk register

| # | Risk | Mitigation in this design | Early detection |
|---|---|---|---|
| R1 | Real inter-vehicle range ≪ 20-500 m spacing ⇒ constant islands | Multi-hop flooding; honest islands / who-hears-me / distance UI and cues; age-capped voice; honest promise in onboarding and store copy; ride recorder quantifies | **T1 in Phase 0**; go/no-go memo |
| R2 | iOS background capture dies on route change or interruption and cannot restart | Raw VPIO never stopped; interruption FSM incl. iOS 27 APIs; MuteWatchdog; CaptureLost ⇒ L2 output-only fallback + notification + Live Activity + PRESENCE flag; location anchor keeps relaying (L3) | **T2 in Phase 0** (scripted helmet reconnects, calls, Siri) |
| R3 | Locked iPhones cannot rediscover each other; Android cannot see backgrounded iPhones | Role rule (iPhone central); lobby-formed links kept alive; Live Activity privileges; pending reconnect variant; Aware rejoin (Phase 4); honest copy | **T3 in Phase 0**; S6 |
| R4 | BLE capacity with HFP active below the estimate | Delay-based pacing; voice-aware node airtime budget sized per link; cap = min(kind, pacing) from the explicit cost model with hysteresis; 40 ms profile; BLE12 via PATH_HINT; stream-level shedding; link target 2 / max 3 until T0c | **T0/T0b/T0c with headset in Phase 0** |
| R5 | No iPhone↔Android Wi-Fi Aware | BLE baseline for every pair; Aware optional per link; empty allow-list until T4 | T4 (Phase 4) |
| R6 | Helmet HFP latency ≫ 25 ms per leg | JB and profile tunable by policy; BLE-fast profile; estimator + T5a per headset | **T5a in Phase 0** |
| R7 | Wi-Fi Aware PIN pairing UX for many riders | Aware optional; riding-order chain or leader star (N−1 pairings) in the lobby; pairings persist | Phase 4 usability check |
| R8 | Android 17 / OEMs kill or silence the service | FGS from visible taps with grant-computed types, `START_NOT_STICKY` + guided Resume, wake lock, battery exemption after first ride, OEM pages, hardening test; Telecom as a flagged experiment | Phase 3 [DEV] |
| R9 | Store rejection (2.5.4 background modes, 2.5.14, FGS declarations) | Only modes actually used; `location` backed by the visible distance feature; explicit Go live + visible mic state; Demo ride + video; Play fallback to `microphone` FGS only | Phase 5 TestFlight / Play review |
| R10 | Battery/thermal on 2-4 h rides | VAD-gated TX, pacing, LOW_POWER adverts when stable, thermal profile step-down, location anchor opt-in, Aware opt-in, ride-recorder battery data | **T6 baseline in Phase 0**, T2 / ride recorder |
| R14 | Leader phone lost: nobody can remove, lock or close | Signed transfer-leadership while present; documented "new group" recovery | Phase 2 UX check |
| R15 | Control state diverges after partitions (stale roster, missed REKEY) | Anti-entropy on link-up and PRESENCE mismatch; JOIN_REQ retransmission; monotonic acceptance | S13; Phase 2 [DEV] lock test |
| R11 | "Cardo replacement" expectations | Honest promise, no trademarks in store metadata, reachability UI | Phase 1 user test |
| R12 | Nonce reuse, insider spoofing, tracking | Per-link Noise (no persisted nonces), first-hop origin check, rotating tokens, per-group identities, no IDs on air, fuzzed parsers, ingress caps, simulator `(key, nonce)` assertion | Fuzz nightly; S9 |
| R13 | Music degraded to mono while HFP is open | Documented in onboarding; PTT-with-SCO-release mode deferred (latency cost) | Rider feedback, Phases 1-2 |
| A1 | UniFFI 0.32 pre-1.0 churn | `=0.32.2` pin; `Vec<u8>` signatures; committed bindings with drift check; engine isolated from the plugin | CI drift check |
| A2 | libopus cross-build fails on a target | `cc` build with SHA-pinned sources and weights, no CMake; cross-target smoke build in Phase 0; opusic-sys + SDK cmake fallback | Phase 0 `cargo xtask opus-smoke` |
| A3 | iOS L2CAP output buffering hides queueing delay | Pacing driven by RTT inflation; T0 "ping under load"; GATT path as alternative | T0 |
| A4 | Capacitor 9 lands mid-project | TS-only plugin + app-local bridges; migration touches two files | Phase 5 |
| A5 | iPhone-central → Android-peripheral throughput unknown (role asymmetry, uncontrollable connection interval) | Explicit [DEV] gate; Android not labelled supported until T0 iPhone↔Android passes; S_MAX may drop to 1 on such links | Phase 3 [DEV] |
| A6 | Play rejects `connectedDevice` for phone-to-phone | Fallback to `microphone` (+`location`) only | Phase 5 |
| A7 | Background TTS rendering not possible | Earcons remain the guaranteed cue; prompts pre-rendered in the foreground | T2 (Phase 2) |
| A8 | `location` anchor declined by riders | Audio anchor still works; UI explains the relaying trade-off; ride recorder measures how often relaying stops | Ride recorder |

## 16. Work breakdown for parallel coding agents

`docs/WORKPLAN.json` is the machine-readable source (owned paths, dependencies, `phase`, `wave`, per-phase `milestones`, interfaces, acceptance, verify commands, `needsDevice`). Every `verify` command runs from the repository root. Owned paths do not overlap and contain **no globs**: a path ending in `/` owns that subtree, and any other path is a single file. `phase` is the first phase in which the WP must deliver; `wave` is the WP's dependency depth (1 + the highest wave among `dependsOn`), so every WP of one wave can run in parallel once the previous waves are done. Multi-phase WPs list `milestones` (phase, scope, acceptance, extra dependencies needed only for that milestone); a WP is "done for phase N" when its milestones up to N pass. `softDependsOn` names WPs whose absence only drops an optional variant. This section explains the rules behind it.

### 16.1 Interface contracts (frozen per phase; changes go through the owner)

| ID | Contract | Owner | Artifact |
|---|---|---|---|
| C1 | UI command/event/snapshot JSON | WP-03 | `crates/mesh-api/src/lib.rs` + generated `packages/engine-api/src/generated/*.ts` |
| C2 | Wire formats and constants (§4, §9.3) | WP-04 | `crates/mesh-wire` public API + `test-vectors/wire/` + `docs/protocol/WIRE.md` |
| C3 | Sans-IO engine API (§3.2) | WP-10A | `crates/mesh-core/src/lib.rs` (signatures stubbed with `todo!()` by day 3) |
| C4 | UniFFI surface + `PlatformHooks` (§3.5) | WP-12 | `crates/mesh-ffi/src/api.rs` + committed generated Swift/Kotlin |
| C5 | Real-time C ABI (§3.6) | WP-12 | `crates/mesh-ffi/include/mesh_rt.h` |
| C6 | Capacitor plugin API (§11.1) | WP-17 | `packages/capacitor-intercom/src/definitions.ts` |
| C7 | `labrun/1` schema (§12.3) | WP-05 | `crates/mesh-lab/schema/labrun-1.schema.json` |
| C8 | Crate APIs: `mesh-crypto::{GroupIdentity, Invite, KeySchedule, LinkSession, ReplayWindow, Roster, Rekey}`, `mesh-voice::{Codec, Vad, TalkFsm, CaptureChain, JitterBuffer, Mixer, Earcons, PromptBank}`, `mesh-routing::{NeighborTable, Router, FloorTable, Pacer, AirtimeBudget, Reachability}`, `mesh-lab::{BenchRunner, Stats, ClockEstimator, LoopbackCorrelator}`, `opus-sys` shim | each crate's WP | `src/lib.rs` signatures published in week 1 of the phase |
| C9 | Platform link interface (§9.1), audio sink (`cmesh_set_sink`) and Live Activity attributes/intents | WP-14 | `native/ios/MeshPlatformKit/Sources/MeshPlatform/BLE/MeshLink.swift`, `Sources/CMeshAudio/include/cmesh_audio.h`, `Sources/MeshActivityShared/`; Kotlin mirror in WP-16 |

### 16.2 Rules for agents

1. **Exclusive ownership:** a WP edits only its `ownedPaths`. Changes elsewhere go through a contract change request to the owner.
2. **Stub hand-off:** a creating WP may write a compiling stub of a file or subtree owned by a later WP; ownership passes to that WP at creation. Stubs in this plan: WP-01 creates `tools/xtask/src/main.rs` with a dispatch table and one stub file per subcommand (`ts.rs`, `vectors.rs`, `opus.rs`, `ios.rs`, `android.rs`, `bindings.rs`, `wasm.rs`) that returns "not implemented (owned by WP-xx)" with exit 1, except `opus.rs`'s `fetch-opus`, which prints "nothing to fetch" and exits 0 so every verify can call it from day 1; WP-01 also creates every workspace crate as a stub (`Cargo.toml` with the §3.1 edges and features + empty `lib.rs`). WP-10A creates the `wire/<module>.rs` no-op registrations and empty module directories for WP-10B/C/D (§3.3). WP-18 creates the stub app-local plugin files (→ WP-19) and the stub `IntercomWidget/` bundle (→ WP-23).
3. **Generated files** are written only by their xtask command (`gen-ts`, `gen-bindings`, `vectors`), and CI checks drift.
4. **Determinism grep** and `clippy -D warnings` must pass; no `std::time`, threads, `getrandom` or HashMap iteration in sans-IO crates (lint denied per crate, §3.1).
5. **Verify prerequisites:** every verify that builds `mesh-core` (directly or through `mesh-sim`/`mesh-ffi`) starts with `cargo xtask fetch-opus`; every verify that runs an app package builds its workspace dependencies first (`pnpm --filter "@meshapp/mobile..." build`); Capacitor CLI calls use `pnpm exec cap …`, never `npx`; Kotlin JVM tests build the host library first (`cargo build -p mesh-ffi --release`).
6. **Changes outside owned paths** (e.g. WP-25's accessibility and privacy fixes in app files) are filed as contract change requests to the owning WP, which applies them.
7. **Physical honesty:** BITACORA entries separate automated checks from "pending physical — not faked".

### 16.3 Waves

Waves are computed from `dependsOn` (WORKPLAN `wave`), so every WP in a wave can start once all earlier waves are done; `P` is the WP's first delivery phase. Later-phase milestones of a WP (e.g. WP-09 Phase 2, WP-16 Phase 3) run when their phase starts and their milestone dependencies are met.

| Wave | Work packages (P = first phase) |
|---|---|
| 0 | WP-00 toolchain + MSRV probe (P0), WP-02 conventions (P0) |
| 1 | WP-01 scaffold, crate stubs, lint split (P0) |
| 2 | WP-03 API (P0 Lab subset), WP-04 wire (P0), WP-06 opus-sys (P0, off the critical path), WP-20 ui-kit (P0) |
| 3 | WP-05 mesh-lab (P0), WP-14 MeshPlatformKit (P0), WP-17 plugin TS (P0), WP-07 crypto (P1, may start early), WP-08 voice (P1), WP-09 routing (P1) |
| 4 | WP-10A core Lab mode (P0), WP-18 app shell (P0) |
| 5 | WP-11 sim (P0: S10), WP-12 FFI + builds (P0), WP-21 Field test UI (P0), WP-10B (P1), WP-10C (P1) |
| 6 | WP-15 MeshEngineKit (P0), WP-16 Android skeleton (P0), WP-13 WASM (P1), WP-10D mesh glue (P2) |
| 7 | WP-19 bridges + native app builds (P0), WP-22 intercom UI (P1), WP-24 Wi-Fi Aware (P4) |
| 8 | WP-23 Live Activity widget (P0 minimal, soft for WP-F0), **WP-F0 field campaign (P0)**, WP-F4 (P4) |
| 9 | WP-F1 (P1) |
| 10 | WP-F2 (P2), WP-F3 Android device validation (P3, deferred until a phone exists) |
| 11 | WP-25 store readiness (P5) |

### 16.4 Definition of done (every WP)

1. Owned-path code + tests; `cargo fmt`/`clippy -D warnings` or `pnpm lint` clean.
2. The WP's `verify` commands are green locally on a clean checkout (after `scripts/bootstrap.sh`), for the milestones of the current phase; `verifyNightly` commands (fuzz) fail loudly when run and are not part of the per-WP gate.
3. Contract artifacts regenerated with no drift.
4. A BITACORA entry that separates automated checks from physical validation.

## Appendix A — Constants

| Constant | Value | Section |
|---|---|---|
| Protocol version / UI API version / RT ABI version | 1 / 1.1 / 1 (16 kHz, wideband ceiling) | §4, §3.7, §3.6 |
| Max link frame | 1024 B (stream/GATT), 1200 B (datagram) | §4.2 |
| Mesh header / voice header | 9 B / 4 B | §4.5-4.6 |
| Records per frame | ≤ 32 | §4.4 |
| Noise pattern | `Noise_XXpsk1_25519_ChaChaPoly_SHA256` | §8.4 |
| Link replay window | 1024 bits per direction and key phase, updated after auth | §4.3 |
| Link key update | per direction, 12 h or 2³⁰ frames, in band; old key kept 10 s | §4.3 |
| SEC AAD | `lf_type ‖ kp_ctr` (5 B) | §4.3 |
| Token rotation | 600 s (accept ± 1 slot) | §8.3 |
| PRESENCE interval / member lost / alone | 2 s / 6 s / 10 s | §5.8 |
| POSITION interval | 5 s moving / 20 s stopped | §4.6 |
| NBR | on change (≤ 4 Hz) + every 1 s; fresh < 1.5 s | §5.1 |
| PING active / idle; link death | 500 ms / 2 s; 4 s | §5.1 |
| `Q_GOOD`; congested hold | 60; 2 s | §5.1 |
| Voice TTL / max age | 4 (policy 3-6) / 150 ms (policy 100-250) | §5.5 |
| Control TTL | 15 (TALK/REJECT 6, PROBE 8, PATH_HINT 3) | §5.5 |
| Dedup window | 1024 seqs per `(origin, sess)`, 5 s idle expiry | §5.3 |
| Voice queue per origin per link / backlog | 2 frames / 60 ms | §5.6 |
| Pacing initial (L2CAP/GATT/Aware/Tunnel) | 60 / 40 / 2000 / 64 kbps | §5.6 |
| Node BLE airtime budget / idle-link grant | 180 kbps [TUNE: T0c] / 8 kbps; only voice links are scaled | §5.6 |
| Cost model | `cost(k) = k × r_stream + 3 kbps`; r_stream BLE16 23.6, BLE12 ≈ 19.6, BLE-DRED 27.6, WIFI20 34.4 kbps | §4.8 |
| `cap_pacing` | max k with `cost(k) ≤ 0.9 × grant`; publish down after 2 s, up after 5 s | §5.6 |
| S_MAX BLE / Aware / Tunnel | 2 / 4 / 4 | §6.1 |
| T_TIE / T_STICKY | 150 ms / 300 ms | §6.1 |
| Floor idle release / max burst | 600 ms / 60 s PTT, 45 s VOX | §6.1 |
| VOX busy delay / denied holdoff | 600 ms / 1.5 s + rand(0-500 ms) | §6.4 |
| VAD hangover (single; no FSM hangover) / pre-roll | 300 ms / 40 ms | §7.3, §6.4 |
| Opus BLE16 / BLE12 / WIFI20 / WIFI24FEC | 16 kbps 40 ms / 12 kbps 40 ms / 20 kbps 20 ms / 24 kbps 20 ms + LBRR | §7.4 |
| `PACKET_LOSS_PERC` with LBRR off | 0 | §7.4 |
| JB init (BLE / Wi-Fi), min, max | 60 / 40 ms, frame + 20 ms (+ frame with LBRR), 200 ms | §7.5 |
| Mixer streams / limiter | 4 / −3 dBFS | §7.6 |
| Prompt rate / bank | ≤ 1 per 4 s / ≤ 512 KB | §7.7 |
| Audio rings (capture / render target) | 4096 samples / 320 samples | §3.6 |
| Engine tick / snapshot rate | 5 ms / ≤ 10 Hz | §3.4 |
| MuteWatchdog stall | 1.5 s | §10.1.4 |
| BLE links target / max; neighbors max | 2 / 3 [TUNE: T0c]; 6 | §5.7, §9.5 |
| Connection-limit backoff | max −1 for 10 min | §3.2, §9.5 |
| Anti-entropy push rate | 1 per (link, kind) per 10 s; JOIN_REQ re-flood 30 s | §8.9 |
| Live Activity renewal | on foreground after 7.5 h (system limit 8 h) | §10.1.3 |
| Connect / handshake timeout; backoff | 8 s / 5 s; 5 → 60 s | §9.5 |
| Android adv forming / stable | 250 ms / 1 s | §9.4 |
| Android scan restart / quota | 9 min / 5 starts per 30 s | §9.4 |
| Roster max | 32 entries (bitmap u32) | §4.7 |
| QR auto-hide | 60 s | §1.3 |
| Retained epochs / old-epoch acceptance | last 4 epochs; no time limit; only non-denied roster members not yet known to hold the newest epoch | §8.6 |
| IDPROOF resend / max / answer gap (datagram links) | 250 ms / 20 / 125 ms [TUNE] | §8.4 |
| Established-link death retry; iOS scan refresh | 3 s settle hold; 4 s | §9.5 |
| Pacer voice floor | one stream of the lowest profile: 25 112 bps BLE, 41 556 bps Aware/Tunnel | §5.6 |
| Speculative link to an ahead peer | epoch_lo 1-4 ahead; ≤ 1 per 30 s; 60 s backoff | §9.3 |

## Appendix B — How bench outcomes change the design

| Bench outcome | Design change |
|---|---|
| T0 with headset: < 2 voice-like streams at < 5 % loss | `S_MAX_BLE = 1` and BLE12 as the default profile; VOX busy logic unchanged |
| T0: ≥ 150 kbps sustained with headset | Enable BLE-fast (20 ms frames) by group policy; JB init 40 ms |
| T0: GATT beats L2CAP on some pair or role | Default `OpenLink` via GATT for that pair/role (core policy table) |
| T0: strong role asymmetry iPhone↔iPhone | Core prefers the faster role as initiator when both can initiate |
| T0b: added p95 per hop > 40 ms or relay node saturates | Voice TTL 3; JB init +10 ms; if the relay is airtime-bound, apply the T0c rule |
| T0c: hub sustains 3 links × cost(2) with HFP glitch-free | BLE links target 3 / max 4; `NODE_BLE_BUDGET` = measured node total × 0.9 |
| T0c: hub cannot sustain 3 links × cost(2) | Keep target 2 / max 3; set `NODE_BLE_BUDGET` so that `links_max × cost(2) ≤ 0.9 × budget` still holds, else `links_max` 2 (pure chains) |
| T6: battery > 5 %/h screen off | Lower PING/adv duty when Stable, LOW_POWER scans, prefer `audio` over `audioLocation` by default, document the cost; set the Phase 5 target from the measured baseline |
| T8: touch → RT flag p95 ≥ 50 ms | Native PTT overlay view above the WebView (v1.x) and Live Activity/notification latch as the recommended PTT for cars |
| T1: useful separation < typical convoy gap | Product positioning stays "compact group"; Aware (Phase 4) prioritized; mount guidance in onboarding; if far below, **no-go** for the motorcycle use case |
| T2: capture lost on helmet reconnect while locked | Ladder L2 is the expected path; onboarding asks riders to connect the helmet before Go live; location anchor recommended |
| T2: mixable session survives better than non-mixable | Switch the default after confirming 2.5.x behaviour |
| T2: TTS cannot render while locked | Prompts limited to pre-rendered ones; names for late joiners use earcons |
| T3: "both locked" never rediscovers | Lobby-formed links + pending reconnect + Live Activity are mandatory; Aware rejoin for iPhones; copy tells riders the locked-rejoin limit |
| T3: pending `connect()` reconnects reliably | `ArmReconnect` on every lost iPhone peer becomes the default |
| T5a: headset legs > 40 ms each | JB init −10 ms, `VOICE_MAX_AGE` 120 ms, voice TTL 3 to stay under KPI3 at 2 hops |
| T5b: helmet AEC/NREC fights VPIO, or the helmet's own intercom interferes | Per-model onboarding tip (disable helmet intercom / audio multitasking); Lab A/B of `BypassVoiceProcessing` per model in v1.x |
| VOX calibration: false triggers > 1/min | Default sensitivity Low for motorcycles; evaluate Silero (`vad-silero`) with the recorded clips |

## Appendix C — Decision log

| Decision | Choice | Rationale | Rejected alternatives |
|---|---|---|---|
| Base architecture | Proposal A, delivery-first vertical slices | Highest total review score; earliest real voice on iPhones; Lab on production code | B (protocol-first, voice only in Phase 2); C (Swift-only Phase 0 Lab, per-platform framing) |
| First field path | iPhone↔iPhone BLE L2CAP | Only iPhones available; BLE is the only radio for every platform pair | Wi-Fi Aware first (iOS-only); Multipeer/AWDL (deprecated in Xcode 27, Apple-only) |
| Phase 0 Lab code | Production Rust framing + production Swift drivers, LAB frames padded to SEC size | De-risks the real stack and the build pipeline; Android gets the benches for free | Swift-only throwaway probe engine with its own framing |
| Byte framing location | Rust only | No iPhone↔Android drift; fuzzing covers shipped code | Per-platform deframers in Swift/Kotlin |
| Link security | Per-link Noise XXpsk1 + IDPROOF | No persisted nonce state; hides all mesh metadata; per-link membership | End-to-end group AEAD with persisted sid (B); per-boot keys + header protection (C), kept as the rule for any future end-to-end format |
| psk position | psk1 | Strangers never see a static key | psk3 (leaks the responder static key), psk0 |
| Identity | Per-group Ed25519 + X25519, `node_id` from id_hash, deterministic collision regeneration | Unlinkable across groups; no leader round trip; no contradiction on collision | Device-wide identity (A); leader-assigned slots/ranges (B) |
| Invite secret | Current epoch secret, auto-hide, optional expiry, Lock/Remove → rekey in Phase 2 | A leaked QR must be revocable before public use | Permanent group secret with rekey deferred to hardening (A) |
| Dedup / replay | After AEAD only; windows per `(origin, sess)` with a random per-boot `sess` | A restart starts a fresh window; no "far behind" heuristic | Restart-by-distance heuristic (A); 4-bit boot tag before AEAD (C) |
| Routing | Managed flooding, immediate voice relay, safe 2-hop pruning | Convoy churn defeats proactive routing; chains need every relay | OLSR/MPR, gossip (dies in degree-2 chains), overhearing suppression (unicast BLE) |
| Pruning safety (rev 2) | **Voice never pruned**; ctrl/bulk pruned on fresh NBR (≤ 250 ms on change), good + uncongested links | Per-link, stateful admission means X cannot predict N's voice forwarding from its own floor table (review finding accepted); voice fan-out is already bounded by the BLE link max | vcap rank check for voice (rev 1, unsafe); NBR carrying admitted BurstKeys per link (correct but adds state and staleness for little gain); NBR every 1 s with 3 s freshness (A) |
| Talker-cap budget (rev 2) | Per-link 60 kbps budget, explicit uncoalesced cost model, `cap_pacing` with 0.9 factor and 2 s/5 s hysteresis, voice-aware node budget 180 kbps, BLE links 2 / 3 until T0c | Rev 1's proportional 150 kbps node scaling made cap = 1 at the link target, breaking L2 (review blocker accepted); the research budgets per link and charges the relay `r × degree` | Coalesced 42.6 kbps as the design cost (relays do not wait to coalesce); keeping 3 / 4 links unmeasured |
| Link AEAD (rev 2) | Raw split keys + `chacha20poly1305` with AAD = `lf_type ‖ kp_ctr`; in-band key update with a phase bit | snow's `StatelessTransportState` has no AD parameter (review finding verified); the phase bit also replaces the re-handshake that conflicted with duplicate-link resolution | Drop the AAD and keep `StatelessTransportState` (works, but loses the phase/type binding we now need); 12 h re-handshake on a second link |
| Control-state consistency (rev 2) | Link-scoped anti-entropy (SYNC records) + monotonic acceptance + JOIN_REQ retransmission | Flood-once loses state across partitions; replays could roll back a deny entry | Periodic full re-floods (airtime); CRDT roster (v1.x scope) |
| Rekey access (rev 2) | Retain 4 epochs; accept old-epoch handshakes from non-denied roster members without a time limit until they hold the newest epoch | Groups persist and are reused weekly; a 24 h grace silently locked out absent members (review finding accepted). Lock's goal (refuse old-QR strangers and removed members) is met by the roster/deny check | 24 h grace (rev 1); unlimited retention of all epochs |
| Capture start ownership (rev 2) | Bridge does gesture-bound OS work synchronously inside `gestureCommand`; core `StartCapture` only connects/unmutes the sink | Removes double-start and late-command races; one gating rule covers Go live, Resume mic, Lab audio and the audio check | Core-driven session activation via async `AudioCommand` |
| Floor timing (rev 2) | VOICE `ts` and TALK `elapsed_10ms` are burst-relative | Late joiners rank a burst exactly like everyone else | Origin media clock (rev 1: undefined offset for late joiners) |
| Hop promise (rev 2) | KPI3 for ≤ 2 BLE hops; 3-4 hops best effort, labelled | The design's own 3-hop typical total is 293 ms, so p95 cannot meet 300 ms there | "Voice reaches ≤ 4 hops" (rev 1) |
| Demo voices (rev 2) | Runtime platform TTS of localized lines, encoded by the bots; no bundled audio | No licensing question, automatic ES/EN, no asset ownership gap | Bundled TTS or recorded clips (licensing and ownership unclear) |
| Leader loss (rev 2) | Signed transfer-leadership while present; "new group" recovery when lost | Simple, no secret backup UX | Exportable leader-key backup (secret handling in JS/UI); "any member may Lock" (lets any member evict others) |
| Insider spoofing (rev 2) | Sign LEAVE, RIDE_CTL (+ leader-signed ROSTER/REKEY/GROUP_CTL); members re-share the leader-signed invite verbatim; native QR rendering | Cheap (rare messages, 64 B) and closes the silence-the-leader and fake-leader-QR attacks | Signing FLOOR_REJECT{preempted} (sent by relays, a relay's signature proves nothing about the leader burst); per-burst voice signatures (v1.x) |
| RT sample rate (rev 2) | 16 kHz fixed, v1 wideband ceiling, SWB bit removed | Simplest real-time ABI; SWB needs a negotiated ABI version | Runtime 16/32 kHz negotiation in v1 |
| Floor | Optimistic, total rank (prio, age-estimated start, hash smear), stickiness, hard per-link caps | KPI1 without grant round trips; no wall clock offline | Grant/request floor; wall-clock `start_utc_ms` ranking (C) |
| Codec | libopus 1.6.1 float, BLE 16 kbps 40 ms no FEC, loss hint 0; Wi-Fi LBRR only at 24 kbps ≥ 3 % | Verified FEC thresholds; the hint lowers bandwidth when LBRR is off | Loss hint up to 10-15 % with LBRR off (B, C); DTX inside bursts (C); LBRR+DRED together |
| libopus build | `cc` crate + fixed-arity shim, SHA-pinned sources and weights | No system cmake here; avoids variadic FFI on Apple arm64 | opusic-sys + cmake (kept as fallback) |
| VAD | nnnoiseless probability + noise floor + wind guard, Silero behind a feature | Pure Rust, WASM-friendly, deterministic | Silero via ONNX Runtime now; Opus DTX as the gate |
| iOS audio I/O | Raw VPIO AudioUnit, never stopped mid-ride | Survives route changes; AVAudioEngine restarts fail from the background | AVAudioEngine stop/start ladder (B) |
| iOS keep-alive | audio + location (opt-in distance sharing) | Relaying survives calls and capture loss; location justified by a visible feature | Audio only (A); location always on by default (B, C); voip/CallKit; PushToTalk (needs APNs) |
| Android audio | FGS microphone\|connectedDevice (+location when granted) from visible taps, MODE_IN_COMMUNICATION, Oboe | Locked decision 5; validatable later without Telecom policy risk | Core-Telecom default + phoneCall FGS + MEDIA_BUTTON receiver (C) |
| Talk mode defaults | VOX for motorcycles, PTT for cars (VOX only with HFP + AEC check) | Loudspeaker VOX is the worst AEC case | VOX everywhere (C) |
| Spoken prompts | Earcons guaranteed; prompts pre-rendered in the foreground | Background TTS rendering unverified | Tones only (A); TTS as the sole cue |
| Packaging | MeshPlatformKit (no Rust) + MeshEngineKit; app-local bridges; TS-only plugin | SwiftPM never resolves a missing xcframework; no SPM path through pnpm symlinks; small Capacitor 9 migration | Plugin `Package.swift` with relative or `#filePath` path deps (A, B) |
| Bindings | Committed generated Swift/Kotlin + drift check; `Vec<u8>` link data | Platform agents compile without cross-building; avoids the 0.32 Kotlin ByteBuffer break | Gitignored bindings (B); `&[u8]` signatures (B) |
| WASM | Raw `extern "C"` JSON ABI, off the Phase 0 critical path | Builds here with only the rustup target | wasm-bindgen-cli global install (A); wasm-pack |
| Lab exposure | Visible Settings → Field test section | App Review 2.3.1; honesty | Hidden 7-tap gesture (B) |
| Group lifetime | Until the user forgets or the leader closes | Weekly club rides reuse groups | 7-day auto-expiry (C) |
| Review findings partly adopted (rev 2) | (1) C1 goes to 1.1 now rather than "before Phase 1": nothing was frozen yet, so Phase 0 freezes 1.1. (2) Native QR: the image is rendered natively, but a full native QR view was not built, because the image itself encodes the secret and the WebView loads only bundled content. (3) FLOOR_REJECT is not signed (see "Insider spoofing"). (4) WP-18 keeps owning the native projects; native app *builds* are verified by WP-19 instead of adding a compile flag to the project. | — | — |
| Scope | No co-leaders, CRDT roster, member policies, Control widgets, Telecom default in v1 | Fastest path to riders talking | B and C v1 extras |
| Min platforms | iOS 26.0 (iPhone 12+); Android minSdk 31, target 36 | Test devices; Aware/Live Activity BT privilege need iOS 26; API 31 avoids background location for BLE | iOS 17/18 support; target 37 now (ACCESS_LOCAL_NETWORK, WIU rules deferred) |
| Pacer voice floor (2026-10-24) | Decrease stops at one stream of the lowest profile (BLE 25 112 bps); BLE12 counts as cap 1; lone burst never shed; stale reject cleared; weakSignal banner | Field test: a lone talker at the edge got "Network busy"; fading is not congestion | Unbounded decrease (cap 0 at the fringe) |
| Range at the edge (2026-10-24) | Android LE 1M PHY on client+server; iOS scan refresh 4 s; 3 s hold after an established link dies; pacer recovery without `qd < 15 ms`, `min(srtt, sample)`; mid-burst BLE16→BLE12; LBRR on BLE16 at ≥ 3 % loss (off after 10 s < 2 %) | More sensitivity and faster rejoin at the fringe; the 30 ms iOS interval kept `srtt − min_rtt` above 15 ms forever | 2M PHY; 5→60 s backoff after walk-away; LBRR only on WIFI24FEC |
| Open channels (2026-10-24) | Channels 1-9, public HKDF key, leaderless restricted Group, fresh identity per session, cap 2, TTL 2, local mute/block; C1 1.2 | User request; reuses the whole stack | Deferred to v1.x (rev 2 scope table) |
| Hybrid Aware backbone + BLE edge (2026-10-24) | Per-segment near-talker cap (4 backbone, 2 BLE); edge links admit by rank without `T_STICKY`, shed locally without FLOOR_REJECT (also on downgrade); cap_local ignores edge and BLE-fallback links; BLE16 when any voice-egress link (incl. fallback) is BLE | Relays never transcode; first-come on edges inverted ranks; idle fallback wrongly bounded cap_local (S16, S16b, S16c) | BLE profiles only when a BLE link carries my voice (Android groups with fallback now stay on BLE16/BLE12; design owner accepts) |
| IDPROOF on datagram links (2026-10-24) | Resend every 250 ms (≤ 20) in IdProof; Up link answers a valid repeat of the admitted identity (≤ 1 per 125 ms, ≤ 20, until CAPS); early SYNC/MESH records buffered (8 / 4 KiB) and replayed after admission; CAPS re-runs link-up announcements | UDP loses/reorders IDPROOF past msg3, leaving one end in IdProof | Send once; buffer early SEC ciphertext in the link layer |
| Sim scenario parameters (2026-10-24) | S16b BLE edge at 64 kbps; S16c star with hub A1 as creator and a second `FORM_LIMIT_MS` window | Exactly 60 kbps leaves no pacer headroom for cap 2; A0-led star formation took ~60.25 s | 60 kbps edge; A0 leader |

## Appendix D — Review must-fix traceability

| Must-fix | Resolved in |
|---|---|
| iOS location keep-alive during rides; Android location FGS type only when granted | §10.1.3, §10.2.3, D8 |
| Android legacy advertising must be connectable + scannable `ADV_IND` | §9.4 |
| Replay/dedup only after AEAD; windows keyed by full sender session; fresh window on new session | §4.3, §4.5, §5.3 |
| Opus on BLE: loss hint 0 with LBRR off, DTX off, LBRR only at 24 kbps ≥ 3 %, no DRED+LBRR, DRED gated by group capability | §7.4 |
| No `ACCESS_LOCAL_NETWORK` at target 36 | §10.2.4, static contract test §13.1 |
| UniFFI 0.32: `Vec<u8>` link data, pin `=0.32.2` | §3.5, §2.6 |
| No SPM path through pnpm symlinks; no blocking missing binaryTarget | §2.4 (MeshPlatformKit/MeshEngineKit split, app-local bridges) |
| BLE link target + per-node airtime budget validated in the field (rev 2: target 2 / max 3 and a voice-aware budget, validated by T0c) | §5.6, §9.5, §12.2 |
| Role rules use only pre-connection information; iPhone central toward Android; same-OS duplicates resolved after handshake | §9.2 |
| Pruning: on-change neighbor tables, good links only, never skip when upstream did not forward; property tests + stale simulation (rev 2: voice never pruned) | §5.1, §5.4, §13.1, S11 |
| Android audio default = FGS from visible tap + MODE_IN_COMMUNICATION + Oboe; Telecom behind a flag; no default MEDIA_BUTTON receiver | §10.2.2, §10.2.3 |
| iOS audio: raw VPIO never stopped; background restart failure → CaptureLost + output-only fallback | §10.1.2, §10.1.4, §7.8 |
| Lab identity collisions impossible; `(key, nonce)` uniqueness asserted in simulation | §8.2, §12.1, §13.2 |
| Byte framing only in Rust | §3.2, §9.1 |
| libopus cross-build risk retired in Phase 0 without system cmake | §2.3, §14 Phase 0 |
| Phase 0 self-consistent (T0b has the minimal Lab flood; the T3 Live Activity variant has a minimal widget or moves) | §12.1, §14 Phase 0 |
| WASM and full product UI off the Phase 0 critical path | §11.2, §14 |
| WASM builds without global installs | §2.3, §11.2 |
| Floor ordering without wall clocks; hash tie-break; relays enforce caps | §6.2, §6.3 |
| Collision-safe identity without contradiction | §8.2 (per-group identity) |
| iPhone↔iPhone T0 does not retire the iPhone-central→Android risk; Android not "supported" until it passes | §9.2, §14 Phase 3, A5 |
| Runnable verification: `pick-sim.sh`, cargo-ndk present + NDK fallback, committed bindings | §2.2, §2.3, §2.5 |
| No B scope extras before 1-hop voice; no persisted counters | §1.2, §8.4 |
| Location mode only with a user-visible distance feature; opt-in; disclosed | §1.1, §1.4, §10.1.3, §10.1.5 |
| Car profile not VOX on a loudspeaker | §1.4 |
| Invite carries the current epoch secret; Lock/Remove → rekey in v1; QR auto-hide and warning; no secrets in JS storage | §8.3, §8.6, §1.3 |
| No 7-day group auto-delete | §1.1 |
| Floor without phone clocks | §6.2 |
| Earcon for every audible state; prompts through the mixer; background TTS verified in T2 first | §6.5, §7.7, §12.2 |
| Lab visible, not hidden; per-run GPS consent; files stay local until shared | §12.1, §12.5, §8.8 |
| Mic only after explicit Go live; mic state always visible incl. muted lobby | §1.3 F3, §10.1.5, §11.4 |
| Mesh header metadata never on air in clear | §4.5, §8.1 |
| No persist-before-use counters | §8.1, §8.4 |
| Honest positioning, no trademarks, safety note | §0.2 |
| "Don't swipe-kill", Resume ride after process death, battery exemption after first ride | §1.3 F0/F7, §10.2.3 |
| Trim v1 scope | §1.2, §14 |

### D.2 Adversarial review (revision 2) traceability

| # | Finding (severity) | Resolution | Where |
|---|---|---|---|
| 1 | BLE cap = 1 at the link target (blocker) | Explicit uncoalesced cost model, `cap_pacing` with 0.9 factor + 2 s/5 s hysteresis, cap below S_MAX no longer "congested", voice-aware 180 kbps node budget (idle links 8 kbps), BLE links 2 / 3 until T0c, steady-state property test, sim node limit = `NODE_BLE_BUDGET` | §4.8, §5.1, §5.6, §5.7, §9.5, §12.2 T0c, §13.1, §13.2, App. A/B/C |
| 2 | Lab benches cannot capture or stay alive (blocker) | `LabParams.audio` and `.anchor` mandatory; Lab audio via `gestureCommand`; gating rule and property test include `labStart{audio: full}`; anchors recorded in labrun/1; T1/T3 default to `audio` | §3.2, §3.7, §12.1-12.3, §12.5, S10 |
| 3 | AAD impossible with `StatelessTransportState` (major) | Raw split + `chacha20poly1305` with AAD; vectors regenerated | §4.3, §8.4, §4.10, WP-07 |
| 4 | Two-hop pruning unsafe for voice (major) | Voice never pruned; ctrl/bulk only; contention property test | §5.2, §5.4, D4, App. C |
| 5 | Control state never resynced; no monotonicity (major) | Anti-entropy (CAPS summary, SYNC records, PRESENCE mismatch), JOIN_REQ retransmission, normative monotonic acceptance, S13 | §4.4, §4.7, §5.8, §8.9, §9.3, §13.2 |
| 6 | Late nodes cannot rank bursts (major) | Burst-relative VOICE `ts`; TALK `elapsed_10ms` (7 B); mid-burst floor test | §4.6, §6.2, §7.5, §13.1, S5 |
| 7 | opus-sys breaks clean builds; wasm feature unification (major) | `fetch-opus` first everywhere (stub exits 0 before WP-06); `mesh-core` default feature `opus`, `mesh-voice/opus` optional; wasm/sim `default-features = false`; crate stubs from WP-01 | §2.3, §2.5, §3.1, §16.2, WORKPLAN |
| 8 | Workspace-wide disallowed-* lints fail CI (major) | Workspace `allow`, `#![deny]` only in the seven sans-IO crates; WP-01 acceptance check | §3.1, WP-01 |
| 9 | C1 lacks audio check / echo / VOX meter (major) | `audioCheck` + `audioCheckSample`; API 1.1 | §1.3 F0, §3.7, §7.3, §11.4 |
| 10 | 24 h rekey grace locks out absent members (major) | Retained epochs, no time limit until the member holds the newest epoch; `previewAccessLoss` before lock/remove | §8.6, §1.3 F9, App. A |
| 11 | No locked-screen PTT; idle timer (major) | `LatchTalkIntent` (RT bit3), Android notification latch, `setKeepScreenOn` for lobby/PTT ride, car "keep screen on while mounted" | §1.1, §1.3 F3/F4, §1.4, §3.6, §10.1.3, §10.1.5, §11.1 |
| 12 | WORKPLAN deps vs owned paths (major) | MeshActivityShared → WP-14; WP-23 widget only; stub hand-offs from WP-18; native app builds verified by WP-19; WP-14 depends on WP-04 | §2.4, §16.1-16.2, WORKPLAN |
| 13 | mesh-core sub-module ownership (major) | All deps declared by WP-10A from crate stubs; `wire/<module>.rs` per sub-WP; `secure/` owned by WP-10B; tests by directory | §3.3, WORKPLAN |
| 14 | Latency claims vs budget (major) | KPI3 for ≤ 2 BLE hops; 3-4 best effort; S2/S4 thresholds include the 50 ms headset budget | §1.1, §5.5, §7.9, §3.7 snapshot, S2/S4 |
| 15 | Re-handshake vs duplicate links (minor) | In-band key update with phase bit | §4.3, §5.7, §8.4 |
| 16 | Epoch transition discovery (minor) | Tags/tokens of all retained epochs; speculative link to an ahead peer; S12 cases | §8.3, §9.3, S12 |
| 17 | Insider forgery, invite authenticity (minor) | Signed LEAVE/RIDE_CTL; `invite_sig` in invite and ROSTER; leader-key cross-check; native QR image; custom-scheme risk documented | §4.6, §4.7, §8.1, §8.3, §11.1 |
| 18 | Wire field inconsistencies (minor) | One width per concept with truncation rules; names in bytes, grapheme truncation; 14-bit prefix + tie rule; TALK profile 5 | §4.6.1, §4.7, §9.2 |
| 19 | Capture start ownership; F3 signature (minor) | `gestureCommand`; bridge does OS work, core only connects/unmutes; F3 fixed | §3.2, §11.1, §1.3 F3 |
| 20 | WASM getrandom; wrong cross-reference (minor) | snow without defaults + custom resolver, dalek without rand features, target-specific `getrandom/wasm_js` guard + tree check; §11.2 reference | §2.3, §2.6, §8.4 |
| 21 | Failure handling gaps (minor) | `RadioState`, `LinkDownReason::{RadioOff, LimitReached}`, cue, PRESENCE bit, link-max backoff, Live Activity 8 h renewal, single 300 ms hangover | §3.2, §4.6.1, §6.4, §6.5, §9.5, §10.1.3 |
| 22 | Wi-Fi Aware claims (minor) | Aware rejoin needs the anchors; riding-order chain or star pairing; T4 battery variant | §9.5, §9.6, §12.2 |
| 23 | Toolchain pin (minor) | `scripts/msrv-probe.sh` in WP-00, pin moves to current stable if needed; doctor reports MSRVs; explicit toolchain install | §2.2, §2.6, WP-00 |
| 24 | RT sample rate vs SWB (minor) | WB ceiling in v1, SWB bit removed, RT ABI version macro | §3.6, §4.4, §4.6, §7.2 |
| M1-M12 | Missing items: T0c, T6, T8, Lab anchors, anti-entropy, audio-check commands, locked PTT + idle timer, radio-off/limit handling, leader loss, Demo clips, T3 Android variant, T5b | Added | §12.2, §8.9, §3.7, §10.1.5, §3.2, §8.6, §3.4, §14 |
| W1-W19 | Work-plan issues | `phase`, `wave`, `milestones`, `softDependsOn`, `verifyNightly` fields; recomputed waves; dependency and ownership fixes; verify prerequisites | §16, WORKPLAN.json |

## Appendix E — Open questions for the user (none block Phase 0)

1. Typical convoy gap the product must cover. It sets the T1 go threshold.
2. Product name, bundle id and URL scheme (working: "convLink Voice", `com.convlink.voice`, `convlinkvoice`) and the brand accent colour.
3. Confirm making `mesh-app` its own git repository (sibling of `comvoy-app`). WP-01 asks before `git init`.
4. Helmet models available for T5a and T0 (Cardo Packtalk, Sena 50S/60S, others).
5. Spanish locale flavour (`es` or `es-DO` as in convLink).
6. When an Android phone becomes available, which model (prefer one Samsung and one Pixel for the OEM checklist).
7. How many iPhones can join one Phase 0 session? T0c's 3-link hub needs 4; with 3 the BLE link target stays 2 / max 3.
8. Leader loss: is "start a new group" an acceptable recovery, or should a v1.x leader-key backup be planned?
9. The lines the Demo-ride bots speak (ES/EN): the defaults are placeholders in the `prompts` catalog.
