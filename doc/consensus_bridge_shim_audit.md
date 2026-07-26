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
| Signing executor | `KeyManager` consumers in DAG proposer, vote, pillar, and slashing transaction paths | Operation-specific signing inputs/results; no manager handle | Secret-key custody and signature execution | Delete compatibility key/materialization calls after native planners use a dedicated signing port. |
| VDF executor | `libraries/vdf` and DAG proposer VDF work execution | VDF operation functions | Proof work and cancellation execution | Keep only the dedicated execution API; delete proposer/manager lifecycle relays around it. |
| Public read clients | RPC, GraphQL, debug/Test RPC, and light-plugin adapters | `BridgeConsensusQueryApi` | Client formatting and protocol-specific response assembly | Migrate all manager/storage construction to client-oriented query DTOs, then keep only stable public reads. |
| Pure-C++ reference | All-Rust-disabled `cpp-reference` build | Untouched upstream implementations; no Rust bridge | Complete legacy behavior in reference mode only | Retain while upstream synchronization requires the pure-C++ validation gate. |

## Checked Surface Budgets

The bridge inventory guard recomputes these exact values. A deletion slice must lower every affected value in the same
change; an addition may not raise a value and must include compensating deletion plus an explicit tracker entry. “Bridge
lines” counts Rust source lines under `rustaxa-bridge`; “shim lines” counts C/C++ source/header lines in consensus shim
directories. CXX functions, carriers, and handles are parsed from the CXX module. Granular flags are
`RUSTAXA_ENABLE_*` CMake options, excluding the master bundle. Partial-service factories are the four currently named
non-production application constructors; compatibility constructor call sites are their C++ shim invocations.
Non-test consumers are C/C++ files outside `tests/` that include the generated bridge header. The guard also rejects a
budget above any historical audit from the target-branch merge base through the local parent/worktree base. The target
comes from explicit `--base-ref`, `RUSTAXA_INVENTORY_BASE_REF`, `GITHUB_BASE_REF`, or `origin/main`, so the effective
ceiling is the minimum value previously reached and a multi-commit change cannot lower then re-raise a budget.

| Metric | Exact budget |
| --- | ---: |
| `bridge_lines` | 50400 |
| `shim_lines` | 19148 |
| `cxx_functions` | 420 |
| `cxx_carriers` | 359 |
| `cxx_handles` | 20 |
| `shim_directories` | 15 |
| `granular_flags` | 9 |
| `partial_service_factories` | 2 |
| `compatibility_constructor_calls` | 2 |
| `non_test_cpp_consumers` | 43 |

## CXX Box Factory Inventory

Every CXX function returning an owned opaque handle is classified here. `Supported boundary` is limited to the client
classes named above. `Production root debt`, `Partial service`, and `Compatibility facade` are all contraction targets,
not compatibility promises. The guard requires exact set equality with the parsed CXX module.

| Factory | Classification | Named client or owner | Delete or narrow when |
| --- | --- | --- | --- |
| `create_consensus_execution_api` | Supported boundary | FinalChain/StateAPI executor adapter | Keep only the narrow external-EVM executor API. |
| `create_consensus_network_api` | Supported boundary | tarcap transport | Keep only transport execution after `CRW-N01`. |
| `create_consensus_query_api` | Supported boundary | RPC, GraphQL, debug/Test RPC, light plugin | Keep only stable client-oriented public reads. |
| `create_dag_transaction_service_from_storage` | Production root debt | `App` bootstrap | Native application construction replaces the bridge-owned service. |
| `create_dag_transaction_service_for_transaction_manager` | Partial service | `transaction_manager_shim.cpp` | Delete with transaction-only compatibility construction. |
| `create_final_chain_execution_session` | Supported boundary | FinalChain/StateAPI executor adapter | Narrow to concrete executor inputs/results during `CRW-E01`. |
| `create_final_chain_with_rewards_config` | Production root debt | `App`/FinalChain bootstrap | Native application owns FinalChain construction and passes only an executor adapter. |
| `create_pbft_chain_service_from_storage` | Partial service | `pbft_chain_shim.cpp` | Delete with chain-only compatibility construction. |
| `create_pbft_service_from_storage` | Production root debt | `App` bootstrap | Native application construction replaces the bridge-owned service. |
| `create_storage` | Production root debt | `DbStorage`/`App` bootstrap | Native application storage construction replaces broad C++ bootstrap ownership. |
| `create_dag_storage_queries` | Compatibility facade | storage shim | Native DAG/query fixtures replace it. |
| `create_final_chain_storage_queries` | Compatibility facade | storage shim | FinalChain/query APIs replace it. |
| `create_metadata_storage_queries` | Compatibility facade | storage shim | Native metadata/query fixtures replace it. |
| `create_pbft_storage_queries` | Compatibility facade | storage shim | Native PBFT/query fixtures replace it. |
| `create_pbft_vote_storage_queries` | Compatibility facade | storage shim | Native vote/query fixtures replace it. |
| `create_period_storage_queries` | Compatibility facade | storage shim | PBFT/query APIs replace it. |
| `create_pillar_chain_storage` | Compatibility facade | storage shim | Native pillar APIs replace it. |
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
| `create_dag_transaction_service_for_transaction_manager` | `libraries/core_libs/consensus/shims/transaction_manager_shim/src/transaction_manager_shim.cpp` | 1 | Native fixtures replace transaction-only construction. |
| `create_pbft_chain_service_from_storage` | `libraries/core_libs/consensus/shims/pbft_chain_shim/src/pbft_chain_shim.cpp` | 1 | Network and public readers use native service/query APIs. |

