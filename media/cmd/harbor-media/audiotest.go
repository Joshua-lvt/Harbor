// Microphone loopback test: hear your own voice outside a call.
//
// The test opens the session's selected capture/playback pair (the same
// boundary calls use), plays the microphone back with a short delay so the
// speaker hears themselves distinctly, and reports a smoothed level the UI
// renders live. It owns no call state, joins no session, and refuses to run
// while a call owns the devices — or when the session has no capture
// device at all — instead of faking a level.
package main

import (
	"encoding/json"
	"sync"
	"time"
)

const (
	// loopbackDelayFrames holds ~300 ms so the playback is recognizably
	// your own voice, not instantaneous feedback.
	loopbackDelayFrames = 30
	loopbackMinSeconds  = 3
	loopbackMaxSeconds  = 15
	loopbackDefaultSecs = 5
)

// loopbackTest is one microphone self-check. The command goroutine owns
// lifecycle; the pump goroutine owns the devices. All shared facts travel
// under mu so a poll can never observe a torn snapshot.
type loopbackTest struct {
	mu          sync.Mutex
	active      bool
	failed      bool
	level       float64
	peak        float64
	secondsLeft int
	stop        chan struct{}
	finished    chan struct{}
}

func newLoopbackTest() *loopbackTest {
	return &loopbackTest{}
}

func (s *service) loopbackSnapshot() map[string]any {
	s.loopback.mu.Lock()
	defer s.loopback.mu.Unlock()
	return map[string]any{
		"active":       s.loopback.active,
		"level":        s.loopback.level,
		"seconds_left": s.loopback.secondsLeft,
		"peak":         s.loopback.peak,
		"failed":       s.loopback.failed,
	}
}

// loopbackStart begins a bounded self-check. It refuses honestly when a
// call owns the devices or when the session cannot even enumerate a
// capture device — a silent level with a green light would be a lie.
func (s *service) loopbackStart(request envelope) envelope {
	var payload struct {
		Seconds int `json:"seconds"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest",
			"loopback payload is invalid", false)
	}
	seconds := payload.Seconds
	if seconds <= 0 {
		seconds = loopbackDefaultSecs
	}
	if seconds < loopbackMinSeconds {
		seconds = loopbackMinSeconds
	}
	if seconds > loopbackMaxSeconds {
		seconds = loopbackMaxSeconds
	}

	s.stateMu.Lock()
	if s.voice != nil {
		s.stateMu.Unlock()
		return errorFor(request, "call_active", "error.audio.busy",
			"microphone test refuses while a call owns the devices", false)
	}
	selection := s.selectedDevices
	s.stateMu.Unlock()

	s.loopback.mu.Lock()
	if s.loopback.active {
		s.loopback.mu.Unlock()
		return errorFor(request, "test_active", "error.audio.busy",
			"a microphone test is already running", false)
	}
	s.loopback.mu.Unlock()

	if s.audioFactory == nil {
		return errorFor(request, "audio_unavailable", "error.audio.deviceUnavailable",
			"no audio boundary to test with", false)
	}
	if inputs, _, err := listAudioDevices(); err != nil || len(inputs) == 0 {
		return errorFor(request, "audio_unavailable", "error.audio.deviceUnavailable",
			"no capture device to test with", false)
	}
	audio, err := s.audioFactory(selection)
	if err != nil {
		return errorFor(request, "audio_unavailable", "error.audio.deviceUnavailable",
			"the selected devices could not be opened", true)
	}

	s.loopback.mu.Lock()
	s.loopback.active = true
	s.loopback.failed = false
	s.loopback.level = 0
	s.loopback.peak = 0
	s.loopback.secondsLeft = seconds
	s.loopback.stop = make(chan struct{})
	s.loopback.finished = make(chan struct{})
	s.loopback.mu.Unlock()

	s.stateMu.Lock()
	inputGain := s.inputVolume
	outputGain := s.outputVolume
	s.stateMu.Unlock()

	go s.runLoopback(audio, seconds, inputGain, outputGain)
	return responseFor(request, s.loopbackSnapshot())
}

// loopbackPoll reports the running self-check without touching devices.
func (s *service) loopbackPoll(request envelope) envelope {
	return responseFor(request, s.loopbackSnapshot())
}

// loopbackStop ends the self-check early and reports the measured peak.
func (s *service) loopbackStop(request envelope) envelope {
	s.loopback.mu.Lock()
	active := s.loopback.active
	stop := s.loopback.stop
	finished := s.loopback.finished
	if active {
		close(stop)
	}
	s.loopback.mu.Unlock()
	if active {
		select {
		case <-finished:
		case <-time.After(500 * time.Millisecond):
		}
	}
	snapshot := s.loopbackSnapshot()
	snapshot["stopped"] = true
	return responseFor(request, snapshot)
}

// runLoopback pumps microphone frames to the speaker with a short delay
// until the budget runs out, the UI stops it, or a device dies. Levels are
// measured on what the microphone really captured.
func (s *service) runLoopback(audio audioIO, seconds int, inputGain, outputGain float64) {
	defer audio.Close()
	defer func() {
		s.loopback.mu.Lock()
		s.loopback.active = false
		s.loopback.level = 0
		close(s.loopback.finished)
		s.loopback.mu.Unlock()
	}()

	deadline := time.Now().Add(time.Duration(seconds) * time.Second)
	delay := make([][]byte, 0, loopbackDelayFrames)
	pcm := make([]int16, frameSamples*audioChannels)
	detector := &speechDetector{}

	for {
		select {
		case <-s.loopback.stop:
			return
		default:
		}
		if time.Now().After(deadline) {
			return
		}
		frame, err := audio.Read()
		if err != nil || len(frame) != frameBytes {
			s.loopback.mu.Lock()
			s.loopback.failed = true
			s.loopback.mu.Unlock()
			return
		}
		pcmFromBytes(frame, pcm)
		detector.observe(frameLevel(pcm))
		level := detector.level
		if level > 0 {
			applyGain(pcm, inputGain)
		}
		delayed := pcmToBytes(pcm)
		if outputGain != 1 {
			var delayedPCM = make([]int16, len(pcm))
			pcmFromBytes(delayed, delayedPCM)
			applyGain(delayedPCM, outputGain)
			delayed = pcmToBytes(delayedPCM)
		}
		delay = append(delay, delayed)
		var play []byte
		if len(delay) > loopbackDelayFrames {
			play = delay[0]
			delay = delay[1:]
		} else {
			play = make([]byte, frameBytes)
		}
		if err := audio.Write(play); err != nil {
			s.loopback.mu.Lock()
			s.loopback.failed = true
			s.loopback.mu.Unlock()
			return
		}
		remaining := int(time.Until(deadline).Seconds() + 0.5)
		if remaining < 0 {
			remaining = 0
		}
		s.loopback.mu.Lock()
		s.loopback.level = level
		if level > s.loopback.peak {
			s.loopback.peak = level
		}
		s.loopback.secondsLeft = remaining
		s.loopback.mu.Unlock()
	}
}
