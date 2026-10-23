package com.convlink.voice.engine.audio

import android.content.Context
import android.os.Bundle
import android.speech.tts.TextToSpeech
import android.speech.tts.UtteranceProgressListener
import com.convlink.voice.core.FfiPromptRequest
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.Locale

/**
 * Renders spoken prompts (DESIGN §7.7) with the platform TTS into 16 kHz mono PCM and hands
 * them to the core (`on_prompt_pcm`), which owns the bounded prompt bank (≤ 512 KB). A
 * prompt that cannot be rendered is reported through [onFailed]; the core keeps its earcon.
 */
class PromptRenderer(
    context: Context,
    private val onPcm: (String, List<Short>) -> Unit,
    private val onFailed: (String) -> Unit,
) {
    private val dir = File(context.cacheDir, "meshengine-prompts").apply { mkdirs() }
    private val pending = ArrayDeque<FfiPromptRequest>()
    private var ready = false
    private val tts: TextToSpeech = TextToSpeech(context.applicationContext) { status ->
        synchronized(this) {
            ready = status == TextToSpeech.SUCCESS
            if (!ready) pending.forEach { onFailed(it.key) }
            pending.clear()
        }
        if (status == TextToSpeech.SUCCESS) drain()
    }
    private val inFlight = HashMap<String, Pair<FfiPromptRequest, File>>()

    init {
        tts.setOnUtteranceProgressListener(object : UtteranceProgressListener() {
            override fun onStart(utteranceId: String) = Unit

            override fun onDone(utteranceId: String) = finish(utteranceId, true)

            @Deprecated("Deprecated in Java")
            override fun onError(utteranceId: String) = finish(utteranceId, false)

            override fun onError(utteranceId: String, errorCode: Int) = finish(utteranceId, false)
        })
    }

    /** `PlatformHooks.render_prompts` (engine thread): queued, rendered asynchronously. */
    fun render(reqs: List<FfiPromptRequest>) {
        synchronized(this) { pending.addAll(reqs) }
        drain()
    }

    fun shutdown() = tts.shutdown()

    private fun drain() {
        val batch = synchronized(this) {
            if (!ready) return
            val b = pending.toList()
            pending.clear()
            b
        }
        for (r in batch) {
            val file = File(dir, "p${r.key.hashCode().toUInt()}.wav")
            synchronized(this) { inFlight[r.key] = r to file }
            tts.language = Locale.forLanguageTag(r.lang)
            val params = Bundle().apply { putFloat(TextToSpeech.Engine.KEY_PARAM_VOLUME, 1f) }
            if (tts.synthesizeToFile(r.text, params, file, r.key) != TextToSpeech.SUCCESS) finish(r.key, false)
        }
    }

    private fun finish(key: String, ok: Boolean) {
        val (_, file) = synchronized(this) { inFlight.remove(key) } ?: return
        val pcm = if (ok) runCatching { Wav.toMono16k(file.readBytes()) }.getOrNull() else null
        file.delete()
        if (pcm == null || pcm.isEmpty()) onFailed(key) else onPcm(key, pcm.toList())
    }
}

/** Minimal PCM WAV decoding and resampling to the RT rate (16 kHz mono int16). */
object Wav {
    const val TARGET_RATE = 16_000

    data class Pcm(val rate: Int, val channels: Int, val samples: ShortArray)

    /** Parses a RIFF/WAVE file with 16-bit PCM data; null when it is anything else. */
    fun parse(bytes: ByteArray): Pcm? {
        if (bytes.size < 12 || String(bytes, 0, 4) != "RIFF" || String(bytes, 8, 4) != "WAVE") return null
        val buf = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        var pos = 12
        var rate = 0
        var channels = 0
        var bits = 0
        while (pos + 8 <= bytes.size) {
            val id = String(bytes, pos, 4)
            val len = buf.getInt(pos + 4)
            val body = pos + 8
            if (len < 0 || body + len > bytes.size && id != "data") return null
            when (id) {
                "fmt " -> {
                    if (buf.getShort(body).toInt() != 1) return null
                    channels = buf.getShort(body + 2).toInt()
                    rate = buf.getInt(body + 4)
                    bits = buf.getShort(body + 14).toInt()
                }
                "data" -> {
                    if (bits != 16 || channels < 1 || rate <= 0) return null
                    val end = minOf(bytes.size, body + len)
                    val n = (end - body) / 2
                    val out = ShortArray(n) { buf.getShort(body + it * 2) }
                    return Pcm(rate, channels, out)
                }
            }
            pos = body + len + (len and 1)
        }
        return null
    }

    /** Down-mixes to mono and linearly resamples to [rate]. */
    fun resample(pcm: Pcm, rate: Int = TARGET_RATE): ShortArray {
        val frames = pcm.samples.size / pcm.channels
        val mono = FloatArray(frames) { f ->
            var acc = 0f
            for (c in 0 until pcm.channels) acc += pcm.samples[f * pcm.channels + c]
            acc / pcm.channels
        }
        if (pcm.rate == rate) return ShortArray(frames) { mono[it].toInt().toShort() }
        val outLen = (frames.toLong() * rate / pcm.rate).toInt()
        val step = pcm.rate.toDouble() / rate
        return ShortArray(outLen) { i ->
            val x = i * step
            val i0 = x.toInt().coerceAtMost(frames - 1)
            val i1 = (i0 + 1).coerceAtMost(frames - 1)
            val t = (x - i0).toFloat()
            (mono[i0] * (1 - t) + mono[i1] * t).toInt().coerceIn(-32768, 32767).toShort()
        }
    }

    fun toMono16k(bytes: ByteArray): ShortArray? = parse(bytes)?.let { resample(it) }
}
