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

- `rust/crates/rustaxa-bridge/src/lib.rs` exports are mostly boundary-facing Rust modules plus narrow internal runtime helpers.
  Remaining cleanup pressure is deleting temporary shims and session handles after native Rust owners are complete.
- `rust/crates/rustaxa-bridge/src/ffi.rs` exports a reduced `Bridge*` surface; remaining pressure is deleting internal
  compatibility helpers once their upstream C++ callers are fully folded into Rust runtimes.
- `libraries/core_libs/consensus/shims/*` contains many overlay classes that should become thin C++ public facades or
  disappear once their public API is no longer needed in Rust mode.
- `rust/crates/rustaxa-consensus/src/network_api.rs` no longer exposes the temporary CXX
  `consensus_network_queue_*` bridge helpers. Remaining cleanup pressure is internal effect-drain plumbing, especially
  PBFT vote gossip through `drain_work` / `report_effect_results` while tarcap still owns transport execution.
- RPC/GraphQL now use the shared injected `ConsensusQueryApi` construction path at app/plugin boundaries.
- The primary residual work is to keep all non-boundary consensus behavior in native Rust and ensure adapters remain
  thin and explicit.

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
3. `pillar_votes_shim` (retired in this slice)
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
- The bridge-only `rustaxa-bridge/src/period_data_queue.rs` module is deleted. Its remaining conversion code is folded
  into `pbft_manager.rs` next to the manager-owned queue APIs, so the retired queue shim no longer leaves a separate
  bridge-shaped Rust module behind.
- `verified_votes_shim` compatibility sidecar fallback was removed from snapshot and conflict-handling paths. The shim now
  materializes vote state exclusively from Rust-retained weighted payloads for conflict resolution, snapshot rebuilds, and
  threshold-weighted vote aggregation. `live_votes_` storage was deleted from production flow, and missing payloads now
  fail fast rather than degrade into compatibility paths.
- `pillar_votes_shim` is retired. C++ now routes live pillar vote indexing and planning through
  `BridgePillarChainRuntime` inside `pillar_chain_manager_shim`. `RUSTAXA_ENABLE_PILLAR_VOTES` no longer wires
  `pillar_votes_shim`, `pillar_votes.cpp` is no longer compiled as `PillarVotesOld`, and
  `pillar_votes_shim_test.cpp` was removed.
- The standalone `BridgePeriodDataQueue` CXX handle, `create_period_data_queue` constructor, `period_data_queue_shim`
  overlay, `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE` CMake/Makefile flag, and bridge/shim tests for the retired facade were
  deleted.
- `BridgePbftSyncQueueDrainSession` is retired. PBFT sync queue-drain planner state now lives inside the long-lived
  `BridgePbftManagerRuntime`, and `pbft_manager_shim::pushSyncedPbftBlocksIntoChain()` resets and drives that
  runtime-owned planner through `pbft_manager_runtime_begin_pbft_sync_queue_drain`,
  `pbft_manager_runtime_pbft_sync_queue_drain_next`, and
  `pbft_manager_runtime_pbft_sync_queue_drain_report`.
- `pbft_manager_shim` keeps a temporary sidecar deque for live `PeriodData` and peer objects only. Queue admission/order/
  pop/cleanup metadata is owned by `pbft_manager_runtime_`; cert-vote payloads are now supplied in the pop plan and no
  longer carried in the sidecar.
- `vote_manager_shim::setNetwork` no longer forwards through `VoteManagerOld`; it writes the inherited protected network
  pointer directly. This removes one completed setter forwarding without changing the public C++ API.
- `transaction_manager_shim::getTransactionsMutex` no longer forwards through `TransactionManagerOld`; it is now a
  shim-owned method that routes to a lock stored on `TransactionManager` itself through the existing friend access helper.
- `dag_manager_shim::setNetwork` no longer forwards to `DagManagerOld`; the shim now only stores the local shim-owned
  network pointer at this seam.
- `slashing_manager_shim` now exposes one Rust-admission `SlashingDoubleVoteEvidence` API containing the two canonical
  vote payloads and a single shared PBFT slot. The live `PbftVote` overload is kept as a compatibility adapter that
  validates same-slot evidence before constructing the payload. This removes the loose two-record-plus-slot-scalar
  slashing route from `vote_manager_shim` without moving the external FinalChain account read, gas-price lookup,
  signing, or transaction insertion boundary yet.
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
- `cmake --build /build --target verified_votes_shim_test --parallel 12`
- `cmake --build /build --target pbft_manager_test --parallel 12`
- `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.PeriodAdmissionPlan*:RustPbftSyncTest.ProcessPeriodRuntime*' --gtest_print_time=1`
- `/build/bin/verified_votes_shim_test`
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
- Completed in this pass: `FinalChain::rustFinalChainForRust()` accessor was removed from the
  `final_chain_shim`, eliminating the direct internal bridge-handle sharing point in FinalChain call-paths.

- Current status: the transaction manager packing path is now thin (`prepare` + single `finalize`) and does not keep shim-side
  estimate-session loops. PBFT block validation no longer stores a bridge runtime cursor. PBFT finalization and duplicate
  resume no longer allocate a standalone CXX `BridgePbftFinalizationRuntimeSession`; their cursors now live on
  `BridgePbftManagerRuntime`. Dynamic-lambda planning and the previous persisted period-lambda lookup are also
  manager-runtime-owned through one finalization-specific API. Manager-owned finalization actions are now drained by
  `BridgePbftManagerRuntime` in normal finalization and duplicate-resume paths. Slice 6 remains incomplete because
  `pbft_manager_shim` still coordinates external finalization effects and lifecycle paths. `pillar_chain_manager_shim`
  now routes live vote state and PBFT-facing pillar finalization through `BridgePillarChainRuntime`, but still owns
  external FinalChain DPoS reads, temporary `PillarBlock` materialization, PBFT `PeriodData` vote materialization,
  current-block sidecar mirrors, network vote-bundle requests, and event emission.

Implementation notes:

- `final_chain_shim` now no longer exposes `rustFinalChainForRust()`; callers must route through explicit
  consensus/runtime APIs, which keeps FinalChain session ownership constrained to the shim constructor and execution
  boundary.
- `dag_manager_shim` now moved `getShared()` and `getDagMutex()` off inherited `DagManagerOld` access and onto shim-owned
  state. `setDagBlockOrder()` no longer acquires an extra outer order lock before Rust-runtime lock flow, since runtime
  callers now perform the lock sequencing directly.
- `pbft_manager_shim` still routes through shim-owned lifecycle/finalization orchestration in multiple places.
  The `transaction_manager_shim` packing path now uses `pack_prepare_sharded` + `pack_finalize_with_estimates` and is already
  reduced to thin conversion plus one Rust service round-trip plus deterministic materialization.
- `pillar_chain_manager_shim` now constructs `BridgePillarChainRuntime`, which owns live pillar-vote aggregation state
  and the native pillar storage handle used by PBFT-facing finalization. The previous live manager field
  `BridgePillarVotes` is gone; the standalone `BridgePillarVotes` CXX handle is also retired after the remaining C++
  bridge test moved to the runtime. The Rust helper remains only as a bridge-module unit-test fixture.
- `pillar_chain_manager_shim::validateSyncPillarVotesBundleDeterministically()` now routes synced bundle RLPs through
  Rust-owned batch inspection and `BridgePillarChainRuntime` weighted apply APIs. C++ only performs the external
  FinalChain DPoS weight lookup in one batched read, then passes canonical RLP bytes and weights back to Rust for
  signature validation, duplicate/conflict checks, threshold selection, and selected-vote insertion. The previous
  shim-local per-vote inspection/weight loop and `getPillarVoteWeight()` helper are gone.
- `pillar_chain_manager_shim::createPillarBlock()` now calls
  `plan_pillar_block_creation_with_vote_counts`, which combines pillar-block shell planning with ordered validator
  vote-count delta planning behind one Rust API. C++ still owns FinalChain DPoS vote-count reads, temporary
  `PillarBlock` materialization, current-block storage payload materialization, and live manager mirrors, but it no
  longer separately orchestrates the creation planner and vote-count planner before constructing a candidate block. The
  creation-only `plan_pillar_block_creation` CXX export and shell-only DTO are deleted; native Rust still owns the
  lower-level domain planner internally.
- The no-caller plain-fact pillar-vote bundle CXX planner is deleted:
  `PillarVoteBundleFact`, `PillarVoteBundleAcceptedVote`, `PillarVoteBundlePlan`, and
  `plan_pillar_vote_bundle` are no longer bridge exports. Live pillar-chain sync keeps the canonical RLP boundary:
  `inspect_pillar_vote_bundle_rlps` returns recovered voters for the one external FinalChain DPoS weight read, then
  `BridgePillarChainRuntime::pillar_chain_runtime_apply_weighted_rlp_bundle` owns weighted validation, threshold initialization,
  selected-vote insertion, and duplicate/idempotent apply classification. Native `rustaxa-consensus` pillar-vote tests
  keep coverage for the plain domain planner.
- The old weighted synced-pillar-vote planner bridge is deleted:
  `plan_pillar_vote_bundle_from_weighted_rlps`, `PillarVoteBundleWeightedPlan`, and
  `PillarVoteBundleAcceptedVoter` are no longer CXX exports. `pillar_chain_manager_shim` no longer maps accepted hashes
  back to live `PillarVote` sidecars and no longer calls `addPlannedVerifiedPillarVoteForRust`; the sync validation path
  passes canonical weighted RLPs into the `BridgePillarChainRuntime` apply API and receives only aggregate status,
  weights, and insertion failure facts. C++ still owns the one external FinalChain DPoS weight read until a broader
  pillar-chain runtime owns that port.
- Single pillar-vote admission now uses the same minimal prepare/apply shape. `pillar_chain_manager_shim` calls
  `BridgePillarChainRuntime::pillar_chain_runtime_prepare_single_vote_admission` to decode canonical RLP, recover the voter, perform
  duplicate/relevance/identity checks, and report whether a period threshold is needed. C++ performs only the external
  FinalChain DPoS eligibility or vote-count lookup and, when needed, threshold lookup, then calls
  `BridgePillarChainRuntime::pillar_chain_runtime_apply_prepared_single_vote_admission` with only canonical RLP and external DPoS
  facts; Rust re-derives signature identity, initializes period state, and inserts into Rust-owned aggregation. The
  piecemeal single-vote CXX exports
  `pillar_votes_period_data_initialized`, `pillar_votes_init_period_data`, `pillar_votes_vote_exists`,
  `pillar_votes_is_unique_identity`, `pillar_votes_is_unique_vote`, and `pillar_votes_insert_vote` are deleted along
  with `PillarVotePayload`, `PillarVoteIdentityPayload`, `PillarVoteUniqueOutcome`, and `PillarVoteInsertOutcome`.
