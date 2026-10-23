import CMeshAudio
import Foundation
import MeshActivityShared

/// The production Live Activity action handler: mute and latch only write RT flags through
/// `cmesh_update_flags` (forwarded to the core's `mesh_rt_set_flags`), which is legal
/// while locked because the app process already captures (DESIGN §10.1.5). The app
/// registers it once at launch (WP-19 through MeshEngineKit).
public enum LiveActivityRtActions {
    /// Bit 1: hard mute (level).
    public static let hardMuteBit: UInt32 = CMESH_FLAG_HARD_MUTE
    /// Bit 3: PTT latch toggled (edge, carried as a toggle the core detects).
    public static let latchToggleBit: UInt32 = CMESH_FLAG_PTT_LATCH_TOGGLE

    /// Installs the handler. `resumeMic` runs when the Resume intent has opened the app.
    public static func register(resumeMic: @escaping @Sendable () -> Void) {
        LiveActivityActionHandler.register { action in
            perform(action, resumeMic: resumeMic)
        }
    }

    /// Applies one action; returns the new RT flags (unchanged for `resumeMic`).
    @discardableResult
    public static func perform(_ action: LiveActivityAction, resumeMic: @Sendable () -> Void) -> UInt32 {
        switch action {
        case .toggleMute:
            return cmesh_update_flags(0, 0, hardMuteBit)
        case .latchTalk:
            return cmesh_update_flags(0, 0, latchToggleBit)
        case .resumeMic:
            resumeMic()
            return cmesh_flags()
        }
    }

    /// PTT fast path for the bridge (bit 0 level).
    @discardableResult
    public static func setPttDown(_ down: Bool) -> UInt32 {
        down ? cmesh_update_flags(CMESH_FLAG_PTT_DOWN, 0, 0) : cmesh_update_flags(0, CMESH_FLAG_PTT_DOWN, 0)
    }
}
