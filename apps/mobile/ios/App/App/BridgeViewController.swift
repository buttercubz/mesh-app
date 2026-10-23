import Capacitor
import UIKit

/// Root view controller (Main.storyboard): Capacitor's bridge plus the app-local
/// MeshIntercom plugin (DESIGN §2.4, convLink pattern). The plugin source lives in
/// `Plugins/` (owned by WP-19), never in an SPM package under node_modules.
final class BridgeViewController: CAPBridgeViewController {
    override func capacitorDidLoad() {
        bridge?.registerPluginInstance(MeshIntercomPlugin())
    }
}
