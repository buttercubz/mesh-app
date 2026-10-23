import CoreBluetooth
import Foundation

// CoreBluetooth callbacks of the central driver; they run on `q.queue`.

extension BleCentralDriver: CBCentralManagerDelegate {
    public func centralManagerDidUpdateState(_ central: CBCentralManager) {
        radio?.update(.central, central.state)
        emit(.radioState(role: .central, state: central.state.rawValue))
        if central.state == .poweredOn {
            lastScan = nil
            applyScan()
            return
        }
        guard central.state == .poweredOff || central.state == .unauthorized else { return }
        // Radio gone: every BLE link of this driver is down with RadioOff (§3.2).
        let reason: LinkDownReason = central.state == .poweredOff ? .radioOff : .error(-1)
        for (peer, link) in linksByPeer {
            registry.down(link.id, reason: reason)
            gattByPeer[peer]?.down(reason)
            emit(.linkDown(link: link.id, reason: reason, error: nil))
        }
        linksByPeer.removeAll()
        gattByPeer.removeAll()
        openVia.removeAll()
        deferredConnect.removeAll()
        lastScan = nil
        if let c = tracker.current { tracker.finish(c.discId) }
    }

    public func centralManager(_ central: CBCentralManager, willRestoreState dict: [String: Any]) {
        // Restore identifiers are set but not relied upon (§10.1.3): peripherals restored
        // here are re-discovered by scanning and re-connected only on the core's command.
        let restored = dict[CBCentralManagerRestoredStatePeripheralsKey] as? [CBPeripheral] ?? []
        for p in restored { peripherals[p.identifier] = p }
    }

    public func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                               advertisementData: [String: Any], rssi RSSI: NSNumber) {
        peripherals[peripheral.identifier] = peripheral
        let id = discId(for: peripheral)
        if tracker.isIgnored(id, now: clock()) { return }
        let parsed = DiscoveryParser.parse(advertisement: advertisementData,
                                           service: constants.meshService)
        if parsed.platform != .unknown { platformByPeer[peripheral.identifier] = parsed.platform }
        registry.discovered(Discovery(discId: id, transport: .ble,
                                      rssi: DiscoveryParser.rssi(RSSI),
                                      tokens: parsed.tokens, platformHint: parsed.platform))
    }

    public func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        let id = discId(for: peripheral)
        tracker.finish(id)
        emit(.connectResult(discId: id, ok: true, timedOut: false, error: nil))
        peripheral.delegate = self
        peripheral.discoverServices([constants.meshService])
    }

    public func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral,
                               error: (any Error)?) {
        let id = discId(for: peripheral)
        tracker.finish(id)
        emit(.connectResult(discId: id, ok: false, timedOut: false, error: BleErrorInfo(error)))
        // No link exists yet, so this is reported as a failed connect (LimitReached included).
        registry.connectFailed(id, reason: RadioStateMonitor.downReason(for: error, cancelledLocally: false))
        reconnectIfArmed(peripheral)
    }

    public func centralManager(_ central: CBCentralManager,
                               didDisconnectPeripheral peripheral: CBPeripheral,
                               timestamp: CFAbsoluteTime, isReconnecting: Bool,
                               error: (any Error)?) {
        let peer = peripheral.identifier
        let local = cancelled.remove(peer) != nil
        let reason = RadioStateMonitor.downReason(for: error, cancelledLocally: local)
        let id = discId(for: peripheral)
        emit(.disconnected(discId: id, local: local, reconnecting: isReconnecting, error: BleErrorInfo(error)))
        if let link = linksByPeer.removeValue(forKey: peer) {
            emit(.linkDown(link: link.id, reason: reason, error: BleErrorInfo(error)))
            if let g = gattByPeer.removeValue(forKey: peer) {
                g.down(reason)
            } else {
                registry.down(link.id, reason: reason)
                link.close(reason: 0)
            }
        }
        helloChar[peer] = nil
        rxChar[peer] = nil
        txChar[peer] = nil
        openVia[peer] = nil
        tracker.finish(id)
        if deferredConnect.remove(peer) != nil, !isReconnecting {
            connect(id, now: clock())
        } else if !isReconnecting && !local {
            reconnectIfArmed(peripheral)
        }
        applyScan()
    }

    /// A GATT setup step failed before a link existed: report it so the core does not
    /// wait for its connect timeout, and drop the connection.
    func setupFailed(_ peripheral: CBPeripheral, step: String, error: (any Error)?) {
        let id = discId(for: peripheral)
        emit(.setupFailed(discId: id, step: step, error: BleErrorInfo(error)))
        registry.connectFailed(id, reason: .error(Int32(clamping: BleErrorInfo(error)?.code ?? -3)))
        cancelled.insert(peripheral.identifier)
        central?.cancelPeripheralConnection(peripheral)
    }

    func reconnectIfArmed(_ p: CBPeripheral) {
        let id = discId(for: p)
        let now = clock()
        if tracker.isArmed(id, now: now) { connect(id, now: now) }
    }
}

