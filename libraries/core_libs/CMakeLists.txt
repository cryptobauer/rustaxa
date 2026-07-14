add_subdirectory(network/rpc)

set(RUSTAXA_VOTE_MANAGER_SHIM_ENABLED OFF)
if(RUSTAXA_ENABLE_VERIFIED_VOTES AND
   (NOT RUSTAXA_ENABLE_STORAGE OR NOT RUSTAXA_ENABLE_FINAL_CHAIN OR
    NOT RUSTAXA_ENABLE_PROPOSED_BLOCKS OR NOT RUSTAXA_ENABLE_SLASHING_MANAGER))
    message(FATAL_ERROR
            "RUSTAXA_ENABLE_VERIFIED_VOTES requires RUSTAXA_ENABLE_STORAGE, "
            "RUSTAXA_ENABLE_FINAL_CHAIN, RUSTAXA_ENABLE_PROPOSED_BLOCKS, and "
            "RUSTAXA_ENABLE_SLASHING_MANAGER")
endif()
if(RUSTAXA_ENABLE_VERIFIED_VOTES)
    set(RUSTAXA_VOTE_MANAGER_SHIM_ENABLED ON)
endif()

file(GLOB_RECURSE CONSENSUS_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.hpp)
file(GLOB_RECURSE CONSENSUS_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.cpp)
list(FILTER CONSENSUS_HEADERS EXCLUDE REGEX "/shims/")
list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/shims/")
if(RUSTAXA_ENABLE)
    # The Rust-mode TransactionManager and its native queue runtime are fully
    # standalone. Keep the untouched legacy implementations only in pure C++
    # reference builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/transaction/transaction_manager\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/transaction/transaction_queue\\.cpp$")
endif()
if(RUSTAXA_ENABLE_FINAL_CHAIN)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_FINAL_CHAIN requires RUSTAXA_ENABLE")
    endif()

    # The Rust-mode FinalChain overlay is fully standalone. Keep the untouched
    # legacy implementation only in pure C++ reference builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/final_chain/final_chain\\.cpp$")
endif()
if(RUSTAXA_VOTE_MANAGER_SHIM_ENABLED)
    # The Rust-mode VoteManager overlay is fully standalone. Keep the untouched
    # legacy implementation only when the overlay is not enabled.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/vote_manager/vote_manager\\.cpp$")
endif()

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
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE requires RUSTAXA_ENABLE_STORAGE for Rust DagManager storage runtime wiring")
    endif()

    target_link_libraries(core_libs PUBLIC rustaxa-bridge RocksDB::rocksdb)

    # In Rust mode, inject a header overlay that remaps legacy DagManager
    # to DagManagerOld and exposes a standalone DagManager shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_manager_shim/src/dag_manager_shim.cpp)

    # In Rust mode, inject a header overlay that remaps legacy DagBlockProposer
    # to DagBlockProposerOld and exposes a standalone proposer shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_block_proposer_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_block_proposer_shim/src/dag_block_proposer_shim.cpp)

    # In Rust mode, inject the standalone Rust-backed TransactionManager facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/transaction_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/transaction_manager_shim/src/transaction_manager_shim.cpp)

    # In Rust mode, inject a header overlay that remaps legacy Dag/PivotTree
    # to DagOld/PivotTreeOld and exposes standalone Rust-backed facades.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/dag_shim/src/dag_shim.cpp)

    # Compile the legacy implementation under old symbols only.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/dag/dag.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "Dag=DagOld;PivotTree=PivotTreeOld"
    )

    # Compile legacy DagManager implementation as DagManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/dag/dag_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "DagManager=DagManagerOld"
    )

    # Compile legacy DagBlockProposer implementation as DagBlockProposerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/dag/dag_block_proposer.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "DagBlockProposer=DagBlockProposerOld"
    )

    # In Rust mode, inject a header overlay that remaps legacy KeyManager
    # to KeyManagerOld and exposes a forwarding KeyManager shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/key_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/key_manager_shim/src/key_manager_shim.cpp)

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
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/storage_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/storage_shim/src/storage_shim.cpp)

    # Compile the legacy implementation as DbStorageOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/storage/src/storage.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "DbStorage=DbStorageOld"
    )