## Test-Only CXX Export Allowlist

The guard classifies every CXX function by call-name use in production C++ and `tests/`. An export with no C++ caller
fails. An export used only from tests also fails unless it appears exactly once here.

| Export | Named test client | Removal condition |
| --- | --- | --- |
| `storage_shim_seed_final_chain_conformance_lookup_rows` | `tests/storage_conformance/storage_conformance_runner.cpp` | Delete when storage conformance seeds rows through native fixtures rather than the storage shim. |

## Rust Bridge Modules

| Module | Surface | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/dag.rs` | DAG commands, sessions, conversions | DAG manager/proposer shims | Internal bridge route | Native DAG service owns behavior and shims use leaf executor effects only. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | `BridgeDagTransactionService` | App, DAG, transaction, gas, sortition shims | Native service wrapper | Move service implementation and behavioral tests to a native application/runtime crate. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and carriers | All C++ bridge clients | External boundary | Keep declarations and plain carriers only; delete each item with its last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | FinalChain and execution APIs | FinalChain shim, execution adapters | External boundary | Split native ownership, public query, and a narrow external-EVM executor API. |
| `rust/crates/rustaxa-bridge/src/network.rs` | ingress, planning, effects | tarcap handlers | External boundary | Complete `CRW-N01`; retain transport-only execution API. |
| `rust/crates/rustaxa-bridge/src/pbft_chain.rs` | Thin DTO adapters over native `PbftChainService` plus chain-only compatibility construction | PBFT chain/manager shims | Internal bridge route | Native storage, restoration, lock ownership, transitions, validation, and lookup live in `rustaxa-consensus`; migrate named C++ readers and delete the facade. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | `BridgePbftService`, manager runtime | App, PBFT/vote/pillar shims | Native service wrapper | Move application service and behavioral tests out of bridge. The production root no longer owns duplicate storage or lifecycle atomic state; manager/native sibling owners retain their exact storage and native `PbftServiceReadiness` instances own monotonic PBFT and pillar bootstrap publication. |
| `rust/crates/rustaxa-bridge/src/pbft_period_cleanup.rs` | combined cleanup operation | PBFT manager shim | Internal bridge route | Native PBFT service owns operation without CXX-shaped API. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | sync admission/runtime methods | PBFT manager and tarcap | Internal bridge route | Fold into native PBFT/network pipeline APIs. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_generation.rs` | vote generation adapters | Vote/PBFT shims | Internal bridge route | Signing becomes a leaf port and vote behavior remains native. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_ingress.rs` | ingress adapter | network/vote path | Internal bridge route | Complete network pipeline migration. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_payload.rs` | payload conversion | PBFT/vote shims | Internal bridge route | Call native codecs directly or keep conversion private to FFI. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_progress.rs` | progress adapter | PBFT/vote shims | Internal bridge route | Fold into native PBFT service. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_validation.rs` | validation conversion | PBFT/vote shims | Internal bridge route | Fold into native PBFT service. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | pillar state/storage adapters | pillar and storage shims | Internal bridge route | Native PBFT/pillar owner replaces partial compatibility services. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | pillar vote planning/runtime | pillar/PBFT shims | Internal bridge route | Move behavior/tests native; retain signing/network leaf effects only. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | Thin DTO adapters over native `ProposedBlocksService`; stateless storage and temporary-candidate compatibility helpers | proposed-block/PBFT shims | Internal bridge route | Native state, storage, restoration, lock ownership, and standalone behavior moved to `rustaxa-consensus`; cross-domain leader selection and combined cleanup remain PBFT-owner debt; delete the C++ facade under `CRW-14`. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi` | RPC, GraphQL, light plugin | External boundary | Keep a client-oriented read API; remove manager/storage construction elsewhere. |
| `rust/crates/rustaxa-bridge/src/slashing.rs` | DTO adapters over native `SlashingProofService` | slashing/vote shims | Internal bridge route | Native service owns planner configuration, duplicate cache, and mutex; retain only the transaction-executor conversion boundary until the C++ slashing facade contracts to signing/submission effects. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | DAG-service sortition methods | sortition/DAG/PBFT shims | Internal bridge route | Delete C++ sortition facade after callers migrate. |
| `rust/crates/rustaxa-bridge/src/storage.rs` | storage facade, queries, batch | storage shim, conformance, bootstrap | Compatibility facade | Native bootstrap/query/test fixtures replace broad storage handles. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | legacy transaction inspection | PBFT/transaction materializers | Internal bridge route | Use native codec internally; retain only if a named C++ client remains. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | transaction runtime and adapters | transaction/DAG/PBFT shims | Internal bridge route | Move runtime/tests native and reduce C++ to submission/EVM leaf adapters. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | VDF operations/cancellation | VDF and proposer C++ | External boundary | Keep until VDF execution is a native or dedicated external API. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | DTO/effect adapters over native `PbftVerifiedVotesService`, plus cross-domain FinalChain and leader/finalization composition | verified-votes/vote/PBFT shims | Internal bridge route | Native storage lifetime, restoration, and vote-runtime lock ownership live in `rustaxa-consensus`; move the remaining cross-domain workflows into the native PBFT owner and delete the materialization facade. |

## Exported CXX Bridge Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, light plugin | External boundary | Keep only client-oriented public reads. |
| `BridgeConsensusNetworkApi` | `network.rs` | tarcap handlers | External boundary | `CRW-N01` leaves a transport-only API. |
| `BridgeDagTransactionService` | `dag_transaction_service.rs` | App and DAG/transaction/sortition/gas shims | Native service wrapper | Native application owner replaces bridge-owned implementation. |
| `BridgePbftService` | `pbft_manager.rs` | App and PBFT/vote/pillar shims | Native service wrapper | Native application owner replaces bridge-owned implementation. |
| `BridgePillarChainStorage` | `pillar_chain.rs` | storage shim | Compatibility facade | Replace remaining pillar storage compatibility calls. |
| `BridgeStorage` | `storage.rs` | storage shim, bootstrap, tests | Compatibility facade | Native construction and narrow query/admin APIs replace it. |
| `BridgeDagStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native DAG/query fixtures replace it. |
| `BridgeMetadataStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native metadata/query fixtures replace it. |
| `BridgePbftStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native PBFT/query fixtures replace it. |
| `BridgePbftVoteStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native vote/query fixtures replace it. |
| `BridgeTransactionStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native transaction/query fixtures replace it. |
| `BridgeFinalChainStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | FinalChain/query APIs replace it. |
| `BridgePeriodStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | PBFT/query APIs replace it. |
| `BridgeStorageBatch` | `storage.rs` | storage shim and tests | Compatibility facade | No named client requires legacy `DbStorage::Batch`. |
| `BridgeFinalChain` | `final_chain.rs` | FinalChain shim, query/execution adapters | External boundary | Split native owner from public query and EVM executor APIs. |
| `BridgeFinalChainExecutionSession` | `final_chain.rs` | FinalChain shim | External boundary | Replace with narrow external executor session or complete `CRW-E01`. |
| `BridgeConsensusExecutionApi` | `final_chain.rs` | FinalChain/PBFT execution adapters | External boundary | Keep only typed EVM/StateAPI leaf effects. |

