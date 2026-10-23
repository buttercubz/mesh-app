package com.convlink.voice.engine.audio

import android.content.Context
import android.util.Log
import com.convlink.voice.core.FfiRoutePreference
import com.convlink.voice.engine.BuildConfig

/**
 * How the engine's audio session is held (DESIGN §10.2.2). [DefaultCommunicationStrategy]
 * is always on; the Jetpack Core-Telecom self-managed call (`TelecomStrategy`) is compiled
 * only with `-Pmeshengine.telecom=true` (src/telecom) and evaluated once a device exists.
 */
interface AudioSessionStrategy {
    val name: String

    /** Enters the session (communication mode, focus, route). Main thread. */
    fun activate(preference: FfiRoutePreference, capture: Boolean): Boolean

    fun setPreference(preference: FfiRoutePreference)

    /** Leaves the session. Main thread. */
    fun deactivate()

    var listener: AudioRouteController.Listener?

    companion object {
        const val TELECOM_CLASS = "com.convlink.voice.engine.audio.telecom.TelecomStrategy"

        /**
         * The strategy for this build: Telecom when compiled in and [preferTelecom], else the
         * default one. The Telecom class is looked up by name because the default variant
         * does not compile it.
         */
        fun create(context: Context, routes: AudioRouteController, preferTelecom: Boolean): AudioSessionStrategy {
            if (BuildConfig.TELECOM && preferTelecom) {
                try {
                    val cls = Class.forName(TELECOM_CLASS)
                    val ctor = cls.getConstructor(Context::class.java, AudioRouteController::class.java)
                    return ctor.newInstance(context, routes) as AudioSessionStrategy
                } catch (e: ReflectiveOperationException) {
                    Log.w("AudioSessionStrategy", "Telecom strategy unavailable: $e")
                }
            }
            return DefaultCommunicationStrategy(routes)
        }
    }
}

/** `MODE_IN_COMMUNICATION` + `setCommunicationDevice` + focus, no Telecom involvement. */
class DefaultCommunicationStrategy(private val routes: AudioRouteController) : AudioSessionStrategy {
    override val name = "default"

    override var listener: AudioRouteController.Listener?
        get() = routes.listener
        set(value) {
            routes.listener = value
        }

    override fun activate(preference: FfiRoutePreference, capture: Boolean) = routes.enter(preference, capture)

    override fun setPreference(preference: FfiRoutePreference) = routes.setPreference(preference)

    override fun deactivate() = routes.exit()
}
