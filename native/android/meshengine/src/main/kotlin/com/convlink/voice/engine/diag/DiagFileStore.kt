package com.convlink.voice.engine.diag

import android.content.Context
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** One diagnostics session on disk (`diag/<sessionId>/seg-NNNN.jsonl`). */
data class DiagSession(val sessionId: String, val segments: List<File>, val bytes: Long, val modifiedMs: Long)

/** Disk usage for the Settings line. */
data class DiagUsage(val bytes: Long, val sessions: Int, val oldestMs: Long?)

/**
 * The diagnostics flight recorder's files (`ridelog/1`, diagnostics spec §3.2, §5):
 * segments the runtime hands over through `PlatformHooks.diag_write`, kept in the
 * app-private `filesDir/diag` (never external storage). Budget: 64 MiB in total, oldest
 * sessions evicted first; the current session is never evicted (its oldest segments are,
 * beyond 48 MiB); 14 days retention. Files leave the device only through an explicit
 * export ([exportZip] → share sheet).
 *
 * Every segment file starts with its `hdr` line: lines for a segment whose file does not
 * exist (cleared or evicted mid-segment) are dropped until the next segment starts.
 * Thread-safe.
 */
class DiagFileStore(
    val directory: File,
    private val maxBytes: Long = DEFAULT_MAX_BYTES,
    private val currentMaxBytes: Long = DEFAULT_CURRENT_MAX_BYTES,
    private val retentionMs: Long = DEFAULT_RETENTION_MS,
    private val now: () -> Long = System::currentTimeMillis,
) {
    private var current: String? = null
    private var openKey: String? = null
    private var open: FileOutputStream? = null
    private var bytesSinceBudget = 0L

    /** Lines dropped because their segment file was gone. */
    @Volatile
    var droppedLines = 0
        private set

    init {
        if (!directory.isDirectory && !directory.mkdirs()) throw IOException("cannot create $directory")
        enforceBudget()
    }

    fun sessionDir(sessionId: String): File = File(directory, checked(sessionId))

    fun segmentFile(sessionId: String, seg: Int): File {
        require(seg in 1..9999) { "bad segment $seg" }
        return File(sessionDir(sessionId), "seg-%04d.jsonl".format(java.util.Locale.ROOT, seg))
    }

    /** Appends a `diag_write` batch (lines without newlines). */
    @Synchronized
    fun append(sessionId: String, seg: Int, lines: List<String>) {
        if (lines.isEmpty()) return
        require(lines.none { it.contains('\n') }) { "line contains a newline" }
        val target = segmentFile(sessionId, seg)
        val key = "$sessionId/$seg"
        current = sessionId
        if (openKey != key || !target.isFile) {
            runCatching { open?.close() }
            open = null
            openKey = null
            if (!target.isFile) {
                if (!lines[0].startsWith("{\"t\":\"hdr\"")) {
                    droppedLines += lines.size
                    return
                }
                target.parentFile?.mkdirs()
            }
            open = FileOutputStream(target, true)
            openKey = key
        }
        val data = (lines.joinToString("\n") + "\n").toByteArray(Charsets.UTF_8)
        open!!.write(data)
        open!!.flush()
        bytesSinceBudget += data.size
        if (bytesSinceBudget >= 1L shl 20) {
            bytesSinceBudget = 0
            enforceBudget()
        }
    }

    /** Stored sessions, newest first. */
    @Synchronized
    fun sessions(): List<DiagSession> = (directory.listFiles() ?: emptyArray())
        .filter { it.isDirectory && validate(it.name) }
        .mapNotNull { d ->
            val segs = (d.listFiles() ?: emptyArray())
                .filter { it.isFile && it.name.startsWith("seg-") && it.name.endsWith(".jsonl") }
                .sortedBy { it.name }
            if (segs.isEmpty()) {
                null
            } else {
                DiagSession(d.name, segs, segs.sumOf { it.length() }, segs.maxOf { it.lastModified() })
            }
        }
        .sortedWith(compareByDescending<DiagSession> { it.modifiedMs }.thenByDescending { it.sessionId })

    fun usage(): DiagUsage {
        val s = sessions()
        return DiagUsage(s.sumOf { it.bytes }, s.size, s.minOfOrNull { it.modifiedMs })
    }

    /** Retention, total budget (oldest first, never the current), current-session cap. */
    @Synchronized
    fun enforceBudget() {
        val cutoff = now() - retentionMs
        sessions().filter { it.sessionId != current && it.modifiedMs < cutoff }.forEach { remove(it.sessionId) }
        val list = sessions()
        var total = list.sumOf { it.bytes }
        for (s in list.asReversed()) {
            if (total <= maxBytes) break
            if (s.sessionId == current) continue
            remove(s.sessionId)
            total -= s.bytes
        }
        val cur = list.firstOrNull { it.sessionId == current } ?: return
        var bytes = cur.bytes
        for (seg in cur.segments.dropLast(1)) {
            if (bytes <= currentMaxBytes) break
            bytes -= seg.length()
            seg.delete()
        }
    }

    @Synchronized
    private fun remove(id: String) {
        if (openKey?.startsWith("$id/") == true) {
            runCatching { open?.close() }
            open = null
            openKey = null
        }
        sessionDir(id).deleteRecursively()
    }

    /** Deletes every diagnostics file (the toggle and the RAM ring are unaffected). */
    @Synchronized
    fun clear() {
        runCatching { open?.close() }
        open = null
        openKey = null
        sessions().forEach { remove(it.sessionId) }
    }

    enum class Scope { LAST, ALL }

    /**
     * Writes `convlink-diag-<label>.zip` into [outDir]: `device.json` plus the chosen
     * sessions' segments (deflated). Without session files, [ring] (the runtime's RAM
     * ring, a self-contained JSONL file) goes in as `diag/ring.jsonl`.
     */
    @Synchronized
    fun exportZip(scope: Scope, label: String, deviceJson: String, ring: List<String>?, outDir: File): Pair<File, Int> {
        runCatching { open?.flush() }
        val all = sessions()
        val chosen = if (scope == Scope.LAST) all.take(1) else all
        if (!outDir.isDirectory && !outDir.mkdirs()) throw IOException("cannot create $outDir")
        val safe = label.filter { it.isLetterOrDigit() && it.code < 128 || it == '-' || it == '_' }.take(80)
        val zip = File(outDir, "convlink-diag-${safe.ifEmpty { "export" }}.zip")
        ZipOutputStream(FileOutputStream(zip)).use { z ->
            z.putNextEntry(ZipEntry("device.json"))
            z.write(deviceJson.toByteArray(Charsets.UTF_8))
            z.closeEntry()
            for (s in chosen) for (seg in s.segments) {
                z.putNextEntry(ZipEntry("diag/${s.sessionId}/${seg.name}"))
                seg.inputStream().use { it.copyTo(z) }
                z.closeEntry()
            }
            if (chosen.isEmpty() && !ring.isNullOrEmpty()) {
                z.putNextEntry(ZipEntry("diag/ring.jsonl"))
                z.write((ring.joinToString("\n") + "\n").toByteArray(Charsets.UTF_8))
                z.closeEntry()
            }
        }
        return zip to chosen.size
    }

    companion object {
        const val DEFAULT_MAX_BYTES = 64L shl 20
        const val DEFAULT_CURRENT_MAX_BYTES = 48L shl 20
        const val DEFAULT_RETENTION_MS = 14L * 86_400_000L
        private val ID = Regex("d-[0-9]{8}-[0-9]{4}-[0-9a-z]{4}")

        /** App-private `filesDir/diag`. */
        fun create(context: Context): DiagFileStore = DiagFileStore(File(context.filesDir, "diag"))

        /** `d-YYYYMMDD-HHMM-xxxx` (generated by the runtime); anything else is refused. */
        fun validate(sessionId: String): Boolean = ID.matches(sessionId)

        private fun checked(id: String): String {
            require(validate(id)) { "bad session id: $id" }
            return id
        }
    }
}
