# Kotlin conventions

Applies to `native/android/` (the `:meshengine` library, its C++ Oboe shim) and the
app-local Android bridge in
`apps/mobile/android/app/src/main/java/com/convlink/voice/plugin/`. Design
references: DESIGN §2.4, §10.2, §13.1.

## Toolchain and build

- AGP 8.13.2, Gradle 8.14.3 (wrapper committed), Kotlin 2.3.x, JDK 21
  (`JAVA_HOME=$(/usr/libexec/java_home -v 21)`, exported by `scripts/env.sh`),
  compileSdk 36, targetSdk 36, minSdk 31, NDK `27.1.12297006`, CMake 3.22.1 from the
  SDK (never system cmake), Oboe prefab `com.google.oboe:oboe:1.9.x` (exact pin), JNA
  `net.java.dev.jna:jna:5.17.0@aar`, kotlinx-coroutines. All versions pinned exactly.
- `native/android` is a standalone Gradle build; the app consumes it through
  `includeBuild("../../../native/android")` and the coordinate
  `com.convlink.voice:meshengine`.
- `kotlin.code.style=official`; `./gradlew :meshengine:lintDebug` must be clean (no
  new baseline entries without a comment explaining why).
- Telecom code lives only in `src/telecom/` and is compiled only with
  `-Pmeshengine.telecom=true`, which also merges its manifest overlay. The default
  build has no `MANAGE_OWN_CALLS`, no `phoneCall` FGS type, no `MEDIA_BUTTON` receiver.

## Packages

- `com.convlink.voice.engine.*` — hand-written library code (`ble/`, `audio/`,
  `sensors/`, `keys/`, `notify/`, `radio/`, `lab/`, `qr/`, `aware/`).
- `com.convlink.voice.core` — **generated** UniFFI bindings (`mesh_ffi.kt`), committed
  and written only by `cargo xtask gen-bindings`. Never edit.
- `com.convlink.voice.plugin` — the app-local Capacitor bridge.
- R8 keeps: `com.sun.jna.**`, `com.convlink.voice.core.**`, `com.convlink.voice.engine.**`.

## Threading and audio

- Byte framing is in Rust: Kotlin moves opaque `ByteArray`s only (`Vec<u8>` in the
  bindings).
- One reader and one writer thread per L2CAP socket; BLE callbacks never block and
  hand data to the engine (`on_link_data`), which queues internally.
- Coroutines for lifecycle and UI glue (structured, scoped to the service); no
  `GlobalScope`.
- Real-time audio is the C++ Oboe shim: callbacks call `mesh_rt_capture` /
  `mesh_rt_render` resolved by `dlsym` (tone/loopback fallback); no JNI calls, no
  allocation, no locks, no logging on the audio thread.

## Platform rules that must not drift

- FGS started from the visible tap: `connectedDevice` for the lobby, then
  `microphone | connectedDevice` at Go live, plus `location` **only** when fine
  location is granted and distance sharing is on. Compute the type set from current
  grants on every `startForeground` (a location type without the permission throws on
  Android 14+). `START_NOT_STICKY`.
- Audio: `MODE_IN_COMMUNICATION`, `setCommunicationDevice` preference order of
  DESIGN §10.2.2, `clearCommunicationDevice()` on exit.
- `BLUETOOTH_SCAN` and `NEARBY_WIFI_DEVICES` with `neverForLocation`. **No
  `ACCESS_LOCAL_NETWORK` at target 36.**
- Secrets only through `SecureStore` (Keystore-wrapped AES-GCM, excluded from backup
  and data extraction). Never in `SharedPreferences` in clear or anything the WebView
  can read.
- Gesture-bound work (FGS start, Oboe open) happens synchronously in the bridge's
  `gestureCommand` handler after checking the activity is resumed.

## Style

- Kotlin official style; `val` over `var`; no `!!` outside tests; sealed classes for
  state machines; explicit visibility for public API.
- Never swallow exceptions silently: every `catch` logs or rethrows, and an
  intentionally ignored exception carries a comment saying why.
- Wrap OS services (`BluetoothLeScanner`, `AudioManager`, `NotificationManager`,
  `LocationManager`) behind interfaces so logic is JVM-testable with fakes.
- No hard-coded user-facing English: notification text comes from `res/values` and
  `res/values-es` resources.

## Tests

- JVM unit tests: `(cd native/android && ./gradlew :meshengine:testDebugUnitTest)`.
  Binding tests load the host `libmesh_ffi.dylib` through `jna.library.path`, so run
  `cargo build -p mesh-ffi --release` first.
- Covered on the JVM: FGS type computation from grants, scan/advertise policy (quota,
  9-minute restart, legacy connectable + scannable parameters), notification builder,
  permission matrix, route preference.
- No Android device exists yet: every radio, audio-route, OEM and background behaviour
  is "pending physical — not faked" until the Phase 3 device gate (WP-F3).
