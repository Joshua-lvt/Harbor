package com.harbor.mobile.storage

import android.content.Context
import android.util.Base64
import org.json.JSONObject
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties

/** Stores the mobile's own identity and target, never the PC identity. */
data class Credentials(
    val relayUrl: String,
    val deviceId: String,
    val deviceSecret: String,
    val targetId: String,
)

data class PendingRegistration(
    val relayUrl: String,
    val deviceId: String,
    val deviceSecret: String,
)

class CredentialsStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    @Synchronized
    fun load(): Credentials? = readJson(KEY_BLOB)?.toCredentials()

    @Synchronized
    fun loadPending(): PendingRegistration? = readJson(KEY_PENDING_BLOB)?.toPendingRegistration()

    @Synchronized
    fun save(credentials: Credentials) {
        writeJson(KEY_BLOB, JSONObject()
            .put("relay_url", credentials.relayUrl)
            .put("device_id", credentials.deviceId)
            .put("device_secret", credentials.deviceSecret)
            .put("target_id", credentials.targetId))
    }

    @Synchronized
    fun savePending(registration: PendingRegistration) {
        writeJson(KEY_PENDING_BLOB, JSONObject()
            .put("relay_url", registration.relayUrl)
            .put("device_id", registration.deviceId)
            .put("device_secret", registration.deviceSecret))
    }

    @Synchronized
    fun clearPending() {
        prefs.edit().remove(KEY_PENDING_BLOB).apply()
    }

    @Synchronized
    fun clear() {
        prefs.edit().remove(KEY_BLOB).remove(KEY_PENDING_BLOB).apply()
    }

    private fun readJson(prefKey: String): JSONObject? {
        val encoded = prefs.getString(prefKey, null) ?: return null
        return runCatching {
            val packed = Base64.decode(encoded, Base64.NO_WRAP)
            val buffer = ByteBuffer.wrap(packed)
            val ivLength = buffer.getInt()
            require(ivLength in 12..32)
            val iv = ByteArray(ivLength)
            buffer.get(iv)
            val ciphertext = ByteArray(buffer.remaining())
            buffer.get(ciphertext)
            val plain = cipher(Cipher.DECRYPT_MODE, key(), iv).doFinal(ciphertext)
            JSONObject(String(plain, Charsets.UTF_8))
        }.getOrNull()
    }

    private fun writeJson(prefKey: String, json: JSONObject) {
        val encrypted = cipher(Cipher.ENCRYPT_MODE, key()).run {
            iv to doFinal(json.toString().toByteArray(Charsets.UTF_8))
        }
        val packed = ByteBuffer.allocate(Int.SIZE_BYTES + encrypted.first.size + encrypted.second.size)
            .putInt(encrypted.first.size)
            .put(encrypted.first)
            .put(encrypted.second)
            .array()
        prefs.edit().putString(prefKey, Base64.encodeToString(packed, Base64.NO_WRAP)).apply()
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
                .build(),
        )
        return generator.generateKey()
    }

    private fun cipher(mode: Int, key: SecretKey, iv: ByteArray? = null): Cipher =
        Cipher.getInstance(TRANSFORMATION).apply {
            if (mode == Cipher.ENCRYPT_MODE) init(mode, key)
            else init(mode, key, GCMParameterSpec(128, requireNotNull(iv)))
        }

    private fun JSONObject.toCredentials() = Credentials(
        relayUrl = getString("relay_url"),
        deviceId = getString("device_id"),
        deviceSecret = getString("device_secret"),
        targetId = getString("target_id"),
    )

    private fun JSONObject.toPendingRegistration() = PendingRegistration(
        relayUrl = getString("relay_url"),
        deviceId = getString("device_id"),
        deviceSecret = getString("device_secret"),
    )

    private companion object {
        const val PREFS = "harbor_secure_credentials"
        const val KEY_BLOB = "encrypted_credentials"
        const val KEY_PENDING_BLOB = "encrypted_pending_registration"
        const val KEY_ALIAS = "harbor_mobile_credentials_v1"
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
