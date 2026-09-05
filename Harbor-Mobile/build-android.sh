#!/usr/bin/env bash
# Harbor Mobile Android build: Rust cdylib + Qt APK, one ABI at a time.
#
#   ./build-android.sh arm64   # device (arm64-v8a)
#   ./build-android.sh x64     # emulator (x86_64)
#
# Prerequisites: Qt for Android (6.11.2 locally; HARBOR_QT_VERSION overrides),
# NDK 28, SDK 35, JDK 17, Rust with
# the aarch64/x86_64-linux-android targets. Everything is validated up
# front with honest errors; nothing proceeds on a broken toolchain.
set -euo pipefail

ABI="${1:-arm64}"
BUILD_TYPE="${HARBOR_BUILD_TYPE:-Debug}"
case "$BUILD_TYPE" in
    Debug|Release) ;;
    *) echo "HARBOR_BUILD_TYPE must be Debug or Release" >&2; exit 1;;
esac
case "$ABI" in
    arm64) TRIPLE=aarch64-linux-android; QT_ABI=android_arm64_v8a; GRADLE_ABI="arm64-v8a";;
    x64)   TRIPLE=x86_64-linux-android;  QT_ABI=android_x86_64;  GRADLE_ABI="x86_64";;
    *) echo "usage: $0 [arm64|x64]" >&2; exit 1;;
esac
[ "$BUILD_TYPE" != Release ] || [ "$ABI" = arm64 ] || {
    echo "Release packages are arm64-only; use x64 for emulator/debug" >&2
    exit 1
}

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MOBILE="$ROOT/Harbor-Mobile"
# Single product version (VERSION file): release tags, updater channel and
# the Android manifest follow it. versionCode is MAJOR*10000+MINOR*100+PATCH
# so every release installs over the previous one.
APP_VERSION="$(tr -d ' \t\r\n' < "$ROOT/VERSION.txt")"
APP_CODE="$(echo "$APP_VERSION" | awk -F. '{ print ($1*10000)+($2*100)+$3 }')"
sed -i -E "s/android:versionCode=\"[0-9]+\"/android:versionCode=\"$APP_CODE\"/" "$MOBILE/android/AndroidManifest.xml"
sed -i -E "s/android:versionName=\"[^\"]+\"/android:versionName=\"$APP_VERSION\"/" "$MOBILE/android/AndroidManifest.xml"
echo "==> version $APP_VERSION (code $APP_CODE)"

# Vendored codec sources travel as tarballs; extract once per checkout.
if [ ! -f "$ROOT/media/win-deps/src/opus-1.5.2/CMakeLists.txt" ]; then
    echo "==> unpack vendored opus"
    mkdir -p "$ROOT/media/win-deps/src/opus-1.5.2"
    tar -xzf "$ROOT/media/win-deps/src/opus-1.5.2.tar.gz" -C "$ROOT/media/win-deps/src/opus-1.5.2" --strip-components=1
fi
QT_VERSION="${HARBOR_QT_VERSION:-6.11.2}"
QT="$HOME/Qt/$QT_VERSION/$QT_ABI"
# Host tools may come from a different (older, fully mirrored) kit than the
# target; moc/rcc output stays compatible for this codebase.
QT_HOST_VERSION="${HARBOR_QT_HOST_VERSION:-$QT_VERSION}"
NDK="$HOME/Android/Sdk/ndk/28.2.13676358"
SDK="$HOME/Android/Sdk"
TOOLCHAIN="$NDK/build/cmake/android.toolchain.cmake"

for need in "$QT/bin/qt-cmake" "$TOOLCHAIN" "$SDK/platform-tools/adb"; do
    [ -e "$need" ] || { echo "missing: $need" >&2; exit 1; }
done
command -v cargo >/dev/null || { echo "missing: cargo" >&2; exit 1; }