- Follow-up relevance cleanup routes `PillarChainManager::isRelevantPillarVote` through
  `BridgePillarChainRuntime::pillar_chain_runtime_plan_vote_relevance`. Rust now decodes the vote RLP and derives
  duplicate membership from runtime-owned vote state, so the C++ shim no longer materializes Rust-retained payloads or
  scans them only to supply `vote_already_known`.
- PBFT-facing pillar-block finalization now calls
  `BridgePillarChainRuntime::pillar_chain_runtime_finalize_block_for_pbft`. Rust owns selected-vote lookup,
  deterministic pillar finalization planning, finalized-block storage persistence, and vote cleanup ordering. The
  bridge-only CXX exports `plan_pbft_finalization_pillar_preflight`,
  `report_pbft_finalization_pillar_preflight`, and `plan_pillar_block_finalization` plus their DTOs are deleted. C++
  still owns the missing-vote network request, legacy vote materialization for PBFT `PeriodData`, live
  `last_finalized_pillar_block_` mirror assignment, and pillar-finalized event emission.
- `GetPillarVotesBundlePacketHandler` no longer calls `PillarChainManager::getVerifiedPillarVotes()` or reconstructs
  C++ `PillarVote` objects for network serving. It now asks `pillar_chain_manager_shim` for packet-ready optimized
  bundle chunks from `BridgePillarChainRuntime::pillar_chain_runtime_build_verified_vote_network_bundles`, wraps each
  inner bundle RLP as the tarcap packet payload, sends it, and marks the returned vote hashes known. Rust serves
  runtime-retained votes first and falls back to stored `PeriodData` only when the embedded optimized bundle matches the
  requested period/hash. `getVerifiedPillarVotes()` remains only for public compatibility/tests and later PBFT
  `PeriodData` cleanup.
- `pbft_manager_shim` proposal and sync PBFT block validation now call the stateless
  `plan_pbft_manager_block_validation` API with a local fact bundle. The bridge-owned
  `block_validation_session` field and begin/next/report CXX exports are gone, so validation no longer stores a cursor in
  `BridgePbftManagerRuntime` while C++ performs external PBFT-chain, FinalChain, vote, pillar, and DAG checks.
- `pbft_manager_shim` finalization and duplicate-resume paths now start, step, report, and abort finalization runtime
  cursors through `BridgePbftManagerRuntime`. The standalone `BridgePbftFinalizationRuntimeSession` handle and CXX
  create/next/report/abort exports are deleted. C++ still executes the external FinalChain/EVM, DAG, transaction,
  PBFT-chain, sortition, vote-manager, and pillar side effects for this cut; the next PBFT finalization slice should
  replace the remaining shim-side finalization coordinator with a one-shot manager-owned operation.
- `pbft_manager_shim` dynamic-lambda planning now calls
  `pbft_manager_runtime_plan_finalization_dynamic_lambda`, which combines the deterministic Cacti lambda calculation
  with the prior persisted period-lambda lookup through the runtime-owned Rust storage handle. The standalone
  `plan_pbft_dynamic_lambda` CXX export and `pbft_manager_runtime_load_finalization_last_period_lambda` CXX export are
  deleted; native Rust tests keep lower-level planner and storage lookup coverage.
- `pbft_manager_shim` normal finalization and duplicate-resume paths now call
  `pbft_manager_runtime_drain_owned_finalization_actions` before and after the external FinalChain dispatch/replay
  boundary. The drain consumes only manager-internal actions (`ApplyDynamicLambda`, `PersistExecutedStatus`, and
  `SetExecutedFlag`), applies the Rust-owned storage writes through the runtime storage handle, mutates
  `BridgePbftManagerRuntime` state, validates the live mutation transcript, and returns a snapshot for C++ mirror
  fields. C++ still owns the external FinalChain/EVM, DAG, transaction-manager, PBFT-chain, sortition, vote-manager,
  advance-period, and pillar side effects until a later one-shot finalization operation absorbs those boundaries.
- `pbft_manager_shim` normal finalization and duplicate-resume paths first reported external live mutations through an
  interim `pbft_manager_runtime_report_finalization_live_mutation` API. That manager-runtime API moved validation of C++
  executor facts into Rust and replaced the standalone `validate_pbft_finalization_live_mutation_report` CXX export and
  bridge-only `PbftFinalizationLiveMutationValidation` DTO. The later boundary consolidation below deletes the interim
  CXX export and folds reporting into manager-owned finalization boundary APIs.
- `pbft_manager_shim` normal finalization and duplicate-resume paths now use the manager-owned two-call finalization
  executor APIs: `pbft_manager_runtime_start_finalization_executor` and
  typed success/failure advancement APIs. Rust owns cursor setup, primary storage apply through the runtime storage
  handle, live mutation validation, manager-owned action drains, final completion classification, and the retained
  finalization plan used by later typed reports. C++ echoes the executor cursor as a scalar argument instead of passing
  an action back into the manager; Rust derives the current action from the cursor. C++ still prepares primary storage
  stages under the existing DAG/transaction locks and still executes external sortition, reward-vote, DAG,
  transaction-manager, PBFT-chain, anchor-cache, FinalChain/EVM, advance-period, and pillar side effects before reporting
  typed facts or explicit failure back to Rust. The CXX exports for `plan_pbft_finalization_runtime`,
  `pbft_manager_runtime_finalization_session_next`, `pbft_manager_runtime_finalization_session_report`,
  `pbft_manager_runtime_finalization_session_report_action`,
  `pbft_manager_runtime_report_finalization_live_mutation`,
  `pbft_manager_runtime_report_finalization_live_mutation_boundary`,
  `pbft_manager_runtime_report_finalization_failure_boundary`,
  `pbft_manager_runtime_drain_owned_finalization_actions`, and
  `pbft_manager_runtime_apply_finalization_storage_writes` plus the older piecemeal finalization boundary APIs are
  deleted.
- The finalization advancement boundary now exposes typed success APIs plus one failure-only API,
  `pbft_manager_runtime_fail_finalization_external_effect(runtime, cursor, status, error_code)`. The duplicate
  `PbftFinalizationExecutorAdvanceReport` DTO, the generic `PbftFinalizationExternalEffectReport` CXX DTO, and the C++
  field-copy helper are deleted. Subsystem executors return only their own narrow reports, and the PBFT manager shim
  passes the current executor cursor separately. Rust derives the current action plus base finalization identity
  (`block_period`, PBFT block hash, and anchor hash) from the cursor and plan retained inside
  `BridgePbftManagerRuntime`, then maps typed subsystem facts directly into the native live-mutation report.
  `PbftFinalizationRuntimeActionReport` is now a private Rust helper, not a CXX DTO.
- Follow-up report-surface cleanup removes both `PbftFinalizationLiveMutationReport` and
  `PbftFinalizationExternalEffectReport` from the CXX bridge. Sortition, reward-vote, DAG, transaction-manager,
  PBFT-chain, anchor-cache, FinalChain replay/dispatch, advance-period, and pillar post-processing executors now return
  or construct only subsystem-specific reports. Rust derives finalization identity from the manager-runtime retained plan
  and maps those typed reports into the native live-mutation report internally, so C++ no longer owns the duplicate
  live-report DTO, the generic external-effect DTO, or the `makeFinalizationExternalEffectReport` mapping helper.
- The manager executor still checks the expected action before each C++ side effect runs, but subsystem reports carry
  only subsystem facts. The executor cursor is the only accepted action identity source for typed success APIs and the
  failure-only `pbft_manager_runtime_fail_finalization_external_effect` API, which removes the last duplicated action
  echo and generic success/failure envelope from sortition, reward-vote, DAG, transaction-manager, PBFT-chain,
  anchor-cache, FinalChain, advance-period, and pillar reports.
- The legacy Rust bridge-crate finalization cursor primitives
  `pbft_manager_runtime_begin_finalization_session`,
  `pbft_manager_runtime_begin_finalization_resume_session`,
  `pbft_manager_runtime_finalization_session_next`,
  `pbft_manager_runtime_finalization_session_report_action`,
  `pbft_manager_runtime_report_finalization_live_mutation`, and
  `pbft_manager_runtime_drain_owned_finalization_actions` are now private implementation helpers. C++ and external bridge
  consumers can only drive the manager-owned finalization path through the executor APIs listed above plus the explicit
  abort call.
- The bridge-only finalization cursor/drain DTOs `PbftFinalizationRuntimeSessionStep` and
  `PbftManagerFinalizationOwnedActionDrainResult` are no longer CXX exports. The same internal facts now live in
  Rust-private `pbft_manager.rs` helper structs, and C++ receives only the stable
  `PbftManagerFinalizationExecutorState` executor boundary.
- The transaction-manager finalization client no longer returns the generic
  `PbftFinalizationExternalEffectReport` from `transaction_manager_shim`. `pbft_manager_shim` now calls the existing
  typed `TransactionManagerFinalizedStatusCommandReport` path and advances the manager-owned finalization cursor through
  `pbft_manager_runtime_advance_finalization_transaction_status`, which maps the finalized transaction count inside the
  Rust bridge.
- The PBFT-chain finalization client no longer returns the generic `PbftFinalizationExternalEffectReport` from
  `pbft_chain_update_for_finalization`. The bridge method now returns `PbftChainFinalizationUpdateReport`, containing
  only PBFT-chain head facts, and `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_pbft_chain` so Rust builds the native live-mutation report internally.
- The sortition finalization client no longer returns the generic `PbftFinalizationExternalEffectReport` from
  `commitPreparedBlockForSortitionFinalization`. The shim now returns `SortitionFinalizationCommitReport`, containing
  only live threshold/change/cache-count facts, and `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_sortition_commit` so Rust builds the native live-mutation report internally.
- The reward-vote reset finalization client no longer returns the generic `PbftFinalizationExternalEffectReport` from
  `commitRewardVotesResetForFinalization`. The shim now returns `RewardVotesFinalizationResetReport`, containing only
  live reward-vote period/round/block-hash/remaining-extra-count facts, and `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_reward_votes_reset` so Rust builds the native live-mutation report
  internally.
- The DAG-order finalization client no longer returns the generic `PbftFinalizationExternalEffectReport` from
  `setDagBlockOrderForPbftFinalization`. The shim now returns `DagFinalizationOrderReport`, containing only the finalized
  DAG-block count, and `pbft_manager_shim` advances through `pbft_manager_runtime_advance_finalization_dag_order` so Rust
  builds the native live-mutation report internally.
- The manager-local anchor DAG cache clear path no longer constructs the generic finalization report directly from the
  cache mutation. It now uses `AnchorDagCacheFinalizationClearReport`, containing only the remaining cached-anchor count,
  and `pbft_manager_shim` advances through the typed Rust bridge API.
- FinalChain PBFT finalization dispatch and resume replay no longer build the generic finalization report from direct
  FinalChain reads at each callsite. The shim-owned `finalize_` wrapper now returns
  `FinalChainPbftFinalizationDispatchReport`, containing only `blocks_per_year` and the observed FinalChain
  `last_block`, and `pbft_manager_shim` advances through
  `pbft_manager_runtime_advance_finalization_final_chain_dispatch` so Rust builds the native live-mutation report
  internally.
