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
| `rust/crates/rustaxa-bridge/src/storage.rs` | `BridgeStorage`, `BridgeStorageBatch`, `Bridge*StorageQueries`, `create_storage`, `create_*_storage_queries`, `create_storage_shim_batch` | `storage_shim`, storage conformance tests, consensus shims that bootstrap native Rust storage handles | C++ public compatibility facade | Production consensus authority is retired. Keep typed app/bootstrap construction, query-family public/network compatibility, and the opaque `DbStorage::Batch` carrier until their owning C++ compatibility callers retire; do not grow them into new consensus routes. |
| `rust/crates/rustaxa-bridge/src/query.rs` | `BridgeConsensusQueryApi`, `create_consensus_query_api` | `network/consensus_query.hpp`, RPC/GraphQL, `plugin/light`, Rust tests | External boundary | Keep as the public query facade for RPC/GraphQL/light clients. Narrow remaining storage-backed reads into this facade, then remove direct external construction except approved app/bootstrap points. |
| `rust/crates/rustaxa-bridge/src/network.rs` | `BridgeConsensusNetworkApi`, `create_consensus_network_api`, ingress/planner/drain/report methods, `consensus_network_gossip_pbft_vote` | Latest tarcap handlers, `tests/rust/consensus/test_network_api.cpp` | External boundary | Queue-named bridge helpers are deleted. Keep narrowing the direct network/tarcap facade until only packet ingress, deterministic planners, gossip/send/sync/report effects, and result reporting remain. |
| `rust/crates/rustaxa-bridge/src/final_chain.rs` | `BridgeFinalChain`, `BridgeFinalChainExecutionSession`, `BridgeConsensusExecutionApi`, `create_final_chain*`, `create_final_chain_execution_session`, `create_consensus_execution_api` | `final_chain_shim`, transaction manager runtime, consensus execution adapters | External boundary | Keep EVM/execution boundary while EVM remains out of scope. Move consensus fact reads to Rust FinalChain ports and delete bridge paths that only materialize C++ facts for Rust consensus. Execution-session construction no longer takes a `BridgeFinalChain`; the session owns only the execution request until commit/publish calls reach the real FinalChain boundary. System-transaction planning and native-session commit are now `BridgeConsensusExecutionApi` methods, not standalone CXX exports. |
| `rust/crates/rustaxa-bridge/src/dag_transaction_service.rs` | `BridgeDagTransactionService` | App bootstrap, `dag_manager_shim`, `pbft_manager_shim`, `sortition_params_manager_shim`, `transaction_manager_shim`, `gas_pricer_shim`, DAG/sortition/transaction compatibility tests | Application/bootstrap handle | One App-owned service contains private DAG, sortition, and transaction state behind sibling mutexes and one shared Rust storage owner. App injects the same service directly into PBFT finalization, so PBFT never discovers it through another facade. Full construction restores all three domains and the initial proposal mapping before publication; transaction/gas compatibility construction omits DAG and sortition state and returns stable unavailable errors for those calls. Keep until retained public facades and external EVM/VDF/signing/network executors move behind narrower application APIs. |
| `rust/crates/rustaxa-bridge/src/pbft_chain.rs` | Chain methods on `BridgePbftService`, `create_pbft_chain_service_from_storage` | `pbft_chain_shim`, PBFT bridge tests | C++ public compatibility facade | Production chain state is owned by the application service. Keep the chain-only service constructor only for the stable public `PbftChain(DbStorage)` compatibility/test adapter; remove it when those direct callers migrate. |
| `rust/crates/rustaxa-bridge/src/pbft_manager.rs` | `BridgePbftService`, `create_pbft_service_from_storage`, bootstrap/runtime/sync/finalization/pillar methods | App bootstrap, `pbft_manager_shim`, `pbft_chain_shim`, `vote_manager_shim`, `slashing_manager_shim`, `pillar_chain_manager_shim`, PBFT/pillar bridge tests | Application/bootstrap handle and internal Rust route | One service owns manager, chain, proposed-block, verified-vote, slashing, and pillar state behind sibling locks. Fresh finalization composes directly with the injected DAG service: Rust decodes canonical period data, derives the PBFT chain count, previews and stages sortition, then publishes only after primary storage succeeds. Pillar has a separate one-way readiness transition because its startup replay precedes PBFT manager bootstrap. Chain-only services omit pillar capability; the partial pillar-capable factory is compatibility-only. Continue narrowing CXX operations until the facades contain only classified lifecycle/executor/materialization work. |
| `rust/crates/rustaxa-bridge/src/pbft_sync.rs` | Manager-owned PBFT sync admission, egress, and cert-vote validation functions | `pbft_manager_shim`, PBFT sync bridge tests | Internal Rust route | The standalone process-period planner and repeated-input fact DTO are retired. Keep narrowing live external checks/materialization until PBFT sync processing is fully composed inside the Rust PBFT manager service. |
| `rust/crates/rustaxa-bridge/src/pbft_vote_*` | Vote validation/generation/progress/ingress/payload helpers | Vote manager shim, network API tests, PBFT/vote tests | Internal Rust route | CXX vote pipeline/admission/session/runtime handles are retired. Standalone planner/event free-function exports and bridge-only DTOs are deleted; live ingress uses `BridgeConsensusNetworkApi`, live validation/admission/reward materialization uses the vote runtime inside `BridgePbftService`, and direct helper exports remain only for canonical inspection, vote generation, and payload conversion still called by C++ shims. |
| `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `BridgePbftService` verified-vote methods and owned compatibility snapshots | `verified_votes_shim`, `vote_manager_shim` | Application-service state plus compatibility adapters | `BridgeVerifiedVotes` and its factory are deleted. Keep narrow service methods while C++ requires vote materialization; combine remaining leader/admission/period-cleanup crossings before retiring the facades. |
| `rust/crates/rustaxa-bridge/src/proposed_blocks.rs` | `BridgePbftService` proposed-block methods, stateless storage compatibility, local candidate lookup | `proposed_blocks_shim`, `storage_shim`, `vote_manager_shim` | Application-service state plus compatibility adapters | `BridgeProposedBlocks` and its factory are deleted. Keep service methods while the C++ PBFT/proposed-block facades need materialization; keep stateless storage functions only while `DbStorage` compatibility remains; move local candidate lookup behind a native vote/PBFT planner when the C++ proposal executor retires. |
| `rust/crates/rustaxa-bridge/src/rewards_stats.rs` | `BridgeRewardsStatsRuntime`, `create_rewards_stats_runtime` | `rewards_stats_shim`, reward compatibility tests | C++ public compatibility facade | Production FinalChain no longer constructs this handle or routes publication through the shim. Keep only for the stable standalone `rewards::Stats` compatibility surface and focused tests; delete both when those callers retire. External-EVM production receives distribution-stat RLP from `BridgeFinalChain` and keeps cache mutation entirely inside Rust publication state. |
| `rust/crates/rustaxa-bridge/src/pillar_chain.rs` | `BridgePillarChainStorage`, service-owned pillar methods, `create_pillar_chain_storage` | `storage_shim` (`BridgePillarChainStorage`), App-owned `BridgePbftService`, `pillar_chain_manager_shim` | Application-service state plus storage compatibility facade | Private pillar state is restored once inside the full PBFT service and published after pillar-specific startup replay; no standalone runtime handle/factory remains. Keep `BridgePillarChainStorage` only as the narrow stable `DbStorage` compatibility implementation until those public callers retire; it is not production pillar storage authority. |
| `rust/crates/rustaxa-bridge/src/pillar_votes.rs` | Stateless inspection plus pillar methods on `BridgePbftService` | `pillar_chain_manager_shim`, PBFT manager direct anchor decisions, bridge tests | Internal Rust route | Live vote aggregation, admission, bundles, payload lookup, and anchor decisions use the application-owned service pillar state. The old runtime receivers and standalone handle are retired; C++ keeps only external DPoS/network/signing/event/materialization effects. |
| `rust/crates/rustaxa-bridge/src/sortition.rs` | Sortition methods on `BridgeDagTransactionService` | `sortition_params_manager_shim` public compatibility methods and the private PBFT cross-service finalization operation | Application-service state plus C++ compatibility facade | The standalone handle and factories are deleted. PBFT finalization decodes, previews, stages, retains, and atomically publishes sortition through the two application-owned services; DAG verification and proposal load historical parameters inside the composed DAG service. The PBFT facade accessor/preview path, C++ count and stage relays, direct preview/commit exports, and bridge-mechanics tests are deleted. Keep the facade only for stable public parameter materialization, efficiency, codec, and storage-owning compatibility methods. |
| `rust/crates/rustaxa-bridge/src/transaction.rs` | Transaction RLP inspection and bridge DTO helpers | Transaction manager, period-data queue, tests | External boundary | Keep only wire/codec compatibility helpers needed at C++ network/RPC boundaries. Move internal transaction facts to `rustaxa-types`/native consensus. |
| `rust/crates/rustaxa-bridge/src/transaction_manager.rs` | Private `TransactionRuntimeState` reached through `BridgeDagTransactionService` | `transaction_manager_shim`, RPC submission paths, tests | Internal Rust route | The standalone runtime handle and constructors are retired. Admission, packing, sidecars, queue, gas oracle, transaction count, and storage state remain private to the application service; C++ keeps external EVM/FinalChain execution and public materialization only. |
| `rust/crates/rustaxa-bridge/src/slashing.rs` | Slashing methods on `BridgePbftService` | `slashing_manager_shim`, `vote_manager_shim` | Application-service state plus C++ executor facade | The standalone planner handle and factory are deleted. Planning and duplicate protection use PBFT-service-owned state; C++ supplies external account/gas facts, constructs/signs/inserts the transaction, and reports only the executor outcome. Delete the facade after those executor ports move behind the Rust application boundary. |
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
| `BridgeStorageBatch` | `storage.rs` | `storage_shim`, `rewards_stats.rs`, storage FFI | C++ public compatibility facade | Opaque atomic carrier inside the stable `DbStorage::Batch` lifecycle; C++ compatibility callers sequence typed append operations, while Rust owns validation, keys, columns, batch storage, and atomic commit. Delete after public, conformance, and standalone rewards compatibility callers retire. |
| `BridgePbftVoteStorageQueries` | `storage.rs` | Storage shim/tests, PBFT/vote bridge tests | C++ public compatibility facade | Vote storage reads move behind Rust vote/PBFT runtime ports. |
| `BridgePbftStorageQueries` | `storage.rs` | PBFT chain/manager/finalization tests and bridge helpers | C++ public compatibility facade | PBFT storage reads move behind native Rust runtime ports. |
| `BridgeMetadataStorageQueries` | `storage.rs` | FinalChain, transaction manager, rewards stats tests/helpers | C++ public compatibility facade | Metadata reads move behind native Rust storage/runtime ports. |
| `BridgeDagStorageQueries` | `storage.rs` | DAG/finalization tests and bridge helpers | C++ public compatibility facade | DAG reads move behind native Rust DAG runtime/storage ports. |
| `BridgeTransactionStorageQueries` | `storage.rs` | Transaction manager, DAG, PBFT sync/finalization tests | C++ public compatibility facade | Transaction storage reads move behind native Rust transaction/PBFT runtime ports. |
| `BridgeFinalChainStorageQueries` | `storage.rs` | FinalChain/query compatibility | C++ public compatibility facade | Final-chain storage reads move behind native Rust FinalChain/query APIs. |
| `BridgePeriodStorageQueries` | `storage.rs` | PBFT sync/finalization tests and query helpers | C++ public compatibility facade | Period-data reads move behind native Rust PBFT/finalization runtime ports. |
| `BridgePbftService` | `pbft_manager.rs`, `pbft_chain.rs`, `pbft_sync.rs`, `proposed_blocks.rs`, `verified_votes.rs`, `slashing.rs`, `pillar_chain.rs`, `pillar_votes.rs` | App bootstrap, PBFT/vote/proposed-block/slashing/pillar shims, PBFT/pillar/storage bridge tests | Application/bootstrap handle and internal Rust route | Owns manager, chain, proposed-block, verified-vote, slashing, and pillar state behind sibling locks. Pillar readiness is independent from PBFT bootstrap; chain-only services omit it and the pillar-only partial factory is compatibility wiring. Narrow retained C++ facades as external executors gain typed ports. |
| `BridgeRewardsStatsRuntime` | `rewards_stats.rs` | `rewards_stats_shim`, compatibility tests | C++ public compatibility facade | No production construction path remains. Delete with the standalone `rewards::Stats` compatibility facade after its public/test callers retire. |
| `BridgePillarChainStorage` | `pillar_chain.rs` | `storage_shim` | C++ public compatibility facade | Pillar chain storage is native Rust-owned; the compatibility handle exposes only storage-shim operations with active C++ callers. |
| `BridgeDagTransactionService` | `dag_transaction_service.rs`, `sortition.rs` | App bootstrap, `dag_manager_shim`, `sortition_params_manager_shim`, `transaction_manager_shim`, `gas_pricer_shim` | Application/bootstrap handle | Owns DAG, sortition, and transaction state behind ordered sibling locks. Retain while the stable C++ manager facades and explicit EVM/VDF/signing/network boundaries need one shared Rust lifetime owner; delete after narrower application APIs replace those facades. |

## Consensus Shim Directories

| Shim directory | Current role | Current consumers | Classification | Removal or narrowing condition |
| --- | --- | --- | --- | --- |
| `dag_block_proposer_shim` | Standalone C++ executor facade over Rust manager-owned proposal sessions; Rust selects historical sortition parameters, while C++ executes VDF from the returned typed command, signs one returned hash, and submits the returned signed RLP; feature-on builds do not compile the original proposer or an Old scaffold | App/DAG manager proposer lifecycle | C++ public compatibility facade | Delete after worker/network lifecycle, VDF execution, signing, and add-block effects move behind Rust application/external ports. |
| `dag_manager_shim` | C++ DAG compatibility/executor facade over the App-owned DAG/transaction service; feature-on builds import and compile no legacy manager scaffold; proposer observation, FinalChain fact collection, construction, unsigned-intent, and signed-RLP state are service-owned | App/consensus code, DAG tests | C++ public compatibility facade | Remove remaining public DAG-block/transaction materialization when callers can use the application service or a thinner public facade. |
| `final_chain_shim` | Rust FinalChain runtime behind C++ FinalChain API | App, PBFT manager, transaction manager, RPC/EVM execution | External boundary | Keep EVM execution adapter; remove consensus fact/materialization methods after Rust consensus consumes FinalChain ports directly. |
| `gas_pricer_shim` | Standalone Rust-backed gas price oracle facade | Transaction/RPC gas price paths | C++ public compatibility facade | The facade no longer imports or compiles `GasPricerOld`; keep its feature flag and public adapter until gas price API is native Rust and external RPC sees only a narrow query method. |
| `key_manager_shim` | Standalone FinalChain key-fact adapter; feature-on builds import and compile no legacy manager scaffold | App/bootstrap/key manager users | External boundary | Keep the public facade until key ownership is redesigned; the dead `KeyManagerOld` compile scaffold is removed. |
| `pbft_chain_shim` | Public compatibility view over the application-owned PBFT service | Network/tarcap, DAG/vote compatibility, RPC/public readers, stats, and tests | C++ public compatibility facade | Chain state and finalization mutation are service-owned. Keep this C++ class for reads, JsonCpp, `PbftBlock` materialization, and the classified constructor/update adapters until direct callers migrate. |
| `pbft_manager_shim` | PBFT lifecycle/executor facade over the application-owned service; feature-on builds import and compile no legacy manager scaffold | App bootstrap and consensus loop | Internal Rust route | Manager and chain state now share the service and chain finalization drains internally. Continue with `CRW-03` private-state absorption, leaving only classified lifecycle/executor/materialization shell work. |
| `pillar_chain_manager_shim` | C++ compatibility/executor facade over pillar state owned by the App PBFT service; performs external FinalChain facts, signing, network/events, and object materialization | App/consensus pillar paths | C++ public compatibility facade | Production receives the existing App service; only the named compatibility constructor creates a pillar-only partial service. Delete after signing/materialization, network/events, and FinalChain fact ports move behind native Rust application boundaries. |
| `pillar_votes_shim` | Pillar vote index/admission facade | Retired | Obsolete scaffold | Removed. Live pillar vote state is owned inside `BridgePbftService`; no C++ shim behavior remains. |
| `proposed_blocks_shim` | Service-backed proposed-block materialization facade | PBFT manager and sync/public compatibility paths | C++ public compatibility facade | The facade owns no lock, storage handle, or proposed-block state. Delete after remaining C++ callers no longer require `PbftBlock` materialization or the stable `ProposedBlocks` view. |
| `rewards_stats_shim` | Standalone rewards statistics compatibility facade | Stable public API and rewards compatibility tests | C++ public compatibility facade | Production FinalChain no longer constructs this facade. Rust owns production planning, cache publication, recovery audit, and runtime reload; the external StateAPI edge decodes distribution RLP directly into temporary `BlockStats`. Delete the facade and its bridge runtime when standalone public/test compatibility callers retire. |
| `slashing_manager_shim` | C++ slashing executor facade over PBFT-service-owned planning/cache state; Rust vote admission passes one `SlashingDoubleVoteEvidence` payload while the live `PbftVote` overload is a compatibility adapter | Slashing manager users | C++ public compatibility facade | The facade no longer owns a Rust handle or imports/compiles `SlashingManagerOld`; its module flag remains valid for partial configurations. Delete after FinalChain fact collection, gas lookup, transaction construction/signing, and insertion move behind Rust application ports. |
| `sortition_params_manager_shim` | Service-backed sortition compatibility facade with shim-owned compatibility carrier/codec and no independent Rust handle | DAG/sortition compatibility paths | C++ public compatibility facade | `SortitionParamsManagerOld`, the standalone Rust handle/factories, the redundant module flag, the PBFT-finalization commit helper, and the PBFT facade accessor are retired. App injects the canonical DAG/transaction service directly into PBFT, and the Rust cross-service operation owns preparation and publication. Delete after remaining public compatibility callers retire. |
| `storage_shim` | `DbStorage` Rust-mode overlay and Rust storage owner | App, consensus shims, storage tests | C++ public compatibility facade | Delete broad storage facade after all C++ consensus callers stop using `DbStorage`; keep only external app/admin bootstrap if needed. |
| `transaction_manager_shim` | Transaction compatibility/materialization facade over the App-owned DAG/transaction service | App, RPC submission, PBFT packing | C++ public compatibility facade | Delete after transaction admission/packing/public submission API is native Rust with EVM boundary adapters. |
| `verified_votes_shim` | Service-backed verified-votes compatibility facade with shim-owned materialized carrier types | Vote manager shim and PBFT/network compatibility readers | C++ public compatibility facade | The facade owns no Rust runtime, storage handle, or mutex; coherent materializers consume owned service snapshots. Delete after callers no longer require C++ `PbftVote` carriers. |
| `vote_manager_shim` | Vote manager Rust runtime facade | PBFT manager, DAG/proposed blocks, network vote paths | Internal Rust route | Collapse into Rust PBFT/vote runtime. Keep only external network adapters until network/tarcap API is complete. |

### CRW-02 PBFT service composition

- `BridgePbftService` replaces the exported `BridgePbftManagerRuntime` and `BridgePbftChain` handles. App bootstrap,
  manager runtime/session operations, chain reads/updates, and PBFT sync operations now address the same Rust owner.
- Production restore is chain-first and derives manager period/Cacti activation internally. A one-way bootstrap gate
  rejects live daemon/proposal/sync session starts until the C++ constructor finishes replay and restart processing.
- The public `PbftChain(DbStorage)` and `updatePbftChain(...)` APIs remain classified compatibility adapters. The former
  creates a chain-only service; the production app path shares one service holder across manager and chain facades.
- The finalization chain mutation/report bounce and its CXX report DTO are deleted. Chain update is drained and validated
  inside the manager-owned finalization cursor before Rust returns another external effect.
- The obsolete CXX `PbftManagerStartupFact` fixture carrier is deleted. Rust tests use a private type; C++ bridge/storage
  fixtures seed the durable chain head and call the production service constructor.

### CRW-03 PBFT-private state absorption

- The item is active as proposed-block and verified-vote sub-slices. Proposed blocks land first so leader selection can
  stop passing a `ProposedBlocks&` across the vote-manager boundary before vote state joins the service.
- The proposed-block closeout deletes `BridgeProposedBlocks` and its factory after the durable/live index moves behind
  `BridgePbftService`, the retained `ProposedBlocks` facade becomes a service client, and storage-shim compatibility
  methods use storage-only operations instead of a second live handle.
- Tentative wallet candidates use a non-persisted Rust-local set. They are not inserted into service state or storage
  before leader selection; only the chosen leader enters the authoritative proposed-block index.
- The verified-vote ownership follow-up is implemented: `BridgeVerifiedVotes`, its factory, the facade-owned box, and
the C++ mutex are deleted after `VoteManager` and retained materialization/network facades became service clients.
- Service-private manager, verified-vote, proposed-block, and chain state use sibling Rust lock domains. Coherent vote
materialization returns owned records from one vote-lock epoch, and no guard crosses C++ validation, FinalChain/EVM,
network, logging, or gossip callbacks.
- The proposed-block sub-slice is implemented: `BridgeProposedBlocks`, its factory, its explicit restore API, the
  storage-shim-owned handle, the C++ facade mutex, and `ProposedBlocks&` leader-selection crossing are deleted.
  Production service construction restores the index once; tentative wallet candidates use one ordered, stateless Rust
  batch lookup and only the selected leader is persisted/published.
- The verified-vote ownership sub-slice is implemented. Production construction restores votes before publication;
chain-only services fail vote calls explicitly; storage tests use the full service; and the standalone handle/factory
are absent from the bridge inventory.
- The leader snapshot/revalidation sub-slice is implemented. The authoritative filtering path captures proposal votes,
  aligned proposed blocks, validation flags, and PBFT-chain membership under `votes -> proposed -> chain`, releases all
  Rust guards for C++ block validation, then revalidates a deterministic content fingerprint before applying validation
  reports or marking blocks valid. The production path no longer performs a separate proposal-vote snapshot, per-vote
  proposed-block lookup, or C++ chain-membership callback. Tentative local wallet candidates retain their stateless path.
- The admission-plus-progress persistence sub-slice is implemented. One service call now validates and admits the vote,
  checkpoints only the touched replay-cache delta, verified-vote round, and payload entry, commits any extra-reward and
  `2t+1` progress writes through the existing Rust batch, and publishes the live transition only after that commit.
  Persistence rejection restores the checkpoint before the vote lock is released and suppresses every network,
  slashing, proposed-block, and PBFT-progress executor effect. Replay-only and accepted no-write transitions remain
  explicitly process-local and storage-free. C++ no longer echoes Rust-built persistence payloads into a second service
  call; the generic progress-persistence adapter remains only for non-admission period/round compatibility restore.
- The atomic period-cleanup sub-slice completes CRW-03. Period advance emits one action and makes one service call under
  the `verified votes -> proposed blocks -> storage batch` lock order. Proposed-block deletes commit before direct
  removal of stale vote periods, vote payload sidecars, and proposal periods; rejection publishes neither memory
  mutation. Action code 8 and the manager-only VoteManager cleanup wrapper are deleted. The individual vote and proposal
  cleanup exports remain compatibility/test routes, not the production manager crossing.

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

- Direct `*Old::method(...)` forwarding in consensus shims has no current matches. `dag_manager_shim` is fully detached
  from `DagManagerOld`: the overlay directly includes its standalone facade, and feature-on builds exclude the original
  manager source instead of compiling renamed symbols. It owns its public `VerifyBlockReturnType` enum and
  shared-pointer identity locally. The standalone Rust-mode `dag_shim` facade has now been retired; Rust-enabled builds
  use the native graphs in `BridgeDagTransactionService`'s private DAG state and continue to exclude the original Boost
  graph source.
  Pure-C++ builds continue to select the untouched original header and source for direct `Dag`/`PivotTree` coverage.
- `dag_block_proposer_shim` is likewise detached from its dead legacy compile scaffold. Rust-enabled builds exclude the
  original proposer source, the overlay directly includes the standalone executor facade, and no
  `DagBlockProposerOld` symbol remains. The facade explicitly owns its configuration/thread-pool dependencies. After
  detaching the PBFT manager legacy-header import, it also forward-declares `Network` instead of importing the broad
  network header only to survive the former rename-macro include order. Pure-C++ builds continue to select the untouched
  original proposer.
- `pbft_manager_shim` is fully detached from `PbftManagerOld`. The overlay wrapper includes only the standalone facade,
  feature-on source selection excludes the original manager instead of compiling it under a renamed symbol, and the
  shim owns the stable public PBFT state/result enums. The empty header-only shim translation unit and Old-identity test
  are deleted; module-disabled and pure-C++ builds retain the untouched original manager.
- `key_manager_shim` is fully detached from `KeyManagerOld`. Master Rust source selection excludes the original manager,
  the wrapper includes only the standalone facade, and the shim owns its complete public API and cache state. The
  retained Rust FinalChain lookup is the classified external key-fact adapter; pure-C++ builds keep the untouched
  original header and source.
- `vote_manager_shim` no longer inherits from or constructs `VoteManagerOld`. It owns the complete public API and its
  temporary C++ compatibility state directly, and the upstream `VoteManager` implementation-state section is restored
  to its original private shape. `setNetwork` therefore writes shim-owned state rather than inherited protected state.
- `BridgePbftService` production construction is fallible and storage-backed. Rust restores own votes, extra reward
  votes, and typed latest-round 2t+1 bundles, validates and deduplicates canonical weighted payloads, rebuilds replay,
  retained-payload, uniqueness, round-marker, and voted-block state before publishing the service. The VoteManager
  `own_verified_votes_` object vector is deleted;
  public own-vote reads materialize transient objects from hash-ordered, key/payload-validated native storage records,
  and zero-input Rust clear-all owns the batch. A mutex on the shared `Storage` serializes production reads, saves, direct
  clears, and PBFT lifecycle enumeration/commit across handles; caller-owned storage-shim batches remain explicitly
  externally serialized compatibility operations. PBFT lifecycle clearing therefore no longer emits
  `clear_own_vote_sidecars` or invokes a VoteManager cleanup callback. The post-construction `attachRustStorage` /
  `verified_votes_attach_storage` handoff and optional production storage state are deleted.
- `vote_manager_shim` no longer owns an `extra_reward_votes_` hash vector. Reward-reset stage preparation validates and
  encodes the certified bundle in the service-owned vote runtime; apply-time Rust code holds the shared storage extra-reward lock
  while enumerating keys and committing the bundle replacement and deletions in the primary finalization batch. The
  startup snapshot, reset stage/request, and manager finalization report no longer carry extra hashes or a C++ remaining
  count. An opaque generation minted by the locked Rust apply is propagated through the finalization executor and
  validated by `BridgePbftManagerRuntime`, closing the post-commit admission race without exposing membership facts to
  C++.
- That temporary reward cursor is now retired from C++. `PbftVoteAdmissionRuntime` owns and restores the complete
  period/round/step/block-hash cursor, classifies stale reward votes without a CXX eligibility flag, and supplies
  cursor-owned selection/current-payload/period APIs. The empty `VerifiedVotesStartupSnapshot` path, three VoteManager
  cursor fields, and `reward_votes_info_mutex_` are deleted. The atomic reset persists a dedicated finalized cursor plus
  canonical bundle, so a newer unfinalized latest-cert write cannot change restart reward authority. Post-reset
  publication uses a typed Rust cursor commit bound to the storage reset generation and byte-equal finalized bundle;
  public/tarcap/proposal callers receive only the scalar or canonical payload materialization they require. Legacy stores
  create the dedicated row only after the cert bundle is proven canonical and consistent with the persisted PBFT head,
  finalized period mapping, and canonical period-data block; ambiguous upgrade state is rejected rather than guessed.
- `dag_manager_shim::getShared` now routes through the shim’s own C++ `shared_from_this()` ownership path, and
  `dag_manager_shim::getDagMutex` now returns a shim-owned mutex to avoid `DagManagerOld` forwarding.
- `transaction_manager_shim` no longer inherits from, constructs, or compiles `TransactionManagerOld` in Rust mode. The
  standalone facade preserves the public API and shared-pointer identity while owning only its locks and classified
  FinalChain/EVM, thread-pool, event, logging, and materialization shell. `BridgeDagTransactionService`'s private
  transaction state is the sole owner of queue, sidecar, gas-cache, transaction-count, and persistence state. Its
  storage-backed constructor now reads
  `TrxCount` through native Rust storage and fails construction on storage errors instead of accepting a C++ bootstrap
  count. The original transaction-manager header/source are clean versus `upstream-main`. Focused Rust/C++ tests, the
  bridge inventory and storage-boundary guards, and the Tier 1/Tier 2 rewrite validation gates pass; the archive contains
  no `TransactionManagerOld` symbol in the Rust-enabled build.
- Rust-mode verified-vote construction is available only through the storage-backed production PBFT service. The
  test-only `VerifiedVotes(addr_t)` overlay constructor and standalone verified-vote factories are deleted; Rust unit
  tests use a private `#[cfg(test)]` service helper.
  The standalone `VoteManager` overlay also no longer imports or compiles `VoteManagerOld` when its exact feature
  predicate is enabled. Verified-votes mode now selects this overlay as one complete ownership bundle and requires the
  Rust storage, FinalChain, ProposedBlocks, and SlashingManager facades; the existing SlashingManager dependency further
  requires GasPricer, while TransactionManager owns the queue internally. Unsupported partial flag combinations fail during configuration instead of
  compiling legacy adapters. Pure-C++ and Rust configurations without verified votes retain the untouched upstream
  implementation.
