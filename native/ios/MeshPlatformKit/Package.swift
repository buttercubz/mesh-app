// swift-tools-version: 6.0
// MeshPlatformKit: the Rust-free iOS platform layer (DESIGN §10.1.1, WP-14).
//
// It must resolve, build and test before any Rust artifact exists: there is no binary
// target here. The Rust bridge (MeshEngineKit, WP-15) depends on this package through
// `.package(path: "../MeshPlatformKit")` and registers the `mesh_rt_*` symbols into
// `cmesh_set_sink` at runtime (DESIGN §2.4).
import PackageDescription

let package = Package(
    name: "MeshPlatformKit",
    defaultLocalization: "en",
    platforms: [.iOS("26.0")],
    products: [
        .library(name: "MeshPlatform", targets: ["MeshPlatform"]),
        .library(name: "MeshActivityShared", targets: ["MeshActivityShared"]),
    ],
    targets: [
        // Real-time audio host in C: VPIO AudioUnit, RT callbacks, function-pointer sink,
        // built-in sinks (meter, loopback, tone, T5a click/capture). No Swift on RT threads.
        .target(
            name: "CMeshAudio",
            path: "Sources/CMeshAudio",
            linkerSettings: [
                .linkedFramework("AudioToolbox"),
                .linkedFramework("CoreAudio"),
            ]
        ),
        // Live Activity attributes and intents shared by the app and the widget (contract C9).
        .target(
            name: "MeshActivityShared",
            path: "Sources/MeshActivityShared",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .target(
            name: "MeshPlatform",
            dependencies: ["CMeshAudio", "MeshActivityShared"],
            path: "Sources/MeshPlatform",
            swiftSettings: [.swiftLanguageMode(.v6)],
            linkerSettings: [
                .linkedFramework("CoreBluetooth"),
                .linkedFramework("AVFAudio"),
                .linkedFramework("CoreLocation"),
                .linkedFramework("ActivityKit"),
                .linkedFramework("UserNotifications"),
                .linkedFramework("Security"),
                .linkedFramework("VisionKit"),
                .linkedFramework("CoreImage"),
            ]
        ),
        .testTarget(
            name: "CMeshAudioTests",
            dependencies: ["CMeshAudio"],
            path: "Tests/CMeshAudioTests",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "MeshActivitySharedTests",
            dependencies: ["MeshActivityShared"],
            path: "Tests/MeshActivitySharedTests",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "MeshPlatformTests",
            dependencies: ["MeshPlatform", "CMeshAudio", "MeshActivityShared"],
            path: "Tests/MeshPlatformTests",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
