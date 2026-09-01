# Consensus Bridge and Shim Inventory

This is the mechanically checked live inventory for the consensus rewrite boundaries defined in `PLAN.md`. It records
what exists, its current boundary, its named consumers, and its deletion condition. Implementation history belongs in
git; actionable work belongs in the normal issue/roadmap process.

## Classifications

| Classification | Meaning |
| --- | --- |
| External boundary | A named C++ transport, EVM, RPC, bootstrap, VDF, signing, admin, or conformance client remains. |
| Bootstrap/application boundary | The sole opaque application root or a plain configuration conversion used to create it. |
| Public-client boundary | A bounded query or mutation API used by a named client formatter. |
| Physical executor | C++ owns only an OS, cryptographic, transport, or concrete-state operation selected and validated by Rust. |
| Supported boundary | An opaque-handle factory exists only for one of the named external/bootstrap/public/executor clients below. |

Compatibility facades, manager/service locators, mutable object graphs, sidecars, partial-service factories, granular
flags, and shim directories are not supported classifications.

## Supported Rust-Mode C++ Clients

| Client family | Named clients | Retained C++ ownership | Narrowing or deletion condition |
| --- | --- | --- | --- |
| Application bootstrap/process host | `App`, Rust-mode fixtures, and `ConsensusApplication` host ports | Root construction, clocks, interruptible wait/stop, worker joining, and best-effort public event dispatch | Delete a leaf when native infrastructure owns that physical operation; never add manager orchestration or a service locator. |
| Tarcap transport | `Network`, latest/v5 packet handlers, and `TaraxaCapability` | Immutable peer/socket facts, packet sealing, physical send/disconnect, known-cache mutation, and lane scheduling | Retain only operation-shaped canonical ingress/egress and exact transport reports. |
| Concrete EVM/StateAPI | Application-owned `ExternalEvmStateOwner`; RPC/GraphQL state, debug trace, and light-prune callers | One bootstrap-created `StateAPI`, serialized concrete EVM/`state_db` execution, tracing, pruning, and descriptor access | Delete each exact operation when concrete EVM/`state_db` moves native; never expose StateAPI through CXX or recreate FinalChain/session/publication authority. |
| Signing | App-owned node-wallet adapter | Secret-key custody, digest signing, and VRF proof execution | Retain only exact requests/reports; Rust owns selection and sequencing. |
| VDF | App-owned asynchronous VDF adapter | Proof work, job lifetime, result construction, and cancellation | Delete when the physical VDF executor moves native; never recreate proposer scheduling. |
| Public submission | RPC and GraphQL mutation adapters | Protocol formatting, error-text mapping, and best-effort event delivery | Keep one operation-shaped application-root submission; no caller-supplied consensus/account facts. |
| Public reads | RPC, GraphQL, debug/Test RPC, stats, and light plugin | Client formatting and bounded response assembly | Keep bounded DTO reads; never expose private services, locks, queues, cursors, or mutable objects. |
| Admin/conformance | Light-history pruning and storage differential fixtures | Named root operation invocation and transcript formatting | Keep only exact admin tasks and the versioned production-root conformance transcript; never expose storage handles/batches. |
| Pure-C++ reference | All-Rust-disabled `cpp-reference` composition | Complete untouched legacy behavior | Retain while upstream synchronization requires the pure-C++ parity gate. |

## Checked Surface Budgets

The bridge inventory guard recomputes these values and rejects count or exact-set drift. Bridge lines count Rust source
under `rustaxa-bridge`; shim lines count consensus shim C/C++ source; non-test consumers are exact repository paths that
include a generated bridge header.

| Metric | Exact budget |
| --- | ---: |
| `bridge_lines` | 4965 |
| `shim_lines` | 0 |
| `cxx_functions` | 83 |
| `cxx_carriers` | 132 |
| `cxx_handles` | 10 |
| `shim_directories` | 0 |
| `granular_flags` | 0 |
| `partial_service_factories` | 0 |
| `compatibility_constructor_calls` | 0 |
| `non_test_cpp_consumers` | 17 |

The final concrete-state lifetime cut deleted the complete Rust-mode `FinalChain` overlay and the last shim directory.
One private `ExternalEvmStateOwner` holds StateAPI behind exact operations. Rust retains request identity, ordered native
and EVM execution validation, rewards planning, commit approval, crash recovery, and publication. Public transaction
submission derives account and finalized-location facts from one native head. No CXX StateAPI handle, execution session,
action loop, range executor, service locator, general storage handle, or C++ publication authority remains.