## Consensus Shim Directories

| Shim directory | Current role | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `dag_block_proposer_shim` | worker/VDF/signing/network executor facade | App/DAG lifecycle | Compatibility facade | Native DAG API owns orchestration; C++ leaf executors replace class. |
| `dag_manager_shim` | DAG manager/materialization facade | App, PBFT, network, tests | Compatibility facade | Callers use native service/query/transport APIs. |
| `final_chain_shim` | FinalChain public/EVM executor facade | App, RPC, PBFT, transaction | External boundary | Split public query and narrow EVM executor; delete manager class when clients migrate. |
| `gas_pricer_shim` | gas-price compatibility view | transaction/RPC | Compatibility facade | Clients use native transaction query API. |
| `key_manager_shim` | signing-key fact adapter | proposer/vote/pillar paths | External boundary | Signing/key ownership receives a dedicated port. |
| `pbft_chain_shim` | PBFT chain view/materializer | network, RPC, PBFT/DAG/vote | Compatibility facade | Clients use PBFT application/query APIs. |
| `pbft_manager_shim` | lifecycle and multi-effect executor facade | App consensus loop | Internal bridge route | Native app owns orchestration; transport/EVM/timer/signing remain leaf adapters. |
| `pillar_chain_manager_shim` | pillar materialization/signing/network executor | App/PBFT pillar paths | Compatibility facade | Native pillar service plus leaf effects replaces class. |
| `proposed_blocks_shim` | materialized proposed-block view | PBFT/sync compatibility | Compatibility facade | Named callers use PBFT service/query views. |
| `slashing_manager_shim` | slashing transaction executor | vote/slashing paths | Compatibility facade | Native plan plus signer/submission leaf ports replaces class. |
| `sortition_params_manager_shim` | sortition compatibility facade | DAG/PBFT | Compatibility facade | Named callers use DAG service/query operations. |
| `storage_shim` | broad `DbStorage` compatibility overlay | App/admin/query/tests | Compatibility facade | Native bootstrap and narrow admin/query clients replace broad facade. |
| `transaction_manager_shim` | transaction materialization/submission/EVM facade | App, RPC, PBFT, DAG | Compatibility facade | Native service and leaf submission/EVM APIs replace class. |
| `verified_votes_shim` | materialized verified-vote view | VoteManager/PBFT/network | Compatibility facade | Named callers use PBFT service/transport views. |
| `vote_manager_shim` | vote runtime/materialization/network facade | PBFT, DAG, network | Internal bridge route | Native PBFT vote service and network pipeline replace class. |

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
