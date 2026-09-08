// Unit tests for the voice-level math, the per-stream gain, the swap-tolerant
// device boundary, the transmit gate, and the audio/activity protocol surface.
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"sync"
	"testing"
	"time"

	"github.com/pion/webrtc/v4"
)

// TestFrameLevelTracksAmplitude proves the level metric reports real signal
// energy: silence reads zero, a full-scale square reads near one.
func TestFrameLevelTracksAmplitude(t *testing.T) {
	silent := make([]int16, frameSamples)
	if level := frameLevel(silent); level != 0 {
		t.Fatalf("silence must read level 0, got %v", level)
	}
	loud := make([]int16, frameSamples)
	for i := range loud {
		loud[i] = 32767
	}
	level := frameLevel(loud)
	if level < 0.99 || level > 1.0 {
		t.Fatalf("full-scale square must read ~1, got %v", level)
	}
	quiet := make([]int16, frameSamples)
	for i := range quiet {
		quiet[i] = 327 // ~1% amplitude
	}
	if got := frameLevel(quiet); got < 0.009 || got > 0.011 {
		t.Fatalf("1%% amplitude must read ~0.01, got %v", got)
	}
}

// TestSmoothLevelAttacksFastAndDecaysSlowly proves the indicator snaps up on
// speech onsets and eases down after them, which is what keeps the UI's
// speaking dot from flickering.
func TestSmoothLevelAttacksFastAndDecaysSlowly(t *testing.T) {
	if got := smoothLevel(0.5, 0.2); got != 0.5*levelDecay {
		t.Fatalf("a quieter raw level must decay the previous one, got %v", got)
	}
	if got := smoothLevel(0.2, 0.8); got != 0.8 {
		t.Fatalf("a louder raw level must pass through immediately, got %v", got)
	}
}

// TestSpeechDetectorHangover proves speaking stays latched through short
// gaps between words and only clears after the hangover window (the smoothed
// level may take a couple of extra frames to decay under the threshold).
func TestSpeechDetectorHangover(t *testing.T) {
	var d speechDetector
	d.observe(speakingThreshold * 2)
	if !d.speaking {
		t.Fatal("loud speech must latch speaking")
	}
	for i := 0; i < speakingHangoverFrames-4; i++ {
		d.observe(0)
	}
	if !d.speaking {
		t.Fatal("speaking must survive a short pause inside the hangover window")
	}
	for i := 0; i < speakingHangoverFrames+8; i++ {
		d.observe(0)
		if !d.speaking {
			return
		}
	}
	t.Fatal("speaking must clear once the hangover window has passed")
}

// TestApplyGainScalesWithoutClipping proves the per-stream volume scales PCM
// in place and clamps instead of wrapping through int16.
func TestApplyGainScalesWithoutClipping(t *testing.T) {
	pcm := []int16{1000, -1000, 20000, -20000}
	applyGain(pcm, 0.5)
	if pcm[0] != 500 || pcm[1] != -500 || pcm[2] != 10000 || pcm[3] != -10000 {
		t.Fatalf("half gain must halve every sample: %#v", pcm)
	}
	peaked := []int16{20000, -20000}
	applyGain(peaked, 4)
	if peaked[0] != 32767 || peaked[1] != -32768 {
		t.Fatalf("overdriven gain must clamp, not wrap: %#v", peaked)
	}
}

// TestClampGainBoundsTheVolume proves a bogus volume from anywhere in the
// stack can never turn into an amplifier above unity.
func TestClampGainBoundsTheVolume(t *testing.T) {
	for _, probe := range []struct{ in, want float64 }{
		{-1, 0}, {0, 0}, {0.25, 0.25}, {1, 1}, {7.5, 1},
	} {
		if got := clampGain(probe.in); got != probe.want {
			t.Fatalf("clampGain(%v) = %v, want %v", probe.in, got, probe.want)
		}
	}
}

// countingAudio records Read/Write traffic and can be closed to simulate the
// device a swap replaces.
type countingAudio struct {
	mu      sync.Mutex
	reads   int
	writes  int
	closed  bool
	stopped chan struct{}
	once    sync.Once
	readGas chan struct{}
}

func newCountingAudio() *countingAudio {
	return &countingAudio{stopped: make(chan struct{}), readGas: make(chan struct{}, 1)}
}

func (c *countingAudio) Read() ([]byte, error) {
	select {
	case c.readGas <- struct{}{}:
		select {
		case <-c.stopped:
			return nil, errAudioStopped
		default:
		}
		c.mu.Lock()
		c.reads++
		c.mu.Unlock()
		return make([]byte, frameBytes), nil
	case <-c.stopped:
		return nil, errAudioStopped
	}
}