## CXX Box Factory Inventory

| Factory | Classification | Named client or owner | Delete or narrow when |
| --- | --- | --- | --- |
| `create_consensus_network_api` | Supported boundary | Tarcap transport | Delete when no C++ transport client remains. |
| `create_consensus_query_api` | Supported boundary | RPC, GraphQL, debug/Test RPC, stats, light plugin | Delete when public formatting no longer crosses CXX. |
| `create_consensus_application` | Supported boundary | `App` and Rust-mode fixture bootstrap | Delete when application bootstrap itself moves native. |
| `make_cancellation_token_with_atomic` | Supported boundary | VDF executor | Delete with the C++ VDF cancellation adapter. |
| `make_solution` | Supported boundary | VDF executor | Delete with C++ VDF result construction. |
| `make_vdf` | Supported boundary | VDF executor | Delete with the C++ VDF proof engine. |
| `prove` | Supported boundary | VDF executor | Delete with the C++ VDF proof engine. |

## Partial-Service Factory Inventory

| CXX factory | Compatibility constructor client path | Exact calls | Delete when |
| --- | --- | ---: | --- |

## Test-Only CXX Export Allowlist

| Export | Named test client | Removal condition |
| --- | --- | --- |
| `consensus_application_run_storage_conformance_v1` | `tests/storage_conformance/storage_conformance_runner.cpp`, `tests/rust/storage/test_storage.cpp` | Delete when the differential no longer requires a production-root transcript. |

## Rust Bridge Modules

| Module | Surface | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/application_host_ffi.rs` | Application-only process, signing, VDF, transport, concrete EVM/gas, and observer declarations | App-owned consensus process | External boundary | Delete each declaration with its physical host leaf. |
| `rust/crates/rustaxa-bridge/src/consensus_host_ports.rs` | Plain conversion for exact application host requests/reports | App process, transport, executor, and public-event adapters | External boundary | Keep conversion/error mapping only. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | Application bootstrap and public transaction submission | App, RPC, GraphQL, fixtures | Bootstrap/application boundary | Delete when bootstrap/submission no longer crosses CXX. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and plain carriers | All C++ bridge clients | External boundary | Delete each declaration with its exact last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | Plain bootstrap conversion for genesis/rewards/FinalChain configuration | Application construction | Bootstrap/application boundary | Delete when bootstrap accepts native configuration directly; no runtime state may enter this module. |
| `rust/crates/rustaxa-bridge/src/network.rs` | Canonical packet operations and typed transport effects/reports | Network/tarcap | External boundary | Keep canonical payloads, immutable peer facts, exact effects, and reports only. |
| `rust/crates/rustaxa-bridge/src/query.rs` | Bounded consensus query DTOs | Public-read clients | Public-client boundary | Delete a DTO with its last formatter; never expose private runtime state. |
| `rust/crates/rustaxa-bridge/src/storage_admin.rs` | Light prune and conformance root tasks | Light plugin and conformance fixtures | External boundary | Keep exact tasks only; never expose storage authority. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | Low-level VDF operations and cancellation | VDF adapter | Physical executor | Delete with the C++ VDF engine. |

## Exported CXX Opaque Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusApplication` | `dag_transaction_service.rs` | App bootstrap/process, network/query clients, RPC/GraphQL submission | Bootstrap/application boundary | Delete when application bootstrap and remaining root tasks no longer cross CXX. |
| `BridgeConsensusNetworkApi` | `network.rs` | `Network` and tarcap handler families | External boundary | Delete when physical transport no longer requires CXX. |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, debug/Test RPC, stats, light plugin | Public-client boundary | Delete when those client formatters move native. |
| `ConsensusProcessPort` | `application_host_ffi.rs` | App-owned consensus process shell | Physical executor | Delete when clocks/wait/stop/join/public observation move native. |
| `ConsensusSignerPort` | `application_host_ffi.rs` | App-owned wallet adapter | Physical executor | Delete when key custody/signing moves native. |
| `ConsensusTransportPort` | `application_host_ffi.rs` | App/tarcap transport adapter | Physical executor | Delete when packet transport moves native. |
| `ExternalEvmPort` | `application_host_ffi.rs` | Application-owned `ExternalEvmStateOwner` | Physical executor | Delete when concrete EVM/StateAPI moves native. |
| `WesolowskiVdf` | `ffi.rs` | `libraries/vdf` sortition adapter | Physical executor | Delete when VDF proof execution moves native. |
| `CancellationToken` | `ffi.rs` | `libraries/vdf` cancellation adapter | Physical executor | Delete with the C++ VDF job lifecycle. |
| `Solution` | `ffi.rs` | `libraries/vdf` result adapter | Physical executor | Delete with C++ VDF result materialization. |

