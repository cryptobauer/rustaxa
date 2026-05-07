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

if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64|armv[0-9]+.*)$")
    set(BRIDGE_LOCALIZE_SYMBOLS
        "__gmpn_mul_1"
        "__gmpn_divrem_1"
    )
else()
    set(BRIDGE_LOCALIZE_SYMBOLS
        "__gmpn_add_n"
        "__gmpn_addmul_1"
        "__gmpn_bdiv_dbm1c"
        "__gmpn_com"
        "__gmpn_copyd"
        "__gmpn_copyi"
        "__gmpn_divexact_1"
        "__gmpn_divrem_1"
        "__gmpn_lshift"
        "__gmpn_lshiftc"
        "__gmpn_mod_34lsub1"
        "__gmpn_mul_1"
        "__gmpn_mul_basecase"
        "__gmpn_mullo_basecase"
        "__gmpn_rshift"
        "__gmpn_sqr_basecase"
        "__gmpn_sub_n"
        "__gmpn_submul_1"
    )
endif()

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
        configure_file(\"\${HEADER}\" \"${BRIDGE_INCLUDE_DIR}/rustaxa-bridge/\${FNAME}\" COPYONLY)
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
        "${CARGO_EXE}" build ${CARGO_MODE_ARGS} --target-dir "${RUST_TARGET_DIR}" -p rustaxa-bridge

    COMMAND ${CMAKE_COMMAND} -P "${FILTER_SCRIPT}"

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
    INTERFACE_INCLUDE_DIRECTORIES "${BRIDGE_INCLUDE_DIR}"
    INTERFACE_LINK_LIBRARIES "${RUSTAXA_BRIDGE_LINK_LIBRARIES}"
)

add_library(Rustaxa::bridge ALIAS rustaxa-bridge)
