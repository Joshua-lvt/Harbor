if(NOT DEFINED ROOT_DIR)
    message(FATAL_ERROR "ROOT_DIR is required")
endif()

file(GLOB_RECURSE HARBOR_SOURCES
    "${ROOT_DIR}/*.cpp"
    "${ROOT_DIR}/*.h"
    "${ROOT_DIR}/qml/*.qml"
    "${ROOT_DIR}/qml/*.js"
)

file(GLOB_RECURSE HARBOR_MOBILE_QML
    "${ROOT_DIR}/Harbor-Mobile/qml/*.qml"
)

# Vendored third-party code (native/third-party) is not written under these
# contracts; only first-party sources are scanned. (The comment-stripping
# regexes below also cannot survive a multi-megabyte vendored header.)
set(HARBOR_FIRST_PARTY_SOURCES)
foreach(source_file IN LISTS HARBOR_SOURCES)
    if(NOT source_file MATCHES "/third-party/")
        list(APPEND HARBOR_FIRST_PARTY_SOURCES "${source_file}")
    endif()
endforeach()

# Android/Qt Quick regression: Material layer effects and unsupported
# advanced effects produced invalid scene-graph geometry on Android GPUs.
# Mobile draws explicit surfaces instead; advanced rendering may only be
# reintroduced together with a new platform guard and reproducible test.
set(HARBOR_MOBILE_FORBIDDEN_RENDER_PATTERNS
    "ShaderEffect"
    "MultiEffect"
    "FastBlur"
    "GaussianBlur"
    "DropShadow"
    "OpacityMask"
    "layer\\.effect"
    "layer\\.enabled"
    "\\bCanvas\\b"
    "\\bShape\\b"
    "ShapePath"
    "\\bParticle"
    "Particles\\b"
    "\\bImage\\b"
    "BorderImage"
    "AnimatedImage"
    "ShaderEffectSource"
    "\\bGradient\\b"
    "textureSize"
    "sourceSize"
    "\\btexture\\b"
    "\\bnoise\\b"
    "scanline"
    "glitch"
    "distortion"
    "displacement"
    "\\bblur\\b"
    "\\bglass\\b"
    "\\bglow\\b"
    "ToolBar"
    "ToolButton"
    "width:[ \t]*(1024|1280|1440|1920|2560)\\b"
    "height:[ \t]*(640|720|800|1024|1080)\\b"
)
foreach(source_file IN LISTS HARBOR_MOBILE_QML)
    file(READ "${source_file}" source_text)
    string(REGEX REPLACE "//[^\n]*" "" source_text "${source_text}")
    string(REGEX REPLACE "/\\*([^*]|\\*+[^*/])*\\*+/" "" source_text "${source_text}")
    foreach(pattern IN LISTS HARBOR_MOBILE_FORBIDDEN_RENDER_PATTERNS)
        if(source_text MATCHES "${pattern}")
            file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
            message(FATAL_ERROR
                "Unsafe mobile rendering pattern in ${relative_path}: ${pattern}")
        endif()
    endforeach()
endforeach()

file(READ "${ROOT_DIR}/Harbor-Mobile/mobile_main.cpp" source_text)
if(NOT source_text MATCHES "HARBOR_DISABLE_EFFECTS")
    message(FATAL_ERROR
        "Harbor Mobile lost the HARBOR_DISABLE_EFFECTS rendering diagnostic")
endif()
set(HARBOR_SOURCES "${HARBOR_FIRST_PARTY_SOURCES}")

set(FORBIDDEN_PATTERNS
    "Q(Settings|SqlDatabase)"
    # Clipboard stays out of QML and out of everything except the typed
    # facade: pairing-code copy goes through HarborFacade.copyToClipboard,
    # facade: pairing-code copy goes through HarborFacade.copyToClipboard,
    # and no other layer may touch the system clipboard.
    "Qt\.application\.clipboard"
    "XMLHttpRequest"
    # Assigning HarborSettingRow's default alias by name spins the layout
    # forever at 100% CPU; trailing controls must be unqualified children.
    "\n[ \t]+trailing[ \t]*:"
)

