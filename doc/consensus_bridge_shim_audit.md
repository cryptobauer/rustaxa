# Consensus Bridge and Shim Audit

This is the Slice 0 deletion map for `doc/consensus_consolidation_plan.md`.
It records the bridge and shim surfaces that still touch consensus rewrite code,
classifies the ownership boundary, and names the condition for narrowing or
removing each item.

## Classification

| Classification | Meaning | Default action |
| --- | --- | --- |
| External boundary | C++ remains the near-term owner because the client is network/tarcap, EVM, RPC, app bootstrap, or tests. | Keep a minimal API and narrow it to client-specific methods. |
| C++ public compatibility facade | C++ class/API is still public but should delegate to Rust-owned internals in Rust mode. | Keep until callers migrate or the public C++ API is retired. |
| Internal Rust route | Logic is consensus/storage implementation detail and should not remain bridge-shaped once Rust callers can use native crates. | Move to native Rust modules and delete CXX bridge/shim access. |
| Obsolete scaffold | Compatibility helper exists only because earlier slices needed temporary wiring. | Delete in the owning cleanup slice. |

## Rust Bridge Modules

| Module | Main exported handles or constructors | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/storage.rs` | `BridgeStorage`, `BridgeStorageBatch`, `Bridge*StorageQueries`, `create_storage`, `create_*_storage_queries`, `create_storage_shim_batch` | `storage_shim`, storage conformance tests, consensus shims that bootstrap native Rust storage handles | C++ public compatibility facade | Delete broad `BridgeStorage` read/write methods and `BridgeStorageBatch` once storage shim callers use narrow Rust storage runtimes or native Rust crates directly. Keep only app/bootstrap creation until C++ `DbStorage` facade is retired. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi`, `create_consensus_query_api` | `network/consensus_query.hpp`, RPC/GraphQL, `plugin/light`, Rust tests | External boundary | Keep as the public query facade for RPC/GraphQL/light clients. Narrow remaining storage-backed reads into this facade, then remove direct external construction except approved app/bootstrap points. |
| `rust/crates/rustaxa-bridge/src/network.rs` | `BridgeConsensusNetworkApi`, `create_consensus_network_api`, ingress/planner/drain/report methods, `consensus_network_gossip_pbft_vote` | Latest tarcap handlers, `tests/rust/consensus/test_network_api.cpp` | External boundary | Queue-named bridge helpers are deleted. Keep narrowing the direct network/tarcap facade until only packet ingress, deterministic planners, gossip/send/sync/report effects, and result reporting remain. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | `BridgeFinalChain`, `BridgeFinalChainExecutionSession`, `BridgeConsensusExecutionApi`, `create_final_chain*`, `create_final_chain_execution_session`, `create_consensus_execution_api` | `final_chain_shim`, transaction manager runtime, consensus execution adapters | External boundary | Keep EVM/execution boundary while EVM remains out of scope. Move consensus fact reads to Rust FinalChain ports and delete bridge paths that only materialize C++ facts for Rust consensus. |
| `rust/crates/rustaxa-bridge/src/dag.rs` | `BridgeDagGraph`, `BridgeDagManagerRuntime` | `dag_shim`, `dag_manager_shim`, DAG tests | C++ public compatibility facade | Remove graph compatibility handles after DAG public callers stop needing C++ graph aliases. |
| `rust/crates/rustaxa-bridge/src/pbft_chain.rs` | `BridgePbftChain`, `create_pbft_chain*` | `pbft_chain_shim`, PBFT manager/runtime tests | C++ public compatibility facade | Delete once PBFT chain public C++ facade is no longer required or PBFT manager owns chain state natively in Rust. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | `BridgePbftManagerRuntime` | `pbft_manager_shim`, app bootstrap runtime creation | Internal Rust route | Runtime, state-action effect, proposal, and block-validation session handles are retired and owned by `BridgePbftManagerRuntime`. Keep only app bootstrap handle until PBFT manager C++ facade is retired. |
| `rust/crates/rustaxa-bridge/src/pbft_finalize.rs` | `BridgePbftFinalizationRuntimeSession`, finalization/resume sessions | PBFT manager/finalization shims and tests | Internal Rust route | Delete bridge sessions once PBFT finalization is invoked inside Rust consensus runtime rather than through C++ shim sessions. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | PBFT sync admission, egress, process-period, transaction-query, and cert-vote validation functions | `pbft_manager_shim`, PBFT sync bridge tests | Internal Rust route | Keep narrowing into `BridgePbftManagerRuntime` service methods. The standalone queue-drain CXX handle is retired; remaining functions disappear when PBFT sync processing is owned fully inside the Rust PBFT manager runtime. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_*` | Vote validation/generation/progress/ingress/event/payload helpers | Vote manager shim, network API tests, PBFT/vote tests | Internal Rust route | CXX vote pipeline/admission session handles are retired. Collapse remaining bridge helpers into native Rust vote pipeline modules and keep only network-facing vote payload/effect adapters until the network/tarcap API owns that boundary. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `BridgeVerifiedVotes`, `create_verified_votes_index`, storage attach | `verified_votes_shim`, `vote_manager_shim` | C++ public compatibility facade | Delete after vote manager no longer needs a C++ `VerifiedVotes` facade and Rust vote state attaches to storage internally. |
| `rust/crates/rustaxa-bridge/src/period_data_queue.rs` | Internal conversion helpers only; no exported CXX handle | `pbft_manager.rs` | Internal Rust route | Delete the helper module after PBFT manager runtime can construct period-data queue facts directly from native Rust payload models instead of C++ sidecars. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | `BridgeProposedBlocks`, `create_proposed_blocks_index*` | `proposed_blocks_shim`, `dag_manager_shim`, `vote_manager_shim` | C++ public compatibility facade | Delete after proposed-block tracking is part of Rust PBFT/DAG runtime and C++ no longer asks for metadata/materialized proposed blocks. |
| `rust/crates/rustaxa-bridge/src/rewards_stats.rs` | `BridgeRewardsStatsRuntime`, `create_rewards_stats_runtime` | `rewards_stats_shim`, finalization/reward tests | C++ public compatibility facade | Delete C++ facade once rewards stats publication and storage writes are driven from Rust finalization. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | `BridgePillarChainStorage`, `create_pillar_chain_storage` | `storage_shim`, `pillar_chain_manager_shim` | C++ public compatibility facade | Keep as a narrow storage handle while pillar C++ facade exists. Delete after pillar chain storage access is native Rust-owned. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | `BridgePillarVotes`, `create_pillar_votes_index` | `pillar_votes_shim`, period-data/vote paths | C++ public compatibility facade | Delete after pillar vote indexing/admission is moved into Rust pillar/PBFT runtime and no C++ index facade remains. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | `BridgeSortitionParamsManager`, `create_sortition_params_manager*` | `sortition_params_manager_shim`, query/RPC paths through storage | C++ public compatibility facade | Delete after sortition parameter persistence and query reads are native Rust consensus/storage APIs. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | Transaction RLP inspection and bridge DTO helpers | Transaction manager, period-data queue, tests | External boundary | Keep only wire/codec compatibility helpers needed at C++ network/RPC boundaries. Move internal transaction facts to `rustaxa-types`/native consensus. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | `BridgeTransactionManagerSidecar`, `BridgeTransactionManagerRuntime`, admission execution/session helpers | `transaction_manager_shim`, RPC submission paths, tests | C++ public compatibility facade | Delete sidecar/runtime bridge after transaction manager public C++ facade is retired or all admission/packing paths are native Rust. Keep external EVM/final-chain callbacks as a minimal API. |
| `rust/crates/rustaxa-bridge/src/transaction_queue.rs` | `BridgeTransactionQueue`, `create_transaction_queue` | `transaction_queue_shim` | C++ public compatibility facade | Delete after queue ownership moves fully to Rust transaction manager and C++ queue facade is no longer constructed. |
| `rust/crates/rustaxa-bridge/src/gas_pricer.rs` | `BridgeGasPricer`, `create_gas_pricer*`, bid/update methods | `gas_pricer_shim`, transaction/RPC gas estimation | C++ public compatibility facade | Delete after gas pricing history and query are Rust-owned behind the transaction/final-chain runtime API. The CXX-only storage init method has been removed; storage restoration is construction-time only. |
| `rust/crates/rustaxa-bridge/src/slashing.rs` | `BridgeSlashingProofPlanner`, `create_slashing_proof_planner` | `slashing_manager_shim` | C++ public compatibility facade | Delete after slashing proof planning is invoked by Rust consensus runtime instead of C++ manager facade. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | VDF bridge helpers | VDF C++ integration/tests | External boundary | Keep until VDF boundary is explicitly folded into native Rust or a dedicated external VDF API. |

## Exported CXX Bridge Handles

This table is the per-handle inventory for `type Bridge*` declarations in `rust/crates/rustaxa-bridge/src/ffi.rs`.

| Handle | Implementing module | Current consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC/GraphQL via `network/consensus_query.hpp`, light plugin, Rust tests | External boundary | Public query clients use one minimal facade and direct storage construction is limited to API construction points. |
| `BridgeConsensusNetworkApi` | `network.rs` | Latest tarcap packet handlers, network API tests | External boundary | Queue helpers are deleted. Keep the minimal network facade until PBFT vote gossip/effect execution can be narrowed further or absorbed by a transport-specific API. |
| `BridgeConsensusExecutionApi` | `final_chain.rs` | Consensus execution/EVM adapters | External boundary | EVM/StateAPI boundary is replaced or execution facts move into a dedicated Rust execution API. |
| `BridgeFinalChain` | `final_chain.rs` | `final_chain_shim`, transaction manager/finalization adapters | External boundary | Consensus fact reads no longer materialize through C++; EVM-only execution remains behind a thinner API. |
| `BridgeFinalChainExecutionSession` | `final_chain.rs` | FinalChain execution shim/tests | External boundary | Execution session is replaced by the dedicated EVM/execution adapter API. |
| `BridgeStorage` | `storage.rs` | `storage_shim`, storage/query/runtime constructors, bridge tests | C++ public compatibility facade | Broad storage facade is replaced by native Rust storage runtimes or narrow bootstrap-only handles. |
| `BridgeStorageBatch` | `storage.rs` | `storage_shim`, `rewards_stats.rs`, storage FFI | Internal Rust route | Rust owns write-batch lifecycle natively and no C++ consensus/storage caller passes bridge batches. |
| `BridgePbftVoteStorageQueries` | `storage.rs` | Storage shim/tests, PBFT/vote bridge tests | C++ public compatibility facade | Vote storage reads move behind Rust vote/PBFT runtime ports. |
| `BridgePbftStorageQueries` | `storage.rs` | PBFT chain/manager/finalization tests and bridge helpers | C++ public compatibility facade | PBFT storage reads move behind native Rust runtime ports. |
| `BridgeMetadataStorageQueries` | `storage.rs` | FinalChain, transaction manager, rewards stats tests/helpers | C++ public compatibility facade | Metadata reads move behind native Rust storage/runtime ports. |
| `BridgeDagStorageQueries` | `storage.rs` | DAG/finalization tests and bridge helpers | C++ public compatibility facade | DAG reads move behind native Rust DAG runtime/storage ports. |
| `BridgeTransactionStorageQueries` | `storage.rs` | Transaction manager, DAG, PBFT sync/finalization tests | C++ public compatibility facade | Transaction storage reads move behind native Rust transaction/PBFT runtime ports. |
| `BridgeFinalChainStorageQueries` | `storage.rs` | FinalChain/query compatibility | C++ public compatibility facade | Final-chain storage reads move behind native Rust FinalChain/query APIs. |
| `BridgePeriodStorageQueries` | `storage.rs` | PBFT sync/finalization tests and query helpers | C++ public compatibility facade | Period-data reads move behind native Rust PBFT/finalization runtime ports. |
| `BridgeDagGraph` | `dag.rs` | DAG shim/tests | C++ public compatibility facade | C++ DAG graph aliases stop being public API. |
| `BridgeDagManagerRuntime` | `dag.rs` | `dag_manager_shim` | C++ public compatibility facade | C++ `DagManager` facade is retired or narrowed to an external API. |
| `BridgePbftChain` | `pbft_chain.rs` | `pbft_chain_shim`, PBFT tests | C++ public compatibility facade | PBFT chain state is private to Rust PBFT manager/runtime. |
| `BridgePbftFinalizationRuntimeSession` | `pbft_finalize.rs` | PBFT finalization paths/tests | Internal Rust route | Finalization is invoked inside Rust PBFT runtime rather than via CXX sessions. |
| `BridgePbftManagerRuntime` | `pbft_manager.rs` | App bootstrap, `pbft_manager_shim` | Internal Rust route | PBFT manager C++ orchestration is collapsed into Rust runtime. |
| `BridgePbftVoteValidationRuntime` | `pbft_vote_validation.rs` | Vote manager/tests | Internal Rust route | Vote validation is private Rust vote runtime behavior. |
| `BridgeVerifiedVotes` | `verified_votes.rs` | `verified_votes_shim`, `vote_manager_shim` | C++ public compatibility facade | Verified vote state is private Rust vote-manager state. |
| `BridgeProposedBlocks` | `proposed_blocks.rs` | `proposed_blocks_shim`, DAG/vote manager shims | C++ public compatibility facade | Proposed-block tracking is private Rust PBFT/DAG runtime state. |
| `BridgeRewardsStatsRuntime` | `rewards_stats.rs` | `rewards_stats_shim`, storage shim batch append | C++ public compatibility facade | Rewards stats writes/reads are driven from Rust finalization without C++ facade/batch passing. |
| `BridgePillarChainStorage` | `pillar_chain.rs` | `storage_shim`, `pillar_chain_manager_shim` | C++ public compatibility facade | Pillar chain storage is native Rust-owned. |
| `BridgePillarVotes` | `pillar_votes.rs` | `pillar_votes_shim` and period-data/vote paths | C++ public compatibility facade | Pillar vote indexing/admission is native Rust runtime state. |
| `BridgeSortitionParamsManager` | `sortition.rs` | `sortition_params_manager_shim` | C++ public compatibility facade | Sortition params persistence/query is native Rust storage/query behavior. |
| `BridgeTransactionQueue` | `transaction_queue.rs` | `transaction_queue_shim` | C++ public compatibility facade | Transaction queue is private Rust transaction-manager state. |
| `BridgeTransactionManagerSidecar` | `transaction_manager.rs` | `transaction_manager_shim` | C++ public compatibility facade | Transaction sidecar materialization is removed from C++ API. |
| `BridgeTransactionManagerRuntime` | `transaction_manager.rs` | `transaction_manager_shim`, app/bootstrap | C++ public compatibility facade | Transaction admission/packing runs behind native Rust runtime and minimal external submission API. |
| `BridgeTransactionManagerAdmissionExecution` | `transaction_manager.rs` | Transaction manager admission/EVM adapter | External boundary | EVM execution callbacks are isolated in a dedicated external API. |
| `BridgeGasPricer` | `gas_pricer.rs` | `gas_pricer_shim` | C++ public compatibility facade | Gas pricing is native Rust query/runtime behavior. The exported CXX surface is limited to construction, bid, pool-aware bid, and finalized-block update. |
| `BridgeSlashingProofPlanner` | `slashing.rs` | `slashing_manager_shim` | C++ public compatibility facade | Slashing planning runs inside Rust consensus runtime. |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `dag_block_proposer_shim` | Rust retry state for DAG proposal attempts | DAG manager/proposer code | C++ public compatibility facade | Delete when DAG proposal planning lives fully inside Rust DAG runtime. |
| `dag_manager_shim` | Rust DAG manager runtime behind C++ `DagManager` API | App/consensus code, DAG tests | C++ public compatibility facade | Remove `DagManagerOld` forwarding and C++ graph materialization when DAG manager callers can use Rust runtime or a thinner public facade. |
| `dag_shim` | C++ DAG facade over legacy graph aliases | DAG manager and DAG tests | C++ public compatibility facade | Delete after DAG graph types no longer leak through public C++ API. |
| `final_chain_shim` | Rust FinalChain runtime behind C++ FinalChain API | App, PBFT manager, transaction manager, RPC/EVM execution | External boundary | Keep EVM execution adapter; remove consensus fact/materialization methods after Rust consensus consumes FinalChain ports directly. |
| `gas_pricer_shim` | Gas price oracle facade | Transaction/RPC gas price paths | C++ public compatibility facade | Delete after gas price API is native Rust and external RPC sees only a narrow query method. |
| `key_manager_shim` | Key manager compatibility | App/bootstrap/key manager users | External boundary | Keep until key ownership is redesigned; not a consensus-internal deletion target. |
| `pbft_chain_shim` | PBFT chain facade | PBFT manager and tests | C++ public compatibility facade | Delete after PBFT chain state becomes private to Rust PBFT manager/runtime. |
| `pbft_manager_shim` | PBFT manager Rust runtime facade | App bootstrap and consensus loop | Internal Rust route | Collapse into native Rust PBFT runtime and remove C++ orchestration once network/EVM/storage external APIs are thin. |
| `pillar_chain_manager_shim` | Pillar chain manager compatibility | App/consensus pillar paths | C++ public compatibility facade | Delete after pillar chain runtime/storage is native Rust-owned. |
| `pillar_votes_shim` | Pillar vote index/admission facade | Pillar vote processing and tests | C++ public compatibility facade | Delete after pillar vote pipeline is native Rust. |
| `proposed_blocks_shim` | Proposed block tracking facade | DAG manager, vote manager, PBFT paths | C++ public compatibility facade | Delete after proposed-block tracking is folded into Rust PBFT/DAG runtime. |
| `rewards_stats_shim` | Rewards statistics facade | Finalization/rewards tests | C++ public compatibility facade | Delete after Rust finalization owns rewards stats writes/reads directly. |
| `slashing_manager_shim` | Slashing proof planner facade | Slashing manager users | C++ public compatibility facade | Delete after slashing planning runs inside Rust consensus runtime. |
| `sortition_params_manager_shim` | Sortition parameter storage facade | DAG/sortition, query paths | C++ public compatibility facade | Delete after sortition parameters are exposed through Rust storage/query APIs only. |
| `storage_shim` | `DbStorage` Rust-mode overlay and Rust storage owner | App, consensus shims, storage tests | C++ public compatibility facade | Delete broad storage facade after all C++ consensus callers stop using `DbStorage`; keep only external app/admin bootstrap if needed. |
| `transaction_manager_shim` | Transaction manager runtime/sidecar facade | App, RPC submission, PBFT packing | C++ public compatibility facade | Delete after transaction admission/packing/public submission API is native Rust with EVM boundary adapters. |
| `transaction_queue_shim` | Transaction queue facade | Transaction manager and tests | C++ public compatibility facade | Delete after queue is private Rust transaction-manager state. |
| `verified_votes_shim` | Verified votes compatibility facade | Vote manager shim | C++ public compatibility facade | Delete after vote manager uses Rust vote state directly. |
| `vote_manager_shim` | Vote manager Rust runtime facade | PBFT manager, DAG/proposed blocks, network vote paths | Internal Rust route | Collapse into Rust PBFT/vote runtime. Keep only external network adapters until network/tarcap API is complete. |

## Required Closeout Checks

Run these after each consolidation slice that touches bridge/shim code. The expected result is either empty output or only
entries listed in this audit as still-live compatibility/external boundaries.

```bash
rg -n '\b[A-Za-z_][A-Za-z0-9_]*Old::[A-Za-z_][A-Za-z0-9_]*\s*\(' \
  libraries/core_libs/consensus/shims -g'*.cpp' -g'*.hpp'
