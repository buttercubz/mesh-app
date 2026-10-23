@preconcurrency import MeshCoreBindings
import CallKit
import Foundation
import MeshPlatform
import Synchronization
import UIKit

// Seams of `EngineHost`: every OS service sits behind a protocol so the host runs with
// fake drivers in tests (Swift conventions: a protocol in front of every OS service).

/// The engine handle (`MeshEngine` from the bindings, or a test decorator).
public typealias EngineHandle = MeshEngineProtocol

/// Builds the engine from its configuration and the host's hooks.
public typealias EngineFactory = @Sendable (FfiEngineConfig, any PlatformHooks) throws -> any EngineHandle

/// A weak, thread-safe reference to the engine. The engine holds the hooks strongly
/// (Rust `Arc`), so everything reachable from the hooks references it weakly.
final class EngineRef: Sendable {
    private struct Box: @unchecked Sendable {
        weak var engine: (any EngineHandle)?
    }

    private let slot = Mutex(Box())

    var engine: (any EngineHandle)? { slot.withLock { $0.engine } }

    func set(_ e: (any EngineHandle)?) {
        slot.withLock { $0.engine = e }
    }
}

/// Is the app foreground-active (`applicationState == .active`)? Read by `gestureCommand`
/// (§3.2) and by audio commands arriving on the engine thread, which must never block on
/// the main thread, so the system implementation keeps a cached value updated on main.
public protocol ForegroundProvider: AnyObject, Sendable {
    var isForegroundActive: Bool { get }
}

/// Cached foreground state, updated by the lifecycle observer.
public final class ForegroundState: ForegroundProvider {
    private let value: Atomic<Bool>

    public init(_ initial: Bool) {
        value = Atomic(initial)
    }

    public var isForegroundActive: Bool { value.load(ordering: .acquiring) }

    public func set(_ active: Bool) {
        value.store(active, ordering: .releasing)
    }

    /// The current `UIApplication` state (main thread only).
    @MainActor
    public static func system() -> ForegroundState {
        ForegroundState(UIApplication.shared.applicationState == .active)
    }
}

/// Whether a phone call is in progress (PRESENCE `on_call`, interruption reasons).
public protocol CallStatePort: AnyObject, Sendable {
    var onCall: Bool { get }
}

/// `CXCallObserver`: any call that has not ended (no `voip` background mode needed).
public final class SystemCallState: CallStatePort, @unchecked Sendable {
    // CXCallObserver is thread-safe for reading `calls`.
    private let observer = CXCallObserver()

    public init() {}

    public var onCall: Bool { observer.calls.contains { !$0.hasEnded } }
}

/// Runs main-actor work from any thread. `sync` is only used on the bridge's call path
/// (gesture commands), never from the engine thread.
enum MainHop {
    static func sync<T: Sendable>(_ body: @MainActor () throws -> T) rethrows -> T {
        if Thread.isMainThread {
            return try MainActor.assumeIsolated(body)
        }
        return try DispatchQueue.main.sync { try MainActor.assumeIsolated(body) }
    }

    static func async(_ body: @escaping @MainActor @Sendable () -> Void) {
        Task { @MainActor in body() }
    }
}

/// The radio drivers behind one object (central + peripheral on iOS; fakes in tests).
public protocol RadioStack: AnyObject, Sendable {
    /// Executes one radio command (each driver ignores the commands of the other role).
    func apply(_ cmd: RadioCommand)
    /// App visibility (foreground duplicates while Forming, §9.4).
    func setForeground(_ fg: Bool)
    /// Published L2CAP PSM (nil when unpublished): the core puts it in HELLO.
    func onPsm(_ listener: @escaping @Sendable (UInt16?) -> Void)
}

/// What a radio stack is built from.
public struct RadioStackContext: Sendable {
    public let constants: BleConstants
    public let registry: LinkRegistry
    public let io: LinkIOThread
    public let radioState: RadioStateMonitor
}

public typealias RadioStackFactory = @Sendable (RadioStackContext) -> any RadioStack

/// The production stack: CoreBluetooth central and peripheral drivers (WP-14).
public final class BleRadioStack: RadioStack {
    public let central: BleCentralDriver
    public let peripheral: BlePeripheralDriver

    public init(_ ctx: RadioStackContext) {
        central = BleCentralDriver(constants: ctx.constants, registry: ctx.registry, io: ctx.io,
                                   radio: ctx.radioState)
        peripheral = BlePeripheralDriver(constants: ctx.constants, registry: ctx.registry, io: ctx.io,
                                         radio: ctx.radioState)
    }

    public func apply(_ cmd: RadioCommand) {
        central.apply(cmd)
        peripheral.apply(cmd)
    }

    public func setForeground(_ fg: Bool) {
        central.setForeground(fg)
    }

    public func onPsm(_ listener: @escaping @Sendable (UInt16?) -> Void) {
        peripheral.onPsm(listener)
    }
}