- PBFT manager advance-period finalization now uses `PbftManagerFinalizationAdvancePeriodReport`, containing only the
  post-advance manager period, before `pbft_manager_shim` advances through the typed Rust bridge API.
- PBFT manager pillar post-processing now uses `PbftManagerFinalizationPillarPostProcessingReport`, containing only the
  pillar processed/request periods, before the Rust bridge builds the native live-mutation report internally. The shim
  derives the request period once with checked delegation-delay arithmetic before executing the pillar side effect. All
  current PBFT finalization subsystem/local facts now have typed reports and the only generic external-effect reporting
  that remains is the failure-only scalar API.
- Follow-up API narrowing moved pillar post-processing onto
  `pbft_manager_runtime_advance_finalization_pillar_post_processing`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that client. Rust injects the live manager period and builds the native
  live-mutation report before using the existing finalization executor validation/drain path internally.
- Follow-up API narrowing moved anchor-cache clear reporting onto
  `pbft_manager_runtime_advance_finalization_anchor_cache_clear`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that manager-local client. Rust maps the single
  `remaining_anchor_count` fact into the native live-mutation report internally before running the existing cursor
  validation and drain path.
- Follow-up API narrowing moved advance-period reporting onto
  `pbft_manager_runtime_advance_finalization_advance_period`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that manager-local client. Rust maps the single post-advance
  `manager_period` fact into the native live-mutation report internally before running the existing cursor validation
  and drain path.
- Follow-up API narrowing moved PBFT-chain update reporting onto
  `pbft_manager_runtime_advance_finalization_pbft_chain`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that external PBFT-chain client. Rust maps the existing
  `PbftChainFinalizationUpdateReport` head facts into the native live-mutation report internally before running the
  existing cursor validation and drain path.
- Follow-up API narrowing moved DAG-order reporting onto
  `pbft_manager_runtime_advance_finalization_dag_order`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that external DAG client. Rust maps the single finalized DAG-block count
  into the native live-mutation report internally before running the existing cursor validation and drain path.
- Follow-up API narrowing moved sortition commit reporting onto
  `pbft_manager_runtime_advance_finalization_sortition_commit`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that external sortition client. C++ maps the sortition-owned report into a
  manager-scoped CXX DTO, and Rust maps only threshold/change/cache-count facts into the native live-mutation report
  internally before running the existing cursor validation and drain path.
- Follow-up API narrowing moved reward-vote reset reporting onto
  `pbft_manager_runtime_advance_finalization_reward_votes_reset`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that vote-manager client. C++ maps the vote-manager-owned report into a
  manager-scoped CXX DTO, and Rust maps only period/round/block-hash/remaining-extra-count facts into the native
  live-mutation report internally before running the existing cursor validation and drain path.
- Follow-up API narrowing moved FinalChain dispatch/replay reporting onto
  `pbft_manager_runtime_advance_finalization_final_chain_dispatch`, so C++ no longer constructs
  `PbftFinalizationExternalEffectReport` for that FinalChain/EVM client. C++ maps the shim-owned
  `FinalChainPbftFinalizationDispatchReport` into a manager-scoped CXX DTO, and Rust maps only blocks-per-year plus
  observed last-block facts into the native live-mutation report internally before running the existing cursor validation
  and drain path.
- Duplicate-finalization resume plans now replay `SetExecutedFlag` after executed-status persistence in executed-only
  tails as well as dynamic-lambda-already-finalized tails, so the owned-action drain cannot complete with durable
  executed status persisted but the live PBFT manager snapshot left stale.
- The direct CXX exports `pbft_manager_runtime_apply_dynamic_lambda` and
  `pbft_manager_runtime_apply_finalization_executed_status` are deleted; their behavior now exists only inside the
  manager-owned finalization drain API.
- PBFT manager period-data queue metadata reads now use one runtime-owned snapshot API:
  `pbft_manager_runtime_period_data_queue_snapshot`. The individual CXX getters
  `pbft_manager_runtime_period_data_queue_period`,
  `pbft_manager_runtime_period_data_queue_syncing_period`,
  `pbft_manager_runtime_period_data_queue_last_block_hash_or_chain`,
  `pbft_manager_runtime_period_data_queue_size`, and `pbft_manager_runtime_period_data_queue_empty` are deleted. C++
  still supplies PBFT-chain size/current-period/last-hash facts, while Rust owns the queue-derived period, sync-period,
  link-hash, size, and empty view.
- Custom-agent delegation was attempted for this Slice 6 increment (`cpp-pro` and `rust-engineer`), but the agent
  backend rejected both starts due to a GPT-5.3-Codex-Spark usage limit. Local implementation and validation proceeded
  using the `$implement-rustaxa-consensus-slice` workflow.
- Custom-agent delegation was attempted again for the PBFT block-validation cursor removal (`cpp-pro` and
  `rust-engineer`); both attempts hit the same GPT-5.3-Codex-Spark usage limit, so the local implementation path
  continued.
- Custom agents used for the PBFT finalization session-handle consolidation:
  - `architect-reviewer`: recommended folding the standalone finalization runtime session into
    `BridgePbftManagerRuntime` before broader pillar-chain ownership work.
  - `api-designer`: confirmed the larger follow-up API should be a manager-owned finalization operation while keeping
    FinalChain/EVM, pillar, DAG, transaction, PBFT-chain, and network side effects external.
- Custom agents used for the dynamic-lambda manager-runtime consolidation:
  - `architect-reviewer`: recommended the next larger PBFT finalization cut as a manager-runtime owned-action drain that
    consumes internal actions while stopping before external FinalChain/EVM, DAG, transaction, PBFT-chain, sortition,
    vote-manager, pillar, and network boundaries.
  - `api-designer`: recommended evolving this into a manager-owned finalization executor API, with intent planning,
    begin/resume, step reporting, and external-effect reports grouped behind `BridgePbftManagerRuntime`.
- Custom agent used for the PBFT finalization live-report consolidation:
  - `api-designer`: recommended a broader manager-owned finalization boundary API that groups begin/resume, external
    effect reports, cursor advancement, and completion reporting while keeping FinalChain/EVM, DAG, transaction-manager,
    PBFT-chain, sortition, vote-manager, advance-period, and pillar execution external for now. This increment implements
    the prerequisite manager-runtime live-report API and leaves the broader boundary API as the next PBFT finalization
    cut.
- Custom agents used for the PBFT finalization boundary consolidation:
  - `architect-reviewer`: confirmed the bounded operation/boundary API shape and highlighted primary-storage ordering,
    duplicate-resume replay, and explicit external action reporting as acceptance risks.
  - `cpp-pro`: attempted for C++ shim review, but the agent backend rejected the start due to a GPT-5.3-Codex-Spark usage
    limit. Local implementation and validation proceeded with the `$implement-rustaxa-consensus-slice` workflow.
- Custom agents used for the pillar-chain creation consolidation:
  - `api-designer`: recommended a broader follow-up `BridgePillarChainRuntime` that would own vote admission and sync
    bundle planning while keeping FinalChain DPoS reads, network requests, event emission, and temporary
    `PillarBlock`/`PillarVote` materialization external.
  - `architect-reviewer`: recommended the no-caller plain-fact pillar-vote bundle planner as the next bridge-surface
    cleanup candidate. Follow-up cleanup retired the last `BridgePillarVotes` CXX handle after moving C++ coverage to
    `BridgePillarChainRuntime`; pillar storage compatibility remains live for now.
- Custom agent attempted for the plain-fact pillar-vote bundle cleanup:
  - `rust-engineer`: requested to review hidden callsites and retained coverage, but the agent backend rejected the
    start due to a GPT-5.3-Codex-Spark usage limit. Local implementation proceeded with call-site search evidence and
    targeted Rust/C++ validation.
- Custom agents used for the PBFT finalization external-effect boundary consolidation:
  - `api-designer`: recommended collapsing success and failure reports into one
    `BridgePbftManagerRuntime` external-effect boundary API while keeping FinalChain/EVM, DAG, transaction-manager,
    PBFT-chain, sortition, vote-manager, advance-period, pillar, and network execution external.
  - `architect-reviewer`: recommended single pillar-vote admission as the next pillar-chain candidate after this PBFT
    slice; this remains the preferred follow-up before introducing a broad `BridgePillarChainRuntime`.
- Custom agents used for the single pillar-vote admission API consolidation:
  - `api-designer`: requested to review whether the new prepare/apply CXX surface is minimal for the external C++ client
    and whether obsolete piecemeal bridge APIs remain exposed.
  - `architect-reviewer`: requested to review whether the C++ shim now retains only the external FinalChain
    DPoS/threshold/logging boundary and whether any legacy fallback remains in production routing.
- Custom agents used for the PBFT finalization report-surface cleanup:
  - `api-designer`: recommended the next larger two-call finalization executor API:
    `pbft_manager_runtime_start_finalization_executor` plus
    the now-retired generic `pbft_manager_runtime_advance_finalization_external_effect`, with C++ reporting outcomes by
    cursor while Rust derives the requested action. Follow-up typed-success/failure-only cleanup superseded that generic
    advancement API.
  - `architect-reviewer`: confirmed the safe next large cut is a Rust-owned finalization executor operation that keeps
    FinalChain/EVM, DAG, transaction-manager, PBFT-chain, sortition, vote-manager, advance-period, pillar, network, and
    local cache effects as explicit external actions for now. The follow-up executor cut below implements that API.
- Custom agents used for the PBFT finalization executor API consolidation:
  - `rust-engineer`: recommended replacing the three piecemeal finalization boundary exports with
    `pbft_manager_runtime_start_finalization_executor` and
    the intermediate generic advancement API, adding cursor to the returned state, and deriving action identity from the
    Rust cursor. The later generic-report removal replaced this with typed success APIs plus
    `pbft_manager_runtime_fail_finalization_external_effect`.
  - `cpp-pro`: reviewed the C++ migration and called out the lock partition, move-only FinalChain payloads, duplicate
    resume replay guard, anchor-cache pairing, and failure/abort semantics that the executor cut must preserve.
- Custom agents used for the PBFT finalization external-effect advance DTO cleanup:
  - `api-designer`: recommended deleting the duplicated `PbftFinalizationExecutorAdvanceReport` copy DTO and advancing
    the manager runtime with a separate cursor plus the then-existing external-effect report; the later typed-success
    cleanup removed that generic report from the CXX surface entirely.
  - `cpp-pro`: mapped all C++ report constructors and confirmed the repeated copy wrapper was the narrow removable
    boundary for that slice. Later slices converted subsystem reports to typed manager APIs and deleted the generic
    report boundary.