## Non-Test C++ Bridge Consumers

| Consumer path | Named client family | Removal condition |
| --- | --- | --- |
| `libraries/core_libs/consensus/include/consensus/consensus_application.hpp` | Application bootstrap/process host | Delete include when no application-root/host handle crosses CXX. |
| `libraries/core_libs/consensus/include/consensus/external_evm_state_owner.hpp` | Concrete EVM/StateAPI | Delete include when concrete state execution moves native. |
| `libraries/core_libs/consensus/src/application/consensus_host_ports.cpp` | Application host, signing, transport, VDF, EVM, observer | Delete declarations with their exact physical leaves. |
| `libraries/core_libs/network/graphql/include/graphql/block.hpp` | Public reads | Delete include when GraphQL block formatting moves native. |
| `libraries/core_libs/network/graphql/include/graphql/query.hpp` | Public reads | Delete include when GraphQL query formatting moves native. |
| `libraries/core_libs/network/graphql/include/graphql/transaction.hpp` | Public reads/submission | Delete include when GraphQL transaction formatting/mutation moves native. |
| `libraries/core_libs/network/graphql/include/graphql/types/dag_block.hpp` | Public reads | Delete include when GraphQL DAG formatting moves native. |
| `libraries/core_libs/network/graphql/src/query.cpp` | Public reads | Delete include when GraphQL query assembly moves native. |
| `libraries/core_libs/network/include/network/consensus_query.hpp` | Public reads | Delete include when the C++ query adapter moves native. |
| `libraries/core_libs/network/include/network/ws_server.hpp` | Public submission/observation | Delete include when WebSocket mutation/event formatting moves native. |
| `libraries/core_libs/network/rpc/Debug.cpp` | Public reads/debug trace | Delete include when debug formatting/tracing moves native. |
| `libraries/core_libs/network/rpc/Taraxa.cpp` | Public reads | Delete include when Taraxa RPC formatting moves native. |
| `libraries/core_libs/network/rpc/Test.cpp` | Public reads/submission | Delete include when Test RPC formatting/mutation moves native. |
| `libraries/core_libs/network/rpc/eth/Eth.h` | Public reads/submission | Delete include when Ethereum RPC formatting/mutation moves native. |
| `libraries/core_libs/network/src/consensus_network_api.cpp` | Tarcap transport | Delete include when physical transport no longer crosses CXX. |
| `libraries/plugin/rpc/src/rpc.cpp` | Public reads/submission | Delete include when RPC plugin wiring moves native. |
| `libraries/vdf/src/sortition.cpp` | VDF | Delete include when VDF execution/result/cancellation moves native. |

## Consensus Shim Directories

| Shim directory | Current role | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |

## Guarded Exceptions

- `consensus_application_run_storage_conformance_v1` is test-only while differential storage conformance requires the
  versioned production-root transcript.
- The upstream-owned StateAPI declaration/implementation contains exact concrete-root marker, projection, commit,
  discard, descriptor, query, trace, and prune operations guarded by `RUSTAXA_ENABLE`. They are physical leaves and may
  not grow into a CXX handle, facade, session, or publication authority.
- The upstream-owned App startup implementation has one guarded timestamped paired-database backup/full-resync hook. It
  must not become in-place replay, synthetic-root rewriting, or C++ publication authority.
- Upstream-owned AppBase, RPC, GraphQL, network/tarcap, light-plugin, and test-util sites may contain only master-
  `RUSTAXA_ENABLE` routing to application/query clients or source-selected Rust handlers. Their legacy branches remain
  the pure-C++ implementation.
- Pure-C++ implementations/tests are not Rust-mode bridge consumers.

## Closeout Checks

```bash
scripts/rewrite_bridge_inventory_guard.sh --self-test
scripts/rewrite_bridge_inventory_guard.sh
scripts/rewrite_storage_boundary_guard.sh
! rg --files libraries/core_libs/consensus/shims
! rg -n 'consensus_network_queue_' rust libraries tests
! rg -n 'BridgeStorage' rust/crates/rustaxa-consensus
git diff --check
```

Every live handle, module, factory, shim directory, and non-test consumer must appear exactly once in the applicable
table. Delete its row in the same change that deletes the surface.
