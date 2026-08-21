package com.harbor.mobile.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

class ProtocolTest {
    @Test fun parsesPresenceAndConvertsNothingPrematurely() {
        val message = Protocol.decode("{\"type\":\"presence\",\"device_id\":\"pc\",\"state\":\"away\",\"ts\":12.5}")
        assertEquals(IncomingMessage.Presence("pc", PresenceState.AWAY, 12.5, null), message)
    }

    @Test fun parsesNullableActivityApp() {
        val message = Protocol.decode("{\"type\":\"activity\",\"device_id\":\"pc\",\"app\":null,\"ts\":3}")
        assertEquals(IncomingMessage.Activity("pc", null, 3.0), message)
    }

    @Test fun ignoresUnknownMalformedAndPeerCommands() {
        assertNull(Protocol.decode("not json"))
        assertNull(Protocol.decode("{\"type\":\"chat\",\"device_id\":\"pc\"}"))
        assertNull(Protocol.decode("{\"type\":\"presence\",\"device_id\":\"pc\",\"state\":\"what\"}"))
    }

    @Test fun acceptsWebSocketRelayUrlsForHttpAndWebSocketLayers() {
        assertEquals("https", relayHttpUrl("wss://relay.example").scheme)
        assertEquals("http", relayHttpUrl("ws://localhost:8787").scheme)
        assertThrows(IllegalArgumentException::class.java) {
            relayHttpUrl("http://public.example")
        }
    }
}