- `verified_votes_shim` is now detached from its final dead legacy scaffold. Its overlay directly includes the standalone
  Rust-backed facade and a shim-owned compatibility-type header preserving the exact public carrier layout and enum
  values. Verified-votes-enabled build metadata excludes the original source, the archive contains no
  `VerifiedVotesOld` symbol, and the scaffold-only non-inheritance test is replaced by carrier-contract plus
  storage-backed facade behavior coverage. Module-disabled and pure-C++ configurations still select the untouched
  original header/source.
- `proposed_blocks_shim` is detached from its dead legacy compile scaffold. The overlay directly includes the standalone
  Rust-backed facade, feature-on builds exclude the untouched original source, and the assertion-only inheritance test
  is removed while storage-backed push/restore/cleanup/materialization behavior coverage remains. `CRW-03` subsequently
  moved the live index and storage owner into `BridgePbftService`; the facade now holds only the shared service and
  temporary C++ `PbftBlock` materialization. `BridgeProposedBlocks`, its factory, and the facade mutex are deleted.
  Module-disabled and pure-C++ configurations still select the untouched original header/source.
- `pbft_chain_shim` is detached from its dead legacy compile scaffold. The overlay directly includes the standalone
  Rust-backed facade, feature-on builds exclude the untouched original source, and the Old-only inheritance assertion is
  replaced by storage-lifetime behavior coverage. `BridgePbftChain` clones and retains its own `Arc<Storage>`, so the
  facade no longer keeps a redundant C++ `DbStorage` lifetime sidecar. The feature flag, bridge handle, public facade,
  legacy JsonCpp formatting, typed finalization report, lock, and temporary `PbftBlock` materialization remain until
  PBFT-chain state and callers fold into the Rust manager/runtime. Module-disabled and pure-C++ configurations still
  select the untouched original header/source.
