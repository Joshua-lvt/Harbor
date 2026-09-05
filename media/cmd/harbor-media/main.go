// harbor-media is Harbor's private, supervised WebRTC worker.
//
// It has no UI, durable state, Harbor identity, network-control client, or
// fallback relay. Its only interface is framed stdin/stdout with harbor-core.
package main

import (
	"bufio"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"sync"
	"sync/atomic"
	"time"

	"github.com/google/uuid"
	"github.com/pion/webrtc/v4"
	"github.com/pion/webrtc/v4/pkg/media"
)

const (
	protocolVersion = 1
	maxFrameBytes   = 256 * 1024
)

type envelope struct {
	Version   int             `json:"v"`
	Type      string          `json:"type"`
	RequestID string          `json:"request_id,omitempty"`
	EventID   string          `json:"event_id,omitempty"`
	ReplyTo   string          `json:"reply_to,omitempty"`
	Timestamp string          `json:"timestamp"`
	Payload   json.RawMessage `json:"payload"`
	Error     *mediaError     `json:"error,omitempty"`
}

type mediaError struct {
	Code      string `json:"code"`
	UIKey     string `json:"ui_key"`
	Retryable bool   `json:"retryable"`
	Detail    string `json:"detail"`
}

type service struct {
	writeMu sync.Mutex
	stateMu sync.Mutex
	output  io.Writer
	pc      *webrtc.PeerConnection
	callID  string
	muted   bool
	voice   *voicePipeline
	share   *sharePipeline
	screen  *webrtc.TrackLocalStaticSample
	// push-to-talk, voice activation and the per-stream volumes are
	// call-scoped facts the voice pipeline consults per frame. Manual mute
	// always wins over both modes, and an enabled PTT outranks voice
	// activation: explicit key intent beats automatic detection.
	pttEnabled      bool
	pttActive       bool
	voiceActivation bool
	inputVolume     float64
	outputVolume    float64
	// selectedDevices names the concrete capture/playback devices calls open;
	// empty ids mean the session defaults. A switch applies to live calls
	// through the pipeline's switchable boundary and to the next call anyway.
	selectedDevices deviceSelection
	// loopback is the microphone self-check. It borrows the selected
	// devices outside any call and never joins a session.
	loopback *loopbackTest
	// channels holds the call's direct DataChannel surfaces (chat, control,
	// file) and the bounded inbound buffers the core polls. It exists only
	// while a call is live and carries no durable state.
	channels *channelSet
	// audioFactory is installed explicitly by main() (real devices) and by
	// automated environments (the silent boundary). The zero value yields the
	// silent boundary so refusal tests never need audio hardware.
	audioFactory func(selection deviceSelection) (audioIO, error)
	// screenFactory is installed explicitly by main() (real capture where
	// the platform boundary exists — X11 on Linux, GDI on Windows builds
	// with -tags harborvpx; an honest refusal on Wayland, plain Windows
	// builds, and anything else). Tests inject a deterministic boundary.
	screenFactory func() (screenCapture, error)
	// inboundVideoPackets counts VP8 packets received from the peer's screen
	// track. The inbound track is read so the direct path stays healthy, and
	// the count is the call-scoped fact the future rendering path and the
	// proof tests build on.
	inboundVideo atomic.Uint64
	// statsStop ends the call's transport sampler (stats.go); nil while no
	// call is live. It exists only under stateMu like the call itself.
	statsStop chan struct{}
}

func main() {
	factory := openSelectedAudio
	if os.Getenv("HARBOR_MEDIA_AUDIO") == "silent" {
		factory = func(deviceSelection) (audioIO, error) { return openSilentAudio() }
	}
	service := newService(bufio.NewWriter(os.Stdout))
	service.audioFactory = factory
	service.screenFactory = openScreenCapture
	if err := service.serve(os.Stdin); err != nil {
		// Detail never crosses IPC. The core discards this private diagnostic
		// stream so worker output cannot block or cross into the UI.
		fmt.Fprintf(os.Stderr, "harbor-media: %v\n", err)
		os.Exit(1)
	}
}

