package com.convlink.voice.plugin

import com.convlink.voice.core.FfiSessionStart
import com.convlink.voice.engine.EngineHostException
import com.convlink.voice.engine.audio.CommandJson
import com.convlink.voice.engine.lab.LabFileStore
import org.json.JSONObject
import java.io.File

/**
 * Pure helpers of the MeshIntercom bridge (WP-19): option parsing and the mapping of
 * engine failures to mesh-api `ErrorCode`s (C1). Kept free of Android framework calls so
 * the rules match the iOS bridge one to one.
 */
internal object BridgeSupport {
    /** `startEngine` options → session start (`demo`, `lab.code` 1-4 digits, 32-hex `groupId`). */
    fun sessionStart(options: JSONObject): FfiSessionStart? {
        if (options.optBoolean("demo", false)) return FfiSessionStart.Demo
        if (options.has("lab")) {
            val lab = options.optJSONObject("lab") ?: return null
            val code = lab.opt("code") as? String ?: return null
            return labCode(code)?.let { FfiSessionStart.Lab(it.toUShort()) }
        }
        val gid = options.opt("groupId") as? String ?: return null
        return groupId(gid)?.let { FfiSessionStart.Group(it) }
    }

    /** A Lab code: 1 to 4 ASCII digits (§12.1). */
    fun labCode(s: String): Int? =
        if (s.length in 1..4 && s.all { it in '0'..'9' }) s.toInt() else null

    /** 32 hex characters → the 16-byte group id. */
    fun groupId(hex: String): ByteArray? {
        if (hex.length != 32) return null
        val out = ByteArray(16)
        for (i in 0 until 16) {
            val hi = Character.digit(hex[2 * i], 16)
            val lo = Character.digit(hex[2 * i + 1], 16)
            if (hi < 0 || lo < 0) return null
            out[i] = ((hi shl 4) or lo).toByte()
        }
        return out
    }

    /** Engine/host failure → C1 `ErrorCode` (same table as the iOS bridge). */
    fun errorCode(error: Throwable): String {
        if (error !is EngineHostException) return "internal"
        return when (error.code) {
            "notForeground" -> "notForeground"
            "apiVersionMismatch" -> "apiVersionMismatch"
            else -> {
                val d = error.message ?: ""
                when {
                    d.contains("UnknownGroup") -> "unknownGroup"
                    d.contains("InvalidStart") || d.contains("InvalidConfig") -> "invalidParams"
                    d.contains("DemoUnavailable") -> "notSupported"
                    d.contains("Crashed") -> "engineCrashed"
                    else -> "internal"
                }
            }
        }
    }

    fun isOk(result: String): Boolean = CommandJson.isOk(result)

    /** The `code` of a refused `CommandResult`, or null. */
    fun resultCode(result: String): String? =
        CommandJson.parse(result)?.optString("code")?.ifEmpty { null }

    fun commandType(json: String): String? =
        CommandJson.parse(json)?.optString("type")?.ifEmpty { null }

    /** The existing Lab file named by `path` inside [dir] (`.jsonl`/`.csv` only), or null. */
    fun labFile(dir: File, path: String): File? {
        val name = path.substringAfterLast('/')
        if (!LabFileStore.validate(name) || !(name.endsWith(".jsonl") || name.endsWith(".csv"))) return null
        val file = File(dir, name)
        return if (file.isFile) file else null
    }
}
