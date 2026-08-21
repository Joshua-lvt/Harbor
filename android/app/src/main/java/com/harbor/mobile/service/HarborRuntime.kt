package com.harbor.mobile.service

import com.harbor.mobile.core.HarborStatus
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Process-local view for Compose; the foreground service remains the owner of the socket. */
object HarborRuntime {
    private val _status = MutableStateFlow(HarborStatus())
    val status: StateFlow<HarborStatus> = _status.asStateFlow()

    fun publish(status: HarborStatus) {
        _status.value = status
    }
}
