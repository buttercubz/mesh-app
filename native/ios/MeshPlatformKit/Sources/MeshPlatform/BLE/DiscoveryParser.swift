import CoreBluetooth
import Foundation

/// Turns a CoreBluetooth advertisement dictionary into the pre-connection information the
/// core uses (§9.2: only pre-connection information is used before connecting).
public enum DiscoveryParser {
    /// Advertising tokens and platform hint found in one advertisement.
    public struct Parsed: Sendable, Equatable {
        public var tokens: AdvTokens?
        public var platform: PeerPlatform
    }

    public static func parse(advertisement: [String: Any], service: CBUUID) -> Parsed {
        // Android: the scan response carries our service data (flags.android = 1).
        if let sd = advertisement[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data],
           let body = sd[service], let tokens = AdvBuilder.parseServiceData(body) {
            return Parsed(tokens: tokens, platform: tokens.android ? .android : .unknown)
        }
        // iPhone foreground: the 6-char base32 local name.
        if let name = advertisement[CBAdvertisementDataLocalNameKey] as? String,
           let info = AdvBuilder.parseLocalName(name) {
            return Parsed(tokens: AdvTokens(groupTag: Data(info.groupTagPrefix),
                                            tokenPrefix14: info.tokenPrefix14),
                          platform: .ios)
        }
        // iPhone background (overflow area) or anything else: UUID only, no tokens.
        let overflow = advertisement[CBAdvertisementDataOverflowServiceUUIDsKey] as? [CBUUID] ?? []
        return Parsed(tokens: nil, platform: overflow.contains(service) ? .ios : .unknown)
    }

    /// Clamps CoreBluetooth's RSSI (127 = unavailable) to the core's `Option<i16>`.
    public static func rssi(_ value: NSNumber) -> Int16? {
        let v = value.intValue
        guard v != 127, v >= Int(Int16.min), v <= 20 else { return nil }
        return Int16(v)
    }
}