- Custom agents used for the transaction finalized-status finalization client cleanup:
  - `api-designer`: recommended PBFT-chain as the smallest standalone bridge-facing generic-report cleanup after this
    transaction cut, with a PBFT-chain-owned head update report and manager-local conversion.
  - `cpp-pro`: mapped the then-remaining C++ producers of `PbftFinalizationExternalEffectReport`; subsequent slices
    narrowed those producers one by one and then removed the generic DTO from the manager executor boundary.
- Custom agents used for the sortition finalization client cleanup:
  - `api-designer`: confirmed sortition should keep the existing Rust bridge `SortitionParamsChangeResult` and use only a
    C++ shim-local `SortitionFinalizationCommitReport`, with no new CXX bridge DTO.
  - `cpp-pro`: confirmed the sortition finalization call graph and recommended removing the unused
    `PbftFinalizationStorageWritePlan` argument from the sortition shim while converting to the manager report only in
    `pbft_manager_overlay`.
- Custom agents used for the reward-vote reset finalization client cleanup:
  - `api-designer`: recommended a C++ shim-local `RewardVotesFinalizationResetReport` with no failure/status fields and
    no Rust FFI changes; PBFT manager remains responsible for canonical success/failure reporting.
  - `cpp-pro`: confirmed the reward-vote reset flow now stages storage in Rust, mutates live C++ metadata after commit,
    and, at that intermediate point, converted to `PbftFinalizationExternalEffectReport` only in
    `pbft_manager_overlay`. The later typed advancement cleanup removed that conversion.
- Custom agents used for the reward-vote reset typed advancement cleanup:
  - `api-designer`: confirmed the manager-scoped CXX DTO should contain only period, round, block hash, and remaining
    extra-reward-vote count, with success/status/cursor/action identity still owned by the PBFT manager executor.
  - `cpp-pro`: confirmed the C++ replacement should preserve the existing require-action, abort, snapshot, and
    `commitRewardVotesResetForFinalization` ordering semantics.
- Custom agents used for the FinalChain dispatch/replay typed advancement cleanup:
  - `api-designer`: recommended a manager-scoped
    `PbftManagerFinalizationFinalChainDispatchReport` with only `blocks_per_year` and `last_block`, while Rust derives
    success/action identity from the cursor and maps the temporary executor envelope internally.
  - `cpp-pro`: confirmed the duplicate-resume sequentiality guard, `finalize_` external FinalChain/EVM boundary, abort
    semantics, and typed advancement call ordering.
- Custom agents used for the generic PBFT finalization report removal:
  - `rust-engineer`: recommended deleting the public `PbftFinalizationExternalEffectReport` CXX DTO, keeping typed
    success APIs, and exposing only a failure-only cursor/status/error API for C++ executor failures.
  - `cpp-pro`: confirmed the generic report had only two production C++ failure callsites plus three focused tests, so
    replacing them with the failure-only API removes the CXX generic boundary without changing external side-effect
    ordering.
- Custom agents used for the DAG-order finalization client cleanup:
  - `api-designer`: recommended a C++ shim-local `DagFinalizationOrderReport` with only `finalized_count` and no Rust
    FFI changes; at that point the PBFT manager still owned the temporary generic executor report boundary.
  - `cpp-pro`: confirmed `setDagBlockOrderForPbftFinalization` did not use the finalization write intent and that the
    only in-repo caller is the PBFT manager overlay.
- Custom agents used for the anchor DAG cache finalization report cleanup:
  - `api-designer`: confirmed the cache clear path is manager-local and already has the right typed fact shape
    (`remaining_anchor_count`), then recommended FinalChain dispatch/replay as the next meaningful external-client cut
    with a `blocks_per_year`/`last_block` report.
  - `cpp-pro`: mapped the then-remaining C++ generic finalization report producers, confirmed anchor cache was the only
    fresh-path producer whose mutation is purely manager/runtime local, and recommended the advance-period pair as the
    lowest-risk overlay-only follow-up.
- Custom-agent guidance applied to the FinalChain PBFT finalization dispatch cleanup:
  - `api-designer`: recommended `FinalChainPbftFinalizationDispatchReport` with only `blocks_per_year` and `last_block`,
    keeping success/status/error/PBFT identity/action identity at the PBFT manager executor boundary.
  - `cpp-pro`: called out the replay non-sequential guard, move-only `PeriodData`/DAG order handling, and post-dispatch
    `last_block` validation as the main risks to preserve.
- Custom-agent guidance applied to the advance-period finalization report cleanup:
  - `cpp-pro`: recommended the advance-period pair as the lowest-risk overlay-only follow-up and identified the minimal
    typed payload as the post-advance `manager_period`.
  - `api-designer`: noted advance-period is manager-local bookkeeping and should not include success/status/error or
    PBFT/action identity in the subsystem report shape.
- Custom-agent guidance applied to the pillar post-processing finalization report cleanup:
  - `api-designer`: recommended a manager-local report containing only processed/request-period post-processing facts
    and leaving success/status, errors, cursor identity, action identity, and manager-period validation at the PBFT
    manager executor boundary.
  - `cpp-pro`: identified the duplicate-resume and fresh callsites around `processPillarBlock(block_pbft_period)` and
    called out the delegation-delay/request-period calculation, unsigned underflow risk, and `processPillarBlock` side
    effects as the invariants to preserve.
  - `rust-engineer`: implemented/reviewed the typed Rust bridge advancement, added a targeted bridge test for the pillar
    report, and confirmed the helper should inject `manager_period` through `pbft_manager_runtime_snapshot`.
- Custom agent used for the anchor-cache typed advancement cleanup:
  - `rust-engineer`: implemented/reviewed the typed Rust bridge advancement, added a targeted bridge test for the
    anchor-cache report, and confirmed the helper should map only `remaining_anchor_count` into the temporary executor
    envelope.
- Custom agents used for the advance-period typed advancement cleanup:
  - `cpp-pro`: implemented/reviewed the fresh and duplicate-resume C++ success wiring and confirmed failure, abort, and
    snapshot semantics remain unchanged.
  - `rust-engineer`: requested for the Rust bridge helper/test; local implementation proceeded while the agent was still
    running, using the established typed pillar and anchor-cache helper pattern.
- Custom agents used for the PBFT-chain typed advancement cleanup:
  - `rust-engineer`: implemented/reviewed the Rust helper/test over the existing
    `PbftChainFinalizationUpdateReport` and confirmed the report size must match the finalization period.
  - `cpp-pro`: requested for C++ shim wiring review preserving abort, snapshot, and failure semantics.
- Custom agents used for the DAG-order typed advancement cleanup:
  - `api-designer`: confirmed the scalar `finalized_count` helper is preferable to adding a one-field CXX DTO and keeps
    DAG identity/action/status at the PBFT manager boundary.
  - `cpp-pro`: confirmed DAG order is the lowest-risk remaining generic producer and that the helper preserves abort,
    snapshot, `can_continue`, and `fail_boundary` semantics without touching the resume path.
- Custom agents used for the sortition typed advancement cleanup:
  - `api-designer`: confirmed the six-field sortition report should use a manager-scoped CXX DTO instead of scalar
    arguments, with success/status/cursor/action identity still owned by the PBFT manager executor.
  - `cpp-pro`: confirmed the typed helper should preserve `should_commit_sortition_runtime` and
    `prepared_sortition_params_change` semantics, cover changed and unchanged outcomes, keep failures on the generic
    failure path, and update the sortition shim comment.

### Slice 6 validation checkpoint (2026-06-27)

- Ran `cmake --build /build --target pbft_manager_test --parallel 12` and
  `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge` plus
  `cargo check --manifest-path rust/Cargo.toml -p rustaxa-consensus --no-run`.
  All passed with no compiler or link-time failures.
- Added targeted Rust checks for the new transaction-manager packing API path:
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager_runtime_pack_prepare` and
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager::tests::bridge_transaction_manager_runtime_pack`.
  Both passed.
- Rebuilt `taraxad` via `cmake --build /build --target taraxad --parallel 12` to verify C++ shim/bridge signature integration.
- Focused integration execution (`PbftManagerTest.pbft_manager_run_multi_nodes`) passed on this branch in the
  requested build configuration.
- Additional validation for the pillar-chain sync bundle reduction:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_votes`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_votes`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarVoteBundleBridgeTest.*:PillarVoteInspectionBridgeTest.*:PillarChainPlanningBridgeTest.*:PillarVoteRelevanceBridgeTest.*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
- Additional validation for PBFT block-validation cursor removal:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_block_validation`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus block_validation`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='*PbftManager*:*PBFT*:*Pbft*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes:PbftManagerTest.*sync*:PbftManagerTest.*pbft_block*' --gtest_print_time=1`
- Additional validation for PBFT finalization session-handle consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge runtime_planner_maps_ordered_finalization_actions -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*:RustPbftSyncTest.FinalizationRuntimePlanOrdersMixedExecutorActions' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
- Additional validation for PBFT dynamic-lambda manager-runtime consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge finalization_dynamic_lambda -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.DynamicLambdaPlannerMatchesCactiAdjustmentPolicy:RustPbftSyncTest.FinalizationRuntime*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
  - `.githooks/pre-commit`
- Additional validation for PBFT finalization owned-action drain:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_drains_owned_finalization_actions -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_drain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus resume_classifier -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.DynamicLambdaPlannerMatchesCactiAdjustmentPolicy:RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
- Additional validation for PBFT finalization live-report consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_validates_and_reports_external_finalization_mutations -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.FinalizationLiveMutationReportsAdvanceManagerRuntime:RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
- Additional validation for PBFT finalization boundary consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*Boundary*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
  - `.githooks/pre-commit`
- Additional validation for PBFT finalization external-effect boundary consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus finalization_live_mutation -- --nocapture`
    returned zero matching tests; the bridge and C++ boundary tests below cover the changed CXX API.
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*Boundary*' --gtest_print_time=1`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
- Additional validation for pillar-chain creation consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_chain -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target pillar_chain_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarChainPlanningBridgeTest.*' --gtest_print_time=1`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarVoteBundleBridgeTest.*:PillarVoteInspectionBridgeTest.*:PillarChainPlanningBridgeTest.*:PillarVoteRelevanceBridgeTest.*' --gtest_print_time=1`
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.pillar_blocks_create' --gtest_print_time=1`
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.votes_count_changes' --gtest_print_time=1`
  - The combined `pillar_chain_test` filter
    `PillarChainTest.pillar_blocks_create:PillarChainTest.votes_count_changes` failed on a `/tmp/taraxa0` RocksDB lock
    when the second test started in the same process; each focused test passed when run in isolation.
- Additional validation for plain-fact pillar-vote bundle CXX API deletion:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_votes -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_votes -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarVoteBundleBridgeTest.*:PillarVoteInspectionBridgeTest.*:PillarVoteRelevanceBridgeTest.*' --gtest_print_time=1`
  - `rg -n "plan_pillar_vote_bundle\\(|PillarVoteBundleFact|PillarVoteBundlePlan\\b|PillarVoteBundleAcceptedVote\\b|bundle_fact_to_consensus_fact|FfiPillarVoteBundleFact|PillarVoteBundlePlanOutput" rust/crates/rustaxa-bridge/src libraries tests -g'*.rs' -g'*.cpp' -g'*.hpp'` returned no matches.
