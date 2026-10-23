import CoreBluetooth
import Foundation

/// BLE central role (DESIGN §9.2-§9.5): scans for `MESH_SVC`, connects when the core says
/// `Connect`, reads HELLO, and opens an L2CAP or GATT link when the core says `OpenLink`.
/// Every decision is the core's; this driver only executes and reports.
///
/// Threading: all CoreBluetooth objects and the mutable state below are confined to
/// `q` (a serial queue); `apply` hops onto it. Hence `@unchecked Sendable`.
public final class BleCentralDriver: NSObject, LinkDriver, @unchecked Sendable {
    let constants: BleConstants
    let registry: LinkRegistry
    let io: LinkIOThread
    let radio: RadioStateMonitor?
    let q = BleQueue(label: "mesh.ble.central")
    let clock: @Sendable () -> TimeInterval

    // Confined to `q`.
    var central: CBCentralManager?
    var discoveries = DiscoveryTable(firstId: 1)
    var peripherals: [UUID: CBPeripheral] = [:]
    var tracker: ConnectTracker
    var mode: DiscoveryMode = .off
    var foreground = true
    var cancelled: Set<UUID> = []
    var openVia: [UUID: OpenVia] = [:]
    var linksByPeer: [UUID: any MeshLink] = [:]
    var platformByPeer: [UUID: PeerPlatform] = [:]
    var gattByPeer: [UUID: GattLink] = [:]
    var helloChar: [UUID: CBCharacteristic] = [:]
    var rxChar: [UUID: CBCharacteristic] = [:]
    var txChar: [UUID: CBCharacteristic] = [:]
    var priorities: [UInt32: LinkPriority] = [:]
    var timer: DispatchSourceTimer?
    var scanHint: ScanHint = .auto
    var lastScan: ScanPolicy?
    /// `Connect` received while the previous connection was still being torn down: issued
    /// from `didDisconnectPeripheral`.
    var deferredConnect: Set<UUID> = []
    var diag: BleDiagSink?

    /// - Parameter restore: set CoreBluetooth's restore identifier (requires the
    ///   `bluetooth-central` background mode; set but not relied upon, §10.1.3).
    public init(constants: BleConstants, registry: LinkRegistry, io: LinkIOThread,
                radio: RadioStateMonitor? = nil, restore: Bool = BleCentralDriver.hasBackgroundMode,
                clock: @escaping @Sendable () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
        self.constants = constants
        self.registry = registry
        self.io = io
        self.radio = radio
        self.clock = clock
        self.tracker = ConnectTracker(timeout: constants.connectTimeout)
        super.init()
        var opts: [String: Any] = [CBCentralManagerOptionShowPowerAlertKey: false]
        if restore {
            opts[CBCentralManagerOptionRestoreIdentifierKey] = constants.centralRestoreId
        }
        q.sync { central = CBCentralManager(delegate: self, queue: q.queue, options: opts) }
    }

