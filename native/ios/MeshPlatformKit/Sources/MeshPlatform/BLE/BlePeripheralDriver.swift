import CoreBluetooth
import Foundation

/// BLE peripheral role (DESIGN §9.2-§9.4): publishes `MESH_SVC` (HELLO read, RX write
/// without response, TX notify; all unencrypted so iOS never shows a pairing dialog), an
/// insecure L2CAP channel, and the foreground advertisement (UUID + 6-char local name).
/// The HELLO value is whatever the core last sent with `SetHello`.
///
/// Threading: CoreBluetooth objects and the state below are confined to `q`.
public final class BlePeripheralDriver: NSObject, LinkDriver, @unchecked Sendable {
    public typealias PsmListener = @Sendable (UInt16?) -> Void

    let constants: BleConstants
    let registry: LinkRegistry
    let io: LinkIOThread
    let radio: RadioStateMonitor?
    let q = BleQueue(label: "mesh.ble.peripheral")

    // Confined to `q`.
    var manager: CBPeripheralManager?
    var hello = Data()
    var helloCharacteristic: CBMutableCharacteristic?
    var txCharacteristic: CBMutableCharacteristic?
    var serviceAdded = false
    var addingService = false
    var serviceBackoff = RepublishBackoff()
    /// L2CAP listener lifecycle; its `psm` is what the core puts in HELLO.
    var listener = L2capListenerState()
    var psm: UInt16? { listener.psm }
    var mode: DiscoveryMode = .off
    var adv: AdvConfig?
    var gattByCentral: [UUID: GattLink] = [:]
    var l2capLinks: [UInt32: L2capLink] = [:]
    var psmListener: PsmListener?
    var diag: BleDiagSink?
    /// Last PSM reported to `psmListener` (reports only on change).
    var reportedPsm: UInt16??

    public init(constants: BleConstants, registry: LinkRegistry, io: LinkIOThread,
                radio: RadioStateMonitor? = nil,
                restore: Bool = BlePeripheralDriver.hasBackgroundMode) {
        self.constants = constants
        self.registry = registry
        self.io = io
        self.radio = radio
        super.init()
        var opts: [String: Any] = [CBPeripheralManagerOptionShowPowerAlertKey: false]
        if restore {
            opts[CBPeripheralManagerOptionRestoreIdentifierKey] = constants.peripheralRestoreId
        }
        q.sync { manager = CBPeripheralManager(delegate: self, queue: q.queue, options: opts) }
    }

