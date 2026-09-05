cmake_minimum_required(VERSION 3.21)
# Single product version for every artifact (desktop, mobile, releases).
# Bump VERSION; CMake, the updater channel, and CI all follow it.
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/VERSION" HARBOR_VERSION_RAW)
string(STRIP "${HARBOR_VERSION_RAW}" HARBOR_VERSION)
project(Harbor VERSION ${HARBOR_VERSION} LANGUAGES CXX)
add_compile_definitions(HARBOR_VERSION_STRING="${HARBOR_VERSION}")

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Core Quick Widgets Network)
# Desktop notifications ride the session bus on Linux; the build stays
# honest (adapter reports unavailable) where the module is absent.
find_package(Qt6 6.5 QUIET COMPONENTS DBus)
find_program(CARGO_EXECUTABLE cargo REQUIRED)
find_program(GO_EXECUTABLE go REQUIRED)
# Windows screen sharing needs a native encoder: libvpx visible to
# pkg-config (e.g. vcpkg with integration) plus this flag, which passes
# -tags harborvpx to the media worker (GDI capture + libvpx VP8). Off by
# default so a plain Windows build needs no extra libraries; share start
# then refuses honestly while voice calls keep working.
option(HARBOR_WINDOWS_VPX "Build the Windows media worker with screen sharing (needs libvpx)" OFF)
include(CTest)

qt_standard_project_setup(REQUIRES 6.5)
qt_policy(SET QTP0004 NEW)

# Cargo recompiles every Rust module, not just lib.rs/main.rs. Keep CMake's
# custom-command dependencies equally complete so an edit cannot leave a stale
# core binary beside the QML application.
file(GLOB_RECURSE HARBOR_CORE_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_SOURCE_DIR}/Cargo.toml"
    "${CMAKE_SOURCE_DIR}/Cargo.lock"
    "${CMAKE_SOURCE_DIR}/protocol/Cargo.toml"
    "${CMAKE_SOURCE_DIR}/protocol/src/*.rs"
    "${CMAKE_SOURCE_DIR}/control/Cargo.toml"
    "${CMAKE_SOURCE_DIR}/control/src/*.rs"
    "${CMAKE_SOURCE_DIR}/core/harbor-core/Cargo.toml"
    "${CMAKE_SOURCE_DIR}/core/harbor-core/src/*.rs"
)
file(GLOB_RECURSE HARBOR_MEDIA_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_SOURCE_DIR}/media/go.mod"
    "${CMAKE_SOURCE_DIR}/media/go.sum"
    "${CMAKE_SOURCE_DIR}/media/*.go"
)
set(HARBOR_CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/cargo-target")
set(HARBOR_CORE_BINARY "${HARBOR_CARGO_TARGET_DIR}/debug/harbor-core${CMAKE_EXECUTABLE_SUFFIX}")

add_custom_command(
    OUTPUT "${HARBOR_CORE_BINARY}"
    COMMAND "${CMAKE_COMMAND}" -E env "CARGO_TARGET_DIR=${HARBOR_CARGO_TARGET_DIR}"
        "${CARGO_EXECUTABLE}" build --locked --manifest-path "${CMAKE_SOURCE_DIR}/Cargo.toml"
        --package harbor-core
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
    DEPENDS ${HARBOR_CORE_SOURCES}
    COMMENT "Building the Harbor Rust core"
    VERBATIM
)
add_custom_target(harbor_core_build DEPENDS "${HARBOR_CORE_BINARY}")

# harbor-media remains an internal child only: build it with the application,
# place it beside harbor-core, and never create a launcher or UI target for it.
set(HARBOR_MEDIA_BINARY "${CMAKE_BINARY_DIR}/media/harbor-media${CMAKE_EXECUTABLE_SUFFIX}")
set(HARBOR_GO_TAGS "")
if(WIN32 AND HARBOR_WINDOWS_VPX)
    set(HARBOR_GO_TAGS -tags harborvpx)
endif()
add_custom_command(
    OUTPUT "${HARBOR_MEDIA_BINARY}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory "${CMAKE_BINARY_DIR}/media"
    COMMAND "${GO_EXECUTABLE}" build -trimpath ${HARBOR_GO_TAGS} -o "${HARBOR_MEDIA_BINARY}" ./cmd/harbor-media
    WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}/media"
    DEPENDS ${HARBOR_MEDIA_SOURCES}
    COMMENT "Building the Harbor internal Pion media worker"
    VERBATIM
)
add_custom_target(harbor_media_build DEPENDS "${HARBOR_MEDIA_BINARY}")

