package com.convlink.voice.engine.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.util.Log
import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkInfo
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiLinkRole
import com.convlink.voice.core.FfiLinkUnit
import com.convlink.voice.core.FfiPeerPlatform
import com.convlink.voice.engine.LinkRegistry
import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

private const val TAG = "MeshL2cap"

/** LinkInfo of an L2CAP CoC link (stream unit, §4.2 stream maximum). */
internal fun l2capInfo(role: FfiLinkRole, maxFrame: Int, discId: Int?) = FfiLinkInfo(
    kind = FfiLinkKind.BLE_L2CAP,
    role = role,
    maxFrame = maxFrame.toUShort(),
    unit = FfiLinkUnit.STREAM,
    peerPlatformHint = if (role == FfiLinkRole.INITIATOR) FfiPeerPlatform.ANDROID else null,
    attMtu = null,
    discId = discId?.toUInt(),
)

/** One connected L2CAP channel (a `BluetoothSocket`, or a fake in the JVM tests). */
interface L2capChannel : Closeable {
    val input: InputStream
    val output: OutputStream
    val maxReceivePacketSize: Int
}

/** A listening L2CAP server socket (a `BluetoothServerSocket`, or a fake in the JVM tests). */
interface L2capListenSocket : Closeable {
    val psm: Int

    /** Blocks until a central connects; throws [IOException] when closed or broken. */
    fun accept(): L2capChannel
}

internal class BtChannel(private val socket: BluetoothSocket) : L2capChannel {
    override val input: InputStream get() = socket.inputStream
    override val output: OutputStream get() = socket.outputStream
    override val maxReceivePacketSize: Int get() = socket.maxReceivePacketSize
    override fun close() = socket.close()
}

internal class BtListenSocket(private val socket: BluetoothServerSocket) : L2capListenSocket {
    override val psm: Int get() = socket.psm
    override fun accept(): L2capChannel = BtChannel(socket.accept())
    override fun close() = socket.close()
}

/**
 * Responder side: `listenUsingInsecureL2capChannel()` (no pairing, DESIGN §9.2) with an
 * accept thread. The PSM is published to the core (`on_l2cap_published`, carried in HELLO).
 *
 * Self-healing (field test 1): when `accept()` fails while the server was **not**
 * deliberately stopped (the stack recycled the channel, Bluetooth hiccup), the old socket
 * is closed, PSM 0 is published while down (peers fall back to GATT instead of dialling a
 * dead PSM), and a new server is opened with backoff; its PSM is published again.
 */
