// Harbor's private voice pipeline.
//
// The pipeline owns one call's audio: it encodes captured microphone frames
// onto the call's send track and decodes the peer's track into playback. The
// only OS-specific code lives behind the audioIO boundary — everything else
// (codec, mute, RTP pumping) is platform-independent and covered by tests.
//
// Mute is enforced at the source: a muted frame never reaches the encoder, so
// silence on the wire is real silence, not a flag the peer has to interpret.
package main

import (
	"context"
	"encoding/binary"
	"errors"
	"sync"
	"time"

	"github.com/gen2brain/malgo"
	"github.com/pion/rtp/codecs"
	"github.com/pion/webrtc/v4"
	"github.com/pion/webrtc/v4/pkg/media"
	"github.com/pion/webrtc/v4/pkg/media/samplebuilder"
)

const (
	audioSampleRate = 48000
	audioChannels   = 1
	// One Opus frame: 10 ms of 48 kHz mono PCM16.
	frameSamples  = 480
	frameBytes    = frameSamples * 2
	frameDuration = 10 * time.Millisecond
	// Comfortably above Opus's 1275-byte per-frame ceiling.
	maxOpusFrameBytes = 4000
	// Opus's longest frame is 120 ms.
	decoderMaxSamples = 5760 * audioChannels
	// ~320 ms of buffered playback keeps underruns rare without building
	// unbounded latency when the peer's clock drifts.
	ringFrames = 32
)

// audioIO is the platform boundary: a microphone source and a speaker sink
// that exchange fixed PCM16 mono frames at the shared 10 ms cadence.
type audioIO interface {
	// Read returns the next captured microphone frame, blocking for one frame.
	Read() ([]byte, error)
	// Write submits one decoded frame for playback.
	Write(frame []byte) error
	// Close stops the devices; later reads and writes fail instead of blocking.
	Close()
}

// voicePipeline owns one call's audio. It starts pumping as soon as the call
// is published — samples written before the peers connect are simply not
// packetized yet — and every pump exit path is observed by shutdown.
type voicePipeline struct {
	audio audioIO
	// boundary is the same device set as audio when no live switch is
	// possible; a call that supports device switching wraps its devices in a
	// switchableAudio so a mid-call swap never touches the pipeline.
	boundary *switchableAudio
	encoder  *opusEncoder
	decoder  *opusDecoder
	track    *webrtc.TrackLocalStaticSample
	cancel   context.CancelFunc
	// captureOpen is the manual gate: whether captured audio may be
	// considered at all. A closed gate reports honest silence (mute, or PTT
	// with the key up) instead of a level the peer has to interpret.
	captureOpen func() bool
	// transmit is the wire gate for an open capture: manual mute always
	// wins, push-to-talk narrows an unmuted mic, and voice activation opens
	// it only while the detector holds a speaking flag.
	transmit func(speaking bool) bool
	// inputGain/outputGain are Harbor's own stream volumes (0..1); they never
	// touch the system mixer.
	inputGain  func() float64
	outputGain func() float64
	// onFault reports an audio-device death mid-call; the service decides how
	// the call fails. Normal shutdown must not raise it.
	onFault func()
	// onLevel publishes smoothed voice levels (~10 Hz) for the call UI.
	onLevel func(local, remote float64, localSpeaking, remoteSpeaking bool)
	// remoteLevel is the playback-side detector the receive loop feeds; the
	// send loop reads it when publishing level facts.
	remoteLevel *speechDetector
	done        sync.WaitGroup
}