func (c *countingAudio) Write([]byte) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	select {
	case <-c.stopped:
		return errAudioStopped
	default:
	}
	c.writes++
	return nil
}

func (c *countingAudio) Close() {
	c.once.Do(func() {
		c.mu.Lock()
		c.closed = true
		c.mu.Unlock()
		close(c.stopped)
	})
}

func (c *countingAudio) counts() (reads, writes int, closed bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.reads, c.writes, c.closed
}

// TestSwitchableAudioServesAcrossASwap proves reads and writes keep flowing
// when the device underneath is replaced mid-stream.
func TestSwitchableAudioServesAcrossASwap(t *testing.T) {
	first, second := newCountingAudio(), newCountingAudio()
	w := newSwitchableAudio(first)

	if _, err := w.Read(); err != nil {
		t.Fatalf("read before swap: %v", err)
	}
	if err := w.Write(make([]byte, frameBytes)); err != nil {
		t.Fatalf("write before swap: %v", err)
	}
	if err := w.Swap(func() (audioIO, error) { return second, nil }); err != nil {
		t.Fatalf("swap: %v", err)
	}
	if _, _, closed := first.counts(); !closed {
		t.Fatal("the replaced device must be closed by the swap")
	}
	if _, err := w.Read(); err != nil {
		t.Fatalf("read after swap: %v", err)
	}
	if err := w.Write(make([]byte, frameBytes)); err != nil {
		t.Fatalf("write after swap: %v", err)
	}
	reads, writes, _ := second.counts()
	if reads != 1 || writes != 1 {
		t.Fatalf("traffic must reach the replacement device: reads=%d writes=%d", reads, writes)
	}
	w.Close()
	if _, _, closed := second.counts(); !closed {
		t.Fatal("close must reach the live device")
	}
}

// TestSwapFailureKeepsTheOldDevices proves a device that refuses to open
// leaves the current pair running — the call never loses audio because a
// switch was rejected.
func TestSwapFailureKeepsTheOldDevices(t *testing.T) {
	current := newCountingAudio()
	w := newSwitchableAudio(current)
	if err := w.Swap(func() (audioIO, error) { return nil, errAudioStopped }); err == nil {
		t.Fatal("a failed open must surface as a swap failure")
	}
	if _, _, closed := current.counts(); closed {
		t.Fatal("a failed swap must keep the current devices open")
	}
	if _, err := w.Read(); err != nil {
		t.Fatalf("the old device must keep serving: %v", err)
	}
	w.Close()
}

// blockedAudio blocks in Read until released, so the test can prove a swap
// unblocks a reader stuck on the old device.
type blockedAudio struct {
	release chan struct{}
	stopped chan struct{}
	once    sync.Once
}

func (b *blockedAudio) Read() ([]byte, error) {
	select {
	case <-b.release:
	case <-b.stopped:
		return nil, errAudioStopped
	}
	return make([]byte, frameBytes), nil
}

func (b *blockedAudio) Write([]byte) error { return nil }

func (b *blockedAudio) Close() {
	b.once.Do(func() { close(b.stopped) })
}

// TestSwitchableAudioReadFindsTheReplacement proves a read that was racing a
// swap retries against the new device instead of erroring the call.
func TestSwitchableAudioReadFindsTheReplacement(t *testing.T) {
	old := &blockedAudio{release: make(chan struct{}), stopped: make(chan struct{})}
	w := newSwitchableAudio(old)
	defer w.Close()

	next := newCountingAudio()
	go func() {
		time.Sleep(20 * time.Millisecond)
		_ = w.Swap(func() (audioIO, error) { return next, nil })
	}()
	if _, err := w.Read(); err != nil {
		t.Fatalf("a read racing a swap must follow the replacement: %v", err)
	}
	close(old.release) // leave the abandoned reader nothing to block on
}

// transmitGateService builds a minimal service for transmit-gate probes.
func transmitGateService(muted, pttEnabled, pttActive, voiceActivation bool) *service {
	return &service{muted: muted, pttEnabled: pttEnabled, pttActive: pttActive,
		voiceActivation: voiceActivation}
}

