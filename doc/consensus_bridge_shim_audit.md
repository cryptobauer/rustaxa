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
| Application process host | `App`'s single Rust-mode consensus process shell | Exact timer/process and best-effort public-observer ports | Monotonic/Unix clocks, interruptible wait/stop mechanics, worker joining, and public event dispatch | Delete each leaf when the native runtime can own that physical operation; never expand it into manager orchestration. |
| Signing executor | App-owned node-wallet adapter | Exact digest-signing and VRF-proof requests/reports; no manager handle | Secret-key custody and signature execution | Keep only operation-shaped signing reports; native vote, pillar, slashing, and DAG-proposer tasks own selection and sequencing. |
| VDF executor | App-owned asynchronous `libraries/vdf` job adapter | Exact start, poll, and cancellation requests/reports | Proof work, job lifetime, and cancellation execution | Keep only the dedicated execution API; native proposer scheduling owns every decision around it. |
| Public submission clients | RPC and GraphQL mutation adapters | `BridgeConsensusApplication` public-transaction operation | Protocol formatting, error-text mapping, and best-effort event delivery | Keep the operation-shaped submission boundary; no transaction-manager handle or legacy object graph may reappear. |
| Public read clients | RPC, GraphQL, debug/Test RPC, and light-plugin adapters | `BridgeConsensusQueryApi` | Client formatting and protocol-specific response assembly | Keep only stable bounded public DTO reads; no consensus manager construction may reappear. |
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
| `bridge_lines` | 11334 |
| `shim_lines` | 2859 |
| `cxx_functions` | 195 |
| `cxx_carriers` | 154 |
| `cxx_handles` | 14 |
| `shim_directories` | 2 |
| `granular_flags` | 0 |
| `partial_service_factories` | 0 |
| `compatibility_constructor_calls` | 0 |
| `non_test_cpp_consumers` | 26 |

This DAG/transaction/proposer cut lowers the preceding 14,340/7,628/277/217/14/6/33 checkpoint by 2,224 bridge lines,
3,553 shim lines, 66 CXX functions, 47 carriers, zero opaque handles, three shim directories, and five non-test C++
consumers. Granular flags, partial-service factories, and compatibility-constructor calls remain zero. The deleted
compatibility family also includes three bridge modules and `transaction_manager_shim_test`.

The pillar cut lowers the 12,100/4,075/211/170/14/3/28 DAG checkpoint by 764 bridge lines, 1,216 shim lines,
16 CXX functions, 16 carriers, zero opaque handles, one shim directory, and two non-test C++ consumers. Granular flags,
partial-service factories, and compatibility-constructor calls remain zero. The deleted compatibility family also
includes the two pillar bridge modules and the bridge-only pillar-vote bundle test file containing two cases.

