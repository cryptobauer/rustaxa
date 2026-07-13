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
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | `BridgeFinalChain`, `BridgeFinalChainExecutionSession`, `BridgeConsensusExecutionApi`, `create_final_chain*`, `create_final_chain_execution_session`, `create_consensus_execution_api` | `final_chain_shim`, transaction manager runtime, consensus execution adapters | External boundary | Keep EVM/execution boundary while EVM remains out of scope. Move consensus fact reads to Rust FinalChain ports and delete bridge paths that only materialize C++ facts for Rust consensus. Execution-session construction no longer takes a `BridgeFinalChain`; the session owns only the execution request until commit/publish calls reach the real FinalChain boundary. System-transaction planning and native-session commit are now `BridgeConsensusExecutionApi` methods, not standalone CXX exports. |
| `rust/crates/rustaxa-bridge/src/dag.rs` | `BridgeDagGraph`, `BridgeDagManagerRuntime` | `dag_shim`, `dag_manager_shim`, DAG tests | C++ public compatibility facade | Remove graph compatibility handles after DAG public callers stop needing C++ graph aliases. |
| `rust/crates/rustaxa-bridge/src/pbft_chain.rs` | `BridgePbftChain`, `create_pbft_chain_from_storage` | `pbft_chain_shim`, PBFT manager/runtime tests | C++ public compatibility facade | Delete once PBFT chain public C++ facade is no longer required or PBFT manager owns chain state natively in Rust. The no-caller `create_pbft_chain_with_storage` export and duplicate storage-taking free lookup exports are deleted. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | `BridgePbftManagerRuntime`, stateless block-validation planner, manager-owned sync-admission/finalization cursors and external-effect APIs | `pbft_manager_shim`, app bootstrap runtime creation | Internal Rust route | Runtime, state-action effect, proposal, PBFT sync queue-drain/admission, and finalization cursors are owned by `BridgePbftManagerRuntime`; block validation is now a stateless planner with C++-local facts. Keep only app bootstrap handle until PBFT manager C++ facade is retired. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | Manager-owned PBFT sync admission, egress, and cert-vote validation functions | `pbft_manager_shim`, PBFT sync bridge tests | Internal Rust route | The standalone process-period planner and repeated-input fact DTO are retired. Keep narrowing live external checks/materialization until PBFT sync processing is fully composed inside the Rust PBFT manager service. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_*` | Vote validation/generation/progress/ingress/payload helpers | Vote manager shim, network API tests, PBFT/vote tests | Internal Rust route | CXX vote pipeline/admission/session/runtime handles are retired. Standalone planner/event free-function exports and bridge-only DTOs are deleted; live ingress uses `BridgeConsensusNetworkApi`, live validation/admission/reward materialization uses `BridgeVerifiedVotes`, and direct helper exports remain only for canonical inspection, vote generation, and payload conversion still called by C++ shims. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `BridgeVerifiedVotes`, in-memory and storage-restoring constructors, startup snapshot | `verified_votes_shim`, `vote_manager_shim` | C++ public compatibility facade | Rust now restores authoritative verified-vote state from native storage during construction; delete the handle after vote state is private to the Rust PBFT manager and C++ no longer needs vote sidecar materialization. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | `BridgeProposedBlocks`, `create_proposed_blocks_index_from_storage` | `proposed_blocks_shim`, `dag_manager_shim`, `vote_manager_shim` | C++ public compatibility facade | Delete after proposed-block tracking is part of Rust PBFT/DAG runtime and C++ no longer asks for metadata/materialized proposed blocks. The no-storage constructor plus cleanup-candidate/remove-period CXX helpers are deleted; Rust-mode construction requires storage. |
| `rust/crates/rustaxa-bridge/src/rewards_stats.rs` | `BridgeRewardsStatsRuntime`, `create_rewards_stats_runtime` | `rewards_stats_shim`, finalization/reward tests | C++ public compatibility facade | Delete C++ facade once rewards stats publication and storage writes are driven from Rust finalization. The direct runtime-owned storage apply method is no longer CXX API; live writes use the storage-shim batch appender or finalization publication commit paths. FinalChain-facing publication no longer carries the full `RewardsStatsProcessResult` back through C++; the shim retains the previewed plan internally and exposes only distribution stats plus the storage update needed by the execution API. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | `BridgePillarChainStorage`, `BridgePillarChainRuntime`, `create_pillar_chain_storage`, `create_pillar_chain_runtime` | `storage_shim`, `pillar_chain_manager_shim` | C++ public compatibility facade | `BridgePillarChainRuntime` now owns pillar-manager startup recovery, live pillar-vote state, admission, synced bundle apply, and PBFT-facing finalization through one native storage handle. Keep `BridgePillarChainStorage` only for the separate storage-shim compatibility surface; delete it when that public storage facade is retired. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | Standalone pillar-vote inspection APIs plus module-local test fixtures and runtime-owned pillar-chain methods | Live `pillar_chain_manager_shim` uses `BridgePillarChainRuntime`; C++ bridge tests use the runtime or standalone inspection APIs | Internal Rust route | The C++ `pillar_votes_shim` facade and standalone pillar-vote CXX handle are retired. The residual pillar-vote fixture is local to Rust bridge-module tests, not `ffi.rs`; production C++ callers must use `BridgePillarChainRuntime` or native Rust modules. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | `BridgeSortitionParamsManager`, `create_sortition_params_manager*` | `sortition_params_manager_shim`, query/RPC paths through storage | C++ public compatibility facade | Delete after sortition parameter persistence and query reads are native Rust consensus/storage APIs. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | Transaction RLP inspection and bridge DTO helpers | Transaction manager, period-data queue, tests | External boundary | Keep only wire/codec compatibility helpers needed at C++ network/RPC boundaries. Move internal transaction facts to `rustaxa-types`/native consensus. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | `BridgeTransactionManagerRuntime` | `transaction_manager_shim`, RPC submission paths, tests | C++ public compatibility facade | Standalone sidecar and admission-execution handles are retired; live sidecar and DAG-save execution state are private runtime state. Delete the remaining runtime bridge after the transaction manager public C++ facade is retired or all admission/packing paths are native Rust. Keep external EVM/final-chain callbacks as a minimal API. |
| `rust/crates/rustaxa-bridge/src/transaction_queue.rs` | `BridgeTransactionQueue`, `create_transaction_queue`, live queue facade methods | `transaction_queue_shim` | C++ public compatibility facade | Delete after queue ownership moves fully to Rust transaction manager and C++ queue facade is no longer constructed. Queue-only planning/hash-view CXX helpers with no shim callers are deleted. |
| `rust/crates/rustaxa-bridge/src/gas_pricer.rs` | `BridgeGasPricer`, `create_gas_pricer*`, bid/update methods | `gas_pricer_shim`, transaction/RPC gas estimation | C++ public compatibility facade | Delete after gas pricing history and query are Rust-owned behind the transaction/final-chain runtime API. The CXX-only storage init method has been removed; storage restoration is construction-time only. |
| `rust/crates/rustaxa-bridge/src/slashing.rs` | `BridgeSlashingProofPlanner`, `create_slashing_proof_planner` | `slashing_manager_shim` | C++ public compatibility facade | Delete after slashing proof planning is invoked by Rust consensus runtime instead of C++ manager facade. Direct mark-only CXX export is deleted; C++ reports executor outcomes through the submission-report API and receives only the submitted/not-submitted boolean it uses. The live Rust-admission path now passes one normalized double-vote evidence payload with a shared PBFT slot instead of two records plus loose slot scalars. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | VDF bridge helpers | VDF C++ integration/tests | External boundary | Keep the live VDF/prove/verify, atomic-backed cancellation token, and legacy sortition APIs until VDF is explicitly folded into native Rust or a dedicated external VDF API. No-caller scalar/helper exports are deleted when they are covered by native `rustaxa-vdf` tests. |

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
| `BridgePbftManagerRuntime` | `pbft_manager.rs` | App bootstrap, `pbft_manager_shim` | Internal Rust route | PBFT manager C++ orchestration is collapsed into Rust runtime. |
| `BridgeVerifiedVotes` | `verified_votes.rs` | `verified_votes_shim`, `vote_manager_shim` | C++ public compatibility facade | Verified vote state is private Rust vote-manager state. |
| `BridgeProposedBlocks` | `proposed_blocks.rs` | `proposed_blocks_shim`, DAG/vote manager shims | C++ public compatibility facade | Proposed-block tracking is private Rust PBFT/DAG runtime state. |
| `BridgeRewardsStatsRuntime` | `rewards_stats.rs` | `rewards_stats_shim`, storage shim batch append | C++ public compatibility facade | Rewards stats writes/reads are driven from Rust finalization without C++ facade/batch passing. |
| `BridgePillarChainStorage` | `pillar_chain.rs` | `storage_shim`, `pillar_chain_manager_shim` | C++ public compatibility facade | Pillar chain storage is native Rust-owned. |
| `BridgePillarChainRuntime` | `pillar_chain.rs`, `pillar_votes.rs` | `pillar_chain_manager_shim` | Internal Rust route | Pillar vote aggregation, synced bundle apply, and PBFT-facing pillar finalization are fully owned by a Rust runtime. Remove after the C++ PillarChainManager facade is retired or replaced by narrower external ports. |
| `BridgeSortitionParamsManager` | `sortition.rs` | `sortition_params_manager_shim` | C++ public compatibility facade | Sortition params persistence/query is native Rust storage/query behavior. |
| `BridgeTransactionQueue` | `transaction_queue.rs` | `transaction_queue_shim` | C++ public compatibility facade | Transaction queue is private Rust transaction-manager state. |
| `BridgeTransactionManagerRuntime` | `transaction_manager.rs` | `transaction_manager_shim`, app/bootstrap | C++ public compatibility facade | Transaction admission/packing runs behind native Rust runtime and minimal external submission API. |
| `BridgeGasPricer` | `gas_pricer.rs` | `gas_pricer_shim` | C++ public compatibility facade | Gas pricing is native Rust query/runtime behavior. The exported CXX surface is limited to construction, bid, pool-aware bid, and finalized-block update. |
| `BridgeSlashingProofPlanner` | `slashing.rs` | `slashing_manager_shim` | C++ public compatibility facade | Slashing planning runs inside Rust consensus runtime. |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `dag_block_proposer_shim` | Rust retry state for DAG proposal attempts | DAG manager/proposer code | C++ public compatibility facade | Delete when DAG proposal planning lives fully inside Rust DAG runtime. |
| `dag_manager_shim` | Rust DAG manager runtime behind C++ `DagManager` API | App/consensus code, DAG tests | C++ public compatibility facade | `DagManagerOld` inheritance/construction is removed. Remove the remaining C++ graph materialization when DAG manager callers can use Rust runtime or a thinner public facade. |
| `dag_shim` | Rust-backed C++ DAG facade with hash-only public graph compatibility | DAG manager and DAG tests | C++ public compatibility facade | Delete after public DAG graph callers can use the Rust runtime or a narrower external facade directly. |
| `final_chain_shim` | Rust FinalChain runtime behind C++ FinalChain API | App, PBFT manager, transaction manager, RPC/EVM execution | External boundary | Keep EVM execution adapter; remove consensus fact/materialization methods after Rust consensus consumes FinalChain ports directly. |
| `gas_pricer_shim` | Gas price oracle facade | Transaction/RPC gas price paths | C++ public compatibility facade | Delete after gas price API is native Rust and external RPC sees only a narrow query method. |
| `key_manager_shim` | Key manager compatibility | App/bootstrap/key manager users | External boundary | Keep until key ownership is redesigned; not a consensus-internal deletion target. |
| `pbft_chain_shim` | PBFT chain facade | PBFT manager and tests | C++ public compatibility facade | Delete after PBFT chain state becomes private to Rust PBFT manager/runtime. |
| `pbft_manager_shim` | PBFT manager Rust runtime facade | App bootstrap and consensus loop | Internal Rust route | Collapse into native Rust PBFT runtime and remove C++ orchestration once network/EVM/storage external APIs are thin. |
| `pillar_chain_manager_shim` | Pillar chain manager compatibility; live vote aggregation, synced bundle apply, and PBFT-facing pillar finalization now route through `BridgePillarChainRuntime` around the external FinalChain DPoS read | App/consensus pillar paths | C++ public compatibility facade | Delete after pillar chain runtime/storage and DPoS-weight access ports are native Rust-owned. |
| `pillar_votes_shim` | Pillar vote index/admission facade | Retired after this slice | Obsolete scaffold | Removed. `pillar_chain_manager_shim` uses `BridgePillarChainRuntime` for live vote state; no C++ shim behavior remains. |
| `proposed_blocks_shim` | Proposed block tracking facade | DAG manager, vote manager, PBFT paths | C++ public compatibility facade | Delete after proposed-block tracking is folded into Rust PBFT/DAG runtime. |
| `rewards_stats_shim` | Rewards statistics facade | Finalization/rewards tests | C++ public compatibility facade | Delete after Rust finalization owns rewards stats writes/reads directly. The FinalChain publication edge is narrowed so callers receive only reward distribution stats and a storage-update DTO; the Rust process plan remains inside the shim until publication commit succeeds. |
| `slashing_manager_shim` | Slashing proof planner facade; Rust vote admission passes one `SlashingDoubleVoteEvidence` payload while the live `PbftVote` overload is only a compatibility adapter | Slashing manager users | C++ public compatibility facade | Delete after slashing planning and transaction submission run inside Rust consensus/runtime ports. |
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

Current snapshot after DAG manager verify-result API cleanup:

- Direct `*Old::method(...)` forwarding in consensus shims has no current matches. `dag_manager_shim` no longer inherits
  from or constructs `DagManagerOld`; it owns its public `VerifyBlockReturnType` enum and shared-pointer identity
  locally. `dag_shim` no longer re-exports unused `DagOld` Boost graph aliases, includes, or protected Boost-vertex
  helpers; the overlay still imports the legacy header only to keep the pure C++ `DagOld` reference type available before
  defining the Rust-mode `Dag` facade.
- `vote_manager_shim` no longer inherits from or constructs `VoteManagerOld`. It owns the complete public API and its
  temporary C++ compatibility state directly, and the upstream `VoteManager` implementation-state section is restored
  to its original private shape. `setNetwork` therefore writes shim-owned state rather than inherited protected state.
- `BridgeVerifiedVotes` production construction is fallible and storage-backed. Rust restores own votes, extra reward
  votes, and typed latest-round 2t+1 bundles, validates and deduplicates canonical weighted payloads, rebuilds replay,
  retained-payload, uniqueness, round-marker, and voted-block state, and returns only a compact startup snapshot for
  temporary C++ own/reward sidecars. The post-construction `attachRustStorage` /
  `verified_votes_attach_storage` handoff and optional production storage state are deleted.
- `dag_manager_shim::getShared` now routes through the shim’s own C++ `shared_from_this()` ownership path, and
  `dag_manager_shim::getDagMutex` now returns a shim-owned mutex to avoid `DagManagerOld` forwarding.
- `transaction_manager_shim::getTransactionsMutex` no longer forwards to `TransactionManagerOld`; the shim method now
  returns a shim-owned lock via `TransactionManagerRustShimAccess`, removing the direct inherited-state dependency for
  lock ownership.
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
- `BridgeStorage::seed_final_chain_conformance_lookup_rows` is deleted. It had no production C++ callsites; Rust bridge
  query fixtures that still need exact FinalChain lookup rows seed them through native `rustaxa-storage`
  `FinalChainStore::write_conformance_lookup_rows` test setup, and the storage conformance runner uses the dedicated
  `storage_shim_seed_final_chain_conformance_lookup_rows` fixture helper. `scripts/rewrite_storage_boundary_guard.sh`
  enforces that this fixture helper remains limited to the conformance runner and its Rust bridge implementation.
- `BridgeTransactionStorageQueries::get_transaction_rlps_by_hashes` is deleted. Live DAG transaction availability and
  sync materialization use runtime-owned DAG APIs; the direct storage query had only a C++ bridge-test caller, with
  native Rust coverage retained for pending, finalized, system, and missing transaction RLP lookups.
- The standalone `inspect_pbft_finalization_resume` CXX export and later runtime-scoped
  `pbft_manager_runtime_inspect_finalization_resume` CXX method are deleted. Live duplicate-finalization recovery enters
  resume mode on `pbft_manager_runtime_start_finalization_executor`; C++ supplies only the FinalChain last-block fact,
  and `BridgePbftManagerRuntime` inspects its Rust storage internally before starting the replay cursor. Rust tests call
  the native consensus resume inspector directly.
- `BridgePeriodStorageQueries::get_pbft_block_hash_by_period` is deleted. It had no C++ or Rust callers after public
  period lookups moved to the typed by-PBFT-hash query and raw period-data readers used by the storage shim,
  conformance fixtures, and PBFT sync tests.
- `BridgeFinalChain::get_vrf_key` and `BridgeFinalChain::estimate_call_gas` are deleted from the CXX surface. Repo
  callers use the block-scoped `get_vrf_key_at_block` shim route and the dedicated `FinalChain::call` gas-estimation
  adapter instead; the removed wrappers had no C++ or Rust callers.
- `BridgeFinalChain::publish_external_evm_publication` is deleted from the CXX surface. Live external-EVM publication
  enters through `BridgeFinalChainExecutionSession` and `BridgeConsensusExecutionApi`; bridge tests that still verify
  malformed direct publication plans now call the native Rust `FinalChain::publish_external_evm_publication` helper
  without exporting that wrapper to C++.
- `BridgeTransactionQueue` CXX exports are narrowed to the methods used by `transaction_queue_shim`. The no-caller
  queue-only planning/hash-view exports `transaction_queue_erase_plan`, `transaction_queue_ordered_hashes`,
  `transaction_queue_ordered_hashes_plan`, `transaction_queue_all_hash_groups`, and
  `transaction_queue_block_finalized_plan` have been deleted from the CXX surface. Native `rustaxa-consensus`
  transaction queue tests keep the internal planner coverage.
- `BridgeTransactionManagerRuntime` no-caller compatibility exports have been trimmed after the transaction-manager shim
  moved to runtime-owned command APIs. Deleted exports include old runtime sidecar lookup/finish/evict helpers, queue
  erase/get/order/known helpers, and sidecar size/remove helpers that had no C++ shim callers.
- Additional no-caller `BridgeTransactionManagerRuntime` CXX exports are deleted:
  `transaction_manager_runtime_pack_begin`, `transaction_manager_runtime_gas_estimation_cache_size`, and
  `transaction_manager_runtime_insert_recovery_entries`. The live shim uses
  `transaction_manager_runtime_pack_prepare_sharded` plus
  `transaction_manager_runtime_pack_finalize_with_estimates`, gas-estimation cache state is verified through planner outputs, and
  recovery insertion is private Rust runtime behavior behind `transaction_manager_recover_nonfinalized_with_runtime`.
- The older transaction-pack cursor CXX API is deleted: `transaction_manager_runtime_pack_begin_sharded`,
  `transaction_manager_runtime_pack_request_next`, `transaction_manager_runtime_pack_record_estimate_step`, and the
  bridge-only `TransactionPackEstimateOutcome` DTO. Live C++ transaction packing uses the batch prepare/finalize route,
  and Rust bridge tests now cover candidate selection, sharding, declared-gas, cache, and finalization behavior through
  that route.
- The stale `TransactionManagerInsertTransactionFact` and `TransactionManagerInsertTransactionOutcome` CXX DTOs are
  deleted. They were not referenced by any exported bridge function or C++ caller; insertion precheck/admission planning
  now uses private Rust structs inside `transaction_manager.rs`, and C++ still receives only the higher-level admission
  command reports it already consumed.
- The bridge-test-only transaction-manager recovery loader exports and DTOs are deleted:
  `transaction_manager_load_nonfinalized_recovery`, `transaction_manager_load_nonfinalized_recovery_inputs`,
  `TransactionManagerRecoveryEntry`, and `TransactionManagerSidecarRecoveryInsertInput`. The only C++ recovery boundary is
  now `transaction_manager_recover_nonfinalized_with_runtime`, which keeps storage scan, stale-row cleanup, payload
  validation, and sidecar rebuild inside Rust-owned runtime code.
- The bridge-test-only transaction-manager stored-lookup exports and DTOs are deleted:
  `transaction_manager_load_stored_transactions`, `transaction_manager_load_proposal_transactions_with_final_chain`,
  `TransactionManagerStoredTransactionRequest`, and `TransactionManagerStoredTransactionLookup`. C++ materialization
  remains behind `TransactionManager` facade methods backed by runtime-owned transaction view APIs.
- Additional no-caller direct `BridgeTransactionManagerRuntime` queue/sidecar helpers are deleted:
  `transaction_manager_runtime_insert_non_finalized`, `transaction_manager_runtime_contains_non_finalized`,
  `transaction_manager_runtime_contains_recently_finalized`, `transaction_manager_runtime_apply_finalized_transition`,
  `transaction_manager_runtime_queue_insert`, `transaction_manager_runtime_insert_transaction_precheck`, and
  `transaction_manager_runtime_queue_contains`. Shared DTOs remain where live C++ high-level runtime commands still use
  them, but the direct mutation/check methods are Rust-internal only.
- Older no-caller transaction-manager FinalChain-backed shortcuts are deleted from the CXX bridge surface:
  `transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report`,
  `transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report`,
  `transaction_manager_runtime_queue_cleanup_with_account_nonce_facts`,
  `save_transactions_from_dag_block_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block_command_report_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block`, `update_finalized_transactions_status`, and
  `transaction_manager_verify_not_finalized_with_runtime_and_final_chain`. Live C++ uses FinalChain-backed fact inputs on
  the high-level runtime commands, runtime-owned DAG-save command reports, and the finalized-status command; the remaining
  Rust helpers are module-local behavior coverage rather than exported compatibility API.
  The queue cleanup helper now remains only as a private Rust bridge method used by
  `update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts`.
- DAG manager CXX sync-selection scaffolding is deleted from the bridge surface:
  `dag_manager_runtime_non_finalized_sync_snapshot`, `dag_manager_runtime_select_non_finalized_hashes`, and
  `DagManagerRuntimeSyncSnapshot`. Live C++ DAG sync materialization uses
  `dag_manager_runtime_non_finalized_sync_payload`; Rust bridge/domain tests keep the lower-level selection and snapshot
  coverage without exposing those helpers to C++.
- Standalone DAG verification and add-block helper functions are deleted from the CXX surface:
  `dag_verify_transaction_availability`, `dag_plan_verify_transaction_query`,
  `dag_plan_non_finalized_transaction_query`, `dag_plan_expired_transaction_cleanup`, `dag_verify_vdf_prepare`,
  `dag_verify_authorization`, `dag_decide_vdf_dpos_authorization`, `dag_verify_vdf_sortition`,
  `dag_plan_add_block_effects`, and `dag_verify_gas`. The live DAG manager shim now reaches those decisions through
  runtime-owned `BridgeDagManagerRuntime` methods, with only `dag_verify_vdf_sortition_from_block` retained as the
  temporary direct VDF verification boundary.
- Additional DAG runtime bridge-test scaffolding is deleted from the CXX surface:
  `dag_manager_runtime_rebuild`, `dag_manager_runtime_block_exists`, `dag_manager_runtime_verify_precheck`,
  `dag_manager_runtime_expired_transaction_cleanup_payload`, `dag_vrf_input`, `DagManagerSnapshot`,
  `DagVerifyPrecheckBlock`, `DagVerifyPrecheckResult`, `DagExpiredTransactionFact`, and
  `DagExpiredTransactionCleanupPayload`. Live C++ uses storage restore, `dag_manager_runtime_is_block_known`, verify
  sessions, finalized-order application, and the retained `dag_vdf_message` public helper; native
  `rustaxa-consensus` DAG tests cover the deleted lower-level behavior.
- The direct timestamp-supplied DAG proposer intent CXX export is deleted:
  `dag_proposer_plan_block_intent` and `DagProposerBlockIntentInput`. Live C++ DAG proposal construction uses
  `dag_proposer_plan_block_intent_with_current_timestamp` and `dag_proposer_finalize_signed_block_intent`, so the CXX
  bridge no longer offers an alternate route where C++ supplies the proposal timestamp. The deterministic fixed-timestamp
  planner remains native `rustaxa-consensus` behavior covered by Rust tests.
- No-caller lower-level VDF/VRF helper exports are deleted from the CXX bridge surface:
  `vdf_sortition_payload_verify_with_modulus`, `vdf_sortition_threshold_from_output`,
  `vdf_sortition_normalize_vote_count`, `vdf_sortition_difficulty`, `vdf_sortition_legacy_modulus`,
  `vrf_proof_to_hash`, and `vrf_prove_output`. Live C++ keeps the coarse VDF object/prove/verify APIs, legacy
  VDF/VRF sortition prove/verify APIs, and the payload encode API used by DAG proposer code. The later bridge-test-only
  payload decode/verify and VRF output verification exports are also deleted from the CXX surface, along with
  `VdfSortitionVerifyConfig`, `VdfSortitionPayloadVerifyResult`, and `VrfVerifyOutput`. All deleted VDF/VRF helper
  behavior remains native `rustaxa-vdf` behavior covered by Rust tests rather than CXX surface area.
- The test-only default `make_cancellation_token`, `cancellation_token_cancel`, and direct
  `verify_legacy_vrf_sortition` CXX exports are deleted. Live C++ uses `make_cancellation_token_with_atomic` for
  cancellation and the operation-level legacy VDF/VRF sortition APIs; direct VRF verification remains native
  `rustaxa-vdf` coverage.
- `BridgeProposedBlocks::proposed_blocks_snapshot` is deleted from the CXX surface. The live proposed-block shim uses
  `proposed_blocks_snapshot_entries`, which carries the block payload and validation flag; grouped hash snapshots remain
  Rust-only test coverage.
- The no-storage `create_proposed_blocks_index` CXX constructor plus the standalone cleanup-candidate/remove-period CXX
  helpers are deleted. Rust-mode `ProposedBlocks` construction now requires `DbStorage`; the local proposal-generation
  scratch index uses storage-backed construction with non-persisting `proposed_blocks_push`.
- `BridgePbftChain::pbft_chain_project_update` is deleted from the CXX surface. Native `rustaxa-consensus` tests cover
  the non-mutating projection helper, and live C++ bridge callers use `pbft_chain_update`,
  `pbft_chain_update_for_finalization`, or `pbft_chain_project_legacy_json_head`.
- The duplicate storage-taking free `pbft_chain_block_exists(storage, hash)` and `pbft_chain_block_rlp(storage, hash)`
  CXX exports are deleted. Live C++ uses the storage-backed `BridgePbftChain` handle methods, so PBFT-chain block lookup
  no longer exposes a second direct `BridgeStorage` route.
- The direct structured-head `create_pbft_chain(PbftChainHeadPayload)` constructor is deleted from the CXX surface. C++
  bridge tests now seed legacy `pbft_head` JSON through the storage-shim batch API and use
  `create_pbft_chain_from_storage`, which is the same constructor path used by the live `pbft_chain_shim`.
- The direct in-memory `create_sortition_params_manager(SortitionRuntimeConfig, Vec<SortitionParamsChangePayload>)`
  constructor is deleted from the CXX surface. C++ bridge tests now use `create_sortition_params_manager_from_storage`,
  which is the same constructor path used by the live `sortition_params_manager_shim`; the direct bridge wrapper is gone.
- The default-rewards `create_final_chain(...)` constructor is deleted from the CXX surface. C++ bridge tests now pass an
  explicit `FinalChainRewardsConfig` through `create_final_chain_with_rewards_config`, which is the constructor shape
  used by the live `final_chain_shim`; the default wrapper is Rust test-only fixture code.
- `BridgeTransactionManagerSidecar` is retired as a CXX handle. No C++ shim callers remained for the standalone sidecar
  constructor, methods, DAG-save route, or finalized-status route; live sidecar state is now private to
  `BridgeTransactionManagerRuntime`, whose command APIs own those paths.
- `BridgeTransactionManagerAdmissionExecution` is retired as a CXX handle. No C++ shim callers remained for the explicit
  execute/commit DAG-save script; the retained `save_transactions_from_dag_block_command_report_with_runtime` boundary
  now keeps the storage-first write and live queue/sidecar mutation ordering inside one runtime-owned bridge method.
- Transaction-manager lower-level DAG-save/finalized-status result APIs are deleted from the CXX surface:
  `save_transactions_from_dag_block_with_runtime`, `update_finalized_transactions_status_with_runtime`,
  `DagTransactionSaveAccepted`, `DagTransactionSaveOutcome`, `FinalizedTransactionStatusAction`, and
  `FinalizedTransactionStatusPlan`. Live C++ callers use command-report APIs, while the lower production helpers and
  result structs are private Rust implementation details; deleted wrapper exports retained for direct Rust unit coverage
  are test-only.
- Additional no-caller transaction-manager CXX exports are deleted:
  `create_transaction_manager_runtime` and
  `update_finalized_transactions_status_command_report_with_runtime`. Production C++ constructs the runtime from storage
  and reports finalized-status updates through the account-nonce/purge-aware command-report API.
- No-caller bridge-test-only CXX exports have been deleted from remaining compatibility handles:
  `create_pbft_chain_with_storage`, `slashing_mark_double_voting_proof_submission`,
  `pillar_votes_get_verified_votes`, and `pillar_votes_snapshot_refs`. Live C++ callers use the storage-restoring PBFT
  chain constructor, slashing executor-report API, and pillar-vote payload lookup API.
- No-caller verified-vote and sortition CXX exports have also been deleted:
  `verified_votes_check_unique_voter`, `verified_votes_vote_in_verified_map`,
  `verified_votes_get_network_t_plus_one_step`, `verified_votes_get_two_t_plus_one_voted_block_votes`,
  `verified_votes_snapshot_weighted_payloads`, and `sortition_restore_finalized_period`. C++ verified-vote callers use
  insertion/admission, retained payload lookups, round-marker snapshots, and explicit sortition finalized-period
  record/persist methods instead.
- `BridgePillarChainStorage::pillar_chain_storage_block_data_rlp` is deleted from the CXX surface. Rust-mode Taraxa RPC
  pillar block-data reads use `BridgeConsensusQueryApi::consensus_query_pillar_block_data_by_period`, and
  pillar/storage shims only require current/latest block, own-vote, finalized-block, and period-data storage methods.
- `pillar_chain_manager_shim` no longer constructs a parallel `BridgePillarChainStorage` handle during startup.
  `BridgePillarChainRuntime::pillar_chain_runtime_load_startup_bootstrap` loads the own vote, current-block sidecar, and
  latest finalized pillar block, decodes the latest block in Rust, derives the following period-data lookup, and returns
  one recovery snapshot for temporary C++ materialization. The storage-only handle remains solely for `storage_shim`
  compatibility callers.
- `pillar_chain_manager_shim::validateSyncPillarVotesBundleDeterministically()` no longer performs shim-local per-vote
  inspection and weight accumulation. It calls the Rust batch RLP inspection API, performs the one remaining external
  FinalChain DPoS weight read as a batch, then calls `BridgePillarChainRuntime` for weighted bundle apply into
  runtime-owned vote state. The obsolete shim helper `getPillarVoteWeight()` has been removed.
- The standalone `plan_pillar_vote_relevance` CXX export is deleted. Production tarcap relevance checks use
  `BridgeConsensusNetworkApi::consensus_network_plan_pillar_vote_relevance`, while the pillar-chain manager uses
  `BridgePillarChainRuntime::pillar_chain_runtime_plan_vote_relevance` so duplicate detection comes from runtime-owned
  pillar-vote state. Native `rustaxa-consensus` and bridge-module tests keep direct planner coverage.
- `pillar_chain_manager_shim::createPillarBlock()` now calls
  `plan_pillar_block_creation_with_vote_counts`, which combines pillar-block shell planning and ordered validator
  vote-count delta planning behind one Rust bridge call. The creation-only `plan_pillar_block_creation` CXX export and
  shell-only DTO are deleted. C++ still owns external FinalChain DPoS vote-count reads, temporary `PillarBlock`
  materialization, current-block storage payload materialization, and live manager mirrors, but the current-block sidecar
  write now enters through `BridgePillarChainRuntime` instead of `BridgePillarChainStorage`.
- The no-caller plain-fact pillar-vote bundle CXX planner is deleted:
  `PillarVoteBundleFact`, `PillarVoteBundleAcceptedVote`, `PillarVoteBundlePlan`, and `plan_pillar_vote_bundle`.
  Live pillar-chain sync uses canonical vote RLPs through `inspect_pillar_vote_bundle_rlps`, performs the one remaining
  external FinalChain DPoS weight read in C++, then calls
  `BridgePillarChainRuntime::pillar_chain_runtime_apply_weighted_rlp_bundle`. The old weighted planner export,
  accepted-voter DTO, shim-side accepted-hash-to-live-vote map, and `addPlannedVerifiedPillarVoteForRust` insertion
  helper are deleted. Native `rustaxa-consensus` tests keep coverage for the plain domain planner.
- The standalone pillar-vote CXX handle is deleted from `ffi.rs` after the last C++ bridge test moved to
  `BridgePillarChainRuntime` for weighted-bundle apply and payload lookup coverage. The remaining pillar-vote fixture is
  module-local test code in `pillar_votes.rs`, so retired handle names no longer live in bridge code.
- The no-caller `pillar_chain_runtime_cleanup_votes_by_period` CXX export is deleted after callsite audit confirmed the
  runtime cleanup method had no live C++ shim or bridge-test caller.
- Single pillar-vote admission in `pillar_chain_manager_shim` now uses
  `BridgePillarChainRuntime::pillar_chain_runtime_prepare_single_vote_admission` plus
  `BridgePillarChainRuntime::pillar_chain_runtime_apply_prepared_single_vote_admission`. Rust owns canonical RLP decode, signature
  recovery, duplicate/relevance/identity checks, period-data initialization, insertion, and conflict/duplicate
  classification. C++ retains only external FinalChain DPoS eligibility/vote-count reads, threshold lookup, and logging.
  The manager's own-vote persistence write now also enters through `BridgePillarChainRuntime`; the matching
  `BridgePillarChainStorage` write methods remain for `storage_shim` compatibility only.
  The piecemeal single-vote CXX exports `pillar_votes_period_data_initialized`, `pillar_votes_init_period_data`,
  `pillar_votes_vote_exists`, `pillar_votes_is_unique_identity`, `pillar_votes_is_unique_vote`, and
  `pillar_votes_insert_vote` are deleted along with `PillarVotePayload`, `PillarVoteIdentityPayload`,
  `PillarVoteUniqueOutcome`, and `PillarVoteInsertOutcome`.
- `PillarChainManager::isRelevantPillarVote` now enters the pillar runtime through
  `pillar_chain_runtime_plan_vote_relevance`. The obsolete C++ `pillarVoteExistsByLookup` payload materialization and
  hash scan are deleted; Rust owns duplicate detection from the runtime vote index before running the relevance planner.
- PBFT-facing pillar-block finalization now enters through
  `BridgePillarChainRuntime::pillar_chain_runtime_finalize_block_for_pbft`, which owns selected-vote lookup,
  deterministic finalization planning, finalized-block storage persistence, and vote cleanup ordering. The old CXX
  planner exports `plan_pbft_finalization_pillar_preflight`, `report_pbft_finalization_pillar_preflight`, and
  `plan_pillar_block_finalization` plus their bridge-only DTOs are deleted. C++ still owns network vote-bundle requests,
  legacy vote materialization for `PeriodData`, live mirror assignment, and event emission.
- Pillar-vote network egress no longer materializes C++ `PillarVote` objects. `GetPillarVotesBundlePacketHandler`
  requests packet-ready optimized bundle chunks from `pillar_chain_manager_shim`, which delegates to
  `BridgePillarChainRuntime::pillar_chain_runtime_build_verified_vote_network_bundles`. Rust returns inner optimized
  bundle RLP bytes plus matching vote hashes for peer-known bookkeeping, using live runtime votes first and a strict
  stored `PeriodData` fallback only when the embedded bundle matches the requested period/hash. Network/tarcap still owns
  request validation, packet wrapping, send execution, and peer-known marking.
- `PillarChainManager::getVerifiedPillarVotes()` no longer performs its own storage-byte fallback after an empty
  runtime lookup. The compatibility method now calls
  `BridgePillarChainRuntime::pillar_chain_runtime_get_verified_vote_payloads`, and Rust owns the live-vote-first,
  stored-`PeriodData` fallback plus period/hash verification before the shim materializes temporary C++ votes.
- The no-caller broad `apply_rewards_stats_storage_writes` CXX export is deleted. The later no-production-caller
  `BridgeRewardsStatsRuntime::rewards_stats_runtime_apply_storage_writes` CXX method and its remaining test-only Rust
  bridge wrapper are also deleted. Live rewards-stat persistence uses the dedicated storage-shim batch appender for
  staged `DbStorage` compatibility writes, while Rust bridge-module tests call the native consensus owned-storage apply
  helper directly for coverage instead of preserving a bridge-shaped wrapper.
- `transaction_manager_shim::removeNonFinalizedTransactions` now routes through the Rust transaction-manager runtime for
  both pending-storage-row deletion and sidecar removal. Rust commits the native storage delete batch first and then
  mutates live sidecar state, matching the legacy C++ behavior without exposing public `DbStorage` batch usage in
  Rust-mode.
- `proposed_blocks_shim::cleanupProposedPbftBlocksByPeriod` is the active Rust-mode route for proposed-block cleanup.
  It calls `BridgeProposedBlocks::proposed_blocks_cleanup_with_storage`, which plans stale period/hash groups, commits a
  native Rust storage delete batch, and only then mutates the Rust proposed-block index. The public batch loop in
  `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` is legacy/reference behavior when
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` enables the overlay, not remaining Rust-mode storage-shim debt.
- The shim no longer constructs a separate no-storage proposed-block bridge. `pushProposedPbftBlock(..., false)` remains
  supported on storage-backed indexes for already-durable candidate blocks, but null-DB Rust-mode construction fails
  explicitly instead of preserving an in-memory compatibility facade.
