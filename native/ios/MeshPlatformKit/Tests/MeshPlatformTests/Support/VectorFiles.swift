import Foundation
import XCTest

/// Loads the golden vectors committed by WP-04 (`test-vectors/wire/`, contract C2). The
/// simulator test process reads the host file system, so the repository copy is used
/// directly: no duplicated fixtures that could drift.
enum VectorFiles {
    static var repoRoot: URL {
        var u = URL(fileURLWithPath: #filePath)
        for _ in 0..<7 { u.deleteLastPathComponent() } // file, Support, MeshPlatformTests, Tests, MeshPlatformKit, ios, native
        return u
    }

    struct Vector {
        var name: String
        var expected: String
        var inputs: [String: Any]
    }

    static func load(_ file: String) throws -> [Vector] {
        let url = repoRoot.appending(path: "test-vectors/wire/\(file)")
        let data = try Data(contentsOf: url)
        let root = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let list = try XCTUnwrap(root["vectors"] as? [[String: Any]])
        return try list.map { v in
            Vector(name: try XCTUnwrap(v["name"] as? String),
                   expected: try XCTUnwrap(v["expected"] as? String),
                   inputs: v["inputs"] as? [String: Any] ?? [:])
        }
    }
}

func hex(_ d: Data) -> String {
    d.map { String(format: "%02x", $0) }.joined()
}

func bytes(hex s: String) -> [UInt8] {
    var out: [UInt8] = []
    var it = s.makeIterator()
    while let a = it.next(), let b = it.next() {
        out.append(UInt8(String([a, b]), radix: 16) ?? 0)
    }
    return out
}

extension Dictionary where Key == String, Value == Any {
    func int(_ k: String) -> Int? { (self[k] as? NSNumber)?.intValue }
    func str(_ k: String) -> String? { self[k] as? String }
}
