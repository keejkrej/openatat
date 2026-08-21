cmake_minimum_required(VERSION 3.16)
project(fcitx5-openatat VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

enable_testing()

# Always build the ImeFilter state machine tests. No Fcitx5, no display.
add_executable(openatat-filter-test
    src/openatat_filter.cpp
    src/openatat_filter_test.cpp
    src/openatat_socket.cpp)
target_include_directories(openatat-filter-test PRIVATE src)
add_test(NAME openatat-filter-test COMMAND openatat-filter-test)

find_package(PkgConfig QUIET)
if(PkgConfig_FOUND)
    pkg_check_modules(DBUS QUIET dbus-1)
endif()

find_package(ECM QUIET)
if(ECM_FOUND)
    list(APPEND CMAKE_MODULE_PATH "${ECM_MODULE_PATH}")
endif()

find_package(Fcitx5Core QUIET)
if(NOT Fcitx5Core_FOUND)
    message(WARNING
        "Fcitx5Core not found — shipping filter tests only, not linking the addon.\n"
        "  Arch/Omarchy:  pacman -S --needed fcitx5 extra-cmake-modules cmake ninja pkgconf\n"
        "  Debian/Ubuntu: apt install fcitx5 libfcitx5core-dev extra-cmake-modules cmake ninja-build\n"
        "Do not treat this warning as a successful Fcitx5 link.")
    return()
endif()

if(EXISTS "${FCITX_INSTALL_CMAKECONFIG_DIR}/Fcitx5Utils/Fcitx5CompilerSettings.cmake")
    include("${FCITX_INSTALL_CMAKECONFIG_DIR}/Fcitx5Utils/Fcitx5CompilerSettings.cmake")
endif()

set(OPENATAT_ADDON_SOURCES
    src/openatat.cpp
    src/openatat_filter.cpp
    src/openatat_socket.cpp
    src/openatat_secure.cpp)

if(COMMAND add_fcitx5_addon)
    add_fcitx5_addon(openatat ${OPENATAT_ADDON_SOURCES})
else()
    add_library(openatat MODULE ${OPENATAT_ADDON_SOURCES})
endif()

target_include_directories(openatat PRIVATE src)
target_link_libraries(openatat PRIVATE Fcitx5::Core)

if(DBUS_FOUND)
    target_compile_definitions(openatat PRIVATE OPENATAT_HAVE_DBUS)
    target_include_directories(openatat PRIVATE ${DBUS_INCLUDE_DIRS})
    target_link_libraries(openatat PRIVATE ${DBUS_LIBRARIES})
    message(STATUS "fcitx5-openatat: AT-SPI probe enabled (dbus-1)")
else()
    message(STATUS
        "fcitx5-openatat: dbus-1 not found; password fields use Fcitx5 "
        "CapabilityFlag::PasswordOrSensitive only")
endif()

if(NOT FCITX_INSTALL_ADDONDIR)
    include(GNUInstallDirs)
    set(FCITX_INSTALL_ADDONDIR "${CMAKE_INSTALL_FULL_LIBDIR}/fcitx5")
endif()
if(NOT FCITX_INSTALL_PKGDATADIR)
    include(GNUInstallDirs)
    set(FCITX_INSTALL_PKGDATADIR "${CMAKE_INSTALL_FULL_DATADIR}/fcitx5")
endif()

install(TARGETS openatat DESTINATION "${FCITX_INSTALL_ADDONDIR}")

configure_file(src/openatat.conf.in openatat.conf @ONLY)
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/openatat.conf"
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon")

message(STATUS "fcitx5-openatat: will install addon to ${FCITX_INSTALL_ADDONDIR}")
message(STATUS "fcitx5-openatat: will install conf to ${FCITX_INSTALL_PKGDATADIR}/addon")
