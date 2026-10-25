package com.convlink.voice.engine.aware

import android.annotation.SuppressLint
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.wifi.aware.AttachCallback
import android.net.wifi.aware.DiscoverySessionCallback
import android.net.wifi.aware.PeerHandle
import android.net.wifi.aware.PublishConfig
import android.net.wifi.aware.PublishDiscoverySession
import android.net.wifi.aware.SubscribeConfig
import android.net.wifi.aware.SubscribeDiscoverySession
import android.net.wifi.aware.WifiAwareManager
import android.net.wifi.aware.WifiAwareNetworkInfo
import android.net.wifi.aware.WifiAwareNetworkSpecifier
import android.net.wifi.aware.WifiAwareSession
import android.os.Handler
import androidx.core.content.ContextCompat
import androidx.core.content.edit
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet6Address
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.SocketAddress

/** Wi-Fi Aware support and the beta switch (Settings → "Wi-Fi Aware (beta, Android)"). */
object AwareSupport {
    private const val PREFS = "convlink.aware"
    private const val KEY = "beta"

    /** FEATURE_WIFI_AWARE on this device (the toggle is hidden otherwise). */
    fun supported(context: Context): Boolean =
        context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_AWARE)

    /** The user's beta toggle; off by default. A plain flag, never a secret. */
    fun enabled(context: Context): Boolean =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(KEY, false)

    fun save(context: Context, on: Boolean) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit { putBoolean(KEY, on) }
    }
}

/**
 * [AwareOs] over `WifiAwareManager` (DESIGN §9.6): attach, publish/subscribe "cvic",
 * follow-up messages, `WifiAwareNetworkSpecifier.setPmk` + `requestNetwork`, a UDP socket
 * bound to the Aware `Network` (IPv6 link-local). `ACTION_WIFI_AWARE_STATE_CHANGED` with
 * Aware unavailable discards everything. The app never starts a hotspot or Wi-Fi Direct;
 * when the user does, Android disables Aware and the state broadcast tears the paths down.
 * Every callback runs on [handler].
 */
