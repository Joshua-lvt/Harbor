qt_add_executable(harbor_qmltests quicktest_main.cpp)

set(HARBOR_TEST_QML_FILES)
foreach(qml_file IN LISTS HARBOR_QML_FILES)
    set(absolute_qml_file "${CMAKE_SOURCE_DIR}/${qml_file}")
    get_filename_component(qml_name "${qml_file}" NAME)
    set_source_files_properties("${absolute_qml_file}" PROPERTIES QT_RESOURCE_ALIAS "${qml_name}")
    list(APPEND HARBOR_TEST_QML_FILES "${absolute_qml_file}")
endforeach()

set_source_files_properties(
    "${CMAKE_SOURCE_DIR}/qml/Theme.qml"
    "${CMAKE_SOURCE_DIR}/qml/AppState.qml"
    "${CMAKE_SOURCE_DIR}/qml/I18n.qml"
    "${CMAKE_SOURCE_DIR}/qml/MockController.qml"
    PROPERTIES QT_QML_SINGLETON_TYPE TRUE
)
set_source_files_properties("${CMAKE_SOURCE_DIR}/qml/i18n/en.js"
    PROPERTIES QT_RESOURCE_ALIAS "i18n/en.js")
set_source_files_properties("${CMAKE_SOURCE_DIR}/qml/i18n/pt_BR.js"
    PROPERTIES QT_RESOURCE_ALIAS "i18n/pt_BR.js")

set(HARBOR_TEST_RESOURCE_FILES)
foreach(resource_file IN LISTS HARBOR_RESOURCE_FILES)
    set(absolute_resource_file "${CMAKE_SOURCE_DIR}/${resource_file}")
    get_filename_component(resource_name "${resource_file}" NAME)
    if(resource_file MATCHES "\\.png$")
        set_source_files_properties("${absolute_resource_file}" PROPERTIES
            QT_RESOURCE_ALIAS "images/${resource_name}"
        )
    else()
        set_source_files_properties("${absolute_resource_file}" PROPERTIES
            QT_RESOURCE_ALIAS "fonts/${resource_name}"
        )
    endif()
    list(APPEND HARBOR_TEST_RESOURCE_FILES "${absolute_resource_file}")
endforeach()

qt_add_qml_module(harbor_qmltests
    URI Harbor
    VERSION 2.0
    OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/Harbor"
    QML_FILES ${HARBOR_TEST_QML_FILES}
    RESOURCES ${HARBOR_TEST_RESOURCE_FILES}
)

target_link_libraries(harbor_qmltests PRIVATE Qt6::Quick Qt6::QuickTest)
target_compile_definitions(harbor_qmltests PRIVATE
    QUICK_TEST_SOURCE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/qml"
)

add_test(NAME harbor_qmltests COMMAND harbor_qmltests)
set_tests_properties(harbor_qmltests PROPERTIES
    ENVIRONMENT "QT_QPA_PLATFORM=offscreen;QT_QUICK_BACKEND=software;QML_DISABLE_DISK_CACHE=1"
)

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
    "HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home;XDG_STATE_HOME=${CMAKE_CURRENT_BINARY_DIR}/test-home/state"
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