export ANDROID_HOME="$SDK" ANDROID_SDK_ROOT="$SDK" ANDROID_NDK_ROOT="$NDK"
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-17-openjdk-amd64}"
[ -d "$JAVA_HOME" ] || { echo "missing JDK 17 at $JAVA_HOME" >&2; exit 1; }

BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
case "$TRIPLE" in
    aarch64-linux-android) P=aarch64;;
    x86_64-linux-android) P=x86_64;;
esac
export "CC_${P}_linux_android=$BIN/${P}-linux-android35-clang"
export "CXX_${P}_linux_android=$BIN/${P}-linux-android35-clang++"
export "AR_${P}_linux_android=$BIN/llvm-ar"
LINKER_VAR="CARGO_TARGET_$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')_LINKER"
export "$LINKER_VAR=$BIN/${P}-linux-android35-clang"

# 1. Rust core as a cdylib for this ABI (debug: fast, debuggable on device).
echo "==> cargo: harbor-core cdylib for $TRIPLE"
if [ "$BUILD_TYPE" = Release ]; then
    CARGO_PROFILE=(--release)
    CORE_PROFILE=release
else
    CARGO_PROFILE=()
    CORE_PROFILE=debug
fi
(
    cd "$ROOT"
    CARGO_TARGET_DIR="$MOBILE/target/$ABI" \
        cargo build --locked --lib -p harbor-core --target "$TRIPLE" "${CARGO_PROFILE[@]}"
)
CORE_LIB="$MOBILE/target/$ABI/$TRIPLE/$CORE_PROFILE/libharbor_core.so"
[ -f "$CORE_LIB" ] || { echo "cdylib missing: $CORE_LIB" >&2; exit 1; }

# 2. Build the private Pion worker for the exact Android ABI. This is an ELF
# executable placed in a Qt resource and extracted into app-private storage
# at runtime; it is never looked up beside libharbor_core.so.
MEDIA_BINARY="$MOBILE/target/$ABI/harbor-media"
OPUS_BUILD="$MOBILE/target/$ABI/opus"
echo "==> cmake: libopus for $TRIPLE"
cmake -S "$ROOT/media/win-deps/src/opus-1.5.2" -B "$OPUS_BUILD" \
    -DCMAKE_TOOLCHAIN_FILE="$TOOLCHAIN" \
    -DANDROID_ABI="$GRADLE_ABI" -DANDROID_PLATFORM=android-35 \
    -DCMAKE_BUILD_TYPE=Release -DOPUS_BUILD_SHARED_LIBRARY=OFF \
    -DOPUS_BUILD_PROGRAMS=OFF -DOPUS_BUILD_TESTING=OFF \
    -DOPUS_INSTALL_PKG_CONFIG_MODULE=OFF >/dev/null
cmake --build "$OPUS_BUILD" --target opus --parallel "$(nproc)" >/dev/null
echo "==> go: harbor-media for $TRIPLE"
(
    cd "$ROOT/media"
    CGO_ENABLED=1 GOOS=android GOARCH="$([ "$ABI" = arm64 ] && echo arm64 || echo amd64)" \
        CC="$BIN/${P}-linux-android35-clang" \
        CXX="$BIN/${P}-linux-android35-clang++" \
        CGO_CFLAGS="-I$ROOT/media/win-deps/src/opus-1.5.2/include" \
        CGO_LDFLAGS="-L$OPUS_BUILD -lopus -lm -landroid -lOpenSLES" \
        go build -trimpath -o "$MEDIA_BINARY" ./cmd/harbor-media
)
[ -x "$MEDIA_BINARY" ] || { echo "Android media worker missing: $MEDIA_BINARY" >&2; exit 1; }