- Additional validation for synced pillar-vote apply API consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_votes -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_votes -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarVoteBundleBridgeTest.*:PillarVoteInspectionBridgeTest.*:PillarVoteRelevanceBridgeTest.*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `rg -n "plan_pillar_vote_bundle_from_weighted_rlps|addPlannedVerifiedPillarVoteForRust|ValidateSyncPillarVotesBundleAcceptedVote|live_pillar_votes|PillarVoteBundleWeightedPlan|PillarVoteBundleAcceptedVoter" libraries tests/rust/consensus rust/crates/rustaxa-bridge/src -g'*.rs' -g'*.cpp' -g'*.hpp'` returned no matches.
- Additional validation for single pillar-vote admission API consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_votes -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pillar_chain_test pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='PillarVote*:*PillarChainPlanningBridgeTest.*' --gtest_print_time=1`
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.pillar_blocks_create' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - The obsolete single-vote bridge API search for
    `pillar_votes_period_data_initialized`, `pillar_votes_init_period_data`, `pillar_votes_vote_exists`,
    `pillar_votes_is_unique_identity`, `pillar_votes_is_unique_vote`, `pillar_votes_insert_vote`,
    `PillarVotePayload`, `PillarVoteIdentityPayload`, `PillarVoteUniqueOutcome`, and `PillarVoteInsertOutcome` returned
    no matches in the exported bridge surface or C++ shim/test callers after the consolidation.
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.votes_count_changes' --gtest_print_time=1` still fails
    in isolation with PBFT progress timing out and no validator vote-count changes on the new pillar block. The observed
    logs did not include the new single-vote admission failure messages, so this remains a broader pillar/PBFT progress
    validation gap rather than evidence of a rejected admission path.
- Additional validation for PBFT finalization cursor helper privacy:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_advances_finalization_with_transaction_status_report -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge bridge_pbft_chain_finalization_update -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*' --gtest_print_time=1`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftChainTest.*:RustPbftSyncTest.Finalization*' --gtest_print_time=1`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.FinalizationBoundary*:RustPbftSyncTest.FinalizationExecutorRejectsStaleCursor:RustPbftSyncTest.FinalizationResumeBoundaryOwnsManagerTailDrain' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `rg -n "pub fn pbft_manager_runtime_(begin_finalization_session|begin_finalization_resume_session|finalization_session_next|finalization_session_report|finalization_session_report_action|report_finalization_live_mutation|drain_owned_finalization_actions)" rust/crates/rustaxa-bridge/src/pbft_manager.rs` returns only the retained public boundary function.
  - `rg -n "PbftFinalizationRuntimeSessionStep|PbftManagerFinalizationOwnedActionDrainResult" rust/crates/rustaxa-bridge/src libraries tests -g'*.rs' -g'*.cpp' -g'*.hpp'`
    returns no live code references.
  - `rg -n "PbftFinalizationExternalEffectReport|updateFinalizedTransactionsStatusForPbftFinalization\\([^\\n]*PbftFinalizationStorageWritePlan" libraries/core_libs/consensus/shims/transaction_manager_shim -g'*.cpp' -g'*.hpp'`
    returns no live transaction-shim references.
  - `rg -n "advance_finalization_pbft_chain|pbft_chain_update_for_finalization\\(|PbftChainFinalizationUpdateReport|pbft_chain_(size|head_hash|last_anchor_hash)" rust/crates/rustaxa-bridge/src/pbft_chain.rs rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/pbft_chain_shim libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.cpp' -g'*.hpp'`
    shows PBFT-chain finalization update returning only `PbftChainFinalizationUpdateReport`, with the manager using the
    typed Rust bridge advancement helper and no generic external-effect DTO in the PBFT-chain update path.
  - `rg -n "advance_finalization_sortition_commit|PbftManagerFinalizationSortitionCommitReport|SortitionFinalizationCommitReport|sortition_(changed|change_period|change_interval_efficiency|change_threshold_upper|current_threshold_upper|params_changes_count)" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/sortition_params_manager_shim libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.hpp' -g'*.cpp'`
    shows sortition runtime commit using the typed Rust bridge advancement helper; any remaining sortition field
    assignment is Rust-private native live-mutation construction, not a C++ generic report in the sortition path.
  - `rg -n "advance_finalization_reward_votes_reset|PbftManagerFinalizationRewardVotesResetReport|commitRewardVotesResetForFinalization|RewardVotesFinalizationResetReport|reward_votes_(period|round|block_hash|extra_count)" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/vote_manager_shim libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.hpp' -g'*.cpp'`
    shows reward-vote reset using the typed Rust bridge advancement helper; any remaining reward-vote field assignment
    is Rust-private native live-mutation construction, not a C++ generic report.
  - `rg -n "advance_finalization_dag_order|dag_finalized_count|setDagBlockOrderForPbftFinalization|DagFinalizationOrderReport" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/dag_manager_shim libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.hpp' -g'*.cpp'`
    shows DAG order using the scalar typed Rust bridge advancement helper; any remaining `dag_finalized_count`
    assignment is Rust-private native live-mutation construction, not a C++ generic report in the DAG-order path.
  - `rg -n "advance_finalization_anchor_cache_clear|PbftManagerFinalizationAnchorCacheClearReport|AnchorDagCacheFinalizationClearReport|anchor_dag_cache_count" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.cpp'`
    shows the anchor DAG cache clear path using a typed Rust bridge advancement instead of C++ constructing the generic
    external-effect report. The remaining `anchor_dag_cache_count` assignment is Rust-private native live-mutation
    construction.
  - `rg -n "advance_finalization_final_chain_dispatch|PbftManagerFinalizationFinalChainDispatchReport|FinalChainPbftFinalizationDispatchReport|final_chain_(dispatched|blocks_per_year|last_block)" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/pbft_manager_shim/include/pbft/pbft_manager_shim.hpp libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.hpp' -g'*.cpp'`
    shows `finalize_` returning a typed FinalChain report and both fresh dispatch plus duplicate-resume replay using the
    typed Rust bridge advancement helper. Any remaining `final_chain_*` assignment is Rust-private native live-mutation
    construction, not C++ generic report construction in the FinalChain dispatch path.
  - `rg -n "advance_finalization_advance_period|PbftManagerFinalizationAdvancePeriodReport|manager_period" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.cpp'`
    shows the advance-period finalization path using a typed Rust bridge advancement instead of C++ constructing the
    generic external-effect report. The remaining `manager_period` assignment for this path is Rust-private native
    live-mutation construction.
  - `rg -n "PbftManagerFinalizationPillarPostProcessingReport|pillar_processed_period|pillar_request_period" libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.cpp'`
    shows the pillar post-processing path producing a typed manager report.
  - `rg -n "advance_finalization_pillar_post_processing|PbftManagerFinalizationPillarPostProcessingReport|pillar_report" rust/crates/rustaxa-bridge/src/pbft_manager.rs rust/crates/rustaxa-bridge/src/ffi.rs libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp -g'*.rs' -g'*.cpp'`
    shows the pillar post-processing path using a typed Rust bridge advancement instead of C++ constructing the generic
    external-effect report.
  - `rg -n "PbftFinalizationExternalEffectReport|pbft_manager_runtime_advance_finalization_external_effect|makeFinalizationExternalEffectFailure" rust/crates/rustaxa-bridge/src/ffi.rs rust/crates/rustaxa-bridge/src/pbft_manager.rs libraries/core_libs/consensus/shims/pbft_manager_shim/src/pbft_manager_overlay.cpp tests/rust/consensus/test_pbft_sync.cpp`
    returns no matches; C++ success reporting uses typed APIs and failure reporting uses
    `pbft_manager_runtime_fail_finalization_external_effect`.
  - `git diff --check`
- Additional validation for PBFT finalization external-effect report-surface cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all`
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_chain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*Boundary*:RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
  - `rg -n "PbftFinalizationLiveMutationReport|FfiPbftFinalizationLiveMutationReport|makeFinalizationExternalEffectReport" rust/crates/rustaxa-bridge/src libraries/core_libs/consensus/shims tests/rust/consensus -g'*.rs' -g'*.cpp' -g'*.hpp'`
    now returns only native Rust-domain `PbftFinalizationLiveMutationReport` references in `pbft_manager.rs`, not CXX
    bridge DTOs or shim helpers.
- Additional validation for PBFT finalization external-effect action-echo cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_chain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*Boundary*:RustPbftSyncTest.Finalization*Executor*:RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
  - `.githooks/pre-commit`
  - `rg -n "external_report\\.action|\\.action = kPbftFinalizationRuntimeAction" libraries/core_libs/consensus/shims tests/rust/consensus rust/crates/rustaxa-bridge/src -g'*.cpp' -g'*.hpp' -g'*.rs'`
    returns no matches; the follow-up generic-report removal check proves the whole
    `PbftFinalizationExternalEffectReport` FFI struct is gone from live code.
- Additional validation for PBFT finalization executor API consolidation:
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target pbft_manager_test --parallel 12`
  - `rg -n "pbft_manager_runtime_begin_finalization_boundary|pbft_manager_runtime_begin_finalization_resume_boundary|pbft_manager_runtime_report_finalization_external_effect_boundary|PbftManagerFinalizationBoundary" rust/crates libraries tests doc -g'*.rs' -g'*.cpp' -g'*.hpp' -g'*.md'`
    now returns no live code references; docs retain only historical validation notes where relevant.
- Additional validation for PBFT finalization external-effect advance DTO cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*Boundary*:RustPbftSyncTest.FinalizationRuntime*:RustPbftSyncTest.FinalizationResumeRuntime*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `rg -n "PbftFinalizationExecutorAdvanceReport|pbft_manager_runtime_advance_finalization_executor|makeFinalizationExecutorAdvanceReport" rust/crates/rustaxa-bridge/src libraries tests -g'*.rs' -g'*.cpp' -g'*.hpp'`
    returns no live code references.
- No new transport/network/VDF failures were introduced by the current slice state, but `pbft_manager_shim` and
  remaining pillar-chain external DPoS/materialization/event paths are still present and remain Slice 6 work.