qt_add_executable(harbor
    main.cpp
    native/HarborAppIcon.cpp
    native/HarborAppIcon.h
    native/HarborCoreSupervisor.cpp
    native/HarborCoreSupervisor.h
    native/HarborFacade.cpp
    native/HarborFacade.h
    native/HarborSettings.cpp
    native/HarborSettings.h
    native/HarborTray.cpp
    native/HarborTray.h
    native/HarborAutostart.cpp
    native/HarborAutostart.h
    native/HarborNotifications.cpp
    native/HarborNotifications.h
    native/HarborPresenceSource.cpp
    native/HarborPresenceSource.h
    native/HarborSounds.cpp
    native/HarborSounds.h
    native/HarborUpdater.cpp
    native/HarborUpdater.h
)
target_include_directories(harbor PRIVATE "${CMAKE_SOURCE_DIR}/native/third-party")
add_dependencies(harbor harbor_core_build harbor_media_build)
add_custom_command(TARGET harbor POST_BUILD
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
        "${HARBOR_CORE_BINARY}" "$<TARGET_FILE_DIR:harbor>/harbor-core${CMAKE_EXECUTABLE_SUFFIX}"
    COMMAND "${CMAKE_COMMAND}" -E copy_if_different
        "${HARBOR_MEDIA_BINARY}" "$<TARGET_FILE_DIR:harbor>/harbor-media${CMAKE_EXECUTABLE_SUFFIX}"
    COMMENT "Placing private Harbor core and media workers beside the application"
)

set(HARBOR_QML_FILES
    qml/Main.qml
    qml/components/HarborActivityItem.qml
    qml/components/HarborAudioVisualizer.qml
    qml/components/HarborAvatar.qml
    qml/components/HarborBadge.qml
    qml/components/HarborButton.qml
    qml/components/HarborCard.qml
    qml/components/HarborChoiceChip.qml
    qml/components/HarborDeveloperPanel.qml
    qml/components/HarborDeviceCard.qml
    qml/components/HarborEmptyState.qml
    qml/components/HarborErrorState.qml
    qml/components/HarborGraph.qml
    qml/components/HarborIcon.qml
    qml/components/HarborIconButton.qml
    qml/components/HarborInput.qml
    qml/components/HarborLogo.qml
    qml/components/HarborMetricCard.qml
    qml/components/HarborMockQr.qml
    qml/components/HarborModal.qml
    qml/components/HarborNavIcon.qml
    qml/components/HarborWidget.qml
    qml/components/HarborOverlayView.qml
    qml/components/HarborPage.qml
    qml/components/HarborPageHeader.qml
    qml/components/HarborPhoneMap.qml
    qml/components/HarborActivityBridge.qml
    qml/components/HarborCallBridge.qml
    qml/components/HarborContactsBridge.qml
    qml/components/HarborNetworkBridge.qml
    qml/components/HarborNotificationsBridge.qml
    qml/components/HarborNotificationWidget.qml
    qml/components/HarborDirectBridge.qml
    qml/components/HarborMobileBridge.qml
    qml/components/HarborPairingBridge.qml
    qml/components/HarborParticles.qml
    qml/components/HarborRouteMap.qml
    qml/components/HarborSectionCard.qml
    qml/components/HarborSelect.qml
    qml/components/HarborSettingRow.qml
    qml/components/HarborSettingsBridge.qml
    qml/components/HarborSidebar.qml
    qml/components/HarborSlider.qml
    qml/components/HarborSpinner.qml
    qml/components/HarborStateLayer.qml
    qml/components/HarborTitleBar.qml
    qml/components/HarborToastHost.qml
    qml/components/HarborToggle.qml
    qml/components/HarborTrayPreview.qml
    qml/components/HarborUpdateOverlay.qml
    qml/components/HarborWave.qml
    qml/views/ActivityView.qml
    qml/views/CallView.qml
    qml/views/ChatView.qml
    qml/views/DevicesView.qml
    qml/views/HomeView.qml
    qml/views/MobileView.qml
    qml/views/NetworkView.qml
    qml/views/NotificationsView.qml
    qml/views/OnboardingView.qml
    qml/views/PairingView.qml
    qml/views/ProfileView.qml
    qml/views/SettingsView.qml
    qml/Theme.qml
    qml/AppState.qml
    qml/I18n.qml
    qml/MockController.qml
    qml/i18n/en.js
    qml/i18n/pt_BR.js
)

