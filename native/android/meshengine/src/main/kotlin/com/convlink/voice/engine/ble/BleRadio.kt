package com.convlink.voice.engine.ble

import android.Manifest
import android.bluetooth.BluetoothManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import androidx.core.content.ContextCompat
import com.convlink.voice.core.FfiDiscoveryMode
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiOpenVia
import com.convlink.voice.core.FfiRadioCommand
import com.convlink.voice.engine.LinkRegistry

/**
 * The Android radio stack (DESIGN §9.2-§9.5): maps the core's `RadioCommand`s onto the
 * scanner, advertiser, GATT server/client and L2CAP endpoints. Every command runs on one
 * `meshengine-ble` handler thread, so the drivers need no further ordering; the link I/O
 * runs on the per-link threads.
 */
class BleRadio(
    private val context: Context,
    private val registry: LinkRegistry,
    val config: BleConfig,
    private val onPsm: (Int) -> Unit,
    private val log: (String) -> Unit = { Log.i("BleRadio", it) },
) : LinkDriver {
    private val thread = HandlerThread("meshengine-ble").apply { start() }
    val handler = Handler(thread.looper)
    private val manager = context.getSystemService(BluetoothManager::class.java)
    private val adapter = manager?.adapter
    private val table = DiscoveryTable()
    private val clients = HashMap<Int, GattClientLink>()
    private var scanner: BleScanner? = null
    private var advertiser: BleAdvertiser? = null
    private var gattServer: GattServer? = null
    private var l2capServer: L2capServer? = null
    private var hello: ByteArray = ByteArray(config.helloLen)
    private val scheduler = HandlerScheduler(handler)
    private var discoveryMode = FfiDiscoveryMode.OFF
    private var advServiceData: ByteArray? = null
    private var coreIntensity: ScanIntensity? = null
    private val dutyTick = Runnable { applyDuty() }

    fun permitted(): Boolean = PERMISSIONS.all {
        ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED
    }

    override fun apply(cmd: FfiRadioCommand) {
        handler.post { handle(cmd) }
    }

    private fun handle(cmd: FfiRadioCommand) {
        val a = adapter
        if (a == null || !permitted() || !a.isEnabled) {
            log("radio command dropped (Bluetooth unavailable): ${cmd.javaClass.simpleName}")
            if (cmd is FfiRadioCommand.Connect) registry.connectFailed(cmd.discId.toInt(), FfiLinkDownReason.RadioOff)
            return
        }
        when (cmd) {
            is FfiRadioCommand.SetDiscovery -> setDiscovery(cmd.mode, cmd.adv?.serviceData, DiscoveryDuty.coreIntensity(cmd))
            is FfiRadioCommand.SetHello -> {
                hello = cmd.bytes.copyOf()
                gattServer?.hello = hello
            }
            is FfiRadioCommand.Connect -> connect(cmd.discId.toInt())
            is FfiRadioCommand.OpenLink -> {
                val c = clients[cmd.discId.toInt()] ?: return
                when (val via = cmd.via) {
                    is FfiOpenVia.L2cap -> c.openL2cap(via.psm.toInt())
                    FfiOpenVia.Gatt -> c.openGatt()
                }
            }
            is FfiRadioCommand.Abort -> {
                val id = cmd.discId.toInt()
                clients[id]?.abort()
                table.address(id)?.let { scanner?.ignore(it, cmd.ignoreS.toInt()) }
            }
            is FfiRadioCommand.Close -> registry.link(cmd.link.toInt())?.close(cmd.reason?.toInt() ?: -1)
                ?: registry.down(cmd.link.toInt(), FfiLinkDownReason.LocalClose)
            // GATT and L2CAP initiator links own a BluetoothGatt (kept open for L2CAP, see
            // GattClientLink) and request CONNECTION_PRIORITY_HIGH / BALANCED. Responder-side
            // links (GATT server subscriptions, accepted L2CAP sockets) cannot: Android has no
            // peripheral-side connection-priority API, the central chooses the interval.
            is FfiRadioCommand.SetLinkParams ->
                clients.values.firstOrNull { it.carries(cmd.link.toInt()) }?.setPriority(cmd.priority)
            // iOS pending-reconnect policy (§9.5): Android keeps scanning instead.
            is FfiRadioCommand.ArmReconnect -> log("ArmReconnect ignored on Android")
            // Phase 4 (Wi-Fi Aware, WP-24) and v2 (Tunnel) have no Android driver yet.
            is FfiRadioCommand.AwareUpgrade -> log("AwareUpgrade: no Aware driver in this build")
            is FfiRadioCommand.TunnelOpen -> log("TunnelOpen: no TunnelProvider in v1")
        }
    }

    private fun setDiscovery(mode: FfiDiscoveryMode, serviceData: ByteArray?, intensity: ScanIntensity?) {
        discoveryMode = mode
        advServiceData = serviceData?.copyOf()
        coreIntensity = intensity
        if (mode == FfiDiscoveryMode.OFF) {
            handler.removeCallbacks(dutyTick)
            scanner?.stop()
            advertiser?.stop()
            return
        }
        ensureServers()
        applyDuty()
    }

    /** BLE links currently up (GATT both roles, L2CAP both roles). */
    private fun bleLinks(): Int = registry.ids(FfiLinkKind.BLE_GATT).size + registry.ids(FfiLinkKind.BLE_L2CAP).size

    /**
     * Scan and advertising duty from the discovery mode and the links already up
     * ([DiscoveryDuty]); re-evaluated every [DUTY_TICK_MS] because links come and go
     * without a new `SetDiscovery`. The scanner restarts only when the duty changes.
     */
    private fun applyDuty() {
        handler.removeCallbacks(dutyTick)
        val a = adapter ?: return
        val intensity = DiscoveryDuty.intensity(discoveryMode, bleLinks(), coreIntensity) ?: return
        if (!permitted() || !a.isEnabled) return // the core re-sends SetDiscovery once the radio is back
        val duty = DiscoveryDuty.duty(intensity, runCatching { a.isOffloadedFilteringSupported }.getOrDefault(false))
        val sc = scanner ?: BleScanner(a, config, handler, table) { id, sd, rssi ->
            registry.discovered(DiscoveryParser.discovery(id, sd, rssi))
        }.also { scanner = it }
        if (sc.currentDuty?.intensity != duty.intensity) log("discovery duty ${duty.intensity} ($discoveryMode)")
        sc.setDuty(if (sc.currentDuty?.intensity == duty.intensity) sc.currentDuty else duty)
        val adv = advertiser ?: BleAdvertiser(a, config).also { advertiser = it }
        val sd = advServiceData
        if (sd != null && sd.size == AdvSpec.SERVICE_DATA_LEN) {
            adv.apply(AdvBuilder.spec(config, duty.advInterval, sd))
        } else {
            if (adv.spec != null) log("SetDiscovery without Android service data: not advertising")
            adv.apply(null)
        }
        handler.postDelayed(dutyTick, DUTY_TICK_MS)
    }

    private fun ensureServers() {
        val a = adapter ?: return
        val m = manager ?: return
        if (gattServer == null) {
            val s = GattServer(context, m, registry, config)
            s.hello = hello
            if (s.start()) gattServer = s else log("GATT server could not start")
        }
        if (l2capServer == null) {
            l2capServer = L2capServer(a, registry, config, onPsm).also { it.start() }
        }
    }

    private fun connect(discId: Int) {
        // One attempt per discovery; a finished client (e.g. after an L2CAP failure, before
        // its removal ran) is replaced so the core's GATT fallback `Connect` is honoured.
        clients[discId]?.let { if (!it.finished) return }
        val address = table.address(discId)
        if (address == null) {
            registry.connectFailed(discId, FfiLinkDownReason.Error(UNKNOWN_DISCOVERY))
            return
        }
        val device = adapter?.getRemoteDevice(address) ?: return
        val c = GattClientLink(
            discId, registry, config, scheduler,
            connectGatt = { events -> AndroidGatt.connect(context, device, config, handler, events) },
            l2capConnect = { psm, onClosed, onDone ->
                L2capClient.connect(device, psm, discId, registry, config, onClosed, onDone)
            },
        ) { done ->
            handler.post { if (clients[discId] === done) clients.remove(discId) }
        }
        clients[discId] = c
        c.connect()
    }

    /** Bluetooth went off: every BLE link is down with `RadioOff`; the OS objects are dropped. */
    fun radioOff() = handler.post {
        registry.dropAll(setOf(FfiLinkKind.BLE_GATT, FfiLinkKind.BLE_L2CAP), FfiLinkDownReason.RadioOff)
        clients.values.toList().forEach { it.abort() }
        clients.clear()
        teardownServers()
    }

    /** Requests RSSI readings of the GATT client links (host sampler, ≈ 1 Hz). */
    fun pollMetrics() = handler.post { clients.values.forEach { it.pollRssi() } }

    /**
     * Engine stop is ordered (WP-10A/10B): the core keeps sending CLOSE/LEAVE after `stop`
     * and closes each link itself, so the stack is torn down only once no link is left or
     * after [boundMs].
     */
    fun shutdownWhenDrained(boundMs: Long = DRAIN_BOUND_MS, done: () -> Unit = {}) {
        val deadline = SystemClock.elapsedRealtime() + boundMs
        fun step() {
            if (registry.count == 0 || SystemClock.elapsedRealtime() >= deadline) {
                shutdownNow()
                done()
            } else {
                handler.postDelayed({ step() }, DRAIN_POLL_MS)
            }
        }
        handler.post { step() }
    }

    private fun shutdownNow() {
        handler.removeCallbacks(dutyTick)
        discoveryMode = FfiDiscoveryMode.OFF
        scanner?.stop()
        advertiser?.stop()
        clients.values.toList().forEach { it.abort() }
        clients.clear()
        registry.ids().forEach { id -> registry.link(id)?.close(-1) ?: registry.down(id, FfiLinkDownReason.LocalClose) }
        teardownServers()
        table.reset()
    }

    private fun teardownServers() {
        gattServer?.stop()
        gattServer = null
        l2capServer?.stop()
        l2capServer = null
    }

    fun quit() {
        handler.post { shutdownNow() }
        thread.quitSafely()
    }

    companion object {
        val PERMISSIONS = listOf(
            Manifest.permission.BLUETOOTH_SCAN,
            Manifest.permission.BLUETOOTH_ADVERTISE,
            Manifest.permission.BLUETOOTH_CONNECT,
        )

        /** CLOSE_DRAIN_US (3 s) plus margin. */
        const val DRAIN_BOUND_MS = 3_500L
        const val DRAIN_POLL_MS = 50L
        const val UNKNOWN_DISCOVERY = -2

        /** Duty re-evaluation period while discovery is on. */
        const val DUTY_TICK_MS = 2_000L
    }
}
