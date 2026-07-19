# Consensus Consolidation Plan

This is the cleanup wave after the dedicated network, execution, and query facades landed: shrink or remove bridge/shim
scaffolding made obsolete by those boundaries, and consolidate Rust consensus/storage usage so internal Rust paths no
longer look like C++ compatibility paths. `PLAN.md` owns the strategic facade boundaries,
`doc/consensus_rewrite_tracker.md` owns the current dependency-ordered **Remaining Consensus Work Queue**, and this
document retains detailed slice design, implementation history, and compatibility-deletion rationale.

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

## Agent Routing

Implementation slices from this plan use `$implement-rustaxa-consensus-slice`. Route each material workstream to the
configured custom agent named by the matching rule below, and use only the roles whose scope applies:

- `api-designer`: review Rust/C++ facade shape, DTO minimality, bridge compatibility, and whether a proposed API keeps
  consensus-internal callers out of shim/bridge routes.
- `architect-reviewer`: review ownership boundaries, shim removal strategy, fallback risk, external-boundary discipline,
  and whether the slice leaves obsolete compatibility code behind.
- `rust-engineer`: implement or review Rust consensus, storage, bridge, codec, native service, and Rust test changes.
- `cpp-pro`: implement or review C++ shim, CMake, bridge wiring, RPC/GraphQL, tarcap adapter, and C++ test changes.

When Codex delegates, keep assignments concrete and non-overlapping. The primary implementer still owns integration,
local code inspection, conflict resolution, validation, deletion of obsolete scaffolding, and the final closeout report.

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
- Gas-pricer finalized-history restoration is construction-time-only through the composed
  `create_transaction_manager_runtime_from_storage` constructor. The obsolete late storage-injection helper and the
  later standalone gas-pricer constructors are deleted; transaction state and gas history now publish through one
  runtime owner.
- Validation for the gas-pricer storage-injection cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_runtime_`

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
- Slice 4 storage-shim direct-mutator cleanup and the original consensus-module batch classification are complete for
  the audited paths. Remaining public `DbStorage::createWriteBatch()` / `commitWriteBatch()` blocks are either excluded
  by an authoritative Rust-mode overlay or retained in an explicitly classified pure-C++ reference path; they must not
  justify new storage-shim APIs.
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
  paths is complete. The original consensus-module public batch blocks are classified below as excluded
  legacy/reference code behind authoritative Rust-mode overlays.
- PBFT manager reset, finish-polling, loopback-finish, and finalization public batch blocks are closed for the current
  Rust-mode route. The active `pbft_manager_shim` overlay overrides those methods and routes transition persistence
  through the manager-owned lifecycle transition executor, which commits manager cursor/status updates,
  cert-voted-block removal, and own-verified-vote cleanup in one native Rust storage batch before updating runtime and
  returning narrow C++ sidecar commands. Executed-block reset and finalization storage writes are also Rust-owned runtime/finalization calls with
  explicit external boundaries for finalization execution and sidecar materialization. The public batch blocks in
  `libraries/core_libs/consensus/src/pbft/pbft_manager.cpp` remain legacy/reference behavior behind
  `RUSTAXA_ENABLE_PBFT_MANAGER`.
- Proposed-block persistence is closed for the current Rust-mode route. The active `proposed_blocks_shim` overlay owns
  its compatibility surface while `BridgePbftService` owns save, startup restore, and stale-period cleanup. Cleanup
  plans stale period/hash groups, commits one native Rust storage delete batch, and mutates the service-owned index only
  after commit. `BridgeProposedBlocks` and its factory are deleted. The public batch loop in
  `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` remains legacy/reference behavior behind
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` and should not drive new storage-shim API expansion.
- Sortition parameter persistence is closed for the current Rust-mode route. The active
  `sortition_params_manager_shim` overlay constructs `BridgeSortitionParamsManager` from Rust storage, persists the
  missing period-zero default change in Rust during startup, ignores the legacy `Batch&` argument in `pbftBlockPushed`,
  and persists emitted finalized-period changes through the Rust runtime before live state is updated. The public batch
  compatibility carrier and its canonical RLP codec are shim-owned. The untouched original source is excluded in master
  `RUSTAXA_ENABLE` mode and remains available only to pure-C++ reference builds.
- FinalChain block publication is closed for the current Rust-mode route. The active `final_chain_shim` overlay is a
  standalone facade over `BridgeFinalChain` and `BridgeConsensusExecutionApi`; native finalization, external-EVM pending
  publication markers, recovery, storage publication, execution counters, rewards-stat attachment, transaction indexes,
  receipts, log blooms, and genesis header creation are committed through native Rust storage. The public batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` remain untouched pure-C++ reference behavior and the
  source is excluded when `RUSTAXA_ENABLE_FINAL_CHAIN` selects the standalone overlay. Rust mode keeps the external
  `StateAPI`/EVM adapter but does not route FinalChain
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
4. `transaction_queue_shim` (retired in this slice)
5. `gas_pricer_shim`
6. `rewards_stats_shim`
7. `sortition_params_manager_shim`
8. `slashing_manager_shim`
9. `key_manager_shim`

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
- The standalone verified-votes facade is fully detached from the dead legacy compile scaffold. Verified-votes-enabled
  builds exclude the untouched original `verified_votes.cpp`, the overlay wrapper directly includes the Rust-backed
  facade, and the former compile rename and assertion-only non-inheritance test are deleted. The exact public carrier
  names, field order, defaults, container types, and threshold-marker values now live in a documented shim-owned
  compatibility header; they remain non-authoritative materialized views for C++ VoteManager, storage, PBFT, and network
  consumers. The feature flag, `BridgeVerifiedVotes`, storage-backed construction, facade APIs, and pure-C++ original
  implementation remain unchanged. Focused validation passed 20 Rust consensus tests, 14 Rust bridge tests, both
  verified-votes shim tests, four isolated VoteManager consumer tests, two isolated PBFT manager consumer tests, all nine
  Rust storage bridge tests, the `taraxad` build, Tier 1, Tier 2 consensus, and startup smoke. Feature-on build metadata
  and the core archive contain neither the original source object nor any `VerifiedVotesOld` symbol; original header and
  source diffs versus `upstream-main` are empty. An all-Rust-off configuration selected and compiled the untouched
  original verified-votes source without the shim or rename before the broader target reached the pre-existing missing
  `PillarChainManager::buildVerifiedPillarVoteNetworkBundles` pure-C++ network API error. Mapping, API, architecture,
  C++ implementation, and independent review use the code-mapper, api-designer, architect-reviewer, cpp-pro, and reviewer
  agents. No Rust or blockchain/EVM implementation agent was needed because runtime, bridge, storage, and contract
  behavior did not change.
- The standalone proposed-blocks facade is fully detached from `ProposedBlocksOld`. Feature-on builds exclude the
  untouched original `proposed_blocks.cpp`, the overlay wrapper directly includes the Rust-backed facade, and the compile
  rename plus assertion-only inheritance test are deleted. No public compatibility carriers required extraction; the
  unused legacy-only `checkOldBlocksPresence` diagnostic remains intentionally retired from the facade.
  At that slice boundary, `BridgeProposedBlocks` remained the authoritative owner of metadata, canonical block RLP,
  persistence, startup restore, and atomic stale-period cleanup. `CRW-03` subsequently moved that ownership into
  `BridgePbftService` and deleted the standalone bridge handle, its factory, and the facade mutex. The feature flag,
  public facade API, and temporary `PbftBlock` return materialization remain until compatibility consumers move behind
  native Rust runtime APIs. Module-disabled and pure-C++ builds continue to select the untouched original implementation.
  Validation passed with eight focused Rust consensus tests, five Rust bridge tests, six standalone facade tests, two
  CXX proposed-block tests, the focused PBFT manager consumer test, all nine Rust storage bridge tests, the `taraxad`
  build, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke`. Feature-on
  build metadata and the core archive contain neither the original source object nor any `ProposedBlocksOld` symbol;
  the original header and source diffs versus `upstream-main` are empty. An all-Rust-off configuration selected and
  compiled the untouched original source without the shim or rename. Mapping, API, architecture, Rust ownership, C++
  implementation, and independent review used the code-mapper, api-designer, architect-reviewer, rust-engineer,
  cpp-pro, and reviewer agents. No blockchain/EVM agent was needed because contract execution was outside this slice.
- The standalone PBFT-chain facade is fully detached from `PbftChainOld`. Feature-on builds exclude the untouched
  original `pbft_chain.cpp`, the overlay wrapper directly includes the Rust-backed facade, and the compile rename plus
  Old-only inheritance assertion are deleted. `BridgePbftChain` remains authoritative for startup restore/default
  initialization, in-memory head state, block existence and canonical RLP lookup, head projection/update, validation,
  and typed finalization-update facts. It clones its own shared Rust storage owner, allowing the facade to delete its
  redundant C++ `DbStorage` lifetime sidecar; focused Rust and C++ tests explicitly destroy the originating storage
  facade before runtime-owned lookups. C++ retains only the public facade, shared lock, legacy JsonCpp formatting, typed
  manager finalization adapter, and temporary `PbftBlock` materialization until PBFT-chain state and callers fold into
  `BridgePbftManagerRuntime`. PBFT-chain mode now explicitly requires master Rust mode and storage, while PBFT-manager
  overlays activated by proposed-block or pillar-vote mode explicitly require the PBFT-chain facade. Module-disabled and
  pure-C++ builds continue to select the untouched original implementation.
  Validation passed with nine focused native Rust consensus tests, five Rust bridge tests, four CXX PBFT-chain tests,
  the standalone facade storage-lifetime test, both PBFT-chain integration tests, the focused single-node PBFT manager
  consumer, all nine Rust storage bridge tests, the `taraxad` build, both boundary guards,
  `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke`. Configuration
  coverage proves PBFT-chain mode rejects missing master/storage dependencies, proposed-block and pillar-vote modes each
  reject a missing PBFT-chain facade, and an all-Rust-off build selects and compiles the untouched original source.
  Feature-on build metadata and the core archive contain neither the original source object nor any `PbftChainOld`
  symbol; original header and source diffs versus `upstream-main` are empty. Mapping, API, architecture, Rust/C++
  implementation, and independent review used the code-mapper, api-designer, architect-reviewer, rust-engineer, cpp-pro,
  and reviewer agents. No blockchain/EVM agent was needed because contract execution was outside this slice.
- The standalone DAG graph facade was first detached from `DagOld`/`PivotTreeOld` and is now retired entirely.
  `BridgeDagGraph`, its CXX methods, `dag_shim`, and bridge-mechanics tests are deleted because Rust-enabled production
  already owns total-DAG and pivot-tree state inside `BridgeDagManagerRuntime`. Rust-disabled pure-C++ builds continue
  to select the untouched original Boost graph implementation, while Rust mode excludes `dag.cpp` and relies on native
  `DagGraph` tests plus production `DagManager` coverage.
  Retirement validation passed 87 focused Rust consensus DAG tests, 28 Rust bridge DAG-manager/proposer tests, all 286
  bridge tests, all six Rust-mode `dag_test` manager cases, all 13 `dag_block_test` cases, the focused PBFT manager
  consumer build, the `taraxad` startup smoke, both boundary guards, and `make rewrite-validate-fast`. The broader
  consensus gate retained the known reward-cursor bootstrap fixture failures, and the Tier 3 CTest gate passed 21 of 27
  binaries with the same five in-process `/tmp/taraxa0` RocksDB-lock failures plus the known Go/cgo static-link failure.
  Feature-on build metadata and the core archive contain neither the original source object nor any
  `DagOld`/`PivotTreeOld` symbol; original DAG and manager source/header diffs versus `upstream-main` remain empty.
- DAG proposer sessions now derive and fingerprint frontier, proposal-period, and period-hash observations inside
  `BridgeDagManagerRuntime`. FinalChain and sortition facts are collected outside the runtime lock and accepted only
  after revalidation; graph changes terminate the attempt before transaction packing or retry mutation. VDF polling and
  stale-proof resume also derive the current proposal level inside the runtime, so C++ no longer echoes DAG-owned
  frontier, proposal-period, or latest-level facts through standalone CXX exports/report carriers. An idempotent abort
  route plus a C++ scope guard removes every live cursor on normal return or exception; fallible Rust report paths also
  remove the cursor before returning, so a session cannot retain wallet-secret material after its caller unwinds.
  Validation passed all 17 focused native Rust proposer tests, all 289 Rust bridge tests, all 13 `dag_block_test` cases,
  all six Rust-mode `dag_test` manager cases, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and the
  Rust-enabled startup smoke. The task-owner-preapproved Tier 3 CTest gate passed 21 of 27 binaries; its failures retained
  the known same-process `/tmp/taraxa0` RocksDB-lock fixture defect in `pillar_chain_test`, `full_node_test`,
  `network_test`, `pbft_manager_test`, and `vote_test`, plus the unrelated Go/cgo static-link failure. The Python Tier 3
  command remained blocked before collection because the Python 3.13 environment lacks development headers required to
  build its pinned `cytoolz` and `pyethash` dependencies. Original DAG manager and proposer files remain clean versus
  `upstream-main`.
- DAG proposer block construction and canonical assembly now remain inside the same keyed Rust session. After VDF
  execution, Rust revalidates the observation, loads tip metadata, applies gas/tip policy, selects the timestamp, and
  stores the unsigned intent. C++ receives only its signing hash and returns only signature bytes; Rust validates the
  signature and returns canonical signed block RLP/hash for the existing add-block executor. The standalone block-
  construction planner, current-timestamp intent planner, signed-intent finalizer, and their bridge-only carriers are
  deleted; public `selectDagBlockTips` compatibility planning remains. Session begin also retains the configured
  proposer address: native finalization rejects unrecoverable signatures, and the bridge requires the recovered signer
  to match that address before exposing an add-block payload. Invalid and wrong-key signatures remove the cursor without
  retry mutation. Validation passed all 291 Rust bridge tests, five focused native block-construction tests, two focused
  native tip-selection tests, three native intent/signature tests, all 13 `dag_block_test` cases,
  all six Rust-mode `dag_test` cases, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and the
  Rust-enabled startup smoke. The task-owner-preapproved Tier 3 CTest gate again passed 21 of 27 binaries; the same five
  same-process RocksDB-lock fixture failures and unrelated Go/cgo static-link failure remain. The Python Tier 3 setup
  failure from this validation round remains applicable because its pinned native dependencies cannot build in the
  unchanged Python 3.13 environment.
- The standalone DAG block proposer facade is fully detached from `DagBlockProposerOld`. Feature-on builds exclude the
  untouched original `dag_block_proposer.cpp`, the overlay wrapper directly includes the self-contained executor
  facade, and the compile rename is deleted. The facade now explicitly includes its configuration, thread-pool, and
  standard-library dependencies. After the PBFT manager legacy-header import was detached, the facade also replaced its
  broad `network/network.hpp` include with a `Network` forward declaration, closing the former rename-macro include-order
  debt. Rust still
  owns proposal sessions, retry state, eligibility and tip policy, transaction-pack commands, VDF control decisions,
  timestamps, and signed-RLP planning; C++ retains the classified worker/network, VDF execution, node-secret signature,
  compatibility-materialization, logging, and add-block executor shell. Pure-C++ builds continue to select the
  untouched original proposer.
  Validation passed 17 focused native Rust proposer tests, three focused Rust bridge proposer tests, all 13
  `dag_block_test` cases, all 13 `dag_test` cases, the focused single-node PBFT manager consumer, the `taraxad` build,
  both boundary guards, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and
  `make rewrite-validate-smoke`. Feature-on build metadata and the core archive contain neither the original source
  object nor any `DagBlockProposerOld` symbol; original header and source diffs versus `upstream-main` are empty. An
  all-Rust-off configuration selected and compiled the untouched original source without the shim or rename. Mapping,
  API, architecture, C++ implementation, and independent review used the code-mapper, api-designer,
  architect-reviewer, cpp-pro, and reviewer agents. No Rust or blockchain/EVM implementation agent was needed because
  runtime, bridge, storage, and contract behavior did not change.
- The standalone PBFT manager facade is fully detached from `PbftManagerOld`. Proposed-block or pillar-vote builds
  exclude the untouched original `pbft_manager.cpp`, the overlay wrapper directly includes the shim-owned facade, and
  the compile rename is deleted. The facade owns the stable PBFT state and state-root-validation enums with their
  original values. It retains the historical public network and FinalChain include chain because a broad `core_libs`
  build proved that upstream network consumers still rely on those transitive definitions; the dangerous legacy-header
  rename/import is gone. The empty header-only shim translation unit and Old-identity assertion are deleted. This is
  scaffold removal only: `BridgePbftManagerRuntime`, live executor behavior, storage, transport, and public API behavior
  are unchanged. Module-disabled and pure-C++ builds retain the untouched original manager.
  Validation passed 86 focused native Rust PBFT manager tests, 62 focused Rust bridge PBFT manager tests, the focused
  single-node PBFT manager consumer, two isolated VoteManager consumers, all 13 DAG-block consumers, the PBFT CXX
  runtime suite through Tier 2, feature-on target builds through `taraxad`, both rewrite boundary guards, Tier 1, and
  the startup smoke gate. Feature-on build metadata and the core archive contain neither the original manager object nor
  a `PbftManagerOld` symbol; an all-Rust-off configuration selected and object-compiled the untouched original source,
  and original-file diffs versus `upstream-main` are empty. Proposed-only and pillar-only configurations also select
  only the overlay under the established feature predicate; their standalone object builds continue to expose the
  pre-existing requirement for companion consensus-module APIs, without restoring a legacy fallback. Mapping, API,
  architecture, C++ implementation, and independent review used the code-mapper, api-designer, architect-reviewer,
  cpp-pro, and reviewer agents. No Rust or blockchain/EVM implementation agent was needed because runtime, bridge,
  storage, and contract behavior did not change.
- The standalone KeyManager facade is fully detached from `KeyManagerOld`. Master Rust builds exclude the untouched
  original `key_manager.cpp`, the overlay wrapper directly includes the self-contained facade, and the compile rename
  plus dead base construction are deleted. The facade preserves its public constructor and `getVrfKey` API, address-keyed
  cache, Rust FinalChain lookup order (`block`, prior block when available, then next block), and missing/future-block
  `nullptr` behavior. The retained `FinalChain` pointer is the classified external key-fact adapter; it is not legacy
  delegation. Pure-C++ builds continue to select and object-compile the untouched original implementation. Validation
  passed feature-on builds through `taraxad`, the focused single-node PBFT manager consumer, both focused vote
  consumers, all 13 DAG-block and all 12 DAG consumers, the focused pillar-block construction consumer, Tier 1, Tier 2,
  and the startup smoke gate. Feature-on build metadata and the core archive contain neither the original manager object
  nor a `KeyManagerOld` symbol, and original-file diffs versus `upstream-main` are empty. Mapping, API, architecture,
  C++ implementation, and independent review used the code-mapper, api-designer, architect-reviewer, cpp-pro, and
  reviewer agents. No Rust or blockchain/EVM implementation agent was needed because
  key lookup behavior, Rust bridge behavior, storage, and contract execution did not change.
- `pillar_votes_shim` is retired. C++ now routes live pillar vote indexing and planning through
  `BridgePillarChainRuntime` inside `pillar_chain_manager_shim`. `RUSTAXA_ENABLE_PILLAR_VOTES` no longer wires
  `pillar_votes_shim`, `pillar_votes.cpp` is no longer compiled as `PillarVotesOld`, and
  `pillar_votes_shim_test.cpp` was removed.
- The standalone pillar-chain manager facade is fully detached from `PillarChainManagerOld`. Pillar-votes builds
  exclude the untouched original manager and the now-unreferenced legacy `PillarVotes` implementation, the wrapper
  directly includes the shim-owned facade, and the compile rename is removed. Module-disabled and pure-C++ builds keep
  both original sources. This is scaffold removal only: `BridgePillarChainRuntime` behavior, the separate
  `BridgePillarChainStorage` compatibility surface, FinalChain fact reads, network transport, signing/materialization,
  events, and finalization effects are unchanged. Validation passed 46 focused native consensus pillar tests, 44
  focused bridge pillar tests, all eight focused CXX pillar bridge tests, the focused single-node PBFT consumer,
  feature-on target builds through `taraxad`, both boundary guards, Tier 1, Tier 2, and the startup smoke gate.
  Feature-on build metadata and the core archive contain neither original pillar implementation nor old-manager/
  legacy-vote-index symbols; module-disabled and pure-C++ configurations select and object-compile both untouched
  sources, and original-file diffs versus `upstream-main` are empty. The monolithic `pillar_chain_test` invocation
  passed 10 of 13 cases but its later node-owning cases encountered the known same-process `/tmp/taraxa0` RocksDB lock;
  this source-selection-only slice does not change manager runtime behavior, and the independently run single-node PBFT
  consumer passed.

- The pillar runtime lifetime is now absorbed by the App-owned `BridgePbftService`. Full service construction restores
  one private pillar state, `PillarChainManager` replays startup data on that same owner, and an independent readiness
  transition prevents PBFT startup or live pillar calls from observing partial state. Production App injects its shared
  service; `BridgePillarChainRuntime`, its factory, and all old runtime receiver exports are deleted. Chain-only services
  fail explicitly, while the narrowly named pillar-capable partial factory is limited to compatibility construction and
  tests. PBFT's four pure current-anchor decisions call the shared service directly; public manager methods remain
  compatibility wrappers. `BridgePillarChainStorage`, FinalChain/DPoS facts, signing, network/events, and C++ object
  materialization remain classified boundaries.
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
- `vote_manager_shim` is now a standalone full overlay: it no longer inherits from or constructs `VoteManagerOld`, owns
  its compatibility state directly, and restores the upstream header's temporary protected hook to its original private
  shape. The shim already implemented the complete public surface, so this removes legacy constructor execution and
  implicit inherited fallback without changing the public C++ API.
- Verified-vote startup is now authoritative in Rust. The storage-backed PBFT service constructor reads own
  votes, extra reward votes, and typed latest-round 2t+1 bundles through `rustaxa-storage`, validates canonical weighted
  payloads and bundle coordinates, deduplicates overlapping families by vote hash, and rebuilds Rust replay, retained
  payload, uniqueness, round-marker, and voted-block state. C++ receives only a compact snapshot to materialize the
  still-public own/reward vote sidecars. The post-construction storage attachment API and the legacy constructor's three
  C++ `DbStorage` scans are deleted. The storage-free Rust-mode C++ constructor and its empty-index CXX factory are also
  deleted, so every C++-reachable Rust verified-vote runtime is restored from authoritative storage. The empty helper is
  private to Rust unit tests. Because the standalone VoteManager overlay implements the complete public surface, its
  feature configuration now excludes the legacy `vote_manager.cpp` source and no longer imports `VoteManagerOld`.
  Verified-votes mode is one complete ownership bundle requiring the Rust storage, FinalChain, ProposedBlocks, and
  SlashingManager facades; the existing SlashingManager dependency also requires GasPricer, while its TransactionManager
  dependency owns the queue internally. Unsupported
  partial flag combinations fail during configuration rather than gaining legacy adapters. The later CRW-03 ownership
  commit moved that restored runtime into `BridgePbftService` and deleted the standalone handle/factory. Pure-C++ and Rust
  configurations without verified votes retain the untouched upstream implementation.
  Validation passed with the 14 focused Rust verified-votes tests, the two storage-backed verified-votes shim tests,
  four isolated VoteManager consumer tests, both focused PBFT manager consumer tests, all nine Rust storage bridge
  tests, both rewrite boundary guards, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and
  `make rewrite-validate-smoke`. Configuration coverage also proved that an incomplete verified-votes feature set fails
  at configure time, the complete ownership bundle compiles with pillar support disabled, and a separate all-Rust-off
  build compiles the untouched upstream `vote_manager.cpp`. Archive/build-metadata audits found no `VoteManagerOld`
  symbol and no Rust-mode compile of the legacy VoteManager source; the original VoteManager header/source remain clean
  versus `upstream-main`. The task owner explicitly authorized Tier 3 tests whenever agent judgment warrants them,
  including `scripts/storage_conformance_diff.sh` whenever the agent classifies that storage differential as required or
  warranted. This standing authorization satisfies the repository's coordination requirement without another prompt. The
  full CTest gate passed 22 of 28 test binaries; `pillar_chain_test`, `full_node_test`, `network_test`,
  `pbft_manager_test`, and `vote_test` reproduced the known same-process fixture-lifetime defect in which the first case
  passes and later cases cannot reacquire `/tmp/taraxa*/db/db/LOCK`. The affected verified-vote, PBFT-manager, and
  next-vote network cases pass when run in isolated processes. The remaining `go_test` failure is an unrelated static
  Go/cgo linker incompatibility in the configured environment. The Python integration gate was attempted but could not
  collect tests: its pinned `cytoolz` and `pyethash` dependencies cannot build under Python 3.13 because the container
  lacks Python development headers, leaving `pytest` unavailable. These Tier 3 environment and fixture failures do not
  contradict the passing focused, rewrite, and smoke gates above.
