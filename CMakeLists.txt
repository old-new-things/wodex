cmake_minimum_required(VERSION 3.20)
file(STRINGS "${CMAKE_CURRENT_LIST_DIR}/VERSION" WODEX_VERSION LIMIT_COUNT 1)
file(READ "${CMAKE_CURRENT_LIST_DIR}/VERSION" WODEX_VERSION_FILE)
string(LENGTH "${WODEX_VERSION}" WODEX_VERSION_LENGTH)
if(NOT WODEX_VERSION MATCHES "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$" OR WODEX_VERSION_LENGTH GREATER 15 OR
   (NOT WODEX_VERSION_FILE STREQUAL "${WODEX_VERSION}" AND NOT WODEX_VERSION_FILE STREQUAL "${WODEX_VERSION}\n"))
  message(FATAL_ERROR "VERSION must contain a stable X.Y.Z version of at most 15 ASCII characters.")
endif()
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${CMAKE_CURRENT_LIST_DIR}/VERSION")
project(wodex VERSION "${WODEX_VERSION}" LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
find_package(SDL2 REQUIRED CONFIG HINTS /opt/homebrew/opt/sdl2 /usr/local/opt/sdl2)
find_package(SDL2_ttf QUIET CONFIG)
find_package(CURL REQUIRED)
find_package(Threads REQUIRED)
find_program(CARGO_EXECUTABLE cargo REQUIRED)

set(WODEX_FONT "${CMAKE_SOURCE_DIR}/romfs/fonts/NotoSansCJKsc-Regular.otf")
if(NOT EXISTS "${WODEX_FONT}" OR NOT EXISTS "${CMAKE_SOURCE_DIR}/romfs/fonts/OFL.txt")
  message(FATAL_ERROR "Missing bundled Noto font or license. Restore romfs/fonts from Git.")
endif()
file(SHA256 "${WODEX_FONT}" WODEX_FONT_SHA256)
if(NOT WODEX_FONT_SHA256 STREQUAL "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b")
  message(FATAL_ERROR "Bundled Noto font checksum does not match the pinned Sans2.004 release.")
endif()
set(WODEX_CODE_FONT "${CMAKE_SOURCE_DIR}/romfs/fonts/SourceCodePro-Regular.otf")
if(NOT EXISTS "${WODEX_CODE_FONT}" OR NOT EXISTS "${CMAKE_SOURCE_DIR}/romfs/fonts/SourceCodePro-OFL.txt")
  message(FATAL_ERROR "Missing bundled code font or license. Restore romfs/fonts from Git.")
endif()
file(SHA256 "${WODEX_CODE_FONT}" WODEX_CODE_FONT_SHA256)
if(NOT WODEX_CODE_FONT_SHA256 STREQUAL "9f9664e2edf6f045c11e774f9bd0be6993971f2544a39061a5ce478b96b051f8")
  message(FATAL_ERROR "Bundled code font checksum does not match the pinned Source Code Pro release.")
endif()
file(GLOB_RECURSE WODEX_FONT_RESOURCES CONFIGURE_DEPENDS "${CMAKE_SOURCE_DIR}/romfs/fonts/*")
set(WODEX_RESOURCE_DIR "${CMAKE_BINARY_DIR}/resources")
add_custom_command(OUTPUT "${WODEX_RESOURCE_DIR}/fonts.stamp"
  COMMAND "${CMAKE_COMMAND}" -E make_directory "${WODEX_RESOURCE_DIR}/fonts"
  COMMAND "${CMAKE_COMMAND}" -E copy_directory "${CMAKE_SOURCE_DIR}/romfs/fonts" "${WODEX_RESOURCE_DIR}/fonts"
  COMMAND "${CMAKE_COMMAND}" -E touch "${WODEX_RESOURCE_DIR}/fonts.stamp"
  DEPENDS ${WODEX_FONT_RESOURCES}
  COMMENT "Copying licensed CJK font resources")
add_custom_target(wodex_resources DEPENDS "${WODEX_RESOURCE_DIR}/fonts.stamp")

if(NOT TARGET SDL2_ttf::SDL2_ttf AND NOT TARGET SDL2_ttf::SDL2_ttf-static)
  if(NOT EXISTS "${CMAKE_SOURCE_DIR}/external/SDL2_ttf-2.24.0/CMakeLists.txt")
    message(FATAL_ERROR "Install SDL2_ttf development files or run scripts/setup-desktop.sh")
  endif()
  set(SDL2TTF_SAMPLES OFF CACHE BOOL "" FORCE)
  set(SDL2TTF_INSTALL OFF CACHE BOOL "" FORCE)
  set(SDL2TTF_VENDORED OFF CACHE BOOL "" FORCE)
  set(SDL2TTF_HARFBUZZ OFF CACHE BOOL "" FORCE)
  set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
  add_subdirectory(external/SDL2_ttf-2.24.0 EXCLUDE_FROM_ALL)
endif()
if(TARGET SDL2_ttf::SDL2_ttf-static)
  set(WODEX_TTF SDL2_ttf::SDL2_ttf-static)
else()
  set(WODEX_TTF SDL2_ttf::SDL2_ttf)
endif()

set(RUST_LIBRARY "${CMAKE_SOURCE_DIR}/target/release/${CMAKE_STATIC_LIBRARY_PREFIX}wodex_core${CMAKE_STATIC_LIBRARY_SUFFIX}")
file(GLOB_RECURSE RUST_SOURCES CONFIGURE_DEPENDS "core/src/*.rs")
# Let Cargo track environment-based build identity as well as source changes.
add_custom_target(wodex_core_build
  COMMAND "${CARGO_EXECUTABLE}" build --locked --release -p wodex-core
  BYPRODUCTS "${RUST_LIBRARY}"
  DEPENDS ${RUST_SOURCES} Cargo.toml Cargo.lock core/Cargo.toml core/build.rs
  WORKING_DIRECTORY "${CMAKE_SOURCE_DIR}"
  COMMENT "Building Rust application and conversation core")
add_library(wodex_core STATIC IMPORTED)
set_target_properties(wodex_core PROPERTIES IMPORTED_LOCATION "${RUST_LIBRARY}")
target_link_libraries(wodex_core INTERFACE ${CMAKE_DL_LIBS})
if(UNIX)
  target_link_libraries(wodex_core INTERFACE m)
endif()
add_dependencies(wodex_core wodex_core_build)

add_library(wodex_native STATIC
  native/src/platform.cpp native/src/host.cpp native/src/render_backend.cpp
  native/src/fs_bindings.cpp native/src/http_bindings.cpp native/src/runtime_bindings.cpp
  native/src/home_menu_bindings.cpp)
target_include_directories(wodex_native PUBLIC native/include)
target_link_libraries(wodex_native PUBLIC wodex_core CURL::libcurl Threads::Threads SDL2::SDL2 ${WODEX_TTF})
target_compile_options(wodex_native PRIVATE -Wall -Wextra -Wpedantic)
if(APPLE)
  target_link_libraries(wodex_native PUBLIC "-framework Security" "-framework SystemConfiguration")
endif()

add_executable(wodex native/src/main.cpp)
add_dependencies(wodex wodex_resources)
target_link_libraries(wodex PRIVATE wodex_native)
target_compile_options(wodex PRIVATE -Wall -Wextra -Wpedantic)
target_compile_definitions(wodex PRIVATE WODEX_VERSION="${PROJECT_VERSION}")

include(CTest)
if(BUILD_TESTING)
  # Thin C++ facades let the established native harnesses exercise Rust backends.
  # This compatibility code is never linked into the application or Switch NRO.
  add_library(wodex_test_backends STATIC tests/backend_compat.cpp tests/storage_compat.cpp)
  target_include_directories(wodex_test_backends PUBLIC tests/include external)
  target_link_libraries(wodex_test_backends PUBLIC wodex_native)
  target_compile_options(wodex_test_backends PRIVATE -Wall -Wextra -Wpedantic)
  add_executable(wodex_native_tests tests/native_tests.cpp)
  target_include_directories(wodex_native_tests PRIVATE native/include external)
  target_link_libraries(wodex_native_tests PRIVATE wodex_test_backends)
  find_package(Python3 REQUIRED COMPONENTS Interpreter)
  add_test(NAME native_integration COMMAND "${Python3_EXECUTABLE}"
    "${CMAKE_SOURCE_DIR}/tests/mock_deepseek.py" "$<TARGET_FILE:wodex_native_tests>")
  add_test(NAME ui_render COMMAND wodex --demo --screenshot "${CMAKE_BINARY_DIR}/smoke.bmp")
  set_tests_properties(ui_render PROPERTIES ENVIRONMENT
    "SDL_VIDEODRIVER=dummy;WODEX_DATA_DIR=${CMAKE_BINARY_DIR}/test-data")
  add_test(NAME ui_updates COMMAND wodex --demo --updates --screenshot "${CMAKE_BINARY_DIR}/updates.bmp")
  set_tests_properties(ui_updates PROPERTIES ENVIRONMENT
    "SDL_VIDEODRIVER=dummy;WODEX_DATA_DIR=${CMAKE_BINARY_DIR}/test-data")
  add_test(NAME ui_home_menu COMMAND wodex --demo --home-menu --screenshot "${CMAKE_BINARY_DIR}/home-menu.bmp")
  set_tests_properties(ui_home_menu PROPERTIES ENVIRONMENT
    "SDL_VIDEODRIVER=dummy;WODEX_DATA_DIR=${CMAKE_BINARY_DIR}/test-data")
  add_test(NAME markdown_render COMMAND "${Python3_EXECUTABLE}"
    "${CMAKE_SOURCE_DIR}/tests/preview_markdown.py" "$<TARGET_FILE:wodex>"
    "${CMAKE_BINARY_DIR}/markdown-previews")
  add_executable(wodex_font_tests tests/font_tests.cpp)
  target_include_directories(wodex_font_tests PRIVATE native/include)
  target_link_libraries(wodex_font_tests PRIVATE wodex_native)
  target_compile_options(wodex_font_tests PRIVATE -Wall -Wextra -Wpedantic)
  add_dependencies(wodex_font_tests wodex_resources)
  add_test(NAME cjk_fonts COMMAND wodex_font_tests "${WODEX_RESOURCE_DIR}")
  set_tests_properties(cjk_fonts PROPERTIES ENVIRONMENT
    "SDL_VIDEODRIVER=dummy;WODEX_DATA_DIR=${CMAKE_BINARY_DIR}/font-test-data;WODEX_FONT=")
  add_executable(wodex_app_tests tests/app_tests.cpp)
  target_include_directories(wodex_app_tests PRIVATE native/include external)
  target_link_libraries(wodex_app_tests PRIVATE wodex_test_backends)
  target_compile_options(wodex_app_tests PRIVATE -Wall -Wextra -Wpedantic)
  add_dependencies(wodex_app_tests wodex_resources)
  add_test(NAME app_integration COMMAND "${Python3_EXECUTABLE}"
    "${CMAKE_SOURCE_DIR}/tests/mock_app.py" "$<TARGET_FILE:wodex_app_tests>" "${WODEX_RESOURCE_DIR}")
  set_tests_properties(app_integration PROPERTIES ENVIRONMENT "SDL_VIDEODRIVER=dummy")
  add_executable(wodex_image_tests tests/image_tests.cpp)
  target_include_directories(wodex_image_tests PRIVATE native/include external)
  target_link_libraries(wodex_image_tests PRIVATE wodex_test_backends)
  target_compile_options(wodex_image_tests PRIVATE -Wall -Wextra -Wpedantic)
  add_dependencies(wodex_image_tests wodex_resources)
  add_test(NAME image_integration COMMAND "${Python3_EXECUTABLE}"
    "${CMAKE_SOURCE_DIR}/tests/mock_images.py" "$<TARGET_FILE:wodex_image_tests>" "${WODEX_RESOURCE_DIR}"
    "${CMAKE_BINARY_DIR}/image-previews")
  add_executable(wodex_host_tests tests/host_tests.cpp)
  target_include_directories(wodex_host_tests PRIVATE native/include)
  target_link_libraries(wodex_host_tests PRIVATE wodex_native)
  target_compile_options(wodex_host_tests PRIVATE -Wall -Wextra -Wpedantic)
  add_dependencies(wodex_host_tests wodex_resources)
  add_test(NAME host_backend COMMAND wodex_host_tests "${WODEX_RESOURCE_DIR}")
  set_tests_properties(host_backend PROPERTIES ENVIRONMENT "SDL_VIDEODRIVER=dummy;WODEX_FONT=")
  add_executable(wodex_update_tests tests/update_tests.cpp)
  target_link_libraries(wodex_update_tests PRIVATE wodex_test_backends)
  target_compile_options(wodex_update_tests PRIVATE -Wall -Wextra -Wpedantic)
  add_test(NAME self_update COMMAND "${Python3_EXECUTABLE}"
    "${CMAKE_SOURCE_DIR}/tests/mock_updates.py" "$<TARGET_FILE:wodex_update_tests>")
endif()
