# Consensus Bridge and Shim Inventory

This is the mechanically checked live inventory for the reduction plan in
`doc/consensus_consolidation_plan.md`. It records what exists, its current boundary, its named consumers, and its
deletion condition. Implementation history belongs in git and concise tracker closeout evidence.

## Classifications

| Classification | Meaning |
| --- | --- |
| External boundary | A named C++ transport, EVM, RPC, bootstrap, VDF, signing, admin, or conformance client remains. |
| Compatibility facade | A stable C++ API still has named callers; it is deleted when those callers migrate. |
| Native service wrapper | The handle currently exposes application state that must move out of `rustaxa-bridge`. |
| Internal bridge route | Rust behavior is bridge-shaped only because a C++ shim still drives it. |

## Supported Rust-Mode C++ Clients

These are the complete long-lived C++ client classes authorized by the aggressive cutover. A consumer named in a live
inventory row below is a migration/deletion client, not an additional compatibility promise. Application bootstrap,
manager-to-manager calls, lifecycle shells, admin construction, compatibility tests, and legacy object materializers
must migrate or disappear; they may not be promoted into this table merely to preserve the current topology.

| Client class | Named C++ clients | Retained boundary | C++ ownership | Narrowing or deletion condition |
| --- | --- | --- | --- | --- |
| Tarcap transport | `network::tarcap` packet handlers and `TaraxaCapability` | `BridgeConsensusNetworkApi` | Peer/socket mechanics, packet wrapping, send/gossip/disconnect execution, physical lane scheduling | Keep transport-only calls; delete all consensus admission, routing, queue, and effect-decision relays as `CRW-N01` lands. |
| Concrete EVM/StateAPI executor | `FinalChain` overlay calling `StateAPI` and `state_db/` | `BridgeConsensusExecutionApi`; temporary `BridgeFinalChainExecutionSession` | Concrete EVM calls, staged `state_db/` mutation, tracing, and raw executor operations | `CRW-E01` moves orchestration and canonical request/result authority native, then narrows the session to executor facts/results. |
| Signing executor | Wallet/node-secret call sites in DAG proposer, vote, pillar, and slashing transaction paths | Operation-specific signing inputs/results; no manager handle | Secret-key custody and signature execution | Delete compatibility key/materialization calls after native planners use a dedicated signing port. |
| VDF executor | `libraries/vdf` and DAG proposer VDF work execution | VDF operation functions | Proof work and cancellation execution | Keep only the dedicated execution API; delete proposer/manager lifecycle relays around it. |
| Public read clients | RPC, GraphQL, debug/Test RPC, and light-plugin adapters | `BridgeConsensusQueryApi` | Client formatting and protocol-specific response assembly | Migrate all manager/storage construction to client-oriented query DTOs, then keep only stable public reads. |
| Pure-C++ reference | All-Rust-disabled `cpp-reference` build | Untouched upstream implementations; no Rust bridge | Complete legacy behavior in reference mode only | Retain while upstream synchronization requires the pure-C++ validation gate. |

## Checked Surface Budgets

The bridge inventory guard recomputes these exact values. A deletion slice must lower every affected value in the same
change; an addition may not raise a value and must include compensating deletion plus an explicit tracker entry. “Bridge
lines” counts Rust source lines under `rustaxa-bridge`; “shim lines” counts C/C++ source/header lines in consensus shim
directories. CXX functions, carriers, and handles are parsed from the CXX module. Granular flags are
`RUSTAXA_ENABLE_*` CMake options, excluding the master bundle. Partial-service factories are the currently named
non-production application constructors; compatibility constructor call sites are their C++ shim invocations.
Non-test consumers are C/C++ files outside `tests/` that include the generated bridge header. The guard also rejects a
budget above any historical audit from the target-branch merge base through the local parent/worktree base. The target
comes from explicit `--base-ref`, `RUSTAXA_INVENTORY_BASE_REF`, `GITHUB_BASE_REF`, or `origin/main`, so the effective
ceiling is the minimum value previously reached and a multi-commit change cannot lower then re-raise a budget.

| Metric | Exact budget |
| --- | ---: |
| `bridge_lines` | 20394 |
| `shim_lines` | 7713 |
| `cxx_functions` | 347 |
| `cxx_carriers` | 288 |
| `cxx_handles` | 15 |
| `shim_directories` | 6 |
| `granular_flags` | 0 |
| `partial_service_factories` | 0 |
| `compatibility_constructor_calls` | 0 |
| `non_test_cpp_consumers` | 35 |