rg -n 'consensus_network_queue_' \
  rust/crates/rustaxa-bridge/src rust/crates/rustaxa-consensus/src libraries/core_libs tests/rust \
  -g'*.rs' -g'*.cpp' -g'*.hpp'
rg -n 'create_consensus_query_api\([^\n]*rustStorage\(\)' \
  libraries rust tests -g'*.cpp' -g'*.hpp' -g'*.rs'
rg -n '\bBridgeStorage\b' rust/crates/rustaxa-consensus -g'*.rs'
rg -n '\brustBatchId\b|BridgeStorageBatch|create_storage_shim_batch|storage_shim_.*batch' \
  libraries rust tests/rust -g'*.cpp' -g'*.hpp' -g'*.rs'
scripts/rewrite_bridge_inventory_guard.sh
rg -n '^mod [a-z0-9_]+;' rust/crates/rustaxa-bridge/src/lib.rs
rg -n '^\s*type Bridge[A-Za-z0-9_]+;' rust/crates/rustaxa-bridge/src/ffi.rs
```

Current snapshot after DAG proposer-session cursor consolidation:

- `Old::` forwarding remains in `dag_manager_shim` and `dag_shim`.
- `vote_manager_shim::setNetwork` writes inherited protected state directly and no longer forwards to
  `VoteManagerOld::setNetwork`.
- `dag_manager_shim::setNetwork` still forwards to `DagManagerOld::setNetwork` as temporary compatibility debt because
  the shim-owned network pointer and the legacy base's private network pointer are distinct until DAG manager runtime
  consolidation removes inherited base-path reliance.
- `dag_manager_shim::getShared` and `getDagMutex` still forward to inherited `DagManagerOld` state with call-site TODOs;
  remove them only when DAG manager ownership/synchronization are shim- or Rust-owned instead of inherited from the
  legacy base. `getDagMutex` cannot simply return the existing shim-owned order mutex because Rust-mode
  `setDagBlockOrder` already locks that mutex internally after callers acquire `getDagMutex`, so that narrow swap would
  deadlock finalization.
- `transaction_manager_shim::getTransactionsMutex` no longer forwards to `TransactionManagerOld`; the shim method returns
  the same inherited mutex through `TransactionManagerRustShimAccess`. The lock itself remains temporary inherited-state
  compatibility debt until transaction lifecycle synchronization moves into the Rust transaction runtime.
- `consensus_network_queue_*` no longer remains in bridge, FFI, latest tarcap network code, Rust consensus network API,
  or network API tests. Keep the closeout check above as a regression guard with empty output expected.
- Remaining live network effect execution is PBFT vote gossip through `consensus_network_gossip_pbft_vote` and
  `drain_work` / `report_effect_results` while tarcap owns peer filtering, packet wrapping, and transport.
- Direct public query API construction from `rustStorage()` remains at `network/consensus_query.hpp`, which is the approved
  helper construction point after Slice 1. RPC/GraphQL and `plugin/light/src/light.cpp` route through that helper.
- `BridgeStorage` remains in bridge storage/query/runtime constructors, storage shim, Rust bridge tests, and shim-owned
  bootstrap points. Native `rustaxa-consensus` modules do not depend on `BridgeStorage`, `BridgeStorageBatch`, or
  bridge-shaped storage query handles.
- `BridgeStorageBatch` and `rustBatchId` remain storage-shim compatibility debt. They must not grow into new consensus
  production routes. `create_storage_shim_batch` is storage-shim-local, and `rustBatchId` no longer has code callsites.
- Storage-shim single-write compatibility methods for DAG block save/remove, status fields, PBFT manager fields/status,
  PBFT heads, own verified votes, 2t+1 vote bundles, extra reward votes, proposal-period DAG-level mappings, and
  cert-voted block writes/removal now stage typed `storage_shim_*` writes through `BridgeStorageBatch` and immediately
  commit the Rust-owned batch.
- Genesis-hash writes now use `storage_shim_set_genesis_hash`, a dedicated storage-shim API that preserves the
  `rustaxa-storage` write-if-empty behavior while avoiding the broad `BridgeStorage::set_genesis_hash` mutator from the
  C++ shim. The obsolete broad `BridgeStorage::set_genesis_hash` CXX export has been deleted; only the dedicated
  storage-shim helper remains. The storage conformance runner now uses that dedicated helper as well.
- Block-reward stats clearing now uses `storage_shim_clear_block_rewards_stats`, a dedicated storage-shim API that
  preserves the Rust storage aggregate delete and native batch commit while avoiding the broad
  `BridgeStorage::clear_block_rewards_stats` mutator from the C++ shim. The obsolete broad
  `BridgeStorage::clear_block_rewards_stats` CXX export has been deleted; only the dedicated storage-shim helper remains.
- The storage-shim direct-mutator cleanup tracked in Slice 4 is complete for the audited single-write and aggregate-clear
  compatibility paths. The no-caller broad `BridgeStorage` CXX mutators for block-reward stats, cert-voted-block
  removal, own-vote removal, extra-reward-vote removal, and 2t+1 vote replacement have been deleted. Remaining broad
  `BridgeStorage` mutators must be justified by live production bridge boundaries until the public `BridgeStorage`
  facade is retired.
- The no-caller broad `BridgeStorage::save_sortition_params_change` CXX mutator has been deleted. C++ `DbStorage`
  compatibility writes use the dedicated `storage_shim_save_sortition_params_change` batch appender, and Rust bridge
  tests seed sortition changes through native `rustaxa-storage` metadata writes.
- The last C++ test fixture caller of broad `BridgeStorage::save_extra_reward_vote` now seeds through
  `storage_shim_save_extra_reward_vote` and a Rust-owned storage-shim batch, so the broad CXX mutator has been deleted.
- The remaining test-only callers of broad `BridgeStorage::save_own_verified_vote` now seed through either
  `storage_shim_save_own_verified_vote` or native Rust PBFT vote persistence helpers, so the broad CXX mutator has been
  deleted.
- The remaining test-only callers of broad `BridgeStorage::persist_pbft_vote_progress` and
  `BridgeStorage::clear_own_verified_votes` now route through the narrower `BridgeVerifiedVotes` persistence facade, so
  those broad CXX storage methods have been deleted.
- The remaining test-only callers of broad `BridgeStorage::save_cert_voted_block_in_round` now route through either
  `storage_shim_save_cert_voted_block_in_round` or native Rust PBFT manager storage helpers, so the broad CXX storage
  method has been deleted.
- The storage conformance caller of broad `BridgeStorage::save_pbft_head` now routes through
  `storage_shim_save_pbft_head` and a Rust-owned storage-shim batch, so the broad CXX storage method has been deleted.
- The remaining callers of broad `BridgeStorage::save_pbft_block_period` now route through either
  `storage_shim_save_pbft_block_period` or native Rust period storage, so the broad CXX storage method has been deleted.
- The storage conformance caller of broad `BridgeStorage::save_rounds_count_dynamic_lambda` now routes through
  `storage_shim_save_rounds_count_dynamic_lambda`, so the broad CXX storage method has been deleted.
- The remaining callers of broad `BridgeStorage::save_period_lambda` now route through either
  `storage_shim_save_period_lambda` or native Rust metadata storage, so the broad CXX storage method has been deleted.
- The remaining callers of broad `BridgeStorage::save_status_field`, `save_pbft_mgr_field`, and
  `save_pbft_mgr_status` now route through dedicated storage-shim batch appenders or native Rust storage repositories,
  so the broad CXX storage methods have been deleted.
- The remaining callers of broad `BridgeStorage::save_dag_block`, `remove_dag_block`,
  `save_proposal_period_dag_levels_map`, and `save_dag_block_period` now route through dedicated storage-shim batch
  appenders or native Rust DAG repositories, so the broad CXX storage methods have been deleted.
- The remaining callers of broad `BridgeStorage::save_period_data` now route through the dedicated storage-shim batch
  appender or native Rust period storage, so the broad CXX storage method has been deleted.
- The remaining callers of broad `BridgeStorage::save_transaction`, `remove_transaction`, `save_transaction_location`,
  `save_system_transaction`, and `save_period_system_transactions_hashes` now route through dedicated storage-shim batch
  appenders or native Rust transaction repositories, so the broad CXX storage methods have been deleted.
- `BridgeStorage::save_non_finalized_transactions` is also deleted. Older transaction-manager bridge paths now call the
  native `rustaxa-consensus` transaction storage helper directly to persist accepted non-finalized transaction payloads
  and the manager-owned `TrxCount` in a single Rust storage batch.
- `transaction_manager_shim::removeNonFinalizedTransactions` now routes through the Rust transaction-manager runtime for
  both pending-storage-row deletion and sidecar removal. Rust commits the native storage delete batch first and then
  mutates live sidecar state, matching the legacy C++ behavior without exposing public `DbStorage` batch usage in
  Rust-mode.
- `proposed_blocks_shim::cleanupProposedPbftBlocksByPeriod` is the active Rust-mode route for proposed-block cleanup.
  It calls `BridgeProposedBlocks::proposed_blocks_cleanup_with_storage`, which plans stale period/hash groups, commits a
  native Rust storage delete batch, and only then mutates the Rust proposed-block index. The public batch loop in
  `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` is legacy/reference behavior when
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` enables the overlay, not remaining Rust-mode storage-shim debt.
- `sortition_params_manager_shim` is the active Rust-mode route for sortition startup and finalized-period persistence.
  It constructs `BridgeSortitionParamsManager` with `DbStorage::rustStorage()`, so the Rust runtime loads persisted
  changes, persists the missing period-zero default change, reads period-specific parameters, and persists emitted
  finalized-period changes through native Rust storage. The public batch block in
  `libraries/core_libs/consensus/src/dag/sortition_params_manager.cpp` is legacy/reference behavior when
  `RUSTAXA_ENABLE_SORTITION_PARAMS` enables the overlay.
