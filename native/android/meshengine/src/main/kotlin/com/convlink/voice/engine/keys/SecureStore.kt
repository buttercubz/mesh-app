package com.convlink.voice.engine.keys

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import androidx.core.content.edit
import android.security.keystore.KeyProperties
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** AES-GCM with a 12-byte random nonce; `seal` output is `nonce ‖ ciphertext ‖ tag`. */
interface Aead {
    fun seal(plain: ByteArray, aad: ByteArray): ByteArray
    fun open(sealed: ByteArray, aad: ByteArray): ByteArray
}

/** Raw string storage behind the store (SharedPreferences on device, a map in tests). */
interface KeyValueStore {
    fun get(key: String): String?
    fun put(key: String, value: String?)
    fun keys(): Set<String>
}

/** AES-256-GCM over a [SecretKey] (Keystore-backed on device, software in JVM tests). */
class AesGcm(private val key: () -> SecretKey, private val random: SecureRandom = SecureRandom()) : Aead {
    override fun seal(plain: ByteArray, aad: ByteArray): ByteArray {
        val c = Cipher.getInstance(TRANSFORMATION)
        // Keystore keys generate their own IV (randomized encryption is enforced).
        c.init(Cipher.ENCRYPT_MODE, key())
        c.updateAAD(aad)
        val body = c.doFinal(plain)
        val iv = c.iv
        check(iv.size == NONCE_LEN) { "unexpected IV length ${iv.size}" }
        return iv + body
    }

    override fun open(sealed: ByteArray, aad: ByteArray): ByteArray {
        require(sealed.size > NONCE_LEN + TAG_LEN) { "sealed value too short" }
        val c = Cipher.getInstance(TRANSFORMATION)
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_LEN * 8, sealed, 0, NONCE_LEN))
        c.updateAAD(aad)
        return c.doFinal(sealed, NONCE_LEN, sealed.size - NONCE_LEN)
    }

    companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val NONCE_LEN = 12
        const val TAG_LEN = 16
    }
}

/**
 * Engine persistence (`PlatformHooks.persist`, per-group identities and epoch secrets):
 * every value is sealed with a non-exportable AndroidKeyStore AES-256-GCM key (convLink
 * vault pattern) and kept in the private SharedPreferences file `meshengine_secure`, which
 * the app's backup and data-extraction rules exclude (WP-18), so an identity is never
 * cloned to another device. The key name is bound as AAD, so values cannot be swapped.
 */
class SecureStore(private val aead: Aead, private val kv: KeyValueStore) {
    /** Stores (or with null, deletes) one value. */
    fun put(key: String, value: ByteArray?) {
        if (value == null) {
            kv.put(PREFIX + key, null)
            return
        }
        val sealed = aead.seal(value, aad(key))
        kv.put(PREFIX + key, Base64Codec.encode(sealed))
    }

    /** The value, or null when absent or no longer decryptable (key invalidated). */
    fun get(key: String): ByteArray? {
        val stored = kv.get(PREFIX + key) ?: return null
        return try {
            aead.open(Base64Codec.decode(stored), aad(key))
        } catch (_: Exception) {
            null
        }
    }

    /** Every readable entry (`FfiEngineConfig.persisted`); unreadable ones are dropped. */
    fun loadAll(): List<Pair<String, ByteArray>> = kv.keys().filter { it.startsWith(PREFIX) }.sorted()
        .mapNotNull { k ->
            val key = k.removePrefix(PREFIX)
            get(key)?.let { key to it } ?: run {
                kv.put(k, null)
                null
            }
        }

    private fun aad(key: String) = ("meshengine/v1/$key").toByteArray(Charsets.UTF_8)

    companion object {
        const val PREFS_NAME = "meshengine_secure"
        const val KEY_ALIAS = "meshengine.secure.v1"
        private const val PREFIX = "v1:"

        /** The device store: Keystore key + backup-excluded SharedPreferences. */
        fun create(context: Context): SecureStore {
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            return SecureStore(AesGcm({ keystoreKey() }), PrefsStore(prefs))
        }

        @Synchronized
        private fun keystoreKey(): SecretKey {
            val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            (ks.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
            val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            gen.init(
                KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .setRandomizedEncryptionRequired(true)
                    .build(),
            )
            return gen.generateKey()
        }
    }
}

/** SharedPreferences adapter (commit() so a crash right after `persist` keeps the value). */
class PrefsStore(private val prefs: SharedPreferences) : KeyValueStore {
    override fun get(key: String): String? = prefs.getString(key, null)

    override fun put(key: String, value: String?) {
        prefs.edit(commit = true) {
            if (value == null) remove(key) else putString(key, value)
        }
    }

    override fun keys(): Set<String> = prefs.all.keys
}

/** Base64 (java.util.Base64 exists on API 26+ and on the JVM). */
internal object Base64Codec {
    fun encode(b: ByteArray): String = java.util.Base64.getEncoder().encodeToString(b)

    fun decode(s: String): ByteArray = java.util.Base64.getDecoder().decode(s)
}