# 3. Qt configure + build (androiddeployqt produces the APK).
BUILD="$MOBILE/build-$ABI"
# Toolchain changes (NDK/Qt/ABI) never survive a stale cache: a fresh
# configure is cheap next to the compile that follows.
rm -rf "$BUILD"
echo "==> cmake: $BUILD"
"$QT/bin/qt-cmake" -S "$MOBILE" -B "$BUILD" \
    -DANDROID_ABI="$GRADLE_ABI" \
    -DANDROID_PLATFORM=android-35 \
    -DQT_HOST_PATH="$HOME/Qt/$QT_HOST_VERSION/gcc_64" \
    -DQt6_DIR="$QT/lib/cmake/Qt6" \
    -DCMAKE_PREFIX_PATH="$QT" \
    -DCMAKE_BUILD_TYPE="$BUILD_TYPE" \
    -DHARBOR_CORE_LIB="$CORE_LIB" \
    -DHARBOR_MEDIA_ANDROID_BINARY="$MEDIA_BINARY"
if [ "$BUILD_TYPE" = Release ]; then
    : "${HARBOR_ANDROID_KEYSTORE:?set HARBOR_ANDROID_KEYSTORE for a Release build}"
    : "${HARBOR_ANDROID_KEY_ALIAS:?set HARBOR_ANDROID_KEY_ALIAS for a Release build}"
    : "${HARBOR_ANDROID_STORE_PASSWORD:?set HARBOR_ANDROID_STORE_PASSWORD for a Release build}"
    : "${HARBOR_ANDROID_KEY_PASSWORD:?set HARBOR_ANDROID_KEY_PASSWORD for a Release build}"
    # Qt's androiddeployqt signer reads these names from its environment. Do
    # not pass credentials as -D arguments: CMake would persist passwords in
    # CMakeCache.txt. They remain available only to this release build.
    export QT_ANDROID_KEYSTORE_PATH="$HARBOR_ANDROID_KEYSTORE"
    export QT_ANDROID_KEYSTORE_ALIAS="$HARBOR_ANDROID_KEY_ALIAS"
    export QT_ANDROID_KEYSTORE_STORE_PASS="$HARBOR_ANDROID_STORE_PASSWORD"
    export QT_ANDROID_KEYSTORE_KEY_PASS="$HARBOR_ANDROID_KEY_PASSWORD"
    # Reconfigure with signing enabled. The Qt deploy macros consume the
    # exported values when they create the APK/AAB package targets.
    "$QT/bin/qt-cmake" -S "$MOBILE" -B "$BUILD" \
        -DANDROID_ABI="$GRADLE_ABI" -DANDROID_PLATFORM=android-35 \
        -DQT_HOST_PATH="$HOME/Qt/$QT_HOST_VERSION/gcc_64" -DQt6_DIR="$QT/lib/cmake/Qt6" \
        -DCMAKE_PREFIX_PATH="$QT" -DCMAKE_BUILD_TYPE=Release \
        -DHARBOR_CORE_LIB="$CORE_LIB" -DHARBOR_MEDIA_ANDROID_BINARY="$MEDIA_BINARY" \
        -DHARBOR_SIGN_APK=ON
fi
echo "==> build APK"
cmake --build "$BUILD" --parallel "$(nproc)"

if [ "$BUILD_TYPE" = Release ]; then
    echo "==> build AAB"
    cmake --build "$BUILD" --target harbor-mobile_make_aab
fi

if [ "$BUILD_TYPE" = Release ]; then
    APK="$(find "$BUILD" -path '*/outputs/apk/release/*-signed.apk' -type f | head -1)"
else
    APK="$(find "$BUILD" -path '*/outputs/apk/debug/*.apk' -type f | head -1)"
fi
if [ -n "$APK" ]; then
    echo "APK: $APK"
else
    echo "no APK produced (see $BUILD)" >&2
    exit 1
fi
if [ "$BUILD_TYPE" = Release ]; then
    AAB="$(find "$BUILD" -path '*/outputs/bundle/release/*.aab' -type f | head -1)"
    if [ -n "$AAB" ]; then
        echo "AAB: $AAB"
    else
        echo "no AAB produced (see $BUILD)" >&2
        exit 1
    fi
fi
