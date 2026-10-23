package com.convlink.voice.engine.notify

import org.json.JSONObject

/**
 * What the native side needs from the UI event stream (mesh-api `UiEvent`, §3.7): whether
 * the session is live, "Share my distance", capture health and Lab runs. Feeds the ongoing
 * notification and the FGS type computation. Pure (JVM-tested against the shared fixtures).
 */
class EventDigest {
    var live = false
        private set
    var inSession = false
        private set
    var shareDistance = false
        private set
    var captureOk = true
        private set
    var labRunId: String? = null
        private set

    /** Applies one event; returns true when the digest changed. */
    fun apply(json: String): Boolean {
        val e = runCatching { JSONObject(json) }.getOrNull() ?: return false
        val before = listOf(live, inSession, shareDistance, captureOk, labRunId)
        when (e.optString("type")) {
            "snapshot" -> {
                val s = e.optJSONObject("s") ?: return false
                val session = s.optJSONObject("session")
                inSession = session != null
                live = session?.optBoolean("live", false) ?: false
                if (session != null && session.has("shareDistance")) shareDistance = session.optBoolean("shareDistance")
                val audio = s.optJSONObject("audio")
                captureOk = audio == null || !live || audio.optBoolean("captureOk", true)
            }
            "labProgress" -> labRunId = e.optString("runId").ifEmpty { null }
            "labFinished" -> labRunId = null
        }
        return before != listOf(live, inSession, shareDistance, captureOk, labRunId)
    }

    /** Tracks accepted commands the engine reflects only later (`setShareDistance`). */
    fun commandAccepted(command: JSONObject) {
        when (command.optString("type")) {
            "setShareDistance" -> shareDistance = command.optBoolean("enabled", shareDistance)
            "labStop" -> labRunId = null
        }
    }

    fun labStarted(runId: String?) {
        labRunId = runId ?: labRunId ?: ""
    }

    fun reset() {
        live = false
        inSession = false
        captureOk = true
        labRunId = null
    }

    fun state(muted: Boolean, captureLost: Boolean): IntercomState = IntercomState(
        stage = when {
            labRunId != null -> IntercomState.Stage.LAB
            live -> IntercomState.Stage.LIVE
            else -> IntercomState.Stage.LOBBY
        },
        muted = muted,
        captureLost = captureLost || !captureOk,
    )
}