- Verified-votes closeout validation passed: 14 focused Rust tests, two storage-backed shim tests, four isolated
  VoteManager tests, two focused PBFT manager tests, nine Rust storage bridge tests, both boundary guards, Tier 1/Tier 2
  rewrite validation, and the startup smoke gate. The configuration matrix rejects an incomplete ownership bundle,
  compiles the complete bundle without pillar support, and compiles the untouched upstream VoteManager source with all
  Rust features disabled. Symbol/source audits find neither `VoteManagerOld` nor a Rust-mode legacy VoteManager object.
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
  `BridgeStorage::clear_own_verified_votes` now route through the production `BridgePbftService` vote persistence API, so
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
- A complete 2026-07-15 CXX function census found no unclassified no-caller function export. Every retained function
  has a production C++ source caller except `storage_shim_seed_final_chain_conformance_lookup_rows`, whose sole
  storage-conformance caller remains enforced by `scripts/rewrite_storage_boundary_guard.sh`. Five shared declarations
  that were not Rust/C++ contracts have been removed from `ffi.rs`: C++ now owns
  `TransactionManagerVerifyNotFinalizedInput` in the transaction-manager overlay, and Rust privately owns the
  transaction admission outcome plus the DAG finalization plan, counter-update, and cleanup carriers. The live
  `TransactionManagerVerifyNotFinalizedSidecarFact`, `TransactionManagerVerifyNotFinalizedOutcome`,
  `TransactionManagerAdmissionCommandReport`, and `DagManagerFinalizationApplyPayload` contracts are unchanged.
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
- `BridgeTransactionQueue`, `create_transaction_queue`, the standalone queue CXX methods, bridge module,
  `transaction_queue_shim`, and its feature flag are deleted. The private transaction state in
  `BridgeDagTransactionService` now exclusively owns the native Rust queue in production. Rust-enabled `core_libs`
  excludes the untouched legacy queue source; direct C++
  queue tests remain pure-C++ reference coverage rather than being retargeted to legacy behavior in Rust mode.
