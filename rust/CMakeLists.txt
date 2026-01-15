# Cargo workspace root build
set(WORKSPACE_ROOT_DIR ${CMAKE_CURRENT_SOURCE_DIR})
set(WORKSPACE_TARGET_DIR ${CMAKE_CURRENT_BINARY_DIR}/target)

# Determine Cargo build mode based on CMAKE_BUILD_TYPE
if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(CARGO_BUILD_MODE "")
    set(CARGO_TARGET_SUBDIR "debug")
else()
    set(CARGO_BUILD_MODE "--release")
    set(CARGO_TARGET_SUBDIR "release")
endif()

set(RUST_VDF_LIB_PATH ${WORKSPACE_TARGET_DIR}/${CARGO_TARGET_SUBDIR}/librustaxa_vdf.a)

# Use a custom target that always runs and let Cargo handle dependency tracking
# This is more efficient as Cargo already knows what needs to be rebuilt
add_custom_target(rust-workspace-build ALL
    COMMAND ${CMAKE_COMMAND} -E echo "Debug: LLVM_VERSION is '${LLVM_VERSION}'"
    COMMAND ${CMAKE_COMMAND} -E env CC=clang-${LLVM_VERSION} CXX=clang++-${LLVM_VERSION} cargo --version
    COMMAND ${CMAKE_COMMAND} -E env
        # Set the CC and CXX environment variables for the build to avoid picking up sscache.
        CC=clang-${LLVM_VERSION}
        CXX=clang++-${LLVM_VERSION}
        cargo build -vv ${CARGO_BUILD_MODE} --target-dir ${WORKSPACE_TARGET_DIR}
    WORKING_DIRECTORY ${WORKSPACE_ROOT_DIR}
    COMMENT "Building Rust workspace via Cargo (${CMAKE_BUILD_TYPE} mode)"
    VERBATIM
    BYPRODUCTS ${RUST_VDF_LIB_PATH}
)

# Ensure the cxxbridge directory exists after build
add_custom_command(TARGET rust-workspace-build POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E make_directory "${WORKSPACE_TARGET_DIR}/cxxbridge"
    COMMAND ${CMAKE_COMMAND} -E make_directory "${WORKSPACE_TARGET_DIR}/cxxbridge/rustaxa-vdf/src"
    COMMAND sh -c "echo 'Searching for bindings.rs.h in ${WORKSPACE_TARGET_DIR}...' && find ${WORKSPACE_TARGET_DIR} -path '*/out/cxxbridge/include/rustaxa-vdf/src/bindings.rs.h' -exec cp -v {} ${WORKSPACE_TARGET_DIR}/cxxbridge/rustaxa-vdf/src/bindings.rs.h \\;"
    COMMENT "Ensuring cxxbridge directory exists and populating with headers"
    VERBATIM
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
