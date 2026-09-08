//go:build (linux && !android) || (windows && harborvpx)

// libvpx VP8 encoding for the screen-share pipeline.
//
// Linux builds it by default (PKG_CONFIG_PATH needs vpx.pc). Windows
// builds it only with `-tags harborvpx` plus a pkg-config-visible libvpx
// (e.g. vcpkg with integration); without the tag vpx_stub.go keeps share
// start on the honest capture_unavailable refusal while voice calls keep
// working.
package main

/*
#cgo linux pkg-config: vpx
#cgo windows pkg-config: vpx
#include <stdlib.h>
#include <string.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vp8cx.h>

static vpx_codec_err_t harborVpxEncoderInit(vpx_codec_ctx_t *ctx, vpx_codec_iface_t *iface,
                                            vpx_codec_enc_cfg_t *cfg) {
    return vpx_codec_enc_init_ver(ctx, iface, cfg, 0, VPX_ENCODER_ABI_VERSION);
}
static int harborPktIsFrame(const vpx_codec_cx_pkt_t *pkt) {
    return pkt->kind == VPX_CODEC_CX_FRAME_PKT;
}
static const unsigned char *harborPktData(const vpx_codec_cx_pkt_t *pkt) {
    return pkt->data.frame.buf;
}
static size_t harborPktSize(const vpx_codec_cx_pkt_t *pkt) {
    return (size_t)pkt->data.frame.sz;
}
static vpx_image_t *harborAllocImage(void) {
    return (vpx_image_t *)malloc(sizeof(vpx_image_t));
}
static void harborFreeBuffer(void *p) {
    free(p);
}
static void harborCopyFrame(unsigned char *dst, const unsigned char *src, size_t n) {
    memcpy(dst, src, n);
}
static unsigned char *harborAllocFrame(int width, int height) {
    return (unsigned char *)malloc((size_t)width * (size_t)height * 3 / 2);
}
*/
import "C"

import (
	"errors"
	"fmt"
	"unsafe"
)

// errVPXUnavailable never carries a value on platforms with the real
// encoder; it exists so tests can reference the stub's skip contract on
// every platform combination.
var errVPXUnavailable = errors.New("video encoder is unavailable on this platform")

// ---- libvpx VP8 ----

// vpxCodec is an opaque initialized encoder context.
type vpxCodec = *C.vpx_codec_ctx_t

func vpxEncoderInit(width, height, fps, bitrateKb int) (vpxCodec, error) {
	var config C.vpx_codec_enc_cfg_t
	if code := C.vpx_codec_enc_config_default(C.vpx_codec_vp8_cx(), &config, 0); code != 0 {
		return nil, fmt.Errorf("vpx default config: %s", vpxErrorText(code))
	}
	config.g_w = C.uint(width)
	config.g_h = C.uint(height)
	config.g_timebase.num = 1
	config.g_timebase.den = C.int(fps)
	config.rc_target_bitrate = C.uint(bitrateKb)
	config.rc_end_usage = C.VPX_CBR
	config.g_lag_in_frames = 0
	config.kf_mode = C.VPX_KF_AUTO

	handle := new(C.vpx_codec_ctx_t)
	if code := C.harborVpxEncoderInit(handle, C.vpx_codec_vp8_cx(), &config); code != 0 {
		return nil, fmt.Errorf("vpx encoder init: %s", vpxErrorText(code))
	}
	return handle, nil
}

// vpxEncoder is a minimal libvpx VP8 encoder for fixed-size realtime video.
// The image and its plane buffer live in C memory: vpx_img_wrap aims the
// image's planes into that buffer, and memory crossing into libvpx must never
// carry Go pointers — the race detector's cgo checks enforce it.
type vpxEncoder struct {
	width  int
	height int
	codec  vpxCodec
	image  *C.vpx_image_t
	frame  unsafe.Pointer
}

func newVPXEncoder(width, height int) (*vpxEncoder, error) {
	if width <= 0 || height <= 0 || width%2 != 0 || height%2 != 0 {
		return nil, fmt.Errorf("capture size must be even and positive")
	}
	codec, err := vpxEncoderInit(width, height, shareFPS, shareBitrateKb)
	if err != nil {
		return nil, err
	}
	image := C.harborAllocImage()
	if image == nil {
		vpxEncoderDestroy(codec)
		return nil, fmt.Errorf("vpx image allocation failed")
	}
	frame := unsafe.Pointer(C.harborAllocFrame(C.int(width), C.int(height)))
	if frame == nil {
		C.harborFreeBuffer(unsafe.Pointer(image))
		vpxEncoderDestroy(codec)
		return nil, fmt.Errorf("vpx frame buffer allocation failed")
	}
	if C.vpx_img_wrap(image, C.VPX_IMG_FMT_I420, C.uint(width), C.uint(height), 1, (*C.uchar)(frame)) == nil {
		C.harborFreeBuffer(frame)
		C.harborFreeBuffer(unsafe.Pointer(image))
		vpxEncoderDestroy(codec)
		return nil, fmt.Errorf("vpx image wrap failed")
	}
	return &vpxEncoder{width: width, height: height, codec: codec, image: image, frame: frame}, nil
}

// encode converts one RGBA frame and produces one VP8 frame; keyframes are
// forced on demand so a recovering receiver can rejoin mid-share.
func (e *vpxEncoder) encode(rgba []byte, keyframe bool) ([]byte, error) {
	if len(rgba) != e.width*e.height*4 {
		return nil, fmt.Errorf("frame does not match the negotiated capture size")
	}
	i420 := rgbaToI420(rgba, e.width, e.height)
	C.harborCopyFrame((*C.uchar)(e.frame), (*C.uchar)(unsafe.Pointer(&i420[0])), C.size_t(len(i420)))
	return vpxEncodeFrame(e.codec, e.image, keyframe)
}

func (e *vpxEncoder) Close() {
	vpxEncoderDestroy(e.codec)
	C.harborFreeBuffer(unsafe.Pointer(e.image))
	C.harborFreeBuffer(e.frame)
}

func vpxEncodeFrame(handle vpxCodec, image *C.vpx_image_t, keyframe bool) ([]byte, error) {
	var flags C.vpx_enc_frame_flags_t
	if keyframe {
		flags = C.VPX_EFLAG_FORCE_KF
	}
	if code := C.vpx_codec_encode(handle, image, C.vpx_codec_pts_t(0), 1, flags, C.VPX_DL_REALTIME); code != 0 {
		return nil, fmt.Errorf("vpx encode: %s", vpxErrorText(code))
	}
	var iterator C.vpx_codec_iter_t
	for {
		packet := C.vpx_codec_get_cx_data(handle, &iterator)
		if packet == nil {
			return nil, fmt.Errorf("vpx produced no frame")
		}
		// The packet payload lives inside a C union, which cgo does not
		// project; the shims read the one variant a compressed frame has.
		if C.harborPktIsFrame(packet) != 0 && C.harborPktSize(packet) > 0 {
			data := unsafe.Slice((*byte)(unsafe.Pointer(C.harborPktData(packet))), int(C.harborPktSize(packet)))
			return append([]byte(nil), data...), nil
		}
	}
}

func vpxEncoderDestroy(handle vpxCodec) {
	_ = C.vpx_codec_destroy(handle)
}

func vpxErrorText(code C.vpx_codec_err_t) string {
	return C.GoString(C.vpx_codec_err_to_string(code))
}
