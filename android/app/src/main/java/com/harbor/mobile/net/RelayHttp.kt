package com.harbor.mobile.net

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.IOException
import java.util.UUID

class RelayHttp(private val client: OkHttpClient = OkHttpClient()) {
    suspend fun register(relayUrl: String, deviceId: String = newDeviceId()): Registration =
        post(relayUrl, "/register", JSONObject().put("device_id", deviceId)).let { body ->
            Registration(deviceId, body.required("device_secret"))
        }

    suspend fun connectMobile(
        relayUrl: String,
        deviceId: String,
        deviceSecret: String,
        mobileCode: String,
    ): MobileBinding = post(
        relayUrl,
        "/connect_mobile",
        JSONObject()
            .put("device_id", deviceId)
            .put("device_secret", deviceSecret)
            .put("mobile_code", mobileCode),
    ).let { body -> MobileBinding(body.required("target_id")) }

    private suspend fun post(relayUrl: String, path: String, payload: JSONObject): JSONObject = withContext(Dispatchers.IO) {
        val url = relayHttpUrl(relayUrl).newBuilder()
            .addPathSegments(path.removePrefix("/"))
            .build()
        val request = Request.Builder()
            .url(url)
            .post(payload.toString().toRequestBody(JSON_MEDIA_TYPE))
            .header("Accept", "application/json")
            .build()
        val response = try {
            client.newCall(request).execute()
        } catch (e: IOException) {
            throw RelayException("Não foi possível alcançar o relay", cause = e)
        }
        response.use { res ->
            val text = res.body?.string().orEmpty()
            val body = runCatching { JSONObject(text) }.getOrElse {
                throw RelayException("Resposta inválida do relay (HTTP ${res.code})", statusCode = res.code)
            }
            if (!res.isSuccessful) {
                throw when (res.code) {
                    401 -> RelayAuthException("A identidade móvel foi recusada")
                    404 -> MobileCodeException("Código expirado, usado ou inexistente")
                    409 -> RelayConflictException(body.optString("error", "Dispositivo não pode ser observador"))
                    else -> RelayException(body.optString("error", "Relay respondeu HTTP ${res.code}"), res.code)
                }
            }
            return@withContext body
        }
    }

    companion object {
        private val JSON_MEDIA_TYPE = "application/json; charset=utf-8".toMediaType()
        fun newDeviceId(): String = "mobile-${UUID.randomUUID()}"
    }
}

data class Registration(val deviceId: String, val deviceSecret: String)
data class MobileBinding(val targetId: String)

open class RelayException(message: String, val statusCode: Int? = null, cause: Throwable? = null) : IOException(message, cause)
class RelayAuthException(message: String) : RelayException(message, 401)
class MobileCodeException(message: String) : RelayException(message, 404)
class RelayConflictException(message: String) : RelayException(message, 409)

private fun JSONObject.required(key: String): String = optString(key).takeIf { it.isNotEmpty() }
    ?: throw RelayException("Resposta do relay sem '$key'")