The Rust-mode `PbftManager` facade and its shim directory are deleted. `App` owns one `ConsensusApplication`, whose
private C++ runtime executes the retained timer, signing, tarcap, lifecycle, and concrete FinalChain/EVM leaves. Public
network, RPC, GraphQL, debug, and stats clients use operation-shaped network/query/status APIs and cannot obtain a
manager. Local generated-vote admission now commits its own-vote row in the same native batch, and proposal publication
derives identity from canonical signed bytes. The private runtime still consumes manager-shaped task/effect CXX
adapters; those adapters and its temporary object materialization are explicit `CRW-15`/`CRW-16` contraction debt, not
supported public compatibility surface.

PBFT finalization resume may dispatch `CommitRewardVotesResetRuntime` without
the fresh DAG/transaction lock pair only when the native manager retained a
nonzero reset generation matching the shared storage owner. The verified-vote
service still owns durable cursor/bundle validation and live publication; no
C++ reward mutation or report carrier is reintroduced. Other protected resume
actions remain fail-closed.

The PBFT finalization adapter now advances all seven retained external actions
through one cursor/action-gated CXX function. Rust dispatches to the typed
native leaf and consumes only the matching leaf payload; C++ retains concrete
FinalChain/pillar/period execution facts, transaction nonce facts, and DAG
compatibility effects. Six per-action exports and their duplicated bridge/shim
wrappers are deleted.

## CXX Box Factory Inventory

Every CXX function returning an owned opaque handle is classified here. `Supported boundary` is limited to the client
classes named above. `Production root debt`, `Partial service`, and `Compatibility facade` are all contraction targets,
not compatibility promises. The guard requires exact set equality with the parsed CXX module.

| Factory | Classification | Named client or owner | Delete or narrow when |
| --- | --- | --- | --- |
| `create_consensus_execution_api` | Supported boundary | FinalChain/StateAPI executor adapter | Keep only the narrow external-EVM executor API. |
| `create_consensus_network_api` | Supported boundary | tarcap transport | Keep only transport execution after `CRW-N01`. |
| `create_consensus_query_api` | Supported boundary | RPC, GraphQL, debug/Test RPC, light plugin | Keep only stable client-oriented public reads. |
| `create_consensus_application` | Supported boundary | `App` and Rust-mode fixture bootstrap | Sole native bootstrap for storage, FinalChain, and restored consensus services. |
| `create_final_chain_execution_session` | Supported boundary | FinalChain/StateAPI executor adapter | Narrow to concrete executor inputs/results during `CRW-E01`. |
| `create_dag_storage_queries` | Compatibility facade | storage shim | Native DAG/query fixtures replace it. |
| `create_final_chain_storage_queries` | Compatibility facade | storage shim | FinalChain/query APIs replace it. |
| `create_pbft_storage_queries` | Compatibility facade | storage shim | Native PBFT/query fixtures replace it. |
| `create_pbft_vote_storage_queries` | Compatibility facade | storage shim | Native vote/query fixtures replace it. |
| `create_period_storage_queries` | Compatibility facade | storage shim | PBFT/query APIs replace it. |
| `create_storage_shim_batch` | Compatibility facade | storage shim | No named client requires the legacy batch lifecycle. |
| `create_transaction_storage_queries` | Compatibility facade | storage shim | Native transaction/query fixtures replace it. |
| `make_cancellation_token_with_atomic` | Supported boundary | VDF executor | Keep only dedicated VDF cancellation execution. |
| `make_solution` | Supported boundary | VDF executor | Keep only dedicated proof-result construction. |
| `make_vdf` | Supported boundary | VDF executor | Keep only dedicated VDF execution. |
| `prove` | Supported boundary | VDF executor | Keep only dedicated VDF proof execution. |

## Partial-Service Factory Inventory

Every row is migration scaffolding, not a supported production composition. The guard requires the live partial-factory
set and its C++ compatibility-constructor call sites to match this table exactly. Client paths are repository-relative
and mechanically compared with bridge-shaped C++ call sites.

| CXX factory | Compatibility constructor client path | Exact calls | Delete when |
| --- | --- | ---: | --- |

## Test-Only CXX Export Allowlist

