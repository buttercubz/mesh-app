@preconcurrency import MeshCoreBindings
import Foundation
import MeshPlatform

// Mapping between the platform layer's Swift types (MeshPlatformKit, contract C9) and the
// UniFFI types of the control plane (contract C4). Pure functions, unit-tested; no byte is
// parsed or framed here (all framing lives in Rust, DESIGN §3.2).

// MARK: Platform → core

extension FfiLinkKind {
    init(_ k: LinkKind) {
        switch k {
        case .bleL2cap: self = .bleL2cap
        case .bleGatt: self = .bleGatt
        case .wifiAware: self = .wifiAware
        case .tunnel: self = .tunnel
        }
    }
}

extension FfiPeerPlatform {
    /// `nil` when the platform layer does not know the peer's OS.
    static func from(_ p: PeerPlatform) -> FfiPeerPlatform? {
        switch p {
        case .unknown: return nil
        case .ios: return .ios
        case .android: return .android
        }
    }
}

extension FfiLinkInfo {
    init(_ i: LinkInfo, attMtu: UInt16? = nil) {
        self.init(
            kind: FfiLinkKind(i.kind),
            role: i.role == .initiator ? .initiator : .responder,
            maxFrame: i.maxFrame,
            unit: {
                switch i.unit {
                case .stream: return .stream
                case .message: return .message
                case .datagram: return .datagram
                }
            }(),
            peerPlatformHint: FfiPeerPlatform.from(i.peerPlatformHint),
            attMtu: attMtu,
            discId: i.discId
        )
    }
}

extension FfiLinkDownReason {
    init(_ r: LinkDownReason) {
        switch r {
        case .timeout: self = .timeout
        case .peerClosed: self = .peerClosed
        case .localClose: self = .localClose
        case .radioOff: self = .radioOff
        case .limitReached: self = .limitReached
        case .error(let code): self = .error(code: code)
        }
    }
}

extension FfiAdvData {
    /// Re-encodes the pre-connection tokens the platform parsed into the raw form the core
    /// classifies (§9.2, §9.4): Android service data (9 B body), the iPhone 6-char local
    /// name, or nothing but the service UUID (iPhone in the background).
    init(_ tokens: AdvTokens?) {
        guard let t = tokens else {
            self = .uuidOnly
            return
        }
        if let token = t.nodeToken, t.groupTag.count == 4 {
            self = .serviceData(bytes: AdvBuilder.serviceData(android: t.android, lab: t.lab,
                                                              groupTag: [UInt8](t.groupTag),
                                                              nodeToken: [UInt8](token)))
        } else {
            self = .localName(name: AdvBuilder.iosLocalName(groupTag: [UInt8](t.groupTag.prefix(2)),
                                                            nodeToken: Self.tokenBytes(prefix14: t.tokenPrefix14)))
        }
    }

    /// Two token bytes whose top 14 bits are `prefix14` (inverse of `tokenPrefix14`).
    static func tokenBytes(prefix14 p: UInt16) -> [UInt8] {
        [UInt8(truncatingIfNeeded: p >> 6), UInt8(truncatingIfNeeded: (p & 0x3F) << 2)]
    }
}

extension FfiDiscovery {
    init(_ d: Discovery) {
        self.init(
            discId: d.discId,
            transport: d.transport == .ble ? .ble : .aware,
            rssi: d.rssi,
            adv: FfiAdvData(d.tokens),
            platformHint: FfiPeerPlatform.from(d.platformHint)
        )
    }
}

extension FfiAudioRouteKind {
    init(_ k: AudioRouteKind) {
        switch k {
        case .hfpWideband: self = .hfpWideband
        case .hfpNarrowband: self = .hfpNarrowband
        case .leAudio: self = .leAudio
        case .builtinSpeaker: self = .builtinSpeaker
        case .builtinReceiver: self = .builtinReceiver
        case .wired: self = .wired
        case .carBluetooth: self = .carBluetooth
        case .none: self = .none
        }
    }

    /// The output side expressed as a route kind (the core has one vocabulary).
    static func output(_ o: AudioOutputKind, input: AudioRouteKind) -> FfiAudioRouteKind {
        switch o {
        case .bluetoothHfp:
            return input == .hfpNarrowband ? .hfpNarrowband : .hfpWideband
        case .bluetoothLe: return .leAudio
        case .speaker: return .builtinSpeaker
        case .receiver: return .builtinReceiver
        case .wired: return .wired
        case .carAudio: return .carBluetooth
        case .airplay, .none: return .none
        }
    }
}

extension FfiAudioRoute {
    init(_ r: AudioRouteInfo) {
        self.init(kind: FfiAudioRouteKind(r.kind), inputRate: r.inputRate,
                  output: FfiAudioRouteKind.output(r.output, input: r.kind),
                  headsetPresent: r.headsetPresent)
    }
}

extension FfiInterruptReason {
    /// iOS cannot tell a phone call from Siri or another app's session, so the call
    /// observer decides (`onCall`): only a real call raises PRESENCE `on_call` (§10.1.4).
    init(_ r: InterruptionReason, onCall: Bool) {
        switch r {
        case .otherSession: self = onCall ? .call : .otherApp
        case .routeDisconnected: self = .routeChange
        case .appWasSuspended, .builtInMicMuted, .unknown: self = onCall ? .call : .other
        }
    }
}

extension FfiAudioIoState {
    init(_ s: AudioIoState, onCall: Bool) {
        switch s {
        case .running: self = .running
        case .interrupted(let r): self = .interrupted(reason: FfiInterruptReason(r, onCall: onCall))
        case .restarting: self = .restarting
        case .captureLost: self = .captureLost
        case .renderLost: self = .renderLost
        case .restored: self = .restored
        }
    }
}

extension FfiRadioAvail {
    init(_ s: RadioState) {
        switch s {
        case .on: self = .on
        case .off: self = .off
        case .unauthorized: self = .unauthorized
        case .unsupported: self = .unsupported
        }
    }
}

extension FfiThermal {
    init(_ t: ThermalLevel) {
        switch t {
        case .nominal: self = .nominal
        case .fair: self = .fair
        case .serious: self = .serious
        case .critical: self = .critical
        }
    }
}

extension FfiGpsFix {
    init(_ f: LocationFix) {
        self.init(lat: f.latitude, lon: f.longitude, speedMps: f.speed,
                  accM: f.accuracy >= 0 ? f.accuracy : nil, headingDeg: nil)
    }
}
