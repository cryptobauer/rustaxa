cmake_minimum_required(VERSION 3.28)

if(APPLE)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "15.5" CACHE STRING "Minimum macOS deployment version")
endif()

project(rustaxa-vdf LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

option(ENABLE_TESTS "Build tests" ON)
option(ENABLE_SANITIZER "Enable AddressSanitizer and UndefinedBehaviorSanitizer" OFF)

# include(FetchContent)
# FetchContent_Declare(
#     googletest
#     URL https://github.com/google/googletest/archive/03597a01ee50ed33e9dfd640b249b4be3799d395.zip
# )
# # For Windows: Prevent overriding the parent project's compiler/linker settings
# set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
# FetchContent_MakeAvailable(googletest)

set(RUST_LIB_PATH ${CMAKE_CURRENT_BINARY_DIR}/cargo_build/release/librustaxa_vdf.a)

add_custom_command(
    OUTPUT ${RUST_LIB_PATH}
    COMMAND cargo build --release --target-dir ${CMAKE_CURRENT_BINARY_DIR}/cargo_build
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}
    DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/src/lib.rs ${CMAKE_CURRENT_SOURCE_DIR}/src/bindings.rs ${CMAKE_CURRENT_SOURCE_DIR}/src/hash.rs ${CMAKE_CURRENT_SOURCE_DIR}/src/puzzle.rs ${CMAKE_CURRENT_SOURCE_DIR}/src/verifier.rs ${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml
    COMMENT "Building Rust library via Cargo"
    VERBATIM
)

add_library(rustaxa-vdf STATIC IMPORTED GLOBAL)
set_target_properties(rustaxa-vdf PROPERTIES
    IMPORTED_LOCATION ${RUST_LIB_PATH}
    INTERFACE_INCLUDE_DIRECTORIES "${CMAKE_CURRENT_BINARY_DIR}/cargo_build/cxxbridge/"
)

add_custom_target(rust-lib-target DEPENDS ${RUST_LIB_PATH})
add_dependencies(rustaxa-vdf rust-lib-target)

if(ENABLE_SANITIZER)
    set(RUST_FLAGS "RUSTFLAGS=-Zsanitizer=address")
    set_property(TARGET rust-lib-target PROPERTY
        ENVIRONMENT "${RUST_FLAGS}")
endif()

# if(ENABLE_TESTS)
#     enable_testing()
#     include(GoogleTest)

#     add_subdirectory(tests/cpp)
# endif()
