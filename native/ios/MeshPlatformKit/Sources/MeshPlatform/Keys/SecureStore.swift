import Foundation
import Security

/// Keychain errors (OSStatus from Security).
public enum SecureStoreError: Error, Equatable, Sendable {
    case status(OSStatus)
    case badKey
}

/// Raw Keychain calls behind a protocol (tests can use a fake; the real one is also
/// exercised in the simulator).
public protocol KeychainPort: Sendable {
    func add(_ attrs: [String: Any]) -> OSStatus
    func update(_ query: [String: Any], _ attrs: [String: Any]) -> OSStatus
    func copyMatching(_ query: [String: Any]) -> (OSStatus, AnyObject?)
    func delete(_ query: [String: Any]) -> OSStatus
}

public struct SystemKeychain: KeychainPort {
    public init() {}

    public func add(_ attrs: [String: Any]) -> OSStatus {
        SecItemAdd(attrs as CFDictionary, nil)
    }

    public func update(_ query: [String: Any], _ attrs: [String: Any]) -> OSStatus {
        SecItemUpdate(query as CFDictionary, attrs as CFDictionary)
    }

    public func copyMatching(_ query: [String: Any]) -> (OSStatus, AnyObject?) {
        var out: AnyObject?
        let st = SecItemCopyMatching(query as CFDictionary, &out)
        return (st, out)
    }

    public func delete(_ query: [String: Any]) -> OSStatus {
        SecItemDelete(query as CFDictionary)
    }
}

/// Secrets store (`PlatformHooks.persist`, DESIGN §8): per-group identities and epoch
/// secrets as generic-password items, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`
/// (readable in the background after the first unlock, never synced or backed up).
/// Never `UserDefaults`, files, or anything the WebView can read.
public final class SecureStore: Sendable {
    public let service: String
    private let keychain: any KeychainPort

    public init(service: String = "com.convlink.voice.engine", keychain: any KeychainPort = SystemKeychain()) {
        self.service = service
        self.keychain = keychain
    }

    private func base(_ key: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
    }

    /// `persist(key, Some(value))`: insert or replace.
    public func set(_ key: String, _ value: Data) throws(SecureStoreError) {
        guard !key.isEmpty else { throw .badKey }
        let attrs: [String: Any] = [
            kSecValueData as String: value,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        let st = keychain.update(base(key), attrs)
        if st == errSecSuccess { return }
        guard st == errSecItemNotFound else { throw .status(st) }
        let add = keychain.add(base(key).merging(attrs) { $1 })
        guard add == errSecSuccess else { throw .status(add) }
    }

    public func get(_ key: String) throws(SecureStoreError) -> Data? {
        var q = base(key)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        let (st, out) = keychain.copyMatching(q)
        if st == errSecItemNotFound { return nil }
        guard st == errSecSuccess else { throw .status(st) }
        return out as? Data
    }

    /// `persist(key, None)`.
    public func delete(_ key: String) throws(SecureStoreError) {
        let st = keychain.delete(base(key))
        guard st == errSecSuccess || st == errSecItemNotFound else { throw .status(st) }
    }

    /// All keys of this service (sorted), e.g. to load stored groups at engine start.
    public func keys() throws(SecureStoreError) -> [String] {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecReturnAttributes as String: true,
            kSecMatchLimit as String: kSecMatchLimitAll,
        ]
        let (st, out) = keychain.copyMatching(q)
        if st == errSecItemNotFound { return [] }
        guard st == errSecSuccess else { throw .status(st) }
        let items = out as? [[String: Any]] ?? []
        return items.compactMap { $0[kSecAttrAccount as String] as? String }.sorted()
    }

    /// Removes every item of this service (reset / tests).
    public func removeAll() throws(SecureStoreError) {
        let st = keychain.delete([kSecClass as String: kSecClassGenericPassword,
                                  kSecAttrService as String: service])
        guard st == errSecSuccess || st == errSecItemNotFound else { throw .status(st) }
    }

    /// `PlatformHooks.persist(key, value)` in one call.
    public func persist(_ key: String, _ value: Data?) throws(SecureStoreError) {
        if let value { try set(key, value) } else { try delete(key) }
    }
}