- `rewards_stats_shim` is detached from `StatsOld` and is selected as part of the FinalChain ownership bundle.
  `RUSTAXA_ENABLE_REWARDS_STATS` is deleted because it described no independently valid configuration: FinalChain
  unconditionally calls the shim-only publication API. Rust FinalChain builds exclude the untouched legacy
  `rewards_stats.cpp`; FinalChain-disabled and pure-C++ builds compile that original source without a rename. The live
  shim remains the classified C++ `BlockStats`/StateAPI publication adapter and does not delegate reward authority to
  the legacy implementation.
- Former `BridgeTransactionManagerRuntime` no-caller compatibility exports were trimmed before that standalone handle
  was retired, after the transaction-manager shim
  moved to runtime-owned command APIs. Deleted exports include old runtime sidecar lookup/finish/evict helpers, queue
  erase/get/order/known helpers, and sidecar size/remove helpers that had no C++ shim callers.
- Additional no-caller exports from the former `BridgeTransactionManagerRuntime` CXX surface are deleted:
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
- Additional no-caller direct queue/sidecar helpers from the former `BridgeTransactionManagerRuntime` are deleted:
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
  runtime-owned private DAG methods in `BridgeDagTransactionService`. The later cursor-bound VDF composition slice also
  deleted `dag_verify_vdf_sortition_from_block`, `DagVerifyVdfSortitionFromBlockInput`,
  `DagVerifyVdfSortitionResult`, `DagVerifyBlockVdfReport`, and the standalone VDF-report export. Live C++ now submits
  only the exact cursor identity, signed block payload/level, PBFT period hash, and FinalChain VRF key to one composed
  service operation; proposal period, normalized vote counts, and historical sortition parameters remain private Rust
  state.
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
- The former `BridgeProposedBlocks::proposed_blocks_snapshot` was deleted before the handle itself. The live facade now
  uses `BridgePbftService::pbft_service_proposed_blocks_snapshot_entries`, which carries the block payload and validation
  flag; grouped hash snapshots remain Rust-only test coverage.
