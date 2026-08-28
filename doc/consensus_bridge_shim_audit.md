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
| Tarcap transport | `network::tarcap` packet handlers and `TaraxaCapability` | `BridgeConsensusNetworkApi` | Canonical peer snapshots, peer/socket mechanics, packet wrapping, send/gossip/disconnect execution, physical lane scheduling | Keep operation-shaped ingress/egress and transport execution calls; delete each remaining handler-local consensus decision relay as `CRW-N01` lands. Status/sync lifecycle state and decisions are already native-owned. |
| Concrete EVM/StateAPI executor | `FinalChain` overlay calling `StateAPI` and `state_db/` | Exact system-fact, ordered-transaction, rewards, and state-commit requests/reports | Concrete EVM calls, staged `state_db/` mutation, tracing, and raw executor operations | Keep only exact typed leaves until concrete EVM and `state_db` move native; no executor or session handle may reappear. |
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
| `bridge_lines` | 5535 |
| `shim_lines` | 1042 |
| `cxx_functions` | 96 |
| `cxx_carriers` | 141 |
| `cxx_handles` | 10 |
| `shim_directories` | 1 |
| `granular_flags` | 0 |
| `partial_service_factories` | 0 |
| `compatibility_constructor_calls` | 0 |
| `non_test_cpp_consumers` | 17 |

This DAG/transaction/proposer cut lowers the preceding 14,340/7,628/277/217/14/6/33 checkpoint by 2,224 bridge lines,
3,553 shim lines, 66 CXX functions, 47 carriers, zero opaque handles, three shim directories, and five non-test C++
consumers. Granular flags, partial-service factories, and compatibility-constructor calls remain zero. The deleted
compatibility family also includes three bridge modules and `transaction_manager_shim_test`.

The storage-facade cut lowers the preceding 11,334/2,859/195/154/14/2 checkpoint to
8,285/1,444/107/152/12/1. It deletes the `DbStorage` Rust overlay, `storage.rs`, two opaque handles, six query-family
factories, the bridge batch lifecycle, 88 CXX functions, compatibility materializers/mutexes, and all Rust-enabled App
ownership. Duplicate bridge storage-seeded query behavior tests move to the native query owner's coverage, removing the
last debug storage-owner escape hatch. The retained conformance and light-history operations are root tasks and do not
expose storage authority.

The application-root execution cut lowers that storage checkpoint to
5,547/1,042/96/144/10/1. It deletes `BridgeConsensusExecutionApi`,
`BridgeFinalChainExecutionSession`, both factories, the C++ action loop, execution/session carriers, C++ consensus
transaction/DAG materializers, and 2,784 lines of superseded bridge compatibility tests. Native
`ConsensusApplication` now owns system-transaction/rewards planning, ordered execution sequencing, canonical
result/receipt/root validation, pending-marker recovery ordering, state-commit approval, and FinalChain publication.
The retained StateAPI boundary is five exact typed request/report leaves: read-only committed-state preflight, system
facts, ordered concrete-EVM execution, rewards, and state commit. Native execution checks the concrete period and,
after genesis, root before mutation; committed reports carry the observed period/root and publication rejects a
mismatch. Canonical PeriodData codec components and the legacy verified
reward-vote weight sidecar cross only at the stable codec boundary.
The Rust overlay no longer declares `FinalChain::finalize` or its private `finalize_` compatibility stub. Rust-mode
fixtures invoke the exact application-root task API and expand canonical public results through `ConsensusQueryApi`,
whose block-index/count/receipt views preserve regular-then-system transaction order.
The private native session state machine is no longer re-exported: action/status constants, session/step types, and
transition functions cannot bypass the application coordinator.

The status-and-sync cut deletes six direct planner functions, their compatibility carriers/tests, and the
network-specific sync snapshot. Five operation-shaped calls replace them: one start-or-select bootstrap, periodic
follow-up, initial admission, native status egress, and one generation-correlated lifecycle command. Public and internal
reads share `BridgeConsensusQueryApi`; Rust mode no longer compiles or injects `PbftSyncingState`. The replacement lowers
bridge lines by 11 to 5,536 and carriers by one to 143 while keeping the other checked totals at 1,042/96/10/1; it eliminates the planner
family, duplicated mutable state, and network-only query route.