@SuppressLint("MissingPermission") // NEARBY_WIFI_DEVICES is asked by the app before the toggle turns on.
class AndroidAwareOs(
    context: Context,
    private val handler: Handler,
    private val log: (String) -> Unit,
) : AwareOs {
    private val app = context.applicationContext
    private val mgr: WifiAwareManager? =
        if (AwareSupport.supported(app)) app.getSystemService(WifiAwareManager::class.java) else null
    private val cm = app.getSystemService(ConnectivityManager::class.java)

    private var listener: AwareOsListener? = null
    private var session: WifiAwareSession? = null
    private var pub: PublishDiscoverySession? = null
    private var sub: SubscribeDiscoverySession? = null
    private var publishing = false
    private var subscribing = false
    private var msgId = 0
    private var receiverOn = false
    private val requests = HashMap<Int, Request>()

    private class Request(val callback: ConnectivityManager.NetworkCallback, val socket: DatagramSocket) {
        var io: UdpIo? = null
    }

    private val stateReceiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            if (mgr?.isAvailable != true) {
                log("aware: service unavailable")
                listener?.onUnavailable()
            }
        }
    }

    override fun available(): Boolean = mgr?.isAvailable == true

    override fun canOpenDataPath(): Boolean =
        (mgr?.availableAwareResources?.availableDataPathsCount ?: 0) > 0

    override fun attach(listener: AwareOsListener) {
        val m = mgr ?: return listener.onAttachFailed()
        this.listener = listener
        if (!receiverOn) {
            ContextCompat.registerReceiver(
                app,
                stateReceiver,
                IntentFilter(WifiAwareManager.ACTION_WIFI_AWARE_STATE_CHANGED),
                null,
                handler,
                ContextCompat.RECEIVER_NOT_EXPORTED,
            )
            receiverOn = true
        }
        try {
            m.attach(
                object : AttachCallback() {
                    override fun onAttached(s: WifiAwareSession) {
                        session = s
                        listener.onAttached()
                    }

                    override fun onAttachFailed() = listener.onAttachFailed()

                    override fun onAwareSessionTerminated() = listener.onUnavailable()
                },
                handler,
            )
        } catch (e: SecurityException) {
            log("aware: attach refused ($e)")
            listener.onAttachFailed()
        }
    }

    override fun publish() {
        val s = session ?: return
        if (publishing) return
        publishing = true
        val cfg = PublishConfig.Builder()
            .setServiceName(AwareCodec.SERVICE)
            .setPublishType(PublishConfig.PUBLISH_TYPE_UNSOLICITED)
            .build()
        s.publish(
            cfg,
            object : DiscoverySessionCallback() {
                override fun onPublishStarted(session: PublishDiscoverySession) {
                    pub = session
                }

                override fun onMessageReceived(peerHandle: PeerHandle, message: ByteArray) {
                    listener?.onMessage(peerHandle, true, message)
                }

                override fun onSessionConfigFailed() {
                    log("aware: publish failed")
                    publishing = false
                }

                override fun onSessionTerminated() {
                    pub = null
                    publishing = false
                }
            },
            handler,
        )
    }

    override fun subscribe() {
        val s = session ?: return
        if (subscribing) return
        subscribing = true
        val cfg = SubscribeConfig.Builder()
            .setServiceName(AwareCodec.SERVICE)
            .setSubscribeType(SubscribeConfig.SUBSCRIBE_TYPE_PASSIVE)
            .build()
        s.subscribe(
            cfg,
            object : DiscoverySessionCallback() {
                override fun onSubscribeStarted(session: SubscribeDiscoverySession) {
                    sub = session
                }

                override fun onServiceDiscovered(peerHandle: PeerHandle, ssi: ByteArray?, filter: List<ByteArray>?) {
                    listener?.onServiceDiscovered(peerHandle)
                }

                override fun onMessageReceived(peerHandle: PeerHandle, message: ByteArray) {
                    listener?.onMessage(peerHandle, false, message)
                }

                override fun onSessionConfigFailed() {
                    log("aware: subscribe failed")
                    subscribing = false
                }

                override fun onSessionTerminated() {
                    sub = null
                    subscribing = false
                }
            },
            handler,
        )
    }

    override fun sendMessage(handle: Any, fromPublisher: Boolean, bytes: ByteArray) {
        val peer = handle as? PeerHandle ?: return
        val s = if (fromPublisher) pub else sub
        s?.sendMessage(peer, ++msgId, bytes)
    }

    override fun requestDataPath(key: Int, role: AwareRole, handle: Any, pmk: ByteArray) {
        val l = listener ?: return
        val peer = handle as? PeerHandle ?: return l.onDataPathLost(key)
        val ds = (if (role == AwareRole.PUBLISHER) pub else sub) ?: return l.onDataPathLost(key)
        val socket = try {
            DatagramSocket(null).apply { bind(InetSocketAddress(0)) }
        } catch (e: java.io.IOException) {
            log("aware: socket failed ($e)")
            return l.onDataPathLost(key)
        }
        val spec = WifiAwareNetworkSpecifier.Builder(ds, peer).setPmk(pmk)
        if (role == AwareRole.PUBLISHER) {
            // Responder: the initiator learns this port from WifiAwareNetworkInfo.
            spec.setPort(socket.localPort).setTransportProtocol(UDP)
        }
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI_AWARE)
            .setNetworkSpecifier(spec.build())
            .build()
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
                val r = requests[key] ?: return
                if (r.io != null) return
                val target: SocketAddress? = if (role == AwareRole.SUBSCRIBER) {
                    val info = caps.transportInfo as? WifiAwareNetworkInfo ?: return
                    val addr = info.peerIpv6Addr ?: return
                    if (info.port <= 0) return
                    InetSocketAddress(scoped(addr, network), info.port)
                } else {
                    null // the responder learns the initiator's address from its first probe
                }
                try {
                    network.bindSocket(r.socket)
                } catch (e: java.io.IOException) {
                    log("aware: bindSocket failed ($e)")
                    l.onDataPathLost(key)
                    return
                }
                val io = UdpIo(r.socket, target)
                r.io = io
                l.onDataPathUp(key, io)
            }

            override fun onLost(network: Network) = l.onDataPathLost(key)

            override fun onUnavailable() = l.onDataPathLost(key)
        }
        requests[key] = Request(callback, socket)
        try {
            cm.requestNetwork(request, callback, handler, NETWORK_TIMEOUT_MS)
        } catch (e: RuntimeException) {
            log("aware: requestNetwork failed ($e)")
            releaseDataPath(key)
            l.onDataPathLost(key)
        }
    }

    /** The peer's link-local address with the Aware interface as scope. */
    private fun scoped(addr: Inet6Address, network: Network): Inet6Address {
        if (addr.scopeId != 0) return addr
        val name = cm.getLinkProperties(network)?.interfaceName ?: return addr
        val nif = NetworkInterface.getByName(name) ?: return addr
        return Inet6Address.getByAddress(null, addr.address, nif)
    }

    override fun releaseDataPath(key: Int) {
        val r = requests.remove(key) ?: return
        try {
            cm.unregisterNetworkCallback(r.callback)
        } catch (_: IllegalArgumentException) {
            // Callback already unregistered (request timed out): nothing to release.
        }
        r.io?.close() ?: r.socket.close()
    }

    override fun release() {
        requests.keys.toList().forEach(::releaseDataPath)
        pub?.close()
        sub?.close()
        session?.close()
        pub = null
        sub = null
        session = null
        publishing = false
        subscribing = false
        if (receiverOn) {
            try {
                app.unregisterReceiver(stateReceiver)
            } catch (_: IllegalArgumentException) {
                // Receiver already unregistered: nothing to do.
            }
            receiverOn = false
        }
        listener = null
    }

    private companion object {
        const val UDP = 17
        const val NETWORK_TIMEOUT_MS = 20_000
    }
}

/**
 * UDP over the Aware network: one datagram per LinkFrame. The responder learns the peer's
 * address from the first datagram and then only accepts that address.
 */
class UdpIo(private val socket: DatagramSocket, peer: SocketAddress?) : DatagramIo {
    @Volatile
    private var peer: SocketAddress? = peer

    override fun send(bytes: ByteArray): Boolean {
        val p = peer ?: return false
        return try {
            socket.send(DatagramPacket(bytes, bytes.size, p))
            true
        } catch (_: java.io.IOException) {
            // Path gone (the network callback reports the loss): the core sees NoLink.
            false
        }
    }

    override fun start(onData: (ByteArray) -> Unit) {
        Thread({
            val buf = ByteArray(2048)
            while (!socket.isClosed) {
                val pkt = DatagramPacket(buf, buf.size)
                try {
                    socket.receive(pkt)
                } catch (_: java.io.IOException) {
                    // Socket closed by close(): the loop ends.
                    break
                }
                val from = pkt.socketAddress
                if (peer == null) peer = from else if (peer != from) continue
                onData(pkt.data.copyOfRange(pkt.offset, pkt.offset + pkt.length))
            }
        }, "meshengine-aware-rx").apply { isDaemon = true }.start()
    }

    override fun close() {
        socket.close()
    }
}
