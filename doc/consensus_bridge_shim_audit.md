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

## Rust Bridge Modules

| Module | Surface | Named consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `rust/crates/rustaxa-bridge/src/dag.rs` | DAG commands, sessions, conversions | DAG manager/proposer shims | Internal bridge route | Native DAG service owns behavior and shims use leaf executor effects only. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | `BridgeDagTransactionService` | App, DAG, transaction, gas, sortition shims | Native service wrapper | Move service implementation and behavioral tests to a native application/runtime crate. |
| `rust/crates/rustaxa-bridge/src/ffi.rs` | CXX declarations and carriers | All C++ bridge clients | External boundary | Keep declarations and plain carriers only; delete each item with its last caller. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | FinalChain and execution APIs | FinalChain shim, execution adapters | External boundary | Split native ownership, public query, and a narrow external-EVM executor API. |
| `rust/crates/rustaxa-bridge/src/network.rs` | ingress, planning, effects | tarcap handlers | External boundary | Complete `CRW-N01`; retain transport-only execution API. |
| `rust/crates/rustaxa-bridge/src/pbft_chain.rs` | PBFT service chain methods | PBFT chain/manager shims | Internal bridge route | Migrate named C++ readers to native service/query APIs. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | `BridgePbftService`, manager runtime | App, PBFT/vote/pillar shims | Native service wrapper | Move application service and behavioral tests out of bridge. |
| `rust/crates/rustaxa-bridge/src/pbft_period_cleanup.rs` | combined cleanup operation | PBFT manager shim | Internal bridge route | Native PBFT service owns operation without CXX-shaped API. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | sync admission/runtime methods | PBFT manager and tarcap | Internal bridge route | Fold into native PBFT/network pipeline APIs. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_generation.rs` | vote generation adapters | Vote/PBFT shims | Internal bridge route | Signing becomes a leaf port and vote behavior remains native. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_ingress.rs` | ingress adapter | network/vote path | Internal bridge route | Complete network pipeline migration. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_payload.rs` | payload conversion | PBFT/vote shims | Internal bridge route | Call native codecs directly or keep conversion private to FFI. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_progress.rs` | progress adapter | PBFT/vote shims | Internal bridge route | Fold into native PBFT service. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_validation.rs` | validation conversion | PBFT/vote shims | Internal bridge route | Fold into native PBFT service. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | pillar state/storage adapters | pillar and storage shims | Internal bridge route | Native PBFT/pillar owner replaces partial compatibility services. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | pillar vote planning/runtime | pillar/PBFT shims | Internal bridge route | Move behavior/tests native; retain signing/network leaf effects only. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | PBFT-service methods | proposed-block/PBFT shims | Internal bridge route | Delete proposed-block C++ facade and fold native methods into PBFT owner. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi` | RPC, GraphQL, light plugin | External boundary | Keep a client-oriented read API; remove manager/storage construction elsewhere. |
| `rust/crates/rustaxa-bridge/src/rewards_stats.rs` | standalone compatibility runtime | rewards shim/tests | Compatibility facade | Delete facade, handle, and compatibility-only tests. |
| `rust/crates/rustaxa-bridge/src/slashing.rs` | PBFT-service slashing planning | slashing/vote shims | Internal bridge route | Native service returns leaf signing/submission effects. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | DAG-service sortition methods | sortition/DAG/PBFT shims | Internal bridge route | Delete C++ sortition facade after callers migrate. |
| `rust/crates/rustaxa-bridge/src/storage.rs` | storage facade, queries, batch | storage shim, conformance, bootstrap | Compatibility facade | Native bootstrap/query/test fixtures replace broad storage handles. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | legacy transaction inspection | PBFT/transaction materializers | Internal bridge route | Use native codec internally; retain only if a named C++ client remains. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | transaction runtime and adapters | transaction/DAG/PBFT shims | Internal bridge route | Move runtime/tests native and reduce C++ to submission/EVM leaf adapters. |
| `rust/crates/rustaxa-bridge/src/vdf.rs` | VDF operations/cancellation | VDF and proposer C++ | External boundary | Keep until VDF execution is a native or dedicated external API. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | PBFT-service vote state adapters | verified-votes/vote/PBFT shims | Internal bridge route | Delete materialization facade and fold native behavior into PBFT owner. |

## Exported CXX Bridge Handles

| Handle | Implementing module | Named consumers | Classification | Delete or narrow when |
| --- | --- | --- | --- | --- |
| `BridgeConsensusQueryApi` | `query.rs` | RPC, GraphQL, light plugin | External boundary | Keep only client-oriented public reads. |
| `BridgeConsensusNetworkApi` | `network.rs` | tarcap handlers | External boundary | `CRW-N01` leaves a transport-only API. |
| `BridgeDagTransactionService` | `dag_transaction_service.rs` | App and DAG/transaction/sortition/gas shims | Native service wrapper | Native application owner replaces bridge-owned implementation. |
| `BridgePbftService` | `pbft_manager.rs` | App and PBFT/vote/pillar shims | Native service wrapper | Native application owner replaces bridge-owned implementation. |
| `BridgeRewardsStatsRuntime` | `rewards_stats.rs` | rewards shim/tests | Compatibility facade | Delete standalone facade and tests. |
| `BridgePillarChainStorage` | `pillar_chain.rs` | storage shim | Compatibility facade | Replace remaining pillar storage compatibility calls. |
| `BridgeStorage` | `storage.rs` | storage shim, bootstrap, tests | Compatibility facade | Native construction and narrow query/admin APIs replace it. |
| `BridgeDagStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native DAG/query fixtures replace it. |
| `BridgeMetadataStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native metadata/query fixtures replace it. |
| `BridgePbftStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native PBFT/query fixtures replace it. |
| `BridgePbftVoteStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native vote/query fixtures replace it. |
| `BridgeTransactionStorageQueries` | `storage.rs` | storage shim/tests | Compatibility facade | Native transaction/query fixtures replace it. |
| `BridgeFinalChainStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | FinalChain/query APIs replace it. |
| `BridgePeriodStorageQueries` | `storage.rs` | storage shim/query/tests | Compatibility facade | PBFT/query APIs replace it. |
| `BridgeStorageBatch` | `storage.rs` | storage shim, rewards compatibility, tests | Compatibility facade | No named client requires legacy `DbStorage::Batch`. |
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
| `rewards_stats_shim` | non-production rewards compatibility facade | compatibility tests | Compatibility facade | Delete with bridge runtime and tests. |
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