    public static var hasBackgroundMode: Bool {
        let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String] ?? []
        return modes.contains("bluetooth-peripheral")
    }

    /// Called with the published PSM (nil when unpublished): the core puts it in HELLO.
    public func onPsm(_ l: PsmListener?) {
        q.async { [self] in
            psmListener = l
            reportedPsm = .some(psm)
            l?(psm)
        }
    }

    /// Subscribes to driver events (nil unsubscribes). The sink runs on the driver queue.
    public func onDiagnostics(_ sink: BleDiagSink?) {
        q.async { [self] in diag = sink }
    }

    func emit(_ e: BleDiagEvent) { diag?(e) }

    /// Reports the PSM to the core when it changed: HELLO advertises a PSM only while
    /// CoreBluetooth confirms a live listener (0 / nil makes peers use GATT).
    func reportPsm() {
        if case .some(let last) = reportedPsm, last == psm { return }
        reportedPsm = .some(psm)
        psmListener?(psm)
    }

    public func apply(_ cmd: RadioCommand) {
        q.async { [self] in handle(cmd) }
    }

    func handle(_ cmd: RadioCommand) {
        switch cmd {
        case .setHello(let bytes):
            hello = bytes
        case .setDiscovery(let m, let a):
            mode = m
            adv = a
            applyAdvertising()
        case .close(let link, let reason):
            if let l = l2capLinks.removeValue(forKey: link) {
                l.close(reason: reason)
            } else if let (central, g) = gattByCentral.first(where: { $0.value.id == link }) {
                gattByCentral[central] = nil
                g.down(.localClose)
            }
        case .connect, .openLink, .abort, .setLinkParams, .armReconnect, .awareUpgrade, .tunnelOpen:
            break // central driver / WP-24 / v2
        }
    }

    func applyAdvertising() {
        guard let manager, manager.state == .poweredOn, serviceAdded else { return }
        if manager.isAdvertising { manager.stopAdvertising() }
        guard mode != .off, let adv, adv.connectable else {
            emit(.advertising(mode: mode, on: false))
            return
        }
        manager.startAdvertising(AdvBuilder.iosAdvertisement(service: constants.meshService, adv: adv))
        emit(.advertising(mode: mode, on: true))
    }

    func publishService() {
        guard let manager, manager.state == .poweredOn, !serviceAdded, !addingService else { return }
        let helloC = CBMutableCharacteristic(type: constants.hello, properties: [.read], value: nil,
                                             permissions: [.readable])
        let rxC = CBMutableCharacteristic(type: constants.rx, properties: [.writeWithoutResponse],
                                          value: nil, permissions: [.writeable])
        let txC = CBMutableCharacteristic(type: constants.tx, properties: [.notify], value: nil,
                                          permissions: [])
        let svc = CBMutableService(type: constants.meshService, primary: true)
        svc.characteristics = [helloC, rxC, txC]
        helloCharacteristic = helloC
        txCharacteristic = txC
        addingService = true
        manager.removeAllServices() // a previous registration may survive a radio cycle
        manager.add(svc)
        ensureListener()
    }

    /// Publishes the insecure L2CAP listener unless one is live or pending.
    func ensureListener() {
        guard let manager, manager.state == .poweredOn else { return }
        if listener.wantListener() == .publish {
            manager.publishL2CAPChannel(withEncryption: false)
        }
    }

    func run(_ action: L2capListenerState.Action) {
        switch action {
        case .publish:
            manager?.publishL2CAPChannel(withEncryption: false)
        case .retry(let after, let generation):
            emit(.l2capListener(psm: nil, error: nil, retryIn: after))
            q.queue.asyncAfter(deadline: .now() + after) { [weak self] in
                guard let self else { return }
                if manager?.state == .poweredOn { run(listener.retryFired(generation: generation)) }
            }
        case .none:
            break
        }
    }

    func dropAll(_ reason: LinkDownReason) {
        for (_, g) in gattByCentral { g.down(reason) }
        gattByCentral.removeAll()
        for (_, l) in l2capLinks {
            registry.down(l.id, reason: reason)
            l.close(reason: 0)
        }
        l2capLinks.removeAll()
        serviceAdded = false
        addingService = false
        serviceBackoff.reset()
        listener.radioLost()
        reportPsm()
    }
}

