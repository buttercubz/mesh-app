import Foundation
import os

/// `os.Logger` channels of the platform layer (subsystem `com.convlink.voice`). Errors that
/// are handled locally (not thrown to the engine host) are logged here; nothing is logged
/// on the audio RT thread.
public enum PlatformLog {
    public static let subsystem = "com.convlink.voice"
    public static let audio = Logger(subsystem: subsystem, category: "audio")
    public static let ble = Logger(subsystem: subsystem, category: "ble")
    public static let host = Logger(subsystem: subsystem, category: "host")
}
