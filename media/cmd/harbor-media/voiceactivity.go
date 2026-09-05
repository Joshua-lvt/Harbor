// Real speaking detection over audio Harbor already captures.
//
// No second audio stream and no extra device exist for this: the voice
// pipeline computes a normalized level from every PCM frame it was going to
// encode (and every frame it decodes for playback), smooths it, and holds a
// speaking flag with a short hangover so the UI does not flicker between
// words. All the math is pure and unit-tested; the pipeline only feeds it.
package main

import (
	"math"
)

const (
	// rmsFullScale maps one full-scale sine's RMS to level 1.0.
	rmsFullScale = 32768.0
	// A level at or above this threshold counts as speech; roughly -27 dBFS,
	// well above room noise but far below normal voice.
	speakingThreshold = 0.045
	// A frame's level decays multiplicatively between observations, giving a
	// fast attack and a release of a few frames.
	levelDecay = 0.6
	// Speaking holds for this many quiet frames (~350 ms) before dropping.
	speakingHangoverFrames = 35
)

// frameLevel converts one PCM16 frame into a normalized 0..1 loudness.
func frameLevel(pcm []int16) float64 {
	if len(pcm) == 0 {
		return 0
	}
	var sum float64
	for _, sample := range pcm {
		value := float64(sample) / rmsFullScale
		sum += value * value
	}
	return math.Sqrt(sum / float64(len(pcm)))
}

// smoothLevel folds one raw frame level into the running display level: the
// result never rises slower than the raw peak and never falls instantly.
func smoothLevel(previous, raw float64) float64 {
	if raw > previous {
		return raw
	}
	return previous * levelDecay
}

// speechDetector tracks one audio direction's display level and speaking
// flag. The zero value is a valid, silent detector.
type speechDetector struct {
	level       float64
	speaking    bool
	quietStreak int
}

// observe folds one raw frame level; muted callers observe 0 so a muted
// microphone reports honest silence immediately.
func (d *speechDetector) observe(raw float64) {
	d.level = smoothLevel(d.level, raw)
	if d.level >= speakingThreshold {
		d.speaking = true
		d.quietStreak = 0
		return
	}
	if d.quietStreak < speakingHangoverFrames {
		d.quietStreak++
		if d.quietStreak >= speakingHangoverFrames {
			d.speaking = false
		}
	}
}

// applyGain scales one PCM16 buffer in place by a 0..2 linear gain with
// clipping. Volume lives on Harbor's own stream — the system mixer of any
// other application is never touched.
func applyGain(pcm []int16, gain float64) {
	if gain == 1 {
		return
	}
	for i, sample := range pcm {
		scaled := float64(sample) * gain
		if scaled > 32767 {
			scaled = 32767
		} else if scaled < -32768 {
			scaled = -32768
		}
		pcm[i] = int16(scaled)
	}
}

// clampGain confines a configured volume to the supported 0..1 range.
func clampGain(volume float64) float64 {
	if volume < 0 {
		return 0
	}
	if volume > 1 {
		return 1
	}
	return volume
}
