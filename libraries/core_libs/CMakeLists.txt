add_subdirectory(network/rpc)

file(GLOB_RECURSE CONSENSUS_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.hpp)
file(GLOB_RECURSE CONSENSUS_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/consensus/*.cpp)
if(RUSTAXA_ENABLE)
  # Keep the main-only finalization execution leaf explicit for configured Rust builds.
  list(APPEND CONSENSUS_SOURCES
       ${CMAKE_CURRENT_SOURCE_DIR}/consensus/src/application/finalization_transaction_order.cpp)
  list(REMOVE_DUPLICATES CONSENSUS_SOURCES)
endif()
list(FILTER CONSENSUS_HEADERS EXCLUDE REGEX "/shims/")
list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/shims/")
if(RUSTAXA_ENABLE)
    # The concrete process, signer, transport, and execution host leaves belong
    # to the application shell. They are compiled by the app target so leaf
    # rustaxa-bridge consumers do not inherit full-node linkage requirements.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX
         "/consensus/src/application/consensus_host_ports\\.cpp$")

    # Rust mode no longer exposes the standalone DAG/PivotTree graph facade. Its
    # DAG manager, DAG block proposer, TransactionManager, and native queue
    # runtime overlays are fully standalone. KeyManager and sortition ownership
    # are native and have no Rust-mode C++ manager facade.
    # Keep the untouched legacy implementations only in pure C++ reference builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/dag/dag\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/dag/dag_manager\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/dag/dag_block_proposer\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/key_manager/key_manager\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/transaction/transaction_manager\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/transaction/transaction_queue\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/dag/sortition_params_manager\\.cpp$")

    # Native verified-vote admission owns Rust-mode slashing planning; no
    # SlashingManager overlay remains. Keep the untouched legacy implementation
    # in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/transaction/gas_pricer\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/slashing_manager/slashing_manager\\.cpp$")

    # The Rust-mode FinalChain overlay is fully standalone. Rewards-stat
    # planning is owned by native FinalChain and has no Rust-mode C++ facade.
    # Keep the untouched legacy implementations only in pure C++ reference builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/final_chain/final_chain\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/rewards/rewards_stats\\.cpp$")

    # The application root owns Rust-mode PBFT vote generation, admission,
    # progress, rewards, and persistence. Keep the untouched legacy VoteManager
    # implementation only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/vote_manager/vote_manager\\.cpp$")

    # Native verified-vote ownership is consumed directly through application
    # tasks. Keep the untouched legacy implementation only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/vote_manager/verified_votes\\.cpp$")

    # Native PBFT service ownership replaces the Rust-mode C++ ProposedBlocks
    # facade. Keep the untouched legacy implementation only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/pbft/proposed_blocks\\.cpp$")

    # Native pillar ownership is application-root internal. Keep the untouched
    # legacy manager and vote runtime only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/pillar_chain/pillar_chain_manager\\.cpp$")
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/pillar_chain/pillar_votes\\.cpp$")

    # Rust mode has no PbftChain facade; the native application root owns its
    # state. Keep the untouched legacy implementation only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/pbft/pbft_chain\\.cpp$")

    # Native PBFT orchestration is hosted privately by ConsensusApplication.
    # Keep the untouched legacy PbftManager implementation only in pure C++ builds.
    list(FILTER CONSENSUS_SOURCES EXCLUDE REGEX "/consensus/src/pbft/pbft_manager\\.cpp$")
endif()

file(GLOB_RECURSE NETWORK_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/network/*.hpp)
file(GLOB_RECURSE NETWORK_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/network/*.cpp)
if(RUSTAXA_ENABLE)
    # PBFT sync lifecycle is application-owned in Rust mode. The legacy shared
    # state remains source-selected only for the untouched pure-C++ composition.
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/shared_states/pbft_syncing_state\\.cpp$")

    # Rust packet adapters inherit only the transport leaf base. The three
    # manager-shaped compatibility bases remain available solely to the
    # untouched pure-C++ handler hierarchy.
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/common/ext_syncing_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/common/ext_votes_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/common/ext_pillar_vote_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/vote_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/pillar_vote_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/pillar_votes_bundle_packet_handler\\.cpp$")

    # Native packet admission owns decoding, verification, queue mutation, and
    # egress selection. Keep the untouched manager-based handlers only in the
    # pure-C++ reference composition.
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/transaction_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/get_dag_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/dag_block_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/dag_sync_packet_handler\\.cpp$")
    # Native PBFT sync egress owns request validation, storage reads, reward-vote
    # attachment, and proposed-block selection. Keep the untouched latest and v5
    # handler implementations only in pure C++ reference builds.
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/get_pbft_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/pbft_blocks_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/pbft_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/v4/get_pbft_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/get_next_votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/interface/get_pillar_votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/interface/get_pillar_votes_bundle_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/latest/get_pillar_votes_bundle_packet_handler\\.hpp$")
    # Status and sync coordination are standalone native adapters in Rust mode.
    # Keep the untouched interface/latest status implementations pure-C++ only.
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/interface/sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/latest/status_packet_handler\\.cpp$")
else()
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/consensus_transport_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/consensus_transport_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/(vote|votes_bundle|pillar_vote|pillar_votes_bundle)_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/(vote|votes_bundle|pillar_vote|pillar_votes_bundle)_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/transaction_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/transaction_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/get_dag_sync_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/get_dag_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/dag_block_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/dag_block_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/dag_sync_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/dag_sync_packet_handler\\.cpp$")
    # The Rust transport adapter depends on the main-only consensus network API.
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/get_pbft_sync_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/get_pbft_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/pbft_sync_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/pbft_sync_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/pbft_blocks_bundle_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/pbft_blocks_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/get_next_votes_bundle_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/get_next_votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/get_pillar_votes_bundle_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/get_pillar_votes_bundle_packet_handler\\.cpp$")
    list(FILTER NETWORK_HEADERS EXCLUDE REGEX
         "/network/include/network/tarcap/packets_handlers/rust/status_packet_handler\\.hpp$")
    list(FILTER NETWORK_SOURCES EXCLUDE REGEX
         "/network/src/tarcap/packets_handlers/rust/status_packet_handler\\.cpp$")
endif()

# GraphQL
include(${PROJECT_SOURCE_DIR}/CMakeModules/cpp_graphql_gen.cmake)
file(GLOB_RECURSE NETWORK_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/network/*.h)

file(GLOB_RECURSE STORAGE_HEADERS ${CMAKE_CURRENT_SOURCE_DIR}/storage/*.hpp)
file(GLOB_RECURSE STORAGE_SOURCES ${CMAKE_CURRENT_SOURCE_DIR}/storage/*.cpp)
if(RUSTAXA_ENABLE)
    # Native application-owned rustaxa-storage is the sole Rust-mode consensus database owner.
    # Legacy DbStorage and migration implementations remain untouched and are selected only in pure-C++ builds.
    set(STORAGE_SOURCES "")
endif()

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
    target_link_libraries(core_libs PUBLIC rustaxa-bridge RocksDB::rocksdb)
endif()

if(RUSTAXA_ENABLE_NETWORK)
    target_include_directories(core_libs PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/network_shim/include)
    target_sources(core_libs PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/network_shim/src/taraxa_capability_shim.cpp)
endif()

install(TARGETS core_libs
  ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
)
