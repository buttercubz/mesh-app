package com.convlink.voice.engine.diag

/**
 * A platform diagnostics event (`ridelog/1` `e` line, src `plat`), injected through
 * `MeshEngine.diagNote` so it shares the engine clock with core events.
 *
 * Privacy (diagnostics spec §5), enforced here **and** by the runtime's redaction filter:
 * only codes and numbers; never device or headset names, MAC addresses, keys or user
 * text. Strings are cut to 64 characters. Values: String, Int, Long, Double, Boolean,
 * null, or IntArray / List<Int>.
 */
class DiagNote(val ev: String, val fields: List<Pair<String, Any?>> = emptyList()) {
    /** The fields as a compact JSON object (keys in order). */
    val fieldsJson: String
        get() = buildString {
            append('{')
            fields.forEachIndexed { i, (k, v) ->
                if (i > 0) append(',')
                quote(k)
                append(':')
                value(v)
            }
            append('}')
        }

    private fun StringBuilder.value(v: Any?) {
        when (v) {
            null -> append("null")
            is Boolean -> append(v)
            is Int, is Long, is Short, is Byte -> append(v)
            is Double -> if (v.isFinite()) append(String.format(java.util.Locale.ROOT, "%.3f", v)) else append("null")
            is Float -> value(v.toDouble())
            is IntArray -> value(v.toList())
            is List<*> -> {
                append('[')
                v.take(MAX_ARRAY).forEachIndexed { i, x ->
                    if (i > 0) append(',')
                    append((x as? Number)?.toLong() ?: 0)
                }
                append(']')
            }
            else -> quote(v.toString().take(MAX_STR))
        }
    }

    private fun StringBuilder.quote(s: String) {
        append('"')
        for (c in s) {
            when {
                c == '"' -> append("\\\"")
                c == '\\' -> append("\\\\")
                c == '\n' -> append("\\n")
                c == '\r' -> append("\\r")
                c == '\t' -> append("\\t")
                c.code < 0x20 -> append(String.format(java.util.Locale.ROOT, "\\u%04x", c.code))
                else -> append(c)
            }
        }
        append('"')
    }

    override fun equals(other: Any?) = other is DiagNote && other.ev == ev && other.fieldsJson == fieldsJson

    override fun hashCode() = ev.hashCode() * 31 + fieldsJson.hashCode()

    override fun toString() = "DiagNote($ev $fieldsJson)"

    companion object {
        const val MAX_STR = 64
        const val MAX_ARRAY = 64

        /** A permission check / request with the exact API used and its answer. */
        fun perm(name: String, state: String, via: String, api: String) =
            DiagNote("perm", listOf("name" to name, "state" to state, "via" to via, "api" to api))

        /** Foreground-service start / started / fail / stop with the requested types. */
        fun fgs(state: String, types: Int, err: String? = null) =
            DiagNote("fgs", listOf("state" to state, "types" to fgsTypeNames(types), "mask" to types, "err" to err))

        /** App visibility (process importance) change. */
        fun app(foreground: Boolean, locked: Boolean?) =
            DiagNote("app", listOf("state" to if (foreground) "foreground" else "background", "locked" to locked))

        /** Bluetooth adapter state. */
        fun bt(state: String) = DiagNote("bt", listOf("state" to state))

        /** Oboe stream open attempt (`api` = oboe-aaudio / oboe-opensles when known). */
        fun audioOpen(capture: Boolean, render: Boolean, bypassVp: Boolean, result: Int, inDev: Int, outDev: Int) =
            DiagNote(
                "audioOpen",
                listOf(
                    "api" to "oboe", "capture" to capture, "render" to render, "bypassVp" to bypassVp,
                    "ok" to (result == 0), "err" to if (result == 0) null else result,
                    "inDev" to inDev, "outDev" to outDev,
                ),
            )

        /** Audio route change reported to the core (route kind only, never a device name). */
        fun audioRoute(route: String, inputRate: Int, headsetClass: String) =
            DiagNote("audioRoute", listOf("route" to route, "inputRate" to inputRate, "headsetClass" to headsetClass))

        /** Audio IO state reported to the core. */
        fun audioIo(state: String) = DiagNote("audioIo", listOf("state" to state))

        /** Audio focus loss, call start/end. */
        fun interruption(kind: String, phase: String) =
            DiagNote("interruption", listOf("kind" to kind, "phase" to phase))

        /** An Oboe stream closed by an error (`onErrorAfterClose`). */
        fun streamError(input: Boolean, code: Int) =
            DiagNote("audioError", listOf("dir" to if (input) "input" else "output", "code" to code))

        /** MuteWatchdog: capture callbacks stalled. */
        fun captureStall(ms: Long) = DiagNote("captureStall", listOf("ms" to ms))

        /** Power state: Doze, battery saver, battery-optimisation exemption, thermal. */
        fun power(dozeIdle: Boolean?, powerSave: Boolean?, batteryOptExempt: Boolean?, thermal: String?, bat: Double?) =
            DiagNote(
                "power",
                listOf(
                    "dozeIdle" to dozeIdle, "lowPower" to powerSave, "batteryOptExempt" to batteryOptExempt,
                    "thermal" to thermal, "bat" to bat,
                ),
            )

        /** OS signals: coldStart, memory pressure, terminate. */
        fun os(kind: String) = DiagNote("os", listOf("kind" to kind))

        /** `connectedDevice,microphone,location` from a FOREGROUND_SERVICE_TYPE_* mask. */
        fun fgsTypeNames(types: Int): String = buildList {
            if (types and 0x10 != 0) add("connectedDevice")
            if (types and 0x80 != 0) add("microphone")
            if (types and 0x8 != 0) add("location")
            if (types and 0x2 != 0) add("mediaPlayback")
        }.joinToString(",")
    }
}

/** Where platform code sends notes (the engine host; a no-op in tests). */
fun interface DiagNoteSink {
    fun note(n: DiagNote)
}