- `sortition_params_manager_shim` is the active Rust-mode route for sortition startup and finalized-period persistence.
  It constructs `BridgeSortitionParamsManager` with `DbStorage::rustStorage()`, so the Rust runtime loads persisted
  changes, persists the missing period-zero default change, reads period-specific parameters, and persists emitted
  finalized-period changes through native Rust storage. The public batch block in
  `libraries/core_libs/consensus/src/dag/sortition_params_manager.cpp` is legacy/reference behavior when
  `RUSTAXA_ENABLE_SORTITION_PARAMS` enables the overlay.
- The direct `sortition_params_for_period(found, change)` CXX export is deleted. Live C++ lookups use the storage-backed
  `sortition_params_for_period_from_storage(period)` route, so callers no longer inject synthetic sortition-change
  payloads through a bridge-shaped helper. Native `rustaxa-consensus` tests keep direct `params_for_period` coverage.
- `final_chain_shim` is the active Rust-mode route for FinalChain startup, native finalization, external-EVM publication,
  pending-publication recovery, and storage audit. It constructs `BridgeFinalChain` and `BridgeConsensusExecutionApi`;
  C++ supplies only the external `StateAPI`/EVM adapter, while Rust commits FinalChain headers, receipts, transaction
  indexes, bloom indexes, execution counters, rewards-stat updates, pending-publication markers, recovery cleanup, and
  genesis/header storage through native Rust storage. The public batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` are legacy/reference behavior when
  `RUSTAXA_ENABLE_FINAL_CHAIN` enables the overlay.
- `pbft_manager_shim` is the active Rust-mode route for PBFT manager reset, finish-polling, loopback-finish, period
  advance, and finalization storage intent execution. Reset/finish transitions call the manager-owned lifecycle
  transition executor, so Rust derives the live cursor, loads own-vote keys natively, and commits manager cursor/status
  rows, cert-voted-block removal, and own-verified-vote cleanup in one native storage batch before returning the runtime
  snapshot and narrow C++ sidecar commands. Executed-block reset is a separate Rust-owned status write that preserves the legacy
  post-finalization wait ordering, and finalization/dynamic-lambda storage writes are owned by the Rust finalization
  storage path behind `pbft_manager_runtime_apply_finalization_storage_writes`. The public batch blocks in
  `libraries/core_libs/consensus/src/pbft/pbft_manager.cpp` are legacy/reference behavior when
  `RUSTAXA_ENABLE_PBFT_MANAGER` enables the overlay; remaining PBFT manager cleanup belongs to Slice 6 service
  consolidation and Slice 8 CXX session-handle shrinkage rather than new storage-shim APIs.
- The standalone lifecycle transition CXX planning/storage surface is retired:
  `PbftManagerTransitionFact`, `PbftManagerTransitionPlan`, `PbftManagerTransitionRuntimeApplyResult`,
  `plan_pbft_manager_transition`, and `pbft_manager_runtime_apply_transition_storage_write` are deleted. Filter,
  certify, finish, finish-polling, loop-back, delay, reset, and advance-period reset enter through
  `pbft_manager_runtime_execute_lifecycle_transition`; native planner/storage types remain internal and unit-tested.
- Advance-period planning now reads the immediately preceding committed reset from the manager runtime and emits the
  remaining external action order. Missing, stale, mismatched, and empty-chain requests are rejected; the duplicated
  `ApplyResetConsensusTransition` action and embedded transition plan are removed. FinalChain wait,
  VoteManager/timer/wallet/proposed-block compatibility effects remain explicit.
- `BridgeGasPricer` no longer exports a separate `gas_pricer_init_from_storage` CXX method. Rust-mode storage history
  restoration is owned by `create_gas_pricer_from_storage`, so C++ cannot create a gas-pricer runtime and later inject
  broad storage access through a second bridge call. The obsolete Rust test-only `gas_pricer_init_from_storage` method is
  also deleted; bridge tests now cover the production constructor path directly.
- `BridgePeriodDataQueue`, `period_data_queue_shim`, and `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE` are retired. PBFT manager
  runtime owns period-data queue metadata through `pbft_manager_runtime_period_data_queue_*`; the C++ PBFT manager shim
  now temporarily owns only live `PeriodData` and peer sidecars, while cert-vote payloads are sourced directly from
  `plan.previous_cert_vote_rlps` / `plan.cert_vote_rlps`.
- The bridge-only `rustaxa-bridge/src/period_data_queue.rs` helper module is also deleted. The remaining CXX-safe
  conversion glue now lives beside the `pbft_manager_runtime_period_data_queue_*` APIs in `pbft_manager.rs`, so queue
  metadata no longer has a standalone bridge module after the handle and shim were retired.
- `BridgePbftSyncQueueDrainSession` is retired. PBFT sync queue-drain planning is now owned by the long-lived
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_pbft_sync_queue_drain`,
  `pbft_manager_runtime_pbft_sync_queue_drain_next`, and
  `pbft_manager_runtime_pbft_sync_queue_drain_report`. C++ remains the temporary executor for live queue sidecars,
  `pushPbftBlock_()` and network sync-state updates. `processPeriodData()` now drives a nested manager-owned Rust
  sync-admission cursor and retains only its requested external checks and legacy materialization.