// newService builds a service with the neutral defaults a real session
// starts from: volumes at unity. The zero value would be permanent silence,
// so the constructor — not the struct literal — is the supported way in.
func newService(output io.Writer) *service {
	return &service{
		output:       output,
		inputVolume:  1,
		outputVolume: 1,
		loopback:     newLoopbackTest(),
	}
}

// captureOpen is the manual gate the voice pipeline consults before
// measuring a frame: a muted mic, or push-to-talk with the key up, reports
// honest silence instead of a level nobody will transmit.
func (s *service) captureOpen() bool {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	if s.muted {
		return false
	}
	if s.pttEnabled {
		return s.pttActive
	}
	return true
}

// transmit is the wire gate for an open capture. Manual mute always wins:
// neither mode can re-enable a microphone the user muted by hand. Push-to-
// talk narrows an unmuted mic to key-held frames; voice activation narrows
// it to frames the speech detector marked as speaking; without either mode
// an unmuted mic transmits everything it captures.
func (s *service) transmit(speaking bool) bool {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	if s.muted {
		return false
	}
	if s.pttEnabled {
		return s.pttActive
	}
	if s.voiceActivation {
		return speaking
	}
	return true
}

// isMuted feeds legacy callers; the pipeline itself consults the gates.
func (s *service) isMuted() bool {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	return s.muted
}

func (s *service) serve(input io.Reader) error {
	for {
		request, err := readFrame(input)
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
		if err := validateRequest(request); err != nil {
			return err
		}
		response, shutdown := s.dispatch(request)
		if err := s.write(response); err != nil {
			return err
		}
		if shutdown {
			s.closePeerConnection()
			return nil
		}
	}
}

func validateRequest(value envelope) error {
	if value.Version != protocolVersion || value.Type == "" || value.RequestID == "" || value.Timestamp == "" {
		return errors.New("invalid private media envelope")
	}
	if _, err := uuid.Parse(value.RequestID); err != nil || value.EventID != "" || value.ReplyTo != "" {
		return errors.New("invalid private media correlation")
	}
	if !isPrivateCommand(value.Type) {
		return errors.New("private media command is not allowed")
	}
	if len(value.Payload) == 0 || !json.Valid(value.Payload) {
		return errors.New("invalid private media payload")
	}
	return nil
}

func isPrivateCommand(kind string) bool {
	switch kind {
	case "media.hello", "media.shutdown", "call.start", "call.accept", "call.end", "call.mute",
		"call.ptt", "call.voice_activation", "call.remote_signal", "call.share_start", "call.share_stop", "chat.send", "chat.poll", "chat.status",
		"transfer.begin", "transfer.accept", "transfer.reject", "transfer.send_chunk", "transfer.recv_chunk",
		"transfer.cancel", "transfer.finalize", "transfer.poll",
		"audio.devices", "audio.config", "audio.switch_devices",
		"audio.loopback_start", "audio.loopback_poll", "audio.loopback_stop",
		"activity.send", "activity.poll",
		"profile.send", "profile.poll":
		return true
	default:
		return false
	}
}

func responseFor(request envelope, payload any) envelope {
	body, _ := json.Marshal(payload)
	return envelope{Version: protocolVersion, Type: request.Type, RequestID: uuid.NewString(), ReplyTo: request.RequestID,
		Timestamp: time.Now().UTC().Format(time.RFC3339Nano), Payload: body}
}

func errorFor(request envelope, code, uiKey, detail string, retryable bool) envelope {
	response := responseFor(request, map[string]any{})
	response.Error = &mediaError{Code: code, UIKey: uiKey, Detail: detail, Retryable: retryable}
	return response
}