// TestTransmitGateMuteWinsOverPTT is the safety contract: a hand-muted
// microphone stays silent no matter what push-to-talk or voice activation do.
func TestTransmitGateMuteWinsOverPTT(t *testing.T) {
	cases := []struct {
		muted, pttEnabled, pttActive, voiceActivation, want bool
	}{
		{false, false, false, false, true}, // plain call
		{false, false, false, true, false}, // voice activation, silence
		{false, false, false, true, true},  // voice activation, speech
		{false, true, true, false, true},   // PTT held
		{false, true, true, true, true},    // held key outranks the mode
		{false, true, false, false, false}, // PTT up
		{false, true, false, true, false},  // PTT up outranks the mode
		{true, false, false, false, false}, // manual mute
		{true, true, true, false, false},   // mute wins over a held key
		{true, true, false, true, false},   // mute wins over every mode
	}
	for _, c := range cases {
		svc := transmitGateService(c.muted, c.pttEnabled, c.pttActive, c.voiceActivation)
		speaking := c.voiceActivation && c.want && !c.muted && !c.pttEnabled
		if got := svc.transmit(speaking); got != c.want {
			t.Fatalf("transmit(muted=%v ptt=%v active=%v va=%v speaking=%v) = %v, want %v",
				c.muted, c.pttEnabled, c.pttActive, c.voiceActivation, speaking, got, c.want)
		}
		if got := svc.captureOpen(); got != (!c.muted && !(c.pttEnabled && !c.pttActive)) {
			t.Fatalf("captureOpen(muted=%v ptt=%v active=%v) = %v", c.muted, c.pttEnabled, c.pttActive, got)
		}
	}
}

// TestAudioConfigRoundTripAndClamp proves audio.config stores the selection,
// clamps volumes into 0..1, and reports the effective configuration.
func TestAudioConfigRoundTripAndClamp(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))

	got := svc.audioConfig(requestForTest("audio.config", `{}`))
	if got.Error != nil {
		t.Fatalf("config read: %#v", got.Error)
	}
	var state struct {
		InputDevice  string  `json:"input_device"`
		OutputDevice string  `json:"output_device"`
		InputVolume  float64 `json:"input_volume"`
		OutputVolume float64 `json:"output_volume"`
	}
	if err := json.Unmarshal(got.Payload, &state); err != nil {
		t.Fatal(err)
	}
	if state.InputVolume != 1 || state.OutputVolume != 1 {
		t.Fatalf("defaults must be unity: %#v", state)
	}

	set := svc.audioConfig(requestForTest("audio.config",
		`{"set":true,"input_device":"aaa1","input_volume":4,"output_volume":-2}`))
	if set.Error != nil {
		t.Fatalf("config set: %#v", set.Error)
	}
	if err := json.Unmarshal(set.Payload, &state); err != nil {
		t.Fatal(err)
	}
	if state.InputDevice != "aaa1" {
		t.Fatalf("selection must persist: %#v", state)
	}
	if state.InputVolume != 1 || state.OutputVolume != 0 {
		t.Fatalf("volumes must clamp into 0..1: %#v", state)
	}
}

// TestActivityChannelRoundTrip proves a sanitized activity frame sent by one
// worker surfaces from the other's activity.poll inbox, in order.
func TestActivityChannelRoundTrip(t *testing.T) {
	sender := newChannelSet()
	receiver := newChannelSet()
	defer sender.close()
	defer receiver.close()

	// Bind the pair through throwaway DataChannels is not possible without a
	// real PeerConnection; drive the same code paths directly instead.
	if err := sender.sendActivity(`{"kind":"app_focus"}`); err == nil {
		t.Fatal("activity on a worker with no control channel must be refused")
	}

	// Deliver one frame through the receive path the DataChannel feeds.
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "activity",
		Data: `{"kind":"app_focus"}`,
	}))
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "activity",
		Data: `{"kind":"game_state"}`,
	}))
	events := receiver.activityInbox()
	if len(events) != 2 || events[0] != `{"kind":"app_focus"}` || events[1] != `{"kind":"game_state"}` {
		t.Fatalf("inbox must drain in order: %#v", events)
	}
	if again := receiver.activityInbox(); len(again) != 0 {
		t.Fatalf("a drained inbox must stay empty: %#v", again)
	}
}

// TestActivityChannelBoundsAndRefuses proves oversized frames never enter
// the worker and a full inbox refuses rather than growing without bound.
func TestActivityChannelBoundsAndRefuses(t *testing.T) {
	receiver := newChannelSet()
	defer receiver.close()

	oversize := make([]byte, activityFrameMaxBytes+1)
	for i := range oversize {
		oversize[i] = 'a'
	}
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "activity",
		Data: string(oversize),
	}))
	if got := receiver.activityInbox(); len(got) != 0 {
		t.Fatalf("an oversized frame must be refused: %d events", len(got))
	}

	for i := 0; i < activityInboxCapacity+4; i++ {
		receiver.receive("control", mustJSON(t, directFrame{
			Version: directFrameVersion, Kind: "control", Action: "activity",
			Data: `{"kind":"tick","seq":1}`,
		}))
	}
	if got := receiver.activityInbox(); len(got) != activityInboxCapacity {
		t.Fatalf("the inbox must cap at %d, got %d", activityInboxCapacity, len(got))
	}
}

