import Capacitor
import UIKit

/// Process-level setup only (WP-18, DESIGN §10.1.5). UIKit creates the window from the
/// scene manifest (`UIApplicationSceneManifest`, iOS 27 requirement) and `SceneDelegate`
/// owns the UI lifecycle and every link (`convlinkvoice://join`), forwarded through
/// Capacitor's `SceneDelegateProxy` to @capacitor/app `appUrlOpen`. The legacy
/// `application(_:open:options:)` callbacks are not used with scenes (deprecated in
/// iOS 26). Live Activity action handlers are registered at launch by the bridge (WP-19).
@main
final class AppDelegate: UIResponder, UIApplicationDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
        true
    }
}
