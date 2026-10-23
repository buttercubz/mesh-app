# Swift conventions

Applies to `native/ios/MeshPlatformKit`, `native/ios/MeshEngineKit`, the app-local
bridge in `apps/mobile/ios/App/App/Plugins/` and the `IntercomWidget` extension.
Design references: DESIGN §2.4, §10.1, §11.1, §13.1.

## Toolchain and packages

- Xcode 27.1, Swift 6.4 toolchain, `swift-tools-version: 6.0`, Swift 6 language mode
  with complete strict concurrency. Deployment target iOS 26.0 (iPhone 12+).
- `MeshPlatformKit` has **no Rust dependency** and no binary target: it must resolve
  and build before any Rust artifact exists. Targets `CMeshAudio` (C), `MeshActivityShared`,
  `MeshPlatform`.
- `MeshEngineKit` holds the `MeshCoreFFI` binary target (gitignored xcframework from
  `cargo xtask build-ios`), the committed generated `MeshCoreBindings` (Swift 5
  language mode, never hand-edited) and `EngineHost`. It depends on
  `.package(path: "../MeshPlatformKit")` — an in-repo relative path, never through
  `node_modules`.
- SPM only; no CocoaPods.

## Concurrency and threading

- UI-facing types are `@MainActor`. CoreBluetooth runs on a dedicated serial queue;
  L2CAP streams run on a `LinkIOThread` run loop. Never block those queues on engine
  calls; engine calls are non-blocking by contract (`link_send` returns
  `Accepted | WouldBlock | NoLink`).
- Mark cross-thread value types `Sendable`; use `@unchecked Sendable` only with a
  comment naming the lock or queue that protects the state.
- Real-time audio lives in C (`CMeshAudio`): VPIO callbacks call the registered
  function-pointer sink (`cmesh_set_sink`) and touch only atomics and preallocated
  buffers. No Swift, no allocation, no locks, no logging, no ObjC messaging on the RT
  thread.

## Platform rules that must not drift

- Byte framing is in Rust: Swift moves opaque `Data`/`[UInt8]` only.
- Audio session exactly as DESIGN §10.1.2 (`.playAndRecord`, `.voiceChat`,
  `.allowBluetoothHFP` [+ `.defaultToSpeaker` for cars], never `.mixWithOthers` by
  default, 16 kHz / 10 ms preferences). VPIO is never stopped during a ride.
- OS-level, gesture-bound work (activate session, start VPIO muted, location anchor,
  Live Activity) happens synchronously inside the bridge's `gestureCommand` handler,
  which first checks `UIApplication.shared.applicationState == .active`.
- New-OS APIs behind `if #available(iOS 27, *)` with the iOS 26 path kept working.
- Secrets only in the Keychain (`SecureStore`,
  `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`). Never in `UserDefaults`, files
  or anything the WebView can read.
- Background modes are exactly `audio`, `bluetooth-central`, `bluetooth-peripheral`,
  `location`. No `voip`, no `push-to-talk` (static contract tests enforce this).
- Lab files under `Documents/lab/` (visible in Files), exported only via the share
  sheet.

## Style

- Swift API Design Guidelines; types `UpperCamelCase`, members `lowerCamelCase`.
- No force unwraps (`!`) or `try!` outside tests; handle every `throws` and log the
  error through the engine log hook or `os.Logger` (subsystem `com.convlink.voice`).
- Protocols in front of every OS service (`CBCentralManager`, `AVAudioSession`,
  `ActivityKit`, `CLLocationManager`) so logic is testable with fakes.
- One primary type per file; files stay focused (~300 lines).
- User-facing strings come from the web UI's i18n, or `InfoPlist.strings`
  (`en.lproj`, `es.lproj`) for system prompts. No hard-coded English in UI code.

## Tests

- From the repository root:
  `(cd native/ios/MeshPlatformKit && xcodebuild -scheme MeshPlatformKit-Package -destination "id=$(../../../scripts/pick-sim.sh)" test)`
  (same form for `MeshEngineKit`). Never hard-code a simulator name.
- CoreBluetooth does not work in the simulator: BLE drivers are tested behind their
  protocols with fakes (adv/HELLO vector replay, LinkRegistry, session-config logic,
  watchdog FSM, SecureStore, LabFileStore).
- Anything that needs radios, HFP, background or a lock screen is "pending physical —
  not faked" in the BITACORA entry.
