# Consensus Consolidation Plan

This follows up on `doc/consensus_touchpoints.md`. That document is now the closed inventory of external consensus
touchpoints and facade shapes. This plan is the next cleanup wave: shrink or remove bridge/shim scaffolding that became
obsolete after the dedicated network, execution, and query APIs landed, and consolidate Rust consensus/storage usage so
internal Rust paths no longer look like C++ compatibility paths.

## Target State

- Rust consensus-internal code calls Rust consensus and `rustaxa-storage` APIs directly.
- `rustaxa-bridge` is a CXX boundary crate only. It should expose C++ compatibility adapters, not become the normal
  internal service layer for Rust consensus.
- C++ shims remain only where a public C++ API or external subsystem still exists: network/tarcap, external EVM/StateAPI,
  RPC/GraphQL/plugins, application bootstrap, and pure C++ reference compatibility.
- The storage shim remains a legacy public `DbStorage` facade, but Rust-mode consensus writes and reads should not be
  assembled from `DbStorage` calls, public bridge batches, or C++ sidecar materialization.
- Obsolete per-feature bridge modules, temporary effect queues, callback bundles, `*Old` forwarding, and compatibility
  tests are deleted once a facade or native Rust runtime owns the route.

## Non-Goals

- Do not move arbitrary EVM execution or `state_db/` mutation into Rust in this cleanup wave.
- Do not rewrite tarcap transport, peer connections, packet framing, or gossip fanout mechanics into Rust yet.
- Do not remove pure C++ reference behavior from `cpp-reference`.
- Do not delete compatibility DTO fields that still have a named external caller.

## Agent Use

Implementation slices from this plan must use `$implement-rustaxa-consensus-slice` and delegate work to the relevant
custom agents when their review or implementation scope is material:

- `api-designer`: review Rust/C++ facade shape, DTO minimality, bridge compatibility, and whether a proposed API keeps
  consensus-internal callers out of shim/bridge routes.
- `architect-reviewer`: review ownership boundaries, shim removal strategy, fallback risk, external-boundary discipline,
  and whether the slice leaves obsolete compatibility code behind.
- `rust-engineer`: implement or review Rust consensus, storage, bridge, codec, native service, and Rust test changes.
- `cpp-pro`: implement or review C++ shim, CMake, bridge wiring, RPC/GraphQL, tarcap adapter, and C++ test changes.

Delegate concrete, non-overlapping work. The primary implementer still owns integration, local code inspection,
conflict resolution, validation, deletion of obsolete scaffolding, and the final closeout report.

## Current Cleanup Pressure Points

- `rust/crates/rustaxa-bridge/src/lib.rs` still exposes many per-feature bridge modules that mirror Rust consensus
  modules one-to-one.
- `rust/crates/rustaxa-bridge/src/ffi.rs` still contains broad storage query/batch APIs and many module-specific bridge
  handles beyond the three external facades.
- `libraries/core_libs/consensus/shims/*` contains many overlay classes that should become thin C++ public facades or
  disappear once their public API is no longer needed in Rust mode.
- `rust/crates/rustaxa-consensus/src/network_api.rs` no longer exposes the temporary CXX
  `consensus_network_queue_*` bridge helpers. Remaining cleanup pressure is internal effect-drain plumbing, especially
  PBFT vote gossip through `drain_work` / `report_effect_results` while tarcap still owns transport execution.
- RPC and GraphQL now use `ConsensusQueryApi`, but many endpoints construct query APIs from `node->getDB()->rustStorage()`
  locally instead of receiving one injected public-query handle.
- Some Rust consensus APIs still accept `BridgeStorage`, `BridgeFinalChain`, or bridge runtime handles where they should
  accept native Rust storage/final-chain ports inside Rust and only convert at CXX entry points.

## Slice 0: Baseline Audit and Deletion Map

Purpose: produce a mechanical map of bridge/shim code that is still live before deleting anything.

Work:

- Generate a checked-in audit table listing every remaining `rustaxa-bridge` module, every `Bridge*` handle exported
  through CXX, every consensus shim directory, and its current consumers.
- Classify each item as:
  - external boundary: keep, but narrow
  - C++ public compatibility facade: keep until caller migration
  - internal Rust route: must move out of bridge/shim
  - obsolete scaffold: delete in the owning slice
- Add `rg`-based closeout checks for:
  - `Old::` forwarding in Rust-enabled shims
  - `consensus_network_queue_`
  - direct `create_consensus_query_api(...rustStorage())` outside API construction points
  - `BridgeStorage` usage from Rust consensus modules
  - public `rustBatchId` or bridge-batch usage outside storage compatibility code

Acceptance:

- New audit file links each bridge/shim item to a removal condition.
- No behavior changes.
- `git diff --check` passes.

Implementation status:

- Complete in `doc/consensus_bridge_shim_audit.md`.
- The audit classifies Rust bridge modules, exported `Bridge*` handles, consensus shim directories, consumers, and
  removal/narrowing conditions.
- Required `rg` closeout checks are recorded in the audit file and should be rerun after each deletion/consolidation
  slice.

## Slice 1: Centralize Public Query API Injection

Purpose: stop RPC/GraphQL callers from repeatedly constructing `ConsensusQueryApi` from `DbStorage` and make public-query
compatibility edges obvious.

Work:

- Create one Rust-mode application-owned `BridgeConsensusQueryApi` handle during app/RPC/GraphQL wiring.
- Inject that handle into Taraxa RPC, ETH RPC, Debug RPC, Test RPC, GraphQL `Query`, and GraphQL child object reader
  factories.
- Remove endpoint-local `rustaxa::create_consensus_query_api(node->getDB()->rustStorage())` construction.
- Keep local reader callback bundles only when they adapt external account/state reads or public formatting.
- Move any remaining storage-backed public read that still uses `DbStorage`, `FinalChain`, `DagManager`,
  `TransactionManager`, or `PbftManager` into `ConsensusQueryApi` or mark it as external-state compatibility.

Removal targets:

- Repeated query API construction in `libraries/core_libs/network/rpc/*.cpp`,
  `libraries/core_libs/network/rpc/eth/Eth.cpp`, and `libraries/core_libs/network/graphql/src/query.cpp`.
- Reader callbacks whose only job is to create a query API from `DbStorage`.

Acceptance:

- Public consensus-storage reads enter through the injected query facade.
- Direct public query access to `DbStorage` is limited to non-Rust fallback branches or external-state adapters.
- Focused `rpc_test` and GraphQL tests pass.

Implementation status:

- Shared C++ boundary handle: `taraxa::net::ConsensusQueryApiPtr` in `network/consensus_query.hpp`.
- Rust-enabled `Rpc::start()` now creates one app/RPC-owned consensus query handle from `DbStorage` and shares it with:
  - ETH RPC callback wiring
  - Taraxa RPC default readers and public Rust-mode query methods
  - Debug RPC default readers
  - Test RPC default readers and public Rust-mode status/sortition methods
  - GraphQL `Query` consensus reader and compatibility block/DAG reader fallbacks
