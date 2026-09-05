package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/pion/webrtc/v4"
)

func encodeForTest(t *testing.T, value envelope) []byte {
	t.Helper()
	body, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	frame := make([]byte, 4+len(body))
	binary.BigEndian.PutUint32(frame[:4], uint32(len(body)))
	copy(frame[4:], body)
	return frame
}

func decodeForTest(t *testing.T, input *bytes.Reader) envelope {
	t.Helper()
	value, err := readFrame(input)
	if err != nil {
		t.Fatal(err)
	}
	return value
}

func requestForTest(kind string, payload string) envelope {
	return envelope{Version: protocolVersion, Type: kind, RequestID: uuid.NewString(),
		Timestamp: "2026-08-31T22:00:00Z", Payload: json.RawMessage(payload)}
}

func TestPrivateProtocolHandshakeAndShutdown(t *testing.T) {
	input := bytes.NewBuffer(nil)
	helloRequest := requestForTest("media.hello", `{}`)
	shutdownRequest := requestForTest("media.shutdown", `{}`)
	input.Write(encodeForTest(t, helloRequest))
	input.Write(encodeForTest(t, shutdownRequest))
	output := bytes.NewBuffer(nil)
	service := newService(output)
	if err := service.serve(input); err != nil {
		t.Fatal(err)
	}

	reader := bytes.NewReader(output.Bytes())
	ready := decodeForTest(t, reader)
	if ready.Type != "media.hello" || ready.ReplyTo != helloRequest.RequestID || ready.Error != nil {
		t.Fatalf("unexpected ready reply: %#v", ready)
	}
	var readyPayload struct {
		Service string   `json:"service"`
		Caps    []string `json:"capabilities"`
	}
	if err := json.Unmarshal(ready.Payload, &readyPayload); err != nil {
		t.Fatal(err)
	}
	if readyPayload.Service != "harbor-media" || len(readyPayload.Caps) != 14 ||
		readyPayload.Caps[0] != "host-ice" || readyPayload.Caps[1] != "local-offer" ||
		readyPayload.Caps[2] != "remote-answer" || readyPayload.Caps[3] != "audio-opus" ||
		readyPayload.Caps[4] != "video-share" || readyPayload.Caps[5] != "chat-channel" ||
		readyPayload.Caps[6] != "file-transfer" || readyPayload.Caps[7] != "audio-devices" ||
		readyPayload.Caps[8] != "push-to-talk" || readyPayload.Caps[9] != "voice-activation" ||
		readyPayload.Caps[10] != "voice-level" || readyPayload.Caps[11] != "activity-channel" ||
		readyPayload.Caps[12] != "profile-channel" || readyPayload.Caps[13] != "audio-loopback" {
		t.Fatalf("unexpected capabilities: %#v", readyPayload)
	}
	stopped := decodeForTest(t, reader)
	if stopped.Type != "media.shutdown" || stopped.Error != nil {
		t.Fatalf("unexpected shutdown reply: %#v", stopped)
	}
}

func TestPrivateProtocolAdmitsDirectCommands(t *testing.T) {
	for _, command := range []string{
		"chat.send", "chat.poll", "chat.status", "transfer.begin", "transfer.accept",
		"transfer.reject", "transfer.send_chunk", "transfer.recv_chunk", "transfer.cancel",
		"transfer.finalize", "transfer.poll",
		"call.ptt", "audio.devices", "audio.config", "audio.switch_devices",
		"audio.loopback_start", "audio.loopback_poll", "audio.loopback_stop",
		"activity.send", "activity.poll",
		"profile.send", "profile.poll",
	} {
		if !isPrivateCommand(command) {
			t.Fatalf("private direct command %q was rejected before dispatch", command)
		}
	}
	input := bytes.NewBuffer(nil)
	request := requestForTest("chat.status", `{"message_ids":[]}`)
	input.Write(encodeForTest(t, request))
	input.Write(encodeForTest(t, requestForTest("media.shutdown", `{}`)))
	output := bytes.NewBuffer(nil)
	if err := (newService(output)).serve(input); err != nil {
		t.Fatalf("framed chat command should pass validation: %v", err)
	}
	reply := decodeForTest(t, bytes.NewReader(output.Bytes()))
	if reply.Type != "chat.status" || reply.Error != nil {
		t.Fatalf("unexpected direct reply: %#v", reply)
	}
}

func TestPrivateProtocolRejectsUnknownCommands(t *testing.T) {
	input := bytes.NewBuffer(encodeForTest(t, requestForTest("media.debug", `{}`)))
	service := newService(bytes.NewBuffer(nil))
	if err := service.serve(input); err == nil {
		t.Fatal("unknown private command must reject the input frame")
	}
}