The Rust-mode manager facades and their PBFT, DAG, transaction, proposer, and pillar bridge/shim modules are deleted. `App` owns one
`ConsensusApplication` and a process-only shell containing one worker thread and the exact timer, signing, tarcap,
VDF, FinalChain account-fact, pillar-anchor-state, concrete EVM/gas, and public-observer ports. Daemon and proposer
scheduling, state progression, sync continuation, startup recovery, DAG/transaction and pillar admission, packing,
pillar finalization persistence/lifecycle, and finalization sequencing live in the native application root. Pillar
events cross the observer only after native durable acknowledgement. Public network, RPC, GraphQL, debug, and stats clients use
operation-shaped network, query, status, and transaction-submission APIs and cannot obtain a manager. Canonical bytes
cross CXX only at named physical transport, signing, VDF, fact-source, execution, public-event, and public-formatting
leaves; no manager task/action carrier remains supported. The master
`RUSTAXA_ENABLE` source selection preserves the untouched manager/runtime path for pure-C++ reference builds.
Scheduled transport rejection is retryable without advancing native broadcast counters, FinalChain account facts are
resolved lazily only for a published slashing conflict, and complete App process start/stop transitions are serialized.

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
| `rust/crates/rustaxa-bridge/src/application_host_ffi.rs` | Application-only CXX declarations and carriers for process, signing, VDF, transport, FinalChain facts, concrete EVM/gas, and public-observer leaves | App-owned consensus process | External boundary | Keep isolated from the aggregate leaf bridge; delete each callback when its concrete host executor or fact source moves native. |
| `rust/crates/rustaxa-bridge/src/consensus_host_ports.rs` | Exact process, signing, asynchronous VDF, tarcap, FinalChain account-fact, pillar-anchor-state, concrete EVM/gas, public-submission, and observer leaf conversion | App-owned consensus process and public mutation clients | External boundary | Keep only physical host/execution/public-client leaves; delete each adapter when that executor, fact source, or public client moves native. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | Sole application-root bootstrap plus operation-shaped public transaction submission | App bootstrap, RPC, and GraphQL mutations | Bootstrap/public-client adapter | Retain only root bootstrap, public submission/status conversion, and focused ABI coverage; native `ConsensusApplication` owns DAG, transaction, sortition, and proposer behavior and state. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and carriers | All C++ bridge clients | External boundary | Keep declarations and plain carriers only; delete each item with its last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | Root-bound FinalChain conversion and external-EVM execution APIs | FinalChain shim, execution adapters | External boundary | Native construction is complete; retain only public-query conversion and the narrow external-EVM executor API. |
| `rust/crates/rustaxa-bridge/src/network.rs` | Root-bound packet-family adapter for native PBFT, pillar-vote, DAG, DAG-sync, transaction, status, sync-response, and gossip pipelines | latest/v5 tarcap handler families | External boundary | Keep only canonical ingress requests, typed network decisions/reports, and tarcap transport execution; remove remaining handler-local consensus routing as `CRW-N01` completes. |
| `rust/crates/rustaxa-bridge/src/network_slashing.rs` | Exact signing and transaction-ingress conversion for network-detected slashing effects | tarcap ingress | External boundary | Delete when the signing and transaction-ingress executors move native; never expand into consensus routing. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi`, including coherent PBFT, period-indexed finalized pillar data, live DAG, transaction-pool, finalized-history, and public status views | RPC, GraphQL, debug/Test RPC, stats, light plugin | External boundary | Keep a bounded client-oriented read API; never expose private services, locks, queues, cursors, or mutable object graphs. |
| `rust/crates/rustaxa-bridge/src/storage.rs` | Root-bound typed compatibility queries and batch operations plus one named conformance seed hook | storage shim and storage conformance | Compatibility facade | Retire query families with their C++ materializers and the conformance seed with the differential CXX runner. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | Low-level VDF operations and cancellation used by the App-owned asynchronous VDF executor | VDF library adapter and application process host | External boundary | Keep the dedicated proof executor; do not recreate proposer scheduling or compatibility payload construction here. |

## Exported CXX Bridge Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, debug/Test RPC, stats, light plugin | External boundary | Keep only bounded client-oriented public reads. |
| `BridgeConsensusNetworkApi` | `network.rs` | One `Network` owner shared by latest/v5 tarcap handler families | External boundary | `CRW-N01` leaves canonical packet-family ingress and typed transport execution reports after all handler-local consensus routing moves native. |
| `BridgeConsensusApplication` | `dag_transaction_service.rs` | App bootstrap/process, query/network adapters, and RPC/GraphQL transaction submission | Bootstrap/application boundary | Keep as the sole opaque application root; it exposes operation-shaped tasks but no private consensus service handle. |
| `BridgeStorageQueries` | `storage.rs` | storage shim/query/conformance tests | Compatibility facade | Native domain/query fixtures replace the remaining legacy storage materializers. |
| `BridgeStorageBatch` | `storage.rs` | storage shim and tests | Compatibility facade | No named client requires legacy `DbStorage::Batch`. |
| `BridgeFinalChainExecutionSession` | `final_chain.rs` | FinalChain shim | External boundary | Replace with narrow external executor session or complete `CRW-E01`. |
| `BridgeConsensusExecutionApi` | `final_chain.rs` | FinalChain/PBFT execution adapters | External boundary | Keep only typed EVM/StateAPI leaf effects. |

## Consensus Shim Directories

| Shim directory | Current role | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `final_chain_shim` | FinalChain public/EVM executor facade | App, RPC, PBFT, transaction | External boundary | Split public query and narrow EVM executor; delete manager class when clients migrate. |
| `storage_shim` | broad `DbStorage` compatibility overlay plus stable sortition-change codec | App/admin/query/tests | Compatibility facade | Native bootstrap and narrow admin/query clients replace the broad facade; retain the codec only while the stable storage API exposes `SortitionParamsChange`. |

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