func (s *service) event(kind string, payload any) {
	body, err := json.Marshal(payload)
	if err != nil {
		return
	}
	// Pion invokes callbacks on its own goroutines. write serializes their
	// framed output with replies, so the core never sees an interleaved frame.
	_ = s.write(envelope{Version: protocolVersion, Type: kind,
		EventID: uuid.NewString(), Timestamp: time.Now().UTC().Format(time.RFC3339Nano), Payload: body})
}

func (s *service) write(value envelope) error {
	body, err := json.Marshal(value)
	if err != nil {
		return err
	}
	if len(body) == 0 || len(body) > maxFrameBytes {
		return errors.New("outbound private media frame exceeds limit")
	}
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	var prefix [4]byte
	binary.BigEndian.PutUint32(prefix[:], uint32(len(body)))
	if _, err := s.output.Write(prefix[:]); err != nil {
		return err
	}
	if _, err := s.output.Write(body); err != nil {
		return err
	}
	if flush, ok := s.output.(interface{ Flush() error }); ok {
		return flush.Flush()
	}
	return nil
}

func readFrame(input io.Reader) (envelope, error) {
	var prefix [4]byte
	if _, err := io.ReadFull(input, prefix[:]); err != nil {
		return envelope{}, err
	}
	length := binary.BigEndian.Uint32(prefix[:])
	if length == 0 || length > maxFrameBytes {
		return envelope{}, errors.New("private media frame length is invalid")
	}
	body := make([]byte, length)
	if _, err := io.ReadFull(input, body); err != nil {
		return envelope{}, err
	}
	var value envelope
	if err := json.Unmarshal(body, &value); err != nil {
		return envelope{}, err
	}
	return value, nil
}

func (s *service) dispatch(request envelope) (envelope, bool) {
	switch request.Type {
	case "media.hello":
		return responseFor(request, map[string]any{
			"service": "harbor-media", "protocol": protocolVersion,
			// The worker can create an offer and gather direct host candidates,
			// complete a direct connection from a peer's answer plus the
			// candidates the core relays, carry voice through its Opus
			// pipeline with real device selection and volume, and move chat,
			// files, and activity across the direct DataChannels.
			// Device availability is proved per call: an audio device
			// that cannot be opened refuses the call honestly, and a screen
			// capture that cannot start refuses the share honestly.
			"capabilities": []string{"host-ice", "local-offer", "remote-answer", "audio-opus", "video-share", "chat-channel", "file-transfer", "audio-devices", "push-to-talk", "voice-activation", 		"voice-level", "activity-channel", "profile-channel", "audio-loopback"},
		}), false
	case "call.start":
		return s.startCall(request), false
	case "call.accept":
		return s.acceptCall(request), false
	case "call.remote_signal":
		return s.applyRemoteSignal(request), false
	case "call.end":
		s.closePeerConnection()
		return responseFor(request, map[string]any{"state": "ENDED"}), false
	case "call.mute":
		var payload struct {
			Muted bool `json:"muted"`
		}
		if json.Unmarshal(request.Payload, &payload) != nil {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "mute payload is invalid", false), false
		}
		s.stateMu.Lock()
		active := s.pc != nil
		if active {
			s.muted = payload.Muted
		}
		muted := s.muted
		s.stateMu.Unlock()
		if !active {
			return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false), false
		}
		// The voice pipeline consults this flag before every frame, so a muted
		// microphone contributes nothing to the wire from now on.
		return responseFor(request, map[string]any{"muted": muted}), false
	case "call.ptt":
		var payload struct {
			Enabled *bool `json:"enabled"`
			Active  *bool `json:"active"`
		}
		if json.Unmarshal(request.Payload, &payload) != nil {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "push-to-talk payload is invalid", false), false
		}
		s.stateMu.Lock()
		active := s.pc != nil
		if active {
			if payload.Enabled != nil {
				s.pttEnabled = *payload.Enabled
				// Enabling PTT mid-call starts idle: a held key from before
				// the mode existed must not open the channel.
				s.pttActive = false
			}
			if payload.Active != nil && s.pttEnabled {
				s.pttActive = *payload.Active
			}
		}
		enabled, held := s.pttEnabled, s.pttActive
		s.stateMu.Unlock()
		if !active {
			return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false), false
		}
		return responseFor(request, map[string]any{"enabled": enabled, "active": held}), false
	case "call.voice_activation":
		var payload struct {
			Enabled *bool `json:"enabled"`
		}
		if json.Unmarshal(request.Payload, &payload) != nil || payload.Enabled == nil {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "voice activation payload is invalid", false), false
		}
		s.stateMu.Lock()
		active := s.pc != nil
		if active {
			s.voiceActivation = *payload.Enabled
		}
		enabled := s.voiceActivation
		s.stateMu.Unlock()
		if !active {
			return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false), false
		}
		return responseFor(request, map[string]any{"enabled": enabled}), false
	case "call.share_start":
		return s.startShare(request), false
	case "call.share_stop":
		return s.stopShare(request), false
	case "audio.devices":
		return s.audioDevices(request), false
	case "audio.config":
		return s.audioConfig(request), false
	case "audio.switch_devices":
		return s.audioSwitch(request), false
	case "audio.loopback_start":
		return s.loopbackStart(request), false
	case "audio.loopback_poll":
		return s.loopbackPoll(request), false
	case "audio.loopback_stop":
		return s.loopbackStop(request), false
	case "activity.send":
		return s.activitySend(request), false
	case "activity.poll":
		return s.activityPoll(request), false
	case "profile.send":
		return s.profileSend(request), false
	case "profile.poll":
		return s.profilePoll(request), false
	case "chat.send":
		return s.chatSend(request), false
	case "chat.poll":
		return s.chatPoll(request), false
	case "chat.status":
		return s.chatStatus(request), false
	case "transfer.begin":
		return s.transferBegin(request), false
	case "transfer.accept":
		return s.transferSettle(request, true), false
	case "transfer.reject":
		return s.transferSettle(request, false), false
	case "transfer.send_chunk":
		return s.transferSend(request), false
	case "transfer.recv_chunk":
		return s.transferRecv(request), false
	case "transfer.cancel":
		return s.transferCancel(request), false
	case "transfer.finalize":
		return s.transferFinalize(request), false
	case "transfer.poll":
		return s.transferPoll(request), false
	case "media.shutdown":
		return responseFor(request, map[string]any{"accepted": true}), true
	default:
		return errorFor(request, "capability_unavailable", "error.media.capabilityUnavailable", "media command is not enabled", false), false
	}
}

