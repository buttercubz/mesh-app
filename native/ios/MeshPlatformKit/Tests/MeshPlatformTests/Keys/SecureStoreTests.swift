import Foundation
@testable import MeshPlatform
import Security
import Synchronization
import XCTest

/// In-memory Keychain with SecItem semantics for the fields SecureStore uses.
final class FakeKeychain: KeychainPort {
    struct Item { var data: Data; var accessible: String }
    let items = Mutex<[String: Item]>([:]) // "service|account"

    private func key(_ q: [String: Any]) -> String? {
        guard let s = q[kSecAttrService as String] as? String, let a = q[kSecAttrAccount as String] as? String else {
            return nil
        }
        return "\(s)|\(a)"
    }

    func add(_ attrs: [String: Any]) -> OSStatus {
        guard let k = key(attrs), let d = attrs[kSecValueData as String] as? Data else { return errSecParam }
        return items.withLock { m in
            if m[k] != nil { return errSecDuplicateItem }
            m[k] = Item(data: d, accessible: attrs[kSecAttrAccessible as String] as? String ?? "")
            return errSecSuccess
        }
    }

    func update(_ query: [String: Any], _ attrs: [String: Any]) -> OSStatus {
        guard let k = key(query), let d = attrs[kSecValueData as String] as? Data else { return errSecParam }
        return items.withLock { m in
            guard m[k] != nil else { return errSecItemNotFound }
            m[k] = Item(data: d, accessible: attrs[kSecAttrAccessible as String] as? String ?? "")
            return errSecSuccess
        }
    }

    func copyMatching(_ query: [String: Any]) -> (OSStatus, AnyObject?) {
        items.withLock { m in
            if let k = key(query) {
                guard let it = m[k] else { return (errSecItemNotFound, nil) }
                return (errSecSuccess, it.data as NSData)
            }
            let svc = query[kSecAttrService as String] as? String ?? ""
            let rows = m.keys.filter { $0.hasPrefix(svc + "|") }
                .map { [kSecAttrAccount as String: String($0.dropFirst(svc.count + 1))] as NSDictionary }
            return rows.isEmpty ? (errSecItemNotFound, nil) : (errSecSuccess, rows as NSArray)
        }
    }

    func delete(_ query: [String: Any]) -> OSStatus {
        items.withLock { m in
            if let k = key(query) { return m.removeValue(forKey: k) == nil ? errSecItemNotFound : errSecSuccess }
            let svc = query[kSecAttrService as String] as? String ?? ""
            let before = m.count
            m = m.filter { !$0.key.hasPrefix(svc + "|") }
            return before == m.count ? errSecItemNotFound : errSecSuccess
        }
    }
}

final class SecureStoreTests: XCTestCase {
    private func exercise(_ s: SecureStore) throws {
        try s.removeAll()
        XCTAssertNil(try s.get("group/a"))
        try s.set("group/a", Data([1, 2, 3]))
        try s.set("group/b", Data([4]))
        XCTAssertEqual(try s.get("group/a"), Data([1, 2, 3]))
        try s.set("group/a", Data([9])) // replace
        XCTAssertEqual(try s.get("group/a"), Data([9]))
        XCTAssertEqual(try s.keys(), ["group/a", "group/b"])
        try s.persist("group/b", nil)
        XCTAssertNil(try s.get("group/b"))
        try s.delete("missing") // idempotent
        XCTAssertThrowsError(try s.set("", Data([1])))
        try s.removeAll()
        XCTAssertEqual(try s.keys(), [])
    }

    func testLogicWithFakeKeychain() throws {
        let fake = FakeKeychain()
        try exercise(SecureStore(service: "test.fake", keychain: fake))
        let s = SecureStore(service: "test.fake", keychain: fake)
        try s.set("k", Data([1]))
        let acc = fake.items.withLock { $0["test.fake|k"]?.accessible }
        XCTAssertEqual(acc, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String,
                       "readable after first unlock, never synced")
    }

    /// The real Keychain of the simulator.
    func testRealKeychainInTheSimulator() throws {
        let s = SecureStore(service: "com.convlink.voice.tests.\(UUID().uuidString)")
        do {
            try s.set("probe", Data([0]))
        } catch SecureStoreError.status(let st) where st == errSecMissingEntitlement {
            throw XCTSkip("this test host has no keychain entitlement (errSecMissingEntitlement)")
        }
        try exercise(s)
    }

    func testErrorsAreSurfaced() {
        final class Broken: KeychainPort {
            func add(_ attrs: [String: Any]) -> OSStatus { errSecInteractionNotAllowed }
            func update(_ q: [String: Any], _ a: [String: Any]) -> OSStatus { errSecItemNotFound }
            func copyMatching(_ q: [String: Any]) -> (OSStatus, AnyObject?) { (errSecInteractionNotAllowed, nil) }
            func delete(_ q: [String: Any]) -> OSStatus { errSecInteractionNotAllowed }
        }
        let s = SecureStore(service: "x", keychain: Broken())
        XCTAssertThrowsError(try s.set("k", Data())) {
            XCTAssertEqual($0 as? SecureStoreError, .status(errSecInteractionNotAllowed))
        }
        XCTAssertThrowsError(try s.get("k"))
        XCTAssertThrowsError(try s.delete("k"))
        XCTAssertThrowsError(try s.keys())
    }
}
