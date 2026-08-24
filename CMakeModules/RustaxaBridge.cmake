if(TARGET rustaxa-bridge)
    return()
endif()

find_program(CARGO_EXE NAMES cargo REQUIRED)
find_program(OBJCOPY_EXE NAMES llvm-objcopy objcopy REQUIRED)
find_package(RocksDB REQUIRED)

get_target_property(RUSTAXA_ROCKSDB_INCLUDE_DIRS RocksDB::rocksdb INTERFACE_INCLUDE_DIRECTORIES)
get_target_property(RUSTAXA_ROCKSDB_LIB_DIRS RocksDB::rocksdb INTERFACE_LINK_DIRECTORIES)
if(NOT RUSTAXA_ROCKSDB_INCLUDE_DIRS)
    set(RUSTAXA_ROCKSDB_INCLUDE_DIRS "")
endif()
if(NOT RUSTAXA_ROCKSDB_LIB_DIRS)
    get_target_property(RUSTAXA_ROCKSDB_LOCATION_RELEASE RocksDB::rocksdb IMPORTED_LOCATION_RELEASE)
    get_target_property(RUSTAXA_ROCKSDB_LOCATION RocksDB::rocksdb IMPORTED_LOCATION)
    if(RUSTAXA_ROCKSDB_LOCATION_RELEASE)
        get_filename_component(RUSTAXA_ROCKSDB_LIB_DIRS "${RUSTAXA_ROCKSDB_LOCATION_RELEASE}" DIRECTORY)
    elseif(RUSTAXA_ROCKSDB_LOCATION)
        get_filename_component(RUSTAXA_ROCKSDB_LIB_DIRS "${RUSTAXA_ROCKSDB_LOCATION}" DIRECTORY)
    else()
        set(RUSTAXA_ROCKSDB_LIB_DIRS "")
    endif()
endif()

set(RUST_ROOT "${PROJECT_SOURCE_DIR}/rust")
set(RUST_TARGET_DIR "${PROJECT_BINARY_DIR}/rust/target")
set(BRIDGE_INCLUDE_DIR "${PROJECT_BINARY_DIR}/rust_bridge_include")

file(MAKE_DIRECTORY "${BRIDGE_INCLUDE_DIR}/rustaxa-bridge")