func TestScreenShareIsHonestlyRefusedWithoutCaptureBoundary(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	// Without an active call there is nothing to attach a share to.
	response, _ := service.dispatch(requestForTest("call.share_start", `{"call_id":"call-1"}`))
	if response.Error == nil || response.Error.Code != "call_inactive" {
		t.Fatalf("share start without a call must be refused as inactive: %#v", response)
	}
	// With a call active but no capture boundary installed, the refusal is the
	// honest capture one — never a simulated SHARING.
	if started, _ := service.dispatch(requestForTest("call.start", `{"call_id":"call-1"}`)); started.Error != nil {
		t.Fatalf("call start failed: %#v", started.Error)
	}
	defer service.dispatch(requestForTest("call.end", `{}`))
	refused, _ := service.dispatch(requestForTest("call.share_start", `{"call_id":"call-1"}`))
	if refused.Error == nil || refused.Error.Code != "capture_unavailable" || refused.Error.UIKey != "error.call.screenShareUnavailable" {
		t.Fatalf("share start without a capture boundary must be refused honestly: %#v", refused)
	}
}

func TestMuteRequiresAnActiveCall(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	response, shutdown := service.dispatch(requestForTest("call.mute", `{"muted":true}`))
	if shutdown || response.Error == nil {
		t.Fatalf("mute must be refused without a call: %#v", response)
	}
	if response.Error.Code != "call_inactive" || response.Error.UIKey != "error.call.inactive" {
		t.Fatalf("unexpected refusal: %#v", response.Error)
	}
}

func TestRemoteSignalRequiresAnActiveCall(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	response, shutdown := service.dispatch(requestForTest("call.remote_signal",
		`{"call_id":"call-1","signal":{"type":"answer","sdp":"v=0\r\n"}}`))
	if shutdown || response.Error == nil {
		t.Fatalf("remote signals must be refused without a call: %#v", response)
	}
	if response.Error.Code != "call_inactive" || response.Error.UIKey != "error.call.inactive" {
		t.Fatalf("unexpected refusal: %#v", response.Error)
	}
}

func TestAcceptRefusesAnythingButAnOffer(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	response, shutdown := service.dispatch(requestForTest("call.accept",
		`{"call_id":"call-1","signal":{"type":"answer","sdp":"v=0\r\n"}}`))
	if shutdown || response.Error == nil {
		t.Fatalf("accept must refuse non-offer descriptions: %#v", response)
	}
	if response.Error.Code != "invalid_request" || response.Error.UIKey != "error.protocol.invalidRequest" {
		t.Fatalf("unexpected refusal: %#v", response.Error)
	}
}

func TestSecondCallIsRefusedWhileOneIsActive(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	started, shutdown := service.dispatch(requestForTest("call.start", `{"call_id":"call-1"}`))
	if shutdown || started.Error != nil {
		t.Fatalf("first call must start: %#v", started)
	}
	var startPayload struct {
		Signal struct {
			Type string `json:"type"`
			SDP  string `json:"sdp"`
		} `json:"signal"`
	}
	if err := json.Unmarshal(started.Payload, &startPayload); err != nil {
		t.Fatal(err)
	}

	again, _ := service.dispatch(requestForTest("call.start", `{"call_id":"call-2"}`))
	if again.Error == nil || again.Error.Code != "call_active" || again.Error.UIKey != "error.call.alreadyActive" {
		t.Fatalf("second start must be refused: %#v", again)
	}
	accept := fmt.Sprintf(`{"call_id":"call-3","signal":{"type":%q,"sdp":%q}}`,
		startPayload.Signal.Type, startPayload.Signal.SDP)
	accepted, _ := service.dispatch(requestForTest("call.accept", accept))
	if accepted.Error == nil || accepted.Error.Code != "call_active" {
		t.Fatalf("accept while active must be refused: %#v", accepted)
	}
	// Release the first call's pipeline so the test leaves no pumps running.
	if ended, _ := service.dispatch(requestForTest("call.end", `{}`)); ended.Error != nil {
		t.Fatalf("teardown failed: %#v", ended)
	}
}

// lockedBuffer lets Pion's callback goroutines append framed events while the
// test drains snapshots of the stream.
type lockedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

// patternAudio is a deterministic test boundary: every captured frame carries
// a square-wave pattern, and every decoded frame submitted for playback is
// recorded. It lets the test prove sound actually crosses the wire — encoded,
// packetized, decoded — without owning a sound card.
type patternAudio struct {
	mu     sync.Mutex
	played [][]byte
	phase  int16
	stop   chan struct{}
	once   sync.Once
}

func newPatternAudio() *patternAudio {
	return &patternAudio{stop: make(chan struct{}), phase: 8000}
}

func (p *patternAudio) Read() ([]byte, error) {
	select {
	case <-p.stop:
		return nil, errAudioStopped
	case <-time.After(frameDuration):
	}
	p.mu.Lock()
	p.phase = -p.phase
	sample := p.phase
	p.mu.Unlock()
	frame := make([]byte, frameBytes)
	for i := range frame {
		frame[i] = byte(uint16(sample) >> ((i % 2) * 8))
	}
	return frame, nil
}

func (p *patternAudio) Write(frame []byte) error {
	p.mu.Lock()
	defer p.mu.Unlock()
	select {
	case <-p.stop:
		return errAudioStopped
	default:
	}
	p.played = append(p.played, append([]byte(nil), frame...))
	return nil
}