func newVoicePipeline(audio audioIO, track *webrtc.TrackLocalStaticSample,
	captureOpen func() bool, transmit func(speaking bool) bool,
	inputGain, outputGain func() float64,
	onLevel func(local, remote float64, localSpeaking, remoteSpeaking bool),
	onFault func()) (*voicePipeline, error) {
	encoder, err := newOpusEncoder(audioSampleRate, audioChannels, opusApplicationVoIP)
	if err != nil {
		return nil, err
	}
	decoder, err := newOpusDecoder(audioSampleRate, audioChannels)
	if err != nil {
		encoder.Close()
		return nil, err
	}
	ctx, cancel := context.WithCancel(context.Background())
	boundary, _ := audio.(*switchableAudio)
	voice := &voicePipeline{
		audio: audio, boundary: boundary, encoder: encoder, decoder: decoder, track: track,
		cancel: cancel, captureOpen: captureOpen, transmit: transmit,
		inputGain: inputGain, outputGain: outputGain,
		onLevel: onLevel, onFault: onFault,
		remoteLevel: &speechDetector{},
	}
	voice.done.Add(1)
	go voice.sendLoop(ctx)
	return voice, nil
}

// receiveFrom pumps the peer's audio track once negotiation delivers it. The
// spawn must happen before shutdown begins waiting: teardown closes the peer
// connection first, which ends this track's reads.
func (v *voicePipeline) receiveFrom(remote *webrtc.TrackRemote) {
	v.done.Add(1)
	go v.recvLoop(remote)
}

// switchDevices swaps the call's live devices through the boundary wrapper.
// A call opened without the wrapper reports honestly that switching needs a
// device set that supports it.
func (v *voicePipeline) switchDevices(open func() (audioIO, error)) error {
	if v.boundary == nil {
		return errAudioStopped
	}
	return v.boundary.Swap(open)
}

func (v *voicePipeline) sendLoop(ctx context.Context) {
	defer v.done.Done()
	pcm := make([]int16, frameSamples)
	encoded := make([]byte, maxOpusFrameBytes)
	local := speechDetector{}
	frames := 0
	for {
		frame, err := v.audio.Read()
		if err != nil {
			// Device death mid-call is a fault; a stopped device during
			// teardown is not.
			if ctx.Err() == nil && v.onFault != nil {
				v.onFault()
			}
			return
		}
		// Mute and idle push-to-talk drop at the source: those frames are
		// never encoded or sent, and the level reports honest silence. A
		// closed voice-activation gate is different: the frame is still
		// measured for real, because the detector's speaking flag is what
		// opens the gate — observing zeros would lock it shut forever.
		if !v.captureOpen() {
			local.observe(0)
		} else {
			pcmFromBytes(normalizeFrame(frame), pcm)
			applyGain(pcm, clampGain(v.inputGain()))
			local.observe(frameLevel(pcm))
			if v.transmit(local.speaking) {
				n, err := v.encoder.encode(pcm, encoded)
				if err != nil {
					continue
				}
				if err := v.track.WriteSample(media.Sample{Data: encoded[:n], Duration: frameDuration}); err != nil {
					return
				}
			}
		}
		// Level facts leave at ~10 Hz; a quiet call still emits a heartbeat so
		// the UI's speaking indicator clears without waiting for words.
		frames++
		if frames%10 == 0 && v.onLevel != nil {
			v.onLevel(local.level, v.remoteLevel.level, local.speaking, v.remoteLevel.speaking)
		}
	}
}

func (v *voicePipeline) recvLoop(remote *webrtc.TrackRemote) {
	defer v.done.Done()
	builder := samplebuilder.New(100, &codecs.OpusPacket{}, audioSampleRate)
	pcm := make([]int16, decoderMaxSamples)
	for {
		packet, _, err := remote.ReadRTP()
		if err != nil {
			return // the track closes with the call
		}
		builder.Push(packet)
		for sample := builder.Pop(); sample != nil; sample = builder.Pop() {
			n, err := v.decoder.decode(sample.Data, pcm)
			if err != nil || n == 0 {
				continue
			}
			decoded := pcm[:n]
			applyGain(decoded, clampGain(v.outputGain()))
			if v.remoteLevel != nil {
				v.remoteLevel.observe(frameLevel(decoded))
			}
			if err := v.audio.Write(pcmToBytes(decoded)); err != nil {
				return // playback stopped during teardown
			}
		}
	}
}

