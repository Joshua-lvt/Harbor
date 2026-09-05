// IPC command handlers for audio configuration and the activity channel.
//
// The worker is transport and device plumbing only: it lists what the
// session really offers, applies the core's device and volume choices to
// Harbor's own streams, and moves bounded activity frames across the direct
// control channel. It never decides policy and stores nothing.
package main

import (
	"encoding/json"
)

const (
	// activityFrameMaxBytes bounds one control-channel activity frame.
	activityFrameMaxBytes = 8 * 1024
	// activityInboxCapacity bounds how many inbound activity frames wait for
	// the core's next poll; beyond that the frame is refused, not queued.
	activityInboxCapacity = 32
)

// applyCallAudioConfig seeds the service state from the core's persisted
// audio configuration at call-open time. Pointer fields mean "absent": an
// absent value keeps whatever is already selected, so a call between two
// workers that exchanged no configuration still uses the session defaults.
func (s *service) applyCallAudioConfig(inputDevice, outputDevice *string, inputVolume, outputVolume *float64) {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	if inputDevice != nil {
		s.selectedDevices.InputID = *inputDevice
	}
	if outputDevice != nil {
		s.selectedDevices.OutputID = *outputDevice
	}
	if inputVolume != nil {
		s.inputVolume = clampGain(*inputVolume)
	}
	if outputVolume != nil {
		s.outputVolume = clampGain(*outputVolume)
	}
}

// audioDevices serves audio.devices: the session's real devices.
func (s *service) audioDevices(request envelope) envelope {
	inputs, outputs, err := listAudioDevices()
	if err != nil {
		return errorFor(request, "audio_unavailable", "error.call.audioUnavailable",
			"audio devices could not be enumerated", true)
	}
	return responseFor(request, map[string]any{"inputs": inputs, "outputs": outputs})
}

// audioConfig gets or sets the call audio configuration: device selection
// and the per-stream volumes. A set while a call is live also swaps that
// call's devices; volumes apply per frame without any swap.
func (s *service) audioConfig(request envelope) envelope {
	var payload struct {
		Set          *bool    `json:"set"`
		InputDevice  *string  `json:"input_device"`
		OutputDevice *string  `json:"output_device"`
		InputVolume  *float64 `json:"input_volume"`
		OutputVolume *float64 `json:"output_volume"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest",
			"audio config payload is invalid", false)
	}

	apply := payload.Set != nil && *payload.Set
	var open func() (audioIO, error)

	s.stateMu.Lock()
	if apply {
		if payload.InputDevice != nil {
			s.selectedDevices.InputID = *payload.InputDevice
		}
		if payload.OutputDevice != nil {
			s.selectedDevices.OutputID = *payload.OutputDevice
		}
		if payload.InputVolume != nil {
			s.inputVolume = clampGain(*payload.InputVolume)
		}
		if payload.OutputVolume != nil {
			s.outputVolume = clampGain(*payload.OutputVolume)
		}
		selection := s.selectedDevices
		factory := s.audioFactory
		open = func() (audioIO, error) {
			if factory == nil {
				return openSilentAudio()
			}
			return factory(selection)
		}
	}
	inputID, outputID := s.selectedDevices.InputID, s.selectedDevices.OutputID
	inputVolume, outputVolume := s.inputVolume, s.outputVolume
	voice := s.voice
	s.stateMu.Unlock()

	if apply && voice != nil {
		// A live call swaps through the pipeline's boundary; a failed open
		// leaves the current devices running and is reported honestly.
		if err := voice.switchDevices(open); err != nil {
			return errorFor(request, "audio_unavailable", "error.audio.deviceUnavailable",
				"the selected audio devices could not be opened", true)
		}
	}
	return responseFor(request, map[string]any{
		"input_device":  inputID,
		"output_device": outputID,
		"input_volume":  inputVolume,
		"output_volume": outputVolume,
	})
}

// audioSwitch swaps a live call's devices without touching the durable
// selection unless the switch actually succeeded.
func (s *service) audioSwitch(request envelope) envelope {
	var payload struct {
		CallID       string `json:"call_id"`
		InputDevice  string `json:"input_device"`
		OutputDevice string `json:"output_device"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest",
			"audio switch payload is invalid", false)
	}
	selection := deviceSelection{InputID: payload.InputDevice, OutputID: payload.OutputDevice}
	if _, err := parseDeviceID(selection.InputID); err != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", err.Error(), false)
	}
	if _, err := parseDeviceID(selection.OutputID); err != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", err.Error(), false)
	}

	s.stateMu.Lock()
	factory := s.audioFactory
	voice := s.voice
	callID := s.callID
	s.stateMu.Unlock()
	if voice == nil || callID != payload.CallID {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := voice.switchDevices(func() (audioIO, error) {
		if factory == nil {
			return openSilentAudio()
		}
		return factory(selection)
	}); err != nil {
		return errorFor(request, "audio_unavailable", "error.audio.deviceUnavailable",
			"the selected audio devices could not be opened", true)
	}
	// Only a successful swap becomes the durable selection.
	s.stateMu.Lock()
	s.selectedDevices = selection
	s.stateMu.Unlock()
	return responseFor(request, map[string]any{"switched": true})
}

// activitySend forwards one bounded activity frame to the peer over the
// direct control channel. The core has already sanitized everything in it.
func (s *service) activitySend(request envelope) envelope {
	var payload struct {
		CallID string `json:"call_id"`
		Events string `json:"events"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.Events == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest",
			"activity payload is invalid", false)
	}
	if len(payload.Events) > activityFrameMaxBytes {
		return errorFor(request, "activity_too_large", "error.activity.tooLarge",
			"activity frame exceeds the direct control limit", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.sendActivity(payload.Events); err != nil {
		return errorFor(request, "channel_unavailable", "error.activity.channelUnavailable",
			err.Error(), true)
	}
	return responseFor(request, map[string]any{"sent": true})
}

// activityPoll drains inbound activity frames for the core.
func (s *service) activityPoll(request envelope) envelope {
	set := s.currentChannels()
	if set == nil {
		return responseFor(request, map[string]any{"events": []string{}})
	}
	return responseFor(request, map[string]any{"events": set.activityInbox()})
}

// sendActivity writes one activity frame on the control channel.
func (c *channelSet) sendActivity(events string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.control == nil {
		return errChannelClosed
	}
	if len(events) > activityFrameMaxBytes {
		return errFrameInvalid
	}
	c.sendControlLocked(directFrame{
		Version: directFrameVersion, Kind: "control", Action: "activity", Data: events,
	})
	return nil
}

// activityInbox drains bounded inbound activity frames (raw sanitized JSON).
func (c *channelSet) activityInbox() []string {
	c.mu.Lock()
	defer c.mu.Unlock()
	events := c.activity
	c.activity = nil
	return events
}