func (p *patternAudio) Close() {
	p.once.Do(func() { close(p.stop) })
}

// playedFrames returns the decoded frames this boundary received, so the test
// can assert audio flowed and then stopped under mute.
func (p *patternAudio) playedFrames() [][]byte {
	p.mu.Lock()
	defer p.mu.Unlock()
	return append([][]byte(nil), p.played...)
}

// audioTestService wires a service to one pattern boundary; both sides of the
// call get their own so each direction of the call is proven independently.
func audioTestService() (*service, *patternAudio) {
	boundary := newPatternAudio()
	svc := newService(&lockedBuffer{})
	svc.audioFactory = func(deviceSelection) (audioIO, error) {
		return boundary, nil
	}
	return svc, boundary
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *lockedBuffer) snapshot() []byte {
	b.mu.Lock()
	defer b.mu.Unlock()
	return append([]byte(nil), b.buf.Bytes()...)
}

// hasAudibleEnergy reports whether any decoded frame carries real signal, not
// the silence a broken pipeline would deliver.
func hasAudibleEnergy(frames [][]byte) bool {
	for _, frame := range frames {
		for i := 0; i+1 < len(frame); i += 2 {
			sample := int16(binary.LittleEndian.Uint16(frame[i:]))
			if sample > 1000 || sample < -1000 {
				return true
			}
		}
	}
	return false
}