The canonical vote-family packet cut removes Rust-mode selection of `ExtVotesPacketHandler`,
`ExtPillarVotePacketHandler`, and `ExtSyncingPacketHandler`; those implementations now compile only in the untouched
all-Rust-disabled source selection. PBFT vote, optimized vote-bundle, pillar-vote, and optimized pillar-bundle ingress
cross CXX once as canonical packet bytes plus a peer/status snapshot. One transport executor owns lane serialization,
source-scoped drain, exact-id acknowledgement, dependency cancellation, packet wrapping, physical send/gossip,
disconnect/report, and known-cache execution. Tarcap contains no generated bridge-header include or raw
`BridgeConsensusNetworkApi` access. Obsolete scalar vote/pillar ingress carriers and exports plus the C++ proposed-block
publication leaf are deleted. Typed malformed-packet reports preserve peer-blacklist behavior without exposing raw
bridge errors. This lowers the checked surface to 5,535 bridge lines, 1,042 shim lines, 96 functions,
141 carriers, 10 handles, one shim directory, and 17 non-test C++ bridge consumers; all flag/factory/constructor metrics
remain zero.

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
| `create_consensus_network_api` | Supported boundary | tarcap transport | Keep only transport execution after `CRW-N01`. |
| `create_consensus_query_api` | Supported boundary | RPC, GraphQL, debug/Test RPC, light plugin | Keep only stable client-oriented public reads. |
| `create_consensus_application` | Supported boundary | `App` and Rust-mode fixture bootstrap | Sole native bootstrap for storage, FinalChain, and restored consensus services. |
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
| `consensus_application_run_storage_conformance_v1` | `tests/storage_conformance/storage_conformance_runner.cpp`, `tests/rust/storage/test_storage.cpp` | Retain only while the differential runner requires a versioned production-root transcript; never expose storage authority. |

## Rust Bridge Modules

| Module | Surface | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/application_host_ffi.rs` | Application-only CXX declarations and carriers for process, signing, VDF, transport, FinalChain facts, concrete EVM/gas, and public-observer leaves | App-owned consensus process | External boundary | Keep isolated from the aggregate leaf bridge; delete each callback when its concrete host executor or fact source moves native. |
| `rust/crates/rustaxa-bridge/src/consensus_host_ports.rs` | Exact process, signing, asynchronous VDF, tarcap, FinalChain account-fact, pillar-anchor-state, concrete EVM/gas, public-submission, and observer leaf conversion | App-owned consensus process and public mutation clients | External boundary | Keep only physical host/execution/public-client leaves; delete each adapter when that executor, fact source, or public client moves native. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | Sole application-root bootstrap plus operation-shaped public transaction submission | App bootstrap, RPC, and GraphQL mutations | Bootstrap/public-client adapter | Retain only root bootstrap, public submission/status conversion, and focused ABI coverage; native `ConsensusApplication` owns DAG, transaction, sortition, and proposer behavior and state. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and carriers | All C++ bridge clients | External boundary | Keep declarations and plain carriers only; delete each item with its last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | Root-bound FinalChain public/query conversion | FinalChain shim and public clients | External boundary | Retain only stable public-query conversion and recovery projection; execution orchestration and compatibility tests have moved native. |
| `rust/crates/rustaxa-bridge/src/network.rs` | Root-bound packet-family adapter for native PBFT, pillar-vote, DAG, DAG-sync, transaction, status, sync lifecycle/response, and prepared exact-target egress pipelines | latest/v5 tarcap handler families and application-root transport leaves | External boundary | Keep only canonical peer/payload requests, bounded preparation probes, immutable peer snapshots, typed network decisions/reports, and exact tarcap transport execution; status/sync lifecycle uses five operation-shaped calls and query snapshots live exclusively in `query.rs`. Remove the remaining scalar ingress decoding as `CRW-N01` completes. |
| `rust/crates/rustaxa-bridge/src/network_slashing.rs` | Exact signing and transaction-ingress conversion for network-detected slashing effects | tarcap ingress | External boundary | Delete when the signing and transaction-ingress executors move native; never expand into consensus routing. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi`, including coherent PBFT, period-indexed finalized pillar data, live DAG, transaction-pool, finalized-history, and public status views | RPC, GraphQL, debug/Test RPC, stats, light plugin | External boundary | Keep a bounded client-oriented read API; never expose private services, locks, queues, cursors, or mutable object graphs. |
| `rust/crates/rustaxa-bridge/src/storage_admin.rs` | Operation-shaped light-history prune and versioned conformance transcript adapters | application root, light plugin, storage conformance | Admin/conformance boundary | Keep only named root operations; never expose storage handles, query families, or caller-owned batches. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | Low-level VDF operations and cancellation used by the App-owned asynchronous VDF executor | VDF library adapter and application process host | External boundary | Keep the dedicated proof executor; do not recreate proposer scheduling or compatibility payload construction here. |

## Exported CXX Bridge Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, debug/Test RPC, stats, light plugin | External boundary | Keep only bounded client-oriented public reads. |
| `BridgeConsensusNetworkApi` | `network.rs` | One `Network` owner shared by latest/v5 tarcap handler families | External boundary | `CRW-N01` leaves canonical packet-family ingress and typed transport execution reports after all handler-local consensus routing moves native. |
| `BridgeConsensusApplication` | `dag_transaction_service.rs` | App bootstrap/process, query/network adapters, and RPC/GraphQL transaction submission | Bootstrap/application boundary | Keep as the sole opaque application root; it exposes operation-shaped tasks but no private consensus service handle. |

## Consensus Shim Directories

| Shim directory | Current role | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `final_chain_shim` | Legacy public FinalChain facade plus exact concrete-EVM/`state_db` leaves | App, RPC, public observers, and application-root execution | External boundary | Keep only named public-state, tracing, concrete EVM, rewards, and state-commit leaves; delete when public clients and the concrete executor migrate. |

## Guarded Exceptions

- `consensus_application_run_storage_conformance_v1` may remain test-only while differential storage conformance requires
  the versioned production-root transcript.
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