class L2capServer(
    private val listen: () -> L2capListenSocket,
    private val registry: LinkRegistry,
    private val config: BleConfig,
    private val onPsm: (Int) -> Unit,
    private val retryMinMs: Long = RETRY_MIN_MS,
    private val retryMaxMs: Long = RETRY_MAX_MS,
    private val sleep: (Long) -> Unit = { Thread.sleep(it) },
) {
    @SuppressLint("MissingPermission") // BLUETOOTH_CONNECT is checked by BleRadio before start
    constructor(adapter: BluetoothAdapter, registry: LinkRegistry, config: BleConfig, onPsm: (Int) -> Unit) :
        this({ BtListenSocket(adapter.listenUsingInsecureL2capChannel()) }, registry, config, onPsm)

    private val lock = Any()

    @Volatile
    private var server: L2capListenSocket? = null

    @Volatile
    private var stopped = true

    /** Bumped by every start/stop so a stale accept thread never relistens. */
    @Volatile
    private var generation = 0
    private var thread: Thread? = null

    /** Listening servers opened so far (diagnostics, tests). */
    @Volatile
    var listens: Int = 0
        private set

    val psm: Int get() = server?.psm ?: 0

    /** Opens the server (or, when listening fails, keeps retrying in the background). */
    fun start() {
        val g = synchronized(lock) {
            if (!stopped) return
            stopped = false
            ++generation
        }
        val t = Thread({ run(g) }, "l2cap-accept").apply { isDaemon = true }
        synchronized(lock) { thread = t }
        t.start()
    }

    fun stop() {
        val s = synchronized(lock) {
            if (stopped) return
            stopped = true
            generation++
            thread = null
            server.also { server = null }
        }
        runCatching { s?.close() }
        onPsm(0)
    }

    private fun alive(g: Int) = !stopped && generation == g

    private fun run(g: Int) {
        var backoff = retryMinMs
        while (alive(g)) {
            val s = try {
                listen()
            } catch (e: IOException) {
                Log.w(TAG, "L2CAP listen failed: $e; GATT only, retry in $backoff ms")
                onPsm(0)
                if (!pause(backoff, g)) return
                backoff = (backoff * 2).coerceAtMost(retryMaxMs)
                continue
            }
            val published = synchronized(lock) {
                if (!alive(g)) false else {
                    server = s
                    listens++
                    true
                }
            }
            if (!published) {
                runCatching { s.close() }
                return
            }
            onPsm(s.psm)
            backoff = retryMinMs
            acceptLoop(s, g)
            // accept() failed: deliberate stop, or a broken server to replace.
            val broken = synchronized(lock) {
                val b = alive(g) && server === s
                if (b) server = null
                b
            }
            if (!broken) return
            Log.w(TAG, "L2CAP server psm ${s.psm} died; relistening")
            runCatching { s.close() }
            onPsm(0)
            if (!pause(retryMinMs, g)) return
        }
    }

    private fun acceptLoop(s: L2capListenSocket, g: Int) {
        while (alive(g)) {
            val ch = try {
                s.accept()
            } catch (_: IOException) {
                return
            }
            open(ch, registry, config, FfiLinkRole.RESPONDER, null)
        }
    }

    /** Sleeps [ms] unless stopped; false when the server was stopped. */
    private fun pause(ms: Long, g: Int): Boolean {
        if (!alive(g)) return false
        try {
            sleep(ms)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
            return false
        }
        return alive(g)
    }

    companion object {
        const val RETRY_MIN_MS = 1_000L
        const val RETRY_MAX_MS = 30_000L
    }
}

/** Initiator side: `createInsecureL2capChannel(psm).connect()` on a worker thread. */
@SuppressLint("MissingPermission")
object L2capClient {
    /**
     * Connects and opens the link; [onDone] gets the new link id (null on failure). The
     * failure is **not** reported to the registry here: the caller does it once its GATT
     * client state is consistent (so a fallback `Connect` is never dropped as a duplicate).
     */
    fun connect(
        device: BluetoothDevice,
        psm: Int,
        discId: Int,
        registry: LinkRegistry,
        config: BleConfig,
        onClosed: (Int) -> Unit,
        onDone: (Int?) -> Unit,
    ) {
        Thread({
            val ch = try {
                BtChannel(device.createInsecureL2capChannel(psm).also { it.connect() })
            } catch (e: IOException) {
                Log.w(TAG, "L2CAP connect to psm $psm failed: $e")
                onDone(null)
                return@Thread
            }
            onDone(open(ch, registry, config, FfiLinkRole.INITIATOR, discId, onClosed))
        }, "l2cap-connect-$discId").apply {
            isDaemon = true
            start()
        }
    }
}

/** Wraps a connected channel into an [L2capLink]; returns its id, or null on failure. */
internal fun open(
    ch: L2capChannel,
    registry: LinkRegistry,
    config: BleConfig,
    role: FfiLinkRole,
    discId: Int?,
    onClosed: (Int) -> Unit = {},
): Int? {
    val id = registry.allocateId()
    val link = try {
        L2capLink(id, ch.input, ch.output, ch, registry,
            readBuffer = maxOf(ch.maxReceivePacketSize, 512), onClosed = onClosed)
    } catch (e: IOException) {
        runCatching { ch.close() }
        return null
    }
    registry.up(link, l2capInfo(role, config.maxStreamFrame, discId))
    link.start()
    return id
}
