# Cargo workspace root build
set(WORKSPACE_ROOT_DIR ${CMAKE_CURRENT_SOURCE_DIR})
set(WORKSPACE_TARGET_DIR ${CMAKE_CURRENT_BINARY_DIR}/target)

# Determine Cargo build mode based on CMAKE_BUILD_TYPE
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(CARGO_BUILD_MODE "")
    set(CARGO_TARGET_SUBDIR "debug")
else()
    # Default to release if CMAKE_BUILD_TYPE is not set or unknown
    set(CARGO_BUILD_MODE "--release")
    set(CARGO_TARGET_SUBDIR "release")
endif()

set(RUST_VDF_LIB_PATH ${WORKSPACE_TARGET_DIR}/${CARGO_TARGET_SUBDIR}/librustaxa_vdf.a)

# Use a custom target that always runs and let Cargo handle dependency tracking
# This is more efficient as Cargo already knows what needs to be rebuilt
add_custom_target(rust-workspace-build ALL
    COMMAND cargo build ${CARGO_BUILD_MODE} --target-dir ${WORKSPACE_TARGET_DIR}
    WORKING_DIRECTORY ${WORKSPACE_ROOT_DIR}
    COMMENT "Building Rust workspace via Cargo (${CMAKE_BUILD_TYPE} mode)"
    VERBATIM
    BYPRODUCTS ${RUST_VDF_LIB_PATH}
)

# Ensure the cxxbridge directory exists after build
add_custom_command(TARGET rust-workspace-build POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E make_directory "${WORKSPACE_TARGET_DIR}/cxxbridge"
    COMMENT "Ensuring cxxbridge directory exists"
)

# Store the cxxbridge path as a variable for consumers to use
# CXX bridge generates headers at target/cxxbridge/ with structure: <crate-name>/src/<file>.rs.h
set(RUSTAXA_VDF_CXXBRIDGE_DIR "${WORKSPACE_TARGET_DIR}/cxxbridge" CACHE INTERNAL "CXX bridge include directory for rustaxa-vdf")

# Create imported library target
add_library(rustaxa-vdf STATIC IMPORTED GLOBAL)

set_target_properties(rustaxa-vdf PROPERTIES
    IMPORTED_LOCATION ${RUST_VDF_LIB_PATH}
    INTERFACE_INCLUDE_DIRECTORIES ${RUSTAXA_VDF_CXXBRIDGE_DIR}
)

# Create the cxxbridge directory during configuration to avoid CMake errors
file(MAKE_DIRECTORY "${RUSTAXA_VDF_CXXBRIDGE_DIR}")

# Make sure the library is built before it's used
add_dependencies(rustaxa-vdf rust-workspace-build)

# Handle sanitizers
if(TARAXA_ENABLE_SANITIZERS)
    set(RUST_FLAGS "RUSTFLAGS=-Zsanitizer=address")
    set_property(TARGET rust-workspace-build PROPERTY
        ENVIRONMENT "${RUST_FLAGS}")
endif()

# Add subdirectories for individual components if they have additional CMake logic
add_subdirectory(libs/vdf)