- `final_chain_shim` is the active Rust-mode route for FinalChain startup, native finalization, external-EVM publication,
  pending-publication recovery, and storage audit. It constructs `BridgeFinalChain` and `BridgeConsensusExecutionApi`;
  C++ supplies only the external `StateAPI`/EVM adapter, while Rust commits FinalChain headers, receipts, transaction
  indexes, bloom indexes, execution counters, rewards-stat updates, pending-publication markers, recovery cleanup, and
  genesis/header storage through native Rust storage. The public batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` are legacy/reference behavior when
  `RUSTAXA_ENABLE_FINAL_CHAIN` enables the overlay.
- `pbft_manager_shim` is the active Rust-mode route for PBFT manager reset, finish-polling, loopback-finish, period
  advance, and finalization storage intent execution. Reset/finish transitions call
  `pbft_manager_runtime_apply_transition_storage_write`, so Rust commits the manager cursor/status rows,
  cert-voted-block removal, and own-verified-vote cleanup in one native storage batch before the runtime snapshot and
  C++ mirrors advance. Executed-block reset is a separate Rust-owned status write that preserves the legacy
  post-finalization wait ordering, and finalization/dynamic-lambda storage writes are owned by the Rust finalization
  storage path behind `pbft_manager_runtime_apply_finalization_storage_writes`. The public batch blocks in
  `libraries/core_libs/consensus/src/pbft/pbft_manager.cpp` are legacy/reference behavior when
  `RUSTAXA_ENABLE_PBFT_MANAGER` enables the overlay; remaining PBFT manager cleanup belongs to Slice 6 service
  consolidation and Slice 8 CXX session-handle shrinkage rather than new storage-shim APIs.
- `BridgeGasPricer` no longer exports a separate `gas_pricer_init_from_storage` CXX method. Rust-mode storage history
  restoration is owned by `create_gas_pricer_from_storage`, so C++ cannot create a gas-pricer runtime and later inject
  broad storage access through a second bridge call. The obsolete Rust test-only `gas_pricer_init_from_storage` method is
  also deleted; bridge tests now cover the production constructor path directly.
- `BridgePeriodDataQueue`, `period_data_queue_shim`, and `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE` are retired. PBFT manager
  runtime owns period-data queue metadata through `pbft_manager_runtime_period_data_queue_*`; the C++ PBFT manager shim
  temporarily owns live `PeriodData`, vote, and peer sidecars until those payload models move to Rust.
- `BridgePbftSyncQueueDrainSession` is retired. PBFT sync queue-drain planning is now owned by the long-lived
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_pbft_sync_queue_drain`,
  `pbft_manager_runtime_pbft_sync_queue_drain_next`, and
  `pbft_manager_runtime_pbft_sync_queue_drain_report`. C++ remains the temporary executor for live queue sidecars,
  `processPeriodData()`, `pushPbftBlock_()`, and network sync-state updates until Slice 6 moves PBFT sync execution into
  the native Rust PBFT manager service.