// waitUntil polls a condition that must become true within the window.
func waitUntil(t *testing.T, what string, within time.Duration, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(within)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// negotiateCall drives one direct call between two live workers exactly as the
// cores would relay it: offer from call.start, answer through call.accept,
// then trickled ICE candidates mirrored across via call.remote_signal until
// both sides report connected.
func negotiateCall(t *testing.T, caller, callee *service, callID string) {
	t.Helper()
	started, _ := caller.dispatch(requestForTest("call.start", fmt.Sprintf(`{"call_id":%q}`, callID)))
	if started.Error != nil {
		t.Fatalf("offer failed: %#v", started.Error)
	}
	var startPayload struct {
		Signal struct {
			Type string `json:"type"`
			SDP  string `json:"sdp"`
		} `json:"signal"`
	}
	if err := json.Unmarshal(started.Payload, &startPayload); err != nil {
		t.Fatal(err)
	}

	accept := fmt.Sprintf(`{"call_id":%q,"signal":{"type":%q,"sdp":%q}}`,
		callID, startPayload.Signal.Type, startPayload.Signal.SDP)
	accepted, _ := callee.dispatch(requestForTest("call.accept", accept))
	if accepted.Error != nil {
		t.Fatalf("answer failed: %#v", accepted.Error)
	}
	var acceptPayload struct {
		Signal struct {
			Type string `json:"type"`
			SDP  string `json:"sdp"`
		} `json:"signal"`
	}
	if err := json.Unmarshal(accepted.Payload, &acceptPayload); err != nil || acceptPayload.Signal.Type != "answer" {
		t.Fatalf("accept must return an answer: %#v", accepted)
	}
	applied := fmt.Sprintf(`{"call_id":%q,"signal":{"type":%q,"sdp":%q}}`,
		callID, acceptPayload.Signal.Type, acceptPayload.Signal.SDP)
	answered, _ := caller.dispatch(requestForTest("call.remote_signal", applied))
	if answered.Error != nil {
		t.Fatalf("answer rejected by the caller: %#v", answered.Error)
	}

	sides := []struct {
		name   string
		origin *service
		peer   *service
	}{
		{"caller", caller, callee},
		{"callee", callee, caller},
	}
	connected := map[string]bool{"caller": false, "callee": false}
	consumed := map[string]int{"caller": 0, "callee": 0}
	drain := func() {
		for _, side := range sides {
			frames := side.origin.output.(*lockedBuffer).snapshot()
			for consumed[side.name]+4 <= len(frames) {
				length := int(binary.BigEndian.Uint32(frames[consumed[side.name] : consumed[side.name]+4]))
				end := consumed[side.name] + 4 + length
				if end > len(frames) {
					break
				}
				body := frames[consumed[side.name]+4 : end]
				consumed[side.name] = end
				var event envelope
				if json.Unmarshal(body, &event) != nil {
					continue
				}
				switch event.Type {
				case "media.call_state":
					var statePayload struct {
						State string `json:"state"`
					}
					if json.Unmarshal(event.Payload, &statePayload) == nil && statePayload.State == "connected" {
						connected[side.name] = true
					}
				case "media.ice_candidate":
					var candidatePayload struct {
						Candidate json.RawMessage `json:"candidate"`
					}
					if json.Unmarshal(event.Payload, &candidatePayload) != nil || len(candidatePayload.Candidate) == 0 {
						continue
					}
					relay := fmt.Sprintf(`{"call_id":%q,"signal":{"type":"candidate","candidate":%s}}`,
						callID, candidatePayload.Candidate)
					forwarded, _ := side.peer.dispatch(requestForTest("call.remote_signal", relay))
					if forwarded.Error != nil {
						t.Fatalf("relayed candidate refused: %#v", forwarded.Error)
					}
				}
			}
		}
	}

	deadline := time.Now().Add(10 * time.Second)
	for !(connected["caller"] && connected["callee"]) {
		if time.Now().After(deadline) {
			t.Fatalf("direct call never connected: caller=%v callee=%v", connected["caller"], connected["callee"])
		}
		drain()
		time.Sleep(10 * time.Millisecond)
	}
}

// waitForAudible asserts a side of the call is decoding the peer's pattern.
func waitForAudible(t *testing.T, name string, boundary *patternAudio) {
	t.Helper()
	waitUntil(t, name+" to hear the peer", 5*time.Second, func() bool {
		return len(boundary.playedFrames()) >= 5 && hasAudibleEnergy(boundary.playedFrames())
	})
}

// Two live workers negotiate a direct call exactly as the cores would relay
// it. Once connected, the test proves voice actually crosses the direct path —
// both sides decode the peer's pattern — and that muting one side stops its
// audio at the source.
func TestOfferAnswerAndRelayedCandidatesCompleteADirectCall(t *testing.T) {
	caller, callerAudio := audioTestService()
	callee, calleeAudio := audioTestService()
	negotiateCall(t, caller, callee, "call-e2e")

	// A connected call carries real voice: each side must receive and decode
	// the peer's pattern through Opus and the direct RTP path.
	waitForAudible(t, "caller", callerAudio)
	waitForAudible(t, "callee", calleeAudio)

	// Mute is enforced at the source: after the callee mutes, the caller's
	// decoded stream stalls (a few in-flight frames are tolerated).
	muted, _ := callee.dispatch(requestForTest("call.mute", `{"muted":true}`))
	if muted.Error != nil {
		t.Fatalf("mute failed: %#v", muted.Error)
	}
	var mutedPayload struct {
		Muted bool `json:"muted"`
	}
	if err := json.Unmarshal(muted.Payload, &mutedPayload); err != nil || !mutedPayload.Muted {
		t.Fatalf("mute reply must confirm the state: %#v", muted)
	}
	baseline := len(callerAudio.playedFrames())
	time.Sleep(500 * time.Millisecond)
	if grew := len(callerAudio.playedFrames()) - baseline; grew > 5 {
		t.Fatalf("muted audio kept flowing: %d additional frames", grew)
	}

	for _, side := range []struct {
		name   string
		origin *service
	}{
		{"caller", caller},
		{"callee", callee},
	} {
		ended, _ := side.origin.dispatch(requestForTest("call.end", `{}`))
		if ended.Error != nil {
			t.Fatalf("teardown failed on %s: %#v", side.name, ended.Error)
		}
	}
	stale, _ := caller.dispatch(requestForTest("call.remote_signal",
		`{"call_id":"call-e2e","signal":{"type":"candidate","candidate":{"candidate":"candidate:1 1 UDP 1 127.0.0.1 40000 typ host"}}}`))
	if stale.Error == nil || stale.Error.Code != "call_inactive" {
		t.Fatalf("signals after teardown must be refused: %#v", stale)
	}
}

// fakeScreen is the deterministic capture boundary: flat 64x48 frames, counted
// so a test can prove the share pump really pulls them — and really stops.
type fakeScreen struct {
	mu      sync.Mutex
	grabbed int
	stop    chan struct{}
	once    sync.Once
}

func newFakeScreen() *fakeScreen { return &fakeScreen{stop: make(chan struct{})} }

func (s *fakeScreen) Size() (int, int) { return 64, 48 }

func (s *fakeScreen) NextFrame() ([]byte, error) {
	select {
	case <-s.stop:
		return nil, errors.New("capture stopped")
	default:
	}
	s.mu.Lock()
	s.grabbed++
	s.mu.Unlock()
	return make([]byte, 64*48*4), nil
}

func (s *fakeScreen) Close() { s.once.Do(func() { close(s.stop) }) }

func (s *fakeScreen) grabbedFrames() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.grabbed
}

// recordedTrack is the pipeline test's track boundary: it records every
// encoded frame the pump writes.
type recordedTrack struct {
	mu     sync.Mutex
	frames [][]byte
}

func (r *recordedTrack) WriteEncoded(frame []byte, _ time.Duration) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.frames = append(r.frames, append([]byte(nil), frame...))
	return nil
}

func (r *recordedTrack) written() [][]byte {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([][]byte(nil), r.frames...)
}

