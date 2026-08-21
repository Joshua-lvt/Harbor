package com.harbor.mobile.core

import com.harbor.mobile.net.IncomingMessage
import com.harbor.mobile.net.PresenceState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HarborStateTest {
    @Test fun filtersOtherTargetsAndRejectsStaleEvents() {
        val state = HarborState("pc-a")
        assertFalse(state.apply(IncomingMessage.Activity("pc-b", "private.exe", 20.0), nowMs = 0))
        assertTrue(state.apply(IncomingMessage.Activity("pc-a", "new.exe", 20.0), nowMs = 0))
        assertFalse(state.apply(IncomingMessage.Activity("pc-a", "old.exe", 19.0), nowMs = 0))
        assertEquals("new.exe", state.status.value.activity)
        assertEquals(20_000L, state.status.value.lastActivityTsMs)
    }

    @Test fun presenceUsesEpochSecondsAndOfflineState() {
        val state = HarborState("pc")
        state.apply(IncomingMessage.Presence("pc", PresenceState.ONLINE, 10.25, null))
        state.apply(IncomingMessage.Activity("pc", "editor.exe", 10.5))
        assertEquals(ConnectionStatus.ONLINE, state.status.value.connection)
        assertEquals(10_500L, state.status.value.lastEventTsMs)
        state.apply(IncomingMessage.Presence("pc", PresenceState.OFFLINE, 11.0, 10.5))
        assertEquals(ConnectionStatus.OFFLINE, state.status.value.connection)
        assertEquals(null, state.status.value.activity)
    }
}