- The no-storage `create_proposed_blocks_index` CXX constructor plus the standalone cleanup-candidate/remove-period CXX
  helpers are deleted. `CRW-03` also deleted the later storage-backed handle factory. Rust-mode `ProposedBlocks`
  construction now requires the shared PBFT service; local proposal generation uses a non-persisted Rust batch lookup.
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
- The retained `FinalChainRewardsConfig` carrier now includes `fix_redelegate_block_num`, `phalaenopsis_period`, and an
  ordered vector of `RedelegationCorrection { validator, delegator, amount }` records copied from genesis hardfork
  configuration. These bounded `CRW-08` replay/activation inputs are consumed inside Rust FinalChain publication; they
  add no handle, constructor, free export, or C++ execution authority.
- The follow-up restart-durable same-validator history-completeness bit and corruption marker set are private DPoS
  snapshot state. They reuse the retained FinalChain handle and existing snapshot publication path, so they add no CXX
  carrier, export, shim, or module flag.
- The `CRW-08` Cornus underfunded-gas nonce correction is private to Rust FinalChain transaction execution and reuses
  the existing finalization request/result carriers. It changes no CXX handle, export, shim, module flag, or inventory
  deletion condition; the standalone C++ fixture is behavior-parity coverage rather than compatibility scaffolding.
- The `CRW-08` redelegation-correction ordering repair is private to Rust FinalChain and its persisted reward-reference
  graph. It reuses the existing hardfork configuration and finalization carriers, adding no bridge handle, CXX export,
  shim route, module flag, compatibility test surface, or inventory deletion condition.
- The `CRW-08` current-ABI completion audit found no remaining DPoS/slashing bridge or shim route to add or remove.
  Historical snapshots without the complete Rust account/DPoS/reward graph remain a fail-closed replay/rebuild boundary;
  they do not authorize fallback to legacy FinalChain execution. The next bridge audit delta belongs to `CRW-09` domain
  types or external-executor adapter contraction.
- The first `CRW-09` slice widens nonce fields on the retained `AccountLookup`, `FinalizationTransaction`,
  `FinalChainEvmTransactionInput`, and `FinalChainSystemTransactionPlanFact` CXX carriers from `u64` to canonical minimal
  big-endian `Vec<u8>`. This removes FinalChain shim/bridge truncation and preserves arbitrary-width Rust account state
  plus executor transcripts. The unchanged C++ account surface rejects values above U256 explicitly. This is a
  `CRW-07` carrier-field delta only: no handle, export, constructor, shim route, module flag, or deletion condition changes.
- The typed transaction-position `CRW-09` slice keeps all retained CXX carrier widths unchanged. Rust-owned execution,
  publication, receipt, location, audit, and index paths use `FinalChainTransactionPosition(u32)`; bridge ingress checks
  the retained `u64` executor/result boundary and outbound conversion widens the inner `u32`. Count overflow rejects
  before external execution, while request-ID and persisted RLP widths remain byte-compatible. No handle, export,
  constructor, shim route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- The canonical log-bloom `CRW-09` slice replaces the duplicate storage bloom alias and unchecked Rust-domain
  `Vec<u8>` values with one shared `FinalChainLogBloom([u8; 256])`. CXX and RPC carriers remain unchanged vectors or
  arrays with explicit edge conversion; valid RLP, hashes, request identities, pending markers, and storage chunks keep
  identical bytes. No carrier, handle, export, constructor, shim route, module flag, compatibility-only test, or
  `CRW-07` inventory entry changes.
- The typed gas-price `CRW-09` slice keeps finalization, transient-call, and external-EVM CXX fields as `Vec<u8>` while
  converting once at the bridge edge to `FinalChainGasPrice(U256)`. Oversized inputs fail with a stable error and
  outbound execution requests retain the fixed 32-byte C++ shape. Request identity keeps fixed regular-transaction
  prices and the prior minimal system-transaction price bytes, so identities and ABI remain compatible. No
  carrier, handle, export, constructor, shim route, module flag, compatibility-only test, or `CRW-07` inventory entry
  changes.
- The typed transaction-value `CRW-09` slice keeps finalization, transient-call, and external-EVM CXX value fields as
  `Vec<u8>` while converting once at bridge ingress to `FinalChainTransactionValue(U256)`. Regular executor output stays
  fixed 32 bytes and system output stays legacy-minimal with zero `[0]`; request identity and canonical transaction RLP
  remain compatible. No carrier, handle, export, constructor, shim route, module flag, compatibility-only test, or
  `CRW-07` inventory entry changes.
- The encoding-preserving account-balance `CRW-09` slice keeps genesis and account-lookup CXX balance fields as
  `Vec<u8>` while Rust owns the numeric value and fixed/minimal snapshot provenance. It adds no CXX carrier, handle,
  export, constructor, shim route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- The typed gas-lifecycle `CRW-09` slice keeps all FinalChain CXX gas fields and public query DTOs as `u64`, converting
  infallibly at bridge ingress and explicitly unwrapping at egress. It adds no carrier, handle, export, constructor, shim
  route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- The typed DPoS-policy `CRW-09` slice keeps the four genesis/config CXX amount fields as `Vec<u8>`, validates their U256
  width at bridge ingress, and passes `DposTokenAmount` through Rust policy arithmetic. It adds no carrier, handle,
  export, constructor, shim route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- The standalone DPoS snapshot-provenance characterization adds Rust codec tests only. It changes no carrier, handle,
  export, constructor, shim route, module flag, compatibility-only bridge test, or `CRW-07` inventory entry.
- The `CRW-09C` block-number lifecycle keeps FinalChain CXX, StateAPI, PBFT, storage, and public query carriers as `u64`.
  Rust wraps values once at FinalChain ingress and explicitly unwraps typed identities at egress, storage, RLP, and
  request-ID boundaries. No carrier, handle, export, constructor, shim route, module flag, compatibility-only test, or
  `CRW-07` inventory entry changes.
- Remaining CRW-09 carrier reconciliation follows behavior-family rows `CRW-09D` through `CRW-09I`. Temporary C++
  `rewards::BlockStats` decoding is classified under scope-gated `CRW-E01` because removing it changes the accepted
  StateAPI external-executor contract; it does not block ordinary non-EVM CRW-09 completion.
- `CRW-09F` keeps reward-index arithmetic and persistence provenance private to `rustaxa-consensus`; snapshot scalar
  mirrors and graph RLP retain their existing byte carriers. No handle, export, constructor, shim route, module flag,
  compatibility-only test, or `CRW-07` inventory entry changes.