# The system tray is real now, but it stays confined to the typed native
# adapter (native/HarborTray.*). QML and every other layer must go through
# it: QSystemTrayIcon outside that adapter is still a contract violation.
foreach(source_file IN LISTS HARBOR_SOURCES)
    if(source_file MATCHES "/native/HarborTray\\.(h|cpp)$")
        continue()
    endif()
    file(READ "${source_file}" source_text)
    string(REGEX REPLACE "//[^\n]*" "" source_text "${source_text}")
    string(REGEX REPLACE "/\\*([^*]|\\*+[^*/])*\\*+/" "" source_text "${source_text}")
    if(source_text MATCHES "SystemTrayIcon")
        file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
        message(FATAL_ERROR
            "QSystemTrayIcon is allowed only in native/HarborTray.*; found in ${relative_path}")
    endif()
endforeach()

# Presence evidence (session/login daemons, idle monitors, input stacks,
# MPRIS) is confined to the native presence detector. The core and the QML
# layer only ever see the committed ONLINE/AWAY/OFFLINE aggregate, so any
# other file reaching for these APIs breaks the privacy boundary.
#
# Generic D-Bus binding gets one narrow exception: the notification output
# adapter (native/HarborNotifications.*) posts to the desktop bus and never
# observes presence — evidence daemons remain presence-source-only.
set(PRESENCE_EVIDENCE_PATTERNS
    "MPRIS"
    "GetLastInputInfo"
    "WTSEnumerateSessions"
    "IdleMonitor"
    "ScreenSaver"
    "login1"
)
foreach(source_file IN LISTS HARBOR_SOURCES)
    if(source_file MATCHES "/native/HarborPresenceSource\\.(h|cpp)$")
        continue()
    endif()
    file(READ "${source_file}" source_text)
    string(REGEX REPLACE "//[^\n]*" "" source_text "${source_text}")
    string(REGEX REPLACE "/\\*([^*]|\\*+[^*/])*\\*+/" "" source_text "${source_text}")
    foreach(pattern IN LISTS PRESENCE_EVIDENCE_PATTERNS)
        if(source_text MATCHES "${pattern}")
            file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
            message(FATAL_ERROR
                "Presence evidence APIs are allowed only in native/HarborPresenceSource.*; found ${pattern} in ${relative_path}")
        endif()
    endforeach()
    if(NOT source_file MATCHES "/native/HarborNotifications\\.(h|cpp)$"
            AND source_text MATCHES "QDBus")
        file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
        message(FATAL_ERROR
            "D-Bus access is allowed only in the native presence and notification adapters; found in ${relative_path}")
    endif()
endforeach()

# Network access is confined to the mandatory updater adapter
# (native/HarborUpdater.*), which is the only first-party code allowed to
# touch the release channel: every other layer stays offline by contract,
# and QML never performs requests (no Qt.Network imports, no XHR).
set(NETWORK_PATTERNS
    "import[ \t]+Qt(Network|WebSockets|Multimedia)"
    "#include[ \t]*[<\"]Q(Network|WebSocket|Media|Audio|Camera|Bluetooth|SerialPort|Settings|Sql)"
    "Q(Settings|SqlDatabase)"
    "Qt\.application\.clipboard"
    "XMLHttpRequest"
)
foreach(source_file IN LISTS HARBOR_SOURCES)
    if(source_file MATCHES "/native/HarborUpdater\\.(h|cpp)$")
        continue()
    endif()
    file(READ "${source_file}" source_text)
    string(REGEX REPLACE "//[^\n]*" "" source_text "${source_text}")
    string(REGEX REPLACE "/\\*([^*]|\\*+[^*/])*\\*+/" "" source_text "${source_text}")
    foreach(pattern IN LISTS NETWORK_PATTERNS)
        if(source_text MATCHES "${pattern}")
            file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
            message(FATAL_ERROR "Network integration is allowed only in native/HarborUpdater.*; found in ${relative_path}: ${pattern}")
        endif()
    endforeach()
endforeach()

foreach(source_file IN LISTS HARBOR_SOURCES)
    file(READ "${source_file}" source_text)
    string(REGEX REPLACE "//[^\n]*" "" source_text "${source_text}")
    string(REGEX REPLACE "/\\*([^*]|\\*+[^*/])*\\*+/" "" source_text "${source_text}")
    foreach(pattern IN LISTS FORBIDDEN_PATTERNS)
        if(source_text MATCHES "${pattern}")
            file(RELATIVE_PATH relative_path "${ROOT_DIR}" "${source_file}")
            message(FATAL_ERROR "Forbidden prototype integration in ${relative_path}: ${pattern}")
        endif()
    endforeach()
endforeach()

message(STATUS "Harbor source contracts passed")
