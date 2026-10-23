import CoreBluetooth
import Foundation

/// GATT UUIDs and BLE timings. The production values come from the core's
/// `ble_constants()` (single source of truth, DESIGN §3.5) and are passed in by
/// MeshEngineKit; `BleConstants.design` holds the DESIGN §9.3/§9.5 values for Lab use and
/// tests, and must stay equal to `ble_constants()` (checked by MeshEngineKit's tests).
public struct BleConstants: Sendable, Equatable {
    public var meshServiceId: UUID
    public var helloId: UUID
    public var rxId: UUID
    public var txId: UUID
    /// iOS `connect` never times out by itself; native cancels after this (§9.5).
    public var connectTimeout: TimeInterval
    /// Restore identifiers (set, not relied upon; §10.1.3).
    public var centralRestoreId: String
    public var peripheralRestoreId: String
    /// L2CAP stream frame ceiling handed to the core as `max_frame` (len u16 + 1024 B).
    public var l2capMaxFrame: UInt16

    public init(meshServiceId: UUID, helloId: UUID, rxId: UUID, txId: UUID,
                connectTimeout: TimeInterval, centralRestoreId: String,
                peripheralRestoreId: String, l2capMaxFrame: UInt16) {
        self.meshServiceId = meshServiceId
        self.helloId = helloId
        self.rxId = rxId
        self.txId = txId
        self.connectTimeout = connectTimeout
        self.centralRestoreId = centralRestoreId
        self.peripheralRestoreId = peripheralRestoreId
        self.l2capMaxFrame = l2capMaxFrame
    }

    // CBUUID is not Sendable, so the stored values are Foundation UUIDs.
    public var meshService: CBUUID { CBUUID(nsuuid: meshServiceId) }
    public var hello: CBUUID { CBUUID(nsuuid: helloId) }
    public var rx: CBUUID { CBUUID(nsuuid: rxId) }
    public var tx: CBUUID { CBUUID(nsuuid: txId) }

    public static let design = BleConstants(
        meshServiceId: UUID(uuid: (0x31, 0xB8, 0xCD, 0x85, 0x9D, 0x3A, 0x46, 0x22,
                                   0xA4, 0x65, 0x3C, 0xDC, 0xA2, 0xC3, 0x2D, 0xC5)),
        helloId: UUID(uuid: (0x86, 0x84, 0x18, 0x42, 0xA7, 0xC3, 0x48, 0x44,
                             0x84, 0xD2, 0x5B, 0x28, 0x1A, 0x3F, 0x61, 0x6C)),
        rxId: UUID(uuid: (0x14, 0xE9, 0x19, 0x43, 0x76, 0x73, 0x44, 0x0A,
                          0x97, 0x9A, 0xF8, 0x5B, 0x4D, 0xBD, 0x4D, 0xBF)),
        txId: UUID(uuid: (0xA7, 0x6E, 0x31, 0x68, 0xB4, 0xF6, 0x4B, 0xE1,
                          0x82, 0x52, 0x35, 0xAB, 0x8A, 0xF1, 0x73, 0x02)),
        connectTimeout: 8.0,
        centralRestoreId: "cvic.central",
        peripheralRestoreId: "cvic.peripheral",
        l2capMaxFrame: 2 + 1024
    )
}

extension CBUUID {
    /// Canonical (big-endian) 16 bytes of a 128-bit UUID.
    var canonicalBytes: Data {
        data.count == 16 ? data : Data(CBUUID(string: uuidString).data)
    }

    /// Little-endian bytes as they appear inside AD structures and ATT (§9.4).
    var littleEndianBytes: Data {
        Data(canonicalBytes.reversed())
    }
}