// newCallPeerConnection builds the direct-connection plumbing shared by both
// sides of a call: host candidates only, connection-state events, relayed ICE,
// one Opus track that the voice pipeline feeds, and one VP8 track the share
// pipeline feeds. The video track is negotiated with the call itself, so a
// share start writes frames onto an existing sender and never renegotiates
// the audio track away.
func (s *service) newCallPeerConnection(callID string) (*webrtc.PeerConnection, *callTracks, error) {
	// Host candidates only. There is deliberately no TURN config or fallback.
	pc, err := webrtc.NewPeerConnection(webrtc.Configuration{})
	if err != nil {
		return nil, nil, err
	}
	pc.OnConnectionStateChange(func(state webrtc.PeerConnectionState) {
		s.event("media.call_state", map[string]any{"call_id": callID, "state": state.String()})
	})
	pc.OnICECandidate(func(candidate *webrtc.ICECandidate) {
		if candidate == nil {
			return
		}
		s.event("media.ice_candidate", map[string]any{"call_id": callID, "candidate": candidate.ToJSON()})
	})
	audioTrack, err := webrtc.NewTrackLocalStaticSample(webrtc.RTPCodecCapability{MimeType: webrtc.MimeTypeOpus}, "audio", callID)
	if err != nil {
		_ = pc.Close()
		return nil, nil, err
	}
	if _, err = pc.AddTrack(audioTrack); err != nil {
		_ = pc.Close()
		return nil, nil, err
	}
	screenTrack, err := webrtc.NewTrackLocalStaticSample(webrtc.RTPCodecCapability{MimeType: webrtc.MimeTypeVP8}, "screen", callID)
	if err != nil {
		_ = pc.Close()
		return nil, nil, err
	}
	if _, err = pc.AddTrack(screenTrack); err != nil {
		_ = pc.Close()
		return nil, nil, err
	}
	return pc, &callTracks{audio: audioTrack, screen: screenTrack}, nil
}