// normalizeFrame forces the exact encoder frame size; device callbacks can
// briefly deliver different chunkings around sample-rate conversion.
func normalizeFrame(frame []byte) []byte {
	if len(frame) == frameBytes {
		return frame
	}
	normalized := make([]byte, frameBytes)
	copy(normalized, frame)
	return normalized
}

// close releases the codec handles once both pumps have joined.
func (v *voicePipeline) close() {
	v.encoder.Close()
	v.decoder.Close()
}

func pcmFromBytes(frame []byte, pcm []int16) {
	for i := range pcm {
		pcm[i] = int16(binary.LittleEndian.Uint16(frame[i*2:]))
	}
}

func pcmToBytes(pcm []int16) []byte {
	frame := make([]byte, len(pcm)*2)
	for i, sample := range pcm {
		binary.LittleEndian.PutUint16(frame[i*2:], uint16(sample))
	}
	return frame
}

// deviceAudio is the production boundary: real microphone and speaker devices
// opened through miniaudio. Callbacks run on miniaudio's own threads and hand
// frames over through bounded rings; a full ring drops the freshest frame so
// latency can never grow without bound.
type deviceAudio struct {
	context  *malgo.AllocatedContext
	capture  *malgo.Device
	playback *malgo.Device

	captured       chan []byte
	playbackFrames chan []byte
	stop           chan struct{}
	stopOnce       sync.Once

	mu      sync.Mutex
	stopped bool
}

var errAudioStopped = errors.New("audio device is closed")

// openDeviceAudio opens the machine's default microphone and speaker. Any
// failure here is the caller's refusal point: a voice call without devices
// must fail honestly before it ever publishes state. The selected-device
// variant lives in audiocapture.go and shares this whole boundary.
func openDeviceAudio() (audioIO, error) {
	return openDeviceAudioWith(nil, nil)
}

func (d *deviceAudio) Read() ([]byte, error) {
	if d.isStopped() {
		return nil, errAudioStopped
	}
	select {
	case frame := <-d.captured:
		return frame, nil
	case <-d.stop:
		return nil, errAudioStopped
	}
}

func (d *deviceAudio) Write(frame []byte) error {
	if d.isStopped() {
		return errAudioStopped
	}
	select {
	case d.playbackFrames <- frame:
		return nil
	default: // the speaker is behind: drop rather than accumulate latency
		return nil
	}
}

func (d *deviceAudio) Close() {
	d.mu.Lock()
	if d.stopped {
		d.mu.Unlock()
		return
	}
	d.stopped = true
	d.mu.Unlock()
	d.stopOnce.Do(func() { close(d.stop) })
	if d.capture != nil {
		_ = d.capture.Stop()
		d.capture.Uninit()
	}
	if d.playback != nil {
		_ = d.playback.Stop()
		d.playback.Uninit()
	}
	if d.context != nil {
		d.context.Free()
	}
}

func (d *deviceAudio) isStopped() bool {
	d.mu.Lock()
	defer d.mu.Unlock()
	return d.stopped
}

// silentAudio is the deterministic test boundary: it produces silence at the
// real-time cadence and discards playback. Production never selects it —
// main() installs the device factory explicitly — and an automated core can
// request it via HARBOR_MEDIA_AUDIO=silent so supervision tests stay
// hermetic on machines without audio hardware.
type silentAudio struct {
	ticker   *time.Ticker
	stop     chan struct{}
	stopOnce sync.Once
}

func openSilentAudio() (audioIO, error) {
	return &silentAudio{
		ticker: time.NewTicker(frameDuration),
		stop:   make(chan struct{}),
	}, nil
}

func (s *silentAudio) Read() ([]byte, error) {
	select {
	case <-s.ticker.C:
		return make([]byte, frameBytes), nil
	case <-s.stop:
		return nil, errAudioStopped
	}
}

func (s *silentAudio) Write([]byte) error {
	select {
	case <-s.stop:
		return errAudioStopped
	default:
		return nil
	}
}

func (s *silentAudio) Close() {
	s.stopOnce.Do(func() {
		close(s.stop)
		s.ticker.Stop()
	})
}
