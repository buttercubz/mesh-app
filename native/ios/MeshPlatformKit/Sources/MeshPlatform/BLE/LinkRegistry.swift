import Foundation
import Synchronization

/// Assigns link ids, keeps the live links and fans their events into the `LinkSink`
/// (DESIGN §9.1). Every driver reports through one registry so ids are unique across
/// transports and `linkDown` is delivered exactly once per link.
public final class LinkRegistry: Sendable {
    private struct State {
        var nextId: UInt32 = 1
        var links: [UInt32: any MeshLink] = [:]
        var infos: [UInt32: LinkInfo] = [:]
        weak var sink: (any LinkSink)?
    }

    private let state = Mutex(State())

    public init(sink: (any LinkSink)? = nil) {
        state.withLock { $0.sink = sink }
    }

    public func setSink(_ sink: (any LinkSink)?) {
        state.withLock { $0.sink = sink }
    }

    /// Reserves a fresh id (never 0, never reused while the registry lives unless it wraps
    /// after 2^32 - 1 links, skipping ids still in use).
    public func allocateId() -> UInt32 {
        state.withLock { s in
            var id = s.nextId
            while id == 0 || s.links[id] != nil {
                id &+= 1
            }
            s.nextId = id &+ 1
            return id
        }
    }

    /// Registers an open link and reports `linkUp`.
    public func up(_ link: any MeshLink, info: LinkInfo) {
        let sink: (any LinkSink)? = state.withLock { s in
            s.links[link.id] = link
            s.infos[link.id] = info
            return s.sink
        }
        sink?.linkUp(link, info: info)
    }

    /// Removes the link and reports `linkDown` once; later calls for the same id are no-ops.
    @discardableResult
    public func down(_ id: UInt32, reason: LinkDownReason) -> Bool {
        let sink: (any LinkSink)?? = state.withLock { s in
            guard s.links.removeValue(forKey: id) != nil else { return .none }
            s.infos.removeValue(forKey: id)
            return .some(s.sink)
        }
        guard case .some(let target) = sink else { return false }
        target?.linkDown(id, reason: reason)
        return true
    }

    /// Inbound bytes; dropped when the link is no longer registered.
    public func data(_ id: UInt32, _ bytes: Data) {
        let sink: (any LinkSink)? = state.withLock { s in s.links[id] != nil ? s.sink : nil }
        sink?.linkData(id, bytes)
    }

    public func writable(_ id: UInt32) {
        let sink: (any LinkSink)? = state.withLock { s in s.links[id] != nil ? s.sink : nil }
        sink?.linkWritable(id)
    }

    public func metrics(_ id: UInt32, rssi: Int16?, attMtu: UInt16?) {
        let sink: (any LinkSink)? = state.withLock { s in s.links[id] != nil ? s.sink : nil }
        sink?.linkMetrics(id, rssi: rssi, attMtu: attMtu)
    }

    public func discovered(_ d: Discovery) {
        state.withLock { $0.sink }?.discovered(d)
    }

    public func connectFailed(_ discId: UInt32, reason: LinkDownReason) {
        state.withLock { $0.sink }?.connectFailed(discId, reason: reason)
    }

    public func helloRead(_ discId: UInt32, _ bytes: Data) {
        state.withLock { $0.sink }?.helloRead(discId, bytes)
    }

    public func link(_ id: UInt32) -> (any MeshLink)? {
        state.withLock { $0.links[id] }
    }

    public func info(_ id: UInt32) -> LinkInfo? {
        state.withLock { $0.infos[id] }
    }

    public var count: Int { state.withLock { $0.links.count } }

    public func ids(kind: LinkKind? = nil) -> [UInt32] {
        state.withLock { s in
            s.infos.values.filter { kind == nil || $0.kind == kind }.map(\.id).sorted()
        }
    }

    /// Non-blocking send by id (`PlatformHooks.link_send`).
    public func send(_ id: UInt32, _ bytes: Data) -> SendOutcome {
        guard let l = link(id) else { return .noLink }
        return l.send(bytes)
    }

    /// Reports every link of `kinds` down with `reason` (e.g. Bluetooth turned off) and
    /// returns their ids. The links themselves are closed by their driver.
    @discardableResult
    public func dropAll(kinds: Set<LinkKind>, reason: LinkDownReason) -> [UInt32] {
        let ids = state.withLock { s in
            s.infos.values.filter { kinds.contains($0.kind) }.map(\.id).sorted()
        }
        return ids.filter { down($0, reason: reason) }
    }
}