- `transaction_manager_shim` is now a standalone full overlay: Rust-enabled builds no longer inherit, construct, or
  compile `TransactionManagerOld`. The facade preserves `enable_shared_from_this<TransactionManager>`, the complete
  public API, event identity, locks, FinalChain/EVM executor, thread pool, and logging shell, while the Rust runtime
  remains the only queue, sidecar, gas-cache, transaction-count, and persistence owner. The upstream header/source are
  restored clean, and Rust now restores `TrxCount` directly from its storage handle during fallible runtime construction
  instead of accepting a C++ `DbStorage` bootstrap fact.
  Validation passed with the 26 focused Rust transaction-manager tests, all 36 transaction-manager shim tests, the six
  transaction-queue shim tests, all 17 transaction tests, all three gas-pricer tests, all nine Rust storage bridge tests,
  a focused PBFT proposal/overlay run, both rewrite boundary guards, `make rewrite-validate-fast`, and
  `make rewrite-validate-consensus`. Two additional PBFT DAG-creation cases encountered the known `/tmp/taraxa0`
  RocksDB fixture self-lock after the proposal case completed; they did not report a transaction-manager assertion or
  behavior failure.
- `transaction_queue_shim` is retired. The standalone `BridgeTransactionQueue` handle, its bridge module and CXX
  methods, the overlay directory/test, and `RUSTAXA_ENABLE_TRANSACTION_QUEUE` wiring are deleted. Rust-enabled
  `core_libs` also excludes the untouched legacy `transaction_queue.cpp`, so removing the overlay cannot silently
  restore C++ queue behavior. `BridgeTransactionManagerRuntime` is the sole production owner of the native
  `rustaxa-consensus::TransactionQueue`; queue-shaped FFI records remain only where the manager runtime exchanges facts
  with its C++ executor shell. Direct legacy queue tests remain enabled only in pure-C++ reference builds, while native
  Rust and manager-runtime tests cover ordering, replacement/demotion, limits, drop observation, expiry, purge, gas
  thresholds, payload views, and known-cache behavior.
  Validation passed with 14 native Rust queue tests, 28 bridge TransactionManager runtime tests, all 36
  `transaction_manager_shim_test` cases, 13 Rust-mode `transaction_test` cases, two Rust-mode `gas_pricer_test` cases,
  the gas-pricer shim test, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and
  `make rewrite-validate-smoke`. Build metadata and archive audits found no legacy queue source or symbols in Rust mode.
  The all-Rust-off build compiled the untouched `transaction_queue.cpp`, but an unrelated pre-existing missing
  PillarChainManager API in `get_pillar_votes_bundle_packet_handler.cpp` blocked linking the pure-C++ transaction tests.
- `rewards_stats_shim` is a standalone compatibility overlay and no longer needs a separate feature boundary or legacy
  `StatsOld` scaffold. `RUSTAXA_ENABLE_REWARDS_STATS` remains retired. Production FinalChain no longer constructs the
  facade: Rust plans external-EVM rewards from the validated execution report, binds the plan to the session/head/runtime
  generation, publishes its cache mutation atomically, audits matching durable publications, and installs only
  head-stable monotonic runtime snapshots after live or recovered publication. Planning fails closed if durable and
  runtime heads differ. C++ decodes only Rust-supplied distribution RLP for `StateAPI::distribute_rewards`; no rewards
  storage-update DTO or commit/clear acknowledgement crosses that boundary. The facade and
  `BridgeRewardsStatsRuntime` remain only for stable public compatibility tests.
  Validation passed 12 focused Rust consensus rewards tests, seven Rust bridge rewards tests, two C++/Rust rewards
  bridge parity tests, all nine storage bridge tests, all seven `rewards_stats_test` cases, all 17 `final_chain_test`
  cases, all 50 RPC cases, `make rewrite-validate-final-chain`, `make rewrite-validate-consensus`, and startup smoke.
  A refreshed Rust cache contains no retired option, legacy source, rename definition, or `StatsOld` archive symbol.
  Fresh FinalChain-off and all-Rust-off configurations select and compile the untouched original source without a
  rename; their broader test targets are blocked by pre-existing partial/pure-C++ network packet-handler API mismatches.
  Slice mapping, API design, architecture review, C++ implementation, and independent closeout review used the
  code-mapper, api-designer, architect-reviewer, cpp-pro, and reviewer agents. No Rust implementation agent was needed
  because no Rust runtime or bridge behavior changed.
- `dag_manager_shim::setNetwork` no longer forwards to `DagManagerOld`; the shim now only stores the local shim-owned
  network pointer at this seam.
- `dag_manager_shim` now owns the public `VerifyBlockReturnType` enum locally instead of aliasing
  `DagManagerOld::VerifyBlockReturnType`. The public `DagManager::VerifyBlockReturnType` spelling and numeric values are
  preserved for tarcap/tests while reducing one remaining legacy type dependency.
- `dag_manager_shim` is fully detached from the dead `DagManagerOld` compile scaffold. The overlay directly includes
  the standalone facade, feature-on builds exclude the original manager source instead of compiling it under a renamed
  symbol, and the Old-only inheritance assertion is deleted. The facade keeps the stable public API and owns
  `std::enable_shared_from_this` identity directly; pure-C++ builds select the untouched original header and source.
  Mapping, API design, and architecture review used `code-mapper`, `api-designer`, and `architect-reviewer`; no Rust
  implementation agent was needed because runtime and bridge behavior did not change.
  Validation passed the 11 native Rust consensus and 23 Rust bridge DAG-manager tests, all 12 `dag_test` cases, all 13
  `dag_block_test` cases, four focused Rust CXX DAG tests, the PBFT single-node test, `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, and `make rewrite-validate-smoke`. Feature-on compile metadata and the core archive
  contain neither the original source object nor `DagManagerOld`; a fresh all-Rust-off configuration selects and
  compiles the untouched original `dag_manager.cpp`, and the original header/source are clean versus `upstream-main`.
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
- The live facade is now fully detached from `GasPricerOld`: Rust gas-pricer builds exclude the untouched legacy
  `gas_pricer.cpp`, the overlay header includes only the self-contained Rust-backed facade, and the scaffold-only
  non-inheritance test is retired in favor of existing Rust bridge/domain and public C++ behavior coverage. The
  `RUSTAXA_ENABLE_GAS_PRICER` flag remains valid and necessary because gas pricing has supported independent
  transaction/RPC consumers and SlashingManager depends on it; configurations without that flag retain the original
  implementation.
  Validation passed ten focused Rust consensus gas-pricer tests, four Rust bridge gas-pricer tests, both public
  `gas_pricer_test` cases, the SlashingManager shim build/test, `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, and `make rewrite-validate-smoke`. Rust build metadata/archive audits contain no
  legacy gas-pricer source or `GasPricerOld` symbol; a module-disabled build compiled the untouched original source
  without a rename, and invalid module configurations without master Rust mode or Rust storage fail during configure.
  The original upstream GasPricer header/source remain unchanged. Slice mapping, API design, architecture review, C++
  implementation, and independent closeout review used the code-mapper, api-designer, architect-reviewer, cpp-pro, and
  reviewer agents. No Rust implementation agent was needed because the Rust runtime and bridge API did not change.
- The standalone sortition facade is now fully detached from the legacy implementation. Master Rust mode excludes the
  untouched original source, the overlay owns the exact shared `SortitionParamsChange` carrier and canonical RLP codec,
  and the redundant module flag, compile rename, header import, and scaffold-only inheritance test are retired. The live
  facade, historical query surface, and ignored `Batch&` compatibility signature remain for stable public C++ callers;
  the production PBFT preview/stage/commit path now stays inside the two application-owned Rust services. Focused Rust
  sortition tests,
  the Rust consensus bridge sortition and PBFT-finalization tests, the C++ shim/public sortition suites, Rust storage
  bridge tests, Tier 1, Tier 2, and startup smoke validation passed. Rust-enabled build metadata and archives contain
  only the shim source/codec symbols and no legacy manager symbols; pure-C++ configuration selects the untouched original
  source, although its target build remains blocked by the pre-existing pillar packet-handler API mismatch. The
  code-mapper, api-designer, architect-reviewer, cpp-pro, and independent reviewer agents covered mapping, design,
  implementation, and closeout. No Rust implementation or blockchain/EVM agent was needed because this slice changed
  only C++ overlay/build ownership and preserved the Rust runtime and bridge API.
- The standalone slashing facade is now fully detached from `SlashingManagerOld`: feature-enabled builds exclude the
  untouched original source, the overlay wrapper includes only its self-contained facade, and the compile rename plus
  assertion-only scaffold test are retired. The module flag remains because master Rust configurations may validly use
  the original manager when Rust VerifiedVotes and its FinalChain/GasPricer dependency bundle are disabled. The live
  facade, `BridgeSlashingProofPlanner`, normalized-evidence overload, and live-`PbftVote` compatibility overload remain
  for the accepted FinalChain account-read, gas-bid, signing, transaction construction, insertion, and network/test
  executor edges. A separate blockchain-facing parity slice must supply Magnolia activation to the Rust planner; the
  current Rust path otherwise lacks the legacy pre-Magnolia proof-submission gate, and dead `*Old` compilation does not
  correct that production behavior. Validation passed 19 focused Rust consensus slashing tests, five Rust bridge
  slashing tests, the `taraxad` and `state_api_test` builds, both rewrite boundary guards, Tier 1, Tier 2 consensus,
  the FinalChain gate, and startup smoke. The focused `StateAPITest.slashing` accepted the proof transaction but twice
  timed out waiting for the validator to become jailed; because source/symbol audits prove the detached legacy object
  was never called and no runtime behavior changed, this is recorded as an existing full-node FinalChain/slashing
  validation gap rather than a reason to retain dead scaffold. Feature-on metadata contains only the shim source and no
  `SlashingManagerOld` symbols, module-disabled/all-off metadata selects the untouched original source without a rename,
  invalid dependency configurations still fail, and the upstream original files remain unchanged. Mapping, API design,
  architecture review, C++ implementation, and independent closeout review use the code-mapper, api-designer,
  architect-reviewer, cpp-pro, and reviewer agents. No Rust implementation or blockchain agent was needed for this
  detachment because the Rust planner and contract executor behavior did not
  change; the Magnolia parity follow-up is explicitly blockchain-facing.
- The Magnolia slashing parity follow-up closes both defects exposed by that detachment validation. The shim now supplies
  the immutable Magnolia activation period when it constructs the Rust planner; Rust checks the legacy vote-A boundary
  after the reporting flag and before slot equality, accepts equality, returns appended stable status code `5` for
  pre-activation evidence, and leaves duplicate state unchanged on rejection. Separately, Rust FinalChain now interprets
  Magnolia and Cacti activation block zero as active from genesis by using the legacy inclusive block comparison. Local
  evidence-period policy remains distinct from transaction-inclusion activation, so old evidence submitted after
  activation remains contract-valid. Focused Rust boundary/execution tests and `StateAPITest.slashing` cover construction
  wiring, pre-activation rejection, equality acceptance, inclusion, jailing, and jailed-vote eligibility. Tier 1, the
  FinalChain Tier 2 gate, startup smoke, and the consensus Tier 2 command passed; the latter still exposed pre-existing
  suite-order database-lock failures, while isolated PBFT and pillar-count reruns passed and the unrelated pillar-sync
  rerun reproduced its existing PBFT runtime panic. Blockchain, API, architecture, Rust, C++, and independent reviewer
  agents covered the policy split, compatibility surface, implementation, and closeout.
- Custom agents used:
  - `rust-engineer`: confirmed Slice 5 bridge handles are still required by C++ public facade surfaces and recommended
    gas-pricer narrowing instead of handle deletion.
  - `cpp-pro`: mapped small-shim CMake/removal candidates; its full `gas_pricer_shim` deletion recommendation was
    rejected because it would re-center Rust-mode pricing in legacy C++.
  - `architect-reviewer`: recommended retiring `period_data_queue_shim` by moving queue ownership into the PBFT manager
    runtime, with sidecar lockstep and PBFT sync drain behavior as the primary risks.
  - `reviewer`: reviewed the final period-data queue consolidation for stale references, sidecar risks, and validation
    coverage before closeout.
  - `api-designer` and `architect-reviewer`: approved retiring the standalone transaction-queue facade while preserving
    native queue ownership inside TransactionManager and pure-C++-only legacy reference coverage.
  - `rust-engineer` and `cpp-pro`: removed the bridge/shim and build/test wiring halves and added replacement runtime
    coverage; the independent reviewer found no remaining issues after a dead private field and stale dependency wording
    were corrected.
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
  now routes current-anchor decisions, threshold arithmetic, live vote state, and PBFT-facing pillar finalization through
  the App-owned `BridgePbftService` pillar state behind its independent readiness gate and sibling mutex, but still owns
  external FinalChain DPoS reads, temporary `PillarBlock` materialization,
  PBFT `PeriodData` vote materialization, the current-block compatibility mirror, network vote-bundle requests, and event
  emission. Latest-finalized identity and validator vote-count history are service-owned in Rust and no longer mirrored
  as C++ decision state; the standalone `BridgePillarChainRuntime` handle and factory are deleted.
  `vote_manager_shim` is no longer a legacy-derived runtime: it is a standalone public facade whose authoritative
  verified-vote restart state is restored inside Rust. Its locally generated own-vote collection is now also
  storage-authoritative: the facade materializes votes only for the public getter/network boundary instead of maintaining
  a second C++ object vector. Remaining vote-manager work is external FinalChain/key/slashing fact execution and eventual
  absorption of the public facade into the Rust PBFT runtime, not legacy base ownership.
  `transaction_manager_shim` is also no longer a legacy-derived runtime. Rust mode excludes the legacy implementation
  source entirely, and the standalone facade owns only public/executor shell state. Its Rust runtime restores the durable
  transaction count natively and remains authoritative for the queue, live payload sidecars, gas cache, and transaction
  persistence. Remaining transaction-manager work is the classified FinalChain/EVM, public materialization, event,
  logging, and lifecycle shell rather than legacy base ownership.

### CRW-01 PBFT application-service boundary decision

The first remaining consolidation item selected a PBFT-cluster-only application service for the next implementation
slice. `App` currently constructs `BridgePbftChain` and `BridgePbftManagerRuntime` independently from the same storage,
derives manager startup period through the C++ chain facade, and passes both owners into separate compatibility shells.
The manager then makes repeated C++ calls back through `PbftChain`; finalization even dispatches a Rust-planned chain
mutation through C++ and reports the Rust chain result back to the Rust manager cursor. This is the smallest active
composition seam with a direct ownership and consistency payoff.

`CRW-02` will introduce one Rust PBFT application service that owns manager runtime/session state, chain state/lookups,
and a shared native storage handle. Rust restores the chain first and derives the manager startup period and hardfork
activation internally. `App` owns one safe shared C++ holder for the service and supplies it to the retained
`PbftManager` and `PbftChain` facades; neither facade owns or restores nested Rust state. Manager and chain synchronization
remain separate lock domains with a documented order rather than a service-wide mutex. Startup replay remains a typed
bootstrap phase until its classified C++ executors move, and live commands reject use before bootstrap completion.

The retained `PbftManager` facade owns only app-host lifecycle, threads, sleeps/timers, external effect execution, and
compatibility materialization. The retained `PbftChain` facade is primarily a narrow read/JSON/block-materialization view
for network, DAG, vote, RPC, stats, and tests. Its public `updatePbftChain(...)` method remains a compatibility/test
mutation adapter that applies against service-owned state until direct callers migrate; the separate finalization
mutation/report bounce moves fully inside the service. Existing manager and chain DTOs should first change receiver
rather than being replaced wholesale, with operation generations preserving stale-report and retry behavior.

The first deletions are the two production storage-backed construction routes, independent exported manager/chain handle
ownership, app-side current-period activation derivation, facade fields that own those handles, and the chain
finalization mutation/report bounce. The public `PbftChain(addr_t, std::shared_ptr<DbStorage>)` and
`PbftChain::updatePbftChain(...)` signatures remain compatibility/test adapters until their direct callers intentionally
migrate; `CRW-02` replaces `App`'s production construction path and the finalization-specific mutation route without
breaking those APIs. Proposed blocks and verified votes are the next PBFT-private states for `CRW-03`: the former
currently has separate storage-shim and manager instances, while the latter is independently owned by the vote facade.
DAG, transaction, pillar, FinalChain, gas, and slashing remain sibling services or typed executor ports; their live
non-PBFT consumers give no present justification for a wider root.

This is a boundary decision, not a change to the accepted scope in `PLAN.md`. Network/tarcap, external EVM/StateAPI,
public query, signing/VDF, and app lifecycle boundaries remain unchanged. The implementation must cover finalization and
duplicate-resume ordering, atomic durable/live publication, storage failure and crash recovery, bootstrap rejection,
concurrent compatibility reads, and shared-service teardown before deleting the standalone routes.

### CRW-02 PBFT application-service implementation result

The PBFT-only service is implemented as the sole exported `BridgePbftService` owner for PBFT manager runtime/session
state, PBFT chain state/lookups, and their native storage lifetime. `App` creates one production service after FinalChain
and transaction setup, then shares one shim-owned RAII holder with `PbftChain` and `PbftManager`. Rust restores the chain
first and derives manager period/Cacti activation from the durable head; C++ no longer constructs two independent bridge
handles or injects those derived facts.

Manager and chain state use separate Rust locks with manager-before-chain ordering for combined operations. The service
starts behind a readiness gate, and `PbftManager` publishes bootstrap completion only after constructor replay and restart
processing. The daemon, proposal, and PBFT sync entry points reject premature use. The public chain constructor and
`updatePbftChain(...)` remain compatibility/test adapters over the service, while production app bootstrap uses the
shared service-aware constructor.

The finalization `UpdatePbftChain` action is now manager-owned service work. Rust applies the chain mutation from the
accepted write intent, validates the resulting size/hash/anchor facts, advances the existing finalization cursor, and
returns the next external effect. The C++ action case, payload sidecar,
`PbftChain::updatePbftChainForPbftFinalization`, `PbftChainFinalizationUpdateReport`, and
`pbft_manager_runtime_advance_finalization_pbft_chain` are deleted. The independent `BridgePbftManagerRuntime` and
`BridgePbftChain` CXX types, production factories, and facade-owned boxes are also gone.

Cross-cutting `CRW-07` cleanup removed the obsolete CXX `PbftManagerStartupFact`. Rust tests use a private startup fixture;
C++ consensus/storage fixtures seed the persisted PBFT head and call the production service constructor. The app changes
remain guarded by `RUSTAXA_ENABLE`; their non-empty `git diff upstream-main` is an accepted app-bootstrap integration
exception, while all original PBFT manager/chain implementation files remain untouched and Rust routing stays in the
full shim overlays.

Tier 1 and Tier 2 validation are complete: `make rewrite-validate-consensus`, `make rewrite-validate-smoke`, the 9-test
`rust_storage_tests` binary, `rewrite_bridge_inventory_guard.sh`, `rewrite_storage_boundary_guard.sh`, and
`git diff --check` all pass. The upstream-file audit is empty for the original PBFT manager/chain headers and sources; its
only output is the guarded App bootstrap exception documented above. Existing CMake developer warnings and expected DAG
restore diagnostics remain non-fatal.

Tier 3 was authorized and executed because this shared service changes production startup, sync, finalization, and
consensus-runtime routing. The Python 3.10 integration run passed its five-node JSON-RPC scenario (`1 passed` in 149.62
seconds). Full CTest built every registered target and passed 22 of 28 suites. Five C++ binaries exposed a pre-existing
test lifecycle defect: default-enabled RPC owns an API that indirectly retains `App`, while `App::close()` does not invoke
plugin shutdown, so fixed `/tmp/taraxa*` RocksDB paths remain locked between gtests in the same process. The new service
adds no reverse ownership edge and replaces two pre-existing cloned Rust storage owners with one shared owner. Clean,
non-overlapping `PillarChainTest.votes_count_changes` runs passed both on this diff (`1 passed` in 24.495 seconds) and in a
separate detached build of committed HEAD `03016f9` (`1 passed` in 20.309 seconds), confirming that the overlapping-run
panic and broad lock cascade are not CRW-02 regressions.

The sixth CTest failure, `go_test`, is an environment/script issue: its generated script applies static RocksDB CGO flags
to every package while static zlib/snappy archives are absent from the default linker path. Supplying temporary Conan-built
Snappy and existing Conan zlib archives allowed all state/contract packages to pass; the two remaining pure-Go packages
failed only from the unconditional static-CGO libc link, then passed separately with `CGO_ENABLED=0` (`150 passed` across
the two packages). These classified pre-existing harness/environment failures do not invalidate the passing CRW-02
Rust/C++ parity, subsystem, startup, and integration evidence. Independent review approved the completed slice.

### CRW-03 PBFT-private state absorption design

`CRW-03` is one ownership objective delivered in two dependency-ordered commits: proposed blocks first, then verified
votes. The proposed-block sub-slice moves the durable/live index behind a sibling `RwLock` in `BridgePbftService`,
restores it during service construction, migrates the PBFT and vote-manager production callers, replaces the storage
shim's independently owned handle with storage-only compatibility functions, and deletes `BridgeProposedBlocks` plus
its factory. A lifetime-composition-only change is insufficient because it would retain duplicate authority and
cross-shim state passing.

Leader selection must also stop passing a `ProposedBlocks&` between C++ facades. The temporary wallet candidate
collection used while proposing remains Rust-local and non-persisted; only the selected leader may be published to the
authoritative index. Stable C++ materialization signatures such as `PbftManager::getProposedBlocks()` may remain as
views over the shared service while direct compatibility callers still need them, but neither those facades nor
`DbStorage` may own an independent live index.

Verified votes follow after this crossing is removed. That sub-slice moves the restored admission runtime into the
same service, converts retained `VerifiedVotes` and `VoteManager` facades into service clients, moves combined
vote/proposal operations behind narrow Rust APIs, and deletes `BridgeVerifiedVotes`. Rust synchronization remains
split into manager, verified-vote, proposed-block, and chain lock domains. Operations avoid nested locks by using
snapshot, unlock, external validation, then relock/revalidate; no Rust guard crosses a C++ validation, FinalChain/EVM,
network, logging, or gossip callback. Storage-family locks follow their owning runtime lock, and construction restores
all private state before the one-way bootstrap publication.

#### CRW-03 proposed-block absorption result

The first sub-slice is implemented. `BridgePbftService` now restores and owns the proposed-block index behind its own
`RwLock`; storage-backed construction rejects malformed or key/hash-mismatched persisted proposals before publishing
the service. Push and cleanup hold that owning lock while Rust commits storage first and publishes memory second, so
concurrent operations cannot split durable and live state. Pivot identity is decoded and checked before a write.

The retained C++ `ProposedBlocks` facade holds only `SharedPbftService` and performs no synchronization. PBFT startup no
longer invokes a second restore, `VoteManager` no longer accepts a `ProposedBlocks&`, and `DbStorage` no longer owns a
live proposed-block handle. Its save/snapshot compatibility methods call stateless Rust storage functions. Local wallet
proposals are converted once into an ordered Rust-local lookup batch; the carrier cannot assert a trusted validation
flag, the temporary index is dropped after lookup, and only the selected leader is later persisted to service state.

Cross-cutting `CRW-07` cleanup deleted `BridgeProposedBlocks`, its factory, explicit restore/non-persisted push exports,
the storage-shim owner, and the C++ facade mutex. The stable facade remains only for current C++ `PbftBlock`
materialization. No original upstream-owned C++ implementation path changed; all routing edits are in full overlays,
storage compatibility shims, Rust crates, and tests.

Validation passed for the moved boundary: seven focused bridge tests, eight native consensus proposed-block tests, all
276 bridge-library tests, `rust_consensus_tests` 62/62, `proposed_blocks_shim_test` 6/6, all nine
`rust_storage_tests`, isolated `PbftManagerWithDagCreation.proposed_blocks`, isolated
`PbftManagerTest.propose_block_and_vote_broadcast`, both bridge/storage guards, formatting, clippy, the Rust workspace
tests, and `git diff --check`. `make rewrite-validate-consensus` completed with status zero. Its non-fail-fast shell loop
also exposed the previously classified same-process node-test lifecycle panic/RocksDB fixture-lock behavior; the two
touched PBFT paths pass in fresh isolated processes, so this is retained as harness debt rather than hidden as a clean
broad-suite signal.

#### CRW-03 verified-vote absorption result

The second ownership sub-slice is implemented. `BridgePbftService` now restores and owns
`PbftVoteAdmissionRuntime` behind a sibling mutex before the production service is published. The explicit chain-only
compatibility service carries no vote runtime; any accidental vote receiver call returns
`PBFT_SERVICE_VERIFIED_VOTES_UNAVAILABLE` instead of panicking or constructing a second authority.

The retained `VerifiedVotes` and `VoteManager` C++ facades share the application-owned service. Their standalone
`BridgeVerifiedVotes` box, storage-backed factory, direct `DbStorage` construction, and C++ `shared_mutex` are deleted.
Full compatibility-map materialization, step-bucket materialization, and current reward-vote materialization each use
one owned Rust snapshot taken under the service vote mutex; C++ decodes those owned canonical records only after the
receiver returns. Network callers continue through `Network -> VoteManager`, and no Rust guard crosses validation,
FinalChain/EVM, network, logging, or gossip work.

Production construction now restores chain, proposed blocks, verified votes, and manager state before returning the
single service. Storage bridge tests construct that production service rather than a separate vote handle, so malformed
or ambiguous persisted vote state remains a startup error. `BridgeVerifiedVotes` and
`create_verified_votes_index_from_storage` no longer exist in the CXX inventory.

