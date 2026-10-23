import Capacitor
import UIKit

/// The single phone scene required by UIKit when built with the iOS 27 SDK (WP-18,
/// DESIGN §10.1.5; convLink fix). `UISceneStoryboardFile = Main` makes UIKit create the
/// window and its root `BridgeViewController` (which registers the app-local
/// MeshIntercom plugin), so this delegate never builds a second bridge.
final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(
        _ scene: UIScene,
        willConnectTo session: UISceneSession,
        options connectionOptions: UIScene.ConnectionOptions
    ) {
        guard scene is UIWindowScene else { return }
        // Load the bridge before forwarding launch links, so AppPlugin can hold
        // `appUrlOpen` until JS subscribes (an invite link that cold-starts the app).
        window?.rootViewController?.loadViewIfNeeded()
        SceneDelegateProxy.shared.scene(scene, willConnectTo: session, options: connectionOptions)
    }

    func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
        SceneDelegateProxy.shared.scene(scene, openURLContexts: URLContexts)
    }

    func scene(_ scene: UIScene, continue userActivity: NSUserActivity) {
        SceneDelegateProxy.shared.scene(scene, continue: userActivity)
    }

    // Capacitor observes UIKit's aggregate UIApplication notifications for
    // pause/resume (the bridge's set_ui_attached); UIKit still posts them with scenes,
    // so nothing is reposted from scene callbacks here.
}