- `BridgePbftVotePipelineSession` and `BridgePbftVoteAdmissionSession` are retired. They had no production C++ callsites;
  the deterministic vote pipeline/admission behavior remains covered by native `rustaxa-consensus` tests while the bridge
  keeps only live C++ facade and network-facing vote helpers.
- `BridgePbftManagerStateActionEffectSession` is retired. PBFT manager state-action effect cursors are now owned by the
  long-lived `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_state_action_effect_session`,
  `pbft_manager_runtime_state_action_effect_session_next`, and
  `pbft_manager_runtime_state_action_effect_session_report`, so C++ no longer allocates a standalone bridge handle for
  this internal PBFT manager transcript.
- `BridgePbftManagerRuntimeSession` is retired. The outer PBFT manager daemon-tick cursor is now owned by
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_session`, `pbft_manager_runtime_session_next`,
  `pbft_manager_runtime_session_report`, and `abort_pbft_manager_runtime_session`, so C++ no longer allocates a
  standalone bridge handle for the scheduler transcript.
- `BridgePbftManagerProposalSession` is retired. PBFT block proposal planning is now a cursor inside
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_proposal_session`,
  `pbft_manager_proposal_session_next`, `pbft_manager_proposal_session_report_dag_order`, and
  `abort_pbft_manager_proposal_session`, so C++ no longer allocates a standalone bridge handle for proposal planning.