// callTracks bundles the tracks one call owns. The voice pipeline writes the
// audio track; the share pipeline writes the screen track only while a share
// is active — an idle share track sends nothing.
type callTracks struct {
	audio  *webrtc.TrackLocalStaticSample
	screen *webrtc.TrackLocalStaticSample
}

// prepareVoice opens the call's audio boundary and builds its pipeline. Device
// failure is the honest refusal point: a voice call without devices fails
// before any call state is published, locally or on the peer. The boundary is
// wrapped in a switchableAudio so a device switch can swap the live devices
// without touching the pipeline.
func (s *service) prepareVoice(callID string, track *webrtc.TrackLocalStaticSample) (*voicePipeline, audioIO, error) {
	s.stateMu.Lock()
	selection := s.selectedDevices
	s.stateMu.Unlock()
	inner, err := s.openAudio(selection)
	if err != nil {
		return nil, nil, err
	}
	audio := newSwitchableAudio(inner)
	voice, err := newVoicePipeline(audio, track, s.captureOpen, s.transmit,
		s.inputGain, s.outputGain,
		func(local, remote float64, localSpeaking, remoteSpeaking bool) {
			// Level facts are call-scoped UI material, not protocol state.
			s.event("media.voice_level", map[string]any{
				"call_id": callID, "level": local, "speaking": localSpeaking,
				"remote_level": remote, "remote_speaking": remoteSpeaking,
				"muted": s.isMuted(),
			})
		},
		func() {
			// A device that dies mid-call fails the call visibly, exactly like a
			// Pion connection failure would.
			s.event("media.call_state", map[string]any{"call_id": callID, "state": "failed"})
		})
	if err != nil {
		audio.Close()
		return nil, nil, err
	}
	return voice, audio, nil
}

func (s *service) inputGain() float64 {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	return s.inputVolume
}

func (s *service) outputGain() float64 {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	return s.outputVolume
}

func (s *service) openAudio(selection deviceSelection) (audioIO, error) {
	factory := s.audioFactory
	if factory == nil {
		factory = func(deviceSelection) (audioIO, error) { return openSilentAudio() }
	}
	return factory(selection)
}

// publishCall attaches a freshly prepared call as the active one unless
// another call won the race; the loser is fully torn down, never leaked.
func (s *service) publishCall(request envelope, pc *webrtc.PeerConnection,
	voice *voicePipeline, audio audioIO, tracks *callTracks, callID string,
	muted, pttEnabled, voiceActivation bool, signal any) envelope {
	s.stateMu.Lock()
	if s.pc != nil {
		s.stateMu.Unlock()
		_ = pc.Close()
		teardownVoice(voice, audio)
		return errorFor(request, "call_active", "error.call.alreadyActive", "a call is already active", false)
	}
	s.pc = pc
	s.callID = callID
	// The host supplies the endpoint policy. Mobile calls start muted and
	// only an explicit call.mute request may open the source gate.
	s.muted = muted
	s.pttEnabled = pttEnabled
	// A new call starts with the key up: a stale held state from a previous
	// call must never open the microphone.
	s.pttActive = false
	s.voiceActivation = voiceActivation
	s.voice = voice
	s.screen = tracks.screen
	s.stateMu.Unlock()
	// The transport sampler is a child of the published call, like the
	// voice and share pipelines; closePeerConnection stops it.
	s.startStatsTicker(callID)
	return responseFor(request, map[string]any{"state": "CONNECTING", "call_id": callID, "signal": signal})
}