The guard classifies every CXX function by call-name use in production C++ and `tests/`. An export with no C++ caller
fails. An export used only from tests also fails unless it appears exactly once here.

| Export | Named test client | Removal condition |
| --- | --- | --- |
| `storage_shim_seed_final_chain_conformance_lookup_rows` | `tests/storage_conformance/storage_conformance_runner.cpp` | Delete when storage conformance seeds rows through native fixtures rather than the storage shim. |

## Rust Bridge Modules

| Module | Surface | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/dag.rs` | Proposer worker-command and legacy VDF-message CXX conversions | DAG manager/proposer shims | Executor conversion leaf | Delete when the C++ worker loop and VDF executor consume native commands and bytes without a standalone bridge module. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | Application-root bootstrap conversion and retained external leaf adapters over native `DagTransactionService` | App, DAG, transaction, gas, sortition shims | Bootstrap adapter | Retain only root bootstrap conversion, focused external-leaf/ABI tests, and EVM, signing, VDF-generation, and transport leaf execution; delete task relays as native application APIs absorb them. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and carriers | All C++ bridge clients | External boundary | Keep declarations and plain carriers only; delete each item with its last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | Root-bound FinalChain conversion and external-EVM execution APIs | FinalChain shim, execution adapters | External boundary | Native construction is complete; retain only public-query conversion and the narrow external-EVM executor API. |
| `rust/crates/rustaxa-bridge/src/network.rs` | Root-bound network adapter with native PBFT vote admission, transport-lane/source-partitioned effects, PBFT-sync response construction, and proposed-block publication | latest/v5 tarcap handler families | External boundary | Retain only tarcap transport execution and the typed slashing-transaction leaf while remaining DAG/status/transaction effects migrate. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | PBFT DTO/effect adapters over `BridgeConsensusApplication` | Private application runtime | Bootstrap task adapter | Retain pure FFI/error mapping only while exact timer, signing, transport, lifecycle, and execution ports replace the private compatibility executor. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | CXX conversion over native synced-period and cert-vote admission | Private application runtime | Internal bridge route | Native `PbftService` owns admission and persistence; delete carrier/status conversion when the exact sync task no longer pauses in the C++ executor. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_generation.rs` | DTO adapters over application-root PBFT generation and FinalChain-composed tasks | Private application signing leaf | Internal bridge route | Replace secret-bearing generation DTOs with an operation-specific signing request/result port. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_progress.rs` | progress adapter | Private application runtime | Internal bridge route | Fold the remaining executor conversion into native PBFT tasks. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | CXX conversion over native PBFT-root pillar tasks | pillar shim | Internal bridge route | Native `PbftService` owns current-anchor mutation/decisions, startup bootstrap, FinalChain-composed block planning, linkage, latest-finalized lookup, readiness, and access to private pillar state; retain only CXX conversion plus focused tag/status and FinalChain-handle sentinels. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | CXX conversion and FinalChain-handle unwrapping over native PBFT-root pillar-vote tasks | pillar/PBFT shims | Internal bridge route | Native `PbftService` owns admission, FinalChain composition, relevance, weighted bundles, direct PBFT-sync reporting, payload/network lookup, finalization prepare/ack, and behavioral tests; the standalone sync-bundle export/carrier are deleted, and remaining CXX conversion exists only for named C++ pillar clients. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | PBFT operation adapters over root `PbftService` proposal tasks | Private application transport/runtime leaves | Internal bridge route | Native state, storage, restoration, lock ownership, snapshotting, and standalone behavior live in `rustaxa-consensus`; retain canonical proposal publication only until transport consumes a native effect port. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi`, including live vote count and quorum reads | RPC, GraphQL, debug/Test RPC, stats, light plugin | External boundary | Keep a client-oriented read API; remove remaining manager/storage construction elsewhere. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | CXX configuration conversion for native `SortitionService` construction | DAG application bootstrap | Bootstrap adapter | Delete or inline the conversion when native application construction no longer accepts the legacy CXX configuration carrier. |
| `rust/crates/rustaxa-bridge/src/storage.rs` | Root-bound typed compatibility queries and batch operations plus one named conformance seed hook | storage shim and storage conformance | Compatibility facade | Retire query families with their C++ materializers and the conformance seed with the differential CXX runner. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | legacy transaction inspection | PBFT/transaction materializers | Internal bridge route | Use native codec internally; retain only if a named C++ client remains. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | DTO and report conversion over native transaction ownership; DAG-save, finalized-status, admission, read, packing, finalized filtering/verification, recovery, cache, sidecar-removal, and queue-finalization tasks call lock-owning native services directly | transaction/DAG/PBFT shims | Internal bridge route | Retain only submission/materialization conversion, the focused status-mapping ABI test, and unlocked EVM leaf adapters. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | VDF operations/cancellation | VDF and proposer C++ | External boundary | Keep until VDF execution is a native or dedicated external API. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | DTO/effect adapters over application-root vote tasks | PBFT signing, finalization, and slashing executor leaves | Internal bridge route | Fold remaining low-level generation/admission/own-vote calls into atomic application tasks, then retain only named leaf conversion. |