- Additional validation for the pillar-chain runtime PBFT-finalization consolidation:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain_runtime_finalizes_block_for_pbft_with_owned_storage -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_chain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pillar_chain_test pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='*Pillar*:*pillar*:*Pbft*Final*' --gtest_print_time=1`
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.pillar_blocks_create:PillarChainTest.addVerifiedPillarVote_insertsWithRecoveredIdentityWeight:PillarChainTest.validatePillarVote_usesRustRecoveredIdentityForUniqueness:PillarChainTest.addVerifiedPillarVote_rejectsInvalidRustInspectedSignature:PillarChainTest.validatePillarVote_rejectsInvalidRustInspectedSignature' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - A parallel all-pillar gtest run failed from test-environment lock/port conflicts while another gtest process was
    active; the focused pillar tests and PBFT smoke passed when rerun sequentially.
- Additional validation for pillar-vote network egress bundle serving:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge network_bundle_chunks -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_votes -- --nocapture`
  - `cmake --build /build --target pillar_chain_test network_test --parallel 12`
  - `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.pillar_blocks_create:PillarChainTest.addVerifiedPillarVote_insertsWithRecoveredIdentityWeight:PillarChainTest.validatePillarVote_usesRustRecoveredIdentityForUniqueness:PillarChainTest.addVerifiedPillarVote_rejectsInvalidRustInspectedSignature' --gtest_print_time=1`
  - `/build/bin/network_test --gtest_filter='*Pillar*:*pillar*' --gtest_print_time=1` ran zero matching tests; the
    `network_test` target build provides the current tarcap handler link coverage.
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `git diff --check`
  - `rg -n "getVerifiedPillarVotes\\(" libraries/core_libs/network libraries/core_libs/consensus/shims tests/rust/consensus -g'*.cpp' -g'*.hpp'`
    shows no network serving callsites; remaining callsites are public compatibility/tests and non-network pillar-chain
    routes.
- The immediate follow-up is collapsing the now-typed PBFT finalization executor loop into a smaller manager-owned
  operation where practical, then continuing Slice 6 service consolidation and the later pillar-chain runtime work that
  still needs external DPoS fact ports plus legacy materialization removal.

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

Implementation status:

- Slice 7 is complete: consensus-facing C++ `FinalChain::finalizeExternalEvm` now uses one-shot
  `consensus_execution_prepare_external_evm_state_commit`, collapsing external publication planning, rewards-stat/proposal-period
  attachments, state-commit intent derivation, and pending-publication marker persistence into a single API boundary call.
- `BridgeConsensusExecutionApi` now exposes the one-shot bridge method in
  `rust/crates/rustaxa-bridge/src/final_chain.rs`, and it is declared in
  `rust/crates/rustaxa-bridge/src/ffi.rs`.
- `ConsensusExecutionApi` adds a matching `prepare_external_evm_state_commit` facade method that delegates to
  `final_chain_execution_session_prepare_external_evm_state_commit`.

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
- `BridgeStorage::seed_final_chain_conformance_lookup_rows` is deleted from the CXX bridge surface. A code-mapper audit
  identified it as a broad storage-method export with no production callsites. Rust bridge query fixtures seed FinalChain
  lookup rows through native `rustaxa-storage` `FinalChainStore::write_conformance_lookup_rows` test setup, and the
  storage conformance runner uses the dedicated `storage_shim_seed_final_chain_conformance_lookup_rows` fixture helper
  instead of a broad storage bridge mutator. The storage-boundary guard fails if that fixture helper gains new callers
  outside the conformance runner or bridge implementation.
- `BridgeTransactionStorageQueries::get_transaction_rlps_by_hashes` is deleted from the CXX bridge surface. Production
  DAG transaction availability and sync payload lookup continue through runtime-owned DAG APIs; the direct
  storage-query method only backed bridge-test scaffolding, and Rust bridge storage tests now cover pending, finalized,
  system, and missing transaction RLP lookup through the native helper.
- The standalone `inspect_pbft_finalization_resume` CXX export is deleted. Production duplicate-finalization recovery
  uses the runtime-owned `pbft_manager_runtime_inspect_finalization_resume` API from `pbft_manager_shim`, while Rust
  bridge and native consensus tests exercise the native resume inspector directly.
- The standalone `plan_pbft_finalization_intent` CXX export is deleted. Production PBFT finalization intent planning now
  enters through `pbft_manager_runtime_plan_finalization_intent`, and Rust bridge tests that need the bridge-shaped plan
  also construct a manager runtime before planning. The runtime is a boundary owner for this stateless fact-driven
  planner today, not an additional state source; future runtime policy should be added behind this API instead of
  reviving the standalone export. The native planner remains in `rustaxa-consensus`; the bridge-level direct wrapper is
  Rust-private test/support code only.
- `BridgePeriodStorageQueries::get_pbft_block_hash_by_period` is deleted from the CXX bridge surface. A caller audit
  found no C++ or Rust users; live compatibility paths keep `get_period_data_raw`, `get_period_from_pbft_hash`, and
  receipt reads.
- `BridgeFinalChain::get_vrf_key` and `BridgeFinalChain::estimate_call_gas` are deleted from the CXX bridge surface.
  Live C++ uses the block-scoped `get_vrf_key_at_block` route and the `FinalChain::call` gas-estimation adapter; the
  removed wrappers had no repo callers.
- `BridgeFinalChain::publish_external_evm_publication` is deleted from the CXX bridge surface. Live publication crosses
  CXX through `BridgeFinalChainExecutionSession` and `BridgeConsensusExecutionApi`; the remaining malformed-publication
  bridge tests now call native Rust `FinalChain::publish_external_evm_publication` through a private test helper.
- The unused CXX `BridgePbftVotePipelineSession` and `BridgePbftVoteAdmissionSession` exports are deleted. Their wrapper
  modules only protected bridge-shaped test scaffolding; production C++ had no callsites, and native
  `rustaxa-consensus` vote pipeline/admission tests now own the behavior coverage.
- `BridgePbftManagerStateActionEffectSession` is deleted. The C++ PBFT manager shim still executes live vote/block
  side effects, but the ordered state-action transcript is now a cursor inside `BridgePbftManagerRuntime`, reducing the
  PBFT manager CXX session surface by one internal handle.
- `BridgePbftManagerRuntimeSession` is deleted. The outer PBFT manager daemon-loop transcript is now a cursor inside
  `BridgePbftManagerRuntime`, so the scheduler no longer creates a standalone bridge handle each tick.
- The standalone PBFT manager sleep CXX planner and `PbftManagerSleepFact` DTO are deleted. The C++ PBFT manager shim
  now requires the long-lived runtime before sleeping and calls only the runtime-owned sleep API; the direct domain
  planner remains covered inside `rustaxa-consensus` rather than exported as an alternate bridge route.
- `BridgePbftManagerProposalSession` is deleted. PBFT block proposal planning is now a cursor inside
  `BridgePbftManagerRuntime`, so `pbft_manager_shim` no longer creates a standalone bridge handle for proposal
  construction.
- `BridgePbftManagerBlockValidationSession` is deleted. PBFT block validation planning now uses the stateless
  `plan_pbft_manager_block_validation` API with a C++-local fact bundle, so `pbft_manager_shim` no longer creates a
  standalone bridge handle or stores a validation cursor in `BridgePbftManagerRuntime`.
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
  existed solely for the removed free functions are deleted. `pbft_inspect_canonical_vote`, weighted vote payload
  conversion, and vote generation helpers remain because `vote_manager_shim` still calls them directly.
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
  uses `transaction_manager_runtime_pack_prepare_sharded` plus
  `transaction_manager_runtime_pack_finalize_with_estimates`, gas-estimation cache behavior is observable through the
  planner result, and non-finalized recovery uses the Rust-owned `transaction_manager_recover_nonfinalized_with_runtime`
  command.
- The older transaction-pack cursor CXX API is also deleted:
  `transaction_manager_runtime_pack_begin_sharded`, `transaction_manager_runtime_pack_request_next`,
  `transaction_manager_runtime_pack_record_estimate_step`, and the bridge-only `TransactionPackEstimateOutcome` DTO.
  The retained C++ shim route is the batch prepare/finalize API; Rust bridge tests for sharding, declared-gas, cached
  gas, candidate selection, and finalization now exercise that route instead of the retired request/record cursor.
- The stale `TransactionManagerInsertTransactionFact` and `TransactionManagerInsertTransactionOutcome` CXX DTOs are
  deleted from the bridge surface. They were not part of any exported CXX function signature; insertion admission now uses
  private Rust structs inside `transaction_manager.rs` before mapping to the higher-level admission command reports.
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
  `transaction_manager_runtime_queue_cleanup_with_account_nonce_facts`,
  `save_transactions_from_dag_block_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block_command_report_with_runtime_and_final_chain`,
  `save_transactions_from_dag_block`, `update_finalized_transactions_status`, and
  `transaction_manager_verify_not_finalized_with_runtime_and_final_chain` are deleted from the bridge surface. Live C++
  admission uses account-nonce facts emitted from proposable queue senders, DAG transaction persistence uses the
  runtime-owned command report, and finalized-status cleanup enters through the high-level runtime command.
  The queue cleanup helper is now private to `transaction_manager.rs`, and its bridge-only CXX DTO is deleted.
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
- The direct timestamp-supplied DAG proposer intent CXX export is deleted:
  `dag_proposer_plan_block_intent` and its bridge-only `DagProposerBlockIntentInput` DTO are no longer part of the CXX
  surface. Live C++ proposal construction uses `dag_proposer_plan_block_intent_with_current_timestamp` followed by
  `dag_proposer_finalize_signed_block_intent`, so Rust owns timestamp selection and signed-block intent derivation at
  the bridge boundary. Native `rustaxa-consensus` keeps the lower-level deterministic planner and fixed-timestamp tests.
  Custom agents used: `api-designer` confirmed this preserves the minimal external DAG proposer API, and
  `architect-reviewer` confirmed there is no in-repo live C++ caller, fallback path, or boundary ownership regression.
  Validation for this CXX export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag_proposer_block_intent -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag_proposer_block_intent -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='*Dag*Proposer*:*DAG*Proposer*:*dag*proposer*' --gtest_print_time=1`
    returned zero matching tests; the C++ bridge target build above covers generated CXX header integration for this
    deleted export.
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `rg -n "dag_proposer_plan_block_intent\\(|DagProposerBlockIntentInput" libraries tests/rust rust/crates/rustaxa-bridge/src rust/crates/rustaxa-consensus/src -g'*.rs' -g'*.cpp' -g'*.hpp'`
    now returns only native `rustaxa-consensus` planner/test references plus the retained bridge module's private domain
    type alias used by `dag_proposer_plan_block_intent_with_current_timestamp`.
  - `git diff --check`
- No-caller lower-level VDF/VRF helpers are no longer CXX exports:
  `vdf_sortition_payload_verify_with_modulus`, `vdf_sortition_threshold_from_output`,
  `vdf_sortition_normalize_vote_count`, `vdf_sortition_difficulty`, `vdf_sortition_legacy_modulus`,
  `vrf_proof_to_hash`, and `vrf_prove_output` are deleted from `rustaxa-bridge`'s CXX surface. Live C++ VDF integration
  keeps the coarse VDF object/prove/verify APIs, legacy VDF/VRF sortition prove/verify APIs, and the payload encode API
  used by DAG proposer code. The later bridge-test-only payload decode/verify and VRF output verification exports are
  also deleted from the CXX surface, along with `VdfSortitionVerifyConfig`, `VdfSortitionPayloadVerifyResult`, and
  `VrfVerifyOutput`. Native `rustaxa-vdf` tests retain coverage for payload decode/verify, scalar difficulty,
  vote-count normalization, explicit-modulus verification, legacy modulus, and VRF proof/hash behavior.
  Custom agents used: `api-designer` confirmed the retained operation-level VDF API shape and corrected the retained
  payload API wording; `architect-reviewer` confirmed no in-repo live C++ caller or fallback regression and noted the
  separate legacy `vrf_wrapper.cpp` C crypto compatibility path as remaining VRF ownership debt.
  Follow-up CXX surface cleanup deleted the test-only default `make_cancellation_token`, `cancellation_token_cancel`, and
  direct `verify_legacy_vrf_sortition` exports. The Rust bridge now exposes only the production atomic-backed
  cancellation-token constructor plus operation-level VDF/prove/verify and legacy sortition APIs; direct VRF verification
  is covered in native `rustaxa-vdf` tests instead of the CXX bridge surface.
  Validation for this CXX export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-vdf vdf_sortition -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-vdf vrf -- --nocapture`
  - `cmake --build /build --target rust_vdf_tests --parallel 12`
  - `/build/bin/rust_vdf_tests --gtest_print_time=1`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `rg -n "vdf_sortition_payload_verify_with_modulus|vdf_sortition_threshold_from_output|vdf_sortition_normalize_vote_count|vdf_sortition_difficulty|vdf_sortition_legacy_modulus|vrf_proof_to_hash|vrf_prove_output" libraries tests rust/crates/rustaxa-bridge/src rust/crates/rustaxa-vdf/src -g'*.rs' -g'*.cpp' -g'*.hpp'`
    now returns only native `rustaxa-vdf` internals/tests and the legacy C crypto symbol, not removed CXX bridge exports.
  - `rg -n "vdf_sortition_payload_decode|vdf_sortition_payload_verify|vrf_verify_output|VdfSortitionPayloadVerifyResult|VrfVerifyOutput|VdfSortitionVerifyConfig" libraries tests rust/crates/rustaxa-bridge/src rust/crates/rustaxa-vdf/src -g'*.rs' -g'*.cpp' -g'*.hpp'`
    now returns only native `rustaxa-vdf` and `rustaxa-consensus` internals/tests, not removed CXX bridge exports.
  - `git diff --check`
