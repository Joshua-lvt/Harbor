// Device-bound audio capture and playback with an explicit device choice.
//
// openDeviceAudio opens the session defaults; openSelectedAudio opens the
// concrete capture/playback pair a deviceSelection names. Both are the same
// miniaudio boundary the voice pipeline has always consumed — only the
// device ids differ — so selection is configuration, not a second pipeline.
package main

import (
	"github.com/gen2brain/malgo"
)

// openSelectedAudio opens the real microphone and speaker named by the
// selection (empty ids mean the session defaults).
func openSelectedAudio(selection deviceSelection) (audioIO, error) {
	captureID, err := parseDeviceID(selection.InputID)
	if err != nil {
		return nil, err
	}
	playbackID, err := parseDeviceID(selection.OutputID)
	if err != nil {
		return nil, err
	}
	return openDeviceAudioWith(captureID, playbackID)
}

// openDeviceAudioWith is the single device-opening path: default devices when
// both ids are nil, the named pair otherwise.
func openDeviceAudioWith(captureID, playbackID *malgo.DeviceID) (audioIO, error) {
	context, err := malgo.InitContext(nil, malgo.ContextConfig{}, nil)
	if err != nil {
		return nil, err
	}
	audio := &deviceAudio{
		context:        context,
		captured:       make(chan []byte, ringFrames),
		playbackFrames: make(chan []byte, ringFrames),
		stop:           make(chan struct{}),
	}

	captureConfig := malgo.DefaultDeviceConfig(malgo.Capture)
	captureConfig.SampleRate = audioSampleRate
	captureConfig.Capture.Format = malgo.FormatS16
	captureConfig.Capture.Channels = audioChannels
	captureConfig.PeriodSizeInMilliseconds = 10
	if captureID != nil {
		id := *captureID
		captureConfig.Capture.DeviceID = id.Pointer()
		// The binding allocated the id in C memory; InitDevice copies it, so
		// the allocation is released as soon as init returns either way.
		defer harborFreeCBytes(captureConfig.Capture.DeviceID)
	}
	capture, err := malgo.InitDevice(context.Context, captureConfig, malgo.DeviceCallbacks{
		Data: func(_, input []byte, _ uint32) {
			if len(input) == 0 {
				return
			}
			frame := make([]byte, len(input))
			copy(frame, input)
			select {
			case audio.captured <- frame:
			default: // the ring is full: drop now instead of growing latency
			}
		},
	})
	if err != nil {
		context.Free()
		return nil, err
	}
	if err := capture.Start(); err != nil {
		capture.Uninit()
		context.Free()
		return nil, err
	}
	audio.capture = capture

	playbackConfig := malgo.DefaultDeviceConfig(malgo.Playback)
	playbackConfig.SampleRate = audioSampleRate
	playbackConfig.Playback.Format = malgo.FormatS16
	playbackConfig.Playback.Channels = audioChannels
	playbackConfig.PeriodSizeInMilliseconds = 10
	if playbackID != nil {
		id := *playbackID
		playbackConfig.Playback.DeviceID = id.Pointer()
		defer harborFreeCBytes(playbackConfig.Playback.DeviceID)
	}
	playback, err := malgo.InitDevice(context.Context, playbackConfig, malgo.DeviceCallbacks{
		Data: func(output, _ []byte, _ uint32) {
			// The buffer arrives zeroed: an underrun is honest silence.
			for filled := 0; filled+frameBytes <= len(output); filled += frameBytes {
				select {
				case frame := <-audio.playbackFrames:
					copy(output[filled:], frame)
				default:
					return
				}
			}
		},
	})
	if err != nil {
		capture.Stop()
		capture.Uninit()
		context.Free()
		return nil, err
	}
	if err := playback.Start(); err != nil {
		capture.Stop()
		capture.Uninit()
		playback.Uninit()
		context.Free()
		return nil, err
	}
	audio.playback = playback
	return audio, nil
}
