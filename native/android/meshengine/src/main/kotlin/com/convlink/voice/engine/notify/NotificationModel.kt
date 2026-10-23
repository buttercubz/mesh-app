package com.convlink.voice.engine.notify

import org.json.JSONArray
import org.json.JSONObject
import java.util.Locale

/** Notification actions (DESIGN §10.2.1): delivered to MeshEngineService as intent actions. */
enum class IntercomAction(val intentAction: String, val labelKey: String) {
    MUTE("com.convlink.voice.engine.action.MUTE", "mute"),
    UNMUTE("com.convlink.voice.engine.action.UNMUTE", "unmute"),

    /** Talk latch: writes `mesh_rt_set_flags` bit 3 (edge). */
    TALK_LATCH("com.convlink.voice.engine.action.TALK_LATCH", "talk"),
    LEAVE("com.convlink.voice.engine.action.LEAVE", "leave"),

    /** Resume mic: a while-in-use exemption path (notification interaction). */
    RESUME("com.convlink.voice.engine.action.RESUME", "resume"),
    ;

    companion object {
        fun fromIntent(action: String?): IntercomAction? = entries.firstOrNull { it.intentAction == action }
    }
}

/**
 * Localised notification text. The engine sends i18n keys + params, never text (§3.7); the
 * bridge hands the UI's catalogs of the current language to the host, so native
 * notifications use the same strings as the toasts. [fallback] covers the ongoing
 * notification before the UI attached (ES/EN only, Appendix E).
 */
data class NotificationCatalog(
    val title: String,
    /** i18n key (`notices.x`, `errors.x`, `notify.x`) → template with `{{param}}`. */
    val templates: Map<String, String>,
) {
    fun text(key: String, params: JSONObject? = null): String? {
        var t = templates[key] ?: return null
        params?.keys()?.forEach { k ->
            val v = params.opt(k)
            val s = if (v is JSONArray) (0 until v.length()).joinToString(", ") { v.opt(it).toString() } else v.toString()
            t = t.replace("{{$k}}", s)
        }
        return t
    }

    fun label(action: IntercomAction): String = templates["notify.${action.labelKey}"]
        ?: fallback(Locale.getDefault()).templates.getValue("notify.${action.labelKey}")

    companion object {
        fun fallback(locale: Locale): NotificationCatalog {
            val es = locale.language == "es"
            return NotificationCatalog(
                title = "convLink Voice",
                templates = if (es) {
                    mapOf(
                        "notify.lobby" to "Grupo listo: conectando con los demás",
                        "notify.live" to "Intercomunicador activo",
                        "notify.muted" to "Intercomunicador activo · micrófono silenciado",
                        "notify.micPaused" to "Micrófono en pausa: toca para reanudar",
                        "notify.lab" to "Prueba de campo en curso",
                        "notify.rejoin" to "Toca para volver al grupo",
                        "notify.mute" to "Silenciar", "notify.unmute" to "Activar mic",
                        "notify.talk" to "Hablar", "notify.leave" to "Salir", "notify.resume" to "Reanudar",
                    )
                } else {
                    mapOf(
                        "notify.lobby" to "Group ready: connecting to the others",
                        "notify.live" to "Intercom live",
                        "notify.muted" to "Intercom live · mic muted",
                        "notify.micPaused" to "Mic paused: tap to resume",
                        "notify.lab" to "Field test running",
                        "notify.rejoin" to "Tap to rejoin the group",
                        "notify.mute" to "Mute", "notify.unmute" to "Unmute",
                        "notify.talk" to "Talk", "notify.leave" to "Leave", "notify.resume" to "Resume",
                    )
                },
            )
        }
    }
}

/** What the ongoing notification shows. */
data class IntercomState(
    val stage: Stage = Stage.LOBBY,
    val muted: Boolean = false,
    val pttLatched: Boolean = false,
    val captureLost: Boolean = false,
) {
    enum class Stage { LOBBY, LIVE, LAB }
}

/** The ongoing notification content, framework-free so the builder logic is JVM-tested. */
data class NotificationModel(val title: String, val text: String, val actions: List<Pair<IntercomAction, String>>)

object NotificationModels {
    fun ongoing(state: IntercomState, catalog: NotificationCatalog): NotificationModel {
        val fb = NotificationCatalog.fallback(Locale.getDefault())
        fun t(key: String) = catalog.text(key) ?: fb.text(key) ?: key
        val text = when {
            state.captureLost -> t("notify.micPaused")
            state.stage == IntercomState.Stage.LOBBY -> t("notify.lobby")
            state.stage == IntercomState.Stage.LAB -> t("notify.lab")
            state.muted -> t("notify.muted")
            else -> t("notify.live")
        }
        val actions = buildList {
            when {
                state.captureLost -> add(IntercomAction.RESUME)
                state.stage == IntercomState.Stage.LIVE -> {
                    add(if (state.muted) IntercomAction.UNMUTE else IntercomAction.MUTE)
                    add(IntercomAction.TALK_LATCH)
                }
                else -> Unit
            }
            add(IntercomAction.LEAVE)
        }.map { it to catalog.label(it) }
        return NotificationModel(catalog.title, text, actions)
    }
}