## Exported CXX Bridge Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, light plugin | External boundary | Keep only client-oriented public reads. |
| `BridgeConsensusNetworkApi` | `network.rs` | One `Network` owner shared by latest/v5 tarcap handler families | External boundary | `CRW-N01` leaves a transport-only API after remaining handler-local routing decisions move native. |
| `BridgeConsensusApplication` | `dag_transaction_service.rs` | App bootstrap and temporary PBFT/DAG manager task adapters | Bootstrap boundary | Keep as the sole opaque application root; narrow its task surface as manager facades move to named external adapters. |
| `BridgeDagStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native DAG/query fixtures replace it. |
| `BridgePbftStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native PBFT/query fixtures replace it. |
| `BridgePbftVoteStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native vote/query fixtures replace it. |
| `BridgeTransactionStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native transaction/query fixtures replace it. |
| `BridgeFinalChainStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | FinalChain/query APIs replace it. |
| `BridgePeriodStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | PBFT/query APIs replace it. |
| `BridgeStorageBatch` | `storage.rs` | storage shim and tests | Compatibility facade | No named client requires legacy `DbStorage::Batch`. |
| `BridgeFinalChainExecutionSession` | `final_chain.rs` | FinalChain shim | External boundary | Replace with narrow external executor session or complete `CRW-E01`. |
| `BridgeConsensusExecutionApi` | `final_chain.rs` | FinalChain/PBFT execution adapters | External boundary | Keep only typed EVM/StateAPI leaf effects. |

## Consensus Shim Directories

| Shim directory | Current role | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `dag_block_proposer_shim` | worker/VDF/signing/network executor facade | App/DAG lifecycle | Compatibility facade | Native DAG API owns orchestration; C++ leaf executors replace class. |
| `dag_manager_shim` | DAG manager/materialization facade | App, PBFT, network, tests | Compatibility facade | Callers use native service/query/transport APIs. |
| `final_chain_shim` | FinalChain public/EVM executor facade | App, RPC, PBFT, transaction | External boundary | Split public query and narrow EVM executor; delete manager class when clients migrate. |
| `pillar_chain_manager_shim` | pillar materialization/signing/network executor | App/PBFT pillar paths | Compatibility facade | Native pillar service plus leaf effects replaces class. |
| `storage_shim` | broad `DbStorage` compatibility overlay plus stable sortition-change codec | App/admin/query/tests | Compatibility facade | Native bootstrap and narrow admin/query clients replace the broad facade; retain the codec only while the stable storage API exposes `SortitionParamsChange`. |
| `transaction_manager_shim` | transaction materialization/submission/EVM facade | App, RPC, PBFT, DAG | Compatibility facade | Native service and leaf submission/EVM APIs replace class. |

## Guarded Exceptions

- `storage_shim_seed_final_chain_conformance_lookup_rows` may remain test-only while storage conformance requires it.
- Pure-C++ implementations and tests are not Rust-mode bridge consumers.
- A test-only export requires an explicit row here; ordinary native behavioral tests must not create CXX surface.

## Closeout Checks

Run after every bridge/shim slice:

```bash
scripts/rewrite_bridge_inventory_guard.sh
scripts/rewrite_bridge_inventory_guard.sh --self-test
scripts/rewrite_storage_boundary_guard.sh
rg -n 'Old::' libraries/core_libs/consensus/shims
rg -n 'consensus_network_queue_' rust libraries tests
rg -n 'BridgeStorage' rust/crates/rustaxa-consensus
git diff --check
```

Every live item must appear exactly once in the applicable table. Delete its row in the same change that deletes the
surface.