// TestActivitySendRequiresBoundedFrame proves the outbound surface refuses
// frames that could never cross the control channel intact.
func TestActivitySendRequiresBoundedFrame(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))

	oversize := make([]byte, activityFrameMaxBytes+1)
	for i := range oversize {
		oversize[i] = 'a'
	}
	payload, err := json.Marshal(struct {
		CallID string `json:"call_id"`
		Events string `json:"events"`
	}{CallID: "c1", Events: string(oversize)})
	if err != nil {
		t.Fatal(err)
	}
	refused := svc.activitySend(requestForTest("activity.send", string(payload)))
	if refused.Error == nil || refused.Error.Code != "activity_too_large" {
		t.Fatalf("an oversized outbound frame must be refused: %#v", refused)
	}

	inactive := svc.activitySend(requestForTest("activity.send",
		`{"call_id":"c1","events":"{}"}`))
	if inactive.Error == nil || inactive.Error.Code != "call_inactive" {
		t.Fatalf("activity without a call must be refused: %#v", inactive)
	}
}

// TestProfileChannelRoundTrip proves an opaque profile frame sent by one
// worker surfaces from the other's profile.poll inbox, in order. The worker
// never interprets the bytes; the core owns revision ordering and schema.
func TestProfileChannelRoundTrip(t *testing.T) {
	sender := newChannelSet()
	receiver := newChannelSet()
	defer sender.close()
	defer receiver.close()

	if err := sender.sendProfile(`{"v":1,"revision":3}`); err == nil {
		t.Fatal("profile on a worker with no control channel must be refused")
	}

	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "profile",
		Data: `{"v":1,"revision":3}`,
	}))
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "profile",
		Data: `{"v":1,"revision":4}`,
	}))
	frames := receiver.profileInbox()
	if len(frames) != 2 || frames[0] != `{"v":1,"revision":3}` || frames[1] != `{"v":1,"revision":4}` {
		t.Fatalf("inbox must drain in order: %#v", frames)
	}
	if again := receiver.profileInbox(); len(again) != 0 {
		t.Fatalf("a drained inbox must stay empty: %#v", again)
	}
}

// TestProfileChannelBoundsAndRefuses proves oversized frames never enter
// the worker and a full inbox refuses rather than growing without bound.
func TestProfileChannelBoundsAndRefuses(t *testing.T) {
	receiver := newChannelSet()
	defer receiver.close()

	oversize := make([]byte, profileFrameMaxBytes+1)
	for i := range oversize {
		oversize[i] = 'a'
	}
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "profile",
		Data: string(oversize),
	}))
	if got := receiver.profileInbox(); len(got) != 0 {
		t.Fatalf("an oversized frame must be refused: %d frames", len(got))
	}
	receiver.receive("control", mustJSON(t, directFrame{
		Version: directFrameVersion, Kind: "control", Action: "profile",
		Data: ``,
	}))
	if got := receiver.profileInbox(); len(got) != 0 {
		t.Fatalf("an empty frame must be refused: %d frames", len(got))
	}

	for i := 0; i < profileInboxCapacity+4; i++ {
		receiver.receive("control", mustJSON(t, directFrame{
			Version: directFrameVersion, Kind: "control", Action: "profile",
			Data: `{"v":1,"revision":1}`,
		}))
	}
	if got := receiver.profileInbox(); len(got) != profileInboxCapacity {
		t.Fatalf("the inbox must cap at %d, got %d", profileInboxCapacity, len(got))
	}
}

// TestProfileSendRequiresBoundedFrame proves the outbound surface refuses
// frames that could never cross the control channel intact, and refuses
// honestly when no call is live.
func TestProfileSendRequiresBoundedFrame(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))

	oversize := make([]byte, profileFrameMaxBytes+1)
	for i := range oversize {
		oversize[i] = 'a'
	}
	payload, err := json.Marshal(struct {
		CallID string `json:"call_id"`
		Frame  string `json:"frame"`
	}{CallID: "c1", Frame: string(oversize)})
	if err != nil {
		t.Fatal(err)
	}
	refused := svc.profileSend(requestForTest("profile.send", string(payload)))
	if refused.Error == nil || refused.Error.Code != "profile_too_large" {
		t.Fatalf("an oversized outbound frame must be refused: %#v", refused)
	}

	inactive := svc.profileSend(requestForTest("profile.send",
		`{"call_id":"c1","frame":"{}"}`))
	if inactive.Error == nil || inactive.Error.Code != "call_inactive" {
		t.Fatalf("profile without a call must be refused: %#v", inactive)
	}
}

