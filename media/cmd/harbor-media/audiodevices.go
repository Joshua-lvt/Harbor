// Audio device enumeration and selection through miniaudio.
//
// The worker lists the session's real capture and playback devices and opens
// the pair a call uses by stable identifier. A device switch on a live call
// goes through switchableAudio: the new devices are opened first (a failure
// keeps the old pair), then the pipeline's boundary swaps underneath it and
// blocked readers wake on the replacement without losing the call.
package main

import (
	"encoding/hex"
	"fmt"
	"sync"

	"github.com/gen2brain/malgo"
)

// audioDeviceDescription is the wire shape of one enumerated device. The id
// is the miniaudio device id in hex; an empty id selects the session default.
type audioDeviceDescription struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	IsDefault bool   `json:"is_default"`
}

// deviceSelection names the concrete capture and playback devices a call
// should open. Empty means the platform default.
type deviceSelection struct {
	InputID  string
	OutputID string
}

// listAudioDevices enumerates the session's real devices. Enumeration needs
// its own short-lived miniaudio context; the call pipeline's context is not
// running while no call exists.
func listAudioDevices() (inputs, outputs []audioDeviceDescription, err error) {
	context, err := malgo.InitContext(nil, malgo.ContextConfig{}, nil)
	if err != nil {
		return nil, nil, err
	}
	defer context.Free()
	inputs, err = devicesOfKind(context, malgo.Capture)
	if err != nil {
		return nil, nil, err
	}
	outputs, err = devicesOfKind(context, malgo.Playback)
	if err != nil {
		return nil, nil, err
	}
	return inputs, outputs, nil
}

func devicesOfKind(context *malgo.AllocatedContext, kind malgo.DeviceType) ([]audioDeviceDescription, error) {
	infos, err := context.Devices(kind)
	if err != nil {
		return nil, err
	}
	devices := make([]audioDeviceDescription, 0, len(infos))
	for i := range infos {
		devices = append(devices, audioDeviceDescription{
			ID:        infos[i].ID.String(),
			Name:      infos[i].Name(),
			IsDefault: infos[i].IsDefault != 0,
		})
	}
	return devices, nil
}

// parseDeviceID turns a listed device's hex id back into the miniaudio
// identifier the opener needs. An empty string yields nil (the default).
func parseDeviceID(hexID string) (*malgo.DeviceID, error) {
	if hexID == "" {
		return nil, nil
	}
	raw, err := hex.DecodeString(hexID)
	if err != nil {
		return nil, fmt.Errorf("device id is not hexadecimal")
	}
	if len(raw) == 0 || len(raw) > len(malgo.DeviceID{}) {
		return nil, fmt.Errorf("device id length is invalid")
	}
	var id malgo.DeviceID
	copy(id[:], raw)
	return &id, nil
}

// switchableAudio is a hot-swappable audioIO boundary. The voice pipeline
// holds it for a call's whole life; a device switch opens the replacement
// first and only then retires the current pair, so a failed switch changes
// nothing and a successful one never blocks a reader on a dead device.
type switchableAudio struct {
	mu  sync.Mutex
	cur audioIO
}

func newSwitchableAudio(first audioIO) *switchableAudio {
	return &switchableAudio{cur: first}
}

// Read blocks on the current device. If a swap retires that device while we
// are blocked (its Close unblocks us with an error), the read transparently
// retries against the replacement.
func (w *switchableAudio) Read() ([]byte, error) {
	for {
		w.mu.Lock()
		cur := w.cur
		w.mu.Unlock()
		if cur == nil {
			return nil, errAudioStopped
		}
		frame, err := cur.Read()
		if err == nil {
			return frame, nil
		}
		w.mu.Lock()
		same := w.cur == cur
		w.mu.Unlock()
		if same {
			return nil, err
		}
	}
}

// Write mirrors Read's swap tolerance for the playback path.
func (w *switchableAudio) Write(frame []byte) error {
	for {
		w.mu.Lock()
		cur := w.cur
		w.mu.Unlock()
		if cur == nil {
			return errAudioStopped
		}
		err := cur.Write(frame)
		if err == nil {
			return nil
		}
		w.mu.Lock()
		same := w.cur == cur
		w.mu.Unlock()
		if same {
			return err
		}
	}
}

// Swap opens the replacement before closing the current devices: a failed
// open leaves the call's audio untouched.
func (w *switchableAudio) Swap(open func() (audioIO, error)) error {
	next, err := open()
	if err != nil {
		return err
	}
	w.mu.Lock()
	old := w.cur
	w.cur = next
	w.mu.Unlock()
	if old != nil {
		old.Close()
	}
	return nil
}

func (w *switchableAudio) Close() {
	w.mu.Lock()
	old := w.cur
	w.cur = nil
	w.mu.Unlock()
	if old != nil {
		old.Close()
	}
}
