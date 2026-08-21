package com.harbor.mobile.net

import org.junit.Assert.assertArrayEquals
import org.junit.Test

class RelayClientTest {
    @Test fun reconnectBackoffIsBoundedAtFifteenSeconds() {
        assertArrayEquals(longArrayOf(1_000L, 2_000L, 4_000L, 8_000L, 15_000L), RelayClient.RECONNECT_BACKOFF_MS)
    }
}