// teardownVoice stops the pipeline in the only safe order: the peer
// connection is already closed (ending track reads), then the audio boundary
// (ending blocking reads), then the pumps are joined and codecs released.
func teardownVoice(voice *voicePipeline, audio audioIO) {
	voice.cancel()
	audio.Close()
	voice.done.Wait()
	voice.close()
}

func (s *service) startCall(request envelope) envelope {
	var payload struct {
		CallID          string   `json:"call_id"`
		Muted           bool     `json:"muted"`
		PttEnabled      bool     `json:"ptt_enabled"`
		VoiceActivation bool     `json:"voice_activation"`
		InputDevice     *string  `json:"input_device"`
		OutputDevice    *string  `json:"output_device"`
		InputVolume     *float64 `json:"input_volume"`
		OutputVolume    *float64 `json:"output_volume"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.CallID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "call_id is required", false)
	}
	s.applyCallAudioConfig(payload.InputDevice, payload.OutputDevice, payload.InputVolume, payload.OutputVolume)

	s.stateMu.Lock()
	active := s.pc != nil
	s.stateMu.Unlock()
	if active {
		return errorFor(request, "call_active", "error.call.alreadyActive", "a call is already active", false)
	}
	// Set the gate before prepareVoice starts its capture goroutine. Otherwise
	// a mobile call could leak a few samples during the publish transition.
	s.stateMu.Lock()
	s.muted = payload.Muted
	s.stateMu.Unlock()

	pc, tracks, err := s.newCallPeerConnection(payload.CallID)
	if err != nil {
		return errorFor(request, "media_unavailable", "error.call.unavailable", "could not prepare direct connection", true)
	}
	// The offering side declares the direct channels; the answering side
	// binds the peer's channels from its OnDataChannel callbacks.
	s.openChannels(pc)
	voice, audio, err := s.prepareVoice(payload.CallID, tracks.audio)
	if err != nil {
		_ = pc.Close()
		return errorFor(request, "audio_unavailable", "error.call.audioUnavailable", "no audio device could be opened", true)
	}
	pc.OnTrack(func(remote *webrtc.TrackRemote, _ *webrtc.RTPReceiver) {
		if remote.Kind() == webrtc.RTPCodecTypeVideo {
			s.drainInboundVideo(remote)
			return
		}
		voice.receiveFrom(remote)
	})
	offer, err := pc.CreateOffer(nil)
	if err != nil || pc.SetLocalDescription(offer) != nil {
		_ = pc.Close()
		teardownVoice(voice, audio)
		return errorFor(request, "media_unavailable", "error.call.unavailable", "could not create direct offer", true)
	}
	return s.publishCall(request, pc, voice, audio, tracks, payload.CallID, payload.Muted, payload.PttEnabled, payload.VoiceActivation, map[string]string{"type": offer.Type.String(), "sdp": offer.SDP})
}

// acceptCall answers an inbound offer the core authenticated. The worker never
// learns who the peer is; it only negotiates the direct media path.
func (s *service) acceptCall(request envelope) envelope {
	var payload struct {
		CallID          string   `json:"call_id"`
		Muted           bool     `json:"muted"`
		PttEnabled      bool     `json:"ptt_enabled"`
		VoiceActivation bool     `json:"voice_activation"`
		InputDevice     *string  `json:"input_device"`
		OutputDevice    *string  `json:"output_device"`
		InputVolume     *float64 `json:"input_volume"`
		OutputVolume    *float64 `json:"output_volume"`
		Signal          struct {
			Type string `json:"type"`
			SDP  string `json:"sdp"`
		} `json:"signal"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.CallID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "call_id and offer are required", false)
	}
	s.applyCallAudioConfig(payload.InputDevice, payload.OutputDevice, payload.InputVolume, payload.OutputVolume)
	if payload.Signal.Type != "offer" || payload.Signal.SDP == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "an offer description is required", false)
	}

	s.stateMu.Lock()
	active := s.pc != nil
	s.stateMu.Unlock()
	if active {
		return errorFor(request, "call_active", "error.call.alreadyActive", "a call is already active", false)
	}
	s.stateMu.Lock()
	s.muted = payload.Muted
	s.stateMu.Unlock()

	pc, tracks, err := s.newCallPeerConnection(payload.CallID)
	if err != nil {
		return errorFor(request, "media_unavailable", "error.call.unavailable", "could not prepare direct connection", true)
	}
	// The answering side binds whatever direct channels the offer declared.
	s.receiveChannels(pc)
	voice, audio, err := s.prepareVoice(payload.CallID, tracks.audio)
	if err != nil {
		_ = pc.Close()
		return errorFor(request, "audio_unavailable", "error.call.audioUnavailable", "no audio device could be opened", true)
	}
	pc.OnTrack(func(remote *webrtc.TrackRemote, _ *webrtc.RTPReceiver) {
		if remote.Kind() == webrtc.RTPCodecTypeVideo {
			s.drainInboundVideo(remote)
			return
		}
		voice.receiveFrom(remote)
	})
	if err = pc.SetRemoteDescription(webrtc.SessionDescription{Type: webrtc.SDPTypeOffer, SDP: payload.Signal.SDP}); err != nil {
		_ = pc.Close()
		teardownVoice(voice, audio)
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "offer description is unusable", false)
	}
	answer, err := pc.CreateAnswer(nil)
	if err != nil || pc.SetLocalDescription(answer) != nil {
		_ = pc.Close()
		teardownVoice(voice, audio)
		return errorFor(request, "media_unavailable", "error.call.unavailable", "could not create direct answer", true)
	}
	return s.publishCall(request, pc, voice, audio, tracks, payload.CallID, payload.Muted, payload.PttEnabled, payload.VoiceActivation, map[string]string{"type": answer.Type.String(), "sdp": answer.SDP})
}

