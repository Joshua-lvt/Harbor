package com.harbor.mobile.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.harbor.mobile.BuildConfig
import com.harbor.mobile.net.RelayException
import com.harbor.mobile.net.RelayHttp
import com.harbor.mobile.storage.Credentials
import com.harbor.mobile.storage.CredentialsStore
import com.harbor.mobile.storage.PendingRegistration
import kotlinx.coroutines.launch

@Composable
fun PairingScreen(
    store: CredentialsStore,
    onConnected: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var relayUrl by remember { mutableStateOf(BuildConfig.DEFAULT_RELAY_URL) }
    var mobileCode by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Column(
        modifier = modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.Center,
    ) {
        Text("Conectar ao Harbor PC", style = MaterialTheme.typography.headlineSmall)
        Spacer(Modifier.height(8.dp))
        Text("No Harbor do PC, gere um código em Configurações → Vincular celular.")
        Spacer(Modifier.height(20.dp))
        OutlinedTextField(
            value = relayUrl,
            onValueChange = { relayUrl = it },
            label = { Text("URL do relay") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        OutlinedTextField(
            value = mobileCode,
            onValueChange = { mobileCode = it.uppercase() },
            label = { Text("Código do celular") },
            placeholder = { Text("XXXX-XXXX") },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii),
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(16.dp))
        Button(
            onClick = {
                busy = true
                error = null
                scope.launch {
                    try {
                        val normalizedUrl = relayUrl.trim().removeSuffix("/")
                        val http = RelayHttp()
                        val pending = store.loadPending()?.takeIf { it.relayUrl == normalizedUrl }
                        val registration = pending?.let {
                            com.harbor.mobile.net.Registration(it.deviceId, it.deviceSecret)
                        } ?: http.register(normalizedUrl).also {
                            store.savePending(PendingRegistration(normalizedUrl, it.deviceId, it.deviceSecret))
                        }
                        val binding = http.connectMobile(
                            normalizedUrl,
                            registration.deviceId,
                            registration.deviceSecret,
                            mobileCode.trim(),
                        )
                        store.save(
                            Credentials(
                                relayUrl = normalizedUrl,
                                deviceId = registration.deviceId,
                                deviceSecret = registration.deviceSecret,
                                targetId = binding.targetId,
                            ),
                        )
                        store.clearPending()
                        onConnected()
                    } catch (e: RelayException) {
                        error = e.message ?: "Falha ao conectar"
                    } catch (e: Exception) {
                        error = "Falha ao conectar: ${e.message ?: "erro desconhecido"}"
                    } finally {
                        busy = false
                    }
                }
            },
            enabled = !busy && mobileCode.trim().isNotEmpty() && relayUrl.isNotBlank(),
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (busy) CircularProgressIndicator(strokeWidth = 2.dp) else Text("Conectar")
        }
        error?.let {
            Spacer(Modifier.height(12.dp))
            Text(it, color = MaterialTheme.colorScheme.error)
        }
        Spacer(Modifier.height(12.dp))
        Text("O código expira e só pode ser usado uma vez. O celular usa uma identidade própria e apenas observa presença e atividade.", style = MaterialTheme.typography.bodySmall)
    }
}

