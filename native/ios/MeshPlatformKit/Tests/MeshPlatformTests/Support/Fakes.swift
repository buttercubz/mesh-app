import Foundation
@testable import MeshPlatform
import Synchronization
import XCTest

/// Thread-safe list recorder.
final class Recorder<T: Sendable>: Sendable {
    private let m = Mutex<[T]>([])
    func add(_ v: T) { m.withLock { $0.append(v) } }
    var values: [T] { m.withLock { $0 } }
    func reset() { m.withLock { $0 = [] } }
}

/// Records every LinkSink callback.
final class RecordingSink: LinkSink {
    enum Event: Sendable, Equatable {
        case up(UInt32, LinkInfo)
        case down(UInt32, LinkDownReason)
        case data(UInt32, Data)
        case writable(UInt32)
        case metrics(UInt32, Int16?, UInt16?)
        case discovered(Discovery)
        case hello(UInt32, Data)
        case connectFailed(UInt32, LinkDownReason)
    }

    let events = Recorder<Event>()

    func linkUp(_ link: any MeshLink, info: LinkInfo) { events.add(.up(link.id, info)) }
    func linkDown(_ id: UInt32, reason: LinkDownReason) { events.add(.down(id, reason)) }
    func linkData(_ id: UInt32, _ bytes: Data) { events.add(.data(id, bytes)) }
    func linkWritable(_ id: UInt32) { events.add(.writable(id)) }
    func linkMetrics(_ id: UInt32, rssi: Int16?, attMtu: UInt16?) { events.add(.metrics(id, rssi, attMtu)) }
    func discovered(_ d: Discovery) { events.add(.discovered(d)) }
    func helloRead(_ discId: UInt32, _ bytes: Data) { events.add(.hello(discId, bytes)) }
    func connectFailed(_ discId: UInt32, reason: LinkDownReason) { events.add(.connectFailed(discId, reason)) }

    /// All inbound bytes of one link, concatenated.
    func received(_ id: UInt32) -> Data {
        events.values.reduce(into: Data()) { acc, e in
            if case .data(let i, let d) = e, i == id { acc.append(d) }
        }
    }

    func downs(_ id: UInt32) -> [LinkDownReason] {
        events.values.compactMap { e in
            if case .down(let i, let r) = e, i == id { return r }
            return nil
        }
    }

    func writables(_ id: UInt32) -> Int {
        events.values.filter { $0 == .writable(id) }.count
    }
}

/// A MeshLink that records sends.
final class FakeLink: MeshLink {
    let id: UInt32
    let kind: LinkKind
    let sent = Recorder<Data>()
    let closed = Recorder<UInt8>()
    let outcome = Mutex<SendOutcome>(.accepted)

    init(id: UInt32, kind: LinkKind = .bleL2cap) {
        self.id = id
        self.kind = kind
    }

    func send(_ bytes: Data) -> SendOutcome {
        sent.add(bytes)
        return outcome.withLock { $0 }
    }

    func close(reason: UInt8) { closed.add(reason) }
}

/// Polls `condition` until true or `timeout` (for IO-thread tests).
func waitUntil(_ timeout: TimeInterval = 3, _ condition: () -> Bool) -> Bool {
    let end = Date().addingTimeInterval(timeout)
    while Date() < end {
        if condition() { return true }
        Thread.sleep(forTimeInterval: 0.005)
    }
    return condition()
}

/// A settable clock for clock-injected components.
final class ManualClock: Sendable {
    private let t = Mutex<TimeInterval>(1000)
    var now: TimeInterval { t.withLock { $0 } }
    func advance(_ s: TimeInterval) { t.withLock { $0 += s } }
    var fn: @Sendable () -> TimeInterval { { [self] in now } }
}
