package com.convlink.voice.engine.audio.telecom

import android.content.Context
import android.net.Uri
import android.util.Log
import android.telecom.DisconnectCause
import androidx.core.telecom.CallAttributesCompat
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallsManager
import com.convlink.voice.core.FfiRoutePreference
import com.convlink.voice.engine.audio.AudioRouteController
import com.convlink.voice.engine.audio.AudioSessionStrategy
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Jetpack Core-Telecom self-managed call (DESIGN §10.2.2), compiled only with
 * `-Pmeshengine.telecom=true` (manifest overlay adds MANAGE_OWN_CALLS and the phoneCall FGS
 * type). The call keeps the intercom ranked like a VoIP call (head units, Android Auto);
 * the route itself is still driven by [AudioRouteController]. Evaluated once an Android
 * device exists (Phase 3 [DEV]).
 */
class TelecomStrategy(context: Context, private val routes: AudioRouteController) : AudioSessionStrategy {
    override val name = "telecom"

    private val calls = CallsManager(context).apply {
        registerAppWithTelecom(CallsManager.CAPABILITY_BASELINE)
    }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var call: Job? = null
    private var control: CallControlScope? = null

    override var listener: AudioRouteController.Listener?
        get() = routes.listener
        set(value) {
            routes.listener = value
        }

    override fun activate(preference: FfiRoutePreference, capture: Boolean): Boolean {
        if (call == null) {
            val attrs = CallAttributesCompat(
                displayName = "convLink Voice",
                address = Uri.parse("convlinkvoice:intercom"),
                direction = CallAttributesCompat.DIRECTION_OUTGOING,
                callType = CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
            )
            call = scope.launch {
                try {
                    calls.addCall(
                        attrs,
                        onAnswer = { },
                        onDisconnect = { },
                        onSetActive = { },
                        onSetInactive = { },
                    ) {
                        control = this
                        launch { setActive() }
                    }
                } catch (e: Exception) {
                    Log.w("TelecomStrategy", "self-managed call failed: $e")
                }
            }
        }
        return routes.enter(preference, capture)
    }

    override fun setPreference(preference: FfiRoutePreference) = routes.setPreference(preference)

    override fun deactivate() {
        val c = control
        control = null
        if (c != null) {
            scope.launch {
                runCatching { c.disconnect(DisconnectCause(DisconnectCause.LOCAL)) }
                call?.cancel()
                call = null
            }
        } else {
            call?.cancel()
            call = null
        }
        routes.exit()
    }
}