- `CRW-09G` keeps reward pools and claim settlement inside Rust FinalChain while reusing the existing byte-compatible
  snapshot and receipt boundaries. Transaction-fee ownership, reward deltas, pools, and successful transfers use
  `DposTokenAmount`; persistence provenance remains private to `rustaxa-consensus`. No carrier, handle, export,
  constructor, shim route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- `CRW-09H` types reward configuration, minted totals, Aspen supply arithmetic, and finalized header rewards after
  bridge ingress while keeping migration phase, yield, and snapshot byte provenance private to `rustaxa-consensus`.
  Existing external EVM/CXX byte carriers and snapshot slots 8-10 remain unchanged. No handle, export, constructor,
  shim route, module flag, compatibility-only test, or `CRW-07` inventory entry changes.
- The first `CRW-09I` contraction deletes the `DagProposerFinalChainFacts` and
  `DagProposerFinalChainFactsReport` CXX carriers, their standalone FinalChain bridge/shim getter, and the C++ copy
  relay. The cursor-bound DAG service now reads Rust FinalChain head and DPoS/VRF facts directly with lock-free query
  separation and cursor/sortition revalidation. The retained `BridgeFinalChain` and `BridgeDagTransactionService`
  handles are passed only through shim-private composition; no new handle or public compatibility API is added.
- The next `CRW-09I` contraction deletes the DAG verification `DagVerifyBlockAuthorizationReport` and
  `DagDposAuthorizationFacts` CXX carriers, the standalone FinalChain getter/shim relay, and the C++ VRF-key copy. The
  DAG cursor retains signed block bytes, recovers the sender at the established authorization stage, reads FinalChain
  facts without service locks held, revalidates the exact cursor, and keeps the accepted VRF key private through VDF
  verification. Retained block/proposal-hash and gas reports remain classified executor/public-object facts.
- The Pillar `CRW-09I` contraction removes `PillarChainManager` as a consumer of generic `PbftFinalChainFact*`
  carriers. Single-vote validation/admission, synced-bundle weighting, threshold lookup, and block creation now borrow
  Rust FinalChain synchronously inside `BridgePbftService`; no pillar mutex is held during the query and mutation
  revalidates the exact preparation or anchor generation. The FinalChain shim exposes one borrowed, non-retainable
  Rust handle accessor for shim-private composition. Generic fact carriers remain because PBFT manager and VoteManager
  still consume them, and block creation returns a validator-count vector solely for temporary C++
  `CurrentPillarBlockDataDb` materialization.
- The VoteManager threshold `CRW-09I` contraction removes `getPbftTwoTPlusOne` as a consumer of generic
  `PbftFinalChainFact*` carriers. `BridgePbftService` derives its live Rust PBFT-chain size, probes the Rust threshold
  cache, borrows Rust FinalChain only on a cache miss that requests the exact-period DPoS total, then re-enters the
  planner without retaining either lock or handle. The operation-specific CXX request is narrowed to period, vote type,
  and committee configuration; PBFT-chain size, DPoS total, and readiness/error fields now exist only in the private
  Rust planner fact. Generic FinalChain fact carriers remain live for VoteManager generation/sortition and PBFT-manager
  validation/eligibility until those operation families
  are composed. The dead non-composed `VerifiedVotes::twoTPlusOneThreshold` facade and
  `pbft_service_verified_votes_two_t_plus_one_threshold` CXX export are deleted; native Rust tests reach the private
  planner under the service mutex without retaining an external-state injection API.
  The CXX result is likewise narrowed to status, error code, threshold presence, and threshold value; sortition
  threshold, cache diagnostics, and the retired two-pass `needs_total_dpos_votes` signal remain Rust-private.
- The VoteManager validation `CRW-09I` contraction removes `validateVote` as a consumer of generic
  `PbftFinalChainFact*` carriers and the C++ `KeyManager`. One PBFT-service call accepts canonical vote bytes plus strict
  VRF and committee configuration, releases its vote-state mutex for voter/total DPoS and exact/prior/next VRF-key
  lookup through Rust FinalChain, and reacquires it only for address-key caching and terminal replay publication.
  Successful validation returns canonical weighted vote RLP; C++ verifies the full identity and hydrates only its
  temporary live sidecar. DPoS counts, keys, readiness flags, and sortition thresholds do not cross this composed
  boundary. The generic carrier remains live for VoteManager generation/sortition and PBFT-manager consumers.
- `BridgeTransactionManagerSidecar` is retired as a CXX handle. No C++ shim callers remained for the standalone sidecar
  constructor, methods, DAG-save route, or finalized-status route; live sidecar state is now private to the transaction
  state in `BridgeDagTransactionService`, whose command APIs own those paths.
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
  pillar/storage shims only require current/latest block, own-vote, and finalized-block storage methods.
- `BridgePillarChainStorage::pillar_chain_storage_load_period_data` is deleted from the CXX surface after a repository-wide
  caller scan found no C++ consumer. The underlying native Rust period-data loader remains in
  `BridgePillarChainRuntime::pillar_chain_runtime_load_startup_bootstrap`, where Rust derives and loads the recovery period
  without exposing a storage getter to C++.
- `pillar_chain_manager_shim` no longer constructs a parallel `BridgePillarChainStorage` handle during startup.
  `BridgePillarChainRuntime::pillar_chain_runtime_load_startup_bootstrap` loads the own vote, current-block sidecar, and
  latest finalized pillar block, decodes the latest block in Rust, derives the following period-data lookup, and returns
  one recovery snapshot for temporary C++ materialization. The storage-only handle remains solely for `storage_shim`
  compatibility callers.
- `pillar_chain_manager_shim` is now a standalone feature-on facade. The overlay wrapper directly includes the
  shim-owned header, `RUSTAXA_ENABLE_PILLAR_VOTES` excludes both untouched legacy implementation sources, and the
  `PillarChainManager=PillarChainManagerOld` compile rename is deleted. Module-disabled and pure-C++ configurations
  continue to select the original manager and `PillarVotes` sources. This removes dead compatibility objects only;
  `BridgePillarChainStorage` remains live for the separate storage facade, and the manager's FinalChain, network,
  signing/materialization, and event boundaries are unchanged. Feature-on compile metadata/archive inspection, focused
  Rust/CXX pillar tests, the focused PBFT consumer, Tier 1, Tier 2, smoke, and pure/module-disabled object compilation
  cover the route change. The full pillar-suite process still exposes its pre-existing same-process `/tmp/taraxa0`
  RocksDB lock after the first node-owning case; 10 of 13 cases complete before that harness failure.
- `BridgePillarChainRuntime` is now the sole pillar-manager current-anchor decision source. Its fallible factory decodes
  and canonically validates persisted `CurrentPillarBlockDataDb`, while current-data apply decodes first, persists under
  the snapshot write lock, and publishes the new anchor only after the write succeeds. The process-local generation is
  carried across external FinalChain DPoS reads so single-vote and synced-bundle apply reject stale work. Single-vote
  admission retains one-time vote-hash preparations and prevents trusted local/restart preparation from replacing an
  existing checked token. The shim records bounded successful-external-validation receipts by vote hash; receipt-backed
  add runs checked prepare again against the current anchor and consumes its receipt only after success, while only
  receipt-free local/restart add uses trusted prepare. Apply consumes the token and reruns external relevance/identity checks while holding the anchor read lock
  through mutation. The Rust registry and shim receipt map are each capped at 4,096 entries; eviction/missing-token apply
  fails closed without converting an external route into trusted admission. PBFT candidate validation,
  proposal/local-vote anchor selection, restart
  post-processing, vote relevance/admission, synced-bundle context, and PBFT-facing finalization all consume that runtime
  snapshot. C++ finalization follows a documented compatibility-mutex-to-Rust lock order and holds the mutex through
  Rust persistence/cleanup and compatibility publication, releasing it before network/event callbacks. C++ retains
  `current_pillar_block_` only for startup/public compatibility, block-creation vote-count materialization, logging, and
  post-decision event payloads.
- Pillar strict-majority threshold arithmetic (`total_vote_count / 2 + 1`) is Rust-owned. The composed Rust service
  borrows FinalChain for the typed total-vote fact and returns the compatibility threshold result to C++. The operation-tagged
  current-anchor planner uses checked PBFT-period subtraction and restart-interval addition, with explicit missing,
  mismatch, underflow, invalid-interval, overflow, and not-due statuses.