    public static var hasBackgroundMode: Bool {
        let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String] ?? []
        return modes.contains("bluetooth-central")
    }

    /// App visibility: duplicates are only requested while Forming in the foreground.
    public func setForeground(_ fg: Bool) {
        q.async { [self] in
            foreground = fg
            applyScan()
        }
    }

    /// Scan intensity override (e.g. from a core scan-intensity field): see `ScanHint`.
    public func setScanHint(_ hint: ScanHint) {
        q.async { [self] in
            scanHint = hint
            applyScan()
        }
    }

    /// Subscribes to driver events (nil unsubscribes). The sink runs on the driver queue.
    public func onDiagnostics(_ sink: BleDiagSink?) {
        q.async { [self] in diag = sink }
    }

    func emit(_ e: BleDiagEvent) { diag?(e) }

    public func apply(_ cmd: RadioCommand) {
        q.async { [self] in handle(cmd) }
    }

    func handle(_ cmd: RadioCommand) {
        let now = clock()
        switch cmd {
        case .setDiscovery(let m, _):
            mode = m
            applyScan()
        case .connect(let discId):
            connect(discId, now: now)
        case .openLink(let discId, let via):
            open(discId, via: via)
        case .abort(let discId, let ignoreS):
            tracker.ignore(discId, seconds: ignoreS, now: now)
            disconnect(discId)
        case .close(let link, _):
            if let peer = linksByPeer.first(where: { $0.value.id == link })?.key {
                cancelled.insert(peer)
                linksByPeer[peer]?.close(reason: 0)
                if let p = peripherals[peer] { central?.cancelPeripheralConnection(p) }
            }
        case .setLinkParams(let link, let priority):
            priorities[link] = priority // iOS cannot set the connection interval (§9.2)
        case .armReconnect(let discId, let untilS):
            tracker.arm(discId, untilS: untilS, now: now)
            applyScan()
            if untilS > 0 { armTimer() }
            if untilS > 0, let peer = discoveries.peer(for: discId), linksByPeer[peer] == nil {
                connect(discId, now: now)
            }
        case .setHello, .awareUpgrade, .tunnelOpen:
            break // peripheral driver / WP-24 / v2
        }
    }

    /// Applies `ScanPolicy`; the scan is only restarted when the decision changes, so it is
    /// cheap to call after every link change.
    func applyScan() {
        guard let central, central.state == .poweredOn else { return }
        let links = registry.ids(kind: .bleL2cap).count + registry.ids(kind: .bleGatt).count
        let d = ScanPolicy.decide(mode: mode, foreground: foreground, bleLinks: links,
                                  armedReconnects: tracker.armedPeers(now: clock()).count,
                                  hint: scanHint)
        if d == lastScan, central.isScanning == d.scan { return }
        lastScan = d
        if d.scan {
            central.scanForPeripherals(withServices: [constants.meshService],
                                       options: [CBCentralManagerScanOptionAllowDuplicatesKey: d.allowDuplicates])
        } else if central.isScanning {
            central.stopScan()
        }
        emit(.scan(mode: mode, on: d.scan, allowDuplicates: d.allowDuplicates))
    }

    func connect(_ discId: UInt32, now: TimeInterval) {
        guard let central, let peer = discoveries.peer(for: discId), let p = peripherals[peer] else {
            emit(.connectSkipped(discId: discId, why: "unknown"))
            return
        }
        switch p.state {
        case .disconnected:
            break
        case .disconnecting:
            deferredConnect.insert(peer)
            emit(.connectSkipped(discId: discId, why: "deferred"))
            return
        case .connected where linksByPeer[peer] == nil && openVia[peer] == nil:
            // Still connected after a failed open or an abort that has not landed yet:
            // read HELLO again on the same connection instead of dropping the command.
            cancelled.remove(peer)
            emit(.connectAttempt(discId: discId, pending: false))
            p.delegate = self
            p.discoverServices([constants.meshService])
            return
        default:
            emit(.connectSkipped(discId: discId, why: "busy"))
            return
        }
        switch tracker.begin(discId, now: now) {
        case .start:
            cancelled.remove(peer)
            central.connect(p, options: nil)
            emit(.connectAttempt(discId: discId, pending: false))
        case .startPending:
            cancelled.remove(peer)
            central.connect(p, options: [CBConnectPeripheralOptionEnableAutoReconnect: true])
            emit(.connectAttempt(discId: discId, pending: true))
        case .busy:
            emit(.connectSkipped(discId: discId, why: "busy"))
            return
        case .ignored:
            emit(.connectSkipped(discId: discId, why: "ignored"))
            return
        }
        armTimer()
    }

    /// `OpenLink`: L2CAP on the peer's PSM, or the GATT data path (subscribe to `TX`
    /// notifications, then write-without-response on `RX`; the link comes up in
    /// `didUpdateNotificationStateFor`). Anything that prevents the open is reported to
    /// the core as `connectFailed`, which then picks the next attempt (e.g. GATT after an
    /// L2CAP failure).
    func open(_ discId: UInt32, via: OpenVia) {
        guard let peer = discoveries.peer(for: discId) else {
            registry.connectFailed(discId, reason: .error(-1))
            return
        }
        let p = peripherals[peer]
        emit(.openAttempt(discId: discId, via: via))
        let check = OpenCheck.check(via: via, connected: p?.state == .connected,
                                    hasRx: rxChar[peer] != nil, hasTx: txChar[peer] != nil)
        guard check == .ok, let p else {
            openFailed(peer, discId: discId, via: via, error: nil)
            return
        }
        openVia[peer] = via
        switch via {
        case .l2cap(let psm):
            p.openL2CAPChannel(CBL2CAPPSM(psm))
        case .gatt:
            if let tx = txChar[peer] { p.setNotifyValue(true, for: tx) }
        }
    }

    /// The data path failed before a link existed: tell the core (so its next decision
    /// can choose another transport) and drop the connection.
    func openFailed(_ peer: UUID, discId: UInt32, via: OpenVia, error: (any Error)?) {
        let info = BleErrorInfo(error)
        emit(.openFailed(discId: discId, via: via, error: info))
        PlatformLog.ble.notice("open \(String(describing: via)) failed: \(error?.localizedDescription ?? "precondition")")
        openVia[peer] = nil
        registry.connectFailed(discId, reason: .error(Int32(clamping: info?.code ?? -2)))
        if let p = peripherals[peer], p.state != .disconnected {
            cancelled.insert(peer)
            central?.cancelPeripheralConnection(p)
        }
    }

    func disconnect(_ discId: UInt32) {
        guard let peer = discoveries.peer(for: discId), let p = peripherals[peer] else { return }
        deferredConnect.remove(peer)
        cancelled.insert(peer)
        central?.cancelPeripheralConnection(p)
    }

    /// 1 s housekeeping while attempts or links exist: 8 s connect timeout, armed windows,
    /// RSSI reads for link metrics (Lab samples at 1 Hz).
    func armTimer() {
        guard timer == nil else { return }
        let t = DispatchSource.makeTimerSource(queue: q.queue)
        t.schedule(deadline: .now() + 1, repeating: 1)
        t.setEventHandler { [weak self] in self?.tick() }
        timer = t
        t.resume()
    }

    func tick() {
        let now = clock()
        var cancel: [UInt32] = []
        if let d = tracker.expired(now: now) {
            cancel.append(d)
            emit(.connectResult(discId: d, ok: false, timedOut: true, error: nil))
            registry.connectFailed(d, reason: .timeout)
        }
        cancel += tracker.expiredPending(now: now)
        for d in cancel { disconnect(d) }
        applyScan() // armed windows may have closed
        tracker.prune(now: now)
        for peer in linksByPeer.keys {
            if let p = peripherals[peer], p.state == .connected { p.readRSSI() }
        }
        if tracker.nextDeadline(now: now) == nil, linksByPeer.isEmpty,
           tracker.armedPeers(now: now).isEmpty {
            timer?.cancel()
            timer = nil
        }
    }

    func discId(for p: CBPeripheral) -> UInt32 { discoveries.id(for: p.identifier) }
}
