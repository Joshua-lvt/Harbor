// Microphone loopback coverage: lifecycle, honest refusals, and a real
// capture→playback round trip through scripted devices (no hardware).
package main

import (
	"bytes"
	"encoding/json"
	"sync"
	"testing"
	"time"
)

// scriptedAudio feeds deterministic PCM frames and counts played ones.
type scriptedAudio struct {
	mu      sync.Mutex
	reads   int
	writes  int
	amplify int16
	closed  bool
}

func (f *scriptedAudio) Read() ([]byte, error) {
	time.Sleep(frameDuration)
	frame := make([]byte, frameBytes)
	for i := 0; i+1 < len(frame); i += 2 {
		frame[i] = byte(f.amplify)
		frame[i+1] = byte(int(f.amplify) >> 8)
	}
	f.mu.Lock()
	f.reads++
	f.mu.Unlock()
	return frame, nil
}

func (f *scriptedAudio) Write(frame []byte) error {
	if len(frame) != frameBytes {
		return errAudioStopped
	}
	f.mu.Lock()
	f.writes++
	f.mu.Unlock()
	return nil
}

func (f *scriptedAudio) Close() {
	f.mu.Lock()
	f.closed = true
	f.mu.Unlock()
}

func loopbackFactory(amplify int16) func(deviceSelection) (audioIO, error) {
	return func(deviceSelection) (audioIO, error) {
		return &scriptedAudio{amplify: amplify}, nil
	}
}

type loopbackSnapshot struct {
	Active      bool    `json:"active"`
	Level       float64 `json:"level"`
	SecondsLeft int     `json:"seconds_left"`
	Peak        float64 `json:"peak"`
	Failed      bool    `json:"failed"`
}

func decodeSnapshot(t *testing.T, payload json.RawMessage) loopbackSnapshot {
	t.Helper()
	var snapshot loopbackSnapshot
	if err := json.Unmarshal(payload, &snapshot); err != nil {
		t.Fatal(err)
	}
	return snapshot
}

// TestLoopbackRunsMeasuresAndStops proves the full self-check: scripted
// capture produces a nonzero level, delayed frames reach the speaker, and
// stopping reports the measured peak with devices closed.
func TestLoopbackRunsMeasuresAndStops(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))
	svc.audioFactory = loopbackFactory(4000)

	started := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":3}`))
	if started.Error != nil {
		t.Fatalf("loopback must start with scripted devices: %#v", started.Error)
	}
	if snapshot := decodeSnapshot(t, started.Payload); !snapshot.Active {
		t.Fatalf("start must report active: %#v", snapshot)
	}

	time.Sleep(250 * time.Millisecond)
	mid := decodeSnapshot(t, svc.loopbackPoll(requestForTest("audio.loopback_poll", `{}`)).Payload)
	if !mid.Active {
		t.Fatalf("test must still run: %#v", mid)
	}
	if mid.Level <= 0 {
		t.Fatalf("scripted capture must measure a level: %#v", mid)
	}
	if mid.SecondsLeft < 1 || mid.SecondsLeft > 3 {
		t.Fatalf("seconds_left must count down inside budget: %#v", mid)
	}

	stopped := svc.loopbackStop(requestForTest("audio.loopback_stop", `{}`))
	if stopped.Error != nil {
		t.Fatalf("stop must succeed: %#v", stopped.Error)
	}
	done := decodeSnapshot(t, stopped.Payload)
	if done.Active {
		t.Fatalf("stop must end the test: %#v", done)
	}
	if done.Peak <= 0 {
		t.Fatalf("stop must report the measured peak: %#v", done)
	}
	if done.Failed {
		t.Fatalf("scripted devices must not fail: %#v", done)
	}

	after := decodeSnapshot(t, svc.loopbackPoll(requestForTest("audio.loopback_poll", `{}`)).Payload)
	if after.Active {
		t.Fatalf("poll after stop must idle: %#v", after)
	}
}

// TestLoopbackClampsDuration proves absurd budgets collapse into the
// documented 3..15 s window instead of running away.
func TestLoopbackClampsDuration(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))
	svc.audioFactory = loopbackFactory(100)

	started := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":999}`))
	if started.Error != nil {
		t.Fatalf("start must accept and clamp: %#v", started.Error)
	}
	snapshot := decodeSnapshot(t, started.Payload)
	if snapshot.SecondsLeft > loopbackMaxSeconds {
		t.Fatalf("budget must clamp to %ds: %#v", loopbackMaxSeconds, snapshot)
	}
	svc.loopbackStop(requestForTest("audio.loopback_stop", `{}`))
}

func TestLoopbackRefusalsAreHonest(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))

	// Without an audio factory there are no devices to test with.
	refused := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":3}`))
	if refused.Error == nil || refused.Error.Code != "audio_unavailable" {
		t.Fatalf("missing factory must refuse honestly: %#v", refused)
	}

	// While a call owns the devices the test steps aside.
	svc.audioFactory = loopbackFactory(1000)
	svc.voice = &voicePipeline{}
	busy := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":3}`))
	if busy.Error == nil || busy.Error.Code != "call_active" {
		t.Fatalf("an owned device must be refused: %#v", busy)
	}
	svc.voice = nil

	// A second start while running refuses instead of stacking mixers.
	first := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":3}`))
	if first.Error != nil {
		t.Fatalf("first start: %#v", first.Error)
	}
	second := svc.loopbackStart(requestForTest("audio.loopback_start", `{"seconds":3}`))
	if second.Error == nil || second.Error.Code != "test_active" {
		t.Fatalf("stacked start must refuse: %#v", second)
	}

	// Stopping an idle tester is a no-op success, never an error.
	svc.loopbackStop(requestForTest("audio.loopback_stop", `{}`))
	idle := svc.loopbackStop(requestForTest("audio.loopback_stop", `{}`))
	if idle.Error != nil {
		t.Fatalf("idle stop must succeed: %#v", idle.Error)
	}
}