- Latest-finalized pillar identity is now part of the runtime snapshot rather than a C++ manager field. Runtime
  construction loads and canonically validates both current and latest-finalized pillar rows; malformed latest bytes fail
  before publication. Creation derives previous vote counts from the canonical current-data row and derives its parent
  from the latest-finalized snapshot. Linkage and PBFT finalization use the same snapshot, and successful persistence
  publishes the new finalized block before the runtime returns. C++ supplies only external block/FinalChain facts and
  materializes canonical latest-block bytes for its public compatibility getter. The standalone CXX
  `plan_pillar_vote_count_changes`, `plan_pillar_block_linkage`, and
  `plan_pillar_block_creation_with_vote_counts` exports, their broad caller-supplied parent facts, and their C++
  bridge-mechanics tests are removed.
- `pillar_chain_manager_shim::validateSyncPillarVotesBundleDeterministically()` no longer performs shim-local per-vote
  inspection or supplies a C++ current hash. Runtime prepare inspects canonical vote RLPs and returns recovered voters,
  expected hash, and anchor generation; the service releases the pillar lock, borrows Rust FinalChain for ordered vote
  weights and the total, then reacquires pillar state to revalidate and apply weighted validation, threshold
  initialization, and insertion.
  The obsolete standalone `inspect_pillar_vote_bundle_rlps` CXX export is removed because it could not bind its recovered
  identities to current-anchor state.
- The standalone `plan_pillar_vote_relevance` CXX export is deleted. Production tarcap relevance checks use
  `BridgeConsensusNetworkApi::consensus_network_plan_pillar_vote_relevance`, while the pillar-chain manager uses
  `BridgePillarChainRuntime::pillar_chain_runtime_plan_vote_relevance` so duplicate detection comes from runtime-owned
  pillar-vote state. Native `rustaxa-consensus` and bridge-module tests keep direct planner coverage.
- `pillar_chain_manager_shim::createPillarBlock()` now calls
  `plan_pillar_block_creation_with_vote_counts`, which combines pillar-block shell planning and ordered validator
  vote-count delta planning behind one Rust bridge call. The creation-only `plan_pillar_block_creation` CXX export and
  shell-only DTO are deleted. Rust now composes exact-period FinalChain DPoS vote-count reads and binds the returned plan
  to its anchor generation. C++ retains temporary `PillarBlock` and current-block storage-payload materialization plus
  live manager mirrors, but persistence uses a generation-checked `BridgePillarChainRuntime` apply instead of
  `BridgePillarChainStorage`.
- The no-caller plain-fact pillar-vote bundle CXX planner is deleted:
  `PillarVoteBundleFact`, `PillarVoteBundleAcceptedVote`, `PillarVoteBundlePlan`, and `plan_pillar_vote_bundle`.
  Live pillar-chain sync uses generation-bound runtime prepare/apply around a Rust-composed FinalChain DPoS weight and
  total-vote lookup. The old standalone inspection/weighted planner exports, accepted-voter DTO, shim-side
  accepted-hash-to-live-vote map, and `addPlannedVerifiedPillarVoteForRust` insertion helper are deleted. Native
  `rustaxa-consensus` tests keep coverage for the plain domain planner.
- The standalone pillar-vote CXX handle is deleted from `ffi.rs` after the last C++ bridge test moved to
  `BridgePillarChainRuntime` for weighted-bundle apply and payload lookup coverage. The remaining pillar-vote fixture is
  module-local test code in `pillar_votes.rs`, so retired handle names no longer live in bridge code.
- The no-caller `pillar_chain_runtime_cleanup_votes_by_period` CXX export is deleted after callsite audit confirmed the
  runtime cleanup method had no live C++ shim or bridge-test caller.
- Single pillar-vote admission in `pillar_chain_manager_shim` now uses
  `BridgePillarChainRuntime::pillar_chain_runtime_prepare_single_vote_admission` plus
  `BridgePillarChainRuntime::pillar_chain_runtime_apply_prepared_single_vote_admission`. Rust owns canonical RLP decode, signature
  recovery, duplicate/relevance/identity checks, period-data initialization, insertion, and conflict/duplicate
  classification. The service composes FinalChain DPoS eligibility/vote-count reads and threshold initialization; C++
  retains logging and temporary live-vote materialization.
  The manager's own-vote persistence write now also enters through `BridgePillarChainRuntime`; the matching
  `BridgePillarChainStorage` write methods remain for `storage_shim` compatibility only.
  The piecemeal single-vote CXX exports `pillar_votes_period_data_initialized`, `pillar_votes_init_period_data`,
  `pillar_votes_vote_exists`, `pillar_votes_is_unique_identity`, `pillar_votes_is_unique_vote`, and
  `pillar_votes_insert_vote` are deleted along with `PillarVotePayload`, `PillarVoteIdentityPayload`,
  `PillarVoteUniqueOutcome`, and `PillarVoteInsertOutcome`.
- `PillarChainManager::isRelevantPillarVote` now enters the pillar runtime through
  `pillar_chain_runtime_plan_vote_relevance`. The obsolete C++ `pillarVoteExistsByLookup` payload materialization and
  hash scan are deleted; Rust owns duplicate detection from the runtime vote index before running the relevance planner.
- PBFT-facing pillar-block finalization now uses a prepare/commit/acknowledge protocol. Rust prepare owns selected-vote
  lookup and deterministic planning but performs no durable pillar write, vote cleanup, snapshot publication, or event
  emission. Its bounded generation-bound registry reuses identical preparations and evicts the oldest entry at its cap.
  The prepared canonical pillar row is appended to the same Rust-owned primary PBFT storage batch; only after that batch
  commits does acknowledge authenticate the exact durable row, publish the latest-finalized runtime snapshot, and clean
  the matching votes. Missing or mismatched rows retain the token for retry. C++ always reconciles after protected locks
  are released, including a protected-action failure after primary commit, and holds its compatibility mutex through
  acknowledgement and latest identity materialization before unlocking for event emission. Direct one-shot
  finalization is an explicit unsupported Rust-mode compatibility path. The old CXX
  planner exports `plan_pbft_finalization_pillar_preflight`, `report_pbft_finalization_pillar_preflight`, and
  `plan_pillar_block_finalization` plus their bridge-only DTOs are deleted. C++ still owns network vote-bundle requests,
  legacy vote materialization for `PeriodData`, and event emission.
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
  It calls `BridgePbftService::pbft_service_proposed_blocks_cleanup_with_storage`, which plans stale period/hash groups,
  commits a native Rust storage delete batch, and only then mutates the service-owned index. The public batch loop in
  `libraries/core_libs/consensus/src/pbft/proposed_blocks.cpp` is legacy/reference behavior when
  `RUSTAXA_ENABLE_PROPOSED_BLOCKS` enables the overlay, not remaining Rust-mode storage-shim debt.
- The shim owns no separate proposed-block bridge or C++ lock. `pushProposedPbftBlock(..., false)` now rejects tentative
  publication; local wallet candidates use the isolated batch lookup, while service pushes always persist before live
  publication.
- `sortition_params_manager_shim` is the active Rust-mode route for sortition startup and finalized-period persistence.
  It constructs `BridgeSortitionParamsManager` with `DbStorage::rustStorage()`, so the Rust runtime loads persisted
  changes, persists the missing period-zero default change, reads period-specific parameters, and persists emitted
  finalized-period changes through native Rust storage. Master `RUSTAXA_ENABLE` mode selects the standalone overlay and
  excludes the untouched original source; the compatibility carrier and its canonical RLP codec are shim-owned, and the
  redundant module flag plus `SortitionParamsManagerOld` scaffold are retired.
- The direct `sortition_params_for_period(found, change)` CXX export is deleted. Live C++ lookups use the storage-backed
  `sortition_params_for_period_from_storage(period)` route, so callers no longer inject synthetic sortition-change
  payloads through a bridge-shaped helper. Native `rustaxa-consensus` tests keep direct `params_for_period` coverage.
- `SortitionParamsManager::applyBlockForSortitionRuntime` and the no-caller
  `sortition_record_finalized_period` CXX wrapper are deleted. They exposed an unstaged live-state mutation alongside the
  authoritative preview/primary-batch/commit finalization route and the storage-owning public compatibility route.
  Native `rustaxa-consensus::sortition::SortitionParamsManager::record_finalized_period` remains the internal domain
  transition used to validate and apply a prepared commit; the redundant bridge-level method is also gone. The legacy
  `Batch&` on `pbftBlockPushed` remains source-compatible for pure C++ and public facade tests; Rust ignores it because
  that compatibility call commits through the runtime-owned native storage handle.