func TestShareStartRequiresAnActiveCall(t *testing.T) {
	service := newService(bytes.NewBuffer(nil))
	service.screenFactory = func() (screenCapture, error) { return newFakeScreen(), nil }
	response, shutdown := service.dispatch(requestForTest("call.share_start", `{"call_id":"call-1"}`))
	if shutdown || response.Error == nil {
		t.Fatalf("share start must be refused without a call: %#v", response)
	}
	if response.Error.Code != "call_inactive" {
		t.Fatalf("unexpected refusal: %#v", response.Error)
	}
	// Stopping a share that never started is honestly NOT_SHARING, not an error.
	stopped, _ := service.dispatch(requestForTest("call.share_stop", `{"call_id":"call-1"}`))
	if stopped.Error != nil {
		t.Fatalf("share stop must not fail: %#v", stopped.Error)
	}
	var stopPayload struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(stopped.Payload, &stopPayload); err != nil || stopPayload.State != "NOT_SHARING" {
		t.Fatalf("share stop must report NOT_SHARING: %#v", stopped)
	}
}

// The share pump must turn captured RGBA frames into VP8 frames on its track
// through the real encoder, and teardown must join it deterministically.
func TestSharePipelineEncodesCapturedFramesAndStopsCleanly(t *testing.T) {
	screen := newFakeScreen()
	encoder, err := newVPXEncoder(64, 48)
	if err != nil {
		if errors.Is(err, errVPXUnavailable) {
			t.Skip("video encoder is unavailable on this platform")
		}
		t.Fatal(err)
	}
	track := &recordedTrack{}
	share := newSharePipeline(screen, encoder, track)
	waitUntil(t, "the pump to encode captured frames", 5*time.Second, func() bool {
		return len(track.written()) >= 3
	})
	share.cancel()
	share.capture.Close()
	share.done.Wait()
	before := len(track.written())
	time.Sleep(100 * time.Millisecond)
	if len(track.written()) != before {
		t.Fatal("share pump kept writing after cancel")
	}
	encoder.Close()
}

// One RGBA pixel layout must come out as the planar I420 libvpx expects, with
// luma per pixel and chroma subsampled from even rows and columns.
func TestRGBAConversionProducesI420Planes(t *testing.T) {
	rgba := []byte{
		255, 0, 0, 255, 255, 0, 0, 255, // row 0: red, red
		128, 128, 128, 255, 128, 128, 128, 255, // row 1: gray, gray
	}
	i420 := rgbaToI420(rgba, 2, 2)
	if len(i420) != 6 { // 1.5 bytes per pixel
		t.Fatalf("I420 frame must be 1.5 bytes per pixel: %d", len(i420))
	}
	y, u, v := i420[:4], i420[4:5], i420[5:6]
	if y[0] != 76 || y[1] != 76 { // red luma: (299*255)/1000
		t.Fatalf("red luma wrong: %v", y)
	}
	if y[2] != 128 || y[3] != 128 { // gray luma: exact
		t.Fatalf("gray luma wrong: %v", y)
	}
	// Chroma subsamples the red pixel: U=(-56*255)/1000+128, V=(179*255)/1000+128.
	if u[0] != 114 || v[0] != 173 {
		t.Fatalf("red chroma wrong: u=%d v=%d", u[0], v[0])
	}
}

// A Wayland session must refuse capture rather than silently share the
// XWayland root window, which misses most of the real screen.
func TestWaylandSessionRefusesScreenCaptureHonestly(t *testing.T) {
	t.Setenv("WAYLAND_DISPLAY", "wayland-1")
	t.Setenv("XDG_SESSION_TYPE", "wayland")
	if _, err := openScreenCapture(); err == nil {
		t.Fatal("a Wayland session must refuse x11 root capture")
	}
}

// The share rides the call's negotiated video track: once a direct call is
// connected, starting a share puts real VP8 frames on the wire — the peer
// receives them without any renegotiation — and stopping it ends the flow
// while the call and its voice survive untouched.
func TestScreenShareFlowsOverAnActiveDirectCall(t *testing.T) {
	caller, _ := audioTestService()
	screen := newFakeScreen()
	caller.screenFactory = func() (screenCapture, error) { return screen, nil }
	callee, calleeAudio := audioTestService()
	negotiateCall(t, caller, callee, "call-share")
	waitForAudible(t, "callee", calleeAudio)

	started, _ := caller.dispatch(requestForTest("call.share_start", `{"call_id":"call-share"}`))
	if started.Error != nil {
		// No capture adapter or encoder off Linux: the refusal is the
		// feature there, not a regression.
		if runtime.GOOS != "linux" && started.Error.Code == "capture_unavailable" {
			t.Skip("screen sharing is unavailable on this platform")
		}
		t.Fatalf("share start refused: %#v", started.Error)
	}
	var sharePayload struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(started.Payload, &sharePayload); err != nil || sharePayload.State != "SHARING" {
		t.Fatalf("share start must report SHARING: %#v", started)
	}
	waitUntil(t, "the caller to pull captured frames", 5*time.Second, func() bool {
		return screen.grabbedFrames() >= 10
	})
	waitUntil(t, "the peer to receive VP8 packets", 5*time.Second, func() bool {
		return callee.inboundVideo.Load() >= 5
	})

	stopped, _ := caller.dispatch(requestForTest("call.share_stop", `{"call_id":"call-share"}`))
	if stopped.Error != nil {
		t.Fatalf("share stop failed: %#v", stopped.Error)
	}
	var stopPayload struct {
		State string `json:"state"`
	}
	if err := json.Unmarshal(stopped.Payload, &stopPayload); err != nil || stopPayload.State != "NOT_SHARING" {
		t.Fatalf("share stop must report NOT_SHARING: %#v", stopped)
	}
	// The flow really ends: capture pulls and inbound packets both stall.
	before := screen.grabbedFrames()
	inboundBefore := callee.inboundVideo.Load()
	time.Sleep(500 * time.Millisecond)
	if grew := screen.grabbedFrames() - before; grew > 1 {
		t.Fatalf("share pump kept running after stop: %d additional grabs", grew)
	}
	if grew := callee.inboundVideo.Load() - inboundBefore; grew > 5 {
		t.Fatalf("VP8 kept flowing after stop: %d additional packets", grew)
	}
	// Stopping the share never ends the call: voice is still live.
	if !hasAudibleEnergy(calleeAudio.playedFrames()) {
		t.Fatal("voice died when the share stopped")
	}

	for _, side := range []*service{caller, callee} {
		if ended, _ := side.dispatch(requestForTest("call.end", `{}`)); ended.Error != nil {
			t.Fatalf("teardown failed: %#v", ended.Error)
		}
	}
}

