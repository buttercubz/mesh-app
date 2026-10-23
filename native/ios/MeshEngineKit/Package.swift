// swift-tools-version: 6.0
// MeshEngineKit: the iOS bridge between the Rust engine and the platform layer
// (DESIGN §2.4, §10.1.1, WP-15).
//
// - `MeshCoreFFI`: the Rust static library + C headers (`mesh_ffiFFI.h`, `mesh_rt.h`),
//   built by `cargo xtask build-ios` into `Frameworks/` (gitignored; SwiftPM needs local
//   binary targets under the package root).
// - `MeshCoreBindings`: the UniFFI-generated Swift (committed, owned by WP-12), compiled
//   in Swift 5 language mode.
// - `MeshEngineKit`: `EngineHost` and its adapters, Swift 6 mode, on top of the Rust-free
//   `MeshPlatformKit` (in-repo relative path, never through node_modules).
import PackageDescription

let package = Package(
    name: "MeshEngineKit",
    platforms: [.iOS("26.0")],
    products: [
        // The App project (WP-18/WP-19) links this product by this exact name.
        .library(name: "MeshEngineKit", targets: ["MeshEngineKit"]),
        // The generated UniFFI API, for bridge code that needs `apiVersion()` & co. directly.
        .library(name: "MeshCoreBindings", targets: ["MeshCoreBindings"]),
    ],
    dependencies: [
        .package(path: "../MeshPlatformKit"),
    ],
    targets: [
        .binaryTarget(
            name: "MeshCoreFFI",
            path: "Frameworks/MeshCoreFFI.xcframework"
        ),
        .target(
            name: "MeshCoreBindings",
            dependencies: ["MeshCoreFFI"],
            path: "Sources/MeshCoreBindings",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .target(
            name: "MeshEngineKit",
            dependencies: [
                "MeshCoreFFI",
                "MeshCoreBindings",
                .product(name: "MeshPlatform", package: "MeshPlatformKit"),
                .product(name: "MeshActivityShared", package: "MeshPlatformKit"),
            ],
            path: "Sources/MeshEngineKit",
            swiftSettings: [.swiftLanguageMode(.v6)],
            linkerSettings: [
                // libmesh_ffi.a (Rust std + libopus) needs only libSystem plus these.
                .linkedLibrary("c++"),
                .linkedFramework("Security"),
            ]
        ),
        .testTarget(
            name: "MeshEngineKitTests",
            dependencies: [
                "MeshEngineKit",
                "MeshCoreBindings",
                .product(name: "MeshPlatform", package: "MeshPlatformKit"),
                .product(name: "MeshActivityShared", package: "MeshPlatformKit"),
            ],
            path: "Tests/MeshEngineKitTests",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
