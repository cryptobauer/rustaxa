
if(TARGET rustaxa-bridge)
    return()
endif()

find_program(CARGO_EXE NAMES cargo REQUIRED)

set(RUST_ROOT "${PROJECT_SOURCE_DIR}/rust")
set(RUST_TARGET_DIR "${PROJECT_BINARY_DIR}/rust/target")
set(BRIDGE_INCLUDE_DIR "${RUST_TARGET_DIR}/cxxbridge")

file(MAKE_DIRECTORY "${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/src")

if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(CARGO_MODE_ARGS "")
    set(RUST_LIB_DIR "${RUST_TARGET_DIR}/debug")
else()
    set(CARGO_MODE_ARGS "--release")
    set(RUST_LIB_DIR "${RUST_TARGET_DIR}/release")
endif()

set(RUST_LIB "${RUST_LIB_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}rustaxa_bridge${CMAKE_STATIC_LIBRARY_SUFFIX}")

# --- Helper Script for Header Sync ---
# Generates a script to copy cxxbridge headers from the cargo build tree to our include dir.
# We generate this file at configure time, to be run at build time.
set(SYNC_SCRIPT "${CMAKE_CURRENT_BINARY_DIR}/sync_bridge_headers.cmake")
file(WRITE "${SYNC_SCRIPT}" "
    file(GLOB_RECURSE HEADERS
        \"\${TARGET_DIR}/cxxbridge/rustaxa-bridge/src/*.rs.h\"
        \"\${TARGET_DIR}/*/out/cxxbridge/include/rustaxa-bridge/src/*.rs.h\"
    )
    foreach(HEADER \${HEADERS})
        get_filename_component(FNAME \"\${HEADER}\" NAME)
        # Using configure_file with COPYONLY updates timestamps only on change, preventing rebuilds
        configure_file(\"\${HEADER}\" \"${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/src/\${FNAME}\" COPYONLY)
    endforeach()
")

# --- Build Target ---

add_custom_target(rust-workspace-build ALL
    COMMENT "Building Rust workspace"

    # 1. Run Cargo
    COMMAND ${CMAKE_COMMAND} -E env
        "CC=${CMAKE_C_COMPILER}"
        "CXX=${CMAKE_CXX_COMPILER}"
        "${CARGO_EXE}" build ${CARGO_MODE_ARGS} --target-dir "${RUST_TARGET_DIR}"

    # 2. Sync Headers
    COMMAND ${CMAKE_COMMAND}
        -DTARGET_DIR=${RUST_TARGET_DIR}
        -P "${SYNC_SCRIPT}"

    WORKING_DIRECTORY "${RUST_ROOT}"
    BYPRODUCTS "${RUST_LIB}"
    VERBATIM
)

# --- Imported Library ---

add_library(rustaxa-bridge STATIC IMPORTED GLOBAL)
add_dependencies(rustaxa-bridge rust-workspace-build)

set_target_properties(rustaxa-bridge PROPERTIES
    IMPORTED_LOCATION "${RUST_LIB}"
    INTERFACE_INCLUDE_DIRECTORIES "${BRIDGE_INCLUDE_DIR}"
)

add_library(Rustaxa::bridge ALIAS rustaxa-bridge)
