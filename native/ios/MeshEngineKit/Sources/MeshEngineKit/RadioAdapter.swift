@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform
import Synchronization

/// Maps the core's `FfiRadioCommand`s onto the platform drivers and the drivers' link
/// events back onto the engine (DESIGN §3.2 "radio decisions are in the core", §9).
///
/// The drivers are created lazily at the first engine start (creating CoreBluetooth
/// managers shows the Bluetooth permission prompt, which belongs to a foreground tap).
final class RadioAdapter: Sendable {
    let registry: LinkRegistry
    let io: LinkIOThread
    let radioState: RadioStateMonitor
    let constants: BleConstants
    private let sink: EngineLinkSink
    private let engine: EngineRef
    private let factory: RadioStackFactory
    private let stack = Mutex<(any RadioStack)?>(nil)
    private let foreground = Mutex(true)

    init(engine: EngineRef, constants: BleConstants, factory: @escaping RadioStackFactory,
         onRadioState: @escaping @Sendable (RadioState) -> Void) {
        self.engine = engine
        self.constants = constants
        self.factory = factory
        sink = EngineLinkSink(engine: engine)
        registry = LinkRegistry(sink: sink)
        io = LinkIOThread()
        radioState = RadioStateMonitor(listener: onRadioState)
    }

    /// The drivers (created on first use).
    @discardableResult
    func ensureStack() -> any RadioStack {
        let (s, created): (any RadioStack, Bool) = stack.withLock { slot in
            if let s = slot { return (s, false) }
            let s = factory(RadioStackContext(constants: constants, registry: registry, io: io,
                                              radioState: radioState))
            slot = s
            return (s, true)
        }
        if created {
            let engine = self.engine
            s.onPsm { psm in engine.engine?.onL2capPublished(psm: psm ?? 0) }
            s.setForeground(foreground.withLock { $0 })
        }
        return s
    }

    var hasStack: Bool { stack.withLock { $0 != nil } }

    /// `PlatformHooks.radio` (engine thread; the drivers hop onto their own queues).
    func handle(_ cmd: FfiRadioCommand) {
        guard let s = stack.withLock({ $0 }) else { return }
        s.apply(RadioCommand(cmd))
    }

    /// `PlatformHooks.link_send`: non-blocking, one unit.
    func send(_ link: UInt32, _ bytes: Data) -> FfiSendOutcome {
        FfiSendOutcome(registry.send(link, bytes))
    }

    func setForeground(_ fg: Bool) {
        foreground.withLock { $0 = fg }
        stack.withLock { $0 }?.setForeground(fg)
    }

    /// The combined Bluetooth state (nil until CoreBluetooth reported one).
    var bluetooth: RadioState? { radioState.current }
}

/// The engine as the drivers' `LinkSink` (contract C9 → C4). Held strongly by the
/// adapter: the registry keeps its sink weakly.
final class EngineLinkSink: LinkSink {
    private let engine: EngineRef

    init(engine: EngineRef) {
        self.engine = engine
    }

    func linkUp(_ link: any MeshLink, info: LinkInfo) {
        engine.engine?.onLinkUp(link: info.id, info: FfiLinkInfo(info))
    }

    func linkDown(_ id: UInt32, reason: LinkDownReason) {
        engine.engine?.onLinkDown(link: id, reason: FfiLinkDownReason(reason))
    }

    func linkData(_ id: UInt32, _ bytes: Data) {
        engine.engine?.onLinkData(link: id, data: bytes)
    }

    func linkWritable(_ id: UInt32) {
        engine.engine?.onLinkWritable(link: id)
    }

    func linkMetrics(_ id: UInt32, rssi: Int16?, attMtu: UInt16?) {
        engine.engine?.onLinkMetrics(link: id, rssi: rssi, attMtu: attMtu)
    }

    func discovered(_ d: Discovery) {
        engine.engine?.onDiscovered(d: FfiDiscovery(d))
    }

    func helloRead(_ discId: UInt32, _ bytes: Data) {
        engine.engine?.onHelloRead(discId: discId, bytes: bytes)
    }

    func connectFailed(_ discId: UInt32, reason: LinkDownReason) {
        engine.engine?.onConnectFailed(discId: discId, reason: FfiLinkDownReason(reason))
    }
}
