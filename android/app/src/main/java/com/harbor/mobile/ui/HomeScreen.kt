package com.harbor.mobile.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.harbor.mobile.core.ConnectionStatus
import com.harbor.mobile.net.PresenceState
import com.harbor.mobile.service.HarborRuntime

@Composable
fun HomeScreen(
    notificationEnabled: Boolean = true,
    modifier: Modifier = Modifier,
) {
    val status by HarborRuntime.status.collectAsStateWithLifecycle()
    val connection = when {
        status.connection == ConnectionStatus.CONNECTING -> "🟡 Conectando"
        status.connection == ConnectionStatus.OFFLINE -> "⚫ PC offline"
        status.presence == PresenceState.ONLINE -> "🟢 PC online"
        status.presence == PresenceState.AWAY -> "🟡 PC ausente"
        else -> "⚫ PC offline"
    }
    Column(
        modifier = modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Harbor Mobile", style = MaterialTheme.typography.headlineSmall)
        Text(connection, style = MaterialTheme.typography.titleMedium)
        Text(
            if (status.presence != PresenceState.OFFLINE && status.activity != null) {
                "🖥️ Usando ${status.activity}"
            } else {
                "Nenhuma atividade recebida"
            },
        )
        if (notificationEnabled) {
            Text(
                "A conexão continua em uma notificação persistente enquanto o serviço estiver ativo.",
                style = MaterialTheme.typography.bodySmall,
            )
        } else {
            Text(
                "Permita as notificações do Harbor para iniciar a conexão persistente.",
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}
