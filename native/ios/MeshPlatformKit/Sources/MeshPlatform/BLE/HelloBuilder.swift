import CryptoKit
import Foundation

/// HELLO characteristic value (16 B, DESIGN §9.3), byte-compatible with mesh-wire's
/// `Hello` and the `test-vectors/wire/hello.json` vectors.
///
/// In production the core supplies HELLO bytes through `RadioCommand.setHello` and the
/// driver publishes them verbatim. This builder exists for the Rust-free Lab paths and for
/// vector replay; it never decides anything the core decides.
public struct Hello: Sendable, Equatable {
    public struct Flags: OptionSet, Sendable, Hashable {
        public let rawValue: UInt8
        public init(rawValue: UInt8) { self.rawValue = rawValue }
        public static let android = Flags(rawValue: 1 << 0)
        public static let lab = Flags(rawValue: 1 << 1)
        public static let accepting = Flags(rawValue: 1 << 2)
        public static let aware = Flags(rawValue: 1 << 3)
        public static let leader = Flags(rawValue: 1 << 4)
    }

    public static let length = 16
    public static let magic: UInt8 = 0xC1
    public static let protocolVersion: UInt8 = 1

    public var protoMin: UInt8
    public var protoMax: UInt8
    public var flags: Flags
    /// L2CAP PSM; 0 means GATT only.
    public var psm: UInt16
    public var groupTag: [UInt8]
    public var nodeToken: [UInt8]
    public var epochLo: UInt16

    public init(protoMin: UInt8 = Hello.protocolVersion, protoMax: UInt8 = Hello.protocolVersion,
                flags: Flags, psm: UInt16, groupTag: [UInt8], nodeToken: [UInt8], epochLo: UInt16) {
        self.protoMin = protoMin
        self.protoMax = protoMax
        self.flags = flags
        self.psm = psm
        self.groupTag = groupTag
        self.nodeToken = nodeToken
        self.epochLo = epochLo
    }

    /// Group HELLO: the lab flag is cleared and `epoch_lo = epoch mod 2^16`.
    public static func group(flags: Flags, psm: UInt16, groupTag: [UInt8], nodeToken: [UInt8],
                             epoch: UInt32) -> Hello {
        Hello(flags: flags.subtracting(.lab), psm: psm, groupTag: groupTag, nodeToken: nodeToken,
              epochLo: UInt16(truncatingIfNeeded: epoch))
    }

    /// Lab HELLO: `flags.lab = 1`, `group_tag = lab_tag (LE) ‖ 0x0000`, `epoch_lo = 0`.
    public static func lab(flags: Flags, psm: UInt16, labTag: UInt16, nodeToken: [UInt8]) -> Hello {
        Hello(flags: flags.union(.lab), psm: psm, groupTag: labGroupTag(labTag), nodeToken: nodeToken,
              epochLo: 0)
    }

    public static func labGroupTag(_ labTag: UInt16) -> [UInt8] {
        [UInt8(labTag & 0xFF), UInt8(labTag >> 8), 0, 0]
    }

    /// `lab_tag = SHA-256("cvic/lab" ‖ u16 lab_code LE)[0..2]` read as u16 LE (§4.3).
    public static func labTag(labCode: UInt16) -> UInt16 {
        var input = Data("cvic/lab".utf8)
        input.append(UInt8(labCode & 0xFF))
        input.append(UInt8(labCode >> 8))
        let digest = Array(SHA256.hash(data: input))
        return UInt16(digest[0]) | (UInt16(digest[1]) << 8)
    }

    public enum CodecError: Error, Equatable {
        case badLength(Int)
        case badMagic(UInt8)
        case badVersionRange
        case badField(String)
    }

    public func encode() throws(CodecError) -> Data {
        guard protoMin <= protoMax else { throw .badVersionRange }
        guard groupTag.count == 4 else { throw .badField("groupTag") }
        guard nodeToken.count == 4 else { throw .badField("nodeToken") }
        var o = Data(capacity: Hello.length)
        o.append(Hello.magic)
        o.append(protoMin)
        o.append(protoMax)
        o.append(flags.rawValue)
        o.append(UInt8(psm & 0xFF))
        o.append(UInt8(psm >> 8))
        o.append(contentsOf: groupTag)
        o.append(contentsOf: nodeToken)
        o.append(UInt8(epochLo & 0xFF))
        o.append(UInt8(epochLo >> 8))
        return o
    }

    public static func decode(_ data: Data) throws(CodecError) -> Hello {
        let b = [UInt8](data)
        guard b.count == length else { throw .badLength(b.count) }
        guard b[0] == magic else { throw .badMagic(b[0]) }
        guard b[1] <= b[2] else { throw .badVersionRange }
        return Hello(protoMin: b[1], protoMax: b[2], flags: Flags(rawValue: b[3]),
                     psm: UInt16(b[4]) | (UInt16(b[5]) << 8), groupTag: Array(b[6..<10]),
                     nodeToken: Array(b[10..<14]), epochLo: UInt16(b[14]) | (UInt16(b[15]) << 8))
    }
}