Focused validation passes all 279 `rustaxa-bridge` tests, 17 verified-vote module tests,
`verified_votes_shim_test` 3/3, `rust_storage_tests` 9/9, and isolated VoteManager admission/snapshot, round-advance,
own-vote restart, PBFT proposal/vote broadcast, and null-anchor finalization paths. The full `vote_test` binary still
has the previously classified same-process `/tmp/taraxa*` database-fixture lifetime failure after its first node test;
the affected paths pass in fresh processes. Strict all-target clippy remains blocked by pre-existing dependency and
bridge findings; `make rewrite-validate-fast`, the final `make rewrite-validate-consensus` run, and
`make rewrite-validate-smoke` pass. An earlier Tier 2 attempt hit the unrelated nanosecond-order assertion in the VDF
precision-cache timing test; that case passed immediately in isolation and the complete rerun exited successfully.

The next CRW-03 sub-slice closes authoritative leader snapshot/revalidation. `BridgePbftService` now prepares one owned,
vote-hash-ordered snapshot containing proposal-vote payloads, aligned proposed-block payloads and validation flags, plus
PBFT-chain membership while holding `votes -> proposed -> chain`. It fingerprints those facts, releases every Rust guard
for the existing C++ `validatePbftBlock` executor, then reacquires the same lock order and rejects stale or identity-
mismatched reports before mutation. Only the existing Rust leader planner can mark validated proposed blocks, and the
selected vote/block pair returns as owned canonical payloads. The filtering path no longer calls `getProposalVotes()`,
performs per-vote proposed-block lookups, or asks C++ for chain membership. The separate stateless local-wallet candidate
path is unchanged.

Focused validation passes all 284 `rustaxa-bridge` tests, including 22 verified-vote/service tests for deterministic
fingerprints, accepted/rejected reports, stale vote/proposed-block/chain facts, invalid reports, and no mutation on stale
or invalid finish. The two new C++ leader tests, isolated `VoteTest.verified_votes`, and isolated
`PbftManagerTest.propose_block_and_vote_broadcast` also pass; both affected C++ test targets build successfully. Broader
rewrite gates also complete as far as the current environment permits: `make rewrite-validate-fast`,
`make rewrite-validate-consensus`, and `make rewrite-validate-smoke` exit successfully. The direct
`rust_consensus_tests` binary passes 43/62 cases; the remaining 19 PBFT-sync fixtures fail during service construction on
the previously classified ambiguous legacy reward-cursor bootstrap, before leader selection runs. Tier 3 CTest passes
22/28 binaries (79%). The six failures retain their known classifications: same-process `/tmp/taraxa0` RocksDB lock
reuse in node-backed suites, the associated network/TLS teardown behavior, and the unrelated Go/cgo static-linker
incompatibility. Both new leader tests and the touched PBFT broadcast path pass in isolated fresh processes. The Python
Tier 3 command does not reach test collection because its Python 3.13 environment cannot build pinned `cytoolz` and
`pyethash` without `Python.h`, leaving `pytest` unavailable.

The next CRW-03 sub-slice closes admission plus vote-progress persistence. The service now holds the vote mutex while it
validates the canonical vote, checkpoints the bounded replay insertion/eviction delta, touched period/round, and incoming
payload entry, applies the admission transition, and commits any Rust-selected extra-reward and `2t+1` rows through the
existing single Rust storage batch. Applied or no-write transitions publish normally; rejected or failed persistence
restores the exact checkpoint before unlock and returns no executable peer-known, gossip, slashing, proposed-block, or
PBFT-progress effects. Replay-only rejection/duplicate/conflict paths and accepted transitions with no durable intent do
not create empty batches. The C++ admission route now makes one service call and no longer receives or echoes Rust-built
storage payloads into a subsequent persistence call. The generic progress persistence API remains for the separate
non-admission period/round compatibility restore path.

Focused validation passes all 641 `rustaxa-consensus` tests and all 284 `rustaxa-bridge` tests. Transaction-specific
coverage proves no-write publication without a persistence call, exact replay FIFO/round/payload rollback on rejected or
operational persistence failure, suppressed failure effects, and successful retry after rollback. The affected C++
targets build, `verified_votes_shim_test` passes 3/3, isolated vote admission/threshold/reward/network tests pass, and
`PbftManagerTest.propose_block_and_vote_broadcast` passes. Broader rewrite and Tier 3 gates also completed at closeout.
`make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke` all exit successfully.
The direct `rust_consensus_tests` binary passes 43/62 cases; the remaining 19 PBFT-sync fixtures fail during service
construction on the previously classified ambiguous legacy reward-cursor bootstrap, before the changed admission path
runs. The task-owner-preapproved Tier 3 CTest gate passes 22/28 binaries (79%). Its six failures retain the known
classifications: same-process `/tmp/taraxa0` RocksDB lock reuse in the five node-backed binaries and the unrelated
Go/cgo static-linker failure. The touched admission and PBFT broadcast cases pass in isolated fresh processes. The Python
Tier 3 command does not reach test collection because its Python 3.13 environment cannot build pinned `cytoolz` and
`pyethash` without `Python.h`, leaving `pytest` unavailable.

The final CRW-03 sub-slice closes manager period cleanup. The advance-period planner now emits one
`CleanupPeriodState` action instead of separate vote and proposed-block actions. Its single service call validates the
finalized-chain/new-period successor relation, acquires verified-vote then proposed-block ownership, plans both removals
without mutation, commits all proposed-block deletes in one Rust batch, and then directly removes stale verified-vote
periods, retained payload sidecars, and proposed-block periods. Commit rejection leaves both memory owners and storage
unchanged and permits retry; empty cleanup publishes a typed storage-free no-op. The old action code 8 and the
manager-only VoteManager cleanup wrapper are deleted. CRW-03 is complete; the individual vote/proposal cleanup APIs are
retained only for compatibility tests and non-manager callers. The live C++ executor also checks the finalized-chain
successor before starting the reset transition, so maximum-period overflow cannot partially reset manager or sidecar
state before Rust rejects the later cleanup plan.

