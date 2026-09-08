// Minimal libopus bindings for the voice pipeline.
//
// The media worker needs exactly two operations — encode one 10 ms frame and
// decode one packet — so it binds libopus directly instead of carrying a
// binding package that drags in unrelated system dependencies (streaming
// readers, opusfile). The libopus ABI is stable and these calls map one-to-one
// onto its documented API; anything richer belongs in a dedicated codec layer.
package main

/*
#cgo !android pkg-config: opus
#include <opus.h>
*/
import "C"

import (
	"fmt"
	"unsafe"
)

// opusApplicationVoIP tunes the encoder for speech: lower latency and
// forward error correction favor a call over music fidelity.
const opusApplicationVoIP = C.OPUS_APPLICATION_VOIP

type opusEncoder struct {
	handle *C.OpusEncoder
}

func newOpusEncoder(sampleRate, channels, application int) (*opusEncoder, error) {
	var errno C.int
	handle := C.opus_encoder_create(C.opus_int32(sampleRate), C.int(channels),
		C.opus_int32(application), &errno)
	if errno != C.OPUS_OK {
		return nil, fmt.Errorf("opus_encoder_create: %s", C.GoString(C.opus_strerror(errno)))
	}
	return &opusEncoder{handle: handle}, nil
}

// encode turns one exact frame of PCM16 into an Opus packet, returning the
// packet's byte length.
func (e *opusEncoder) encode(pcm []int16, data []byte) (int, error) {
	n := C.opus_encode(e.handle,
		(*C.opus_int16)(unsafe.Pointer(&pcm[0])), C.int(len(pcm)),
		(*C.uchar)(unsafe.Pointer(&data[0])), C.opus_int32(len(data)))
	if n < 0 {
		return 0, fmt.Errorf("opus_encode: %s", C.GoString(C.opus_strerror(n)))
	}
	return int(n), nil
}

func (e *opusEncoder) Close() {
	C.opus_encoder_destroy(e.handle)
}

type opusDecoder struct {
	handle *C.OpusDecoder
}

func newOpusDecoder(sampleRate, channels int) (*opusDecoder, error) {
	var errno C.int
	handle := C.opus_decoder_create(C.opus_int32(sampleRate), C.int(channels), &errno)
	if errno != C.OPUS_OK {
		return nil, fmt.Errorf("opus_decoder_create: %s", C.GoString(C.opus_strerror(errno)))
	}
	return &opusDecoder{handle: handle}, nil
}

// decode turns one Opus packet into at most len(pcm) PCM16 samples per
// channel, returning how many were written.
func (d *opusDecoder) decode(data []byte, pcm []int16) (int, error) {
	if len(data) == 0 || len(pcm) == 0 {
		return 0, fmt.Errorf("opus_decode: empty input")
	}
	n := C.opus_decode(d.handle,
		(*C.uchar)(unsafe.Pointer(&data[0])), C.opus_int32(len(data)),
		(*C.opus_int16)(unsafe.Pointer(&pcm[0])), C.int(len(pcm)), 0)
	if n < 0 {
		return 0, fmt.Errorf("opus_decode: %s", C.GoString(C.opus_strerror(n)))
	}
	return int(n), nil
}

func (d *opusDecoder) Close() {
	C.opus_decoder_destroy(d.handle)
}
