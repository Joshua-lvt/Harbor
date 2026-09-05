// IPC command handlers for the public-profile channel.
//
// The worker is transport plumbing only: it moves bounded, opaque profile
// frames across the direct control channel and stores nothing. Every policy
// decision — what a profile may contain, which revision wins, how an avatar
// is chunked, reassembled, hashed and cached — belongs to the Rust core.
// The core has already sanitized everything it hands here, and it validates
// everything this inbox returns before any UI ever sees it.
package main

import (
	"encoding/json"
)

const (
	// profileFrameMaxBytes bounds one control-channel profile frame. A full
	// profile with a small inline avatar fits; larger avatars travel as
	// peer-paced chunk frames the core fragments and reassembles itself.
	profileFrameMaxBytes = 8 * 1024
	// profileInboxCapacity bounds how many inbound profile frames wait for
	// the core's next poll; beyond that the frame is refused, not queued.
	profileInboxCapacity = 32
)

// profileSend forwards one bounded profile frame to the peer over the
// direct control channel. The bytes are opaque here.
func (s *service) profileSend(request envelope) envelope {
	var payload struct {
		CallID string `json:"call_id"`
		Frame  string `json:"frame"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.Frame == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest",
			"profile payload is invalid", false)
	}
	if len(payload.Frame) > profileFrameMaxBytes {
		return errorFor(request, "profile_too_large", "error.profile.tooLarge",
			"profile frame exceeds the direct control limit", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.sendProfile(payload.Frame); err != nil {
		return errorFor(request, "channel_unavailable", "error.profile.channelUnavailable",
			err.Error(), true)
	}
	return responseFor(request, map[string]any{"sent": true})
}

// profilePoll drains inbound profile frames for the core.
func (s *service) profilePoll(request envelope) envelope {
	set := s.currentChannels()
	if set == nil {
		return responseFor(request, map[string]any{"frames": []string{}})
	}
	return responseFor(request, map[string]any{"frames": set.profileInbox()})
}

// sendProfile writes one profile frame on the control channel.
func (c *channelSet) sendProfile(frame string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.control == nil {
		return errChannelClosed
	}
	if len(frame) > profileFrameMaxBytes {
		return errFrameInvalid
	}
	c.sendControlLocked(directFrame{
		Version: directFrameVersion, Kind: "control", Action: "profile", Data: frame,
	})
	return nil
}

// profileInbox drains bounded inbound profile frames (raw opaque JSON).
func (c *channelSet) profileInbox() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	frames := c.profile
	c.profile = nil
	return frames
}