endif()

if(RUSTAXA_ENABLE_GAS_PRICER)
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_GAS_PRICER requires RUSTAXA_ENABLE_STORAGE")
    endif()
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_GAS_PRICER requires RUSTAXA_ENABLE")
    endif()
    # In Rust GasPricer mode, inject a header overlay that remaps legacy
    # GasPricer to GasPricerOld and exposes a standalone GasPricer shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/gas_pricer_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/gas_pricer_shim/src/gas_pricer_shim.cpp)

    # Compile the legacy implementation as GasPricerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/transaction/gas_pricer.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "GasPricer=GasPricerOld"
    )
endif()

if(RUSTAXA_ENABLE_SLASHING_MANAGER)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_SLASHING_MANAGER requires RUSTAXA_ENABLE")
    endif()
    if(NOT RUSTAXA_ENABLE_FINAL_CHAIN)
        message(FATAL_ERROR "RUSTAXA_ENABLE_SLASHING_MANAGER requires RUSTAXA_ENABLE_FINAL_CHAIN")
    endif()
    if(NOT RUSTAXA_ENABLE_GAS_PRICER)
        message(FATAL_ERROR "RUSTAXA_ENABLE_SLASHING_MANAGER requires RUSTAXA_ENABLE_GAS_PRICER")
    endif()

    # In Rust SlashingManager mode, inject a header overlay that remaps legacy
    # SlashingManager to SlashingManagerOld and exposes a standalone Rust-planning facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/slashing_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/slashing_manager_shim/src/slashing_manager_shim.cpp)

    # Compile the legacy SlashingManager implementation as SlashingManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/slashing_manager/slashing_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "SlashingManager=SlashingManagerOld"
    )
endif()

if(RUSTAXA_ENABLE_FINAL_CHAIN)
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_FINAL_CHAIN requires RUSTAXA_ENABLE_STORAGE")
    endif()

    # In Rust final-chain mode, inject the standalone FinalChain shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/final_chain_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/final_chain_shim/src/final_chain_shim.cpp)
endif()

if(RUSTAXA_ENABLE_REWARDS_STATS)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_REWARDS_STATS requires RUSTAXA_ENABLE")
    endif()
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_REWARDS_STATS requires RUSTAXA_ENABLE_STORAGE")
    endif()
    if(NOT RUSTAXA_ENABLE_FINAL_CHAIN)
        message(FATAL_ERROR "RUSTAXA_ENABLE_REWARDS_STATS requires RUSTAXA_ENABLE_FINAL_CHAIN")
    endif()

    # In Rust rewards-stats mode, inject a header overlay that remaps legacy
    # rewards::Stats to rewards::StatsOld and exposes a Rust-backed facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/rewards_stats_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/rewards_stats_shim/src/rewards_stats_shim.cpp)

    # Compile the legacy implementation as rewards::StatsOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/rewards/rewards_stats.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "Stats=StatsOld"
    )
endif()

if(RUSTAXA_ENABLE_SORTITION_PARAMS)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_SORTITION_PARAMS requires RUSTAXA_ENABLE")
    endif()

    # In Rust sortition-params mode, inject a header overlay that remaps legacy
    # SortitionParamsManager to SortitionParamsManagerOld and exposes a
    # standalone SortitionParamsManager shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/sortition_params_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/sortition_params_manager_shim/src/sortition_params_manager_shim.cpp)

    # Compile the legacy implementation as SortitionParamsManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/dag/sortition_params_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "SortitionParamsManager=SortitionParamsManagerOld"
    )
endif()