- The manager-owned sync-admission cursor captures immutable candidate facts once, retains same-candidate
  FinalChain-behind retry state, validates typed report cursors/check identities, and auto-clears on every terminal or
  contract-error step. The C++ executor aborts before propagating an exception from any live FinalChain, vote,
  transaction, or pillar check.
- `BridgePbftVotePipelineSession` and `BridgePbftVoteAdmissionSession` are retired. They had no production C++ callsites;
  the deterministic vote pipeline/admission behavior remains covered by native `rustaxa-consensus` tests while the bridge
  keeps only live C++ facade and network-facing vote helpers.
- Standalone PBFT vote planner/event free-function exports are retired. `BridgeConsensusNetworkApi` owns production
  vote ingress planning, `BridgeVerifiedVotes` owns validation/admission/reward-vote materialization, and the removed
  bridge-only modules/DTOs no longer expose alternate CXX routes into the same Rust vote logic. Direct canonical
  inspection, vote generation, and payload conversion helpers remain live until their C++ shim callers are moved behind
  a facade.
- `pbft_vote_sortition_threshold_for_bridge` is retired as a no-caller scalar helper. Native Rust consensus keeps the
  threshold calculation internally, while C++ proposer screening continues through the live `pbft_proposer_sortition_plan`
  boundary.
