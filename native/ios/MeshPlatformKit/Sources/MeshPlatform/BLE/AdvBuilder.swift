import CoreBluetooth
import Foundation

/// Advertising payloads (DESIGN §9.4), byte-compatible with mesh-wire `adv` and the
/// `test-vectors/wire/adv.json` vectors.
public enum AdvBuilder {
    public static let adFlags: UInt8 = 0x01
    public static let adUuid128Complete: UInt8 = 0x07
    public static let adLocalName: UInt8 = 0x09
    public static let adServiceData128: UInt8 = 0x21
    public static let flagsLeGeneralNoBrEdr: UInt8 = 0x06
    public static let localNameLength = 6
    public static let serviceDataLength = 9
    public static let base32Alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".utf8)

    /// Top 14 bits of the node token (the iPhone fg role tie-break, §9.2).
    public static func tokenPrefix14(_ nodeToken: [UInt8]) -> UInt16 {
        guard nodeToken.count >= 2 else { return 0 }
        return (UInt16(nodeToken[0]) << 6) | (UInt16(nodeToken[1]) >> 2)
    }

    /// iPhone foreground local name: 6 RFC 4648 base32 chars (MSB first) of
    /// `group_tag[0] ‖ group_tag[1] ‖ token_prefix14` (30 bits).
    public static func iosLocalName(groupTag: [UInt8], nodeToken: [UInt8]) -> String {
        let g0 = UInt32(groupTag.count > 0 ? groupTag[0] : 0)
        let g1 = UInt32(groupTag.count > 1 ? groupTag[1] : 0)
        let v: UInt32 = (g0 << 22) | (g1 << 14) | UInt32(tokenPrefix14(nodeToken))
        var out = [UInt8]()
        out.reserveCapacity(localNameLength)
        for i in 0..<localNameLength {
            let shift = UInt32(5 * (localNameLength - 1 - i))
            out.append(base32Alphabet[Int((v >> shift) & 0x1F)])
        }
        return String(decoding: out, as: UTF8.self)
    }

    /// The local name as an AD structure (`len ‖ 0x09 ‖ name`), 8 B.
    public static func iosLocalNameAd(groupTag: [UInt8], nodeToken: [UInt8]) -> Data {
        var o = Data([UInt8(1 + localNameLength), adLocalName])
        o.append(contentsOf: Array(iosLocalName(groupTag: groupTag, nodeToken: nodeToken).utf8))
        return o
    }

    /// Parsed iPhone local name.
    public struct LocalNameInfo: Sendable, Equatable {
        public var groupTagPrefix: [UInt8]
        public var tokenPrefix14: UInt16

        public func matches(groupTag: [UInt8]) -> Bool {
            groupTag.count >= 2 && groupTagPrefix == Array(groupTag[0..<2])
        }
    }

    /// Parses a 6-char local name; nil for any other name (other apps, other devices).
    public static func parseLocalName(_ name: String) -> LocalNameInfo? {
        let b = Array(name.utf8)
        guard b.count == localNameLength else { return nil }
        var v: UInt32 = 0
        for c in b {
            guard let d = base32Alphabet.firstIndex(of: c) else { return nil }
            v = (v << 5) | UInt32(d)
        }
        return LocalNameInfo(groupTagPrefix: [UInt8((v >> 22) & 0xFF), UInt8((v >> 14) & 0xFF)],
                             tokenPrefix14: UInt16(v & 0x3FFF))
    }

    /// Android primary ADV: Flags (3 B) + complete 128-bit UUID list (18 B) = 21 B.
    public static func androidAdvData(service: CBUUID) -> Data {
        var o = Data([2, adFlags, flagsLeGeneralNoBrEdr, 17, adUuid128Complete])
        o.append(service.littleEndianBytes)
        return o
    }

    /// Service data body: `flags u8 (b0 android, b1 lab) ‖ group_tag[4] ‖ node_token[4]`.
    public static func serviceData(android: Bool, lab: Bool, groupTag: [UInt8], nodeToken: [UInt8]) -> Data {
        var o = Data([(android ? 1 : 0) | (lab ? 2 : 0)])
        o.append(contentsOf: groupTag.prefix(4))
        o.append(contentsOf: nodeToken.prefix(4))
        return o
    }

    /// Android scan response: Service Data 128-bit (AD 0x21) = 27 B.
    public static func scanResponse(service: CBUUID, serviceData: Data) -> Data {
        var o = Data([UInt8(1 + 16 + serviceData.count), adServiceData128])
        o.append(service.littleEndianBytes)
        o.append(serviceData)
        return o
    }

    /// Decodes a 9-byte service-data body into advertising tokens.
    public static func parseServiceData(_ data: Data) -> AdvTokens? {
        let b = [UInt8](data)
        guard b.count == serviceDataLength else { return nil }
        let token = Array(b[5..<9])
        return AdvTokens(groupTag: Data(b[1..<5]), tokenPrefix14: tokenPrefix14(token),
                         nodeToken: Data(token), android: b[0] & 1 != 0, lab: b[0] & 2 != 0)
    }

    /// CoreBluetooth advertisement dictionary for the iPhone foreground (§9.4): service
    /// UUID in the primary ADV, 6-char local name in the scan response. In the background
    /// iOS moves the UUID to the overflow area and drops the name by itself.
    public static func iosAdvertisement(service: CBUUID, adv: AdvConfig) -> [String: Any] {
        [
            CBAdvertisementDataServiceUUIDsKey: [service],
            CBAdvertisementDataLocalNameKey: iosLocalName(groupTag: [UInt8](adv.groupTag),
                                                          nodeToken: [UInt8](adv.nodeToken)),
        ]
    }
}