- Endpoint-local construction of `rustaxa::create_consensus_query_api(...rustStorage())` has been removed from
  `libraries/core_libs/network/rpc`, `libraries/core_libs/network/graphql`, `libraries/plugin/rpc`, and
  `libraries/plugin/light`.
- Compatibility reader bundles remain for non-Rust fallback, public formatting, live status, account/state reads,
  transaction submission, and external EVM/StateAPI boundaries.
- Custom agents used:
  - `api-designer`: confirmed shared immutable query facade shape and recommended callback-only ETH wiring.
  - `architect-reviewer`: confirmed app/RPC bootstrap as the right owner and highlighted compatibility boundaries.
  - `rust-engineer`: confirmed `BridgeConsensusQueryApi` is immutable/read-only and safe to share from C++.
  - `cpp-pro`: mapped constructor/call-site changes and test targets.
- Validation:
  - `cmake --build /build --target rpc_test --parallel 12`
  - `/build/bin/rpc_test --gtest_filter='RPCTest.test_node_status_uses_status_reader:RPCTest.test_sortition_change_uses_sortition_reader:RPCTest.graphql_query_status_fields_use_injected_readers:RPCTest.graphql_query_blocks_use_consensus_query_reader:RPCTest.graphql_query_dag_blocks_use_query_dag_block_reader:RPCTest.graphql_query_transaction_uses_query_transaction_reader:RPCTest.graphql_query_blocks_use_query_block_reader' --gtest_print_time=1`

## Slice 2: Retire Temporary Network Effect Queue Helpers

Purpose: reduce `ConsensusNetworkApi` to packet ingress, deterministic planning, and truly external network effects.

Work:

- Delete or deprecate `consensus_network_queue_*_admission_request_effects` routes that now have direct tarcap or manager
  execution paths.
- Keep only effects that represent real network-owned actions: send packet, gossip packet, request sync, report peer,
  disconnect peer, and peer ordering.
- Replace stale tests in `tests/rust/consensus/test_network_api.cpp` and Rust unit tests that only assert temporary
  record/admission effects.
- Move remaining packet-adjacent deterministic decisions into named planner methods rather than effect-queue methods.

Initial removal candidates:

- PBFT vote admission request effects.
- PBFT block admission record effects.
- transaction admission request effects.
- DAG block and DAG sync admission request effects.
- PBFT sync period-data admission request effects.
- pillar vote record/validation request effects when direct pillar manager validation is authoritative.

Keep for now:

- PBFT vote gossip as a direct network/tarcap bridge command backed by `NetworkEffect` drain/report while tarcap owns
  peer filtering, packet wrapping, and transport.
- Status, PBFT sync-start, pending-DAG, and max-chain peer-selection planners until the network facade owns a live
  status snapshot port.

Acceptance:

- `ConsensusNetworkApi` no longer exposes queue helpers for behaviors that execute synchronously in tarcap.
- Effect result acknowledgement remains only for effects the network actually executes.
- `test_network_api.cpp` coverage describes current production semantics, not retired scaffolding.

Implementation status:

- Complete for bridge-helper retirement.
- Removed the CXX `consensus_network_queue_*` methods and queue-only DTOs from
  `rust/crates/rustaxa-bridge/src/ffi.rs` and `rust/crates/rustaxa-bridge/src/network.rs`.
- Removed matching stale bridge tests from `tests/rust/consensus/test_network_api.cpp`.
- Removed Rust-internal record/admission queue DTOs, helper methods, and unit tests from
  `rust/crates/rustaxa-consensus/src/network_api.rs`.
- PBFT vote gossip now enters through `consensus_network_gossip_pbft_vote` / `gossip_pbft_vote`; this is the remaining
  direct external network command using `NetworkEffect` drain/report until tarcap transport can be narrowed further.
- Custom agents used:
  - `api-designer`: reviewed the minimal network/tarcap API shape and recommended deleting queue-named bridge exports.
  - `architect-reviewer`: reviewed boundary ownership and confirmed the remaining direct gossip/effect boundary is the
    right temporary external seam.
  - `rust-engineer`: mapped Rust bridge/domain queue helpers and tests for deletion.
  - `cpp-pro`: confirmed production C++ callsites and reviewed the C++ rename/bridge-test risk.
- Validation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus network_api`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge network`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='ConsensusNetworkApiBridgeTest.*' --gtest_print_time=1`
  - `git diff --check`

## Slice 3: Collapse Storage Bridge Queries into Native Rust Ports

Purpose: make `rustaxa-consensus` depend on `rustaxa-storage` abstractions directly instead of calling bridge-shaped
storage query helpers.

Work:

- Identify consensus modules that still use CXX bridge storage query handles indirectly through bridge wrappers.
- Move reusable storage accessors into `rustaxa-storage` or consensus-owned port implementations.
- Replace Rust-side bridge query helpers with direct `Storage`/domain repository calls.
- Keep CXX `BridgeStorage` query methods only for legacy C++ compatibility shims and tests that explicitly exercise the
  public storage facade.
- Delete bridge query constructors when no C++ shim uses them.

High-value areas:

- PBFT manager startup/replay and transition storage.
- transaction manager sidecar/runtime storage lookups.
- PBFT finalization session storage queries.
- FinalChain publication/audit storage queries.
- DAG manager runtime graph restore and DAG block period lookup.

Acceptance:

- No `BridgeStorage` type is referenced from `rustaxa-consensus`.
- `rustaxa-bridge` storage query APIs exist only for C++ shims or compatibility tests.
- `cargo test -p rustaxa-consensus`, `cargo test -p rustaxa-storage`, and `rust_storage_tests` pass.

Implementation status:

- Current audit confirms `rustaxa-consensus` has no `BridgeStorage`, `BridgeStorageBatch`, `rustBatchId`, or
  `Bridge*StorageQueries` references.
- Remaining storage query handles live in `rustaxa-bridge` compatibility modules, C++ storage/query shims, and bridge or
  conformance tests. They are still valid compatibility debt until the corresponding C++ public facades move to native
  Rust services.
- Gas-pricer finalized-history restoration is construction-time-only through `create_gas_pricer_from_storage`; the
  obsolete `gas_pricer_init_from_storage` storage injection helper has been deleted from the Rust bridge tests and
  implementation.
- Validation for the gas-pricer storage-injection cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge gas_pricer`

## Slice 4: Thin the Storage Shim to a Compatibility Shell

Purpose: make the C++ `DbStorage` shim visibly legacy-facing and remove bridge batch usage from Rust-owned operations.

Work:

- Audit `libraries/core_libs/consensus/shims/storage_shim` for methods that are now only used by Rust-mode consensus
  internals.
- Move those callers to native Rust storage handles or dedicated consensus APIs.
- Keep public `DbStorage` methods for legacy C++ callers and pure C++ reference compatibility.
- Remove public bridge-batch helpers from non-storage shim call sites.
- Strengthen `scripts/rewrite_storage_boundary_guard.sh` to reject new Rust-mode consensus uses of storage shim batch
  helpers outside the storage compatibility shell.

Acceptance:

- Rust-mode consensus code does not assemble write batches through C++ `DbStorage` or public bridge-batch helpers.
- Storage shim comments and guard exceptions name only external compatibility callers.
- Storage conformance and targeted storage bridge tests pass.

Implementation notes:

- C++ review confirmed `create_storage_shim_batch` and `BridgeStorageBatch` are confined to `storage_shim` internals, and
  `rustBatchId` no longer has code callsites.
- Slice 4 storage-shim direct-mutator cleanup is complete for the audited compatibility paths. Remaining public
  `DbStorage::createWriteBatch()` / `commitWriteBatch()` blocks in original consensus modules must be classified by
  their active Rust-mode overlay before adding new storage-shim APIs.
- The batch-only conversion is implemented for DAG block save/remove, status fields, PBFT manager fields/status, PBFT
  heads, own verified votes, 2t+1 vote bundles, extra reward votes, proposal-period DAG-level mappings, and cert-voted
  block writes/removal. These methods now use a private `DbStorage::commitImmediateRustBatch` helper, typed
  `storage_shim_*` appenders, and the existing Rust-owned batch commit path.
- Cert-voted block writes now have a native `rustaxa-storage` in-batch writer that preserves the legacy `[round, block]`
  RLP wrapper while allowing the C++ storage shim to stage the write through `BridgeStorageBatch` instead of calling the
  broad `BridgeStorage` mutator directly.
- Genesis-hash writes now route through a dedicated `storage_shim_set_genesis_hash` API that preserves
  `rustaxa-storage` write-if-empty semantics without exposing the broad `BridgeStorage::set_genesis_hash` mutator to the
  C++ storage shim. The obsolete broad `BridgeStorage::set_genesis_hash` CXX export has been deleted.
- Block-reward stats clearing now routes through a dedicated `storage_shim_clear_block_rewards_stats` API. Rust storage
  still owns the aggregate row-by-row delete and native batch commit. The obsolete broad
  `BridgeStorage::clear_block_rewards_stats` CXX export has been deleted.
- The tracked direct `BridgeStorage` mutator cleanup for storage-shim single-write and aggregate-clear compatibility
  paths is complete. Remaining Slice 4 work should classify original consensus-module public batch blocks as either
  active Rust-mode gaps or legacy/reference code behind an authoritative overlay.
- PBFT manager reset, finish-polling, loopback-finish, and finalization public batch blocks are closed for the current
  Rust-mode route. The active `pbft_manager_shim` overlay overrides those methods and routes transition persistence
  through `pbft_manager_runtime_apply_transition_storage_write`, which commits manager cursor/status updates,
  cert-voted-block removal, and own-verified-vote cleanup in one native Rust storage batch before updating runtime and
  C++ mirrors. Executed-block reset and finalization storage writes are also Rust-owned runtime/finalization calls with
  explicit external boundaries for finalization execution and sidecar materialization. The public batch blocks in
  `libraries/core_libs/consensus/src/pbft/pbft_manager.cpp` remain legacy/reference behavior behind
  `RUSTAXA_ENABLE_PBFT_MANAGER`.
- Proposed-block persistence is closed for the current Rust-mode route. The active `proposed_blocks_shim` overlay owns
  save, startup restore, and stale-period cleanup through `BridgeProposedBlocks`; storage-backed cleanup plans stale
  period/hash groups, commits one native Rust storage delete batch, and mutates the Rust index only after commit. The
  public batch loop in `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` remains legacy/reference behavior
  behind `RUSTAXA_ENABLE_PROPOSED_BLOCKS` and should not drive new storage-shim API expansion.
- Sortition parameter persistence is closed for the current Rust-mode route. The active
  `sortition_params_manager_shim` overlay constructs `BridgeSortitionParamsManager` from Rust storage, persists the
  missing period-zero default change in Rust during startup, ignores the legacy `Batch&` argument in `pbftBlockPushed`,
  and persists emitted finalized-period changes through the Rust runtime before live state is updated. The public batch
  block in `libraries/core_libs/consensus/src/dag/sortition_params_manager.cpp` remains legacy/reference behavior behind
  `RUSTAXA_ENABLE_SORTITION_PARAMS`.