func TestPionLoopbackNegotiatesAnOpusTrackAndTearsDown(t *testing.T) {
	first, err := webrtc.NewPeerConnection(webrtc.Configuration{})
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := webrtc.NewPeerConnection(webrtc.Configuration{})
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()

	connected := make(chan struct{}, 1)
	second.OnConnectionStateChange(func(state webrtc.PeerConnectionState) {
		if state == webrtc.PeerConnectionStateConnected {
			select {
			case connected <- struct{}{}:
			default:
			}
		}
	})
	track, err := webrtc.NewTrackLocalStaticSample(
		webrtc.RTPCodecCapability{MimeType: webrtc.MimeTypeOpus}, "audio", "loopback")
	if err != nil {
		t.Fatal(err)
	}
	if _, err = first.AddTrack(track); err != nil {
		t.Fatal(err)
	}

	gatherFirst := webrtc.GatheringCompletePromise(first)
	offer, err := first.CreateOffer(nil)
	if err != nil || first.SetLocalDescription(offer) != nil {
		t.Fatalf("offer failed: %v", err)
	}
	<-gatherFirst
	if err = second.SetRemoteDescription(*first.LocalDescription()); err != nil {
		t.Fatal(err)
	}
	gatherSecond := webrtc.GatheringCompletePromise(second)
	answer, err := second.CreateAnswer(nil)
	if err != nil || second.SetLocalDescription(answer) != nil {
		t.Fatalf("answer failed: %v", err)
	}
	<-gatherSecond
	if err = first.SetRemoteDescription(*second.LocalDescription()); err != nil {
		t.Fatal(err)
	}

	select {
	case <-connected:
	case <-time.After(5 * time.Second):
		t.Fatal("Pion loopback never connected")
	}
	if err = first.Close(); err != nil {
		t.Fatal(err)
	}
}

// requestForTestPayload builds a request envelope from a pre-encoded payload.
func requestForTestPayload(kind string, payload []byte) envelope {
	return envelope{
		Version:   protocolVersion,
		Type:      kind,
		RequestID: fmt.Sprintf("req-%s", kind),
		Timestamp: "2026-09-01T00:00:00Z",
		Payload:   payload,
	}
}

// dispatchJSON marshals and dispatches in one step.
func dispatchJSON(svc *service, kind string, payload any) envelope {
	body, err := json.Marshal(payload)
	if err != nil {
		panic(err)
	}
	reply, _ := svc.dispatch(requestForTestPayload(kind, body))
	return reply
}

// waitForOpenChannels waits until the direct channels are usable on both
// sides of a just-negotiated call: DataChannels open asynchronously after the
// peer connection reports connected.
func waitForOpenChannels(t *testing.T, caller, callee *service) {
	t.Helper()
	waitUntil(t, "direct channels to open on both sides", 15*time.Second, func() bool {
		return caller.currentChannels() != nil && callee.currentChannels() != nil
	})
}

// chatSendRetry sends one chat body, retrying while the channels are still
// opening. DataChannel readiness is timing, not policy, so a retry here never
// hides a real refusal (every other error stops the retry).
func chatSendRetry(t *testing.T, svc *service, id, body string) envelope {
	t.Helper()
	var reply envelope
	waitUntil(t, "chat channel to accept a send", 15*time.Second, func() bool {
		reply = dispatchJSON(svc, "chat.send", map[string]any{
			"call_id": "call-dc", "message_id": id, "body": body,
		})
		return reply.Error == nil ||
			reply.Error.Code == "chat_too_large" ||
			reply.Error.Code == "invalid_request"
	})
	return reply
}

