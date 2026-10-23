import Foundation
@testable import MeshEngineKit
import MeshPlatform
import Synchronization

/// An in-process radio medium for several hosts: it executes the same `RadioCommand`s the
/// CoreBluetooth drivers execute (scan/advertise per `SetDiscovery`, HELLO read on
/// `Connect`, link open on `OpenLink`, `Close`) and reports through each host's real
/// `LinkRegistry`, so the whole RadioAdapter / LinkSink / FFI path runs as on a phone.
final class FakeAir: @unchecked Sendable {
    struct Node {
        var registry: LinkRegistry
        var mode: DiscoveryMode = .off
        var adv: AdvConfig?
        var hello = Data()
    }

    static let psm: UInt16 = 0x0081
    private let lock = NSLock()
    private var nodes: [Int: Node] = [:]
    private var links: [UInt32: (node: Int, link: FakeLink)] = [:]
    private let q = DispatchQueue(label: "fake.air")
    /// Every command each node executed (node index, command).
    private(set) var commands: [(Int, RadioCommand)] = []

    func stack(_ ctx: RadioStackContext) -> FakeRadioStack {
        let idx = lock.withLock { () -> Int in
            let i = nodes.count
            nodes[i] = Node(registry: ctx.registry)
            return i
        }
        return FakeRadioStack(air: self, node: idx)
    }

    func discId(of peer: Int) -> UInt32 { UInt32(100 + peer) }

    func apply(_ node: Int, _ cmd: RadioCommand) {
        q.async { [self] in handle(node, cmd) }
    }

    func executed(_ node: Int) -> [RadioCommand] {
        lock.withLock { commands.filter { $0.0 == node }.map(\.1) }
    }

    private func handle(_ node: Int, _ cmd: RadioCommand) {
        lock.withLock { commands.append((node, cmd)) }
        switch cmd {
        case .setDiscovery(let mode, let adv):
            lock.withLock {
                nodes[node]?.mode = mode
                nodes[node]?.adv = adv
            }
            scan()
        case .setHello(let bytes):
            lock.withLock { nodes[node]?.hello = bytes }
        case .connect(let discId):
            let peer = Int(discId) - 100
            let (reg, hello) = lock.withLock { (nodes[node]?.registry, nodes[peer]?.hello) }
            if let hello, !hello.isEmpty { reg?.helloRead(discId, hello) }
        case .openLink(let discId, let via):
            open(node, peer: Int(discId) - 100, discId: discId, via: via)
        case .close(let id, _):
            let l = lock.withLock { links[id]?.link }
            l?.close(reason: 0)
        case .abort, .setLinkParams, .armReconnect, .awareUpgrade, .tunnelOpen:
            break
        }
    }

    /// Every pair of nodes that both advertise and scan sees each other on each discovery
    /// change (iPhone in the foreground: the 6-char local name; the core ignores peers it
    /// is already linked to).
    private func scan() {
        let deliveries: [(LinkRegistry, Discovery)] = lock.withLock {
            var out: [(LinkRegistry, Discovery)] = []
            for (i, a) in nodes where a.mode != .off {
                for (j, b) in nodes where j != i && b.mode != .off {
                    guard let adv = b.adv else { continue }
                    let name = AdvBuilder.iosLocalName(groupTag: [UInt8](adv.groupTag),
                                                       nodeToken: [UInt8](adv.nodeToken))
                    let info = AdvBuilder.parseLocalName(name)!
                    out.append((a.registry, Discovery(
                        discId: discId(of: j), transport: .ble, rssi: -50,
                        tokens: AdvTokens(groupTag: Data(info.groupTagPrefix), tokenPrefix14: info.tokenPrefix14),
                        platformHint: .ios)))
                }
            }
            return out
        }
        for (reg, d) in deliveries { reg.discovered(d) }
    }

    private func open(_ node: Int, peer: Int, discId: UInt32, via: OpenVia) {
        guard let (ra, rb) = lock.withLock({ () -> (LinkRegistry, LinkRegistry)? in
            guard let a = nodes[node]?.registry, let b = nodes[peer]?.registry else { return nil }
            return (a, b)
        }) else { return }
        let kind: LinkKind
        let unit: LinkUnit
        let maxFrame: UInt16
        switch via {
        case .l2cap:
            (kind, unit, maxFrame) = (.bleL2cap, .stream, 2 + 1024)
        case .gatt:
            (kind, unit, maxFrame) = (.bleGatt, .message, 182)
        }
        let a = FakeLink(id: ra.allocateId(), kind: kind, registry: ra)
        let b = FakeLink(id: rb.allocateId(), kind: kind, registry: rb)
        a.peer = b
        b.peer = a
        lock.withLock {
            links[a.id] = (node, a)
            links[b.id] = (peer, b)
        }
        rb.up(b, info: LinkInfo(id: b.id, kind: kind, role: .responder, maxFrame: maxFrame, unit: unit,
                                peerPlatformHint: .ios))
        ra.up(a, info: LinkInfo(id: a.id, kind: kind, role: .initiator, maxFrame: maxFrame, unit: unit,
                                peerPlatformHint: .ios, discId: discId))
    }

    /// Units sent so far over the links of `node`.
    func sentUnits(_ node: Int) -> Int {
        lock.withLock { links.values.filter { $0.node == node }.map { $0.link.sent.withLock { $0 } }.reduce(0, +) }
    }

    /// Links of `node` that are still open.
    func openLinks(_ node: Int) -> Int {
        lock.withLock { links.values.filter { $0.node == node && !$0.link.isClosed }.count }
    }

    /// Drops every link (walking apart).
    func cutAll() {
        let all = lock.withLock { links.values.map(\.link) }
        for l in all { l.close(reason: 0) }
    }
}

/// One end of an in-memory link: `send` delivers the unit to the peer's registry.
final class FakeLink: MeshLink, @unchecked Sendable {
    let id: UInt32
    let kind: LinkKind
    let registry: LinkRegistry
    weak var peer: FakeLink?
    private let closed = Mutex(false)
    let sent = Mutex(0)

    init(id: UInt32, kind: LinkKind, registry: LinkRegistry) {
        self.id = id
        self.kind = kind
        self.registry = registry
    }

    func send(_ bytes: Data) -> SendOutcome {
        guard !closed.withLock({ $0 }), let p = peer, !p.closed.withLock({ $0 }) else { return .noLink }
        sent.withLock { $0 += 1 }
        p.registry.data(p.id, bytes)
        return .accepted
    }

    var isClosed: Bool { closed.withLock { $0 } }

    func close(reason: UInt8) {
        let was = closed.withLock { c -> Bool in
            let w = c
            c = true
            return w
        }
        guard !was else { return }
        registry.down(id, reason: .localClose)
        if let p = peer {
            p.closed.withLock { $0 = true }
            p.registry.down(p.id, reason: .peerClosed)
        }
    }
}

/// The fake drivers of one host.
final class FakeRadioStack: RadioStack, @unchecked Sendable {
    let air: FakeAir
    let node: Int
    private(set) var foreground = true

    init(air: FakeAir, node: Int) {
        self.air = air
        self.node = node
    }

    func apply(_ cmd: RadioCommand) { air.apply(node, cmd) }
    func setForeground(_ fg: Bool) { foreground = fg }
    func onPsm(_ listener: @escaping @Sendable (UInt16?) -> Void) { listener(FakeAir.psm) }
}
