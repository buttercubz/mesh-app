package com.convlink.voice.engine.audio

import org.json.JSONException
import org.json.JSONObject

/**
 * The ★ capture-gated commands (DESIGN §3.2, §3.7, §11.1). Mirrors mesh-api
 * `GESTURE_COMMANDS`: goLive, resumeMic, labStart with `params.audio ≠ "off"`, audioCheck
 * with `op == "start"`. The shared fixture crates/mesh-api/fixtures/commands.json
 * (`requiresGesture`) is replayed by the tests so both stay equal.
 */
sealed class GestureCommand {
    data object GoLive : GestureCommand()
    data object ResumeMic : GestureCommand()

    /** [audio] is "renderOnly" or "full"; [anchor] "none" | "audio" | "audioLocation". */
    data class LabStart(val bench: String, val audio: String, val anchor: String) : GestureCommand()
    data class AudioCheckStart(val mode: String) : GestureCommand()

    companion object {
        /** Classifies a UI command; null for every non-gated command (and invalid JSON). */
        fun classify(json: String): GestureCommand? {
            val obj = CommandJson.parse(json) ?: return null
            return when (obj.optString("type")) {
                "goLive" -> GoLive
                "resumeMic" -> ResumeMic
                "labStart" -> {
                    val params = obj.optJSONObject("params") ?: return null
                    // NotEquals: the field must be present and differ from "off".
                    if (!params.has("audio") || params.isNull("audio")) return null
                    val audio = params.optString("audio")
                    if (audio == "off") return null
                    LabStart(obj.optString("bench"), audio, params.optString("anchor", "audio"))
                }
                "audioCheck" -> if (obj.optString("op") == "start") AudioCheckStart(obj.optString("mode", "tone")) else null
                else -> null
            }
        }

        fun requiresGesture(json: String) = classify(json) != null
    }
}

/** What the audio path of a gesture opens (§12.1 Lab audio paths). */
enum class AudioPath { FULL, RENDER_ONLY, BYPASS_VP }

/**
 * The OS-level work the bridge performs synchronously inside the tap, before the command
 * reaches the engine (§3.2, §10.2.3): `startForegroundService` with the microphone type
 * (+ location when requested and granted), then open the Oboe streams with capture muted.
 */
data class GesturePlan(
    val path: AudioPath,
    /** The microphone FGS type is needed (capture). */
    val microphone: Boolean,
    /** Location requested by the session ("Share my distance") or Lab anchor. */
    val wantsLocation: Boolean,
    /** Restarts a lost capture of a live session. */
    val resume: Boolean,
) {
    companion object {
        fun make(g: GestureCommand, shareDistance: Boolean): GesturePlan = when (g) {
            GestureCommand.GoLive -> GesturePlan(AudioPath.FULL, true, shareDistance, false)
            GestureCommand.ResumeMic -> GesturePlan(AudioPath.FULL, true, shareDistance, true)
            is GestureCommand.LabStart -> {
                val path = when {
                    g.audio == "renderOnly" -> AudioPath.RENDER_ONLY
                    // T5a measures the headset loop with voice processing bypassed (§12.2).
                    g.bench == "T5a" -> AudioPath.BYPASS_VP
                    else -> AudioPath.FULL
                }
                GesturePlan(path, path != AudioPath.RENDER_ONLY, g.anchor == "audioLocation", false)
            }
            is GestureCommand.AudioCheckStart -> GesturePlan(AudioPath.FULL, true, false, false)
        }
    }
}

/** Small JSON helpers for UI commands and results (the bridge only peeks at discriminants). */
object CommandJson {
    fun parse(json: String): JSONObject? = try {
        JSONObject(json)
    } catch (_: JSONException) {
        null
    }

    /** `{"ok":false,"code":<code>}` (mesh-api `CommandResult` error shape). */
    fun error(code: String): String = JSONObject().put("ok", false).put("code", code).toString()

    fun isOk(result: String): Boolean = parse(result)?.optBoolean("ok", false) == true
}