func TestChatFlowsDirectlyBetweenTwoWorkers(t *testing.T) {
	caller, _ := audioTestService()
	callee, _ := audioTestService()
	negotiateCall(t, caller, callee, "call-dc")
	defer func() {
		caller.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
		callee.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
	}()
	waitForOpenChannels(t, caller, callee)

	// Outbound ack: the worker confirms the frame, the core's DELIVERED comes
	// only when the peer's worker acknowledges receipt.
	reply := chatSendRetry(t, caller, "m-1", "olá do outro lado")
	if reply.Error != nil {
		t.Fatalf("chat send refused: %#v", reply.Error)
	}
	if got := string(reply.Payload); !strings.Contains(got, "SENT") {
		t.Fatalf("expected SENT, got %s", got)
	}

	// Inbound: the callee's core polls the message out of the worker.
	var polled struct {
		Messages []struct {
			MessageID string `json:"message_id"`
			Body      string `json:"body"`
		} `json:"messages"`
		Overflowed bool `json:"overflowed"`
	}
	waitUntil(t, "chat message to cross the direct path", 15*time.Second, func() bool {
		poll := dispatchJSON(callee, "chat.poll", map[string]any{})
		if poll.Error != nil {
			return false
		}
		if json.Unmarshal(poll.Payload, &polled) != nil {
			return false
		}
		return len(polled.Messages) > 0
	})
	if polled.Overflowed {
		t.Fatal("an ordinary message must never overflow the inbox")
	}
	if polled.Messages[0].MessageID != "m-1" || polled.Messages[0].Body != "olá do outro lado" {
		t.Fatalf("message arrived mangled: %+v", polled.Messages[0])
	}

	// The receiving worker acknowledged the frame on its control channel, so
	// the sender's delivery fact flips without any server involvement.
	waitUntil(t, "peer ack to reach the sender", 15*time.Second, func() bool {
		status := dispatchJSON(caller, "chat.status", map[string]any{"message_ids": []string{"m-1"}})
		var payload struct {
			Deliveries map[string]bool `json:"deliveries"`
		}
		if json.Unmarshal(status.Payload, &payload) != nil {
			return false
		}
		return payload.Deliveries["m-1"] == true
	})
}

func TestChatRefusesOversizedBodiesOnTheWire(t *testing.T) {
	caller, _ := audioTestService()
	callee, _ := audioTestService()
	negotiateCall(t, caller, callee, "call-dc")
	defer func() {
		caller.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
		callee.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
	}()
	waitForOpenChannels(t, caller, callee)

	oversized := strings.Repeat("x", chatMaxBodyBytes+1)
	reply := chatSendRetry(t, caller, "m-big", oversized)
	if reply.Error == nil || reply.Error.Code != "chat_too_large" {
		t.Fatalf("an oversized body must be refused on the wire, got %#v", reply.Error)
	}
}

