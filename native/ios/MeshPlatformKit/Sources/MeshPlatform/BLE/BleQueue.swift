import CoreBluetooth
import Foundation

/// The serial queue that owns a driver's CoreBluetooth objects (Swift conventions:
/// CoreBluetooth runs on a dedicated serial queue). `sync` runs inline when already on
/// the queue, so a sink that calls back into a link from a CB callback cannot deadlock.
public final class BleQueue: Sendable {
    public let queue: DispatchQueue
    private let key = DispatchSpecificKey<UInt8>()

    public init(label: String) {
        queue = DispatchQueue(label: label, qos: .userInitiated)
        queue.setSpecific(key: key, value: 1)
    }

    public var isCurrent: Bool { DispatchQueue.getSpecific(key: key) != nil }

    public func sync<T>(_ body: () throws -> T) rethrows -> T {
        if isCurrent { return try body() }
        return try queue.sync(execute: body)
    }

    public func async(_ body: @escaping @Sendable () -> Void) {
        queue.async(execute: body)
    }
}

/// The CoreBluetooth calls a central GATT data path needs (a `CBPeripheral` plus its `RX`
/// characteristic in production, a fake in tests). Called on the driver queue only.
protocol GattWriteTarget: AnyObject {
    var isConnected: Bool { get }
    var canSendWriteWithoutResponse: Bool { get }
    /// `maximumWriteValueLength(for: .withoutResponse)` (ATT MTU - 3).
    var maxWriteWithoutResponse: Int { get }
    func writeWithoutResponse(_ data: Data)
    func cancelConnection()
}

/// Production target: write-without-response on the peer's `RX`.
final class PeripheralRxTarget: GattWriteTarget {
    private let central: CBCentralManager
    private let peripheral: CBPeripheral
    private let rx: CBCharacteristic

    init(central: CBCentralManager, peripheral: CBPeripheral, rx: CBCharacteristic) {
        self.central = central
        self.peripheral = peripheral
        self.rx = rx
    }

    var isConnected: Bool { peripheral.state == .connected }
    var canSendWriteWithoutResponse: Bool { peripheral.canSendWriteWithoutResponse }
    var maxWriteWithoutResponse: Int { peripheral.maximumWriteValueLength(for: .withoutResponse) }
    func writeWithoutResponse(_ data: Data) { peripheral.writeValue(data, for: rx, type: .withoutResponse) }
    func cancelConnection() { central.cancelPeripheralConnection(peripheral) }
}

/// Central side of a GATT link: write-without-response on the peer's `RX`, one ATT value
/// per `send`. The unit size is fixed when the link opens (`maximumWriteValueLength`, at
/// least the ATT minimum of 20 B) and handed to the core as `maxFrame`; the core chunks
/// frames to it (§4.2). A larger unit would be truncated by CoreBluetooth, so it is
/// dropped and reported instead.
final class CentralGattPort: GattTxPort, @unchecked Sendable {
    // The target is only touched on `q` (every access goes through q.sync / q.async).
    private let q: BleQueue
    private let target: any GattWriteTarget
    private let unit: Int
    private let diag: (@Sendable (Int, Int) -> Void)?

    init(q: BleQueue, target: any GattWriteTarget, diag: (@Sendable (Int, Int) -> Void)? = nil) {
        self.q = q
        self.target = target
        self.diag = diag
        unit = q.sync { max(20, target.maxWriteWithoutResponse) }
    }

    func trySend(_ data: Data) -> Bool {
        if data.count > unit {
            diag?(data.count, unit)
            return true // dropped: never retried, the core's GATT reassembly discards the frame
        }
        return q.sync {
            guard target.isConnected, target.canSendWriteWithoutResponse else { return false }
            target.writeWithoutResponse(data)
            return true
        }
    }

    var maxUnit: Int { unit }

    func shutdown() {
        q.async { [self] in target.cancelConnection() }
    }
}

/// Peripheral side of a GATT link: notifications on our `TX` to one subscribed central.
final class PeripheralGattPort: GattTxPort, @unchecked Sendable {
    // CoreBluetooth objects are only touched on `q`.
    private let q: BleQueue
    private let manager: CBPeripheralManager
    private let central: CBCentral
    private let tx: CBMutableCharacteristic

    init(q: BleQueue, manager: CBPeripheralManager, central: CBCentral, tx: CBMutableCharacteristic) {
        self.q = q
        self.manager = manager
        self.central = central
        self.tx = tx
    }

    func trySend(_ data: Data) -> Bool {
        q.sync { manager.updateValue(data, for: tx, onSubscribedCentrals: [central]) }
    }

    var maxUnit: Int {
        q.sync { central.maximumUpdateValueLength }
    }

    func shutdown() {
        // A peripheral cannot disconnect a central; the core stops sending and the central
        // sees the link die (4 s without frames) or unsubscribes.
    }
}