// TestAudioSwitchRequiresLiveCall proves audio.switch_devices is honest about
// a call that is not live instead of silently storing a selection.
func TestAudioSwitchRequiresLiveCall(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))
	response := svc.audioSwitch(requestForTest("audio.switch_devices",
		`{"call_id":"c1","input_device":"aaa1","output_device":"bbb1"}`))
	if response.Error == nil || response.Error.Code != "call_inactive" {
		t.Fatalf("switch without a call must be refused: %#v", response)
	}
}

// TestDeviceIDValidationRefusesGarbage proves a malformed device id from the
// core is refused before it can reach miniaudio.
func TestDeviceIDValidationRefusesGarbage(t *testing.T) {
	if _, err := parseDeviceID("not-hex!"); err == nil {
		t.Fatal("non-hex device ids must be refused")
	}
}

// mustJSON marshals a frame or fails the test.
func mustJSON(t *testing.T, frame directFrame) []byte {
	t.Helper()
	body, err := json.Marshal(frame)
	if err != nil {
		t.Fatal(err)
	}
	return body
}

// silence the unused-binary warning in builds that do not exercise it
var _ = binary.LittleEndian

func TestExtractCallStatsReducesThePionReport(t *testing.T) {
	report := webrtc.StatsReport{
		"pair": webrtc.ICECandidatePairStats{
			Nominated:            true,
			State:                webrtc.StatsICECandidatePairStateSucceeded,
			CurrentRoundTripTime: 0.042,
		},
		"audio-in": webrtc.InboundRTPStreamStats{
			Kind:            "audio",
			PacketsReceived: 980,
			PacketsLost:     10,
		},
		// A video stream (screen share) must not be mistaken for voice.
		"video-in": webrtc.InboundRTPStreamStats{
			Kind:            "video",
			PacketsReceived: 5000,
			PacketsLost:     400,
		},
	}
	stats, ok := extractCallStats(report)
	if !ok {
		t.Fatal("a nominated pair and an audio stream must yield stats")
	}
	if stats.RttMs != 42.0 {
		t.Fatalf("rtt in seconds must surface as ms, got %v", stats.RttMs)
	}
	if stats.Received != 980 || stats.Lost != 10 {
		t.Fatalf("audio counters must win over video, got %v/%v", stats.Received, stats.Lost)
	}
}

func TestExtractCallStatsStaysSilentWithoutRealFacts(t *testing.T) {
	// Pre-connect: no nominated pair, no inbound stream. The tick must stay
	// silent instead of publishing zeros that look like measurements.
	empty, ok := extractCallStats(webrtc.StatsReport{})
	if ok {
		t.Fatal("an empty report must not produce stats")
	}
	if empty != (callStats{}) {
		t.Fatalf("no facts must mean zero facts, got %v", empty)
	}

	// A nominated pair without an audio stream yet is still incomplete.
	pairOnly, ok := extractCallStats(webrtc.StatsReport{
		"pair": webrtc.ICECandidatePairStats{
			Nominated:            true,
			CurrentRoundTripTime: 0.05,
		},
	})
	if ok {
		t.Fatal("a report without audio counters must not produce stats")
	}
	if pairOnly != (callStats{}) {
		t.Fatalf("no facts must mean zero facts, got %v", pairOnly)
	}
}

// TestVoiceActivationHandlerIsCallScoped proves the mode only lands on a
// live call: an idle worker refuses it instead of absorbing a fact no
// pipeline would read, and a malformed patch is a protocol error.
func TestVoiceActivationHandlerIsCallScoped(t *testing.T) {
	svc := newService(bytes.NewBuffer(nil))

	reply, _ := svc.dispatch(requestForTest("call.voice_activation", `{"enabled":true}`))
	if reply.Error == nil || reply.Error.Code != "call_inactive" {
		t.Fatalf("an idle worker must refuse voice activation: %#v", reply.Error)
	}

	reply, _ = svc.dispatch(requestForTest("call.voice_activation", `{}`))
	if reply.Error == nil || reply.Error.Code != "invalid_request" {
		t.Fatalf("a patch without a mode must be invalid: %#v", reply.Error)
	}
}