if(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(CARGO_MODE_ARGS "")
    set(RUST_LIB_DIR "${RUST_TARGET_DIR}/debug")
else()
    set(CARGO_MODE_ARGS "--release")
    set(RUST_LIB_DIR "${RUST_TARGET_DIR}/release")
endif()

set(RUST_LIB "${RUST_LIB_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}rustaxa_bridge${CMAKE_STATIC_LIBRARY_SUFFIX}")
set(RUST_APPLICATION_HOST_LIB
    "${RUST_LIB_DIR}/${CMAKE_STATIC_LIBRARY_PREFIX}rustaxa_application_host_bridge${CMAKE_STATIC_LIBRARY_SUFFIX}")

set(BRIDGE_LOCALIZE_SYMBOLS "")

# --- Helper Script for Header Sync ---
# Generates a script to copy cxxbridge headers from the cargo build tree to our include dir.
# We generate this file at configure time, to be run at build time.
set(SYNC_SCRIPT "${CMAKE_CURRENT_BINARY_DIR}/sync_bridge_headers.cmake")
file(WRITE "${SYNC_SCRIPT}" "
    file(MAKE_DIRECTORY \"${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/src\")
    file(GLOB_RECURSE HEADERS
        \"\${TARGET_DIR}/cxxbridge/rustaxa-bridge/src/*.rs.h\"
        \"\${TARGET_DIR}/*/out/cxxbridge/include/rustaxa-bridge/src/*.rs.h\"
    )
    foreach(HEADER \${HEADERS})
        get_filename_component(FNAME \"\${HEADER}\" NAME)
        # Using configure_file with COPYONLY updates timestamps only on change, preventing rebuilds
        configure_file(\"\${HEADER}\" \"${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/\${FNAME}\" COPYONLY)
        configure_file(\"\${HEADER}\" \"${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/src/\${FNAME}\" COPYONLY)
    endforeach()

    # Copy rust/cxx.h
    file(MAKE_DIRECTORY \"${BRIDGE_INCLUDE_DIR}/rust\")
    set(CXX_H \"\${TARGET_DIR}/cxxbridge/rust/cxx.h\")
    if(EXISTS \"\${CXX_H}\")
        configure_file(\"\${CXX_H}\" \"${BRIDGE_INCLUDE_DIR}/rust/cxx.h\" COPYONLY)
    endif()
")

set(FILTER_SCRIPT "${CMAKE_CURRENT_BINARY_DIR}/filter_bridge_symbols.cmake")
file(WRITE "${FILTER_SCRIPT}" "
    execute_process(
        COMMAND \"${OBJCOPY_EXE}\" --wildcard
")

foreach(SYMBOL IN LISTS BRIDGE_LOCALIZE_SYMBOLS)
    file(APPEND "${FILTER_SCRIPT}" "        --localize-symbol \"${SYMBOL}\"\n")
endforeach()

file(APPEND "${FILTER_SCRIPT}" "
        \"${RUST_LIB}\"
        RESULT_VARIABLE OBJCOPY_RES
    )
    if(NOT OBJCOPY_RES EQUAL 0)
        message(FATAL_ERROR \"Failed to localize bridge dependency symbols\")
    endif()
")

# --- Build Target ---

add_custom_target(rust-workspace-build ALL
    COMMENT "Building Rust workspace"

    # 1. Run Cargo
    COMMAND ${CMAKE_COMMAND} -E env
        "CC=${CMAKE_C_COMPILER}"
        "CXX=${CMAKE_CXX_COMPILER}"
        "ROCKSDB_INCLUDE_DIR=${RUSTAXA_ROCKSDB_INCLUDE_DIRS}"
        "ROCKSDB_LIB_DIR=${RUSTAXA_ROCKSDB_LIB_DIRS}"
        "ROCKSDB_STATIC=1"
        "TARAXA_VRF_LIB_DIR=${PROJECT_BINARY_DIR}/deps/taraxa-vrf/lib"
        "RUSTAXA_APPLICATION_HOST_BRIDGE_OUT=${RUST_APPLICATION_HOST_LIB}"
        "${CARGO_EXE}" build ${CARGO_MODE_ARGS} --target-dir "${RUST_TARGET_DIR}" -p rustaxa-bridge

    COMMAND ${CMAKE_COMMAND} -P "${FILTER_SCRIPT}"

    # 2. Sync Headers
    COMMAND ${CMAKE_COMMAND}
        -DTARGET_DIR=${RUST_TARGET_DIR}
        -P "${SYNC_SCRIPT}"

    WORKING_DIRECTORY "${RUST_ROOT}"
    BYPRODUCTS "${RUST_LIB}" "${RUST_APPLICATION_HOST_LIB}"
    VERBATIM
)

if(TARGET vrf_lib_submodule)
    add_dependencies(rust-workspace-build vrf_lib_submodule)
endif()

# --- Imported Library ---

add_library(rustaxa-bridge STATIC IMPORTED GLOBAL)
add_dependencies(rustaxa-bridge rust-workspace-build)

add_library(rustaxa-application-host-bridge STATIC IMPORTED GLOBAL)
add_dependencies(rustaxa-application-host-bridge rust-workspace-build)

set(RUSTAXA_BRIDGE_LINK_LIBRARIES
    gmp::gmp
    mpfr::mpfr
)

if(UNIX AND NOT APPLE)
    list(APPEND RUSTAXA_BRIDGE_LINK_LIBRARIES pthread dl)
elseif(APPLE)
    list(APPEND RUSTAXA_BRIDGE_LINK_LIBRARIES pthread)
endif()

set_target_properties(rustaxa-bridge PROPERTIES
    IMPORTED_LOCATION "${RUST_LIB}"
    INTERFACE_INCLUDE_DIRECTORIES
        "${BRIDGE_INCLUDE_DIR};${PROJECT_SOURCE_DIR}/libraries/core_libs/consensus/include"
    INTERFACE_LINK_LIBRARIES "${RUSTAXA_BRIDGE_LINK_LIBRARIES}"
)

set_target_properties(rustaxa-application-host-bridge PROPERTIES
    IMPORTED_LOCATION "${RUST_APPLICATION_HOST_LIB}"
    INTERFACE_INCLUDE_DIRECTORIES "${BRIDGE_INCLUDE_DIR}"
    INTERFACE_LINK_LIBRARIES "rustaxa-bridge"
)

# The CXX callbacks live in the application-host archive, but Rust codegen can
# still place unused host-adapter functions in a base-staticlib CGU selected by
# a leaf bridge call. Discard those unreferenced function sections so Unix leaf
# consumers do not acquire application-host symbol requirements.
if(UNIX AND NOT APPLE)
    set_property(TARGET rustaxa-bridge APPEND PROPERTY INTERFACE_LINK_OPTIONS "-Wl,--gc-sections")
endif()

add_library(Rustaxa::bridge ALIAS rustaxa-bridge)
add_library(Rustaxa::application-host-bridge ALIAS rustaxa-application-host-bridge)
