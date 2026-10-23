@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform

// MARK: Core → platform

extension AdvConfig {
    /// The core builds the advertising payloads (§9.4); the iOS drivers take the tokens and
    /// rebuild the OS-specific forms. The 9-byte service-data body carries every token
    /// (`flags ‖ group_tag[4] ‖ node_token[4]`); the 6-char local name is the fallback.
    init?(_ p: FfiAdvPayload) {
        if let t = AdvBuilder.parseServiceData(p.serviceData), let token = t.nodeToken {
            self.init(groupTag: t.groupTag, nodeToken: token, lab: t.lab)
        } else if let info = AdvBuilder.parseLocalName(p.localName) {
            self.init(groupTag: Data(info.groupTagPrefix),
                      nodeToken: Data(FfiAdvData.tokenBytes(prefix14: info.tokenPrefix14)), lab: false)
        } else {
            return nil
        }
    }
}

extension DiscoveryMode {
    init(_ m: FfiDiscoveryMode) {
        switch m {
        case .off: self = .off
        case .forming: self = .forming
        case .stable: self = .stable
        }
    }
}

extension OpenVia {
    init(_ v: FfiOpenVia) {
        switch v {
        case .l2cap(let psm): self = .l2cap(psm: psm)
        case .gatt: self = .gatt
        }
    }
}

extension LinkPriority {
    init(_ p: FfiLinkPriority) {
        switch p {
        case .high: self = .high
        case .balanced: self = .balanced
        }
    }
}

extension RadioCommand {
    /// Every radio decision is the core's; the drivers only execute (§3.2).
    init(_ c: FfiRadioCommand) {
        switch c {
        case .setDiscovery(let mode, let adv, let scan):
            self = .setDiscovery(mode: DiscoveryMode(mode), adv: adv.flatMap(AdvConfig.init))
        case .setHello(let bytes):
            self = .setHello(bytes)
        case .connect(let discId):
            self = .connect(discId: discId)
        case .openLink(let discId, let via):
            self = .openLink(discId: discId, via: OpenVia(via))
        case .abort(let discId, let ignoreS):
            self = .abort(discId: discId, ignoreS: UInt32(ignoreS))
        case .close(let link, let reason):
            self = .close(link: link, reason: reason ?? 0)
        case .setLinkParams(let link, let priority):
            self = .setLinkParams(link: link, priority: LinkPriority(priority))
        case .armReconnect(let peerRef, let untilS):
            self = .armReconnect(discId: peerRef, untilS: UInt32(untilS))
        case .awareUpgrade(let peer, let op):
            self = .awareUpgrade(peer: UInt32(peer), op: op)
        case .tunnelOpen(let peer):
            // v2 hook (§9.8): the peer's node id is the only hint the core has today.
            self = .tunnelOpen(rendezvous: Data(),
                               peerHint: Data([UInt8(truncatingIfNeeded: peer),
                                               UInt8(truncatingIfNeeded: peer >> 8)]))
        }
    }
}

extension FfiSendOutcome {
    init(_ o: SendOutcome) {
        switch o {
        case .accepted: self = .accepted
        case .wouldBlock: self = .wouldBlock
        case .noLink: self = .noLink
        }
    }
}

extension BleConstants {
    /// The production BLE constants from the core's `ble_constants()` (single source of
    /// truth, §3.5); fields the core does not own (restore ids) come from `base`.
    init?(_ c: FfiBleConstants, base: BleConstants = .design) {
        guard let svc = UUID(uuidString: c.serviceUuid), let hello = UUID(uuidString: c.helloUuid),
              let rx = UUID(uuidString: c.rxUuid), let tx = UUID(uuidString: c.txUuid) else { return nil }
        self.init(meshServiceId: svc, helloId: hello, rxId: rx, txId: tx,
                  connectTimeout: TimeInterval(c.connectTimeoutMs) / 1000,
                  centralRestoreId: base.centralRestoreId, peripheralRestoreId: base.peripheralRestoreId,
                  l2capMaxFrame: 2 + c.maxStreamFrame)
    }
}

extension VehicleProfile {
    /// mesh-api `vehicle` ("moto" | "car").
    init?(apiValue: String) {
        switch apiValue {
        case "moto": self = .motorcycle
        case "car": self = .car
        default: return nil
        }
    }
}

/// Route preference (`AudioCommand::SetRoutePreference`).
public enum RoutePreference: String, Sendable, Equatable {
    case auto
    case headset
    case speaker

    init(_ p: FfiRoutePreference) {
        switch p {
        case .auto: self = .auto
        case .headset: self = .headset
        case .speaker: self = .speaker
        }
    }
}

/// The session to start (`FfiSessionStart` without the binding types).
public enum SessionStart: Sendable, Equatable {
    /// Lobby of a stored group: radios on, audio off until Go live.
    case group(groupId: Data)
    /// Lab mode with the 4-digit code (§12.1).
    case lab(code: UInt16)
    /// Demo ride (runtime-owned world, Phase 1).
    case demo

    var ffi: FfiSessionStart {
        switch self {
        case .group(let id): return .group(groupId: id)
        case .lab(let code): return .lab(labCode: code)
        case .demo: return .demo
        }
    }

    /// `startEngine({groupId})` passes the id as 32 hex chars.
    public static func group(hex: String) -> SessionStart? {
        let chars = Array(hex.utf8)
        guard chars.count == 32 else { return nil }
        var out = Data(capacity: 16)
        var i = 0
        while i < 32 {
            guard let hi = hexValue(chars[i]), let lo = hexValue(chars[i + 1]) else { return nil }
            out.append(hi << 4 | lo)
            i += 2
        }
        return .group(groupId: out)
    }

    private static func hexValue(_ c: UInt8) -> UInt8? {
        switch c {
        case UInt8(ascii: "0")...UInt8(ascii: "9"): return c - UInt8(ascii: "0")
        case UInt8(ascii: "a")...UInt8(ascii: "f"): return c - UInt8(ascii: "a") + 10
        case UInt8(ascii: "A")...UInt8(ascii: "F"): return c - UInt8(ascii: "A") + 10
        default: return nil
        }
    }
}
