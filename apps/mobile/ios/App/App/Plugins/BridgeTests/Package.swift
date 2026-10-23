// swift-tools-version: 6.2
// Bridge tests for the app-local MeshIntercom plugin (WP-19, DESIGN §11.1).
//
// The App target compiles `Plugins/MeshIntercomPlugin.swift` directly; this package
// compiles the very same file (through a symlink) against the same Capacitor release and
// the in-repo MeshEngineKit / MeshPlatformKit, so its gating logic can run under XCTest
// on a simulator:
//
//   cd apps/mobile/ios/App/App/Plugins/BridgeTests && \
//   xcodebuild -scheme MeshIntercomBridge -destination "id=$(../../../../../../../scripts/pick-sim.sh)" test
import PackageDescription

let package = Package(
    name: "MeshIntercomBridge",
    platforms: [.iOS(.v26)],
    products: [
        .library(name: "MeshIntercomBridge", targets: ["MeshIntercomBridge"]),
    ],
    dependencies: [
        // Same pin as the generated CapApp-SPM package.
        .package(url: "https://github.com/ionic-team/capacitor-swift-pm.git", exact: "8.5.3"),
        .package(path: "../../../../../../../native/ios/MeshEngineKit"),
        .package(path: "../../../../../../../native/ios/MeshPlatformKit"),
    ],
    targets: [
        .target(
            name: "MeshIntercomBridge",
            dependencies: [
                .product(name: "Capacitor", package: "capacitor-swift-pm"),
                // Capacitor's headers import Cordova (CapApp-SPM links both as well).
                .product(name: "Cordova", package: "capacitor-swift-pm"),
                .product(name: "MeshEngineKit", package: "MeshEngineKit"),
                .product(name: "MeshPlatform", package: "MeshPlatformKit"),
            ],
            path: "Sources/MeshIntercomBridge",
            // The App target builds the plugin in Swift 5 mode.
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "MeshIntercomBridgeTests",
            dependencies: [
                "MeshIntercomBridge",
                .product(name: "Capacitor", package: "capacitor-swift-pm"),
                // Capacitor's headers import Cordova (CapApp-SPM links both as well).
                .product(name: "Cordova", package: "capacitor-swift-pm"),
                .product(name: "MeshEngineKit", package: "MeshEngineKit"),
                .product(name: "MeshPlatform", package: "MeshPlatformKit"),
            ],
            path: "Tests/MeshIntercomBridgeTests",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