extension BlePeripheralDriver: CBPeripheralManagerDelegate {
    public func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
        radio?.update(.peripheral, peripheral.state)
        emit(.radioState(role: .peripheral, state: peripheral.state.rawValue))
        switch peripheral.state {
        case .poweredOn:
            publishService()
        case .poweredOff:
            dropAll(.radioOff)
        case .unauthorized, .unsupported:
            dropAll(.error(-1))
        default:
            break
        }
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, willRestoreState dict: [String: Any]) {
        // Restore identifiers are set but not relied upon (§10.1.3); the service is
        // re-published on poweredOn.
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService,
                                  error: (any Error)?) {
        addingService = false
        serviceAdded = error == nil
        guard error != nil else {
            serviceBackoff.succeeded()
            emit(.serviceAdded(ok: true, error: nil, retryIn: nil))
            applyAdvertising()
            return
        }
        let delay = serviceBackoff.failed()
        let generation = serviceBackoff.generation
        emit(.serviceAdded(ok: false, error: BleErrorInfo(error), retryIn: delay))
        q.queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, serviceBackoff.generation == generation else { return }
            publishService()
        }
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didPublishL2CAPChannel PSM: CBL2CAPPSM,
                                  error: (any Error)?) {
        let action = listener.published(psm: UInt16(PSM), failed: error != nil)
        if action == .none {
            emit(.l2capListener(psm: psm, error: nil, retryIn: nil))
        } else {
            emit(.l2capListener(psm: nil, error: BleErrorInfo(error), retryIn: nil))
        }
        reportPsm()
        run(action)
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didUnpublishL2CAPChannel PSM: CBL2CAPPSM,
                                  error: (any Error)?) {
        // Never requested by this driver: the listener died (e.g. Bluetooth daemon reset).
        guard UInt16(PSM) == psm || psm == nil else { return }
        emit(.l2capListener(psm: nil, error: BleErrorInfo(error), retryIn: nil))
        let action = listener.unpublished()
        reportPsm()
        run(action)
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
        guard request.characteristic.uuid == constants.hello else {
            peripheral.respond(to: request, withResult: .attributeNotFound)
            return
        }
        guard request.offset <= hello.count else {
            peripheral.respond(to: request, withResult: .invalidOffset)
            return
        }
        request.value = hello.subdata(in: request.offset..<hello.count)
        peripheral.respond(to: request, withResult: .success)
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
        for r in requests where r.characteristic.uuid == constants.rx {
            if let v = r.value { gattByCentral[r.central.identifier]?.received(v) }
        }
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                                  didSubscribeTo characteristic: CBCharacteristic) {
        guard characteristic.uuid == constants.tx, gattByCentral[central.identifier] == nil,
              let tx = txCharacteristic else { return }
        let port = PeripheralGattPort(q: q, manager: peripheral, central: central, tx: tx)
        let link = GattLink(id: registry.allocateId(), role: .responder, port: port, registry: registry)
        gattByCentral[central.identifier] = link
        emit(.linkUp(link: link.id, discId: nil, kind: .bleGatt, role: .responder,
                     maxFrame: link.maxFrame, mtu: UInt16(clamping: central.maximumUpdateValueLength + 3)))
        registry.up(link, info: LinkInfo(id: link.id, kind: .bleGatt, role: .responder,
                                         maxFrame: link.maxFrame, unit: .message,
                                         peerPlatformHint: .unknown))
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                                  didUnsubscribeFrom characteristic: CBCharacteristic) {
        guard characteristic.uuid == constants.tx,
              let g = gattByCentral.removeValue(forKey: central.identifier) else { return }
        emit(.linkDown(link: g.id, reason: .peerClosed, error: nil))
        g.down(.peerClosed)
    }

    public func peripheralManagerIsReady(toUpdateSubscribers peripheral: CBPeripheralManager) {
        for g in gattByCentral.values { g.readyToSend() }
    }

    public func peripheralManager(_ peripheral: CBPeripheralManager, didOpen channel: CBL2CAPChannel?,
                                  error: (any Error)?) {
        guard error == nil, let channel, let input = channel.inputStream,
              let output = channel.outputStream else {
            emit(.l2capListener(psm: psm, error: BleErrorInfo(error) ?? BleErrorInfo(domain: "open", code: -1),
                                retryIn: nil))
            return
        }
        let link = L2capLink(id: registry.allocateId(), input: input, output: output, io: io,
                             registry: registry, channel: channel)
        l2capLinks = l2capLinks.filter { registry.link($0.key) != nil } // drop closed links
        l2capLinks[link.id] = link
        link.open()
        emit(.linkUp(link: link.id, discId: nil, kind: .bleL2cap, role: .responder,
                     maxFrame: constants.l2capMaxFrame, mtu: nil))
        registry.up(link, info: LinkInfo(id: link.id, kind: .bleL2cap, role: .responder,
                                         maxFrame: constants.l2capMaxFrame, unit: .stream,
                                         peerPlatformHint: .unknown))
    }
}
