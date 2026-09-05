# NOTE: harbor_qmltests lives in the root CMakeLists on purpose. Its QML
# module reuses HARBOR_QML_FILES with tree-relative paths, so the qmlcache
# output names never escape into ".." segments — Windows path handling
# mangles trailing-dot segments and neither MSBuild nor Ninja can create
# those directories there.

qt_add_executable(harbor_core_supervisor_tests
    HarborCoreSupervisorTest.cpp
    "${CMAKE_SOURCE_DIR}/native/HarborAppIcon.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborAppIcon.h"
    "${CMAKE_SOURCE_DIR}/native/HarborCoreSupervisor.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborCoreSupervisor.h"
    "${CMAKE_SOURCE_DIR}/native/HarborFacade.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborFacade.h"
    "${CMAKE_SOURCE_DIR}/native/HarborSettings.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborSettings.h"
    "${CMAKE_SOURCE_DIR}/native/HarborPresenceSource.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborPresenceSource.h"
)
add_dependencies(harbor_core_supervisor_tests harbor_core_build)
target_include_directories(harbor_core_supervisor_tests PRIVATE "${CMAKE_SOURCE_DIR}/native")
target_compile_definitions(harbor_core_supervisor_tests PRIVATE
    HARBOR_TEST_CORE_EXECUTABLE=\"${HARBOR_CORE_BINARY}\"
)
target_link_libraries(harbor_core_supervisor_tests PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
add_test(NAME harbor_core_supervisor_tests COMMAND harbor_core_supervisor_tests)
# The core resolves durable state from XDG_STATE_HOME first and HOME only as
# a fallback, so both must be pinned — pinning HOME alone leaks test identity
# and settings writes into the developer's real state directory.
set_tests_properties(harbor_core_supervisor_tests PROPERTIES
    ENVIRONMENT
    "HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home;XDG_STATE_HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home/state;QT_QPA_PLATFORM=offscreen"
)

# The avatar importer only needs the facade object, never a running core,
# but it shares the supervisor target's sources and state pinning because
# HarborFacade's constructor requires the supervisor.
qt_add_executable(harbor_avatar_import_tests
    HarborAvatarImportTest.cpp
    "${CMAKE_SOURCE_DIR}/native/HarborAppIcon.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborAppIcon.h"
    "${CMAKE_SOURCE_DIR}/native/HarborCoreSupervisor.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborCoreSupervisor.h"
    "${CMAKE_SOURCE_DIR}/native/HarborFacade.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborFacade.h"
    "${CMAKE_SOURCE_DIR}/native/HarborSettings.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborSettings.h"
    "${CMAKE_SOURCE_DIR}/native/HarborPresenceSource.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborPresenceSource.h"
)
add_dependencies(harbor_avatar_import_tests harbor_core_build)
target_include_directories(harbor_avatar_import_tests PRIVATE "${CMAKE_SOURCE_DIR}/native")
target_compile_definitions(harbor_avatar_import_tests PRIVATE
    HARBOR_TEST_CORE_EXECUTABLE=\"${HARBOR_CORE_BINARY}\"
)
target_link_libraries(harbor_avatar_import_tests PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
add_test(NAME harbor_avatar_import_tests COMMAND harbor_avatar_import_tests)
set_tests_properties(harbor_avatar_import_tests PROPERTIES
    ENVIRONMENT
    "QT_QPA_PLATFORM=offscreen;HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home;XDG_STATE_HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home/state"
)

add_test(
    NAME harbor_source_contracts
    COMMAND "${CMAKE_COMMAND}"
        -DROOT_DIR=${CMAKE_SOURCE_DIR}
        -P "${CMAKE_CURRENT_SOURCE_DIR}/SourceContracts.cmake"
)

qt_add_executable(harbor_lifecycle_tests HarborLifecycleAdapterTest.cpp
    "${CMAKE_SOURCE_DIR}/native/HarborTray.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborTray.h"
    "${CMAKE_SOURCE_DIR}/native/HarborAutostart.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborAutostart.h"
)
target_include_directories(harbor_lifecycle_tests PRIVATE "${CMAKE_SOURCE_DIR}/native")
target_link_libraries(harbor_lifecycle_tests PRIVATE Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Test)
add_test(NAME harbor_lifecycle_tests COMMAND harbor_lifecycle_tests)
set_tests_properties(harbor_lifecycle_tests PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen")

qt_add_executable(harbor_updater_tests HarborUpdaterTest.cpp
    "${CMAKE_SOURCE_DIR}/native/HarborUpdater.cpp"
    "${CMAKE_SOURCE_DIR}/native/HarborUpdater.h"
)
target_include_directories(harbor_updater_tests PRIVATE "${CMAKE_SOURCE_DIR}/native")
target_compile_definitions(harbor_updater_tests PRIVATE HARBOR_VERSION_STRING="2.1.0-test")
target_link_libraries(harbor_updater_tests PRIVATE Qt6::Core Qt6::Network Qt6::Test)
add_test(NAME harbor_updater_tests COMMAND harbor_updater_tests)
set_tests_properties(harbor_updater_tests PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen")
