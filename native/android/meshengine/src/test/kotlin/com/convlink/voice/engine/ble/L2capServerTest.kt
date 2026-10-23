package com.convlink.voice.engine.ble

import com.convlink.voice.engine.LinkRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/** A listening socket whose accept() is driven by the test (a channel, or an IOException). */
private class FakeListen(override val psm: Int) : L2capListenSocket {
    val next = LinkedBlockingQueue<Any>()

    @Volatile
    var closed = false

    override fun accept(): L2capChannel {
        while (true) {
            if (closed) throw IOException("closed")
            when (val v = next.poll(10, TimeUnit.MILLISECONDS)) {
                null -> continue
                is IOException -> throw v
                is L2capChannel -> return v
            }
        }
    }

    override fun close() {
        closed = true
    }
}

private class FakeChannel : L2capChannel {
    private val peer = PipedOutputStream()
    override val input: InputStream = PipedInputStream(peer, 1024)
    override val output: OutputStream = object : OutputStream() {
        override fun write(b: Int) = Unit
    }
    override val maxReceivePacketSize = 512
    override fun close() = peer.close()
}

class L2capServerTest {
    private fun waitUntil(cond: () -> Boolean) {
        val end = System.currentTimeMillis() + 3_000
        while (!cond()) {
            check(System.currentTimeMillis() < end) { "timed out" }
            Thread.sleep(5)
        }
    }

    @Test
    fun acceptFailureRelistensAndRepublishesThePsm() {
        val psms = CopyOnWriteArrayList<Int>()
        val sockets = CopyOnWriteArrayList<FakeListen>()
        var nextPsm = 0x80
        val sink = RecordingSink()
        val server = L2capServer(
            listen = { FakeListen(nextPsm++).also { sockets.add(it) } },
            registry = LinkRegistry(sink), config = BleConfig.DESIGN, onPsm = { psms.add(it) },
            retryMinMs = 1, retryMaxMs = 4,
        )
        server.start()
        waitUntil { psms == listOf(0x80) }
        // A central connects: a responder link comes up on the first server.
        sockets[0].next.put(FakeChannel())
        waitUntil { sink.ups.size == 1 }
        // The stack breaks the server socket: PSM 0 while down, then the new PSM.
        sockets[0].next.put(IOException("broken"))
        waitUntil { psms.size == 3 }
        assertEquals(listOf(0x80, 0, 0x81), psms)
        assertTrue(sockets[0].closed)
        assertEquals(0x81, server.psm)
        assertEquals(2, server.listens)
        // The new server accepts too.
        sockets[1].next.put(FakeChannel())
        waitUntil { sink.ups.size == 2 }
        server.stop()
        assertTrue(sockets[1].closed)
        assertEquals(listOf(0x80, 0, 0x81, 0), psms)
        Thread.sleep(30)
        assertEquals("a deliberate stop never relistens", 2, sockets.size)
    }

    @Test
    fun listenFailuresRetryWithBackoffAndPublishZero() {
        val psms = CopyOnWriteArrayList<Int>()
        val sleeps = CopyOnWriteArrayList<Long>()
        var attempts = 0
        val server = L2capServer(
            listen = {
                attempts++
                if (attempts <= 3) throw IOException("no channel") else FakeListen(0x90)
            },
            registry = LinkRegistry(RecordingSink()), config = BleConfig.DESIGN, onPsm = { psms.add(it) },
            retryMinMs = 100, retryMaxMs = 250, sleep = { sleeps.add(it) },
        )
        server.start()
        waitUntil { psms.lastOrNull() == 0x90 }
        assertEquals(listOf(0, 0, 0, 0x90), psms)
        assertEquals(listOf(100L, 200L, 250L), sleeps)
        server.stop()
    }

    @Test
    fun stopDuringBackoffEndsTheThread() {
        val psms = CopyOnWriteArrayList<Int>()
        lateinit var server: L2capServer
        server = L2capServer(
            listen = { throw IOException("down") },
            registry = LinkRegistry(RecordingSink()), config = BleConfig.DESIGN, onPsm = { psms.add(it) },
            retryMinMs = 1, sleep = { server.stop() },
        )
        server.start()
        waitUntil { psms.size >= 2 }
        Thread.sleep(20)
        assertEquals(listOf(0, 0), psms)
        assertEquals(0, server.listens)
    }
}