func TestFileTransferCompletesAcrossTheDirectPath(t *testing.T) {
	caller, _ := audioTestService()
	callee, _ := audioTestService()
	negotiateCall(t, caller, callee, "call-dc")
	defer func() {
		caller.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
		callee.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
	}()
	waitForOpenChannels(t, caller, callee)

	// A deterministic payload larger than one chunk, with its real SHA-256.
	payload := make([]byte, 200*1024)
	for i := range payload {
		payload[i] = byte(i * 31)
	}
	sum := sha256.Sum256(payload)
	sumHex := hex.EncodeToString(sum[:])
	const chunkSize = 16 * 1024
	const transferID = "t-1"

	reply := dispatchJSON(caller, "transfer.begin", map[string]any{
		"call_id": "call-dc", "transfer_id": transferID,
		"name": "screenshot-2026.png", "size": len(payload),
		"sha256": sumHex, "chunk_size": chunkSize,
	})
	if reply.Error != nil {
		t.Fatalf("transfer.begin refused: %#v", reply.Error)
	}

	// The callee's poll surfaces the offer; the event is only a hint.
	waitUntil(t, "the offer to reach the callee's poll", 15*time.Second, func() bool {
		poll := dispatchJSON(callee, "transfer.poll", map[string]any{})
		return poll.Error == nil && strings.Contains(string(poll.Payload), `"phase":"offered"`)
	})

	accepted := dispatchJSON(callee, "transfer.accept", map[string]any{"transfer_id": transferID})
	if accepted.Error != nil {
		t.Fatalf("transfer.accept refused: %#v", accepted.Error)
	}

	// The offerer learns through its poll that the transfer is active.
	waitUntil(t, "the caller to observe the active transfer", 15*time.Second, func() bool {
		poll := dispatchJSON(caller, "transfer.poll", map[string]any{})
		return poll.Error == nil && strings.Contains(string(poll.Payload), `"phase":"active"`)
	})

	// Stream every chunk; the callee drains concurrently. The final chunk is
	// marked, and both sides converge without any relay involvement.
	done := make(chan error, 1)
	go func() {
		const drainDeadline = 60 * time.Second
		started := time.Now()
		hasher := sha256.New()
		var reassembled []byte
		for time.Since(started) < drainDeadline {
			recv := dispatchJSON(callee, "transfer.recv_chunk", map[string]any{"transfer_id": transferID})
			if recv.Error != nil {
				time.Sleep(5 * time.Millisecond)
				continue
			}
			var chunk struct {
				Empty  bool   `json:"empty"`
				Seq    int    `json:"seq"`
				Offset int64  `json:"offset"`
				Data   string `json:"data"`
				Final  bool   `json:"final"`
			}
			if json.Unmarshal(recv.Payload, &chunk) != nil {
				done <- errors.New("chunk shape was invalid")
				return
			}
			if chunk.Empty {
				time.Sleep(5 * time.Millisecond)
				continue
			}
			raw, err := base64.StdEncoding.DecodeString(chunk.Data)
			if err != nil {
				done <- err
				return
			}
			hasher.Write(raw)
			reassembled = append(reassembled, raw...)
			if chunk.Final {
				if len(reassembled) != len(payload) {
					done <- fmt.Errorf("size mismatch: %d != %d", len(reassembled), len(payload))
					return
				}
				if !bytes.Equal(reassembled, payload) {
					done <- errors.New("reassembled bytes differ")
					return
				}
				if hex.EncodeToString(hasher.Sum(nil)) != sumHex {
					done <- errors.New("sha256 mismatch")
					return
				}
				done <- nil
				return
			}
		}
		done <- errors.New("the final chunk never arrived")
	}()

	for seq, offset := 0, 0; offset < len(payload); seq, offset = seq+1, offset+chunkSize {
		end := offset + chunkSize
		if end > len(payload) {
			end = len(payload)
		}
		final := end == len(payload)
		var sent envelope
		waitUntil(t, fmt.Sprintf("chunk %d to be accepted", seq), 15*time.Second, func() bool {
			sent = dispatchJSON(caller, "transfer.send_chunk", map[string]any{
				"transfer_id": transferID, "seq": seq, "offset": offset,
				"data":  base64.StdEncoding.EncodeToString(payload[offset:end]),
				"final": final,
			})
			return sent.Error == nil
		})
	}
	if err := <-done; err != nil {
		t.Fatalf("inbound verification failed: %v", err)
	}

	// Verified receipt: the receiver finalizes, the sender's poll flips to
	// completed with peer_received — the direct-path delivery fact.
	finalized := dispatchJSON(callee, "transfer.finalize", map[string]any{"transfer_id": transferID, "ok": true})
	if finalized.Error != nil {
		t.Fatalf("transfer.finalize refused: %#v", finalized.Error)
	}
	waitUntil(t, "the sender to observe completion", 15*time.Second, func() bool {
		poll := dispatchJSON(caller, "transfer.poll", map[string]any{})
		return poll.Error == nil &&
			strings.Contains(string(poll.Payload), `"phase":"completed"`) &&
			strings.Contains(string(poll.Payload), `"peer_received":true`)
	})
}

func TestFileTransferRefusesOversizedOffersAndCancelsBilaterally(t *testing.T) {
	caller, _ := audioTestService()
	callee, _ := audioTestService()
	negotiateCall(t, caller, callee, "call-dc")
	defer func() {
		caller.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
		callee.dispatch(requestForTest("call.end", `{"call_id":"call-dc"}`))
	}()
	waitForOpenChannels(t, caller, callee)

	// An offer above the size limit never enters the wire at all.
	refused := dispatchJSON(caller, "transfer.begin", map[string]any{
		"call_id": "call-dc", "transfer_id": "t-big",
		"name": "huge.iso", "size": fileMaxBytes + 1, "sha256": strings.Repeat("a", 64),
	})
	if refused.Error == nil || refused.Error.Code != "transfer_too_large" {
		t.Fatalf("an oversized offer must be refused locally, got %#v", refused.Error)
	}

	// A live transfer canceled on one side disappears from both polls.
	started := dispatchJSON(caller, "transfer.begin", map[string]any{
		"call_id": "call-dc", "transfer_id": "t-x",
		"name": "notes.txt", "size": 1024, "sha256": strings.Repeat("b", 64),
	})
	if started.Error != nil {
		t.Fatalf("transfer.begin refused: %#v", started.Error)
	}
	waitUntil(t, "the offer to reach the callee", 15*time.Second, func() bool {
		poll := dispatchJSON(callee, "transfer.poll", map[string]any{})
		return poll.Error == nil && strings.Contains(string(poll.Payload), `"transfer_id":"t-x"`)
	})
	cancelled := dispatchJSON(caller, "transfer.cancel", map[string]any{"transfer_id": "t-x"})
	if cancelled.Error != nil {
		t.Fatalf("transfer.cancel refused: %#v", cancelled.Error)
	}
	waitUntil(t, "the cancel to reach the callee", 15*time.Second, func() bool {
		poll := dispatchJSON(callee, "transfer.poll", map[string]any{})
		return poll.Error == nil && strings.Contains(string(poll.Payload), `"canceled":true`)
	})
}