- FinalChain block publication is closed for the current Rust-mode route. The active `final_chain_shim` overlay is a
  standalone facade over `BridgeFinalChain` and `BridgeConsensusExecutionApi`; native finalization, external-EVM pending
  publication markers, recovery, storage publication, execution counters, rewards-stat attachment, transaction indexes,
  receipts, log blooms, and genesis header creation are committed through native Rust storage. The public batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` remain legacy/reference behavior behind
  `RUSTAXA_ENABLE_FINAL_CHAIN`; Rust mode keeps the external `StateAPI`/EVM adapter but does not route FinalChain
  storage publication through C++ `DbStorage` batches.
- Custom agents used for the current storage-boundary audit:
  - `rust-engineer`: confirmed `rustaxa-consensus` is free of `BridgeStorage` and identified direct storage-shim mutators
    that can be converted to typed batch appenders.
  - `cpp-pro`: confirmed bridge batch use is currently storage-shim-local and mapped the remaining public `DbStorage`
    batch callsites in original consensus modules.
  - `rust-engineer`: confirmed own and extra reward vote batch appenders are low-risk additions, while genesis,
    block-reward clear, and cert-voted block should remain direct until native storage support exists.
  - `cpp-pro`: confirmed the private immediate-commit helper shape and the C++ call paths suitable for conversion.
- Validation for the first batch-only conversion:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge storage`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-storage pbft`
  - `cmake --build /build --target rust_storage_tests --parallel 12`
  - `/build/bin/rust_storage_tests`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `cmake --build /build --target dag_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.propose_block_and_vote_broadcast' --gtest_print_time=1`
  - `/build/bin/dag_test --gtest_filter='DagTest.receive_block_in_order:DagTest.build_dag' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `.githooks/pre-commit`

## Slice 5: Retire Small Completed Consensus Shims

Purpose: delete small overlay shims where Rust implementation is already authoritative and remaining C++ surface is only
legacy public API compatibility.

Work:

- For each small shim, check whether all Rust-enabled production methods call Rust directly and whether `*Old` forwarding
  is only parity scaffold.
- Move any remaining public compatibility materialization to a single adapter function or facade.
- Delete the shim overlay and module-specific CMake flag when no Rust-mode caller needs the C++ class.
- Otherwise, replace inherited/forwarding behavior with explicit stubs plus documented removal condition.

Candidate order:

1. `period_data_queue_shim`
2. `verified_votes_shim`
3. `pillar_votes_shim`
4. `transaction_queue_shim`
5. `gas_pricer_shim`
6. `rewards_stats_shim`
7. `sortition_params_manager_shim`
8. `slashing_manager_shim`

Acceptance:

- Each slice removes at least one shim directory, one CMake overlay block, or one complete `*Old` forwarding family.
- Equivalent Rust or bridge-level tests remain for moved behavior.
- `make cpp-intersection-list` does not grow from unnecessary original upstream edits.

Implementation notes:

- `period_data_queue_shim` is retired. Period-data queue metadata now lives in `BridgePbftManagerRuntime` and is exposed
  only through `pbft_manager_runtime_period_data_queue_*` methods used by `pbft_manager_shim`.
- The standalone `BridgePeriodDataQueue` CXX handle, `create_period_data_queue` constructor, `period_data_queue_shim`
  overlay, `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE` CMake/Makefile flag, and bridge/shim tests for the retired facade were
  deleted.
- `BridgePbftSyncQueueDrainSession` is retired. PBFT sync queue-drain planner state now lives inside the long-lived
  `BridgePbftManagerRuntime`, and `pbft_manager_shim::pushSyncedPbftBlocksIntoChain()` resets and drives that
  runtime-owned planner through `pbft_manager_runtime_begin_pbft_sync_queue_drain`,
  `pbft_manager_runtime_pbft_sync_queue_drain_next`, and
  `pbft_manager_runtime_pbft_sync_queue_drain_report`.
- `pbft_manager_shim` keeps a temporary sidecar deque for live `PeriodData`, `PbftVote`, and peer objects. Rust owns the
  queue admission/order/pop/cleanup metadata; the sidecar deque should disappear when those payload model types move to
  Rust.
- `vote_manager_shim::setNetwork` no longer forwards through `VoteManagerOld`; it writes the inherited protected network
  pointer directly. This removes one completed setter forwarding without changing the public C++ API.
- `transaction_manager_shim::getTransactionsMutex` no longer forwards through `TransactionManagerOld`; it is now a
  shim-owned method that returns the same inherited mutex through the existing friend access helper. Transaction lifecycle
  synchronization is still temporary inherited-state debt until the transaction runtime owns the lifecycle lock.
- `dag_manager_shim::setNetwork` remains documented temporary compatibility debt: the shim has its own network pointer,
  while the legacy base still has private network state that may be read if an inherited base path executes. Remove that
  forwarding only with the broader DAG manager runtime/service consolidation.
- Replacement bridge coverage is in the Rust `rustaxa-bridge` PBFT manager runtime test for period-data queue metadata,
  the Rust `rustaxa-bridge` PBFT manager runtime test for queue-drain planner ownership, plus the existing Rust
  `rustaxa-consensus` period-data queue and PBFT sync queue-drain domain tests.
- Full `gas_pricer_shim` removal is not valid yet. Removing the overlay would route Rust-enabled builds back to the
  legacy C++ implementation instead of preserving Rust ownership. A future removal must first replace the C++ public
  facade with a native transaction/final-chain runtime API or a narrower external query API. The first gas-pricer cleanup
  was therefore recorded as a Slice 8 CXX surface shrink instead of a Slice 5 shim retirement.
- Custom agents used:
  - `rust-engineer`: confirmed Slice 5 bridge handles are still required by C++ public facade surfaces and recommended
    gas-pricer narrowing instead of handle deletion.
  - `cpp-pro`: mapped small-shim CMake/removal candidates; its full `gas_pricer_shim` deletion recommendation was
    rejected because it would re-center Rust-mode pricing in legacy C++.
  - `architect-reviewer`: recommended retiring `period_data_queue_shim` by moving queue ownership into the PBFT manager
    runtime, with sidecar lockstep and PBFT sync drain behavior as the primary risks.
  - `reviewer`: reviewed the final period-data queue consolidation for stale references, sidecar risks, and validation
    coverage before closeout.
- Validation run:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge bridge_runtime_owns_period_data_queue_metadata`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge bridge_runtime_owns_pbft_sync_queue_drain_session`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_sync`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus period_data_queue`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.PeriodAdmissionPlan*:RustPbftSyncTest.ProcessPeriodRuntime*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.propose_block_and_vote_broadcast' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `git diff --check`
  - `.githooks/pre-commit`

## Slice 6: Consolidate Runtime-Heavy Shims into Rust Application Services

Purpose: shrink the large shims that still orchestrate Rust runtimes from C++ and carry too much consensus logic.

Work:

- Define Rust-owned application service handles for:
  - DAG runtime
  - transaction manager runtime
  - PBFT manager runtime
  - FinalChain runtime
  - pillar-chain runtime
- Move multi-step session orchestration from C++ shims into Rust services where all dependencies are already Rust-owned.
- Keep C++ shims as public API shells that translate inputs, call one Rust service method, and materialize legacy return
  values only when needed by external callers.
- Replace `rustFinalChainForRust()`-style cross-shim handle sharing with native Rust service composition.
- Remove `BridgeFinalChain` and `BridgeStorage` parameters from Rust consensus bridge functions when the service already
  owns those dependencies.

Priority order:

1. `final_chain_shim`: keep `ExternalEvmStateApiClient` as the explicit external adapter, move all other session
   ownership behind Rust FinalChain service methods.
2. `transaction_manager_shim`: collapse sidecar/runtime split and FinalChain bridge calls into a Rust transaction
   service with native final-chain/storage ports.
3. `dag_manager_shim`: move graph restore, proposal planning, and period lookup into native Rust DAG service methods.
4. `pbft_manager_shim`: split remaining C++ orchestration by PBFT manager lifecycle, proposal, validation, sync, and
   finalization service methods; delete bridge session types as each group moves.
5. `pillar_chain_manager_shim`: move validator/vote planning and storage access into a Rust pillar-chain service.

Acceptance:

- C++ shims no longer pass one bridge handle into another bridge handle for internal consensus dependencies.
- Rust services own their storage/final-chain dependencies through native Rust structs or traits.
- Large shim methods become input conversion plus one Rust call plus output conversion.

## Slice 7: Narrow External Execution API and StateAPI Adapter

Purpose: keep the EVM boundary external while removing consensus logic and storage publication from the C++ adapter.

Work:

- Ensure `FinalChain::ExternalEvmStateApiClient` contains only:
  - external EVM transaction execution
  - rewards execution
  - `state_db/` commit
  - committed account/storage/code reads
  - bridge-contract reads
  - dry-run/trace calls
- Move any remaining request identity, publication, audit, restart recovery, rewards-stat attachment, and final-chain
  storage decisions into `ConsensusExecutionApi` or native Rust FinalChain service methods.
- Replace temporary `FinalizationResult` materialization in consensus-internal flow with a Rust DTO and materialize
  legacy C++ only at public API boundaries.

Acceptance:

- No consensus-internal method calls `StateAPI` directly.
- The execution adapter has no `DbStorage` publication or FinalChain storage responsibilities.
- External EVM tests and final-chain Rust bridge tests pass.

## Slice 8: Shrink CXX FFI Surface and Module Flags

Purpose: make the bridge small enough to audit.

Work:

- After each consolidation slice, delete unused `Bridge*` structs, functions, constants, and tests from
  `rust/crates/rustaxa-bridge/src/ffi.rs` and module files.
- Collapse per-module feature flags that no longer select independently useful Rust shims.
- Keep only:
  - `BridgeConsensusNetworkApi`
  - `BridgeConsensusExecutionApi`
  - `BridgeConsensusQueryApi`
  - application/bootstrap handles
  - C++ public compatibility handles with active callers
- Add a bridge API inventory check that fails when a new `Bridge*` type is exported without an entry in the audit table.

Acceptance:

- `rustaxa-bridge` no longer mirrors every `rustaxa-consensus` module by default.
- CXX exports are explainable from the external boundary inventory or the compatibility audit table.
- `cargo check -p rustaxa-bridge` and affected C++ build targets pass.

Implementation status:

- Bridge export inventory guard is implemented as `scripts/rewrite_bridge_inventory_guard.sh` and wired into
  `make rewrite-validate-fast` through `make rewrite-bridge-inventory-guard`.
- The guard compares CXX `type Bridge*;` exports in `rust/crates/rustaxa-bridge/src/ffi.rs` to the exported-handle table
  in `doc/consensus_bridge_shim_audit.md`, fails on undocumented exported handles, and warns on stale audit rows after
  deletions.
- `BridgeGasPricer` CXX exports have been narrowed: `gas_pricer_init_from_storage` is no longer exported because
  Rust-mode storage history restoration is owned by `create_gas_pricer_from_storage`.
- The obsolete test-only `BridgeGasPricer::gas_pricer_init_from_storage` helper has also been removed; bridge tests now
  exercise `create_gas_pricer_from_storage` directly.
- `BridgeStorage` CXX exports have been narrowed: the obsolete broad `set_genesis_hash` mutator is deleted because
  Rust-mode `DbStorage::setGenesisHash` uses the dedicated `storage_shim_set_genesis_hash` compatibility API.
- The storage conformance runner now also seeds genesis through `storage_shim_set_genesis_hash`, so it no longer depends
  on the deleted broad `BridgeStorage::set_genesis_hash` method.
- `BridgeStorage` CXX exports have been narrowed further: the obsolete broad `clear_block_rewards_stats` mutator is
  deleted because Rust-mode `DbStorage::deleteColumnData(block_rewards_stats)` uses the dedicated
  `storage_shim_clear_block_rewards_stats` compatibility API.
- Additional unused broad `BridgeStorage` CXX mutators have been deleted after dedicated storage-shim paths became the
  production route: `save_block_rewards_stats`, `remove_cert_voted_block_in_round`, `remove_own_verified_vote`,
  `remove_extra_reward_vote`, and `replace_two_t_plus_one_votes`.
- The unused broad `BridgeStorage::save_sortition_params_change` CXX mutator has been deleted. Rust bridge tests now seed
  sortition params through native `rustaxa-storage` metadata writes, while the C++ `DbStorage` compatibility shell keeps
  the dedicated `storage_shim_save_sortition_params_change` batch appender.
- The last C++ test fixture caller of broad `BridgeStorage::save_extra_reward_vote` has been migrated to the dedicated
  `storage_shim_save_extra_reward_vote` batch appender, and the broad CXX mutator is deleted.
- The remaining test-only callers of broad `BridgeStorage::save_own_verified_vote` have been migrated to either the
  dedicated `storage_shim_save_own_verified_vote` batch appender or native `rustaxa-consensus` vote persistence helpers,
  and the broad CXX mutator is deleted.
- The remaining test-only callers of broad `BridgeStorage::persist_pbft_vote_progress` and
  `BridgeStorage::clear_own_verified_votes` now use the narrower `BridgeVerifiedVotes` persistence facade, and the broad
  CXX methods are deleted from `BridgeStorage`.
- The last test-only callers of broad `BridgeStorage::save_cert_voted_block_in_round` have been migrated to either the
  dedicated `storage_shim_save_cert_voted_block_in_round` batch appender or native Rust PBFT manager storage helpers, and
  the broad CXX mutator is deleted.
- The storage conformance caller of broad `BridgeStorage::save_pbft_head` now seeds through the dedicated
  `storage_shim_save_pbft_head` batch appender, and the broad CXX mutator is deleted.
- The remaining callers of broad `BridgeStorage::save_pbft_block_period` now seed through either
  `storage_shim_save_pbft_block_period` or native Rust period storage, and the broad CXX mutator is deleted.
- The storage conformance caller of broad `BridgeStorage::save_rounds_count_dynamic_lambda` now seeds through
  `storage_shim_save_rounds_count_dynamic_lambda`, and the broad CXX mutator is deleted.
- The remaining callers of broad `BridgeStorage::save_period_lambda` now seed through either
  `storage_shim_save_period_lambda` or native Rust metadata storage, and the broad CXX mutator is deleted.
- The remaining callers of broad `BridgeStorage::save_status_field`, `save_pbft_mgr_field`, and
  `save_pbft_mgr_status` now seed through dedicated storage-shim batch appenders or native Rust storage repositories,
  and the broad CXX mutators are deleted.
- The remaining callers of broad `BridgeStorage::save_dag_block`, `remove_dag_block`,
  `save_proposal_period_dag_levels_map`, and `save_dag_block_period` now seed through dedicated storage-shim batch
  appenders or native Rust DAG repositories, and the broad CXX mutators are deleted.
- The remaining callers of broad `BridgeStorage::save_period_data` now seed through the dedicated storage-shim batch
  appender or native Rust period storage, and the broad CXX mutator is deleted.
- The remaining callers of broad `BridgeStorage::save_transaction`, `remove_transaction`, `save_transaction_location`,
  `save_system_transaction`, and `save_period_system_transactions_hashes` now seed through dedicated storage-shim batch
  appenders or native Rust transaction repositories, and the broad CXX mutators are deleted.
- `BridgeStorage::save_non_finalized_transactions` is also deleted. Older transaction-manager bridge paths now call the
  native `rustaxa-consensus` transaction storage helper directly, matching the runtime commit path and preserving the
  atomic accepted-transaction RLP plus `TrxCount` write group without a broad `BridgeStorage` mutator.
- The unused CXX `BridgePbftVotePipelineSession` and `BridgePbftVoteAdmissionSession` exports are deleted. Their wrapper
  modules only protected bridge-shaped test scaffolding; production C++ had no callsites, and native
  `rustaxa-consensus` vote pipeline/admission tests now own the behavior coverage.
- `BridgePbftManagerStateActionEffectSession` is deleted. The C++ PBFT manager shim still executes live vote/block
  side effects, but the ordered state-action transcript is now a cursor inside `BridgePbftManagerRuntime`, reducing the
  PBFT manager CXX session surface by one internal handle.
- `BridgePbftManagerRuntimeSession` is deleted. The outer PBFT manager daemon-loop transcript is now a cursor inside
  `BridgePbftManagerRuntime`, so the scheduler no longer creates a standalone bridge handle each tick.
- `BridgePbftManagerProposalSession` is deleted. PBFT block proposal planning is now a cursor inside
  `BridgePbftManagerRuntime`, so `pbft_manager_shim` no longer creates a standalone bridge handle for proposal
  construction.
- `BridgePbftManagerBlockValidationSession` is deleted. PBFT block validation planning is now a cursor inside
  `BridgePbftManagerRuntime`, so `pbft_manager_shim` no longer creates a standalone bridge handle for validation
  checks.
- `BridgeDagVerifyBlockSession` is deleted. DAG block verification still reports live external facts from C++ for
  transaction lookup, FinalChain authorization, VDF verification, and gas estimation, but the ordered cursor now lives
  inside `BridgeDagManagerRuntime` through runtime-owned begin/next/report functions. `dag_manager_shim::verifyBlock`
  no longer allocates a standalone bridge session handle.
- `BridgeDagProposerSession` is deleted. DAG proposal attempts still report live external facts and executor outcomes
  from C++ for transaction packing, VDF work, signing/materialization, and add-block execution, but the ordered cursor
  now lives inside `BridgeDagManagerRuntime` as a keyed per-attempt cursor through runtime-owned begin/next/report
  functions. `dag_block_proposer_shim` no longer allocates a standalone bridge session handle for each attempt, and
  concurrent per-wallet proposal attempts keep separate runtime cursor ids.
- `BridgeDagProposerRetryState` is deleted. Per-wallet DAG proposer retry state moved into `BridgeDagManagerRuntime`,
  keyed by wallet VRF public key, so `dag_block_proposer_shim` no longer snapshots or applies retry state through a
  standalone bridge handle.
- `BridgeDagManagerState` is deleted. The unused storage-free DAG manager state handle and its CXX methods were removed;
  live DAG manager state is owned by `BridgeDagManagerRuntime`, while standalone graph compatibility remains isolated in
  `BridgeDagGraph`.
- No-caller CXX exports for standalone DAG helper planners (`dag_derive_frontier`,
  `dag_validate_pivot_tips_metadata`), PBFT-chain storage restore (`restore_pbft_chain_storage`), and the old
  fact-shaped transaction-manager runtime known check (`transaction_manager_runtime_is_transaction_known`) are deleted.
  Live callers use runtime-owned DAG methods, `create_pbft_chain_from_storage`, and the hash-only transaction-manager
  runtime known check instead.
- `BridgePbftVoteValidationRuntime` is deleted. The standalone validation replay/threshold runtime had no external C++
  callsites and only protected older bridge tests; production Rust-mode vote validation uses `BridgeVerifiedVotes`, whose
  admission runtime owns replay protection, threshold caching, verified-vote metadata, and retained payloads together.
- Standalone PBFT vote planner CXX exports are deleted:
  `pbft_vote_progress_plan_precheck`, `pbft_vote_progress_plan_after_add`, `pbft_vote_ingress_plan`,
  `pbft_vote_bundle_ingress_plan`, `pbft_reward_votes_plan`, `pbft_vote_validation_plan`,
  `pbft_validate_canonical_vote`, `pbft_vote_event_fact_from_canonical_vote`, and
  `pbft_derive_vote_progress_fact_from_canonical_vote`. Live C++ ingress now uses `BridgeConsensusNetworkApi`, live
  validation/admission/reward-vote materialization uses `BridgeVerifiedVotes`, and the bridge-only DTOs/modules that
  existed solely for the removed free functions are deleted. `pbft_inspect_canonical_vote`, vote payload conversion, and
  vote generation helpers remain because `vote_manager_shim` and `slashing_manager_shim` still call them directly.
- The no-caller scalar threshold helper `pbft_vote_sortition_threshold_for_bridge` is also deleted from the CXX surface.
  Native `rustaxa-consensus` keeps `pbft_vote_sortition_threshold` for validation, threshold planning, and vote
  generation; live C++ proposer screening still uses `pbft_proposer_sortition_plan`.
- `BridgeTransactionQueue` CXX exports have been narrowed to the live `transaction_queue_shim` facade methods. No-caller
  queue-only planning/hash-view exports and bridge wrapper methods are deleted; native `rustaxa-consensus` transaction
  queue tests keep planner coverage.
- `BridgeTransactionManagerRuntime` CXX exports have been narrowed further: old no-caller sidecar lookup/finish/evict
  helpers, queue erase/get/order/known helpers, and sidecar size/remove helpers are deleted now that live
  `transaction_manager_shim` routing uses runtime-owned command and lookup APIs.
- `BridgeTransactionManagerRuntime` no-caller transaction-manager exports are narrowed again:
  `transaction_manager_runtime_pack_begin`, `transaction_manager_runtime_gas_estimation_cache_size`, and
  `transaction_manager_runtime_insert_recovery_entries` are deleted from the CXX surface. Live C++ transaction packing
  uses `transaction_manager_runtime_pack_begin_sharded`, gas-estimation cache behavior is observable through the planner
  result, and non-finalized recovery uses the Rust-owned `transaction_manager_recover_nonfinalized_with_runtime` command.
- Transaction-manager non-finalized recovery loaders are no longer CXX exports:
  `transaction_manager_load_nonfinalized_recovery`, `transaction_manager_load_nonfinalized_recovery_inputs`,
  `TransactionManagerRecoveryEntry`, and `TransactionManagerSidecarRecoveryInsertInput` are deleted from the bridge
  surface. C++ recovery uses the single high-level `transaction_manager_recover_nonfinalized_with_runtime` command while
  Rust bridge tests exercise native recovery helpers directly.
- Transaction-manager storage lookup helpers are no longer CXX exports:
  `transaction_manager_load_stored_transactions`, `transaction_manager_load_proposal_transactions_with_final_chain`,
  `TransactionManagerStoredTransactionRequest`, and `TransactionManagerStoredTransactionLookup` are deleted from the
  bridge surface. Live C++ lookup paths use the runtime-owned transaction view APIs that combine queue, sidecar, storage,
  and proposal-period filtering in one command.
- Transaction-manager direct queue/sidecar test helpers are no longer CXX exports:
  `transaction_manager_runtime_insert_non_finalized`, `transaction_manager_runtime_contains_non_finalized`,
  `transaction_manager_runtime_contains_recently_finalized`, `transaction_manager_runtime_apply_finalized_transition`,
  `transaction_manager_runtime_queue_insert`, `transaction_manager_runtime_insert_transaction_precheck`, and
  `transaction_manager_runtime_queue_contains` are deleted from the bridge surface. Live C++ uses the runtime-owned
  admission, initialization, lookup, cleanup, and size APIs instead.
- Older transaction-manager FinalChain-backed bridge shortcuts are no longer CXX exports:
  `transaction_manager_runtime_execute_transaction_admission_with_final_chain_command_report`,
  `transaction_manager_runtime_execute_public_transaction_admission_with_final_chain_command_report`,
  `transaction_manager_runtime_queue_cleanup_with_final_chain`,
  `save_transactions_from_dag_block_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block_command_report_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block`, `update_finalized_transactions_status`, and
  `transaction_manager_verify_not_finalized_with_runtime_and_final_chain` are deleted from the bridge surface. Live C++
  admission uses fact-backed external-FinalChain facts, DAG transaction persistence uses the runtime-owned command
  report, and finalized-status cleanup enters through the high-level runtime command.
- DAG manager bridge-test-only sync selection helpers are no longer CXX exports:
  `dag_manager_runtime_non_finalized_sync_snapshot`, `dag_manager_runtime_select_non_finalized_hashes`, and the
  `DagManagerRuntimeSyncSnapshot` DTO. Live C++ uses `dag_manager_runtime_non_finalized_sync_payload` for
  storage-backed DAG sync payload materialization; the lower-level selection/snapshot helpers remain Rust-private or
  test-only behavior covered by Rust bridge/domain tests.
- Standalone DAG verify/add-block helper exports are no longer CXX API:
  `dag_verify_transaction_availability`, `dag_plan_verify_transaction_query`,
  `dag_plan_non_finalized_transaction_query`, `dag_plan_expired_transaction_cleanup`, `dag_verify_vdf_prepare`,
  `dag_verify_authorization`, `dag_decide_vdf_dpos_authorization`, `dag_verify_vdf_sortition`,
  `dag_plan_add_block_effects`, and `dag_verify_gas` are deleted from the bridge surface. Live C++ routes through the
  runtime-owned `BridgeDagManagerRuntime` verify/proposer/add-block methods, while the direct
  `dag_verify_vdf_sortition_from_block` boundary remains for the current DAG manager shim VDF executor path.
  Custom agents used: `architect-reviewer` confirmed the boundary shape and retained VDF route; `rust-engineer`
  confirmed DTO/test impact and native `rustaxa-consensus` DAG coverage for the deleted wrappers.
- Additional DAG runtime bridge-test scaffolding is no longer CXX API:
  `dag_manager_runtime_rebuild`, `dag_manager_runtime_block_exists`, `dag_manager_runtime_verify_precheck`,
  `dag_manager_runtime_expired_transaction_cleanup_payload`, `dag_vrf_input`, `DagManagerSnapshot`,
  `DagVerifyPrecheckBlock`, `DagVerifyPrecheckResult`, `DagExpiredTransactionFact`, and
  `DagExpiredTransactionCleanupPayload` are deleted from the bridge surface. Live C++ uses storage restore,
  `dag_manager_runtime_is_block_known`, verify sessions, finalized-order application, and the retained
  `dag_vdf_message` public helper; native `rustaxa-consensus` DAG tests cover the deleted precheck, VRF-input, and
  expired-transaction cleanup behavior.
  Custom agents used: `architect-reviewer` confirmed the no-caller status and live-route replacements.
- `BridgeProposedBlocks::proposed_blocks_snapshot` is no longer a CXX export. Production C++ uses
  `proposed_blocks_snapshot_entries`, which preserves validation flags and payloads needed by the shim facade; grouped
  hash snapshots remain Rust test-only coverage.
- `BridgePbftChain::pbft_chain_project_update` is no longer a CXX export. The non-mutating append projection is covered
  by native `rustaxa-consensus` PBFT-chain tests, while live C++ bridge callers use `pbft_chain_update`,
  `pbft_chain_update_for_finalization`, or the retained legacy JSON projection facade.
- The direct `create_pbft_chain(PbftChainHeadPayload)` CXX constructor is no longer exported. C++ PBFT-chain bridge
  tests now seed legacy `pbft_head` JSON through the storage shim and construct via `create_pbft_chain_from_storage`,
  matching the production shim path; the direct structured-head constructor remains Rust test-only for in-memory bridge
  unit coverage.
- The direct `create_sortition_params_manager(SortitionRuntimeConfig, Vec<SortitionParamsChangePayload>)` CXX
  constructor is no longer exported. C++ sortition bridge tests now construct through
  `create_sortition_params_manager_from_storage`, matching the production shim path; the direct in-memory bridge
  constructor wrapper has been deleted entirely.
- The default-rewards `create_final_chain(...)` CXX constructor is no longer exported. C++ FinalChain bridge tests now
  call `create_final_chain_with_rewards_config`, matching the production `final_chain_shim` constructor shape; the
  default wrapper remains Rust test-only for bridge unit fixtures.
- `BridgeTransactionManagerSidecar` is deleted as a CXX handle. Its constructor, standalone sidecar methods, DAG-save
  route, finalized-status route, and bridge-only test are gone; live sidecar state is private to
  `BridgeTransactionManagerRuntime`.
- `BridgeTransactionManagerAdmissionExecution` is deleted as a CXX handle. The unused explicit execute/commit DAG-save
  script and bridge-only test are gone; `save_transactions_from_dag_block_command_report_with_runtime` preserves
  storage-first ordering at the CXX boundary while the lower `save_transactions_from_dag_block_with_runtime` helper is
  Rust-private.
- Transaction-manager lower-level DAG-save/finalized-status result APIs are no longer CXX exports:
  `save_transactions_from_dag_block_with_runtime`, `update_finalized_transactions_status_with_runtime`,
  `DagTransactionSaveAccepted`, `DagTransactionSaveOutcome`, `FinalizedTransactionStatusAction`, and
  `FinalizedTransactionStatusPlan` are deleted from the bridge surface. Live C++ uses the DAG-save command-report API
  and `update_finalized_transactions_status_command_report_with_runtime_and_final_chain`; private Rust helpers still own
  the storage-first mutation and command-report conversion.
  Custom agents used: `architect-reviewer` confirmed the live C++ command-report boundary; `rust-engineer` confirmed the
  private DTO/test impact.
- Additional standalone PBFT runtime wrappers are no longer CXX exports:
  `plan_pbft_sync_runtime`, `abort_pbft_manager_proposal_session`, `plan_pbft_manager_block_validation`,
  `load_pbft_finalization_last_period_lambda_storage`, and the bridge-only `PbftSyncRuntimePlan` DTO are deleted from
  the bridge surface. Live C++ uses `plan_pbft_sync_process_period_data_runtime`, proposal/block-validation runtime
  sessions, and `pbft_manager_runtime_load_finalization_last_period_lambda`; native `rustaxa-consensus` tests cover the
  deleted lower-level planners and lambda lookup.
  Custom agents used: `architect-reviewer` identified the next FinalChain execution-session cleanup and confirmed the
  PBFT manager standalone planner lane as a secondary cleanup candidate.
- Direct FinalChain execution-session step/report/publication helpers are no longer CXX exports. The live
  `final_chain_shim` path uses `BridgeConsensusExecutionApi` for external-EVM/`StateAPI` interaction, while the CXX
  bridge keeps only session creation/commit, the dedicated execution API, and retained pending-publication
  recovery/publication compatibility calls. The Rust-internal wrapper methods and their bridge-only DTOs remain as a
  follow-up Slice 8/9 cleanup because bridge tests and the native-only compatibility finalizer still call them directly.
  Custom agents used: `rust-engineer` confirmed the live C++ route and identified the remaining Rust-internal wrapper
  callsites that must be migrated before deleting the implementation helpers.
- Additional no-caller CXX exports are deleted after callsite audit showed they were bridge-test scaffolding only:
  `create_pbft_chain_with_storage`, `slashing_mark_double_voting_proof_submission`,
  `pillar_votes_get_verified_votes`, and `pillar_votes_snapshot_refs`. Live C++ paths use
  `create_pbft_chain_from_storage`, `slashing_report_double_voting_proof_submission`, and
  `pillar_votes_get_verified_vote_payloads`.
- Additional no-caller verified-vote and sortition CXX exports are deleted:
  `verified_votes_check_unique_voter`, `verified_votes_vote_in_verified_map`,
  `verified_votes_get_network_t_plus_one_step`, `verified_votes_get_two_t_plus_one_voted_block_votes`,
  `verified_votes_snapshot_weighted_payloads`, and `sortition_restore_finalized_period`. Live C++ verified-vote paths
  use admission, payload lookup, retained-payload 2t+1 lookup, round-marker snapshots, and explicit sortition
  record/persist APIs instead.
- The standalone broad `apply_rewards_stats_storage_writes` CXX export is deleted. Rewards-stat storage writes now enter
  through either `BridgeRewardsStatsRuntime::rewards_stats_runtime_apply_storage_writes` with runtime-owned storage or
  the dedicated storage-shim batch appender.
- Transaction-manager Rust-mode expired non-finalized cleanup now deletes pending transaction storage rows through a
  native `rustaxa-consensus` batch helper before mutating the live sidecar. This closes the Rust shim gap where
  `removeNonFinalizedTransactions` previously cleared only sidecar state while the legacy C++ implementation also
  removed matching DB rows. The remaining public `DbStorage` batch blocks in
  `libraries/core_libs/consensus/src/transaction/transaction_manager.cpp` are legacy-only under the current overlay.
- Proposed-block Rust-mode persistence and cleanup are also closed under the current overlay. The remaining public
  `DbStorage` batch block in `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` is legacy-only when
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` is enabled; Rust-mode cleanup enters
  `BridgeProposedBlocks::proposed_blocks_cleanup_with_storage`, which commits the delete batch in native Rust storage
  before removing stale periods from the Rust index.