- PBFT finalization no longer calls a sortition-facade commit helper and copies a six-field
  `PbftManagerFinalizationSortitionCommitReport` back into the manager cursor. The manager service retains the optional
  sortition change recorded in the committed primary stages, validates cursor/action identity, then calls one
  manager-to-sortition Rust operation with only finalized transaction/reference counts and the non-empty chain size.
  The sortition owner clones and validates its next state before publication, so stale cursors, invalid counts, and
  preview/stage mismatches do not mutate live sortition state.
- Sortition closeout validation passed: nine focused Rust bridge tests, three CXX sortition bridge tests, three shim
  tests, 13 public sortition tests, Tier 1/Tier 2 rewrite gates, and the startup smoke gate. The removed symbols have no
  remaining repository references, and original upstream sortition sources remain unchanged.
- `final_chain_shim` is the active Rust-mode route for FinalChain startup, native finalization, external-EVM publication,
  pending-publication recovery, and storage audit. It constructs `BridgeFinalChain` and `BridgeConsensusExecutionApi`;
  C++ supplies only the external `StateAPI`/EVM adapter, while Rust commits FinalChain headers, receipts, transaction
  indexes, bloom indexes, execution counters, rewards-stat updates, pending-publication markers, recovery cleanup, and
  genesis/header storage through native Rust storage. The standalone overlay no longer imports `FinalChainOld`, and
  Rust-mode builds exclude the original `final_chain.cpp`; that untouched implementation remains pure-C++ reference
  behavior only.
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
- `BridgeGasPricer`, its bridge module, both constructors, and all bid/update exports are retired. The native
  `GasPriceOracle` is composed into `BridgeDagTransactionService`'s private transaction state; its production
  constructor restores gas history
  through the same native storage owner that restores transaction state. Pool bids inspect the runtime-owned queue and
  proposal gas limit directly. The C++ `GasPricer` remains only as a public compatibility facade and delegates production
  work to `TransactionManager`; its storage-free combined runtime is limited to standalone facade tests.
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
  vote ingress planning, `BridgePbftService` owns validation/admission/reward-vote materialization, and the removed
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
- Sortition finalization update facts no longer leak through `sortition_params_manager_shim`. PBFT advances through the
  cross-service `pbft_manager_runtime_advance_finalization_sortition_commit` operation with only finalized-period count
  facts. Rust validates the manager cursor and retained committed-stage change, atomically publishes the next sortition
  state, and fills the native live-mutation report internally; the former C++ commit helper and report carrier are
  deleted. A failure after the primary storage batch is an explicit fatal invariant because normal duplicate resume
  intentionally does not replay protected sortition mutation.
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
  lives inside the private DAG state of `BridgeDagTransactionService` through `dag_manager_runtime_begin_verify_block_session`,
  `dag_manager_runtime_verify_block_session_next`, and `dag_manager_runtime_verify_block_session_report_*`, so C++ no
  longer allocates a standalone bridge handle for `DagManager::verifyBlock`.
- `BridgeDagProposerSession` is retired. DAG proposal attempts still have C++ executor boundaries for live transaction
  packing, async VDF proof work, block signing/materialization, and `addDagBlock`, but the ordered proposal cursor now
  lives inside the private DAG state of `BridgeDagTransactionService` as a keyed per-attempt cursor through
  `dag_manager_runtime_begin_proposer_session`, `dag_manager_runtime_proposer_session_next`, and
  `dag_manager_runtime_proposer_session_report_*`, so `DagBlockProposer` no longer allocates a standalone bridge handle
  for each attempt while still preserving concurrent per-wallet proposal attempts.
- `BridgeDagProposerRetryState` is retired. Per-wallet DAG proposer retry cursors now live inside
  `BridgeDagTransactionService`, keyed by wallet VRF public key. `dag_block_proposer_shim` passes only the configured retry
  budget, and terminal runtime-owned proposal sessions apply retry updates before deleting their cursor.
- `BridgeDagManagerRuntime`, `BridgeTransactionManagerRuntime`, and their standalone CXX factories are retired. App now
  owns one `BridgeDagTransactionService` through a small C++ RAII holder and passes the same holder to the retained
  `TransactionManager` and `DagManager` facades. Rust keeps `DagRuntimeState` and `TransactionRuntimeState` private behind
  sibling mutexes, all service receivers are shared references, and full construction restores both state families plus
  the initial proposal-period mapping before publication. Transaction-only compatibility factories are limited to
  standalone facade tests and reject every DAG call with `DAG_SERVICE_UNAVAILABLE`.
- `DagProposerTransactionPackRequest`, `DagProposerTransactionPackReport`, `DagManager::reportProposerTransactions`,
  `DagBlockProposer::getShardedTrxs`, and the shim-only `packShardedTransactionPayloads` carrier path are retired.
  `BridgeDagTransactionService` now validates the private DAG cursor, derives its proposal/shard limits, opens an
  owner-bound transaction pack cursor, and transfers selected hash/RLP/gas payloads directly into the DAG session.
  C++ observes network throttling and executes only requested EVM estimates while `TransactionManager::pack_mutex_`
  prevents compatibility packing from replacing the live proposer cursor. Composite Rust calls lock DAG before
  transaction and release both locks before returning to the EVM executor.
- `DagManagerFinalizationApplyPayload::remove_transaction_hashes` and the finalized-order
  `DagManager -> TransactionManager::removeNonFinalizedTransactions` relay are retired. The composed service now locks
  DAG before transaction, performs the fallible DAG/storage commit, then infallibly removes matching private
  transaction sidecars before returning only finalized count and expired DAG hashes to C++. The public transaction
  removal API remains for direct compatibility callers. Factory-only DAG restore and initial proposal-mapping methods
  are no longer exported through CXX.
- `DagVerifyBlockSessionStep::query_hashes`, `DagVerifyBlockTransactionReport`, and the public transaction-report CXX
  export are retired. The composed service privately reads the active DAG verification query and prepares ordered
  transaction views without advancing. A private `DagManager`-friend TransactionManager adapter materializes and
  hash-validates those views, reads every resolved sender at the exact proposal period, and returns a cursor-bound nonce
  completion; Rust revalidates the cursor and lookup, applies finalized-transaction filtering, and only then advances.
  Caller-supplied transactions retain precedence, block order and duplicate references are preserved, and the later EVM
  gas estimator remains in C++.
- `DagProposerSessionBeginInput::transaction_pool_size` and `non_finalized_transaction_count` are retired. The composed
  proposer-session start locks DAG before transaction and snapshots queue size plus non-finalized sidecar size directly
  from the sibling Rust runtime. `DagBlockProposer` no longer relays those observations through public TransactionManager
  getters; the cursor retains them for empty-pool, non-finalized-limit, and pack decisions.
- Accepted DAG insertion now uses a cursor-bound composed prepare/complete transition. The direct
  DagManager-to-TransactionManager save relay, standalone DAG block save, C++ graph-add helpers, and obsolete DAG
  plan/save/add CXX exports are retired. The former add-order mutex is replaced by cursor-lifetime serialization across
  each complete C++ add flow; matching idempotent abort guards release a prepared Rust cursor if external fact lookup or
  completion throws. Rust stages transaction rows/count and DAG block/index/counters in one shared batch, commits before
  publishing either runtime, and returns only counters, queue-erasure logs, and shell effects. The public
  `TransactionManager::saveTransactionsFromDagBlock` compatibility API remains unchanged.
- Accepted-add account facts now resolve inside `dag_manager_shim.cpp` through DagManager's existing FinalChain facade.
  The private `TransactionManager::resolveDagAddBlockAccountNonceFacts` declaration, forwarding definition, and
  `TransactionManagerRustShimAccess` implementation are deleted. Indexed request order, zero-account fallback on lookup
  failure, and the missing-FinalChain exception remain unchanged without a DAG-to-transaction-manager relay.
- Verify-block tip-gas lookup is now private Rust session work. The C++ `needs_tip_gas` calculation and
  `dag_manager_runtime_tip_gas_estimations` call are deleted, along with the exported lookup, `DagTipGas`, and
  `DagVerifyBlockGasReport::tip_gas_estimations`. C++ still reports externally sourced block gas, aggregate transaction
  weight, and configured DAG/PBFT limits through the existing cursor-bound gas-report call.
- The storage differential's pure-C++ build now keeps the upstream pillar-vote bundle materialization path behind
  `!RUSTAXA_ENABLE_PILLAR_VOTES`; only feature-on builds call the shim-only optimized bundle API. This is an explicit
  guarded integration change in the upstream-owned network handler, preventing main-only pillar routing from leaking
  into the C++ reference configuration.
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