- `BridgePbftManagerBlockValidationSession` is retired. PBFT block validation planning is now a cursor inside
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_block_validation_session`,
  `pbft_manager_block_validation_session_next`, and `pbft_manager_block_validation_session_report`, so C++ no longer
  allocates a standalone bridge handle for validation planning.
- `BridgeDagVerifyBlockSession` is retired. DAG block verification still has C++ executor boundaries for transaction
  lookup, FinalChain authorization facts, VDF verification, and gas estimation, but the ordered verification cursor now
  lives inside `BridgeDagManagerRuntime` through `dag_manager_runtime_begin_verify_block_session`,
  `dag_manager_runtime_verify_block_session_next`, and `dag_manager_runtime_verify_block_session_report_*`, so C++ no
  longer allocates a standalone bridge handle for `DagManager::verifyBlock`.
- `BridgeDagProposerSession` is retired. DAG proposal attempts still have C++ executor boundaries for live transaction
  packing, async VDF proof work, block signing/materialization, and `addDagBlock`, but the ordered proposal cursor now
  lives inside `BridgeDagManagerRuntime` as a keyed per-attempt cursor through
  `dag_manager_runtime_begin_proposer_session`, `dag_manager_runtime_proposer_session_next`, and
  `dag_manager_runtime_proposer_session_report_*`, so `DagBlockProposer` no longer allocates a standalone bridge handle
  for each attempt while still preserving concurrent per-wallet proposal attempts.
- `BridgeDagProposerRetryState` is retired. Per-wallet DAG proposer retry cursors now live inside
  `BridgeDagManagerRuntime`, keyed by wallet VRF public key. `dag_block_proposer_shim` passes only the configured retry
  budget, and terminal runtime-owned proposal sessions apply retry updates before deleting their cursor.
- `scripts/rewrite_bridge_inventory_guard.sh` now enforces that every exported CXX `Bridge*` handle in
  `rust/crates/rustaxa-bridge/src/ffi.rs` has an entry in the exported-handle audit table. It also warns when an audit
  row remains after a bridge handle is deleted.

## Agent Use

Slice 0 used the `$implement-rustaxa-consensus-slice` workflow. Custom agents were started for independent read-only
coverage:

- `rust-engineer`: Rust bridge module and exported handle inventory.
- `cpp-pro`: C++ shim and closeout-pattern inventory.
- `architect-reviewer`: audit structure and classification review.

The committed file is based on the local mechanical inventory and should be updated with any later slice-specific
findings when bridge/shim code is removed or narrowed.