- Sortition Rust-mode startup and finalized-period persistence are closed under the current overlay. The remaining public
  `DbStorage` batch block in `libraries/core_libs/consensus/src/dag/sortition_params_manager.cpp` is legacy-only when
  `RUSTAXA_ENABLE_SORTITION_PARAMS` is enabled; Rust-mode construction and updates enter
  `BridgeSortitionParamsManager` with an attached native Rust storage handle.
- FinalChain Rust-mode startup, native finalization, external-EVM publication, crash recovery, and storage audit are
  closed under the current overlay. The remaining public `DbStorage` batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` are legacy-only when `RUSTAXA_ENABLE_FINAL_CHAIN` is
  enabled; Rust-mode publication enters `BridgeFinalChain`/`BridgeConsensusExecutionApi` and commits FinalChain storage
  rows through native Rust storage. `StateAPI` remains the external EVM/state database boundary.
- The broader Slice 8 API shrink remains open; this guard is the closeout mechanism for future bridge-handle deletions
  and additions.

## Slice 9: Delete Compatibility Tests That Only Protect Retired Scaffolding

Purpose: keep test coverage aligned with target Rust-mode architecture.

Work:

- Replace shim tests that assert bridge/shim mechanics with Rust module tests or facade-level C++ tests that assert
  product behavior.
- Delete tests for removed queue helpers, bridge batches, `*Old` forwarding, or compatibility readers that no longer
  exist.
- Keep C++ tests when they cover public API behavior still served through a C++ facade.

Acceptance:

- No test depends on a retired bridge/shim helper.
- Behavior coverage moves closer to Rust domain modules and the three external facades.
- Narrow targeted test commands are documented in each implementation commit.

## Suggested Execution Order

1. Slice 0: audit table and closeout checks.
2. Slice 1: query API injection, because it is the lowest-risk repeated bridge construction cleanup.
3. Slice 2: network effect queue bridge retirement is complete; future network cleanup belongs to internal effect-queue
   narrowing and Slice 8/9 bridge/FFI cleanup.
4. Slice 3 and Slice 4 together where possible: move Rust storage access native first, then thin the C++ storage shim.
5. Slice 5: remove small completed shims in independent commits.
6. Slice 7: finish execution adapter contraction while preserving the external EVM boundary.
7. Slice 6: consolidate runtime-heavy shims after storage and query dependencies are no longer bridge-shaped.
8. Slice 8 and Slice 9 continuously after each slice, with a final bridge/FFI cleanup pass at the end.

## Per-Slice Closeout Checklist

- Use `$implement-rustaxa-consensus-slice` and record which custom agents were used or why a direct-only doc/process
  change did not need delegation.
- Search for and delete code made unused by the slice.
- Update this plan or the audit table with removed bridge/shim items.
- Run `git diff --check`.
- Run targeted Rust tests for touched crates.
- Run targeted C++ build/tests for touched shims, RPC, GraphQL, network, or storage paths.
- Run `rust_storage_tests` for storage-family changes.
- Run `.githooks/pre-commit` before commit when code changed.
- Commit in focused chunks: docs/audit, implementation, test cleanup, and follow-up deletion when those are separable.
