package com.convlink.voice.engine.ble

import com.convlink.voice.core.FfiLinkDownReason
import com.convlink.voice.core.FfiLinkKind
import com.convlink.voice.core.FfiSendOutcome
import com.convlink.voice.engine.LinkRegistry
import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.util.ArrayDeque

/**
 * One L2CAP CoC link (DESIGN §9.1, §10.2.1): a reader thread that hands every chunk to the
 * registry (stream unit: the core reassembles frames) and a writer thread draining a
 * bounded queue. [send] never blocks: when the queue is full it answers `WouldBlock`, and
 * `linkWritable` follows once the writer has drained it below the low-water mark.
 *
 * Built over plain streams so it is JVM-testable; [L2capServer] / [L2capClient] pass a
 * `BluetoothSocket`'s streams and the socket as [closer].
 */
class L2capLink(
    override val id: Int,
    private val input: InputStream,
    private val output: OutputStream,
    private val closer: Closeable,
    private val registry: LinkRegistry,
    private val readBuffer: Int = 2048,
    private val maxQueuedBytes: Int = DEFAULT_MAX_QUEUED,
    /** Called once after the link went down (the initiator then drops its GATT client). */
    private val onClosed: (Int) -> Unit = {},
) : MeshLink {
    override val kind = FfiLinkKind.BLE_L2CAP

    private val lock = Object()
    private val queue = ArrayDeque<ByteArray>()
    private var queuedBytes = 0
    private var blocked = false

    @Volatile
    private var closed = false

    @Volatile
    private var localClose = false

    private val reader = Thread({ readLoop() }, "l2cap-r-$id")
    private val writer = Thread({ writeLoop() }, "l2cap-w-$id")

    /** Starts the reader and writer threads (after `registry.up`). */
    fun start() {
        reader.isDaemon = true
        writer.isDaemon = true
        reader.start()
        writer.start()
    }

    override fun send(bytes: ByteArray): FfiSendOutcome = synchronized(lock) {
        if (closed) return FfiSendOutcome.NO_LINK
        if (queuedBytes > 0 && queuedBytes + bytes.size > maxQueuedBytes) {
            blocked = true
            return FfiSendOutcome.WOULD_BLOCK
        }
        queue.addLast(bytes.copyOf())
        queuedBytes += bytes.size
        lock.notifyAll()
        FfiSendOutcome.ACCEPTED
    }

    override fun close(reason: Int) {
        localClose = true
        shutdown(FfiLinkDownReason.LocalClose)
    }

    /** Bytes waiting for the writer (tests, diagnostics). */
    val pendingBytes: Int get() = synchronized(lock) { queuedBytes }

    private fun readLoop() {
        val buf = ByteArray(readBuffer)
        try {
            while (!closed) {
                val n = input.read(buf)
                if (n < 0) break
                if (n > 0) registry.data(id, buf.copyOf(n))
            }
            shutdown(if (localClose) FfiLinkDownReason.LocalClose else FfiLinkDownReason.PeerClosed)
        } catch (e: IOException) {
            shutdown(if (localClose || closed) FfiLinkDownReason.LocalClose else FfiLinkDownReason.Error(IO_ERROR))
        }
    }

    private fun writeLoop() {
        try {
            while (true) {
                val unit: ByteArray
                synchronized(lock) {
                    while (queue.isEmpty() && !closed) lock.wait()
                    if (closed) return
                    unit = queue.peekFirst()!!
                }
                output.write(unit)
                output.flush()
                val signal = synchronized(lock) {
                    queue.pollFirst()
                    queuedBytes -= unit.size
                    val s = blocked && queuedBytes <= maxQueuedBytes / 2
                    if (s) blocked = false
                    s
                }
                if (signal) registry.writable(id)
            }
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        } catch (e: IOException) {
            shutdown(if (localClose || closed) FfiLinkDownReason.LocalClose else FfiLinkDownReason.Error(IO_ERROR))
        }
    }

    private fun shutdown(reason: FfiLinkDownReason) {
        val first = synchronized(lock) {
            val f = !closed
            closed = true
            queue.clear()
            queuedBytes = 0
            lock.notifyAll()
            f
        }
        if (first) runCatching { closer.close() }
        registry.down(id, reason)
        if (first) onClosed(id)
    }

    companion object {
        /** About 64 KB of queued stream data (≈ 2.5 s of 3-talker voice at 180 kbps). */
        const val DEFAULT_MAX_QUEUED = 64 * 1024
        const val IO_ERROR = -5
    }
}
