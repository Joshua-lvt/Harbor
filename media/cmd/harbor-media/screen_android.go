//go:build android

package main

import "errors"

// Android screen capture requires the MediaProjection permission flow owned
// by the Qt host. Until that explicit flow exists, refuse screen sharing
// rather than returning a black or fabricated frame. Voice calls remain
// independent of this boundary.
func openPlatformScreenCapture() (screenCapture, error) {
	return nil, errors.New("screen sharing is not supported on Android yet")
}
