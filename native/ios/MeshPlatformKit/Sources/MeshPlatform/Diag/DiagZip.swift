import Foundation

/// A minimal ZIP writer (stored entries, CRC-32) for diagnostics exports
/// (`convlink-diag-*.zip`: `diag/<sessionId>/seg-NNNN.jsonl` + `device.json`). Read by
/// `tools/lab-analyze` (`lab-analyze ride <zip>`). Stored, not deflated: a 2 h ride is
/// about 10 MB and the share sheet compresses nothing anyway.
public enum DiagZip {
    public struct Entry: Sendable {
        public var name: String
        public var data: Data

        public init(name: String, data: Data) {
            self.name = name
            self.data = data
        }
    }

    private static let table: [UInt32] = (0..<256).map { n -> UInt32 in
        var c = UInt32(n)
        for _ in 0..<8 { c = (c & 1) != 0 ? 0xEDB8_8320 ^ (c >> 1) : c >> 1 }
        return c
    }

    /// CRC-32 (IEEE 802.3) of `data`.
    public static func crc32(_ data: Data) -> UInt32 {
        var c: UInt32 = 0xFFFF_FFFF
        for b in data { c = table[Int((c ^ UInt32(b)) & 0xFF)] ^ (c >> 8) }
        return c ^ 0xFFFF_FFFF
    }

    /// The ZIP archive of `entries` (names in UTF-8, no directories needed).
    public static func archive(_ entries: [Entry]) -> Data {
        var out = Data()
        var central = Data()
        for e in entries {
            let name = Data(e.name.utf8)
            let crc = crc32(e.data)
            let size = UInt32(clamping: e.data.count)
            let offset = UInt32(clamping: out.count)
            out.le32(0x0403_4B50)
            out.le16(20); out.le16(0x0800); out.le16(0)  // version, UTF-8 flag, stored
            out.le16(0); out.le16(0x21)  // time, date (1980-01-01)
            out.le32(crc); out.le32(size); out.le32(size)
            out.le16(UInt16(clamping: name.count)); out.le16(0)
            out.append(name)
            out.append(e.data)
            central.le32(0x0201_4B50)
            central.le16(20); central.le16(20); central.le16(0x0800); central.le16(0)
            central.le16(0); central.le16(0x21)
            central.le32(crc); central.le32(size); central.le32(size)
            central.le16(UInt16(clamping: name.count)); central.le16(0); central.le16(0)
            central.le16(0); central.le16(0); central.le32(0)
            central.le32(offset)
            central.append(name)
        }
        let cdOffset = UInt32(clamping: out.count)
        out.append(central)
        out.le32(0x0605_4B50)
        out.le16(0); out.le16(0)
        out.le16(UInt16(clamping: entries.count)); out.le16(UInt16(clamping: entries.count))
        out.le32(UInt32(clamping: central.count)); out.le32(cdOffset)
        out.le16(0)
        return out
    }

    /// Entries of a stored-only archive written by `archive` (tests, round trips).
    public static func entries(_ zip: Data) -> [Entry] {
        var out: [Entry] = []
        var p = 0
        let bytes = [UInt8](zip)
        func u16(_ i: Int) -> Int { Int(bytes[i]) | Int(bytes[i + 1]) << 8 }
        func u32(_ i: Int) -> Int { u16(i) | u16(i + 2) << 16 }
        while p + 30 <= bytes.count, u32(p) == 0x0403_4B50 {
            let size = u32(p + 18)
            let nameLen = u16(p + 26)
            let extra = u16(p + 28)
            let nameStart = p + 30
            let dataStart = nameStart + nameLen + extra
            guard dataStart + size <= bytes.count else { break }
            let name = String(decoding: bytes[nameStart..<nameStart + nameLen], as: UTF8.self)
            out.append(Entry(name: name, data: Data(bytes[dataStart..<dataStart + size])))
            p = dataStart + size
        }
        return out
    }
}

extension Data {
    fileprivate mutating func le16(_ v: UInt16) {
        append(UInt8(v & 0xFF))
        append(UInt8(v >> 8))
    }

    fileprivate mutating func le32(_ v: UInt32) {
        le16(UInt16(v & 0xFFFF))
        le16(UInt16(v >> 16))
    }
}
