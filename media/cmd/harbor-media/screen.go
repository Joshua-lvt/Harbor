// Harbor's private screen-share pipeline.
//
// Capture is an OS boundary exactly like the audio devices: the production
// adapter captures the real screen on X11 sessions, and a Wayland session is
// an honest refusal — XWayland's root window would silently miss most of the
// screen, and a portal-based adapter is a later, deliberate addition. The
// frames are converted to I420, encoded with VP8, and written onto the call's
// video track, which the initial call negotiation already reserved — starting
// or stopping a share never renegotiates the audio track away.
package main

import (
	"context"
	"errors"
	"os"
	"sync"
	"time"
)

const (
	shareFPS        = 30
	shareKeyframe   = 5 * time.Second
	shareBitrateKb  = 2500
	shareRealtimeUS = 100000 // VPX_DL_REALTIME
)

// screenCapture is the platform boundary: fixed-size RGBA frames of the real
// screen, produced at the caller's cadence.
type screenCapture interface {
	// Size returns the capture dimensions; both are even (I420 needs it).
	Size() (width, height int)
	// NextFrame blocks until the next RGBA frame is available.
	NextFrame() ([]byte, error)
	Close()
}

// sharePipeline owns one active share: it pulls captured frames, encodes
// them, and writes them onto the call's video track. Stopping the share
// stops the pump; the call and the voice pipeline are untouched.
type sharePipeline struct {
	capture screenCapture
	encoder *vpxEncoder
	track   videoTrack
	cancel  context.CancelFunc
	done    sync.WaitGroup
}

// videoTrack is the narrow slice of the Pion track the pipeline needs; the
// narrow type keeps the pipeline testable against a recording stub.
type videoTrack interface {
	WriteEncoded(frame []byte, duration time.Duration) error
}

func newSharePipeline(capture screenCapture, encoder *vpxEncoder, track videoTrack) *sharePipeline {
	ctx, cancel := context.WithCancel(context.Background())
	share := &sharePipeline{
		capture: capture, encoder: encoder, track: track, cancel: cancel,
	}
	share.done.Add(1)
	go share.sendLoop(ctx)
	return share
}

func (p *sharePipeline) sendLoop(ctx context.Context) {
	defer p.done.Done()
	ticker := time.NewTicker(time.Second / shareFPS)
	defer ticker.Stop()
	keyframeEvery := shareFPS * int(shareKeyframe/time.Second)
	frame := 0
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
		rgba, err := p.capture.NextFrame()
		if err != nil {
			return // capture stopped during teardown
		}
		encoded, err := p.encoder.encode(rgba, frame%keyframeEvery == 0)
		if err != nil {
			continue // one bad frame must not end a share
		}
		frame++
		if err := p.track.WriteEncoded(encoded, time.Second/shareFPS); err != nil {
			return
		}
	}
}

// rgbaToI420 converts one RGBA frame into the planar I420 layout libvpx
// expects, using the BT.601 limited-range matrix used across desktop video.
func rgbaToI420(rgba []byte, width, height int) []byte {
	ySize := width * height
	uvWidth := width / 2
	uvHeight := height / 2
	i420 := make([]byte, ySize+2*uvWidth*uvHeight)
	y := i420[:ySize]
	u := i420[ySize : ySize+uvWidth*uvHeight]
	v := i420[ySize+uvWidth*uvHeight:]
	for row := 0; row < height; row++ {
		for col := 0; col < width; col++ {
			src := (row*width + col) * 4
			r, g, b := rgba[src], rgba[src+1], rgba[src+2]
			y[row*width+col] = byte((299*int(r) + 587*int(g) + 114*int(b)) / 1000)
			if row%2 == 0 && col%2 == 0 {
				i := (row/2)*uvWidth + col/2
				u[i] = byte((-56*int(r)-123*int(g)+179*int(b))/1000 + 128)
				v[i] = byte((179*int(r)-151*int(g)-28*int(b))/1000 + 128)
			}
		}
	}
	return i420
}

// x11ScreenCapture and the platform entry point live behind build tags:
// screen_x11.go (real root-window capture on Linux), screen_windows.go and
// screen_other.go (honest refusals elsewhere). This file keeps the shared
// pipeline, which never changes per OS.
// openScreenCapture is the share pipeline's honest entry point: real capture
// where the platform boundary exists, and a refusal everywhere it would lie
// (Wayland, Windows, and anything else without a capture adapter).
func openScreenCapture() (screenCapture, error) {
	if os.Getenv("WAYLAND_DISPLAY") != "" || os.Getenv("XDG_SESSION_TYPE") == "wayland" {
		return nil, errors.New("wayland sessions need a portal adapter; x11 root capture would silently miss most of the screen")
	}
	return openPlatformScreenCapture()
}