extension BleCentralDriver: CBPeripheralDelegate {
    public func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: (any Error)?) {
        guard error == nil,
              let svc = peripheral.services?.first(where: { $0.uuid == constants.meshService }) else {
            setupFailed(peripheral, step: "services", error: error)
            return
        }
        peripheral.discoverCharacteristics([constants.hello, constants.rx, constants.tx], for: svc)
    }

    public func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService,
                           error: (any Error)?) {
        let peer = peripheral.identifier
        for c in service.characteristics ?? [] {
            switch c.uuid {
            case constants.hello: helloChar[peer] = c
            case constants.rx: rxChar[peer] = c
            case constants.tx: txChar[peer] = c
            default: break
            }
        }
        guard error == nil, let hello = helloChar[peer] else {
            setupFailed(peripheral, step: "characteristics", error: error)
            return
        }
        peripheral.readValue(for: hello)
    }

    public func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic,
                           error: (any Error)?) {
        let peer = peripheral.identifier
        if characteristic.uuid == constants.hello {
            guard error == nil, let value = characteristic.value else {
                setupFailed(peripheral, step: "hello", error: error)
                return
            }
            let id = discId(for: peripheral)
            emit(.helloRead(discId: id, bytes: value.count))
            registry.helloRead(id, value)
        } else if characteristic.uuid == constants.tx, error == nil, let value = characteristic.value {
            gattByPeer[peer]?.received(value)
        }
    }

    /// GATT data path, step 2: `TX` notifications are on, so the link is usable both ways
    /// (inbound notifications on `TX`, outbound write-without-response on `RX`).
    public func peripheral(_ peripheral: CBPeripheral,
                           didUpdateNotificationStateFor characteristic: CBCharacteristic,
                           error: (any Error)?) {
        let peer = peripheral.identifier
        guard characteristic.uuid == constants.tx, case .gatt = openVia[peer], gattByPeer[peer] == nil
        else { return }
        let id = discId(for: peripheral)
        guard error == nil, characteristic.isNotifying, let rx = rxChar[peer], let central else {
            openFailed(peer, discId: id, via: .gatt, error: error)
            return
        }
        let linkId = registry.allocateId()
        let port = CentralGattPort(
            q: q, target: PeripheralRxTarget(central: central, peripheral: peripheral, rx: rx),
            diag: { [weak self] bytes, max in
                guard let driver = self else { return }
                driver.q.async { driver.emit(.oversizeUnit(link: linkId, bytes: bytes, max: max)) }
            })
        let link = GattLink(id: linkId, role: .initiator, port: port, registry: registry)
        gattByPeer[peer] = link
        linksByPeer[peer] = link
        armTimer()
        linkUp(link, peripheral: peripheral, kind: .bleGatt, maxFrame: link.maxFrame, unit: .message)
    }

    /// GATT data path is the fallback when L2CAP fails: an open failure is reported to the
    /// core as `connectFailed` (see `openFailed`), and the core decides whether the next
    /// attempt to this peer uses GATT. The driver never switches transport on its own.
    public func peripheral(_ peripheral: CBPeripheral, didOpen channel: CBL2CAPChannel?,
                           error: (any Error)?) {
        let peer = peripheral.identifier
        guard case .l2cap = openVia[peer] else { return }
        guard error == nil, let channel, let input = channel.inputStream,
              let output = channel.outputStream else {
            if case .l2cap(let psm) = openVia[peer] {
                openFailed(peer, discId: discId(for: peripheral), via: .l2cap(psm: psm), error: error)
            }
            return
        }
        let link = L2capLink(id: registry.allocateId(), input: input, output: output, io: io,
                             registry: registry, channel: channel)
        linksByPeer[peer] = link
        link.open()
        armTimer()
        linkUp(link, peripheral: peripheral, kind: .bleL2cap, maxFrame: constants.l2capMaxFrame,
               unit: .stream)
    }

    func linkUp(_ link: any MeshLink, peripheral: CBPeripheral, kind: LinkKind, maxFrame: UInt16,
                unit: LinkUnit) {
        let peer = peripheral.identifier
        let id = discId(for: peripheral)
        let mtu = UInt16(clamping: peripheral.maximumWriteValueLength(for: .withoutResponse) + 3)
        emit(.linkUp(link: link.id, discId: id, kind: kind, role: .initiator, maxFrame: maxFrame, mtu: mtu))
        registry.up(link, info: LinkInfo(id: link.id, kind: kind, role: .initiator,
                                         maxFrame: maxFrame, unit: unit,
                                         peerPlatformHint: platformByPeer[peer] ?? .unknown,
                                         discId: id))
        applyScan()
    }

    public func peripheralIsReady(toSendWriteWithoutResponse peripheral: CBPeripheral) {
        gattByPeer[peripheral.identifier]?.readyToSend()
    }

    public func peripheral(_ peripheral: CBPeripheral, didReadRSSI RSSI: NSNumber, error: (any Error)?) {
        guard error == nil, let link = linksByPeer[peripheral.identifier] else { return }
        let mtu = UInt16(clamping: peripheral.maximumWriteValueLength(for: .withoutResponse) + 3)
        let rssi = DiscoveryParser.rssi(RSSI)
        emit(.linkSample(link: link.id, rssi: rssi, mtu: mtu))
        registry.metrics(link.id, rssi: rssi, attMtu: mtu)
    }
}