- `BridgePbftManagerStateActionEffectSession` is retired. PBFT manager state-action effect cursors are now owned by the
  long-lived `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_state_action_effect_session`,
  `pbft_manager_runtime_state_action_effect_session_next`, and
  `pbft_manager_runtime_state_action_effect_session_report`, so C++ no longer allocates a standalone bridge handle for
  this internal PBFT manager transcript.
- Standalone PBFT state-action planners are retired from the CXX surface:
  `plan_pbft_manager_state_action` and `plan_pbft_manager_state_action_effects` no longer exist.
  Live C++ uses `pbft_manager_runtime_begin_state_action_effect_session` + next/report advancement for the same
  transcript, with planner coverage kept in Rust bridge/runtime tests rather than bridge-surface planner functions.
- `BridgePbftManagerRuntimeSession` is retired. The outer PBFT manager daemon-tick cursor is now owned by
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_session`, `pbft_manager_runtime_session_next`,
  `pbft_manager_runtime_session_report`, and `abort_pbft_manager_runtime_session`, so C++ no longer allocates a
  standalone bridge handle for the scheduler transcript.
- The standalone PBFT manager sleep CXX planner `plan_pbft_manager_sleep_until_next_step` and its
  `PbftManagerSleepFact` DTO are retired. `pbft_manager_shim::sleep_()` now requires `BridgePbftManagerRuntime` and
  calls `plan_pbft_manager_runtime_sleep_until_next_step`, so C++ no longer copies deadline and step facts out of the
  manager snapshot to drive a fallback route. The native `rustaxa-consensus` planner remains internal domain logic behind
  the runtime-owned bridge API.
- `BridgePbftManagerProposalSession` is retired. PBFT block proposal planning is now a cursor inside
  `BridgePbftManagerRuntime` through `pbft_manager_runtime_begin_proposal_session`,
  `pbft_manager_proposal_session_next`, and `pbft_manager_proposal_session_report_dag_order`, so C++ no longer allocates
  a standalone bridge handle for proposal planning.
- `BridgePbftManagerBlockValidationSession` is retired, and the later bridge-runtime block-validation cursor has also
  been removed. `pbft_manager_shim` now calls stateless `plan_pbft_manager_block_validation` with a C++-local fact
  bundle after each external PBFT-chain, FinalChain, reward-vote, pillar, or DAG check.
- `BridgePbftFinalizationRuntimeSession` is retired. Normal PBFT finalization and duplicate-finalization resume now use
  the manager-owned two-call finalization executor on `BridgePbftManagerRuntime`; C++ still executes external side
  effects until a later manager-owned one-shot finalization operation absorbs the remaining coordinator loop.
- The standalone `validate_pbft_finalization_live_mutation_report` CXX export and bridge-only
  `PbftFinalizationLiveMutationValidation` DTO are deleted. The interim manager-runtime live-report API moved external
  FinalChain/EVM, DAG, transaction-manager, PBFT-chain, sortition, vote-manager, advance-period, and pillar fact
  validation into Rust; the later executor APIs below delete that interim CXX export and fold reporting into the
  manager-owned finalization executor.
- Normal PBFT finalization and duplicate-finalization resume now enter through the manager-owned executor APIs
  `pbft_manager_runtime_start_finalization_executor`, typed success advancement APIs, and
  `pbft_manager_runtime_fail_finalization_external_effect`. The direct CXX exports for finalization runtime planning,
  cursor next/report, standalone live-mutation report, separate boundary failure report, owned-action drain,
  manager-runtime storage apply, and the older piecemeal finalization boundary APIs are deleted. Rust retains the
  accepted finalization plan inside `BridgePbftManagerRuntime`, derives the current action from the cursor, derives base
  finalization identity for typed reports, and returns explicit cursor/action executor states. C++ remains the executor for
  FinalChain/EVM, DAG, transaction-manager, PBFT-chain, sortition, vote-manager, advance-period, pillar, and local cache
  side effects.
- The explicit `abort_pbft_manager_runtime_finalization_session` CXX export is deleted. The finalization executor
  boundary now clears the retained session and accepted plan inside Rust on terminal completion, terminal failure, stale
  cursor mismatch, and any `Result::Err` crossing CXX, so C++ no longer owns Rust cursor cleanup after failed
  finalization attempts.
- The no-caller `pbft_manager_runtime_cached_anchor_dag_order_count` and
  `pbft_manager_runtime_clear_cached_anchor_dag_order` CXX exports are deleted. C++ uses only the `has`, `record`, and
  `remove` anchor-cache metadata APIs required by the active PBFT manager shim; count/clear remain native
  `BridgePbftManagerRuntime` state behavior covered by Rust tests and by the manager-owned finalization drain.
- Duplicate-finalization resume inspection is folded into `pbft_manager_runtime_start_finalization_executor` resume
  mode. C++ supplies only `final_chain_last_block`; the manager runtime inspects its Rust storage internally, starts the
  replay cursor, and returns either the next external action or a completed no-op state. The public runtime inspector and
  `PbftFinalizationResumePlan` CXX DTO are deleted.
- The PBFT manager shim no longer carries separate fresh-finalization and duplicate-resume boundary helper stacks.
  `pushPbftBlock_()` uses one local helper set for snapshot application, action checks, failure reporting, and typed
  advancement while retaining the explicit external FinalChain, DAG, transaction-manager, PBFT-chain, sortition,
  vote-manager, advance-period, pillar, and anchor-cache client APIs.
- The remaining post-start coordinator is now one shared Rust-driven action loop for fresh finalization and durable
  resume. C++ no longer sequences effects from cleanup booleans or a resume-specific action chain; it dispatches the
  current manager-runtime cursor action, consumes each prepared external payload at most once, and reports through the
  typed advancement API. Fresh protected-prefix effects remain under the existing DAG/transaction locks, which are
  released before the same loop executes FinalChain, period-advance, or pillar effects. Rust-owned storage,
  dynamic-lambda, executed-status, and anchor-cache actions remain internally drained and never become C++ cases.
- The standalone `plan_pbft_finalization_intent` CXX export is retired. C++ now calls
  `pbft_manager_runtime_plan_finalization_intent` on the long-lived `BridgePbftManagerRuntime`, making the manager
  runtime the required bridge owner for finalization intent planning. The planner is still stateless and fact-driven at
  this point; the runtime-scoped API prevents new CXX callers from bypassing the manager boundary and leaves room for
  runtime policy without reviving the standalone export. The direct bridge wrapper remains Rust-private for module tests
  and delegates to the native `rustaxa-consensus` planner.
- The standalone `apply_pbft_finalization_storage_writes` CXX export is deleted. Production primary, dynamic-lambda, and
  executed-status finalization storage writes are manager-runtime-owned, and the retained verified-votes storage API
  remains a compatibility surface for vote-manager finalization storage facts. The lower test-only bridge wrapper is
  deleted; direct storage-apply behavior is covered by native `rustaxa-consensus` finalization tests and retained live
  bridge coverage through the verified-votes compatibility API instead of a standalone bridge wrapper.
- The standalone `rustaxa-bridge/src/pbft_finalize.rs` bridge module is retired. Live finalization CXX APIs are
  manager-owned in `pbft_manager.rs`; the finalization FFI/domain conversion impls moved there, and the remaining
  storage-apply result mapper moved to `verified_votes.rs`, its only live compatibility consumer.
- `PbftFinalizationRuntimeActionReport` is no longer a CXX DTO. It is a private Rust helper used inside
  `pbft_manager.rs`; C++ no longer owns the scalar action report DTO.
- `PbftFinalizationLiveMutationReport` and `PbftFinalizationExternalEffectReport` are no longer CXX DTOs. External
  finalization executors now return or construct only subsystem-specific reports, and `BridgePbftManagerRuntime` derives
  the finalization identity from the retained Rust plan before building the native live-mutation validation report. The
  shim-local `makeFinalizationExternalEffectReport` mapper, duplicate cursor-stuffed
  `PbftFinalizationExecutorAdvanceReport` CXX DTO, generic external-effect DTO, and field-copy helpers are deleted.
- The PBFT manager shim still validates the expected action on `PbftManagerFinalizationExecutorState` before executing
  each external side effect, then reports typed success facts by cursor or failure with status/error only. This keeps the
  manager cursor as the only accepted action identity source and prevents sortition, reward-vote, DAG,
  transaction-manager, PBFT-chain, anchor-cache, FinalChain, advance-period, or pillar reports from echoing a second
  action value or a generic success/failure envelope back into Rust.
- `PbftFinalizationRuntimeSessionStep` and `PbftManagerFinalizationOwnedActionDrainResult` are no longer CXX DTOs. The
  manager-owned finalization cursor/drain internals remain Rust-private in `pbft_manager.rs`, and C++ only receives the
  stable `PbftManagerFinalizationExecutorState` executor boundary.
- Transaction finalized-status post-state facts no longer leak through
  `transaction_manager_shim` as `PbftFinalizationExternalEffectReport`. The shim returns its typed
  `TransactionManagerFinalizedStatusCommandReport`, and `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_transaction_status` so the PBFT-specific report mapping is Rust-private.
- PBFT-chain finalization update facts no longer leak through `pbft_chain_shim` as
  `PbftFinalizationExternalEffectReport`. `pbft_chain_update_for_finalization` returns
  `PbftChainFinalizationUpdateReport` with only head size/hash/anchor facts; `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_pbft_chain`, so Rust fills the native live-mutation report internally.
- Sortition finalization update facts no longer leak through `sortition_params_manager_shim` as
  `PbftFinalizationExternalEffectReport`. `commitPreparedBlockForSortitionFinalization` returns
  `SortitionFinalizationCommitReport` with only changed/change/current-threshold/cache-count facts; `pbft_manager_shim`
  advances through `pbft_manager_runtime_advance_finalization_sortition_commit`, so Rust fills the native live-mutation
  report internally.
- Reward-vote reset finalization facts no longer leak through `vote_manager_shim` as
  `PbftFinalizationExternalEffectReport`. `commitRewardVotesResetForFinalization` returns
  `RewardVotesFinalizationResetReport` with only period/round/block-hash/extra-count facts; `pbft_manager_shim`
  advances through `pbft_manager_runtime_advance_finalization_reward_votes_reset`, so Rust fills the native
  live-mutation report internally.
- DAG-order finalization facts no longer leak through `dag_manager_shim` as `PbftFinalizationExternalEffectReport`.
  `setDagBlockOrderForPbftFinalization` returns `DagFinalizationOrderReport` with only the finalized DAG-block count;
  `pbft_manager_shim` advances through `pbft_manager_runtime_advance_finalization_dag_order`, so Rust fills the native
  live-mutation report internally.
- Anchor-DAG-cache clear no longer has a CXX report path. `BridgePbftManagerRuntime` now drains
  `ClearAnchorDagCache` as a manager-owned finalization action, clears Rust anchor-cache metadata, validates the native
  live-mutation report with zero remaining anchors, and returns `cleared_anchor_dag_cache` so the C++ shim clears only
  its temporary materialized `DagBlock` sidecar map. The typed
  `pbft_manager_runtime_advance_finalization_anchor_cache_clear` export and anchor-cache report DTO/helper are deleted.
- FinalChain PBFT finalization dispatch/replay facts now use the shim-owned
  `FinalChainPbftFinalizationDispatchReport` returned by `PbftManager::finalize_`. The report carries only
  `blocks_per_year` and observed FinalChain `last_block`; `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_final_chain_dispatch`, so Rust fills the native live-mutation report
  internally.
- PBFT manager advance-period finalization facts now advance through
  `pbft_manager_runtime_advance_finalization_advance_period`. C++ passes only the typed
  `PbftManagerFinalizationAdvancePeriodReport` fact (`manager_period`), and Rust fills the native live-mutation report
  internally.
- PBFT manager pillar post-processing facts now use the manager-local
  `PbftManagerFinalizationPillarPostProcessingReport` and advance through
  `pbft_manager_runtime_advance_finalization_pillar_post_processing` instead of constructing
  `PbftFinalizationExternalEffectReport` in C++. The report carries only the pillar processed/request periods, and the
  shim now rejects invalid delegation-delay request-period derivation before executing the pillar side effect. The
  only remaining external failure reporting is the scalar `pbft_manager_runtime_fail_finalization_external_effect` API.
- Manager-owned PBFT finalization actions are now drained inside the boundary implementation. The drain owns
  dynamic-lambda persistence/state and executed-status persistence/state inside `BridgePbftManagerRuntime`, while
  stopping at external FinalChain/EVM, DAG, transaction-manager, PBFT-chain, sortition, vote-manager, advance-period,
  pillar, and network boundaries. The direct `pbft_manager_runtime_apply_dynamic_lambda`,
  `pbft_manager_runtime_apply_finalization_executed_status`, and public bridge-crate
  `pbft_manager_runtime_drain_owned_finalization_actions` surfaces are deleted. Duplicate-finalization resume tails
  include the paired `SetExecutedFlag` replay after executed-status persistence so Rust-owned drain completion keeps
  durable and live manager state aligned.
- PBFT manager period-data queue metadata now crosses CXX through one
  `pbft_manager_runtime_period_data_queue_snapshot` API. The separate queue period, syncing-period,
  last-block-hash-or-chain, size, and empty getters are deleted; C++ supplies only the PBFT-chain compatibility facts
  needed to compute the snapshot.
- Additional no-caller standalone PBFT runtime wrappers are retired from the CXX surface:
  `plan_pbft_sync_runtime`, `abort_pbft_manager_proposal_session`,
  `load_pbft_finalization_last_period_lambda_storage`, `plan_pbft_dynamic_lambda`,
  `pbft_manager_runtime_load_finalization_last_period_lambda`, and the bridge-only `PbftSyncRuntimePlan` DTO. Live C++
  uses manager-owned PBFT sync admission, `plan_pbft_manager_block_validation`, proposal runtime sessions, and
  `pbft_manager_runtime_plan_finalization_dynamic_lambda`; native `rustaxa-consensus` tests keep coverage for the
  deleted lower-level planners and lambda lookup.
- Direct PBFT sync admission and transaction-query planners are also retired from the CXX surface:
  `plan_pbft_sync_period_admission`, `plan_pbft_sync_transaction_query`, and their bridge-only fact/plan DTOs are
  deleted. The later runtime consolidation also deletes `plan_pbft_sync_process_period_data_runtime` and its
  `PbftSyncProcessPeriodDataRuntimeFact` DTO. Live C++ now drives the manager-owned sync-admission cursor, whose step
  carries transaction-query output only when requested; native `rustaxa-consensus` tests keep coverage for the
  lower-level admission and transaction-query planners.
- Follow-up cleanup removed the stale `PbftSyncPeriodAdmissionFact` CXX DTO that remained after the direct admission
  planner export was deleted. Admission facts now stay native to `rustaxa-consensus`; the bridge exposes only the staged
  immutable sync-admission start fact and typed cursor reports used by the live PBFT manager shim.
- Lower-level FinalChain execution API helpers that were superseded by the one-shot
  `consensus_execution_prepare_external_evm_state_commit` call are retired from the CXX surface:
  `consensus_execution_plan_publication`, `consensus_execution_attach_rewards_stats`,
  `consensus_execution_attach_proposal_period_dag_level`, `consensus_execution_next_state_commit_request`,
  `consensus_execution_persist_pending_publication`, and `consensus_execution_publication_audit`. Live C++ drives
  external EVM and `StateAPI` through the remaining `BridgeConsensusExecutionApi` methods; the CXX surface still keeps
  session creation/commit plus the minimal step/report/publish methods called by `final_chain_shim`. The obsolete
  `FinalChainExternalEvmPublicationPlan` and `FinalChainExternalEvmTransactionPublication` CXX DTOs are deleted; bridge
  tests that still inspect publication internals use native `rustaxa-consensus` publication-plan structs. The oversized
  `FinalChainExternalEvmCommitPlan` CXX DTO is also deleted; live C++ now receives only
  `FinalChainExternalEvmCommitReport` with request id, period, and error text before the one-shot state-commit
  preparation call, while bridge tests that need roots, blooms, receipts, and counters assert on the native Rust commit
  plan.
- The older direct `BridgeFinalChain::finalize_block*` compatibility exports and bridge-only `FinalizationOutcome` DTO
  are retired. FinalChain execution now crosses the bridge through `BridgeFinalChainExecutionSession` and
  `BridgeConsensusExecutionApi`, while native direct finalization remains covered in `rustaxa-consensus`.
  `create_final_chain_execution_session` no longer accepts a `BridgeFinalChain` because the session constructor only
  needs the execution request; commit, recovery, and publication APIs retain explicit `BridgeFinalChain` parameters at
  the actual storage/FinalChain boundary.
- The standalone `plan_external_evm_system_transactions` CXX export is deleted. C++ still gathers external StateAPI
  facts, but deterministic system-transaction selection and legacy RLP construction now enter through
  `BridgeConsensusExecutionApi::consensus_execution_plan_system_transactions`, keeping the execution client on the
  dedicated API surface.
- The standalone `final_chain_execution_session_commit` CXX export is deleted. Native FinalChain session commit now
  enters through `BridgeConsensusExecutionApi::consensus_execution_commit_session`, so both native and external-EVM
  execution advancement use the dedicated execution facade while commit still takes an explicit `BridgeFinalChain` at the
  storage boundary.
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