- `BridgeProposedBlocks::proposed_blocks_snapshot` is no longer a CXX export. Production C++ uses
  `proposed_blocks_snapshot_entries`, which preserves validation flags and payloads needed by the shim facade; grouped
  hash snapshots remain Rust test-only coverage.
- The no-storage `create_proposed_blocks_index` CXX constructor plus standalone
  `proposed_blocks_cleanup_candidates`/`proposed_blocks_remove_period` CXX helpers are deleted. Rust-mode
  `ProposedBlocks` now requires `DbStorage`, and the PBFT local proposal scratch path uses the storage-backed index with
  non-persisting `proposed_blocks_push` for temporary candidate admission.
- `BridgePbftChain::pbft_chain_project_update` is no longer a CXX export. The non-mutating append projection is covered
  by native `rustaxa-consensus` PBFT-chain tests, while live C++ bridge callers use `pbft_chain_update`,
  `pbft_chain_update_for_finalization`, or the retained legacy JSON projection facade.
- The duplicate storage-taking free `pbft_chain_block_exists(storage, hash)` and `pbft_chain_block_rlp(storage, hash)`
  CXX exports are deleted. The live `pbft_chain_shim` uses the storage-backed `BridgePbftChain` handle methods, keeping
  PBFT-chain block lookup tied to the runtime facade instead of a second direct `BridgeStorage` API.
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
  and `update_finalized_transactions_status_command_report_with_runtime_and_account_nonce_facts`; Rust-private helpers
  own the storage-first mutation and command-report conversion, while deleted wrapper exports that remain for direct Rust
  unit coverage are explicitly test-only.
  Custom agents used: `architect-reviewer` confirmed the live C++ command-report boundary; `rust-engineer` confirmed the
  private DTO/test impact.
- Follow-up no-caller transaction-manager exports are also deleted from the CXX surface:
  `create_transaction_manager_runtime` and
  `update_finalized_transactions_status_command_report_with_runtime`. Live C++ constructs the runtime from storage and
  reports finalized-status updates through the account-nonce/purge-aware command-report API; the deleted wrappers remain
  Rust-test-only fixtures behind `#[cfg(test)]`.
- Additional standalone PBFT runtime wrappers are no longer CXX exports:
  `plan_pbft_sync_runtime`, `abort_pbft_manager_proposal_session`,
  `load_pbft_finalization_last_period_lambda_storage`, `plan_pbft_dynamic_lambda`,
  `pbft_manager_runtime_load_finalization_last_period_lambda`, and the bridge-only `PbftSyncRuntimePlan` DTO are deleted
  from the bridge surface. Live C++ uses `plan_pbft_sync_process_period_data_runtime`,
  `plan_pbft_manager_block_validation`, proposal runtime sessions, and
  `pbft_manager_runtime_plan_finalization_dynamic_lambda`; native `rustaxa-consensus` tests cover the deleted lower-level
  planners and lambda lookup.
- FinalChain execution-session construction no longer takes a `BridgeFinalChain` parameter. The Rust bridge function
  ignored that handle, so C++ now creates the session from only `FinalChainExecutionRequest`; explicit `BridgeFinalChain`
  parameters remain only on commit, recovery, and publication calls that actually touch FinalChain storage/state.
  Custom agents used: `architect-reviewer` identified this cleanup and confirmed the PBFT manager standalone planner
  lane as a secondary cleanup candidate.
- The standalone `plan_external_evm_system_transactions` CXX export is also gone. `final_chain_shim` now plans external
  EVM system transactions through `BridgeConsensusExecutionApi::consensus_execution_plan_system_transactions`, so the
  execution client stays on the dedicated API while C++ still supplies the external StateAPI facts.
- The standalone `final_chain_execution_session_commit` CXX export is also gone. Native session commit now routes through
  `BridgeConsensusExecutionApi::consensus_execution_commit_session`, keeping both native and external-EVM execution
  advancement on the dedicated execution facade while still passing `BridgeFinalChain` only at the storage commit
  boundary.
- PBFT manager period-data queue scalar/hash metadata getters are no longer CXX exports:
  `pbft_manager_runtime_period_data_queue_period`,
  `pbft_manager_runtime_period_data_queue_syncing_period`,
  `pbft_manager_runtime_period_data_queue_last_block_hash_or_chain`,
  `pbft_manager_runtime_period_data_queue_size`, and `pbft_manager_runtime_period_data_queue_empty` are replaced by the
  single `pbft_manager_runtime_period_data_queue_snapshot` API. Live C++ still pushes, pops, clears, and cleans queue
  entries through command-style methods, but metadata reads now cross as one runtime-owned snapshot DTO.
- Follow-up cleanup removes the standalone bridge helper module `period_data_queue.rs`; push/pop/cleanup conversions are
  now private PBFT-manager runtime implementation details in `pbft_manager.rs`.
  Validation for this CXX export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge bridge_runtime_owns_period_data_queue_metadata -- --nocapture`
  - `cmake --build /build --target pbft_manager_test rust_consensus_tests --parallel 12`
  - `rg -n "pbft_manager_runtime_period_data_queue_(period|syncing_period|last_block_hash_or_chain|size|empty)\\b" rust/crates libraries tests -g'*.rs' -g'*.cpp' -g'*.hpp'`
    returns no matches.
- `plan_pbft_manager_state_action` and `plan_pbft_manager_state_action_effects` are deleted from the CXX surface.
  Their behavior is covered through `BridgePbftManagerRuntime` state-action sessions (`begin`/`next`/`report`), and the
  removed route no longer appears in `rust/crates/rustaxa-bridge/src/ffi.rs`.
  `RustPbftSyncTest.ManagerStateActionEffectSessionRecordsFinishPollingTranscript` was migrated to session assertions only.
  `bridge_runtime_owns_state_action_effect_session` and related session-level tests remain as coverage.
  Custom agents used: `cpp-pro` confirmed no remaining C++ callsites; `rust-engineer` confirmed bridge/runtime test
  coverage for ordered effects remains intact.
- Direct PBFT sync admission and transaction-query planners are no longer CXX exports:
  `plan_pbft_sync_period_admission`, `plan_pbft_sync_transaction_query`, and their bridge-only fact/plan DTOs are
  deleted from the bridge surface. Live C++ uses the staged `plan_pbft_sync_process_period_data_runtime` API, whose
  runtime plan still carries transaction-query output when the process-period executor needs it; native
  `rustaxa-consensus` tests cover the lower-level admission and transaction-query planners.
  Follow-up cleanup also deleted the stale `PbftSyncPeriodAdmissionFact` CXX DTO that no live C++ caller used after the
  direct admission planner was removed.
  Custom agents used: `rust-engineer` identified these two direct planners as bridge-test-only exports after the live
  PBFT sync route moved to the staged runtime API.
  Validation for this CXX export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_sync`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_sync`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.*' --gtest_print_time=1`