Focused validation passes all 642 `rustaxa-consensus` tests and all 286 `rustaxa-bridge` tests, including injected
storage-commit rejection with exact no-mutation retry, durable proposed-row deletion, stale vote-payload pruning, typed
no-op/invalid/chain-only results, the one-action planner contract, and rejection of retired action code 8. The affected
C++ targets build; `verified_votes_shim_test` passes 3/3, `proposed_blocks_shim_test` passes 6/6, and isolated vote,
proposal, and PBFT broadcast cases pass. `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and
`make rewrite-validate-smoke` exit successfully. The direct `rust_consensus_tests` result remains 43/62 because the same
19 PBFT-sync fixtures fail during unrelated ambiguous legacy reward-cursor bootstrap. The task-owner-preapproved Tier 3
CTest result remains 22/28 (79%): five node-backed binaries retain the same-process `/tmp/taraxa0` lock failures and the
unrelated Go/cgo binary retains its static-link failure. The Python Tier 3 command remains blocked before collection
because Python 3.13 cannot build pinned `cytoolz` and `pyethash` without `Python.h`, leaving `pytest` unavailable.

Implementation notes:

- VoteManager no longer mirrors locally generated own votes in `own_verified_votes_`. The service-owned vote runtime enumerates
  validated canonical own-vote records directly from native Rust storage in hash order; the public C++ getter creates
  transient `PbftVote` objects only when compatibility/network callers require them. Save and zero-input clear-all are
  Rust-owned persistence operations serialized across production handles by a shared `Storage` mutex; startup returns
  only extra-reward hashes plus reward coordinates, and PBFT lifecycle transitions no longer return a
  `clear_own_vote_sidecars` command or call back into VoteManager after their atomic clear.
  Focused coverage includes save/read/order/restart/clear and malformed key/payload/hash rejection, plus lifecycle clear,
  `rust_storage_tests`, the own-vote `vote_test`, and the PBFT/consensus bridge build targets. `make
  rewrite-validate-fast` passes. `make rewrite-validate-consensus` completed its Rust, guard, build, and most C++ stages;
  the known full-binary sequential fixture issue reappeared in `pbft_manager_test`/`vote_test` as reused `/tmp/taraxa*`
  RocksDB lock failures. The affected own-vote test and PBFT single-/multi-node smoke tests pass when run independently.
- VoteManager also no longer mirrors persisted extra reward-vote hashes in `extra_reward_votes_`. Rust validates the
  certified reward-vote mapping and builds the canonical reset bundle during side-effect-free stage preparation; the
  finalization storage apply then acquires a dedicated shared-storage lock, enumerates the authoritative extra-reward
  keys, and commits the bundle replacement plus all deletes in the existing atomic finalization batch. Production
  extra-reward admission/removal uses the same lock. Startup no longer exports extra hashes, C++ no longer materializes
  cert votes or supplies delete keys, and the locked apply mints an opaque storage reset generation only after a
  successful/idempotent commit. The finalization executor carries that Rust-authenticated generation to the reward-reset
  boundary, and the PBFT manager validates it against the shared storage instance instead of rereading a row count after
  releasing the lock or accepting a C++-derived remaining count. Later-cycle reward admission therefore cannot
  retroactively invalidate the completed reset. Reward period/round/block-hash compatibility cursors remain for the
  subsequent vote-runtime consolidation slice. Focused storage, vote-runtime, finalization, bridge-manager, C++ storage,
  and finalization-boundary tests pass, as does `make rewrite-validate-fast`; the Tier 2 command was also run with the
  same known sequential `/tmp/taraxa*` RocksDB fixture-lock limitation documented above for the full PBFT/vote binaries.
- The remaining reward-vote cursor is now owned by `PbftVoteAdmissionRuntime` as
  `RewardVoteCursor { period, round, step, block_hash }`. The atomic reward reset writes a dedicated
  `finalized_reward_vote_cursor` row containing both those coordinates and the canonical certified bundle; Rust restores
  from that immutable finalized record rather than the mutable latest-cert slot. A restart regression advances the
  generic cert slot to the next unfinalized period and proves that the finalized cursor, payloads, and reward selection
  remain unchanged. Existing databases bootstrap the row once only when the canonical legacy cert bundle matches the
  validated persisted PBFT head, finalized period index, and canonical period-data block; missing or newer ambiguous
  legacy state fails closed. Rust derives stale-reward eligibility internally (including the saturating `round + 100`
  bound) and uses the cursor for reward selection, current payloads, and the public period query. The now-empty startup
  snapshot/export, caller-supplied
  `valid_stale_reward_vote` flag, C++ period/round/block-hash fields, and reward cursor mutex are deleted. After the
  durable reward reset commits, a typed Rust cursor commit validates the authenticated reset generation, runtime mapping,
  retained payloads, and byte-equal dedicated durable bundle before an idempotent/monotonic live update; C++ relays only
  Rust-derived cursor facts into PBFT manager advancement. Restart reconstructs the cursor from the finalized record,
  while public/network
  compatibility methods materialize canonical votes only at their existing boundaries.
  Focused validation covers the 18 vote-runtime cursor/restart cases, 42 finalization cases, 22 storage PBFT cases,
  bridge verified-vote and manager suites, `rust_storage_tests`, the C++ reward fallback, and finalization-boundary tests.
  `make rewrite-validate-fast` passes. `make rewrite-validate-consensus` completed with its known full-binary sequential
  fixture limitation: later tests in the PBFT manager and pillar binaries encounter reused `/tmp/taraxa*` RocksDB locks,
  while the focused reward, storage, and finalization tests pass independently.
  Custom-agent review and implementation used `rust-engineer`, `cpp-pro`, and `architect-reviewer`; the final architecture
  review approved the dedicated durable record, fail-closed legacy bootstrap, and live generation boundary.

- `final_chain_shim` now no longer exposes `rustFinalChainForRust()`; callers must route through explicit
  consensus/runtime APIs, which keeps FinalChain session ownership constrained to the shim constructor and execution
  boundary.
- `final_chain_shim` is fully detached from `FinalChainOld`. Its overlay header now includes only the self-contained
  standalone facade, and Rust FinalChain builds exclude the untouched legacy `final_chain.cpp` instead of compiling it
  under renamed symbols. The public C++ API and the classified `ExternalEvmStateApiClient`/`StateAPI` executor boundary
  are unchanged; pure-C++ reference builds continue using the original header and source. Validation passed 25 focused
  Rust FinalChain bridge tests, all 17 `final_chain_test` cases, all 50 `rpc_test` cases,
  `make rewrite-validate-final-chain`, `make rewrite-validate-consensus`, startup smoke, archive/build-metadata audits,
  whitespace validation, and the upstream-owned-file diff check. The pure-C++ tree configured successfully, selected
  and compiled the original `final_chain.cpp` without renamed symbols, then the broader `final_chain_test` target was
  blocked by the pre-existing unrelated pillar-vote packet-handler API mismatch. Slice selection, mapping, API and
  architecture review, C++ implementation, and independent closeout review used the code-mapper, api-designer,
  architect-reviewer, cpp-pro, and reviewer agents. No Rust implementation agent was needed because the
  Rust bridge and runtime behavior did not change.
- `dag_manager_shim` now moved `getShared()` and `getDagMutex()` off inherited `DagManagerOld` access and onto shim-owned
  state. `setDagBlockOrder()` no longer acquires an extra outer order lock before Rust-runtime lock flow, since runtime
  callers now perform the lock sequencing directly.
- Follow-up DAG manager cleanup removed the `DagManagerOld::VerifyBlockReturnType` alias and then removed
  `DagManagerOld` inheritance/construction from the shim API. The later detachment slice also removed the legacy-header
  import, renamed-source compile, and Old-only test. Remaining DAG manager compatibility debt is C++ graph
  materialization and the broader public facade itself, not legacy DAG manager base or compile identity.
- Follow-up DAG facade cleanup first removed the unused Boost graph alias re-exports, direct Boost includes, and
  protected Boost-vertex helper stubs from `dag_shim`, then detached the facade from the legacy implementation entirely.
  At that stage the Rust-mode DAG facade was self-contained and hash-only, and feature-on builds neither imported the
  legacy header nor compiled the original source under `DagOld`/`PivotTreeOld` names. The facade and its bridge handle
  were later retired because production graph state already lived inside `BridgeDagManagerRuntime`.
- `pbft_manager_shim` still routes through shim-owned lifecycle/finalization orchestration in multiple places.
  The `transaction_manager_shim` packing path now uses `pack_prepare_sharded` + `pack_finalize_with_estimates` and is already
  reduced to thin conversion plus one Rust service round-trip plus deterministic materialization.
- `pbft_manager_shim` is fully detached from the dead `PbftManagerOld` compile scaffold. The wrapper directly includes
  the standalone facade, the stable public PBFT phase and FinalChain-validation enums are shim-owned with preserved
  ordinals, and feature-on builds exclude the untouched original source rather than compiling renamed symbols. The
  dangerous rename macro is gone, eliminating the former type-substitution include-order hazard. The facade retains its
  broad network/FinalChain includes for public transitive compatibility after a full build proved upstream network vote
  code still consumes that include chain. The empty header-only shim translation unit and Old-identity test are deleted.
  Module-disabled and pure-C++ builds retain the original manager.
- `pillar_chain_manager_shim` now constructs `BridgePillarChainRuntime`, which owns live pillar-vote aggregation state
  and the native pillar storage handle used by PBFT-facing finalization. The previous live manager field
  `BridgePillarVotes` is gone; the standalone `BridgePillarVotes` CXX handle is also retired after the remaining C++
  bridge test moved to the runtime. The Rust helper remains only as a bridge-module unit-test fixture.
- Pillar-chain manager startup now uses that same `BridgePillarChainRuntime` for one Rust-owned recovery snapshot instead
  of constructing a parallel `BridgePillarChainStorage`. Rust loads the own vote, current-block data, and latest block,
  decodes the latest block to derive its following period-data lookup, and propagates malformed or overflowing latest
  blocks as startup errors. C++ retains only temporary pillar object materialization. The storage-only handle remains for
  the separate `DbStorage` compatibility shim and is no longer part of pillar-manager bootstrap.
- `BridgePillarChainRuntime` now owns a lock-protected canonical current-anchor snapshot and process-local generation.
  Its fallible factory decodes and canonically validates persisted `CurrentPillarBlockDataDb`; current-data apply decodes
  before locking, persists while holding the snapshot write lock, and publishes only after success. Startup bootstrap
  returns the exact canonical bytes held by that snapshot. Rust's operation-tagged current-anchor planner now owns PBFT
  candidate hash validation, proposal/local-vote previous-period selection, and restart post-processing with checked
  underflow/overflow statuses. The unused C++ `CurrentPillarBlockAnchor/currentPillarBlockAnchor` surface is deleted.
- Every pillar-manager current-anchor consumer now uses the runtime snapshot rather than C++ sidecar facts. Vote
  relevance and single admission derive the anchor internally. Checked external single-vote prepare retains a one-time
  token keyed by canonical vote hash. The shim retains a bounded receipt for every successful external validation, so
  its corresponding add always runs checked preparation again against the then-current anchor, and the receipt is
  consumed only after successful checked admission so retries cannot fall through; only receipt-free local/restart
  calls use trusted preparation. Trusted prepare cannot replace an existing checked token, and apply
  consumes the token, verifies its generation, and reruns relevance/identity checks under the anchor read lock. Both
  the Rust preparation registry and shim receipt map are capped at 4,096 entries; eviction/missing-token apply fails
  closed, and the checked re-prepare route cannot be converted into trusted admission.
  Synced bundles use generation-bound runtime prepare/apply, and PBFT-facing finalization derives the current
  period/hash/canonical block RLP and checked vote-request period from Rust state. Read guards cover the corresponding
  vote mutation or finalization persistence/cleanup, so a concurrent current-block publish cannot invalidate accepted
  work. C++ finalization takes its compatibility mutex before entering Rust and holds it through durable Rust effects and
  compatibility publication; callbacks run after unlock. The C++ current-block object remains only for startup/public
  compatibility, block-creation vote-count materialization, logging, and post-decision legacy event payload matching.
- Pillar strict-majority threshold calculation is Rust-owned through
  `BridgePillarChainRuntime::pillar_chain_runtime_consensus_threshold`. C++ supplies only the typed external FinalChain
  total-vote fact. The standalone `inspect_pillar_vote_bundle_rlps` CXX API and DTO are deleted; runtime bundle prepare
  performs canonical inspection while binding recovered voters to the current anchor generation before C++ obtains
  external DPoS weights. Validation passed 903 focused `rustaxa-consensus`/`rustaxa-bridge` tests, all three focused CXX
  pillar runtime tests, all nine Rust storage bridge tests, isolated pillar creation, sync, admission, and recovered-
  identity uniqueness consumers, the PBFT single-node consumer, feature-on builds through `taraxad`, Tier 1, Tier 2,
  and the startup smoke gate. Mapping, API/architecture design, Rust/C++ implementation, and independent closeout review
  used the code-mapper, api-designer, architect-reviewer, rust-engineer, cpp-pro, and reviewer agents. No blockchain/EVM
  agent was needed because FinalChain/EVM execution and contract behavior remain unchanged external boundaries.
- `BridgePillarChainRuntime` now also restores, canonically validates, publishes, and queries the latest-finalized pillar
  block inside the same lock-protected pillar snapshot. Pillar creation reads its previous validator vote-count snapshot
  from the runtime-owned canonical current-data row, while creation and linkage planning derive the finalized parent
  period/hash internally. PBFT-facing finalization now prepares a generation-bound canonical pillar row without mutating
  durable or published state, appends that row to the same Rust-owned primary PBFT storage batch, and acknowledges only
  after commit to authenticate the exact durable row, publish the latest-finalized snapshot, and clean matching votes.
  Missing or mismatched durable rows retain the bounded, same-generation reusable preparation token for retry. PBFT
  reconciliation runs after protected locks are released even when a later protected action reports failure, and the
  compatibility mutex covers acknowledge plus latest-block identity materialization but not event callbacks. Startup
  rejects a latest period ahead of current, conflicting same-period identities, or broken successor linkage, and
  latest-row lookup compares decoded numeric periods so little-endian rollover cannot select
  period 255 over period 256. The C++ facade no longer owns
  `last_finalized_pillar_block_` or `current_pillar_block_vote_counts_`; its public latest-block getter materializes the
  runtime's canonical bytes only at the compatibility boundary. The standalone CXX vote-count, linkage, and creation
  planners plus their bridge-mechanics tests are deleted; native consensus planner tests and runtime-owned bridge tests
  retain the behavior coverage. The active delegation interface could not select or report `.codex/agents/*.toml`
  profiles, so the resulting reports were generic delegation and were not claimed as verified custom-profile
  invocations.
  Validation completed for the uncommitted follow-up:
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge runtime_finalization -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_chain -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pillar_chain_test pbft_manager_test --parallel 12`
  - focused CXX pillar bridge, pillar creation/sync, and PBFT proposal tests
  - `cmake --build /build --target rust_storage_tests --parallel 12` and `/build/bin/rust_storage_tests`
  - both rewrite boundary guards, `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and
    `.githooks/pre-commit`
  - original upstream pillar-manager header/source diffs remain empty
  The isolated `PillarChainTest.finalize_root_in_pillar_block` fixture still aborts before the changed runtime route at
  the classified external-EVM boundary because block 3 has no committed external-EVM bridge-root state; pillar creation,
  pillar sync, runtime finalization, and the PBFT proposal consumer pass independently.
- `pillar_chain_manager_shim::validateSyncPillarVotesBundleDeterministically()` now routes synced bundle RLPs through
  Rust-owned batch inspection and `BridgePillarChainRuntime` weighted apply APIs. C++ only performs the external
  FinalChain DPoS weight lookup in one batched read, then passes canonical RLP bytes and weights back to Rust for
  signature validation, duplicate/conflict checks, threshold selection, and selected-vote insertion. The previous
  shim-local per-vote inspection/weight loop and `getPillarVoteWeight()` helper are gone.
- `pillar_chain_manager_shim::createPillarBlock()` now calls
  `plan_pillar_block_creation_with_vote_counts`, which combines pillar-block shell planning with ordered validator
  vote-count delta planning behind one Rust API. C++ still owns FinalChain DPoS vote-count reads, temporary
  `PillarBlock` materialization, current-block storage payload materialization, and live manager mirrors, but the
  persisted current-block sidecar write now enters through `BridgePillarChainRuntime` instead of the storage-only handle.
  It no longer separately orchestrates the creation planner and vote-count planner before constructing a candidate block.
  The creation-only `plan_pillar_block_creation` CXX export and shell-only DTO are deleted; native Rust still owns the
  lower-level domain planner internally.
- The no-caller plain-fact pillar-vote bundle CXX planner is deleted:
  `PillarVoteBundleFact`, `PillarVoteBundleAcceptedVote`, `PillarVoteBundlePlan`, and
  `plan_pillar_vote_bundle` are no longer bridge exports. Live pillar-chain sync keeps the canonical RLP boundary:
  `BridgePillarChainRuntime::pillar_chain_runtime_prepare_weighted_rlp_bundle` returns recovered voters and a current
  anchor generation for the one external FinalChain DPoS weight read, then generation-bound
  `pillar_chain_runtime_apply_weighted_rlp_bundle` owns weighted validation, threshold initialization, selected-vote
  insertion, and duplicate/idempotent apply classification. Native `rustaxa-consensus` pillar-vote tests keep coverage
  for the plain domain planner.
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
  facts; Rust re-derives signature identity, initializes period state, and inserts into Rust-owned aggregation. The own
  pillar-vote persistence write now also enters through `BridgePillarChainRuntime`, leaving the storage-only write API to
  `storage_shim` compatibility. The
  piecemeal single-vote CXX exports
  `pillar_votes_period_data_initialized`, `pillar_votes_init_period_data`, `pillar_votes_vote_exists`,
  `pillar_votes_is_unique_identity`, `pillar_votes_is_unique_vote`, and `pillar_votes_insert_vote` are deleted along
  with `PillarVotePayload`, `PillarVoteIdentityPayload`, `PillarVoteUniqueOutcome`, and `PillarVoteInsertOutcome`.
- Follow-up relevance cleanup routes `PillarChainManager::isRelevantPillarVote` through
  `BridgePillarChainRuntime::pillar_chain_runtime_plan_vote_relevance`. Rust now decodes the vote RLP and derives
  duplicate membership from runtime-owned vote state, so the C++ shim no longer materializes Rust-retained payloads or
  scans them only to supply `vote_already_known`.
- PBFT-facing pillar-block finalization now calls Rust prepare before the PBFT primary batch and Rust acknowledge after
  that batch commits. Rust owns selected-vote lookup, deterministic planning, same-batch pillar persistence, durable-row
  authentication, runtime publication, and vote cleanup ordering; prepare itself is side-effect-free apart from its
  bounded generation-bound token registry, which reuses identical requests and evicts the oldest entry at its cap.
  Missing or mismatched durable rows preserve the token for retry, while successful acknowledgement consumes it. C++
  runs acknowledgement after protected locks are released even if a protected finalization action failed, and holds the
  compatibility mutex through acknowledgement and latest identity materialization before unlocking for event emission. The
  bridge-only CXX exports `plan_pbft_finalization_pillar_preflight`,
  `report_pbft_finalization_pillar_preflight`, and `plan_pillar_block_finalization` plus their DTOs are deleted. C++
  still owns the missing-vote network request, legacy vote materialization for PBFT `PeriodData`, live
  compatibility materialization, and pillar-finalized event emission.
- `GetPillarVotesBundlePacketHandler` no longer calls `PillarChainManager::getVerifiedPillarVotes()` or reconstructs
  C++ `PillarVote` objects for network serving. It now asks `pillar_chain_manager_shim` for packet-ready optimized
  bundle chunks from `BridgePillarChainRuntime::pillar_chain_runtime_build_verified_vote_network_bundles`, wraps each
  inner bundle RLP as the tarcap packet payload, sends it, and marks the returned vote hashes known. Rust serves
  runtime-retained votes first and falls back to stored `PeriodData` only when the embedded optimized bundle matches the
  requested period/hash.
- `PillarChainManager::getVerifiedPillarVotes()` remains only for public compatibility/tests and later PBFT
  `PeriodData` cleanup, but it no longer reads persisted period-data bytes directly. Its runtime API now returns
  live vote payloads first and performs the same strict stored-`PeriodData` fallback in Rust, leaving the shim as
  temporary C++ `PillarVote` materialization only.
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
  transaction-manager, PBFT-chain, FinalChain/EVM, advance-period, and pillar side effects before reporting
  typed facts or explicit failure back to Rust. The CXX exports for `plan_pbft_finalization_runtime`,
  `pbft_manager_runtime_finalization_session_next`, `pbft_manager_runtime_finalization_session_report`,
  `pbft_manager_runtime_finalization_session_report_action`,
  `pbft_manager_runtime_report_finalization_live_mutation`,
  `pbft_manager_runtime_report_finalization_live_mutation_boundary`,
  `pbft_manager_runtime_report_finalization_failure_boundary`,
  `pbft_manager_runtime_drain_owned_finalization_actions`, and
  `pbft_manager_runtime_apply_finalization_storage_writes` plus the older piecemeal finalization boundary APIs are
  deleted.
- Fresh finalization and durable duplicate-resume now share one Rust-cursor-driven external-action dispatcher in
  `pbft_manager_shim`. After the runtime starts, C++ no longer consults `finalization_plan.cleanup.*` booleans or a
  separate resume action chain to decide sequencing: it executes only the current Rust action and reports through the
  existing typed subsystem API. The fresh sortition/reward/DAG/transaction/PBFT-chain prefix remains under one
  DAG-and-transaction lock scope; the same loop releases those locks before FinalChain, period advance, and pillar
  effects. Resume rejects protected-prefix actions, and prepared payloads are single-use contract inputs.
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
  PBFT-chain, FinalChain replay/dispatch, advance-period, and pillar post-processing executors now return
  or construct only subsystem-specific reports. Rust derives finalization identity from the manager-runtime retained plan
  and maps those typed reports into the native live-mutation report internally, so C++ no longer owns the duplicate
  live-report DTO, the generic external-effect DTO, or the `makeFinalizationExternalEffectReport` mapping helper.
- The manager executor still checks the expected action before each C++ side effect runs, but subsystem reports carry
  only subsystem facts. The executor cursor is the only accepted action identity source for typed success APIs and the
  failure-only `pbft_manager_runtime_fail_finalization_external_effect` API, which removes the last duplicated action
  echo and generic success/failure envelope from sortition, reward-vote, DAG, transaction-manager, PBFT-chain,
  FinalChain, advance-period, and pillar reports.
- The legacy Rust bridge-crate finalization cursor primitives
  `pbft_manager_runtime_begin_finalization_session`,
  `pbft_manager_runtime_begin_finalization_resume_session`,
  `pbft_manager_runtime_finalization_session_next`,
  `pbft_manager_runtime_finalization_session_report_action`,
  `pbft_manager_runtime_report_finalization_live_mutation`, and
  `pbft_manager_runtime_drain_owned_finalization_actions` are now private implementation helpers. C++ and external bridge
  consumers can only drive the manager-owned finalization path through the executor APIs listed above.
- The explicit abort/reset call is now removed as well. `pbft_manager_runtime_start_finalization_executor`, the
  failure-only external-effect API, and all typed finalization advancement APIs clear the retained runtime cursor and
  accepted plan internally whenever they return a terminal state or throw an error across CXX. C++ no longer performs
  Rust cursor hygiene after failure; post-terminal reports now observe `PBFT_FINALIZE_RUNTIME_SESSION_NOT_STARTED`
  instead of stale cursor state.
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
  cache mutation. It first moved through a typed `AnchorDagCacheFinalizationClearReport`, and the current runtime-owned
  drain has since absorbed the action completely, leaving C++ only the temporary sidecar-map clear signaled by
  `cleared_anchor_dag_cache`.
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
- Follow-up API narrowing first moved anchor-cache clear reporting onto a typed Rust bridge advancement API. The current
  runtime-owned drain has now absorbed that manager-local action entirely: `BridgePbftManagerRuntime` clears Rust
  anchor-cache metadata, validates the native live-mutation report with zero remaining anchors, advances the
  finalization cursor, and returns `cleared_anchor_dag_cache` so the C++ shim clears only its temporary materialized
  `DagBlock` sidecar map. The `AnchorDagCacheFinalizationClearReport` C++ helper,
  `PbftManagerFinalizationAnchorCacheClearReport` CXX DTO, and
  `pbft_manager_runtime_advance_finalization_anchor_cache_clear` export are deleted.
- Follow-up CXX surface cleanup deleted the no-caller anchor-cache metadata wrappers
  `pbft_manager_runtime_cached_anchor_dag_order_count` and
  `pbft_manager_runtime_clear_cached_anchor_dag_order`. The PBFT manager runtime still owns native count/clear behavior
  and tests it inside `rustaxa-consensus`; bridge-module tests now inspect the native runtime state directly instead of
  preserving CXX exports with no C++ shim caller.
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
- Custom agents used for the anchor-cache owned-drain cleanup:
  - `architect-reviewer`: recommended moving `ClearAnchorDagCache` into the manager-owned finalization drain and keeping
    only a C++ sidecar-clear signal for temporary `DagBlock` materialization.
  - `code-mapper`: confirmed the broader surface candidates and helped separate this Slice 6-owned action from lower
    value test-only FFI shrink candidates.
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
- PBFT lifecycle transitions are now one-shot operations on `BridgePbftManagerRuntime`. Rust retains the startup
  Cacti/lambda/timing policy, derives the current period/round/step and transition timing, cert-voted metadata, and executed flag from its runtime; loads own-vote
  hashes directly from native storage; plans and commits transition persistence; and mutates runtime only after commit.
  C++ receives only the authoritative snapshot and temporary sidecar/timer/print/VoteManager commands plus the
  externally ordered executed-block reset follow-up.
- Filter, certify, finish, finish-polling, loop-back, both delays, explicit reset, and advance-period reset all use the
  lifecycle executor. Advance-period planning now proves that it follows the immediately preceding committed reset and
  returns only the remaining external follow-ups; missing, stale, mismatched, and empty-chain requests are rejected, and
  the duplicated `ApplyResetConsensusTransition` action is removed.
- The CXX `PbftManagerTransitionFact`, `PbftManagerTransitionPlan`, and
  `PbftManagerTransitionRuntimeApplyResult` DTOs plus `plan_pbft_manager_transition` and
  `pbft_manager_runtime_apply_transition_storage_write` exports are deleted. Native transition planner/storage types and
  tests remain Rust implementation machinery.
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
- Additional validation for anchor-cache owned-drain cleanup:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_drains_anchor_cache_clear_as_owned_finalization_action -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge manager_runtime_finalization -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.Finalization*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
  - `rg -n "AnchorDagCacheFinalizationClearReport|PbftManagerFinalizationAnchorCacheClearReport|advance_finalization_anchor_cache_clear" rust/crates/rustaxa-bridge/src libraries/core_libs/consensus/shims/pbft_manager_shim tests/rust/consensus -g'*.rs' -g'*.cpp' -g'*.hpp'`
    returns no live code references.
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
- PBFT duplicate-finalization resume inspection is folded into
  `pbft_manager_runtime_start_finalization_executor`. Resume mode now takes the accepted finalization plan plus the
  external FinalChain last-block fact, then `BridgePbftManagerRuntime` inspects runtime-owned storage and starts the
  replay cursor internally. The CXX `PbftFinalizationResumePlan` DTO and public
  `pbft_manager_runtime_inspect_finalization_resume` method are deleted; native Rust resume plans remain only for
  domain/private bridge tests. Complete duplicates return a completed no-action executor state, making no-op duplicate
  acceptance explicit at the manager boundary.
- The PBFT manager shim's duplicate-finalization resume and fresh-finalization paths now share one local
  finalization-boundary helper set for snapshot application, action requirement checks, failure reporting, and typed
  subsystem report advancement. This shrinks the remaining C++ executor loop without adding a generic Rust external
  effect API; FinalChain, DAG, transaction-manager, PBFT-chain, sortition, vote-manager, advance-period, pillar, and
  anchor-cache cleanup is now manager-runtime owned; remaining FinalChain, DAG, transaction-manager, PBFT-chain,
  sortition, vote-manager, advance-period, pillar, and network side effects still cross through the existing minimal
  typed APIs.
- No new transport/network/VDF failures were introduced by the current slice state, but `pbft_manager_shim` and
  remaining pillar-chain external DPoS/materialization/event paths are still present and remain Slice 6 work.
- PBFT sync period admission is now a cursor owned by the long-lived `BridgePbftManagerRuntime`. `processPeriodData()`
  captures immutable queue/chain facts once, then executes only the FinalChain, reward-vote, cert-vote, transaction, and
  pillar checks requested by Rust and reports typed results with the current cursor. Rust owns check ordering,
  accumulated validation state, transaction-warning classification, replacement intent, and the terminal
  accept/drop/wait/clear-and-report decision. C++ retains live-object materialization and the explicit external
  FinalChain, VoteManager, TransactionManager, PillarChainManager, and network effects.
- The standalone `plan_pbft_sync_process_period_data_runtime` CXX export and its 25-field
  `PbftSyncProcessPeriodDataRuntimeFact` DTO are deleted. Bridge-shaped C++ tests for repeatedly rebuilding that fact are
  also deleted; native and manager-runtime session tests now cover the complete transcript, optional pillar path,
  cursor/report mismatch, peer failure, and abort behavior.
- FinalChain-behind handling remains a same-candidate retry: Rust returns an active wait step, C++ waits at the external
  FinalChain boundary, and the next cursor rechecks the already-popped candidate. Terminal and contract-error steps clear
  the retained runtime cursor automatically, while C++ aborts the cursor before propagating any external-executor
  exception.
- Validation for the manager-owned PBFT sync-admission cursor:
  - `cargo fmt --manifest-path rust/Cargo.toml --all --check`
  - `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_sync -- --nocapture`
  - `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_sync -- --nocapture`
  - `cmake --build /build --target rust_consensus_tests pbft_manager_test --parallel 12`
  - `/build/bin/rust_consensus_tests --gtest_filter='RustPbftSyncTest.*' --gtest_print_time=1`
  - `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.pbft_manager_run_multi_nodes' --gtest_print_time=1`
  - `scripts/rewrite_bridge_inventory_guard.sh`
  - `scripts/rewrite_storage_boundary_guard.sh`
  - `git diff --check`
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
- The immediate follow-up is continuing Slice 6 service consolidation and the later pillar-chain runtime work that still
  needs external DPoS fact ports plus legacy materialization removal.

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
- `BridgeGasPricer` is retired after transaction/gas service composition. `BridgeTransactionManagerRuntime` now owns
  `GasPriceOracle`, the proposal DAG gas limit, and the queue used to derive pool bids. Storage-backed construction
  restores both transaction count and finalized gas-price history before returning the runtime, so production has one
  state owner and no independent gas-pricer handle or late storage injection.
- The old `create_gas_pricer`, `create_gas_pricer_from_storage`, `gas_pricer_bid`, `gas_pricer_bid_from_pool`, and
  `gas_pricer_update` CXX exports and the bridge-only `gas_pricer.rs` module are deleted. Production `GasPricer` calls
  shim-owned `TransactionManager` bid/update methods under the transaction runtime lock; pool mode derives its queue
  floor in Rust instead of passing a scalar through C++. A clearly storage-free combined runtime constructor remains
  only to preserve standalone `GasPricer` facade tests.
- Transaction/gas composition validation passed all 286 `rustaxa-bridge` tests, all 643 `rustaxa-consensus` tests,
  `gas_pricer_test` (2/2), `transaction_manager_shim_test` (36/36), `transaction_test` (13/13), `dag_test` (12/12),
  the fast bridge/storage inventory gates, and the Rust-enabled startup smoke. Tier 3 CTest remained 22/28: the same
  five broad binaries reuse `/tmp/taraxa0` within one process and fail on RocksDB lock acquisition, while `go_test`
  remains blocked by the existing static cgo linker environment. The Python entrypoint remained blocked before test
  collection because pinned `cytoolz` and `pyethash` cannot build against the available Python 3.13 environment without
  `Python.h`. The direct `rust_consensus_tests` result remained 43/62 with the established legacy reward-cursor bootstrap
  fixture ambiguity; no transaction/gas case failed.
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
  `BridgeStorage::clear_own_verified_votes` now use the `BridgePbftService` vote persistence API, and the broad
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
- The standalone `inspect_pbft_finalization_resume` CXX export and later runtime-scoped
  `pbft_manager_runtime_inspect_finalization_resume` CXX method are deleted. Production duplicate-finalization recovery
  starts resume mode through `pbft_manager_runtime_start_finalization_executor`; C++ supplies only the FinalChain
  last-block fact, and `BridgePbftManagerRuntime` inspects runtime-owned Rust storage internally before creating the
  replay cursor. Rust bridge and native consensus tests exercise the native resume inspector directly.
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
  live DAG manager state and both native DAG graphs are owned by `BridgeDagManagerRuntime`. The later standalone graph
  cleanup also deleted `BridgeDagGraph`; direct `Dag`/`PivotTree` compatibility now remains pure-C++ reference coverage
  rather than a Rust-mode production surface.
- No-caller CXX exports for standalone DAG helper planners (`dag_derive_frontier`,
  `dag_validate_pivot_tips_metadata`), PBFT-chain storage restore (`restore_pbft_chain_storage`), and the old
  fact-shaped transaction-manager runtime known check (`transaction_manager_runtime_is_transaction_known`) are deleted.
  Live callers use runtime-owned DAG methods, `create_pbft_chain_from_storage`, and the hash-only transaction-manager
  runtime known check instead.
- `BridgePbftVoteValidationRuntime` is deleted. The standalone validation replay/threshold runtime had no external C++
  callsites and only protected older bridge tests; production Rust-mode vote validation uses the runtime inside
  `BridgePbftService`, whose
  admission runtime owns replay protection, threshold caching, verified-vote metadata, and retained payloads together.
- Standalone PBFT vote planner CXX exports are deleted:
  `pbft_vote_progress_plan_precheck`, `pbft_vote_progress_plan_after_add`, `pbft_vote_ingress_plan`,
  `pbft_vote_bundle_ingress_plan`, `pbft_reward_votes_plan`, `pbft_vote_validation_plan`,
  `pbft_validate_canonical_vote`, `pbft_vote_event_fact_from_canonical_vote`, and
  `pbft_derive_vote_progress_fact_from_canonical_vote`. Live C++ ingress now uses `BridgeConsensusNetworkApi`, live
  validation/admission/reward-vote materialization uses `BridgePbftService`, and the bridge-only DTOs/modules that
  existed solely for the removed free functions are deleted. `pbft_inspect_canonical_vote`, weighted vote payload
  conversion, and vote generation helpers remain because `vote_manager_shim` still calls them directly.
- The no-caller scalar threshold helper `pbft_vote_sortition_threshold_for_bridge` is also deleted from the CXX surface.
  Native `rustaxa-consensus` keeps `pbft_vote_sortition_threshold` for validation, threshold planning, and vote
  generation; live C++ proposer screening still uses `pbft_proposer_sortition_plan`.
- `BridgeTransactionQueue` and its standalone CXX exports are deleted with `transaction_queue_shim`. Production queue
  state is private to `BridgeTransactionManagerRuntime`; native `rustaxa-consensus` queue tests retain domain coverage.
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
- The former `BridgeProposedBlocks::proposed_blocks_snapshot` was removed before the handle itself. Production C++ now
  uses the PBFT service snapshot method, which preserves validation flags and payloads needed by the shim facade;
  grouped hash snapshots remain Rust test-only coverage.
- The no-storage `create_proposed_blocks_index` CXX constructor plus standalone
  `proposed_blocks_cleanup_candidates`/`proposed_blocks_remove_period` CXX helpers are deleted. Rust-mode
  `ProposedBlocks` now requires the shared PBFT service, and the PBFT local proposal path uses one isolated,
  non-persisted Rust candidate lookup batch.
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
  from the bridge surface. Live C++ uses manager-owned PBFT sync admission and proposal runtime sessions,
  `plan_pbft_manager_block_validation`, and
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
  deleted from the bridge surface. The later Slice 6 consolidation also deleted
  `plan_pbft_sync_process_period_data_runtime` and its repeated-input fact DTO: live C++ now drives the manager-owned
  sync-admission cursor, whose step carries transaction-query output only when the transaction executor needs it.
  Native `rustaxa-consensus` tests cover the lower-level admission and transaction-query planners.
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
  pillar/storage shims retain only the narrower current/latest block, own-vote, and finalized-block storage methods they
  call.
- `BridgePillarChainStorage::pillar_chain_storage_load_period_data` is deleted from the CXX bridge surface after the
  current export/caller inventory found no C++ consumer. Native period-data recovery remains inside
  `BridgePillarChainRuntime::pillar_chain_runtime_load_startup_bootstrap`; the storage compatibility handle now exposes
  only methods used by `storage_shim`.
  Validation for this pillar storage export shrink:
  - `rtk cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge --tests`
  - `rtk cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain -- --nocapture`
  - `rtk cmake --build /build --target pillar_chain_test rust_consensus_tests --parallel 12`
  - `rtk /build/bin/pillar_chain_test --gtest_filter='PillarChainTest.pillar_chain_db' --gtest_print_time=1`
  - focused pillar planning, vote-bundle, vote-inspection, and network API tests in `rust_consensus_tests`
  - `rtk scripts/rewrite_bridge_inventory_guard.sh`
  - `rtk scripts/rewrite_storage_boundary_guard.sh`
  - `rtk make rewrite-validate-fast`
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
- FinalChain rewards-stat publication is fully session-owned in Rust. The validated execution report prepares the
  distribution and cache plan against the exact request, period, prior head, and runtime generation; the publication
  identity covers the cache mutation. C++ receives only distribution-stat RLP for temporary `BlockStats`
  materialization at `StateAPI`, while Rust attaches, publishes, audits, and reloads the durable cache. The former
  `FinalChainExternalEvmRewardsStatsUpdate` carrier and C++ commit/clear acknowledgements are deleted.
- Transaction-manager Rust-mode expired non-finalized cleanup now deletes pending transaction storage rows through a
  native `rustaxa-consensus` batch helper before mutating the live sidecar. This closes the Rust shim gap where
  `removeNonFinalizedTransactions` previously cleared only sidecar state while the legacy C++ implementation also
  removed matching DB rows. The remaining public `DbStorage` batch blocks in
  `libraries/core_libs/consensus/src/transaction/transaction_manager.cpp` are legacy-only under the current overlay.
- Proposed-block Rust-mode persistence and cleanup are also closed under the current overlay. The remaining public
  `DbStorage` batch block in `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` is legacy-only when
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` is enabled; Rust-mode cleanup enters
  `BridgePbftService::pbft_service_proposed_blocks_cleanup_with_storage`, which commits the delete batch in native Rust
  storage before removing stale periods from the service-owned index.
- Sortition Rust-mode startup and finalized-period persistence are closed under the current overlay. Master
  `RUSTAXA_ENABLE` mode selects the standalone facade and excludes the untouched original implementation; the redundant
  sortition-specific feature flag and `SortitionParamsManagerOld` scaffold are retired. The shared
  `SortitionParamsChange` compatibility carrier and canonical RLP codec now live in shim-owned files. Rust-mode
  construction and updates enter `BridgeSortitionParamsManager` with an attached native Rust storage handle.
- The direct `sortition_params_for_period(found, change)` CXX export is deleted. C++ sortition callers now use only
  `sortition_params_for_period_from_storage(period)` for historical lookups, while direct change-payload lookup coverage
  remains in native Rust sortition tests.
- The no-caller unstaged sortition mutation route is deleted. `SortitionParamsManager::applyBlockForSortitionRuntime`
  and the CXX-only `sortition_record_finalized_period` wrapper no longer provide a second way to publish live threshold
  state without persistence. PBFT finalization continues through preview, primary Rust-owned batch persistence, and
  post-commit runtime commit; the public `pbftBlockPushed(..., Batch&, ...)` signature remains an intentional cross-mode
  compatibility API whose Rust implementation persists atomically through its native storage handle.
  Validation passed with all nine focused Rust bridge sortition tests, all three CXX sortition bridge tests, all three
  sortition shim tests, all 13 public sortition tests, `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, and `make rewrite-validate-smoke`; source audits found no removed API references and
  confirmed the original upstream sortition header/source remain unchanged.
- FinalChain Rust-mode startup, native finalization, external-EVM publication, crash recovery, and storage audit are
  closed under the current overlay. The remaining public `DbStorage` batch blocks in
  `libraries/core_libs/consensus/src/final_chain/final_chain.cpp` are pure-C++ reference-only and the source is not
  compiled when `RUSTAXA_ENABLE_FINAL_CHAIN` is enabled; Rust-mode publication enters
  `BridgeFinalChain`/`BridgeConsensusExecutionApi` and commits FinalChain storage rows through native Rust storage.
  `StateAPI` remains the external EVM/state database boundary.
- The broader Slice 8 API shrink remains open; this guard is the closeout mechanism for future bridge-handle deletions
  and additions.
- The 2026-07-15 minimum-surface audit found no ordinary no-caller CXX function export: every exported function has a
  production C++ caller except the guard-confined FinalChain storage-conformance fixture helper. The first carrier
  cleanup therefore removes bridge declarations rather than behavior: the C++-only transaction finalized-check input
  is now a documented shim-owned type, while the transaction admission outcome and three DAG finalization staging
  carriers are private Rust module types. C++ continues to receive only the existing transaction admission command
  report, finalized-check sidecar outcome, and DAG finalization apply payload. Further material shrink requires
  migrating active compatibility callers behind an application-owned Rust service; it must not be presented as an
  unused-export deletion pass.
- PBFT finalization report-surface cleanup removed the bridge-only `PbftFinalizationLiveMutationReport` CXX DTO and the
  PBFT manager shim's `makeFinalizationExternalEffectReport` mapper. Follow-up cleanup removed the public generic
  `PbftFinalizationExternalEffectReport` CXX DTO and the generic advancement API entirely. Live C++ finalization
  executors now report success through typed subsystem APIs and failure through
  `pbft_manager_runtime_fail_finalization_external_effect`; Rust bridge internals build native live-mutation reports
  after deriving finalization identity from `BridgePbftManagerRuntime`.
- The standalone `apply_pbft_finalization_storage_writes` CXX export is deleted. Live manager-owned finalization storage
  writes enter through `BridgePbftManagerRuntime`, while the retained verified-votes storage API remains the compatibility
  surface for vote-manager finalization storage facts. The lower test-only bridge wrapper is deleted; direct
  storage-apply behavior is covered by native `rustaxa-consensus` finalization tests and retained live bridge coverage
  through the verified-votes compatibility API.
- The standalone `rustaxa-bridge/src/pbft_finalize.rs` bridge module is retired. Live finalization CXX APIs now sit on
  `BridgePbftManagerRuntime` in `pbft_manager.rs`, finalization FFI/domain conversion impls moved beside those manager
  APIs, and the only live storage-apply result mapper moved to `verified_votes.rs`.

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

## CRW-04 DAG/Transaction Application-Service Ownership Slice

This slice composes the previously independent DAG-manager and transaction-manager runtimes behind one
application-owned Rust service without yet changing the proposer transaction-pack protocol.

- `BridgeDagTransactionService` owns private `DagRuntimeState` and `TransactionRuntimeState` behind sibling mutexes and
  cloned handles to the same Rust storage owner. Normal calls lock exactly one sibling; all CXX receivers use shared
  Rust references so concurrent DAG and transaction calls do not create aliased root mutable references.
- `App` creates the fully initialized service before either retained facade and passes the same C++ RAII holder to
  `TransactionManager` and `DagManager`. Full construction restores transaction count/gas history and DAG graphs,
  counters, anchors, and the initial proposal-period mapping before publishing the service; ephemeral proposer and
  verification sessions start empty.
- Fresh storage reports a zero PBFT DAG anchor. The composed factory now preserves the configured genesis anchor only in
  that case; persisted nonzero anchors remain authoritative. This fixed a fail-fast fresh-node startup regression found
  by the focused DAG suite.
- Standalone TransactionManager and GasPricer tests use explicitly transaction-only compatibility services. Their DAG
  methods return `DAG_SERVICE_UNAVAILABLE`; production and DAG construction use the full service.
- The old `BridgeDagManagerRuntime`, `BridgeTransactionManagerRuntime`, standalone CXX factories,
  `DagManager::RustDagManagerGraphs`, facade-owned runtime boxes, and C++ restore/initial-mapping bootstrap calls are
  deleted. The existing proposer pack request/report relay, EVM gas execution, VDF, signing, network effects, and public
  materialization remain for the next bounded slice.
- Focused validation passed: the full `rustaxa-bridge` suite (292 tests), `transaction_manager_shim_test` (36 tests),
  `dag_test` (6 tests), `gas_pricer_test` (2 tests), and a freshly rebuilt
  `FullNodeTest.save_period_lambda_cacti_hf` bootstrap case. `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, `make rewrite-validate-smoke`, the bridge inventory guard, and
  `git diff --check` also passed.
- The authorized Tier 3 full CTest run passed 21 of 27 registered tests. Five suites (`pillar_chain_test`,
  `full_node_test`, `network_test`, `pbft_manager_test`, and `vote_test`) hit the known shared
  `/tmp/taraxa0/db/db/LOCK` test-harness collision, while `go_test` hit the existing static Go/cgo host-link failure.
  The focused freshly rebuilt full-node case above passed when run in isolation. The Python integration runner could
  not collect tests because this host lacks the Python 3.13 development headers needed to build pinned native
  dependencies (`cytoolz` and `pyethash`), after which `pytest` was unavailable. These are classified environment and
  harness gaps rather than failures of the changed route.

### CRW-04 Proposer Pack Relay Contraction

The follow-up slice composes the DAG proposer and transaction pack cursors inside `BridgeDagTransactionService` while
preserving network throttling and EVM gas estimation as explicit C++ executor boundaries.

- `DagProposerTransactionPackRequest`, `DagProposerTransactionPackReport`, the session-step request field,
  `DagManager::reportProposerTransactions`, `DagBlockProposer::getShardedTrxs`, and the shim-only sharded payload carrier
  are deleted. Proposal period, weight, and shard limits no longer cross CXX.
- Composite prepare/finalize/abort calls validate the private DAG cursor, bind the transaction cursor to its proposer
  session id, and transfer selected canonical hashes, RLP payloads, and gas estimates directly into DAG session state.
  Wrong-owner, out-of-order, malformed-estimate, and external-executor failure paths clean up matching cursors.
- Composite calls use one DAG-then-transaction lock order and return before C++ performs EVM work. The existing C++
  `pack_mutex_` spans prepare, unlocked EVM estimates, and finalize so public `packTrxs` cannot replace the single live
  transaction cursor. Transaction-only services reject composite calls before transaction mutation.
- C++ now observes only `Network::pbft_syncing()`, executes requested EVM estimates, and later materializes selected
  payloads at the existing add-block boundary. VDF, signing, network ownership, and add-block execution are unchanged.
- Focused validation passed: the full `rustaxa-bridge` suite (298 tests), native DAG proposer tests (18 tests),
  `transaction_manager_shim_test` (35 tests), `dag_test` (6 tests), and
  `FullNodeTest.multiple_wallets_support` (1 test). `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, and `make rewrite-validate-smoke` also passed.
- The authorized Tier 3 full CTest run again passed 21 of 27 registered suites. `pillar_chain_test`, `full_node_test`,
  `network_test`, `pbft_manager_test`, and `vote_test` failed on the existing shared `/tmp/taraxa0/db/db/LOCK`
  collision after individual cases had already passed; `go_test` reproduced the existing static Go/cgo host-link
  failure. The focused Rust-enabled full-node proposer/startup case above passed independently. The Python integration
  runner again stopped before collection because the host lacks Python 3.13 development headers for pinned `cytoolz`
  and `pyethash`, leaving `pytest` unavailable. These results match the previously classified harness/environment gaps.

### CRW-04 Finalized-DAG Transaction Cleanup Composition

This follow-up removes the live finalized-order DAG-to-C++-to-TransactionManager relay while preserving the public
transaction-removal compatibility API.

- `BridgeDagTransactionService::dag_manager_runtime_apply_finalized_order` now locks DAG before transaction, performs
  the fallible DAG/storage finalization commit, and then infallibly removes matching private non-finalized transaction
  sidecars. A failed DAG apply leaves transaction runtime state untouched, and no second storage delete can fail after
  the DAG commit.
- `DagManagerFinalizationApplyPayload::remove_transaction_hashes`, its C++ hash/set materialization, and
  `DagManager::setDagBlockOrder`'s call to `TransactionManager::removeNonFinalizedTransactions` are deleted. C++ receives
  only finalized count and expired DAG hashes for its retained public/cache shell.
- The public TransactionManager removal route and its direct compatibility tests remain. The no-caller
  `dag_manager_runtime_restore_from_storage` and `dag_manager_runtime_ensure_proposal_period_mapping` CXX exports are
  also deleted; production construction and Rust tests call the private DAG methods directly.
- Focused validation passed: `rustaxa-bridge` (299 tests), `dag_test` (6 tests),
  `transaction_manager_shim_test` (35 tests), and `FullNodeTest.multiple_wallets_support` (1 test).
  `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke` also exited
  successfully; the consensus target retained its already classified reward-cursor bootstrap and shared fixture-lock
  diagnostics outside the changed DAG/transaction path. The authorized Tier 3 full CTest run passed 21 of 27 registered
  suites. `pillar_chain_test`, `full_node_test`, `network_test`, `pbft_manager_test`, and `vote_test` reproduced the known
  same-process `/tmp/taraxa0/db/db/LOCK` collision, while `go_test` reproduced the unrelated static Go/cgo host-link
  failure. The Python integration runner stopped before collection because Python 3.13 development headers are absent,
  so pinned `cytoolz` and `pyethash` could not build and `pytest` remained unavailable.

### CRW-04 Verify-Block Transaction Availability Composition

This follow-up removes the live DAG-query-to-C++-to-TransactionManager-report relay without moving FinalChain account
reads, public transaction construction, or EVM gas estimation into Rust.

- The composed service locks DAG before transaction, privately reads the active verification cursor's query hashes and
  proposal period, and prepares ordered transaction views through queue, sidecar, and storage precedence without
  advancing. The private C++ adapter materializes and hash-validates every view, reads each resolved sender's FinalChain
  account at the exact proposal period, and submits a cursor-bound nonce completion. Rust revalidates the cursor and
  lookup, applies finalized-transaction filtering, and only then advances transaction availability.
- `DagVerifyBlockSessionStep::query_hashes`, `DagVerifyBlockTransactionReport`, the transaction-report CXX export, C++
  query-hash conversion/map construction, and the public `TransactionManager::getTransactions` call are deleted from
  `DagManager::verifyBlock`. The public compatibility API and its tests remain unchanged.
- A private `DagManager`-friend TransactionManager adapter materializes only Rust-returned transaction views, then
  constructs cursor-bound account-nonce facts for every materialized sender from exact-proposal-period FinalChain
  reads. Caller-supplied transactions retain precedence, original block order and duplicate references are reconstructed
  exactly, and the later C++ EVM `estimateTransactions` boundary is unchanged. Missing and old-finalized transactions
  terminate with the existing typed missing-transaction result.
- Focused validation passed: `rustaxa-bridge` (304 tests), `dag_block_test` (13 tests), `dag_test` (6 tests),
  `transaction_manager_shim_test` (35 tests), and `FullNodeTest.multiple_wallets_support` (1 test).
- `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke` completed. The
  consensus gate retained the known unrelated 19-test PBFT reward-cursor bootstrap failure cluster and in-process
  RocksDB lock collisions while all changed DAG/transaction targets passed. The preapproved Tier 3 CTest gate passed
  21 of 27 binaries; `pillar_chain_test`, `full_node_test`, `network_test`, `pbft_manager_test`, and `vote_test` reproduced
  the same `/tmp/taraxa0/db/db/LOCK` collision, while `go_test` reproduced the unrelated static Go/cgo host-link failure.
  The Python integration runner stopped before collection because the host lacks Python 3.13 development headers for
  pinned `cytoolz` and `pyethash`, leaving `pytest` unavailable.

### CRW-04 Proposer Transaction-Pressure Observation Composition

This follow-up removes the two remaining TransactionManager size-getter relays from proposer-session start while keeping
wallet identity, configured policy limits, and external executor facts explicit.

- `DagProposerSessionBeginInput::transaction_pool_size` and `non_finalized_transaction_count` are deleted from CXX.
  `BridgeDagTransactionService` now locks DAG before transaction, snapshots the Rust queue and non-finalized sidecar
  sizes, and installs those observations with the proposer cursor in one composed start call.
- The Rust cursor retains the snapshot for empty-pool, non-finalized-limit, and transaction-pack decisions.
  `DagBlockProposer` no longer calls `getTransactionPoolSize()` or `getNonfinalizedTrxSize()` during session start; all
  wallet, gas, tip, shard, retry, and proposal-limit inputs remain unchanged.
- Focused validation passed: `rustaxa-bridge` (305 tests), `dag_block_test` (13 tests), and
  `FullNodeTest.multiple_wallets_support` (1 test).
- `make rewrite-validate-fast`, `make rewrite-validate-consensus`, and `make rewrite-validate-smoke` completed; changed
  proposer/DAG targets passed while the consensus gate retained its known unrelated PBFT reward-cursor and in-process
  RocksDB lock failures. The preapproved Tier 3 CTest gate passed 21 of 27 binaries;
  `pillar_chain_test`, `full_node_test`, `network_test`, `pbft_manager_test`, and `vote_test` reproduced the same
  `/tmp/taraxa0/db/db/LOCK` collision, while `go_test` reproduced the unrelated static Go/cgo host-link failure. The
  Python integration runner stopped before collection because the host lacks Python 3.13 development headers for pinned
  `cytoolz` and `pyethash`, leaving `pytest` unavailable.

### CRW-04 Atomic Accepted-DAG Persistence Composition

This follow-up replaces the split transaction-save, DAG-save, and graph-publication sequence with one revalidated Rust
application-service transition while preserving public object APIs and post-commit C++ effects.

- PREPARE decodes and plans the DAG block under DAG-then-transaction locking, validates supplied transaction payloads,
  and returns either a terminal duplicate/expired/missing-reference result or indexed sender account requests. C++ reads
  latest FinalChain nonces only for those requests.
- COMPLETE revalidates the cursor and current DAG plan, applies nonce filtering, validates graph mutation on cloned
  state, and stages accepted transaction rows, `TrxCount`, the DAG block/level index, and DAG counters in one shared
  storage batch. Graph and transaction sidecar/queue/count state publish only after the batch commits.
- The canonical proposer-RLP path enforces block hash plus transaction count/order. The stable object compatibility path
  retains its supplied block identity and persists only supplied transaction objects, matching legacy callers.
- The direct DagManager-to-TransactionManager save relay, direct DAG save and graph-add calls, and obsolete DAG
  plan/save/add CXX exports are deleted. The former add-order mutex is replaced by cursor-lifetime serialization across
  each complete C++ add flow, with matching idempotent abort guards for external fact-read or completion exceptions.
  C++ retains queue-erasure logging, counter mirroring, public block materialization/cache, verified events, and network
  gossip after commit. The public TransactionManager DAG-save API is retained for compatibility callers.
- Focused validation passed: `rustaxa-bridge` (312 tests), `rustaxa-storage` (96 tests), consensus transaction-storage
  tests (6), `rust_storage_tests` (9), `dag_block_test` (13), `dag_test` (6), `transaction_manager_shim_test` (35),
  `NetworkTest.propagate_block` (1), and `FullNodeTest.multiple_wallets_support` (1). `make rewrite-validate-fast`,
  `make rewrite-validate-consensus`, and the Rust-enabled startup smoke gate passed. The task-owner-preapproved Tier 3
  CTest gate passed 21 of 27 registered suites; the five established same-process RocksDB-lock failures remained in
  `pillar_chain_test`, `full_node_test`, `network_test`, `pbft_manager_test`, and `vote_test`, while `go_test` retained
  its unrelated static Go/cgo link failure. The Python Tier 3 command did not reach collection because the Python 3.13
  environment lacks development headers needed to build pinned `cytoolz` and `pyethash`, leaving `pytest` unavailable.
  The standing Tier 3 authorization also covered `scripts/storage_conformance_diff.sh`; its C++ and Rust transcripts
  match after the runner's Rust scenario was strengthened to execute the same level-2 DAG counter update already present
  in the C++ scenario. Running that gate also exposed an older unguarded shim-only pillar bundle call in an upstream-owned
  network handler. The handler now uses the optimized Rust route only under `RUSTAXA_ENABLE_PILLAR_VOTES` and restores
  upstream packet materialization in pure-C++ mode; both configurations build. The independent reviewer returned
  `APPROVED` after the cursor atomicity and closeout-documentation fixes.

### CRW-04 Accepted-DAG Account-Fact Relay Deletion

This bounded follow-up removes the remaining accepted-add DagManager-to-TransactionManager fact relay without moving
the explicit FinalChain executor boundary into Rust.

- A shim-local DagManager adapter resolves Rust-requested senders through DagManager's existing FinalChain facade and
  returns the indexed nonce facts consumed by COMPLETE. It preserves the zero-account fallback when `getAccount`
  throws and the existing `DbException` when non-empty requests have no FinalChain.
- Both public DAG add paths use that adapter under the existing cursor lifetime guard and add-session serialization.
  The private TransactionManager declaration, forwarding definition, and shim-access implementation are deleted; the
  separate verify-block transaction-availability adapter remains unchanged.
- Focused validation passed: `dag_block_test` (13 tests), `dag_test` (6), and `transaction_manager_shim_test` (35).
  The three Rust-enabled targets rebuilt successfully, the full pre-commit hook passed, and
  `make rewrite-validate-consensus` completed with its existing classified shared-fixture lock diagnostics. Formatting,
  bridge-inventory, upstream-shape, and whitespace checks also passed. Independent review returned `APPROVED`.

### CRW-04 Verify-Block Tip-Gas Relay Deletion

This bounded CRW-04/CRW-07 follow-up removes a storage-derived verification fact roundtrip while preserving external
EVM gas estimation as an explicit C++ executor boundary.

- The Rust verification cursor retains candidate tips and conditionally loads their canonical gas estimations itself
  when evaluating the PBFT aggregate gas rule. The standalone `dag_manager_runtime_tip_gas_estimations` CXX export,
  `DagTipGas` carrier, and `DagVerifyBlockGasReport::tip_gas_estimations` field are deleted.
- `DagManager::verifyBlock` no longer duplicates Rust's `needs_tip_gas` policy or builds a tip vector. Its existing gas
  report now carries only block gas estimation, aggregate transaction weight, DAG gas limit, and PBFT gas limit.
- Focused C++ validation passed: the Rust-enabled `dag_block_test` and `dag_test` targets rebuilt against the narrowed
  ABI, then passed 13 and 6 tests respectively. Focused Rust coverage passed 3 tests, the full `rustaxa-bridge` suite
  passed 315 tests, and the full pre-commit hook passed. `make rewrite-validate-consensus` completed with its existing
  classified shared-fixture lock diagnostics. C++ formatting, bridge inventory, upstream-shape, and whitespace checks
  also passed. Independent review returned `APPROVED` after the validation record was completed.

### CRW-04 Completion Audit

The final production-boundary audit at `113020d53` found no remaining internal DAG/transaction bridge handle or state
relay. The application-owned `BridgeDagTransactionService` is the sole production lifetime owner for private DAG and
transaction state. Remaining cross-facade calls are limited to public transaction materialization, narrow FinalChain
fact collection, explicit EVM/network execution, and logging; standalone gas-pricer and transaction-service factories
are compatibility-test support only. `CRW-04` therefore satisfies its tracker completion condition. `CRW-05` is the next
dependency-ready application-owner composition item, with `CRW-07` continuing alongside its bridge deletions.

### CRW-05 PBFT-Service Slashing Ownership

The first bounded CRW-05/CRW-07 slice absorbs deterministic slashing planning and duplicate-proof state into the
application-owned PBFT service.

- `BridgePbftService` owns the planner configuration and bounded duplicate cache behind an independent sibling mutex.
  The standalone `BridgeSlashingProofPlanner` handle, its factory, and the facade-owned box are deleted.
- App bootstrap copies reporting enablement and Magnolia activation into the canonical PBFT service. The Rust-mode
  `SlashingManager` receives that same service and calls its plan/report operations, so verified-vote and direct
  compatibility submission routes share duplicate protection.
- C++ remains the explicit executor for FinalChain submitter facts, gas-price lookup, transaction construction/signing,
  and TransactionManager insertion. No Rust guard crosses those calls. Chain-only compatibility services reject
  slashing operations explicitly because they do not own planner state.
- The production verified-vote admission path remains the slashing trigger. Conflicting accepted-slot evidence reaches
  the application-owned slashing executor through the existing typed admission report; no test-only public manager API
  or second planner lifetime is introduced.
- Focused Rust bridge coverage verifies disabled reporting, the Magnolia boundary, byte-compatible proof output,
  accepted duplicate-cache publication, and rejected-submission retry. `StateAPITest.slashing` exercises the canonical
  node-owned path through pre-activation rejection, activation-period submission, and validator jailing.
- Validation passed all 318 `rustaxa-bridge` tests, including ten focused slashing tests and a two-caller shared-cache
  ownership case. The Rust pre-commit hook, bridge/storage guards, `git diff --check`, the consensus Tier 2 gate, and
  startup smoke completed; the Tier 2 gate retained its classified legacy reward-cursor and shared RocksDB fixture
  diagnostics. `StateAPITest.slashing` passed twice through real verified-vote admission. The authorized Tier 3 CTest
  run passed 21 of 27 binaries; its five C++ failures were the known same-process RocksDB-lock suites and `go_test`
  retained the unrelated static cgo link failure. The Python Tier 3 command did not reach collection because Python
  3.13 development headers are absent, so pinned `cytoolz` and `pyethash` could not build. The storage differential was
  not warranted because this slice changes no storage behavior. Independent review findings were resolved by removing
  a proposed test-only public VoteManager method, adding fail-fast slashing-capability validation, documenting the
  complete PBFT service lock domains, and proving shared service cache ownership.

### CRW-05 DAG-Service Sortition Ownership

The second bounded CRW-05/CRW-07 slice absorbs sortition runtime state into the application-owned DAG/transaction
service.

- `BridgeDagTransactionService` owns sortition as a private sibling of DAG and transaction state. Full construction
  restores all three domains from one Rust storage owner before publication; transaction-manager and gas-pricer
  compatibility services omit DAG and sortition state and return stable unavailable errors.
- Current calls acquire only their domain lock. The documented order for future composed operations is DAG, then
  sortition, then transaction, and no Rust guard crosses an external executor callback.
- The standalone `BridgeSortitionParamsManager`, its factories, and the C++ facade-owned box are deleted. App-owned
  `DagManager` injects its canonical service into the facade, while the stable three-argument compatibility constructor
  creates a full service and injected construction rejects null or transaction-only services.
- The PBFT preview/stage/commit contract and C++ `SortitionParamsChange` materialization remain typed compatibility
  boundaries. The later composition slices internalized PBFT finalization commit, DAG verification, and proposer
  historical-parameter selection. The PBFT accessor/preview and public compatibility surfaces require a separate
  retirement audit before this facade can be retired.
- Focused Rust coverage verifies full restore, compatibility rejection, and sortition behavior. Focused C++ coverage
  verifies the facade, canonical sortition behavior, service capability rejection, DAG integration, and a PBFT
  single-node production path.

Validation passed the 318-test Rust bridge suite, the full pre-commit gate, sortition shim 4/4, sortition 13/13,
Rust sortition 3/3, DAG 6/6, focused PBFT single-node coverage, the consensus Tier 2 gate, startup smoke, bridge inventory,
formatting, and whitespace checks. The authorized Tier 3 CTest run passed 21 of 27 binaries; the five C++ failures were
the classified same-process RocksDB-lock suites and `go_test` retained its unrelated static cgo link failure. Python
Tier 3 did not reach collection because Python 3.13 development headers are absent, so `cytoolz` and `pyethash` could
not build. The storage differential was not warranted because this slice changes sortition lifetime/routing, not
storage behavior. Independent review found no ownership or lock-order defect and requested retained three-argument
constructor coverage; the startup/default-state test now exercises that compatibility path and a representative read.

The next CRW-05 ownership candidates remain rewards behind FinalChain and pillar planning behind the PBFT service.
Select the next bounded slice from fresh code mapping after this slice closes.

### CRW-05 PBFT Finalization Sortition Commit Composition

The follow-up CRW-05/CRW-07 slice removes the remaining PBFT-finalization commit/report bounce through the C++
sortition facade.

- The primary finalization batch still receives the optional Rust-previewed `SortitionParamsChange` before any live
  mutation. `BridgePbftService` now retains that committed-stage identity for the active finalization session.
- The commit action calls one Rust operation over `BridgePbftService` and `BridgeDagTransactionService`. Rust validates
  the manager cursor and action first, derives period/pivot and the expected change from the manager-owned plan/session,
  then clones, validates, and publishes the next sortition state under the sortition guard before advancing the cursor.
- C++ supplies only finalized unique-transaction count, total DAG transaction references, and the post-finalization
  non-empty PBFT chain size. It no longer commits the live sortition runtime, reconstructs live threshold/cache facts,
  or echoes those facts back to Rust.
- `PbftManagerFinalizationSortitionCommitReport`, `SortitionFinalizationCommitReport`, and the rewrite-only
  `commitPreparedBlockForSortitionFinalization` helper are deleted. The public storage-owning `pbftBlockPushed` adapter,
  preview method, canonical compatibility carrier/codec, and proposer crossings remain classified; the following slice
  internalizes DAG verification.
- The cross-service lock order is PBFT manager then DAG-service sortition. No Rust guard crosses C++ callbacks; stale
  cursors fail before live sortition publication. Preview/stage divergence cannot safely enter normal duplicate resume
  after the primary batch commits, so the boundary raises a fatal post-storage invariant instead of returning a
  retryable action failure; the lower-level cloned-state helper proves mismatches do not publish live state.

### CRW-05 DAG Verification Sortition Composition

This CRW-05/CRW-07 follow-up removes the remaining direct VDF verifier and sortition-fact relay from live DAG block
verification.

- Each Rust verification step exposes its cursor identity, while the cursor privately retains the complete signed-block
  hash, action generation, proposal period, and normalized vote counts. The later request must match both that cursor and
  the exact block RLP before any proof result can advance it.
- `BridgeDagTransactionService` snapshots the active VDF action under the DAG lock, releases it, copies the historical
  sortition parameters under the sortition lock alone, verifies the proof without service locks, then reacquires the DAG
  lock and revalidates the cursor, block fingerprint, action, counts, and generation before advancing exactly once.
- Missing capability, storage failure, wrong action, request mismatch, and stale revalidation are operational errors that
  do not advance the cursor. Malformed and invalid proofs retain the existing deterministic failed-VDF consensus result.
- `DagManager::verifyBlock` now supplies only the cursor ID, signed block payload/level, external PBFT period hash, and
  FinalChain VRF key. It no longer materializes historical sortition parameters, relays cursor-owned vote counts, calls a
  free verifier, converts its result, or reports VDF status through a second bridge operation.
- `DagVerifyVdfSortitionFromBlockInput`, `DagVerifyVdfSortitionResult`, `DagVerifyBlockVdfReport`, the direct verifier
  export, and the standalone VDF-report export are deleted. Proposer sortition-parameter selection is internalized by
  the following bounded CRW-05 ownership slice.

Validation passed all 326 `rustaxa-bridge` tests, including four focused cursor-bound VDF cases; the Rust CXX bridge and
focused DAG/sortition targets rebuilt; and the DAG, DAG-block, and sortition-shim suites passed 6/6, 13/13, and 4/4.
The Rust pre-commit hook, Tier 1 and Tier 2 rewrite gates, startup smoke, bridge inventory, upstream-shape check,
formatting, and whitespace validation also passed. Independent configured review returned `APPROVED` with no blocking
correctness, concurrency, error-classification, deletion, documentation, or coverage finding.

### CRW-05 DAG Proposer Sortition Composition

This CRW-05/CRW-07 follow-up removes the remaining historical-sortition lookup and inbound parameter relay from live
DAG proposal planning while preserving asynchronous VDF execution as an explicit C++ executor boundary.

- `DagProposerFinalChainFactsReport` carries only the last finalized period and proposer authorization facts requested
  from external FinalChain. The cursor-owned proposal period is never echoed through C++.
- `BridgeDagTransactionService` snapshots the keyed cursor/action/observation/period under the DAG lock, reads the
  historical parameters under the sortition lock alone, then reacquires `DAG -> sortition`, revalidates the exact cursor,
  repeats the indexed lookup, and compares the complete parameter value before privately planning and advancing.
- Lookup, decode, capability, and planner failures clean only the matching session. A changed exact parameter value
  returns `DAG_PROPOSER_SESSION_SORTITION_PARAMS_STALE_RETRY` without advancing, and missing/wrong-action reports retain
  their existing typed session semantics.
- The selected parameters remain private during planning and are exposed as `LegacySortitionParams` only on the
  `StartVdf` executor command. C++ passes that typed command to the existing asynchronous Rust VDF proof helper; polling,
  cancellation, proof materialization, signing, add-block, network, and cleanup behavior remain outside this slice.
- The proposer-side `rustSortitionParamsForRust` lookup, inbound `SortitionRuntimeParams`, conversion helper, and old
  external-facts report/export/wrapper names are deleted. The following closeout slice retires the separately audited
  PBFT accessor/preview path; the sortition facade remains only for public compatibility.

Validation passed all 331 `rustaxa-bridge` tests, including five focused composed proposer-final-chain cases; the Rust
CXX bridge and focused DAG/sortition targets rebuilt; and the DAG, DAG-block, and sortition-shim suites passed 6/6,
13/13, and 4/4. The Rust pre-commit hook, Tier 1 and Tier 2 rewrite gates, startup smoke, bridge inventory,
upstream-shape check, formatting, Clippy, obsolete-symbol scan, and whitespace validation also passed. Independent
configured review returned `APPROVED` with no blocking correctness, concurrency, cleanup, boundary, deletion,
documentation, or coverage finding.

### CRW-05 PBFT Sortition Preparation Composition and Closeout

The final CRW-05/CRW-07 slice removes the remaining live PBFT-to-sortition-facade lookup, preview, count relay, and
caller-built storage stage.

- App injects its one `SharedDagTransactionService` directly into `PbftManager`; construction rejects null or
  transaction-only services. PBFT no longer recovers an internal Rust handle through `DagManager` or
  `SortitionParamsManager`.
- Fresh Rust finalization validates the pre-finalization PBFT head period, derives the next non-empty size from the
  service-owned chain, and decodes pivot, unique-transaction count, and total DAG references directly from canonical
  retained `PeriodData` RLP. It rejects every caller-supplied sortition stage or payload, previews sortition under the
  private service lock, appends the optional native storage stage itself, and retains the exact inputs and expected
  change across primary storage.
- The post-storage commit takes only the PBFT service, DAG service, and Rust cursor. It requires the retained
  preparation, clones and validates the next sortition state, publishes it atomically, then consumes the preparation.
  Missing or divergent preparation remains a fatal post-storage invariant and never publishes a second state.
- Resume bypasses decoding, preview, and staging because durable duplicate-finalization inspection never replays
  sortition publication. Malformed RLP, head mismatch/overflow, capability failure, storage rejection, stale cursor, and
  every terminal/error path clear the cursor and retained preparation.
- The shim-only `getDagTransactionService`, `prepareBlockForSortitionFinalization`, and `rustSortitionParamsForRust`
  methods; C++ count loops, optional-change/stage builder, and preparation flags; the commit request carrier; direct CXX
  preview/commit exports; and their bridge-mechanics test file are deleted. Public parameter materialization,
  efficiency, canonical change codec, and storage-owning compatibility APIs remain classified.

This closes CRW-05: pillar, slashing, rewards, and sortition production planning/state now live behind their Rust
application owners, and the remaining C++ surfaces are the classified FinalChain/DPoS, signing, transaction insertion,
tarcap/event, lifecycle/executor, and public-materialization boundaries.

Validation passed all 338 `rustaxa-bridge` tests, 31 focused PBFT-manager runtime tests, 28 focused native sortition
tests, all 56 `rust_consensus_tests` cases, and all four `sortition_params_manager_shim_test` cases. The Rust pre-commit
checks, Tier 1 and Tier 2 consensus rewrite gates, startup smoke, bridge inventory, upstream-shape check, obsolete-symbol
scan, and whitespace validation also passed. A full same-process `pbft_manager_test` run passed its first full-node case;
the remaining fixtures were blocked before their assertions by the existing `/tmp/taraxa0/db/db/LOCK` reuse in that
test harness, so this slice does not claim the broad suite as passing. Configured `code-mapper`, `architect-reviewer`,
`rust-engineer`, and `cpp-pro` agents mapped, approved, and implemented the boundary. Independent configured review
returned `APPROVED` with no blocking correctness, security, regression, documentation, or coverage finding; residual
risk is limited to the documented broad-suite harness gap.

### CRW-06 Storage Compatibility Classification Closeout

The bounded CRW-06 audit found no remaining unclassified Rust-mode production consensus route through `BridgeStorage`,
`BridgeStorageBatch`, bridge query-family handles, broad `DbStorage` calls, direct `getDB()`, or `rustBatchId` authority.
Native consensus and storage crates do not depend on bridge-shaped storage handles, and `rustBatchId` has no code call
sites.

Remaining storage surfaces are classified rather than silently treated as rewrite authority: typed app/service
bootstrap construction; stable `DbStorage` public compatibility; RPC, GraphQL, light, and network query views; external
FinalChain/EVM boundaries; admin, migration, and lifecycle behavior; storage conformance; and tests.
`BridgeStorageBatch` is an opaque carrier inside the stable `DbStorage::Batch` lifecycle. C++ compatibility callers
still sequence typed append operations, while Rust owns validation, key and column selection, batch storage, and atomic
commit. `BridgePillarChainStorage` remains only as the narrow public storage-shim implementation. The standalone
`rewards::Stats::processStats(..., Batch&)` surface also remains public/test
compatibility: its append semantics preserve atomic ordering in the caller's batch, and removing it would require an API
and transaction-boundary redesign without reducing production consensus authority.

Further deletions therefore belong to the owning public, network/query, admin, conformance, or test migrations rather
than CRW-06. Configured `code-mapper` and `architect-reviewer` agents independently confirmed this boundary. The
storage-boundary and bridge-inventory guards, targeted symbol searches, reusable skill/prompt drift check, and whitespace
validation passed. Independent configured review returned `APPROVED` after correcting duplicated slice-status wording and
distinguishing C++ compatibility append sequencing from Rust-owned atomic commit. CRW-06 is complete; CRW-07 remains
active alongside the next dependency-ordered parity item.

### CRW-08 Native DPoS Delegate Receipt/State Parity

This bounded CRW-08 slice makes the existing native Rust delegate path byte- and state-compatible with the legacy
FinalChain transaction boundary and adds a repeatable Tier 3 pure-C++ comparison gate.

- The first isolated pure-C++ run exposed three real divergences: Rust charged only the 40,000 delegate action gas
  instead of 22,680 intrinsic plus 40,000 action gas; its fee/balance result therefore differed; and its genesis DPoS
  account omitted the validator stake escrow and legacy nonce one.
- Rust now adds calldata-dependent transaction intrinsic gas exactly once around every Rust-native DPoS and slashing
  action. Native value transfers remain intrinsic-only. If the gas limit covers intrinsic gas but not action gas, the
  receipt fails after consuming intrinsic gas and advancing the sender nonce without transferring value, publishing
  logs, or scheduling contract mutation.
- Genesis construction sums validator `total_stake`, credits that escrow once to any explicitly configured DPoS
  precompile balance, initializes its nonce to one, and includes the result in the fallback genesis balance sum.
  Persisted account snapshots replace the derived genesis map during restart, preventing a second credit.
- `FinalChainTest.native_dpos_delegate_persists_receipt_and_state` runs unchanged expectations in Rust and pure-C++
  modes and checks exact receipt RLP/status, 62,680 gas, cumulative/header gas, log topics/data, bloom, delegator and
  precompile balances/nonces, validator stake/votes, and restart state.
- `make rewrite-validate-final-chain-parity` first runs the normal FinalChain Tier 2 gate, then configures a guarded,
  reusable out-of-tree Release build with every `RUSTAXA_ENABLE*` option disabled, builds with 12 jobs, and runs the
  focused delegate fixture followed by the complete pure-C++ `final_chain_test`. The script rejects unsafe or
  source-tree build roots, validates the CMake option inventory/cache, serializes cache access, and verifies that the
  source fingerprint did not change.

Focused Rust tests and all 650 `rustaxa-consensus` tests passed. The Rust-enabled C++ delegate fixture passed. The full
Tier 3 target passed once from a new empty cache and again from the retained warm cache, including the full Rust rewrite
gate and complete Rust-enabled and pure-C++ FinalChain suites. The pre-existing Clippy warnings remained warnings.
Configured `blockchain-engineer`, `rust-engineer`, `cpp-pro`, and `architect-reviewer` agents supplied the legacy gas and
genesis-state audit, implementation/test work, and gate design review. Existing databases containing pre-fix Rust DPoS
account snapshots require an explicit migration-or-rebuild decision; no silent persisted-state correction is included.
CRW-08 remains active for the next bounded method or failed-receipt family.

### CRW-08 Native DPoS Delegate Contract-Failure Parity

This bounded follow-up closes the legacy `delegate(address)` business-failure family without widening the accepted
external-EVM boundary.

- Current-source legacy mapping showed that missing validators, first delegations below the minimum, and delegations
  above the validator maximum are contract reverts: FinalChain charges intrinsic plus 40,000 action gas, advances the
  sender nonce, records a status-zero receipt, reverts value and DPoS mutation, and continues block execution.
- Rust previously debited the sender and credited the DPoS account before applying the contract, then propagated
  expected delegate rejection through `anyhow`. The corrected missing-validator fixture passed in pure C++ with 61,464
  gas but initially aborted Rust finalization, proving the divergence.
- Native execution now applies each DPoS/slashing call directly to the block-local working state in transaction order.
  Expected contract failures are classified before mutation, payable value and claim-gas updates commit only after a
  successful outcome, and gas plus nonce effects remain on failure. This avoids both a post-hoc refund and complete
  account/DPoS snapshot clones per native call while keeping reverted value available to a later same-sender
  transaction in the same block.
- Mutable DPoS execution starts from the immediately preceding finalized snapshot, independent of the delayed snapshot
  used for historical eligibility and authorization reads. A nonzero-delay multi-block test proves that recent
  registration/delegation state survives subsequent finalization and restart. Native contract transactions also pass
  through the common fee-reward accounting path; focused pre-Magnolia tests cover both status-one and status-zero calls.
- `apply_dpos_delegate` returns typed contract failure for the three business rejections while keeping invariant and
  overflow errors hard. Minimum-deposit enforcement applies only to a first delegation; an existing delegation may
  receive a smaller top-up, matching the legacy contract.
- Rust tests cover all three rejection reasons, the allowed small top-up, successful payable publication, and a failed
  delegate followed by a same-sender transaction that depends on immediate value rollback. The unchanged dual-mode C++
  fixture checks exact receipt RLP/status/gas, empty logs and bloom, header gas/bloom, balances, nonce, escrow, stake,
  votes, and restart state. The reusable pure-C++ parity script now runs both successful and failed delegate fixtures
  before the complete suite.

Tier 1, the FinalChain Tier 2 gate, and `make rewrite-validate-final-chain-parity` passed. The Tier 3 gate covered the
complete Rust-enabled and isolated all-Rustaxa-disabled pure-C++ `final_chain_test` suites as well as both focused
delegate fixtures. Pre-existing Clippy and dependency-build warnings remain warnings; no new warning is introduced by
this slice. Configured `blockchain-engineer`, `architect-reviewer`, `rust-engineer`, and `cpp-pro` agents supplied the
legacy contract audit, transaction-boundary review, implementation, and dual-mode fixture/gate update. No bridge handle,
CXX carrier, shim, module flag, or compatibility-only test was removed, so the bridge audit and `CRW-07` inventory do
not change. Remaining registration, undelegation, redelegation, reward, and other DPoS failed-receipt families stay
explicit follow-up work under active `CRW-08`.

### CRW-08 Native DPoS Direct Reward-Claim Contract-Failure Parity

This bounded follow-up closes the first direct reward-claim business failure without changing reward calculation or the
accepted external-EVM boundary.

- Current-source legacy contract code checks the caller/validator delegation pair before reward mutation and returns
  `ErrNonExistentDelegation` through the normal EVM contract-error channel. FinalChain therefore persists a status-zero
  receipt, charges intrinsic plus 40,000 action gas, advances the nonce, and continues the block.
- Rust previously called the internal reward-claim mutator directly and propagated its missing-delegation `anyhow`
  error, aborting native finalization instead of producing a failed receipt. The new contract-facing wrapper classifies
  only an absent delegation pair as expected failure before mutation. Cursor ordering, reward arithmetic,
  contract-balance insufficiency, storage, and codec failures remain hard errors.
- Focused Rust coverage keeps the existing successful nonzero claim path, covers a nonexistent validator separately,
  and exercises a registered validator whose delegation belongs to another account with a later same-sender transaction
  in the same block. It proves exact 61,464 claim gas, cumulative/header gas, empty logs, gas-only balance and nonce
  effects, unchanged stake/vote/reward/cursor state, persisted receipt bytes, and restart.
- The dual-mode C++ fixture registers the same `0x...01` target for an owner distinct from the caller and uses the same
  selector, gas price, gas limit, and same-block continuation. It checks canonical receipt RLP for both transactions,
  block bloom/header gas, public DPoS facts, balances, nonce, and restart.
  The reusable isolated pure-C++ parity script now includes this fixture in its focused preflight.

No CXX handle, carrier, export, shim, module flag, or compatibility-only test changes in this slice, so the `CRW-07`
bridge inventory is unchanged. Registration proof/metadata failures, undelegation lifecycles, broader redelegation
failures, and remaining reward/method families remain explicit follow-up work under active `CRW-08`.

Tier 1, the FinalChain Tier 2 gate, and `make rewrite-validate-final-chain-parity` passed. The Tier 3 gate covered the
focused three-fixture native DPoS preflight and the complete Rust-enabled and isolated pure-C++ FinalChain suites.
Pre-existing Clippy and dependency-build warnings remain warnings; this slice introduces no new warning.

### CRW-08 Native DPoS Validator-Registration Business-Failure Parity

This bounded follow-up closes current-ABI `registerValidator(address,bytes,bytes,uint16,string,string)` validation while
keeping arbitrary EVM execution outside the Rust FinalChain boundary.

- Legacy validates a 65-byte recoverable proof over the validator address, minimum stake, endpoint/description byte
  limits, 32-byte VRF key, maximum commission, duplicate registration, and maximum stake before creating contract state.
  These are contract errors: FinalChain charges intrinsic plus 80,000 action gas, advances nonce, rolls back payable value,
  emits no logs, records status zero, and continues the block.
- Rust previously discarded proof bytes, hard-failed non-32-byte VRF payloads during decoding, omitted proof/minimum/
  metadata/commission checks, and propagated duplicate or over-maximum registration as `anyhow`, aborting finalization.
  The decoded registration now retains proof and VRF bytes. A contract-facing validator normalizes legacy recovery IDs
  27/28, recovers secp256k1 over `keccak256(validator_address)`, and rejects the selected business family before mutation.
  Orphaned registration-owned rows, vote-count overflow, storage, and codec faults remain hard invariants.
- Successful state now matches the funding relationship: the transaction caller owns the initial delegation even when
  caller and validator differ. Zero-value registration with a zero minimum creates the validator but no phantom
  delegation, delegator index, or reward cursor. The staged payable value still moves only after status one.
- Rust coverage uses valid deterministic proofs for every successful registration path and covers wrong signer, proof
  length/recovery/signature failures, minimum and maximum boundaries, endpoint/description byte boundaries including
  multibyte UTF-8, VRF 31/32/33, commission 10,000/10,001, duplicate state, orphan-state hard failure, zero-value success,
  same-sender affordability after rollback, persisted receipts/header, and restart.
- The dual-mode C++ fixture executes one valid registration, nine otherwise-valid business failures, and a final
  same-sender transfer in one block. Every expected gas value is independently computed as intrinsic plus 80,000; exact
  receipt RLP/status/log/bloom/cumulative gas, balances, nonce, DPoS facts, continuation affordability, and restart are
  checked. The isolated pure-C++ focused filter includes this fixture.

Malformed ABI heads/offsets/tails, noncanonical typed words, and invalid-UTF-8 ABI strings still abort before native
gas/nonce staging. They remain a cross-method DPoS decode-failure family under active `CRW-08`; fixing only registration
would create inconsistent selector behavior. No CXX handle, carrier, export, shim, module flag, or compatibility-only test
changed, so the `CRW-07` bridge inventory is unchanged. Undelegation, broader redelegation, and remaining reward/method
failure families also remain active follow-up work.

Validation passed the workspace fast gate, the FinalChain Tier 2 gate, the Tier 3 Rust-enabled/pure-C++ parity gate, and
the bridge-inventory guard. The complete `rustaxa-consensus` package suite passed 672 tests across its library and boundary
test binaries. Gate output contained only the repository's existing compiler, clippy, CMake, and Conan warnings.

### CRW-08 Native DPoS ABI Decode-Failure Parity

This bounded follow-up closes the cross-method malformed-calldata family left by validator-registration business parity.

- Native DPoS execution now classifies supported mutation selectors before ABI decoding. Malformed known fixed-argument
  methods advance nonce, charge intrinsic plus the selector action gas, roll back value/state/logs, emit status zero, and
  continue the block. Malformed pre-fix `claimAllRewards(uint32)` preserves the legacy successful no-op with
  intrinsic-only gas. Short/unknown inputs and hardfork-disabled methods also retain zero action gas but fail normally.
- Mutation gas selection is shared by FinalChain execution and read-only call estimation. Current claim-all and valid
  legacy batches remain snapshot-dependent; both accept legacy-permitted trailing calldata. The legacy batch decoder
  rejects `uint32` overflow, while other retained narrow integer decoders intentionally truncate high bytes to match Go
  ABI behavior.
- Cornus nonpayable calls with value fail before decode or snapshot access and charge intrinsic gas only. Before Cornus,
  successful nonpayable calls retain the legacy value transfer to the DPoS account.
- Finalized validator descriptions and endpoints are raw bytes. Genesis/CXX configuration remains UTF-8 `String`, while
  ABI mutation ingress, queries, and snapshot RLP preserve arbitrary byte payloads. Existing valid-string snapshot bytes
  need no migration. Once invalid metadata is finalized, rollback to a pre-slice Rust binary is unsafe because its
  snapshot decoder still requires UTF-8; rollout therefore remains consensus-sensitive.
- Rust coverage proves claim-all trailing acceptance, scalar truncation, mutation-call gas classification, invalid-byte
  persistence, and the affected package suites. The dual-mode FinalChain fixture exercises malformed fixed/dynamic,
  short/unknown, pre-/post-fix claim-all, Cornus nonpayability, same-sender continuation, invalid-byte registration,
  byte-exact query output, and restart persistence.

No CXX handle, carrier, export, shim, module flag, or compatibility-only test changed, so `CRW-07` has no inventory delta.
Undelegation, redelegation, and remaining reward/method contract-failure families remain active under `CRW-08`.

Validation passed the workspace fast gate, the FinalChain Tier 2 gate, the Tier 3 Rust-enabled/pure-C++ parity gate, and
the bridge-inventory guard. Focused package validation passed 673 `rustaxa-consensus` library tests plus 377
`rustaxa-types`/`rustaxa-bridge` tests. Gate output contained only the repository's existing compiler, Clippy, CMake, and
Conan warnings.

### CRW-08 Native DPoS V1 Undelegate Pre-Mutation Failure Parity

This bounded follow-up closes four ledger-derived V1 `undelegate(address,uint256)` business failures without claiming
the V1 request lifecycle or successful escrow release.

- Rust preflight now classifies a missing validator, a missing caller/validator delegation, an amount greater than the
  delegation, and a nonzero remainder below `minimum_deposit` as normal contract failures before mutation. Aggregate
  validator stake that cannot cover an otherwise valid removal remains a hard invariant error.
- The selected failures consume the legacy 60,000 action gas, produce status-zero receipts, advance nonce, charge gas,
  roll back value, emit no logs or bloom, and preserve account, delegation, aggregate stake, vote, and reward state.
- Rust unit coverage proves every selected rejection plus the corrupt aggregate-stake invariant. The dual-mode
  FinalChain fixture proves exact receipts and cumulative gas, same-sender block continuation, unchanged DPoS facts,
  balances/nonces, and restart persistence.
- Existing V1 undelegation requests, pending-request persistence, `confirmUndelegate(address)`,
  `cancelUndelegate(address)`, V1 query behavior, zero-amount requests, and successful end-to-end escrow release remain
  explicit follow-up work.

No CXX handle, carrier, export, shim, module flag, or compatibility-only test changed, so `CRW-07` has no inventory delta.
`CRW-08` remains active for the V1 undelegation lifecycle, redelegation, and remaining reward/method failure families.

Validation passed the focused Rust V1 failure and V2 regression tests, the retained V1 undelegate success test, the
focused Rust-enabled FinalChain fixture, the workspace fast gate, the FinalChain Tier 2 gate, the Tier 3
Rust-enabled/pure-C++ parity gate, the bridge-inventory guard, formatting, and whitespace checks. Gate output contained
only the repository's existing compiler, Clippy, CMake, and Conan warnings.

### CRW-08 Native DPoS V1 Undelegation Lifecycle Parity

This bounded follow-up closes the complete V1 request lifecycle rather than persisting principal without both terminal
operations.

- Successful `undelegate(address,uint256)` writes one ordered request per delegator/validator pair in the same staged
  DPoS snapshot as reward, delegation, stake, and vote changes. Duplicate detection and unlock-block overflow checks run
  before mutation; a valid zero-amount call still creates the legacy slot and log.
- `getUndelegations(address,uint32)` returns the legacy static-entry array payload, 20-entry pages, end flag, current
  validator-existence fact, and at least one 5,000-gas storage read. `getValidator(address)` derives its pending count
  from both V1 and V2 queues at Magnolia and later, while pre-Magnolia queries retain the legacy zero count. The
  post-Magnolia derivation intentionally fixes the legacy `ValidatorV1` persisted-counter blind spot for requests
  created before the fork, preserving validator metadata and cancelability while a request remains pending.
- `confirmUndelegate(address)` keeps missing and locked requests as status-zero outcomes, transfers the queued principal
  from DPoS escrow only after unlock, and emits `UndelegateConfirmed`. Magnolia validator deletion requires zero stake,
  zero commission rewards, and no remaining V1 or V2 requests.
- `cancelUndelegate(address)` keeps missing requests and missing validators as status-zero outcomes, removes the request,
  claims or checkpoints rewards, restores the delegation/stake/vote state without an escrow transfer, and emits
  `UndelegateCanceled`.
- The DPoS snapshot codec appends the ordered V1 queue as item 21 and retains decoding for the previous 20-item schema.
  A pre-slice binary cannot decode newly finalized 21-item snapshots, so rollback requires restoring pre-slice storage.
- Focused Rust and restart-backed dual-mode FinalChain coverage protects request order/codec compatibility, query shape,
  duplicate/missing/locked rollback, cancel restoration, confirm payout, exact gas/logs, and mixed V1/V2 deletion rules.

No CXX handle, carrier, export, shim, module flag, or compatibility-only test changed, so `CRW-07` has no inventory delta.
`CRW-08` remains active for redelegation and remaining DPoS method/failure families.

Validation passed the complete `rustaxa-consensus` test suite, the focused Rust V1/V2 lifecycle and snapshot tests, the
restart-backed Rust-enabled FinalChain fixtures, `rewrite-validate-fast`, the FinalChain Tier 2 gate, the Tier 3
Rust-enabled/pure-C++ parity gate, the bridge-inventory guard, Rust/C++ formatting, and whitespace checks. Gate output
contained only the repository's existing Clippy, CMake, Conan, and compiler warnings.

### CRW-08 Native DPoS Redelegation and Configured Correction Parity

This bounded follow-up closes normal `reDelegate(address,address,uint256)` behavior plus the configured historical
correction transcript rather than implementing only the post-fix happy path.

- Rust preserves the legacy validation order: same-validator after the fix block, Aspen-part-two zero amount, missing
  source validator, missing destination validator, enabled maximum stake, missing source delegation, amount greater
  than the delegation, and nonzero remainder below `minimum_deposit`. Expected failures are status-zero contract
  outcomes; inconsistent aggregate stake remains a hard invariant.
- Success claims source rewards and then existing-destination rewards, moves stake between delegation rows without an
  escrow or aggregate-delegated transfer, allows a new destination pair below the standalone delegation minimum,
  preserves the pre-Aspen zero-pair/index/cursor behavior, and emits reward logs before `Redelegated`. Empty-source
  deletion uses the combined V1/V2 pending count after Magnolia and the legacy pre-Magnolia rule before it.
- A zero configured validator maximum disables the ceiling. Before and at `fix_redelegate_block_num`, the native path
  reproduces the legacy same-validator stale validator writes, ordered vote deltas, and stale destination reward-record
  write that restores the pre-claim delegator reward pool. At the exact fix block, ordered
  configured redelegation corrections run after reward distribution and transaction effects but before the DPoS
  snapshot is encoded and published. They subtract only the configured amounts and refresh the validator's derived vote
  count while preserving the legacy global eligible-vote total; an additional same-validator call at the fix block may
  therefore leave the same unconfigured stake gap as legacy. Later same-validator calls fail normally.
- A repeated reward-bearing same-validator call after prior stale reference corruption depends on legacy
  validator/delegation reward-state reference counts and `LastUpdated` history that the current scalar Rust reward
  snapshot does not retain. Rust rejects that topology as a hard unsupported replay instead of publishing an approximate
  payout; modeling that reference graph remains active `CRW-08` work.
- Focused Rust and dual-mode FinalChain tests cover normal success, complete source deletion, all selected pre-mutation
  failures, enabled and disabled maximums, pre-Aspen zero and below-minimum new destination pairs, Aspen zero rejection,
  the fix boundary including a new unconfigured fix-block gap, exact duplicate reward-pool payouts, exact
  gas/log/bloom behavior, unchanged escrow/aggregate stake, and restart persistence.

`FinalChainRewardsConfig` and its CXX carrier now include `fix_redelegate_block_num` and ordered
`RedelegationCorrection` entries sourced from genesis hardfork configuration. This is a documented `CRW-07` carrier
field delta; no bridge handle, constructor, or standalone export was added. `CRW-08` remains active for the remaining
DPoS method and failure families.

### CRW-08 Persistent Same-Validator Reward-Corruption Guard

The reward-reference audit found that stake-gap inference alone did not protect the bounded redelegation path. A
pre-Aspen zero-amount same-validator call restores the stale reward pool and corrupts legacy `LastUpdated`/reference-count
state without changing validator stake, so a later reward-bearing call could evade the original guard.

- Rust records complete-history state plus the validator in a restart-durable corruption set after every successful
  same-validator call at or before `fix_redelegate_block_num`, including zero-amount and zero-pool calls. Before mutation,
  an incomplete older snapshot, that marker, or any validator-stake versus delegation-principal mismatch classifies the
  history as unsafe for scalar replay.
- A repeated marked-or-inferred call with a nonzero delegator reward pool hard-fails before account, cursor, stake, vote,
  reward, or log mutation. Repeated zero-pool calls remain allowed because the bounded configured-correction transcript
  can represent them exactly; post-fix same-validator calls remain normal status-zero contract failures.
- The DPoS snapshot appends a `(history_complete, marker_set)` state as item 22. Prior 5-through-21-item forms decode as
  history-incomplete, and that bit remains false when the snapshot is re-encoded, because a markerless zero-amount call
  cannot be inferred from scalar stake state. Reward-bearing pre-fix same-validator replay from such a database requires
  rebuild/replay rather than silent upgrade. Rollback to a binary that only accepts 21 items is unsafe after a new snapshot
  is finalized.
- Focused Rust coverage protects zero-amount first/repeated calls, pre-mutation failure state, encode/decode and restart
  persistence, older positive-gap inference, repeated zero-pool compatibility, and the post-fix boundary. The existing
  dual-mode correction fixture remains the honest C++ reference for the representable zero-pool transcript; the explicit
  safety abort is intentionally not described as general legacy replay parity.

This snapshot-only state adds no CXX carrier, handle, export, shim, or module flag, so `CRW-07` has no new inventory delta.
Full reward-version graph ownership and a production-history replay fixture remain active `CRW-08` work.

### CRW-08 Native DPoS Commission-Claim Terminal Lifecycle

This bounded follow-up closes the validator-deletion behavior that follows a successful
`claimCommissionRewards(address)` call.

- The owner and validator are validated before mutation. A metadata row without a matching total-stake row is treated
  as hard snapshot corruption, so Rust cannot pay from an orphaned validator record and then reinterpret the missing
  stake as zero.
- Rust pays the exact commission pool, emits `CommissionRewardsClaimed`, and zeros the pool before applying the legacy
  deletion boundary. A zero-stake validator is deleted before Magnolia regardless of pending requests; at Magnolia and
  later it is deleted only when the combined V1/V2 pending count is zero. This intentionally uses the corrected combined
  queue view already established by the V1 lifecycle slice.
- Pending V1/V2 requests and the V2 last-ID cursor survive validator deletion because they represent custody/history
  state. Validator metadata, order, VRF key, reward rows, delegation rows, vote state, and the Rust-only
  same-validator corruption marker are validator-owned and are removed, allowing a clean same-address registration
  after restart. A marker left without validator rows by an older binary is a hard snapshot inconsistency; operators
  must repair or migrate that state, or replay/rebuild from an unaffected point, before registration can proceed.
- The legacy Magnolia-to-Phalaenopsis interval sometimes left the zeroed reward row unpersisted while retaining a
  validator with pending undelegations. Rust native finalization does not yet carry the Phalaenopsis boundary, so this
  slice protects current-ABI lifecycle parity and pre-/post-Magnolia deletion semantics without claiming exact replay of
  that historical storage-write bug.
- Focused Rust tests cover terminal deletion, V1/V2 pending retention, the pre-Magnolia rule, owner and orphan-state
  failures, restart durability, and re-registration. A dual-mode `FinalChainTest.native_dpos_*` fixture protects exact
  action gas, receipt/log/bloom behavior, payout, and restart-backed retention while a V2 request remains pending. The
  pure-C++ reference keeps its observable pending counter after V2 confirmation in this path, while Rust intentionally
  uses the corrected combined live-queue view, so terminal deletion and fresh same-address registration are not claimed
  as a common dual-mode transcript.

No CXX carrier, handle, export, shim, module flag, or compatibility-only test surface changes, so `CRW-07` has no
inventory delta. `CRW-08` remains active for the remaining method/failure families and the explicit historical reward
reference graph.

### CRW-08 Native DPoS V2 Custody and Same-Block Gas Lifecycle

This bounded follow-up closes the pre-Magnolia terminal-validator boundary for
`undelegateV2(address,uint256)` and the staged claim-gas effect of V2 confirmation/cancellation.

- Successful V2 undelegation preserves legacy mutation order: validate, checkpoint/claim rewards, remove delegation and
  aggregate stake, apply the pre-Magnolia zero-stake/zero-commission validator deletion rule, then create the custody
  request and increment its per-delegator/validator ID. Deletion deliberately retains the V2 queue and last-ID cursor.
- The unlock block selects Cacti, Cornus, then base locking configuration according to the active hardfork. A retained
  request remains queryable and confirmable after pre-Magnolia registration deletion; confirmation removes the request,
  transfers its exact principal from DPoS escrow, and emits `UndelegateConfirmedV2`. Cancellation still fails when the
  validator registration is absent.
- At Magnolia and later, undelegation leaves the validator registered while custody is pending, and confirmation uses the
  corrected combined live V1/V2 queue guard already established by the V1 lifecycle work.
- The in-block DPoS view used for current-ABI claim-all gas now removes the selected V2 entry after successful
  confirmation. After successful V2 cancellation it restores the delegation membership and amount before removing the
  entry, so a later same-block `claimAllRewards()` charges for the restored validator. Normal transaction rollback keeps
  failed calls from updating this staged view.
- Focused Rust tests protect pre-Magnolia deletion with queue/ID snapshot round-trips and confirm/cancel gas-view
  transitions. Restart-backed dual-mode FinalChain fixtures protect exact gas, receipt/log/bloom behavior, explicit
  Cornus-over-base lock selection while Cacti is inactive, confirmation payout after validator deletion, same-block
  undelegate/cancel/claim-all behavior, and durable state.

The fixtures use `delegation_delay = 0`; the later live-membership claim-all slice below closes exact nonzero-delay gas
parity. They also avoid
the known legacy Magnolia persisted-counter divergence already documented by the commission-claim slice. No CXX handle,
carrier, export, shim, module flag, or compatibility-only test surface changes, so `CRW-07` has no inventory delta.
`CRW-08` remains active for the remaining method/failure families and the explicit historical reward reference graph.

Validation passed the complete `rustaxa-consensus` suite, focused Rust lifecycle tests, both focused Rust-enabled and
pure-C++ FinalChain fixtures, `rewrite-validate-fast`, the FinalChain Tier 2 gate, the Tier 3
Rust-enabled/pure-C++ parity gate, the bridge-inventory guard, formatting, and whitespace checks. Gate output contained
only the repository's existing Clippy, CMake, Conan, and compiler warnings.

### CRW-08 Phalaenopsis DPoS Escrow-Transfer Parity

This bounded follow-up restores the special account-only transfer action that is intentionally outside the Solidity
DPoS ABI.

- Calldata must equal exactly `0x44df8e70`. Before `phalaenopsis_hf_block_num`, and for the four-byte selector with any
  trailing data, the action remains an unknown method: a normal status-zero receipt charges intrinsic gas, advances a
  valid sender nonce, rolls back value, and emits no logs or bloom.
- At the activation block and later, the action charges 1,000 action gas and remains payable after Cornus because the
  legacy precompile recognizes and returns from this special branch before applying ABI-method nonpayability checks.
  Zero-value calls are likewise successful.
- Success reuses the native FinalChain contract-payment commit path to debit the sender and credit the DPoS account only
  after all gas, nonce, balance, and action checks succeed. The action returns no payload, emits no logs, and leaves the
  complete DPoS snapshot unchanged. Receipt, account snapshots, balances, nonce, block gas, and head publication remain
  atomic and restart-durable.
- Focused Rust coverage protects exact-input classification, fork-minus-one/fork/post-Cornus boundaries, action gas,
  payable handling, failure rollback, empty logs/bloom, balance movement, and persistence. A restart-backed dual-mode
  FinalChain fixture protects the same receipt, sender, escrow-value, nonce, DPoS-state, and restart transcript in
  Rust-enabled and pure-C++ modes while accounting explicitly for the modes' pre-existing difference in DPoS
  transaction-fee custody representation.

`FinalChainRewardsConfig` and its CXX carrier gain `phalaenopsis_period`, populated from genesis hardfork configuration.
This is a documented `CRW-07` carrier-field delta; no bridge handle, free export, constructor, shim, module flag, DPoS
snapshot field, or external-EVM responsibility is added. Existing Rust-finalized databases that encountered this selector
before the slice may contain status-zero receipts and missing escrow credits; recovery requires replay/rebuild from before
the first affected block or an explicitly designed migration, not an inferred balance top-up. `CRW-08` remains active
for the remaining DPoS method/failure families and the explicit historical reward reference graph.

Validation passed with the focused Rust classifier/gas test, all 701 `rustaxa-consensus` tests, the restart-backed selector
fixture in both Rust-enabled and pure-C++ FinalChain builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, and whitespace checks.

### CRW-08 Live Claim-All Gas with Nonzero Delegation Delay

This bounded follow-up separates mutation-gas membership from delayed eligibility views.

- Legacy current and pre-fix batch claim-all gas counts the caller's live DPoS contract memberships. Rust now initializes
  the staged claim-gas snapshot from the immediately preceding finalized DPoS state, matching the state against which the
  block's native mutations execute, instead of applying `delegation_delay` to that initial gas view.
- Successful same-block registration, delegation, undelegation, cancellation, confirmation, and redelegation updates
  continue to project into the staged gas view. Failed calls still leave it unchanged. Current and legacy batch
  claim-all therefore share the same live-membership source while retaining their existing paging and hardfork rules.
- Eligibility and historical authorization APIs continue selecting `period - delegation_delay`; the gas fix does not
  change validator eligibility, vote counts, PBFT authorization, or delayed FinalChain hash semantics.
- A restart-backed dual-mode FinalChain fixture uses delay two, creates a post-genesis delegation, and proves that the
  following current claim-all charges intrinsic gas plus one 45,000-gas item even though eligible votes still reflect
  genesis. Zero gas price isolates membership pricing from rewards and protects an empty log/bloom, sender nonce and
  balance, DPoS escrow, receipt RLP, live total stake, and restart durability.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or snapshot schema changes, so
`CRW-07` has no inventory delta. Existing Rust-finalized receipts may contain deficient gas/cumulative-gas values when a
claim-all call observed live memberships absent from its delayed snapshot; correction requires replay/rebuild from before
the first affected receipt rather than an in-place state mutation. `CRW-08` remains active for the remaining DPoS
method/failure families and the explicit historical reward reference graph.

Validation passed all 701 `rustaxa-consensus` tests, the focused restart-backed fixture in Rust-enabled and pure-C++
FinalChain builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the
bridge inventory guard, skill validation, the pre-commit hook, formatting, and whitespace checks.

### CRW-08 Slashing Proof Value Custody

This bounded follow-up restores the legacy value-bearing transcript for native
`commitDoubleVotingProof(bytes,bytes)` execution.

- Legacy EVM call handling transfers value into the callee before running a precompile and reverts that transfer on
  execution error. The slashing precompile does not apply the DPoS Cornus nonpayability guard, so a valid proof is
  payable in practice even though its ABI metadata says nonpayable.
- Rust now identifies the slashing contract as the deferred payment recipient for proof transactions. Gas is charged
  first, but sender value is debited and the slashing account is credited only after proof verification, duplicate
  detection, validator authorization, jail mutation, and log construction succeed. A failed proof therefore retains
  gas and nonce while rolling back value and slashing state.
- The first successful legacy jail write initializes the slashing account nonce. Rust mirrors that success-only rule by
  changing nonce zero to one without incrementing an already initialized account on later proofs or failures.
- A restart-backed dual-mode fixture executes a valid value-bearing proof followed by a duplicate value-bearing proof in
  the same block. It protects exact action and cumulative gas, receipt RLP, the `Jailed` log and block bloom, success-only
  sender/slashing balances, sender and contract nonces, Cacti eligibility removal, duplicate rollback, and restart
  persistence. Focused Rust tests separately protect successful custody, duplicate rollback, and invalid-proof rollback.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes, so
`CRW-07` has no inventory delta. Existing Rust-finalized blocks with valid value-bearing proofs contain a different
receipt, account state, and jail state; replay/rebuild from before the first affected proof or a separately designed
migration is required. The slice does not infer a slashing-account top-up. `CRW-08` remains active for native
precompile-read transactions, the remaining DPoS method/failure families, and the explicit historical reward reference
graph.

Validation passed the focused 25-test Rust slashing filter, all 703 `rustaxa-consensus` tests, the restart-backed fixture
in Rust-enabled and pure-C++ FinalChain builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, the pre-commit hook, formatting, and
whitespace checks. Existing repository clippy and CMake warnings remain unchanged.

### CRW-08 Native Slashing Read Transactions

This bounded follow-up extends the Rust-owned native slashing transaction surface to the two fixed-gas read selectors.

- At Magnolia and later, `getJailBlock(address)` and `getJailedValidators()` charge the legacy 5,000 action gas in
  addition to transaction intrinsic gas. Successful reads produce status-one receipts with no logs or bloom and do not
  mutate jail state. The transaction path evaluates the frozen delayed previously committed snapshot, so it does not
  expose a proof written earlier in the same block.
- Before Magnolia registers the precompile, calls to its future address retain ordinary empty-account behavior:
  intrinsic-only success, value transfer only when nonzero, and no persisted receiver account for a zero-value call.
- Legacy precompile execution transfers call value before running either read despite their ABI-view metadata. Rust
  therefore defers the sender debit and slashing-account credit until a recognized read succeeds. A malformed
  `getJailBlock` argument or insufficient action gas keeps the normal gas/nonce charge but rolls value back. The
  zero-argument jailed-validator selector continues accepting trailing calldata because the legacy method performs no
  argument unpack.
- Slashing storage initialization is write-specific. Successful reads leave a fresh slashing account nonce at zero and
  preserve any existing nonce; only a successful `commitDoubleVotingProof` jail write can initialize zero to one.
- A restart-backed dual-mode FinalChain fixture executes two value-bearing reads, a malformed value-bearing jail-block
  read, and a same-sender native continuation. It protects exact receipt RLP, action/cumulative/header gas, empty
  logs/bloom, successful-only value custody, sender/receiver balances and nonces, slashing nonce, and persisted receipts.
  Focused Rust coverage additionally protects selector decoding, malformed and out-of-gas rollback, trailing calldata,
  and the frozen read view.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes, so
`CRW-07` has no inventory delta. Existing Rust-finalized recognized read transactions may have status-zero,
intrinsic-only receipts and missing slashing-account custody. Because cumulative gas, sender balances, and later
affordability may differ, correction requires replay/rebuild from before the first affected transaction or a separately
designed migration rather than an inferred balance top-up. `CRW-08` remains active for DPoS precompile read
transactions, the remaining DPoS method/failure families, and the explicit historical reward reference graph.

Validation passed the focused 29-test Rust slashing filter, all 707 `rustaxa-consensus` tests, the restart-backed
fixture in Rust-enabled and pure-C++ FinalChain builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, the pre-commit hook, formatting,
and whitespace checks. Existing repository clippy and CMake warnings remain unchanged.

### CRW-08 Fixed-Gas DPoS Eligibility Reads

This bounded follow-up completes the native and direct-call surface for the three delayed eligibility queries.

- `isValidatorEligible(address)`, `getTotalEligibleVotesCount()`, and
  `getValidatorEligibleVotesCount(address)` now share the legacy 20,000 action-gas rule. The total-count direct call no
  longer uses the incorrect 22,000 estimate, and malformed recognized address inputs return contract errors after fixed
  gas instead of escaping as Rust execution failures.
- Direct calls and native transactions select the configured delayed snapshot and evaluate the threshold/vote schedule,
  Cacti activation, jail membership, and jail expiry at that delayed block. Native execution freezes the preceding-head
  view for the whole block, so an earlier registration, delegation, undelegation, or slashing proof cannot affect a later
  eligibility-read result in the same block.
- Before Cornus, legacy DPoS reads accept value in actual EVM execution; Rust commits that value to the DPoS account only
  after a recognized read succeeds. At Cornus and later, nonpayability is checked before ABI decoding: value-bearing
  reads consume intrinsic gas, advance the sender nonce, and roll value back. Malformed recognized reads retain 20,000
  action gas before Cornus, unknown selectors retain zero action gas, and action-out-of-gas calls commit only the normal
  gas/nonce effects.
- Legacy Cornus also advances the sender nonce when intrinsic gas is insufficient, while earlier periods leave it
  unchanged. The shared Rust native transaction state machine now applies that fork rule, and focused plus dual-mode
  coverage protects both sides with same-sender continuation.
- A restart-backed dual-mode FinalChain fixture exercises all three successful pre-Cornus reads, malformed and action-
  and intrinsic-out-of-gas calls, Cornus nonpayability, Cornus intrinsic-out-of-gas, and ordinary continuations. It
  protects exact receipt RLP, gas/cumulative/header gas, empty logs/bloom, sender/receiver balances and nonces, DPoS
  custody and nonce, and persisted receipts. Direct-call and focused Rust tests protect ABI return values, delayed
  threshold/vote/jail semantics, trailing calldata, and frozen same-block visibility.

Validation passed the three focused Rust eligibility tests, all 709 `rustaxa-consensus` tests, the restart-backed fixture
in Rust-enabled and pure-C++ builds, the corrected `RPCTest.eth_call` boundary, `rewrite-validate-fast`,
`rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, the
repository pre-commit hook, and whitespace validation.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes, so
`CRW-07` has no inventory delta. Historical Rust execution may contain unsupported or mispriced eligibility reads and
pre-correction Cornus intrinsic-out-of-gas nonces. Because receipts, cumulative gas, balances, nonces, and later
affordability may differ, correction requires replay/rebuild from the first affected transaction or a separately
designed migration rather than inferred account repairs. `CRW-08` remains active for remaining static and dynamic DPoS
read transactions, the remaining DPoS method/failure families, and the explicit historical reward reference graph.

### CRW-08 Fixed-Gas DPoS Singleton Reads

This bounded follow-up completes the native and direct-call surface for the two remaining fixed-5,000 singleton reads.

- `getValidator(address)` is available throughout history; `getUndelegationV2(address,address,uint64)` remains
  unsupported before Cornus and becomes active at the fork. Active valid, missing-record, and malformed calls retain the
  legacy 5,000 action-gas rule. Direct-call decode and expected missing-record failures become typed contract errors
  rather than escaping as Rust executor failures. Partially deleted validator rows, including a marker-only persisted
  same-validator corruption record, remain a hard snapshot inconsistency instead of being normalized as an absent
  validator or accepted by registration.
- Unlike eligibility queries, both methods read exact live block-local DPoS state. Native execution can therefore
  observe successful earlier same-block registration, metadata/commission, stake, V2 creation, cancel, or confirmation
  transitions. The V2 payload reports the current validator-existence flag alongside stake, unlock block, validator,
  and ID; `getValidator` preserves its dynamic strings and full validator tuple.
- Before Cornus, a successful validator read accepts legacy EVM call value into the DPoS account, while the unavailable
  V2 selector fails and rolls value back. At Cornus and later both methods reject value before decoding and charge only
  intrinsic gas. Action-out-of-gas calls charge intrinsic gas, intrinsic-out-of-gas follows the existing fork-specific
  sender-nonce rule, and read execution never emits logs, mutates DPoS state, or changes the DPoS account nonce.
- A restart-backed dual-mode fixture protects trailing validator calldata, malformed and missing validator reads,
  pre-Cornus V2 rejection, action/intrinsic out-of-gas, successful-only value custody, sender continuation, a Cornus
  `undelegateV2` followed by a successful same-block singleton read, a missing V2 ID, Cornus nonpayability, exact receipt
  RLP and cumulative/header gas, empty read logs/bloom, DPoS and receiver balances/nonces, and persisted receipts.

Validation passed both focused singleton-read Rust tests, all 711 `rustaxa-consensus` tests, the restart-backed fixture
in Rust-enabled and pure-C++ builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, the repository pre-commit hook,
changed-line C++ formatting, Rust formatting, and whitespace validation.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes, so
`CRW-07` has no inventory delta. Historical Rust execution may contain intrinsic-only failed receipts for these native
selectors and missing pre-Cornus validator-read value custody. Because receipt roots, cumulative gas, balances, later
affordability, and transaction ordering may differ, correction requires replay/rebuild from the first affected
transaction or a separately designed migration rather than inferred account repairs. `CRW-08` remains active for
dynamic validator pages, delegation and undelegation reads, remaining DPoS method/failure families, and the explicit
historical reward reference graph.

### CRW-08 Dynamic Validator Pages

This bounded follow-up executes the two validator-list page reads through native Rust finalization while retaining the
existing direct-call ABI surface.

- `getValidators(uint32)` charges 5,000 action gas for each returned validator, up to 20, and preserves the legacy
  single-read 5,000 charge for empty or out-of-range pages. `getValidatorsFor(address,uint32)` retains its fixed 100,000
  action-gas charge because legacy execution scans the validator list even when no validator matches the owner.
- Both selectors use the exact live block-local DPoS snapshot. Successful registration or terminal deletion earlier in
  the same block is therefore visible to output and gas calculation. They do not use the delayed eligibility snapshot
  or claim-all gas projection.
- Execution preserves legacy narrow ABI decoding, trailing calldata, and wrapping `uint32 batch * 20` offsets. Gas
  estimation intentionally widens the batch first, so an overflow batch can select a wrapped nonempty page while still
  receiving the one-read out-of-range `getValidators` action charge.
- Validator deletion now uses swap-remove ordering: the last validator replaces a deleted middle entry, matching the
  legacy iterable map and its persisted page order. Missing validator stake or metadata referenced by that order is a
  hard snapshot inconsistency rather than a silently filtered page entry.
- Before Cornus, successful value-bearing reads retain value in the DPoS account. At Cornus and later, value is rejected
  before ABI decoding with intrinsic gas only. Malformed recognized inputs have zero action gas, action out-of-gas
  charges intrinsic gas, and successful reads emit no logs or state changes.

Validation passed the three focused validator-page Rust tests, all 713 `rustaxa-consensus` tests, the restart-backed
fixture in Rust-enabled and pure-C++ builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, the repository pre-commit hook,
changed-line C++ formatting, Rust formatting, and whitespace validation.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes are
needed, so `CRW-07` has no inventory delta. Previously finalized Rust page transactions and snapshots containing
stable-shift order after non-tail deletion can differ in status, gas, receipt roots, value custody, balance, ordering,
and later affordability. Correction requires replay/rebuild from the first affected read or deletion, or a separately
designed migration; current snapshot order is insufficient to reconstruct the original legacy order reliably.
`CRW-08` remains active for delegation and undelegation reads, the remaining DPoS method/failure families, and the
explicit historical reward graph.

### CRW-08 Dynamic Undelegation Pages

This bounded follow-up executes the V1 and V2 undelegation-list page reads through native Rust finalization while
retaining the existing direct-call ABI surface.

- `getUndelegations(address,uint32)` charges 5,000 action gas for each returned request, up to 20, and preserves the
  legacy 5,000 minimum for empty and out-of-range pages. Cornus-gated `getUndelegationsV2(address,uint32)` charges two
  5,000-gas reads for each visited validator group plus two reads for each returned request.
- Both selectors use the exact live block-local DPoS snapshot. Successful create, cancel, or confirm transitions earlier
  in the same block are therefore visible to output and gas. They do not use delayed eligibility or claim-all gas views.
- Execution preserves legacy narrow ABI decoding, trailing calldata, wrapping `uint32 batch * 20` offsets, and iterable
  swap-remove ordering. Gas estimation intentionally widens the batch first. V2 output flattens validator-group order
  followed by per-validator request-ID order and includes each request ID; both versions report current validator
  existence from live stake membership.
- V1 is active throughout history, while V2 is unsupported before Cornus. Before Cornus, successful value-bearing V1
  reads retain value in DPoS custody. At Cornus and later both active selectors reject value before ABI decoding.
  Recognized malformed calls have zero action gas, action out-of-gas charges intrinsic gas, and successful reads emit no
  logs or state changes.
- Missing request or validator objects referenced by representable Rust queue indexes remain hard snapshot corruption
  rather than being silently filtered or normalized.

Validation passed both focused undelegation-page Rust tests, all 715 `rustaxa-consensus` tests, the restart-backed
fixture in Rust-enabled and pure-C++ builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, changed-line C++ formatting, Rust
formatting, and whitespace validation.

No CXX carrier, bridge handle/export, shim, module flag, compatibility-only surface, or DPoS snapshot schema changes are
needed, so `CRW-07` has no inventory delta. Previously finalized Rust native page transactions can differ in status,
gas, receipt roots, value custody, balances, ordering, and later affordability. Correction requires replay/rebuild from
the first affected transaction or a separately designed migration rather than an inferred balance or queue edit.
`CRW-08` remains active for delegation reads and their explicit historical reward reference graph, plus the remaining
DPoS method/failure families.

### CRW-08 Native Total Delegation Read

This bounded follow-up executes `getTotalDelegation(address)` through native Rust finalization while keeping paged
delegation rewards deferred to the historical reward-state reference graph.

- The output is the sum of principal for every validator in the delegator's authoritative membership order. It does not
  read validator reward pools, reward-per-stake state, or delegation reward cursors. Duplicate or dangling membership,
  principal wider than `uint256`, and sum overflow are hard snapshot corruption rather than filtered output.
- Action gas is exactly 5,000 per validator membership; an empty delegator costs zero action gas. Native execution uses
  transaction-point live state, so earlier same-block delegate, undelegate, cancel, redelegate, and terminal-validator
  transitions affect both total and gas. Direct calls retain requested-finalized-snapshot semantics.
- Legacy address decoding ignores the high twelve bytes and accepts trailing calldata. Malformed recognized input has
  zero action gas. Before Cornus a successful value-bearing read retains value in DPoS custody; Cornus rejects value
  before decoding. Successful reads emit no logs or state mutation, and normal action/intrinsic out-of-gas nonce rules
  remain shared with the native DPoS transaction state machine.
- The snapshot codec appends an independent `delegation_ledger_history_complete` bit as item 23. Genesis/current and
  schema-seven through schema-22 snapshots are complete. Direct schema-five/six snapshots remain incomplete across
  re-encoding and the read rejects them pending replay/rebuild. This marker is deliberately separate from the
  same-validator reward-corruption history marker.

Validation passed all three focused total-delegation Rust tests, all 718 `rustaxa-consensus` tests, the restart-backed
fixture in Rust-enabled and pure-C++ builds, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, Rust formatting, and whitespace
validation.

No CXX carrier, bridge handle/export, shim, module flag, or compatibility-only surface changes are needed, so `CRW-07`
has no bridge-inventory delta. The internal DPoS storage schema advances from 22 to 23 items. A schema-five/six snapshot
already rewritten as schema 22 by an older binary is indistinguishable and remains a documented migration limitation.
Previously finalized Rust native read transactions can differ in status, gas, receipt roots, value custody, balances,
and later affordability; correction requires replay/rebuild from the first affected read rather than an inferred
principal edit. `CRW-08` remains active for `getDelegations(address,uint32)`, its explicit historical reward-per-stake
reference graph, and the remaining DPoS method/failure families.

### CRW-08 Reward Reference Graph Foundation and Snapshot Integration

This bounded work adds a deterministic reward-reference graph to `rustaxa-consensus` and then integrates it into
FinalChain persistence and every supported DPoS reward-reference mutation.

- Reward nodes are keyed by validator and block and retain arbitrary-width reward-per-stake values plus the exact
  persisted reference count. Validator heads, delegation cursors, incomplete-history provenance, and permitted legacy
  stale heads are explicit graph state.
- Clone-staged mutations preserve legacy load-copy-write ordering. Same-key cursor writes can inflate counts and
  resurrect a deleted node; positive orphan counts remain representable and are never recomputed from live references.
  Checkpoint creation can atomically move existing heads and cursors without double-counting them.
- The canonical seven-field RLP codec has deterministic table ordering and rejects trailing bytes, non-list tables,
  duplicate or unsorted rows, noncanonical integers, dangling complete-history references, and undercounted nodes.
- Reward-per-stake and claim arithmetic uses exact `BigUint` intermediates. Principal, stake, and maximum inputs retain
  the `uint256` domain, while only the final ABI reward is reduced modulo 2^256.

- The DPoS snapshot advances from 23 to 24 items. Item 24 persists the canonical graph; every accepted older schema
  decodes with incomplete provenance and graph-dependent behavior fails closed pending replay/rebuild.
- Genesis and registration create exact nodes, validator heads, and delegation cursors. Transaction checkpoints move
  only the validator head, claims and stake mutations update their specific cursor, and reward distribution grows the
  pool without checkpointing. Terminal deletion keeps pre-Magnolia force-delete distinct from Magnolia decrement and
  orphan retention.
- Reward claims use graph nodes and arbitrary-width arithmetic as authority. Scalar reward-per-stake and cursor rows are
  retained only as derived compatibility state.
- Pre-fix same-validator redelegation reproduces fresh/repeated partial and full load-copy-write counts, stale live or
  missing heads, and the configured count-neutral correction conflict boundary.

Native and direct `getDelegations(address,uint32)` now route through this graph. The read preserves genesis validator
insertion order and swap-last removal order, uses wrapping `uint32` page offsets while retaining the legacy widened gas
calculation, and resolves rewards only for selected rows. Malformed input has zero action gas, low-width ABI decoding
and trailing calldata remain compatible, pre-Cornus value is retained on success, and Cornus rejects value before
decode. Duplicate or dangling membership, missing principal/validator rows, incomplete history, and missing graph
references fail hard; zero aggregate validator stake returns zero reward without consulting an older cursor. Rust unit
coverage proves native same-block membership visibility through read gas. Restart-backed Rust-enabled and pure-C++
coverage protects paging, ordering, decoder variants, corruption handling, and persistence. No CXX carrier, bridge
handle/export, shim, module flag, snapshot schema, or
`CRW-07` inventory delta belongs to the read integration.

Read integration validation passed the focused Rust page test, all 747 `rustaxa-consensus` tests, the focused
Rust-enabled and pure-C++ restart fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust and changed-line C++ formatting, whitespace validation, and independent configured review.

### CRW-08 Finalized DPoS Validator Owner-Update Parity

This bounded slice closes finalized native transaction parity for `setValidatorInfo(address,string,string)` and
`setCommission(address,uint16)`. Rust preserves legacy user-error ordering, exact byte/commission/frequency/delta
boundaries, fixed 20,000 action gas, pre-Cornus successful value custody, Cornus rejection before decode, exact success
logs/blooms, failure rollback, and restart persistence. Commission updates mutate the live block-local snapshot before
reward planning, so a successful update affects that block's minted reward split while a failed update preserves the
old split.

Metadata without canonical stake and a future persisted commission-change block now fail as hard snapshot corruption
after user-error precedence. Clean absence remains a normal contract failure. The restart-backed dual-mode fixture
protects ordered failures, gas, value, logs, persisted receipts, owner metadata, commission, and fork behavior; focused
Rust tests protect corruption ordering, exact frequency/delta boundaries, and same-block reward effects. No CXX carrier,
bridge handle/export, shim, module flag, snapshot schema, or `CRW-07` inventory delta is introduced.

All 25 current Solidity DPoS ABI methods now have Rust selector/decode and native apply/read routing. General DPoS
mutation simulation through `FinalChain::call` remains a cross-method `CRW-08` gap: the read-oriented Rust surface
recognizes mutation ABI/gas but does not execute business rules. It requires a shared ephemeral native executor rather
than setter-only simulation.

Owner-update validation passed the focused maximum-height frequency regression, all 753 `rustaxa-consensus` tests,
the focused Rust-enabled and pure-C++ restart fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust and changed-line C++ formatting, whitespace validation, the repository pre-commit hook, and
independent configured review.

### CRW-08 Shared DPoS Mutation Kernel Foundation

All finalized DPoS mutation dispatch now passes through one staged-snapshot kernel. The surrounding finalized executor
retains transaction-envelope and block responsibilities: gas, fees, nonces, payable-argument value injection, value
custody and rollback, receipts, reward planning, cleanup, and publication. The kernel owns deterministic DPoS
transition application over caller-owned account and DPoS maps. Focused registration/delegation success and failure
coverage protects value injection and failed-transition isolation; `FinalChain::call` remains unchanged in this
foundation slice.

Legacy dry-run parity requires an atomic follow-up across all 16 mutation branches. That transient envelope must use
exact requested-block snapshots, intrinsic plus action gas, combined gas-and-value affordability, staged precompile
value with rollback, typed business errors, mutation outputs, and logs, while excluding reward advancement, receipts,
end-block cleanup, and persistence. Historical blocks without complete Rust snapshots remain a separate replay,
migration, or explicitly retained hybrid-routing gap.

Kernel-foundation validation passed the focused register/delegate success and failure tests, the same-block
register-then-claim-all live-gas regression, all 757 `rustaxa-consensus` tests, `rewrite-validate-fast`,
`rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy
baseline, the bridge inventory guard, skill validation, Rust formatting, whitespace validation, and the repository
pre-commit hook, followed by independent configured review.

### CRW-08 DPoS Mutation Success-Output Preservation

The shared kernel result now retains the legacy ABI outputs that finalized receipt publication intentionally discards:
the `undelegateV2(address,uint256)` request ID and the pre-fix `claimAllRewards(uint32)` `bool is_end` result. The end
flag preserves legacy wrapping-`uint32` offsets and empty, exact-final, partial-final, non-final, and out-of-range
boundaries; current `claimAllRewards()` remains outputless. Established widened finalized page selection remains
unchanged, including no mutation for wrapping batches that are out of range on the native path; correcting selection
has separate historical replay consequences and stays pending. Finalized status, gas, logs, state, and receipt shape are
unchanged. Exact typed business errors remain the next kernel-result prerequisite for atomic mutation-call routing.

Output-preservation validation passed the focused V2 ID and claim-all end-flag boundary tests, all 758 `rustaxa-consensus`
tests, `rewrite-validate-fast`, `rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the unchanged
46-warning consensus clippy baseline, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook, followed by independent configured review approval.

### CRW-08 DPoS Typed Mutation Errors

The shared kernel outcome now carries exact legacy mutation business errors separately from successful ABI return
bytes. All reachable validation branches are typed, claim-all preserves validator context and sequential first-error
ordering without adding global-state work to canonical success, and registration proof recovery matches the pinned
no-CGO btcec compact-signature behavior, including dynamic recovery errors, zero-S and identity recovery, and a hard
failure for non-invertible R. Delegate checks preserve legacy maximum-before-minimum order. V2 aggregate-stake
underflow, reward-graph faults, account inconsistencies, and impossible arithmetic remain hard errors rather than
status-zero contract outcomes. ABI lookup failures for missing, unknown, retired, and non-exact inputs remain untyped;
only genuine named method rejections receive `Method not supported`. Finalized status, gas, logs, output publication,
and receipt shape remain unchanged for successful and ordinary typed business outcomes because that boundary
intentionally discards the typed reason. Documented V2 aggregate-stake underflow and non-invertible registration proof
recovery remain hard classifications that abort without publishing a receipt.

Typed-result validation passed eight focused error, ordering, recovery, dispatch, and rollback regressions, all 766
`rustaxa-consensus` tests, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust formatting, whitespace validation, and the repository pre-commit hook, followed by independent
configured review approval.

### CRW-08 Atomic DPoS Mutation Call Envelope

`FinalChain::call` now executes all 16 DPoS mutation selectors through the shared staged-snapshot kernel. The call
envelope clones exact requested-block account and DPoS snapshots, preserves legacy zero-sender exemptions and dry-run
nonce replacement semantics, checks full gas-cap affordability, validates intrinsic gas before combined value
affordability, charges method action gas, and stages payable value before contract execution. Cornus nonpayable
calls consume intrinsic gas only. Successful calls return kernel ABI output and transient logs; typed business failures
return exact legacy error text; snapshot corruption, reward-graph faults, and impossible arithmetic remain hard errors.
All staged account, value, reward, and DPoS changes are discarded for every outcome, with no receipt, bloom, reward
advancement, end-block cleanup, storage write, or publication.

The call outcome gained a documented log vector that reuses the existing bridge EVM-log DTO and maps into C++
`ExecutionResult::logs`; no handle, request field, module flag, or new export was added. The bridge fixture now proves a
successful transient delegation log and unchanged state. Historical blocks without complete Rust account and DPoS
snapshots fail closed, preserving the existing replay/migration or explicit hybrid-routing requirement. Coverage spans
all selectors, historical selection, both missing-snapshot cases, gas/value boundaries, intrinsic and action OOG,
typed failure, rollback, V2 and legacy/current claim-all outputs, and log conversion. The C++ bridge page fixture also
corrects its ABI dynamic-array offset base so it validates the existing encoder rather than reading tuple metadata.

Mutation-call validation passed all 768 `rustaxa-consensus` tests, all 56 `rust_consensus_tests` bridge cases,
`rewrite-validate-fast`, `rewrite-validate-final-chain`, the authorized Tier 3
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook with the unchanged warning baseline, followed by independent configured
review approval.

### CRW-08 Native DPoS V2 Pre-Mutation Failure Evidence

Cornus `undelegateV2(address,uint256)` now has explicit parity evidence for its four ordered pre-mutation business
failures without a production-code change. A missing validator returns `Validator does not exist`; an absent
caller/validator delegation returns `Delegation does not exist`; an amount above the delegation and a nonzero remainder
below `minimum_deposit` return `Insufficient delegation`. Each well-formed failure consumes calldata intrinsic gas plus
the fixed 60,000 action gas, advances the sender nonce, emits no logs, publishes a status-zero receipt and empty bloom,
and rolls back value, DPoS custody, delegation/stake/vote/reward state, V2 queues, and request IDs. Failure does not stop a
later same-sender transaction in the block.

One table-driven Rust preflight test protects exact typed errors and unchanged staged state, and one combined Rust
finalization test protects receipt status/gas, continuation, account effects, snapshot rollback, and header bloom. The
restart-backed dual-mode FinalChain fixture protects exact receipt RLP, cumulative/header gas, gas-only balances,
nonces, empty logs/blooms, unchanged DPoS and V2 request state, continuation, and restart persistence. No production,
bridge, carrier, handle, export, module flag, snapshot schema, migration, or `CRW-07` inventory change is required.

Validation passed both focused Rust tests, all 770 `rustaxa-consensus` tests, the focused Rust-enabled C++ fixture,
`rewrite-validate-fast`, `rewrite-validate-final-chain`, the authorized Tier 3
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook with the unchanged warning baseline, followed by independent configured
review.

### CRW-08 Native DPoS V2 Request-Consumption Failure Evidence

Cornus V2 confirmation and cancellation now have explicit failure-transcript parity evidence without production-code
changes. `confirmUndelegateV2(address,uint64)` and `cancelUndelegateV2(address,uint64)` first index a request by caller,
validator, and ID, making missing IDs and wrong callers the same `Undelegation does not exist` business failure.
Confirmation next rejects an unexpired request with `Undelegation is not yet ready to be withdrawn`; cancellation next
requires the validator and returns `Validator does not exist` when a pre-Magnolia full undelegation retained custody
history after terminal validator deletion. An unlocked confirmation deliberately remains valid without the validator.

The restart-backed dual-mode fixture creates request ID one while deleting its zero-stake validator, then executes
missing and locked confirmations, missing and validator-absent cancellations, and a later same-sender transfer. Confirm
failures consume calldata intrinsic gas plus 20,000 action gas; cancel failures consume intrinsic plus 60,000. All four
publish exact status-zero receipt RLP with empty logs/bloom, charge gas while advancing nonce, preserve escrow and the
request/last-ID state, and leave the validator absent. Block gas, cumulative gas, account state, request queries, and
every receipt remain identical after restart. A compact Rust test separately protects typed branch order, exact legacy
messages, wrong-caller behavior, and staged-state rollback. There is no production, bridge, carrier, handle, export,
module-flag, snapshot-schema, migration, or `CRW-07` inventory change. Rust-only duplicate-key corruption remains a
separate invariant-validation question because the legacy iterable map cannot represent it naturally.

Validation passed the focused Rust request-consumption test, all 771 `rustaxa-consensus` tests, the focused restart-backed
dual-mode C++ fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the Tier 3
`rewrite-validate-final-chain-parity` differential gate, `rewrite-bridge-inventory-guard`, skill validation, whitespace
validation, and the repository pre-commit hook. The configured `reviewer` returned `APPROVED` after the restart fixture
also reloaded and compared the exact block-two header, gas usage, and DPoS escrow balance.

### CRW-08 Native Slashing Semantic Invalid-Proof Evidence

The remaining well-formed semantic failures for native `commitDoubleVotingProof(bytes,bytes)` now have explicit parity
evidence without production-code changes. The matrix covers identical votes; period, round, and step mismatches; equal
block hashes with distinct unsigned vote hashes; both odd-next-step mixed zero/nonzero block-hash orientations; invalid
first and second signatures; different recovered validators; and a common valid signer absent from the delayed validator
view. The already established value-custody fixture retains duplicate-proof coverage.

Every selected post-Magnolia failure charges calldata intrinsic gas plus the fixed 20,000 action gas, advances the sender
nonce, rolls back nonzero call value, emits no logs, and leaves the proof set, jail facts, validator eligibility, DPoS
state, and slashing account unchanged. The restart-backed dual-mode transcript proves exact status-zero receipt RLP,
cumulative and header gas, empty blooms, gas-only sender debit, same-sender continuation, persisted accounts, and durable
receipts/header state. Rust-only tests protect the verifier branches and delayed validator-membership
selection. No production, bridge, carrier, handle, export, module-flag, snapshot-schema, migration, or `CRW-07` inventory
change is introduced.

This evidence deliberately does not claim exact diagnostic-text or combined-invalid precedence parity. Legacy checks a
stored duplicate before most semantic validation, while Rust verifies the proof before consulting stored state, and the
current receipt carrier does not publish either diagnostic. Malformed inner vote or sortition RLP is also excluded:
legacy uses a must-decode hard boundary while Rust currently normalizes that verifier failure to status zero. Those are
separate production-design questions rather than members of this ordinary semantic receipt family.

Validation passed both focused Rust slashing tests, all 773 `rustaxa-consensus` tests, the focused fixture in both
Rust-enabled and pure-C++ binaries, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the Tier 3
`rewrite-validate-final-chain-parity` differential gate, `rewrite-bridge-inventory-guard`, skill validation, whitespace
validation, and the repository pre-commit hook. The configured `reviewer` returned `APPROVED` for the final scoped diff.

### CRW-08 Malformed Nested Slashing-Proof Hard Boundary

Native `commitDoubleVotingProof(bytes,bytes)` now distinguishes malformed outer ABI framing from malformed nested
legacy RLP. Solidity selector/offset/length/tail failures remain ordinary status-zero contract execution. After the two
byte arguments decode, Rust requires exactly the legacy three-field `Vote` and four-field `VrfPbftSortition` shapes,
their fixed 65-byte signature and 80-byte proof, canonical full-value consumption, and no trailing bytes. This matches
the legacy Go `rlp.MustDecodeBytes` boundary: a nested decode failure aborts FinalChain before receipt, head, account,
DPoS, proof, or jail publication rather than continuing the block with a failed receipt.

The verifier exposes a typed `OuterAbi`, `NestedRlp`, or `Semantic` result. Native FinalChain maps outer and semantic
failures to the existing contract-failure outcome and propagates only nested RLP failures through the transaction-loop
hard-error boundary. Valid-RLP slot/hash/signature/signer/validator failures and successful proofs retain their existing
behavior. Focused Rust tables cover every nested shape and width class, including the legacy rejection of four-item
weighted vote rows, while a dual-mode FinalChain fixture proves the outer-failure control and atomic no-publication
behavior for malformed vote shape, sortition shape, proof width, and nested trailing bytes.

Duplicate-versus-semantic diagnostic precedence remains intentionally unchanged because diagnostics are not published
in receipts and both paths have identical status, gas, log/bloom, value rollback, and state results. No CXX carrier,
bridge handle/export, shim, module flag, compatibility-only surface, snapshot schema, migration, or `CRW-07` inventory
change is required.

Validation passed both focused Rust classification tables, all 775 `rustaxa-consensus` tests, the focused
Rust-enabled FinalChain fixture, the standalone pure-C++ legacy death test, `rewrite-validate-fast`,
`rewrite-validate-final-chain`, and the Tier 3 `rewrite-validate-final-chain-parity` differential gate. The standalone
reference test uses GoogleTest re-exec mode and constructs its database and FinalChain only inside the child, so the
legacy process-abort assertion does not fork live RocksDB or executor threads.

### CRW-08 Commission-Claim Failure Receipt Evidence

Finalized `claimCommissionRewards(address)` now has an exact dual-mode failure transcript for the two ordinary owner
validation branches already implemented in Rust. Missing validator metadata and a caller different from the registered
owner both return the typed `WrongOwnerAcc` contract failure before stake-row validation and leave staged account and
DPoS state unchanged. Zero-value Cornus calls are used deliberately: a positive value is rejected at the earlier
nonpayable boundary and would consume intrinsic gas only instead of exercising the fixed-20,000-gas owner checks.

The restart-backed fixture proves byte-identical status-zero receipt RLP, intrinsic plus action gas, cumulative/header
gas, empty logs/bloom, gas-only balances, nonce continuation, unchanged DPoS account/validator/stake/delegation state,
and durable receipts for missing-validator, wrong-owner, and selector-only malformed calls in Rust-enabled and pure-C++
modes. This is evidence-only production closeout: no bridge handle, carrier, export, shim, module flag, snapshot schema,
migration, or `CRW-07` inventory changes.

The audit leaves a separate generic pre-Cornus payable-envelope risk visible for a future bounded slice. Legacy reserves
the full gas cap and credits call value before DPoS execution; Rust currently checks value after used-gas charging and
applies commission-claim logic before crediting incoming value. Fully backed normal state is equivalent, but marginal
sender affordability and undercollateralized/corrupt DPoS state require an explicit parity decision before broader
pre-Cornus payable-envelope closeout.

Validation passed all six focused Rust commission-claim tests, all 776 `rustaxa-consensus` tests, the focused fixture in
Rust-enabled and pure-C++ binaries, `rewrite-validate-fast`, and the Tier 3
`rewrite-validate-final-chain-parity` target. Its broad Rust-enabled FinalChain phase retains the pre-existing unrelated
redelegation-correction dangling-cursor failure; the commission fixture and full pure-C++ suite pass.

### CRW-08 Native Payable-Contract Envelope Parity

Native DPoS and slashing transaction execution now preserves the legacy EVM payment envelope. Once intrinsic gas is
valid, combined affordability is measured against the complete gas-cap reservation plus call value. Marginally
underfunded calls consume the full limit, advance the nonce, emit a status-zero receipt, and skip contract execution.
For executable payable calls, Rust stages sender-to-contract value before the shared kernel so reward and custody checks
see the legacy balance; typed contract failure reverses only that payment, while successful execution retains it. The
existing block-local account and DPoS snapshots still provide hard-error atomicity, and the shared mutation kernel keeps
transaction-envelope concerns out of its interface.

The standalone dual-mode pre-Cornus `delegate(address)` transcript starts the sender at
`gas_limit * gas_price + value - 1`, then verifies the full-cap failure and a same-sender continuation across restart.
Together with existing payable-success and typed-failure rollback fixtures, it covers all three ordering branches:
marginal affordability, value-visible execution, and payment rollback. Focused Rust coverage additionally proves that
maximum call value produces the same normal failure without bounded-arithmetic overflow and that failure rollback
removes a payment recipient absent before staging. The slice changes no bridge, carrier, handle, export, shim, module
flag, snapshot schema, migration, or `CRW-07` inventory.

Validation passed all 777 `rustaxa-consensus` tests, focused Rust recipient-absence rollback, the standalone fixture in
Rust-enabled and pure-C++ modes, existing payable-success and typed-failure rollback fixtures,
`rewrite-validate-fast`, and the Tier 3 `rewrite-validate-final-chain-parity` gate. The known unrelated
redelegation-correction dangling-cursor failure remains confined to the broad Rust-enabled FinalChain run; the complete
pure-C++ suite and parity target pass.

### CRW-08 Cornus Underfunded-Gas Nonce Parity

Native FinalChain now applies the legacy full-gas-cap affordability decision before native precompile decoding and
state-dependent gas calculation. The comparison preserves legacy arbitrary-precision semantics: a `U256` product
overflow is necessarily unaffordable, not a hard finalization error. Failed transactions charge only the affordable gas
quotient, retain the gas-price remainder, transfer no value, emit no logs, and continue the block. Pre-Cornus and stale
transactions preserve the sender nonce; at Cornus and later, equal or skipped nonces advance to
`transaction_nonce + 1`. Newly materialized empty pre-Cornus senders are removed under EIP-161, while Cornus nonce
advancement makes the account durable.

The standalone dual-mode FinalChain fixture covers pre-Cornus, Cornus equal-nonce, and Cornus skipped-nonce senders with
exact receipt RLP, affordable gas, balances, absent receiver, and restart persistence. Rust tests extend the matrix to
stale nonces, overflowing gas-cap multiplication, absent accounts, malformed nested slashing calldata that must be
masked by the earlier affordability failure, and a later successful transaction proving continuation. This slice adds
no CXX carrier, bridge handle/export, shim, module flag, snapshot schema, migration, or `CRW-07` inventory delta. The
remaining `u64` nonce ceiling belongs to the broader FinalChain domain-type work tracked by `CRW-09`.

Foundation validation passed all 20 focused reward-graph tests and all 738 then-current `rustaxa-consensus` tests.
Integration validation passed 23 focused graph tests and all 746 `rustaxa-consensus` tests, including schema/restart,
corruption, checkpoint, reward-delta, terminal-deletion, same-block re-registration, and same-validator transcript
coverage. Normal-policy clippy retained the 46-warning baseline; `rewrite-validate-fast`,
`rewrite-validate-final-chain`, the bridge inventory guard, skill validator, repository pre-commit hook, Rust formatting,
whitespace validation, and independent configured review also passed.

## Historical Execution Order

This was the original consolidation sequence and is retained as implementation history. Do not use it to select current
work. Select the next `ready` item from the **Remaining Consensus Work Queue** in
`doc/consensus_rewrite_tracker.md`; that queue owns current dependencies, scope gates, and completion conditions.

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