// applyRemoteSignal feeds the active call the peer's answer or ICE candidates
// relayed by the core. Without an active call there is nothing to apply. The
// peer connection is snapshotted under the state lock; a concurrent call.end
// may close it, in which case Pion refuses the operation and it is reported.
func (s *service) applyRemoteSignal(request envelope) envelope {
	var payload struct {
		CallID string `json:"call_id"`
		Signal struct {
			Type      string                  `json:"type"`
			SDP       string                  `json:"sdp"`
			Candidate webrtc.ICECandidateInit `json:"candidate"`
		} `json:"signal"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.CallID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "call_id and signal are required", false)
	}
	s.stateMu.Lock()
	pc := s.pc
	active := pc != nil && s.callID == payload.CallID
	s.stateMu.Unlock()
	if !active {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	switch payload.Signal.Type {
	case "answer":
		if payload.Signal.SDP == "" {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "answer description is required", false)
		}
		if err := pc.SetRemoteDescription(webrtc.SessionDescription{Type: webrtc.SDPTypeAnswer, SDP: payload.Signal.SDP}); err != nil {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "answer description is unusable", false)
		}
	case "candidate":
		if payload.Signal.Candidate.Candidate == "" {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "candidate is required", false)
		}
		if err := pc.AddICECandidate(payload.Signal.Candidate); err != nil {
			return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "candidate is unusable", false)
		}
	default:
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "signal type is not supported", false)
	}
	return responseFor(request, map[string]any{"applied": true})
}

// pionVideoTrack adapts the call's negotiated Pion track to the share
// pipeline's narrow videoTrack boundary.
type pionVideoTrack struct {
	track *webrtc.TrackLocalStaticSample
}

func (t pionVideoTrack) WriteEncoded(frame []byte, duration time.Duration) error {
	return t.track.WriteSample(media.Sample{Data: frame, Duration: duration})
}

// drainInboundVideo consumes the peer's screen track until the call ends.
// Harbor renders no incoming video yet; the frames are counted and dropped,
// which keeps the RTP path healthy without pretending anything is displayed.
func (s *service) drainInboundVideo(remote *webrtc.TrackRemote) {
	for {
		if _, _, err := remote.ReadRTP(); err != nil {
			return
		}
		s.inboundVideo.Add(1)
	}
}

// startShare attaches real screen capture to the active call's video track.
// The video track was negotiated with the call, so this only opens the capture
// and encoder boundaries and starts writing frames — no renegotiation. Any
// boundary failure (no display, encoder init) refuses the share honestly and
// leaves the call untouched.
func (s *service) startShare(request envelope) envelope {
	var payload struct {
		CallID string `json:"call_id"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.CallID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "call_id is required", false)
	}

	s.stateMu.Lock()
	active := s.pc != nil && s.callID == payload.CallID
	share := s.share
	s.stateMu.Unlock()
	if !active {
		return errorFor(request, "call_inactive", "error.call.inactive", "a connected call is required before sharing", false)
	}
	if share != nil {
		return responseFor(request, map[string]any{"state": "SHARING"})
	}

	factory := s.screenFactory
	if factory == nil {
		// Tests run without a capture boundary; the honest answer stays the
		// honest answer.
		return errorFor(request, "capture_unavailable", "error.call.screenShareUnavailable", "no native capture adapter is available", false)
	}
	capture, err := factory()
	if err != nil {
		return errorFor(request, "capture_unavailable", "error.call.screenShareUnavailable", "screen capture is unavailable on this session", false)
	}
	width, height := capture.Size()
	encoder, err := newVPXEncoder(width, height)
	if err != nil {
		capture.Close()
		return errorFor(request, "capture_unavailable", "error.call.screenShareUnavailable", "the video encoder could not be prepared", false)
	}

	// The call may have ended while the boundaries were opening; the share is
	// published only if its call still owns the connection.
	s.stateMu.Lock()
	if s.pc == nil || s.callID != payload.CallID || s.share != nil {
		s.stateMu.Unlock()
		encoder.Close()
		capture.Close()
		return errorFor(request, "call_inactive", "error.call.inactive", "the call ended before sharing started", false)
	}
	pipeline := newSharePipeline(capture, encoder, pionVideoTrack{track: s.screen})
	s.share = pipeline
	s.stateMu.Unlock()
	return responseFor(request, map[string]any{"state": "SHARING"})
}