- Lower-level FinalChain execution API helpers that were superseded by the one-shot
  `consensus_execution_prepare_external_evm_state_commit` call are no longer CXX exports:
  `consensus_execution_plan_publication`, `consensus_execution_attach_rewards_stats`,
  `consensus_execution_attach_proposal_period_dag_level`, `consensus_execution_next_state_commit_request`,
  `consensus_execution_persist_pending_publication`, and `consensus_execution_publication_audit`. The live
  `final_chain_shim` path still uses `BridgeConsensusExecutionApi` for external-EVM/`StateAPI` interaction, and the CXX
  bridge keeps session creation/commit plus the minimal step/report/publish methods that are still called by that
  external execution adapter. Follow-up cleanup moved bridge tests off the obsolete publication-plan DTOs and deleted
  `FinalChainExternalEvmPublicationPlan` plus `FinalChainExternalEvmTransactionPublication` from the CXX surface. A
  second cleanup deleted the oversized `FinalChainExternalEvmCommitPlan` CXX DTO; live C++ now receives only
  `FinalChainExternalEvmCommitReport` with request id, period, and error text before calling the one-shot state-commit
  preparation API, while Rust tests assert roots, blooms, receipts, and counters through the native commit plan.
  Custom agents used: `rust-engineer` confirmed the live C++ route and identified the remaining Rust-internal wrapper
  callsites, while `api-designer` confirmed the publication-plan DTOs were not live C++ and recommended the follow-up
  `FinalChainExternalEvmCommitPlan` shrink that is now complete.
  Validation for this publication-plan DTO shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain -- --nocapture`
  - `cmake --build /build --target final_chain_test rust_consensus_tests --parallel 12`
  - `/build/bin/final_chain_test --gtest_filter='FinalChainTest.*' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `rg -n "FinalChainExternalEvmCommitPlan|FinalChainExternalEvmCommitReport" rust/crates/rustaxa-bridge/src/ffi.rs rust/crates/rustaxa-bridge/src/final_chain.rs libraries/core_libs/consensus/shims/final_chain_shim tests/rust/consensus -g'*.rs' -g'*.cpp' -g'*.hpp'`
    now returns only the CXX `FinalChainExternalEvmCommitReport` plus native Rust test/internal commit-plan references,
    not a CXX `FinalChainExternalEvmCommitPlan` export.
  Validation for this commit-plan DTO shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain -- --nocapture`
- The older direct `BridgeFinalChain::finalize_block*` compatibility exports and bridge-only `FinalizationOutcome` DTO
  are deleted. FinalChain execution now crosses the CXX bridge through `BridgeFinalChainExecutionSession` and
  `BridgeConsensusExecutionApi`; native `rustaxa-consensus` FinalChain tests cover direct native finalization.
  Custom agents used: `code-mapper` identified additional orphan CXX exports and confirmed this cleanup class has no
  production C++ callsites.
  Validation for this direct finalizer export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager_runtime_lookup_transaction_views_with_final_chain_marks_old_finalized`
  - `cmake --build /build --target final_chain_test rust_consensus_tests --parallel 12`
  - `/build/bin/final_chain_test --gtest_filter='FinalChainTest.*' --gtest_print_time=1`
  Validation for this FinalChain conformance seed export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge query::tests::bridge_consensus_query_api_reads_public_block_view`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge query::tests::bridge_consensus_query_api_reads_indexed_transaction_view`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge query::tests::bridge_consensus_query_api_reads_transaction_receipt`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  Validation for this transaction-RLP storage-query export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_rlp_batch_lookup_reads_pending_finalized_system_and_missing`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  Validation for this standalone PBFT finalization resume inspector export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge resume_inspector_classifies_primary_finalization_crash_windows`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus resume_inspector_classifies_storage_backed_duplicate_restart_windows`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  Validation for this CXX export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`
  - `cmake --build /build --target final_chain_test --parallel 12`
  - `/build/bin/final_chain_test --gtest_filter='FinalChainTest.*' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `git diff --check`
  Validation for this transaction pack cursor export shrink:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager_runtime_pack -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests --parallel 12`
  - `cmake --build /build --target taraxad --parallel 12`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `rg -n "TransactionPackEstimateOutcome|transaction_manager_runtime_pack_(begin_sharded|request_next|record_estimate_step)|pack_begin_sharded|pack_request_next|pack_record_estimate_step|transaction_manager_runtime_pack_step_inner" rust/crates/rustaxa-bridge/src libraries tests -g'*.rs' -g'*.cpp' -g'*.hpp'` returned no matches.
  - `git diff --check`
- Additional no-caller CXX exports are deleted after callsite audit showed they were bridge-test scaffolding only:
  `create_pbft_chain_with_storage`, `slashing_mark_double_voting_proof_submission`,
  `pillar_votes_get_verified_votes`, and `pillar_votes_snapshot_refs`. Live C++ paths use
  `create_pbft_chain_from_storage`, `slashing_report_double_voting_proof_submission`, and runtime-owned
  pillar vote payload lookup APIs.
- The slashing CXX planner input is narrowed from duplicated vote coordinate fields to one shared PBFT slot plus the two
  canonical vote payloads. Rust bridge code expands this evidence-shaped CXX DTO into the consensus-domain input while
  the C++ compatibility adapter rejects mismatched live `PbftVote` slots before planning.
- The slashing-only `pbft_vote_slashing_payload_from_canonical_vote` CXX helper is deleted. Rust vote admission still
  normalizes slashing evidence inside `rustaxa-consensus`; the live `PbftVote` compatibility adapter now constructs the
  evidence DTO directly from unweighted canonical vote bytes and the live vote hash.
- The bridge-only `DoubleVotingProofSubmissionPlan` CXX DTO is deleted. Rust consensus keeps the richer submission
  classification internally, while the C++ slashing facade receives only the submitted/not-submitted boolean it uses for
  executor flow.
- The remaining standalone pillar-vote CXX surface is deleted after
  `PillarVoteBundleBridgeTest.applyPillarVoteBundleFromWeightedRlpsInsertsAcceptedVotes` moved to
  `BridgePillarChainRuntime`. Follow-up cleanup moved the residual Rust-only pillar-vote fixture out of `ffi.rs`,
  renamed it as a module-local test fixture in `pillar_votes.rs`, and removed the retired handle name from bridge code.
- The no-caller `pillar_chain_runtime_cleanup_votes_by_period` CXX export is deleted; live pillar cleanup remains
  manager/runtime-owned and no C++ shim or bridge test calls the standalone cleanup method.
- Additional no-caller verified-vote and sortition CXX exports are deleted:
  `verified_votes_check_unique_voter`, `verified_votes_vote_in_verified_map`,
  `verified_votes_get_network_t_plus_one_step`, `verified_votes_get_two_t_plus_one_voted_block_votes`,
  `verified_votes_snapshot_weighted_payloads`, and `sortition_restore_finalized_period`. Live C++ verified-vote paths
  use admission, payload lookup, retained-payload 2t+1 lookup, round-marker snapshots, and explicit sortition
  record/persist APIs instead.
- `BridgePillarChainStorage::pillar_chain_storage_block_data_rlp` is deleted from the CXX bridge surface. Rust-mode
  Taraxa RPC pillar block-data reads use `BridgeConsensusQueryApi::consensus_query_pillar_block_data_by_period`, while
  pillar/storage shims retain only the narrower current/latest block, own-vote, finalized-block, and period-data storage
  methods they call.
- The standalone `plan_pillar_vote_relevance` CXX export is deleted. The live network/tarcap client uses
  `BridgeConsensusNetworkApi::consensus_network_plan_pillar_vote_relevance`, and the live pillar-chain manager client
  uses `BridgePillarChainRuntime::pillar_chain_runtime_plan_vote_relevance`. The removed direct bridge export only
  protected bridge-shaped C++ test scaffolding; direct planner coverage remains in native `rustaxa-consensus` tests and
  bridge-module tests, while C++ network API coverage exercises the external tarcap-facing facade.
  Validation for this pillar relevance export shrink:
  - `rtk cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `rtk cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `rtk cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_vote -- --nocapture`
  - `rtk cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_vote -- --nocapture`
  - `rtk cmake --build /build --target rust_consensus_tests --parallel 12`
  - `rtk /build/bin/rust_consensus_tests --gtest_filter='PillarVoteBundleBridgeTest.*:PillarChainPlanningBridgeTest.*:PillarVoteInspectionBridgeTest.*:ConsensusNetworkApiBridgeTest.pillarVoteRelevancePlanningRoutesThroughNetworkApi' --gtest_print_time=1`
  - `rtk scripts/rewrite_bridge_inventory_guard.sh`
  - `rtk scripts/rewrite_storage_boundary_guard.sh`
  - `rtk rg -n "pub fn plan_pillar_vote_relevance|\\bplan_pillar_vote_relevance\\(" rust/crates/rustaxa-bridge/src/ffi.rs tests/rust/consensus libraries/core_libs/consensus/shims libraries/core_libs/network -g'*.rs' -g'*.cpp' -g'*.hpp'`
    returns no matches, proving the direct CXX export and its C++ bridge-test callers are gone.
- The standalone broad `apply_rewards_stats_storage_writes` CXX export is deleted. The follow-up no-production-caller
  `BridgeRewardsStatsRuntime::rewards_stats_runtime_apply_storage_writes` CXX method and its remaining test-only Rust
  bridge wrapper are also deleted. Rewards-stat storage writes now enter through the dedicated storage-shim batch
  appender for staged compatibility writes, while direct owned-storage apply coverage calls the native consensus helper
  from bridge-module tests instead of preserving a bridge-shaped wrapper.
- FinalChain rewards-stat publication no longer exposes the full `RewardsStatsProcessResult` through
  `FinalChainPublicationRewardsStats`. The rewards shim keeps the previewed process plan as internal pending state and
  FinalChain receives only decoded distribution stats plus the storage-update DTO that `BridgeConsensusExecutionApi`
  needs for the atomic publication batch. The C++ commit call is now a zero-argument acknowledgement after Rust
  FinalChain publication succeeds.
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
- The direct `sortition_params_for_period(found, change)` CXX export is deleted. C++ sortition callers now use only
  `sortition_params_for_period_from_storage(period)` for historical lookups, while direct change-payload lookup coverage
  remains in native Rust sortition tests.
- FinalChain Rust-mode startup, native finalization, external-EVM publication, crash recovery, and storage audit are
  closed under the current overlay. The remaining public `DbStorage` batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` are legacy-only when `RUSTAXA_ENABLE_FINAL_CHAIN` is
  enabled; Rust-mode publication enters `BridgeFinalChain`/`BridgeConsensusExecutionApi` and commits FinalChain storage
  rows through native Rust storage. `StateAPI` remains the external EVM/state database boundary.
- The broader Slice 8 API shrink remains open; this guard is the closeout mechanism for future bridge-handle deletions
  and additions.
- PBFT finalization report-surface cleanup removed the bridge-only `PbftFinalizationLiveMutationReport` CXX DTO and the
  PBFT manager shim's `makeFinalizationExternalEffectReport` mapper. Follow-up cleanup removed the public generic
  `PbftFinalizationExternalEffectReport` CXX DTO and the generic advancement API entirely. Live C++ finalization
  executors now report success through typed subsystem APIs and failure through
  `pbft_manager_runtime_fail_finalization_external_effect`; Rust bridge internals build native live-mutation reports
  after deriving finalization identity from `BridgePbftManagerRuntime`.
- The standalone `apply_pbft_finalization_storage_writes` CXX export is deleted. Live manager-owned finalization storage
  writes enter through `BridgePbftManagerRuntime`, while the retained verified-votes storage API remains the compatibility
  surface for vote-manager finalization storage facts. The lower test-only bridge wrapper is deleted; direct
  storage-apply scenarios now call the native consensus helper through a private bridge-module test adapter.

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

### Implementation status

- After the prior test-surface deletions, the remaining suite coverage in both C++ and Rust now centers on either:
  - external-facing shim/public API behavior still intentionally used by the app, RPC, or GraphQL clients, or
  - native Rust module behavior that owns the production route.
- No remaining active CXX-only test-only session/planner paths were found that map directly to deleted exports that had no Rust or public-facing replacement; follow-up work is now primarily to keep this boundary healthy as new surface is deleted in future slices.
- Validation and maintenance checkpoints run during this closeout:
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_sync`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus network_api`
  - `cmake --build /build --target rust_consensus_tests final_chain_test pbft_manager_test --parallel 12`

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
