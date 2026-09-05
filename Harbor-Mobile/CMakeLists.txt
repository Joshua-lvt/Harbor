# Harbor Mobile: Qt 6 / QML + in-process Rust core for Android.
#
# This is a SEPARATE configure from the desktop build: point CMake at a Qt
# for Android kit with the NDK toolchain file, one ABI at a time (the
# build-android.sh script drives both arm64-v8a and x86_64). The Rust core
# is built first as a cdylib for the same ABI and linked directly —
# Android cannot spawn it as a child process, so there is no supervisor.
cmake_minimum_required(VERSION 3.21)
# Single product version: same file the desktop build, the updater channel
# and CI releases follow. build-android.sh additionally syncs the Android
# versionCode/versionName from it (single source of truth).
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/../VERSION.txt" HARBOR_VERSION_RAW)
string(STRIP "${HARBOR_VERSION_RAW}" HARBOR_VERSION)
project(HarborMobile VERSION ${HARBOR_VERSION} LANGUAGES CXX)

option(HARBOR_SIGN_APK "Sign the Android package with an external keystore" OFF)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(NOT ANDROID)
    message(FATAL_ERROR "Configure against a Qt for Android kit with the NDK toolchain (see build-android.sh)")
endif()

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Quick Qml Gui)

# The Rust core cdylib for this ABI. Built by build-android.sh into
# Harbor-Mobile/target/<abi>/ with the matching NDK toolchain; a plain
# host build never reaches this file (see the guard above).
if(NOT DEFINED HARBOR_CORE_LIB)
    message(FATAL_ERROR "Set HARBOR_CORE_LIB to libharbor_core.so for ${ANDROID_ABI}")
endif()
if(NOT DEFINED HARBOR_MEDIA_ANDROID_BINARY OR NOT EXISTS "${HARBOR_MEDIA_ANDROID_BINARY}")
    message(FATAL_ERROR "Set HARBOR_MEDIA_ANDROID_BINARY to the ABI-matched harbor-media executable")
endif()
qt_standard_project_setup(REQUIRES 6.5)
qt_policy(SET QTP0002 NEW)
qt_policy(SET QTP0004 NEW)

qt_add_executable(harbor-mobile
    mobile_main.cpp
    native/HarborCoreAdapter.cpp
    native/HarborCoreAdapter.h
    native/HarborAndroid.cpp
    native/HarborAndroid.h
)

# Java adapters under android/src ship with the package sources; the
# manifest declares the activities and services.
set_target_properties(harbor-mobile PROPERTIES
    QT_ANDROID_PACKAGE_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/android"
    QT_ANDROID_PACKAGE_NAME org.harbor.mobile
    QT_ANDROID_MIN_SDK_VERSION 28
    QT_ANDROID_TARGET_SDK_VERSION 35
    QT_ANDROID_COMPILE_SDK_VERSION 36
)

if(HARBOR_SIGN_APK)
    # Qt's androiddeployqt signer consumes these values from the environment
    # at package time. Keep passwords out of CMakeCache.txt and out of target
    # properties; only the release build environment should contain them.
    foreach(required QT_ANDROID_KEYSTORE_PATH QT_ANDROID_KEYSTORE_ALIAS
            QT_ANDROID_KEYSTORE_STORE_PASS QT_ANDROID_KEYSTORE_KEY_PASS)
        if(NOT DEFINED ENV{${required}} OR "$ENV{${required}}" STREQUAL "")
            message(FATAL_ERROR "${required} environment variable is required when HARBOR_SIGN_APK=ON")
        endif()
    endforeach()
    # These are read by Qt6AndroidMacros.cmake while it creates the package
    # targets. APK and AAB use the same externally supplied keystore policy.
    set(QT_ANDROID_SIGN_APK TRUE)
    set(QT_ANDROID_SIGN_AAB TRUE)
endif()

set_source_files_properties(qml/HarborVisualEffects.qml PROPERTIES
    QT_QML_SINGLETON_TYPE TRUE
)

qt_add_qml_module(harbor-mobile
    URI HarborMobile
    VERSION 1.0
    QML_FILES
        qml/HarborVisualEffects.qml
        qml/MobileShell.qml
        qml/components/MobilePresenceBar.qml
        qml/components/MobilePairingSheet.qml
        qml/components/MobileButton.qml
        qml/components/MobileTheme.qml
        qml/components/MobileUpdateBlock.qml
        qml/views/MobileHomeView.qml
        qml/views/MobileChatView.qml
        qml/views/MobileCallView.qml
        qml/views/MobileTabView.qml
        qml/views/MobileSettingsView.qml
        qml/Host/HarborMobileHost.qml
    RESOURCES
        qml/qmldir
        qml/components/qmldir
        qml/views/qmldir
        qml/Host/qmldir
)

# The Go/Pion worker is an opaque private asset, not a shared library. It is
# extracted by HarborAndroid into app-private storage before the Rust core is
# told about it. Keeping it in the Qt resource system avoids relying on
# androiddeployqt's ABI-specific native-library packaging rules for an ELF
# executable.
set_source_files_properties("${HARBOR_MEDIA_ANDROID_BINARY}" PROPERTIES
    QT_RESOURCE_ALIAS "harbor-media")
qt_add_resources(harbor-mobile harbor_media_asset
    PREFIX "/harbor-media"
    FILES "${HARBOR_MEDIA_ANDROID_BINARY}")

target_include_directories(harbor-mobile PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/native")
# Link by -l name so the APK records a bare SONAME instead of a host path.
get_filename_component(HARBOR_CORE_DIR "${HARBOR_CORE_LIB}" DIRECTORY)
target_link_directories(harbor-mobile PRIVATE "${HARBOR_CORE_DIR}")
# Force packaging of the cdylib next to the Qt libs.
set_target_properties(harbor-mobile PROPERTIES
    QT_ANDROID_EXTRA_LIBS "${HARBOR_CORE_LIB}"
)
target_link_libraries(harbor-mobile PRIVATE
    Qt6::Core Qt6::Quick Qt6::Qml Qt6::Gui
    harbor_core
)
if(TARGET Qt6::DBus)
    target_link_libraries(harbor-mobile PRIVATE Qt6::DBus)
endif()

# No install step: the APK androiddeployqt produces is the artifact.