if(RUSTAXA_ENABLE_PBFT_CHAIN)
    if(NOT RUSTAXA_ENABLE_STORAGE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_PBFT_CHAIN requires RUSTAXA_ENABLE_STORAGE")
    endif()

    # In Rust PBFT-chain mode, inject a header overlay that remaps legacy
    # PbftChain to PbftChainOld and exposes a standalone Rust-backed facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pbft_chain_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pbft_chain_shim/src/pbft_chain_shim.cpp)

    # Compile the legacy implementation as PbftChainOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/pbft/pbft_chain.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "PbftChain=PbftChainOld"
    )
endif()

if(RUSTAXA_ENABLE_PROPOSED_BLOCKS)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_PROPOSED_BLOCKS requires RUSTAXA_ENABLE")
    endif()

    # In Rust proposed-blocks mode, inject a header overlay that remaps legacy
    # ProposedBlocks to ProposedBlocksOld and exposes a standalone shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/proposed_blocks_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/proposed_blocks_shim/src/proposed_blocks_shim.cpp)

    # Compile the legacy implementation as ProposedBlocksOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/pbft/proposed_blocks.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "ProposedBlocks=ProposedBlocksOld"
    )
endif()

if(RUSTAXA_ENABLE_VERIFIED_VOTES)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_VERIFIED_VOTES requires RUSTAXA_ENABLE")
    endif()
    # In Rust verified-votes mode, inject a header overlay that remaps legacy
    # VerifiedVotes to VerifiedVotesOld and exposes a standalone shim facade.
    target_include_directories(core_libs BEFORE PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/verified_votes_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/verified_votes_shim/src/verified_votes_shim.cpp)

    # Compile the legacy implementation as VerifiedVotesOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/vote_manager/verified_votes.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "VerifiedVotes=VerifiedVotesOld"
    )
endif()

if(RUSTAXA_VOTE_MANAGER_SHIM_ENABLED)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "VoteManager shim requires RUSTAXA_ENABLE")
    endif()

    # In storage-backed Rust verified-votes mode, inject the standalone
    # Rust-compatible VoteManager shim facade for every consumer.
    target_include_directories(core_libs BEFORE PUBLIC
                             ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/vote_manager_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/vote_manager_shim/src/vote_manager_shim.cpp)
endif()

if(RUSTAXA_ENABLE_PILLAR_VOTES)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_PILLAR_VOTES requires RUSTAXA_ENABLE")
    endif()

    # In Rust pillar-votes mode, inject a header overlay that remaps legacy
    # PillarChainManager to PillarChainManagerOld and exposes a standalone
    # PillarChainManager shim facade.
    target_include_directories(core_libs BEFORE PUBLIC
                             ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pillar_chain_manager_shim/include)
    target_sources(core_libs PRIVATE
                  ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pillar_chain_manager_shim/src/pillar_chain_manager_shim.cpp)

    # Compile the legacy implementation as PillarChainManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/pillar_chain/pillar_chain_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "PillarChainManager=PillarChainManagerOld"
    )
endif()

if(RUSTAXA_ENABLE_PILLAR_VOTES OR RUSTAXA_ENABLE_PROPOSED_BLOCKS)
    if(NOT RUSTAXA_ENABLE)
        message(FATAL_ERROR "RUSTAXA_ENABLE_PILLAR_VOTES or RUSTAXA_ENABLE_PROPOSED_BLOCKS requires RUSTAXA_ENABLE")
    endif()

    # In Rust pillar-votes/proposed-blocks mode, inject a header overlay that remaps
    # legacy PbftManager to PbftManagerOld and exposes a standalone facade owned by the shim.
    target_include_directories(core_libs BEFORE PUBLIC
                             ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pbft_manager_shim/include)
    target_sources(core_libs PRIVATE
                  ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp
                  ${CMAKE_CURRENT_SOURCE_DIR}/consensus/shims/pbft_manager_shim/src/pbft_manager_shim.cpp)

    # Compile legacy PbftManager as PbftManagerOld.
    set_source_files_properties(
        ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/pbft/pbft_manager.cpp
        PROPERTIES
        COMPILE_DEFINITIONS "PbftManager=PbftManagerOld"
    )

endif()

install(TARGETS core_libs
  ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
)