// stopShare stops the frame pump and releases the capture and encoder
// boundaries. The call and the voice pipeline are untouched: stopping a share
// never ends a call.
func (s *service) stopShare(request envelope) envelope {
	s.stateMu.Lock()
	share := s.share
	s.share = nil
	s.stateMu.Unlock()
	if share != nil {
		teardownShare(share)
	}
	return responseFor(request, map[string]any{"state": "NOT_SHARING"})
}

// teardownShare stops the share in the only safe order: cancel the pump, then
// the capture boundary, then join the pump, then release the encoder.
func teardownShare(share *sharePipeline) {
	share.cancel()
	share.capture.Close()
	share.done.Wait()
	share.encoder.Close()
}

func (s *service) closePeerConnection() {
	// The sampler reads the peer connection; it stops before the connection
	// closes so no tick races the teardown.
	s.stopStatsTicker()
	s.stateMu.Lock()
	pc := s.pc
	voice := s.voice
	share := s.share
	channels := s.channels
	s.pc = nil
	s.callID = ""
	s.muted = false
	s.pttEnabled = false
	s.pttActive = false
	s.voice = nil
	s.share = nil
	s.screen = nil
	s.channels = nil
	s.stateMu.Unlock()
	if pc != nil {
		_ = pc.Close()
	}
	if channels != nil {
		// The direct chat/transfer surfaces are children of the call too.
		channels.close()
	}
	if share != nil {
		// Ending the call always ends the share first, then the voice.
		teardownShare(share)
	}
	if voice != nil {
		// The pipeline owns its audio boundary; cancel, stop it, and join the
		// pumps so no goroutine outlives the call.
		teardownVoice(voice, voice.audio)
	}
}
