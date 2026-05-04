add_subdirectory(network/rpc)

file(GLOB_RECURSE CONSENSUS_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.hpp)
file(GLOB_RECURSE CONSENSUS_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.cpp)

file(GLOB_RECURSE NETWORK_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/network/*.hpp)
file(GLOB_RECURSE NETWORK_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/network/*.cpp)

# GraphQL
include(${PROJECT_SOURCE_DIR}/CMakeModules/cpp_graphql_gen.cmake)
file(GLOB_RECURSE NETWORK_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/network/*.h)

file(GLOB_RECURSE STORAGE_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/storage/*.hpp)
file(GLOB_RECURSE STORAGE_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/storage/*.cpp)

file(GLOB_RECURSE REWARDS_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/rewards/*.hpp)
file(GLOB_RECURSE REWARDS_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/rewards/*.cpp)

set(HEADERS
  ${CONSENSUS_HEADERS}
  ${NETWORK_HEADERS}
  ${STORAGE_HEADERS}
  ${NODE_HEADERS}
  ${REWARDS_HEADERS}
)

set(SOURCES
  ${CONSENSUS_SOURCES}
  ${NETWORK_SOURCES}
  ${STORAGE_SOURCES}
  ${NODE_SOURCES}
  ${GRAPHQL_GENERATED_SOURCES}
  ${REWARDS_SOURCES}
)

add_library(core_libs ${SOURCES} ${HEADERS})
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/network/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/node/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/storage/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/rewards/include)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR})

# GraphQL
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/network/graphql/gen)
target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/network/graphql/include)

find_package(RocksDB REQUIRED)
target_link_libraries(core_libs PUBLIC
  cli
  pbft_block
  taraxa-evm
  p2p
  metrics
  Jsonrpccpp
  RocksDB::rocksdb

  # GraphQL
  cppgraphqlgen::graphqlservice
  cppgraphqlgen::graphqljson
)

if(TCMALLOC_LIB)
    target_link_libraries(core_libs INTERFACE ${TCMALLOC_LIB}) # MUST BE LINKED LAST!!!
endif()

if(RUSTAXA_ENABLE)
    target_link_libraries(core_libs PUBLIC rustaxa-bridge)

    # In Rust mode, inject a header overlay that remaps legacy KeyManager
    # to KeyManagerOld and exposes a forwarding KeyManager shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/key_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/key_manager_shim/src/key_manager_shim.cpp)

    # Compile the legacy implementation as KeyManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/key_manager/key_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "KeyManager=KeyManagerOld"
    )
endif()

if(RUSTAXA_ENABLE_STORAGE)
    # In Rust storage mode, inject a header overlay that remaps legacy DbStorage
    # to DbStorageOld and exposes a forwarding DbStorage shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/storage_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/storage_shim/src/storage_shim.cpp)

    # Compile the legacy implementation as DbStorageOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/storage/src/storage.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "DbStorage=DbStorageOld"
    )
endif()

if(RUSTAXA_ENABLE_FINAL_CHAIN)
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_FINAL_CHAIN requires RUSTAXA_ENABLE_STORAGE")
    endif()

    # In Rust final-chain mode, inject a header overlay that remaps legacy FinalChain
    # to FinalChainOld and exposes a forwarding FinalChain shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/final_chain_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/final_chain_shim/src/final_chain_shim.cpp)

    # Compile the legacy implementation as FinalChainOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/final_chain/final_chain.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "FinalChain=FinalChainOld"
    )
endif()

install(TARGETS core_libs
  ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
)