# The source tree is organized by role, but every public QML type belongs to
# one Harbor module. Resource aliases keep loadFromModule and intra-module
# imports stable regardless of the on-disk folders.
foreach(qml_file IN LISTS HARBOR_QML_FILES)
    get_filename_component(qml_name "${qml_file}" NAME)
    set_source_files_properties("${qml_file}" PROPERTIES QT_RESOURCE_ALIAS "${qml_name}")
endforeach()

set_source_files_properties(qml/Theme.qml qml/AppState.qml qml/I18n.qml qml/MockController.qml
    PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

# JS library catalogs are imported by relative path ("i18n/en.js" from
# I18n.qml), so unlike QML types their alias must preserve the subfolder.
set_source_files_properties(qml/i18n/en.js PROPERTIES QT_RESOURCE_ALIAS "i18n/en.js")
set_source_files_properties(qml/i18n/pt_BR.js PROPERTIES QT_RESOURCE_ALIAS "i18n/pt_BR.js")

set(HARBOR_RESOURCE_FILES
    qml/assets/fonts/InterVariable.ttf
    qml/assets/fonts/JetBrainsMono-Regular.ttf
    qml/assets/fonts/Inter-OFL.txt
    qml/assets/fonts/JetBrainsMono-OFL.txt
    qml/assets/images/harbor.png
)
foreach(resource_file IN LISTS HARBOR_RESOURCE_FILES)
    get_filename_component(resource_name "${resource_file}" NAME)
    if(resource_file MATCHES "\\.png$")
        set_source_files_properties("${resource_file}" PROPERTIES
            QT_RESOURCE_ALIAS "images/${resource_name}"
        )
    else()
        set_source_files_properties("${resource_file}" PROPERTIES
            QT_RESOURCE_ALIAS "fonts/${resource_name}"
        )
    endif()
endforeach()

qt_add_qml_module(harbor
    URI Harbor
    VERSION 2.0
    QML_FILES ${HARBOR_QML_FILES}
    RESOURCES ${HARBOR_RESOURCE_FILES}
)

target_link_libraries(harbor PRIVATE Qt6::Core Qt6::Quick Qt6::Widgets Qt6::Network)
if(WIN32)
    # GUI subsystem: no console window behind the app. Development output
    # still lands in the debugger / `qDebug` sinks.
    set_target_properties(harbor PROPERTIES WIN32_EXECUTABLE TRUE)
endif()
if(TARGET Qt6::DBus)
    target_link_libraries(harbor PRIVATE Qt6::DBus)
    target_compile_definitions(harbor PRIVATE HARBOR_HAVE_DBUS=1)
endif()

if(BUILD_TESTING)
    find_package(Qt6 6.5 REQUIRED COMPONENTS QuickTest Test)
    add_subdirectory(tests)
endif()

install(TARGETS harbor BUNDLE DESTINATION . RUNTIME DESTINATION bin)
install(PROGRAMS "${HARBOR_CORE_BINARY}" "${HARBOR_MEDIA_BINARY}" DESTINATION bin)
if(UNIX AND NOT APPLE)
    # Linux desktop integration for the real tray icon and the XDG autostart
    # entry: the icon name here must match the one HarborTray looks up.
    # Windows carries its own icon resource and Startup-folder shortcut, so
    # these files stay out of non-Linux packages.
    install(FILES packaging/harbor.desktop DESTINATION share/applications)
    install(DIRECTORY packaging/icons/hicolor DESTINATION share/icons)
endif()
