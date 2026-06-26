# Consensus Touchpoints

This inventory groups areas that still touch consensus code or consensus-owned data after the current rewrite closeout.
It is based on `PLAN.md`, `doc/consensus_rewrite_tracker.md`, and repository include/build references.

## Goals

1. **Remove internal bridge and shim routing**
   - Consensus-internal code should not route through bridge or shim compatibility APIs once the owning Rust runtime,
     storage port, or domain service exists.
   - Internal callers should use Rust-owned modules, typed ports, runtime handles, and domain APIs directly.
   - Remaining bridge and shim code should be treated as temporary compatibility surface, not as normal internal
     architecture.
   - When internal behavior still requires a shim or bridge, that dependency should be visible, narrow, and tied to a
     specific removal condition.

2. **Create dedicated external APIs for the major external clients**
   - External boundaries should use small, purpose-built APIs instead of broad consensus object access.
   - The first focus areas are:
     - network and tarcap transport
     - external EVM, `StateAPI`, and state DB mutation
     - RPC, GraphQL, plugins, and public query APIs
   - These APIs should be minimal and practical: expose only the facts, commands, requests, reports, or DTOs each
     external client actually needs.
   - The target shape is a thin interface that makes it easy to identify which behavior still needs a shim or bridge,
     why it exists, and when it can be removed.

## External Facing API Shape

These APIs are target facades for the major external clients. They should be implemented as narrow Rust-owned
interfaces with C++ adapters only at the boundary. They must not expose C++ consensus managers, mutable internal
runtime state, `DbStorage`, bridge batch ids, or legacy consensus objects as decision authority.

### 1. Network and Tarcap API

Purpose: let tarcap deliver canonical packet bytes to Rust consensus and execute typed network effects without giving
the network module direct access to consensus managers.

Target facade: `ConsensusNetworkApi`

Inbound methods:

```text
create(config) -> ConsensusNetworkApi
ingest_packet(packet) -> IngressReceipt
drain_work(budget) -> NetworkEffectBatch
report_effect_results(results) -> NetworkEffectAck
```

Minimal DTOs:

- `NetworkApiConfig`
  - `max_payload_bytes`: largest canonical packet payload accepted at the boundary.
  - `max_retained_payloads`: maximum retained ingress arena entries.
  - `max_effects_per_drain`: maximum effects returned by one drain call.
- `NetworkIngressPacket`
  - `packet_type`: latest-tarcap packet type id as `u32`.
  - `peer_id`: fixed 64-byte sender node id.
  - `payload_bytes`: canonical packet bytes.
  - `received_at_mono_ms`: network-supplied monotonic receive timestamp.
  - `source_packet_id`: optional network-owned diagnostic id.
- `IngressReceipt`
  - `accepted`: whether bytes entered the Rust-owned ingress arena or queue.
  - `payload_id`: opaque id for accepted bytes.
  - `status`: stable rejection/status code for malformed or unsupported ingress.
  - `error_code`: stable textual status for boundary logs.
- `NetworkEffectBatch`
  - `status`: stable batch status code.
  - ordered list of effects for the network executor.
  - `more_available`: whether another drain may return more effects.
  - `error_code`: stable textual status for boundary logs.
  - no direct peer transport calls and no packet wrapping side effects.
- `NetworkEffect`
  - `effect_id`
  - `source_payload_id`
  - `send_packet { peer_id, packet_kind, payload_bytes }`
  - `gossip_packet { packet_kind, payload_bytes, exclude_peers }`
  - `mark_peer_known { peer_id, object_kind, object_hash }`
  - `request_sync { peer_id, sync_kind, start_period_or_level }`
  - `report_peer { peer_id, report_kind, detail_code }`
  - `disconnect_peer { peer_id, reason_code }`
  - `block_peer_order { peer_id, dependency_id }`
  - `drive_consensus_progress { period, round }`
  - `record_consensus_object { peer_id, packet_kind, object_kind, object_hash, payload_bytes }`
- `NetworkEffectResult`
  - `effect_id`
  - echoed effect identity fields: effect kind, target peer, packet kind, object kind, and object hash.
  - `status`
  - optional diagnostic text.
- `NetworkEffectAck`
  - `status`: stable acknowledgement status.
  - `accepted_results`: number of accepted result reports.
  - `failed_results`: number of failed executor reports.
  - `error_code`: stable textual status for boundary logs.

Rules:

- Network owns peer connections, packet framing, packet priority, queues, gossip fanout mechanics, and disconnect
  execution.
- Rust consensus owns packet interpretation after ingress, deterministic admission, consensus validation plans,
  and the decision to request network effects.
- Packet payloads should be canonical bytes or opaque ingress ids until materialization is unavoidable.
- This API replaces direct tarcap access to `PbftManager`, `DagManager`, `VoteManager`, `TransactionManager`,
  `PillarChainManager`, and `DbStorage` for consensus decisions.
- The first implemented direct bridge slice is intentionally narrow:
  - Rust domain facade: `rust/crates/rustaxa-consensus/src/network_api.rs`
  - CXX bridge facade: `BridgeConsensusNetworkApi` in `rust/crates/rustaxa-bridge/src/network.rs`
  - It accepts latest vote, get-next-votes sync, vote-bundle, DAG block, DAG sync, transaction, get-PBFT-sync, PBFT
    sync, get-DAG-sync, pillar vote, get-pillar-votes-bundle, pillar votes bundle, and PBFT blocks bundle packet ids
    (`kVotePacket = 1`, `kGetNextVotesSyncPacket = 2`, `kVotesBundlePacket = 3`, `kDagBlockPacket = 5`,
    `kDagSyncPacket = 6`, `kTransactionPacket = 7`, `kGetPbftSyncPacket = 10`, `kPbftSyncPacket = 11`,
    `kGetDagSyncPacket = 12`, `kPillarVotePacket = 13`, `kGetPillarVotesBundlePacket = 14`,
    `kPillarVotesBundlePacket = 15`, `kPbftBlocksBundlePacket = 16`) into a bounded Rust-owned ingress arena.
  - It exposes effect-drain and effect-result-reporting contracts used by the Rust-enabled vote, DAG block, PBFT blocks
    bundle, and transaction packet handlers.
  - Rust-enabled `TaraxaCapability::interpretCapabilityPacket` now shadow-submits peer-gated canonical packet bytes
    directly to `BridgeConsensusNetworkApi` before the legacy tarcap thread-pool enqueue.
  - Shadow ingress is non-authoritative: unsupported packet types are rejected by the API, accepted packet bytes are
    retained only by the bounded Rust ingress arena, and legacy tarcap handlers continue to execute exactly as before.
  - PBFT vote and vote-bundle ingress planning now routes through methods on `BridgeConsensusNetworkApi` rather than
    standalone vote-planner bridge helpers. Tarcap still supplies decoded scalar vote facts and local PBFT/network
    window context, but the packet-adjacent accept/reject and sync-hint decision is owned by the external
    Network/Tarcap facade.
  - PBFT vote and vote-bundle rejection decisions now have an authoritative network API route:
    `consensus_network_ingest_pbft_vote` / `consensus_network_ingest_pbft_vote_bundle_member` return a
    `NetworkIngressDecision` and queue `REQUEST_SYNC`, `REPORT_PEER`, and `DISCONNECT_PEER` effects for
    `drain_work` / `report_effect_results`.
  - Accepted PBFT vote packets now enter deterministic admission directly in `ExtVotesPacketHandler` via
    `vote_mgr_->addVerifiedVoteWithReport`, eliminating the temporary `consensus_network_queue_pbft_vote_admission_request_effects`
    hop for this decision boundary. Tarcap still uses network effect draining/reports for peer-visible follow-up actions.
  - Rust-enabled single-vote and vote-bundle handling no longer perform direct `VoteManager::voteAlreadyValidated`
    pre-checks outside the common admission path; duplicate/non-admitted votes flow through the same direct
    Rust-managed admission/reporting path in tarcap's vote handlers.
  - Accepted PBFT vote admission now marks the peer as having seen the vote directly in tarcap after successful
    `addVerifiedVoteWithReport` admission.
  - Accepted PBFT vote packets with attached PBFT block sidecars now call `pbft_mgr_->processProposedBlock` directly and
    mark that PBFT block as known directly on the peer; no temporary vote/block `MARK_PEER_KNOWN` queue path is used.
  - Accepted PBFT vote gossip can now queue `GOSSIP_PACKET` through
    `consensus_network_queue_pbft_vote_gossip_effects`; tarcap executes the effect with existing peer filtering,
    optional block-sidecar packet wrapping, send policy, and peer known-cache updates instead of calling
    `PbftManager::gossipVote` in Rust-enabled latest vote handling.
  - Get-next-votes egress now runs directly in `GetNextVotesBundlePacketHandler`: tarcap supplies the local previous PBFT
    round and reads candidate next-vote/full-null votes directly from `VoteManager` before sending `VotesBundlePacket`.
  - Get-PBFT-sync egress now runs directly in `GetPbftSyncPacketHandler`: tarcap supplies the requested start period,
    transfer count, and synced-chain flag before directly reading period data, building `PbftSyncPacket` payloads, sending
    current proposed blocks when needed, and updating the peer sync state.
  - Accepted PBFT vote proposed-block sidecars now pass canonical PBFT block RLP and compact block facts to `PbftManager`
    directly in tarcap on the accepted-vote path; temporary consensus-object sidecar recording queue usage for this path is
    no longer used there.
  - PBFT blocks bundle proposed-block intake now runs directly in `PbftBlocksBundlePacketHandler`: tarcap supplies each proposed
    block from the packet to `PbftManager` for direct processing.
  - Transaction packet admission now executes directly in `TransactionPacketHandler` against `TransactionManager` for
    verification and insertion, eliminating the temporary `consensus_network_queue_transaction_admission_request_effects`
    executor hop. The handler now reports validation and overflow handling directly through existing peer telemetry.
  - DAG block intake now bypasses `consensus_network_queue_dag_block_admission_request_effects`; tarcap decodes packet
    data and calls `onNewBlockReceived` directly so `DagManager` verifies and inserts the block using the existing local DAG
    path.
  - DAG sync block intake now bypasses `consensus_network_queue_dag_sync_block_admission_request_effects`; tarcap decodes
    packet data and verifies/adds each DAG block directly in `DagSyncPacketHandler` using `DagManager`.
  - Get-DAG-sync egress now runs directly in `GetDagSyncPacketHandler`: tarcap supplies the requesting peer period and
    requested DAG block hashes before directly reading non-finalized blocks/transactions, updating peer sync state,
    materializing `DagSyncPacket`, and sending it.
  - PBFT sync finalized-period intake now bypasses `consensus_network_queue_pbft_sync_period_data_admission_request_effects`;
    tarcap validates and queues period data directly in `PbftSyncPacketHandler` through `PbftManager::periodDataQueuePush`.
  - Pillar vote and pillar votes bundle member admission now runs directly in tarcap handlers: tarcap supplies canonical pillar
    vote facts and calls `PillarChainManager::validatePillarVote` plus `addVerifiedPillarVote` directly, then marks the vote
    known on the peer.
  - Pillar vote relevance planning now routes through
    `consensus_network_plan_pillar_vote_relevance`; tarcap supplies decoded vote facts and current pillar-block context
    to the facade before it decides whether the vote is locally relevant.
  - Pillar vote duplicate/signature/eligibility validation now runs directly through `PillarChainManager::validatePillarVote`.
  - Get-pillar-votes-bundle egress now runs directly in `GetPillarVotesBundlePacketHandler`; tarcap supplies requested period and
    pillar block hash, reads verified votes directly, chunks `PillarVotesBundlePacket` payloads, sends them, and marks sent votes
    known for the peer.
  - Standard status follow-up planning now routes through `consensus_network_plan_status_sync`; tarcap supplies compact
    local/peer PBFT and DAG facts before Rust decides whether to request PBFT sync, pending DAG blocks, or PBFT
    next-votes bundles. Next-votes follow-up now executes directly in tarcap via `requestPbftNextVotesAtPeriodRound`.
    PBFT sync-start and pending-DAG follow-up are still planned by Rust and then executed directly by tarcap in
    `startSyncingPbft` and `requestPendingDagBlocks`.
  - Initial status admission now routes through `consensus_network_plan_initial_status`; tarcap supplies configured and
    peer-advertised chain id, genesis hash, PBFT sync period, peer chain size, and light-node history before Rust decides
    whether the peer should be accepted or disconnected. Tarcap still owns pending-peer lookup, peer-state
    materialization, logging, and disconnect execution.
  - Status packet egress now routes through `consensus_network_plan_status_egress`; tarcap supplies local PBFT/DAG snapshot
    facts plus node identity/config metadata before Rust shapes the initial or standard status payload. Tarcap still owns live
    snapshot reads, RLP encoding, packet framing, and transport send execution.
  - PBFT sync-start planning now routes through `consensus_network_plan_pbft_sync_start`; tarcap supplies local PBFT
    sync facts plus compact peer candidates before Rust chooses the max-chain sync peer, applies light-node history
    eligibility, decides the first requested period, and reports whether snapshot creation should be re-enabled because
    sync is not needed. Tarcap now executes selected-peer state install, `GetPbftSyncPacket` request emission, and snapshot
    toggle behavior directly after the plan is returned.
  - Pending-DAG-block request planning now routes through
    `consensus_network_plan_pending_dag_blocks_request`; tarcap supplies the local PBFT sync period plus either an
    explicit peer snapshot or compact live peer candidates before Rust selects an eligible peer and gates the PBFT-period
    match required for `GetDagSyncPacket`. Tarcap now executes live peer reservation, non-finalized DAG snapshot read,
    packet construction, and `GetDagSyncPacket` transport directly after the plan is returned.
  - Network-level max-chain peer selection now routes through
    `consensus_network_plan_max_chain_peer_selection`; `Network` supplies compact peer candidates from all active tarcap
    versions before Rust applies PBFT chain size, DAG-level tie-breaking, and light-node history eligibility. `Network`
    still performs live peer lookup and dispatches `GetPillarVotesBundlePacket` through the selected tarcap handler.
  - Network effect result reports now echo typed effect identity fields, and Rust rejects mismatched reports before
    accepting acknowledgements. This keeps temporary executor work visible instead of treating an `effect_id` alone as
    proof that the intended action ran.
  - This is still not the final production route: accepted PBFT vote gossip still uses the temporary
    `consensus_network_queue_pbft_vote_gossip_effects` / `NetworkEffect` queue so Rust can request gossip while tarcap
    owns peer filtering, packet wrapping, and transport execution.
  - Status packet ingress still performs pending-peer lookup and peer-state materialization in tarcap. Status egress
    still reads local PBFT/DAG snapshot facts directly until the facade is injected with Rust-owned local status
    snapshot state.
  - The facade methods themselves do not call consensus shims, C++ consensus managers, `DbStorage`, peer transport,
    packet wrapping, or gossip.
First useful routes:

- PBFT vote and vote-bundle ingress.
- Transaction and DAG block direct intake.
- PBFT sync and DAG sync egress.
- Pillar vote and pillar-vote-bundle ingress.
- Status follow-up control actions (PBFT sync-start, pending DAG request, next-votes request) now execute directly in tarcap after Rust planning.

### 2. External EVM, StateAPI, and State DB API

Purpose: let Rust FinalChain and consensus plan execution/publication work while C++/EVM remains the executor for
arbitrary contract execution and committed state mutation.

Target facade: `ConsensusExecutionApi`

Request methods:

```text
next_execution_request(period) -> Option<ExternalExecutionRequest>
report_execution_result(result) -> ExecutionReportAck
next_state_commit_request() -> Option<StateCommitRequest>
report_state_commit_result(result) -> StateCommitAck
publication_audit(period) -> PublicationAudit
```

Minimal DTOs:

- `ExternalExecutionRequest`
  - `request_id`
  - `period`
  - `pbft_block_hash`
  - `previous_state_root`
  - ordered transaction RLPs, including regular and Rust-planned system transactions.
  - execution environment facts needed by EVM.
  - expected request identity hash.
- `ExternalExecutionResult`
  - `request_id`
  - ordered receipt facts.
  - cumulative gas facts.
  - post-execution root.
  - execution status for every requested transaction.
  - diagnostic text for executor failures.
- `StateCommitRequest`
  - `commit_id`
  - `request_id`
  - `period`
  - `pre_commit_root`
  - `post_execution_root`
  - `post_rewards_root`
  - publication block hash.
- `StateCommitResult`
  - `commit_id`
  - committed descriptor.
  - status and diagnostic text.
- `PublicationAudit`
  - whether Rust storage publication, pending markers, committed `StateAPI` descriptor period/root, transaction hash
    rows, receipts, blooms, and `LAST_NUMBER` agree for the requested period.

Rules:

- Rust owns request identity, ordered transaction selection, supported system transaction planning, report validation,
  rewards/state-root boundary decisions, storage publication planning, and recovery classification.
- External EVM owns arbitrary contract execution, state transition execution, low-level receipt/log execution details,
  and committed state mutation.
- C++ may adapt `StateAPI` into this facade, but consensus code should only see request/report/commit DTOs.
- No consensus-internal code should call `StateAPI` directly once an execution API route exists.

First useful routes:

- External-EVM execution requests for finalization sessions.
- Execution report validation and publication planning.
- State commit result reporting.
- Publication recovery and audit after restart.

Implemented first slice:

- Rust domain facade: `rust/crates/rustaxa-consensus/src/consensus_execution_api.rs`
- CXX bridge facade: `BridgeConsensusExecutionApi` in `rust/crates/rustaxa-bridge/src/final_chain.rs`
- Rust-enabled `FinalChain::finalizeExternalEvm` now holds a dedicated execution API handle and routes the external
  EVM/session boundary through it for:
  - next execution/action requests
  - system-transaction report validation
  - arbitrary EVM execution report validation
  - rewards execution report validation and commit-plan derivation
  - external-EVM publication planning
  - rewards-stat and proposal-period publication attachments
  - state-commit intent creation
  - pending-publication marker persistence
  - state-commit result reporting
  - Rust FinalChain storage publication
- `ConsensusExecutionApi` is intentionally stateless. C++ still passes the live `BridgeFinalChain` and
  `BridgeFinalChainExecutionSession` handles, while Rust owns request identity, report validation, publication plans,
  pending marker handling, storage publication, and publication audit decisions.
- A CXX-facing `FinalChainExternalEvmPublicationAuditReport` is now available through
  `consensus_execution_publication_audit`, making restart/publication verification part of the external execution facade
  instead of a test-only bridge helper.
- The facade audit accepts the committed `StateAPI` descriptor and verifies the descriptor period and committed root
  against the Rust publication plan before checking persisted FinalChain rows. The lower-level storage-only audit remains
  available for Rust bridge tests that intentionally do not model the external state commit boundary.
- The facade does not call `StateAPI`, execute EVM, mutate `state_db/`, read bridge-contract state, or own rewards
  execution. Those remain the external executor responsibilities for this section.
- Native-only FinalChain commits still use the existing Rust session commit helper because they are not part of the
  external EVM/StateAPI boundary.

Implemented second slice:

- Shim-owned executor adapter: `FinalChain::ExternalEvmStateApiClient` in the Rust-mode FinalChain shim.
- Rust-enabled `FinalChain::finalizeExternalEvm` now consumes adapter outcomes instead of calling `StateAPI` directly for:
  - bridge-contract fact collection for Rust-planned system transactions
  - arbitrary external EVM transaction execution
  - external reward distribution
  - staged `state_db/` commit
  - committed-state descriptor reads used by pending-publication recovery
- Direct `StateAPI::execute_transactions`, `StateAPI::distribute_rewards`, and `StateAPI::transition_state_commit` calls
  are confined to the adapter. The consensus flow sees Rust bridge request/report/commit DTOs plus temporary public
  `FinalizationResult` materialization.
- The adapter remains intentionally C++ shim-owned because arbitrary EVM execution and `state_db/` mutation are still
  external to the Rust consensus rewrite. It must not publish Rust FinalChain storage or decide session state; those stay
  behind `ConsensusExecutionApi`.

Implemented third slice:

- The same shim-owned `ExternalEvmStateApiClient` now owns the remaining Rust-mode `StateAPI` read boundary in
  FinalChain:
  - account, storage, and code reads
  - read-only external EVM dry-run calls
  - trace calls for committed external-EVM blocks
  - bridge-contract root/epoch reads
  - `StateAPI` config updates
- Direct `state_api_` access in the Rust-mode FinalChain shim is now confined to adapter implementation and construction.
  Public FinalChain methods either call Rust-owned storage/FinalChain APIs or this explicit external StateAPI adapter.
- These read routes are still external-client compatibility, not Rust consensus ownership. Section 3 should decide which
  public query views move behind a future `ConsensusQueryApi`.

### 3. Public Query API

Purpose: serve RPC, GraphQL, plugins, debug, and CLI read paths without exposing consensus managers, storage internals,
or legacy public objects as the query authority.

Target facade: `ConsensusQueryApi`

Read methods:

```text
node_consensus_status() -> ConsensusStatusView
pbft_block(query) -> Option<PbftBlockView>
pbft_blocks(range) -> Page<PbftBlockView>
dag_block(query, include_transactions) -> Option<DagBlockView>
dag_blocks(query, page) -> Page<DagBlockView>
transaction(query) -> Option<TransactionView>
transaction_receipt(hash) -> Option<ReceiptView>
final_chain_block(query) -> Option<FinalChainBlockView>
account_state(query) -> AccountStateView
pillar_block(period) -> Option<PillarBlockView>
sync_state() -> SyncStateView
```

Minimal DTOs:

- `ConsensusStatusView`
  - PBFT period, round, step.
  - PBFT chain size.
  - finalized block number/hash.
  - DAG max level and latest period.
  - sync state summary.
- `PbftBlockView`
  - period, hash, previous hash, pivot hash, author, timestamp.
  - optional canonical RLP for clients that still need legacy encoding.
  - vote and reward summaries only when requested.
- `DagBlockView`
  - hash, level, period if finalized, pivot/tips, author, gas, transaction hashes.
  - optional transaction views when requested.
- `TransactionView`
  - hash, sender, nonce, gas, value, status, source location.
  - optional canonical RLP.
- `ReceiptView`
  - transaction hash, status, gas used, logs, bloom, block/period location.
- `FinalChainBlockView`
  - number, hash, state root, receipts root, transactions root, timestamp, author.
- `AccountStateView`
  - balance, nonce, code hash, DPoS snapshot facts when requested.
- `PillarBlockView`
  - period, hash, parent, validator vote-count deltas, signature summary.
- `SyncStateView`
  - PBFT sync, DAG sync, peer progress, local head/finalized facts.

Rules:

- Query APIs are read-only and should not drive consensus progress.
- Query callers receive stable DTOs, canonical bytes, or paged views. They should not receive manager pointers,
  storage iterators, bridge batch ids, or mutable sidecars.
- RPC, GraphQL, plugin, debug, and CLI layers may format DTOs into JSON or legacy objects, but formatting should be
  outside consensus.
- Query APIs may retain compatibility fields temporarily, but each compatibility field should correspond to a named
  public caller.

First useful routes:

- Replace `taraxa_getDagBlockByHash`, `taraxa_getDagBlockByLevel`, PBFT block lookups, and pillar block reads.
- Replace RPC/GraphQL direct `PbftManager`, `DagManager`, `TransactionManager`, `FinalChain`, and `DbStorage` reads.
- Provide a single status/sync view for metrics, RPC, GraphQL, and debug surfaces.

Implemented first slice:

- Rust domain facade: `rust/crates/rustaxa-consensus/src/consensus_query_api.rs`
- CXX bridge facade: `BridgeConsensusQueryApi` in `rust/crates/rustaxa-bridge/src/query.rs`
- The facade is read-only and owns only a cloned Rust storage handle. Public adapters create it from `BridgeStorage` and
  receive stable DTOs; they do not receive consensus manager pointers, storage iterators, batches, or mutable sidecars.
- Implemented routes:
  - `consensus_query_pbft_block_hash_by_period(period) -> HashLookup`
  - `consensus_query_final_chain_block_by_number(number) -> FinalChainBlockView`
  - `consensus_query_pbft_schedule_block_by_period(period) -> PbftScheduleBlockView`
  - `consensus_query_pbft_node_version_by_period(period) -> PbftNodeVersionView`
  - `consensus_query_pillar_block_data_by_period(period) -> PillarBlockDataView`
  - `consensus_query_dag_block_by_hash(hash) -> DagBlockPublicView`
  - `consensus_query_dag_blocks_by_level(level, number_of_levels) -> Vec<DagBlockPublicView>`
  - `consensus_query_finalized_dag_blocks_by_period(period) -> Vec<DagBlockPublicView>`
  - `consensus_query_transaction_by_hash(hash) -> TransactionPublicView`
  - `consensus_query_transaction_by_block_number_and_index(block_number, transaction_index) -> TransactionPublicView`
  - `consensus_query_transaction_by_block_hash_and_index(block_hash, transaction_index) -> TransactionPublicView`
  - `consensus_query_transaction_count_by_block_number(block_number) -> u64`
  - `consensus_query_transaction_count_by_block_hash(block_hash) -> u64`
  - `consensus_query_transaction_receipt_by_hash(hash) -> TransactionReceiptPublicView`
  - `consensus_query_transaction_receipts_by_block_number(block_number) -> Vec<TransactionReceiptPublicView>`
  - `consensus_query_final_chain_block_number_by_hash(block_hash) -> FinalChainBlockNumberLookup`
  - `consensus_query_final_chain_last_block_number() -> u64`
  - `consensus_query_final_chain_blocks_with_bloom(bloom, from, to) -> Vec<u64>`
- `taraxa_pbftBlockHashByPeriod` and GraphQL final-chain block composition now use `ConsensusQueryApi` in Rust mode for
  PBFT hash-by-period lookup plus finalized block header, hash-to-number, and latest-block reads instead of creating
  endpoint-local period-storage query handles or reading `FinalChain` directly for those facts.
- GraphQL `Block.transactionCount`, `Block.transactions`, and `Block.transactionAt` now use `ConsensusQueryApi` in Rust
  mode for finalized transaction counts, indexed transaction payloads, and receipt DTOs instead of lazy-loading
  `FinalChain` transaction vectors and receipts from the block object.
- GraphQL `nodeState.finalBlock` now uses `ConsensusQueryApi` in Rust mode for the finalized head number instead of
  reading `FinalChain` directly. `nodeState.dagBlockLevel` and `nodeState.dagBlockPeriod` remain live DAG-manager
  compatibility reads until a dedicated consensus status DTO owns the mixed live/finalized status view.
- `taraxa_getScheduleBlockByPeriod` now uses `ConsensusQueryApi` for PBFT schedule block facts and finalized DAG order
  in Rust mode instead of creating an endpoint-local period-storage query handle.
- `taraxa_getNodeVersions` now uses `ConsensusQueryApi` for PBFT block author/version facts in Rust mode instead of
  creating an endpoint-local period-storage query handle. The route intentionally leaves scan policy, version string
  formatting, and DPoS vote-count aggregation in the public RPC layer until live FinalChain/state reads move behind a
  dedicated query view.
- `taraxa_getPillarBlockData` now uses `ConsensusQueryApi` for finalized pillar block facts and the following period's
  optimized pillar-vote bundle in Rust mode instead of creating an endpoint-local pillar storage query handle.
- `taraxa_getDagBlockByHash` and `taraxa_getDagBlockByLevel` now use `ConsensusQueryApi` for DAG block facts and
  finalized period/position lookup in Rust mode instead of creating endpoint-local DAG query handles and asking the live
  `PbftManager` for block period.
- `debug_getPeriodDagBlocks` and GraphQL `periodDagBlocks` now use `ConsensusQueryApi` for finalized DAG block facts by
  PBFT period in Rust mode instead of creating endpoint-local period-storage query handles. GraphQL DAG objects also use
  the finalized-period facts carried by Rust DTOs before falling back to live PBFT-manager lookup for older routes.
- GraphQL `dagBlock` and `dagBlocks` now use `ConsensusQueryApi` for DAG hash, latest-level, and paged level-window
  reads in Rust mode instead of creating endpoint-local DAG storage query handles.
- GraphQL top-level `transaction(hash)` now uses `ConsensusQueryApi` for storage-backed transaction payload lookup in
  Rust mode instead of asking the live `TransactionManager` to resolve the hash. The query view returns canonical RLP
  plus source classification for pending, finalized regular, and finalized system transaction materialization.
- GraphQL top-level `transaction(hash)` expanded receipt fields (`status`, `gasUsed`, `cumulativeGasUsed`,
  `createdContract`, and `logs`) now consume the `ConsensusQueryApi` transaction-receipt DTO in Rust mode instead of
  lazy-loading the transaction location and receipt through `FinalChain`.
- `eth_getTransactionByHash` and `eth_getTransactionReceipt` now use `ConsensusQueryApi` in Rust mode for
  location-aware transaction payload and receipt lookup instead of calling the generic ETH transaction callback or
  reading `FinalChain` transaction location/receipt rows directly.
- `eth_getTransactionByBlockNumberAndIndex` and `eth_getTransactionByBlockHashAndIndex` now use `ConsensusQueryApi` in
  Rust mode for indexed finalized transaction payload lookup instead of reading `FinalChain` transaction vectors and
  block hash/number indexes directly.
- `eth_getBlockTransactionCountByNumber` and `eth_getBlockTransactionCountByHash` now use `ConsensusQueryApi` in Rust
  mode for finalized transaction counts instead of reading `FinalChain` transaction-count and block-number indexes
  directly.
- `eth_getBlockReceipts` now uses `ConsensusQueryApi` in Rust mode for finalized regular-transaction receipt expansion
  instead of reading `FinalChain` block hashes, transaction vectors, block receipt lists, and per-transaction receipts
  directly.
- `eth_getBlockByNumber` and `eth_getBlockByHash` now use `ConsensusQueryApi` in Rust mode for finalized block-header
  views, hash-to-number resolution, transaction counts, and optional indexed transaction expansion instead of reading
  `FinalChain` block headers, block-number indexes, transaction vectors, and transaction hashes directly.
- `eth_blockNumber` now uses `ConsensusQueryApi` in Rust mode for the latest finalized block number instead of reading
  `FinalChain` directly.
- GraphQL `syncing.currentBlock` now uses `ConsensusQueryApi` in Rust mode for the latest finalized block number
  instead of reading `FinalChain` directly. `syncing.highestBlock` remains a network peer-progress view until the
  network-backed `SyncStateView` route exists.
- `debug_getPeriodTransactionsWithReceipts` now uses the same `ConsensusQueryApi` block-receipts DTO in Rust mode
  instead of reading period transactions and receipts through `DbStorage`/`FinalChain`.
- `eth_getLogs` and installed `eth_getFilterLogs` replay now use `ConsensusQueryApi` in Rust mode for latest finalized
  block lookup, bloom-index candidate block lookup, and block receipt expansion instead of asking `FinalChain` for bloom
  matches and receipt rows directly. Live subscription delivery still remains on the execution-event compatibility route.
- The first `FinalChainBlockView` route returns finalized block number/hash, stored header roots, bloom/gas/reward facts,
  canonical stored-header bytes, and optional PBFT hash. It intentionally does not expand transactions, receipts, logs,
  account state, DPoS snapshots, or external `StateAPI` reads.
- Existing account-state, live subscription log delivery, debug log filtering, and sync/status routes remain
  compatibility or typed-storage routes until they are moved behind `ConsensusQueryApi` in later slices.

## Consensus Internal

Internal areas are consensus-owned or consensus-adjacent code that has already been rewritten in Rust, is actively
shimmed toward Rust, or can reasonably continue moving into Rust as part of the rewrite track.

1. **PBFT manager, PBFT chain, proposed blocks, and period data queue**
   - `libraries/core_libs/consensus/include/pbft/`
   - `libraries/core_libs/consensus/src/pbft/`
   - `libraries/core_libs/consensus/shims/pbft_manager_shim/`
   - `libraries/core_libs/consensus/shims/pbft_chain_shim/`
   - `libraries/core_libs/consensus/shims/proposed_blocks_shim/`
   - `libraries/core_libs/consensus/shims/period_data_queue_shim/`
   - `rust/crates/rustaxa-consensus/src/pbft_manager.rs`
   - `rust/crates/rustaxa-consensus/src/pbft_chain.rs`
   - `rust/crates/rustaxa-consensus/src/pbft_sync.rs`
   - `rust/crates/rustaxa-consensus/src/pbft_finalize.rs`
   - Main protocol brain. It is largely Rust-planned now, with remaining executor and materialization edges.

2. **DAG manager, DAG graph, DAG proposer, sortition, and VDF**
   - `libraries/core_libs/consensus/include/dag/`
   - `libraries/core_libs/consensus/src/dag/`
   - `libraries/core_libs/consensus/shims/dag_shim/`
   - `libraries/core_libs/consensus/shims/dag_manager_shim/`
   - `libraries/core_libs/consensus/shims/dag_block_proposer_shim/`
   - `libraries/core_libs/consensus/shims/sortition_params_manager_shim/`
   - `libraries/vdf/`
   - `rust/crates/rustaxa-consensus/src/dag.rs`
   - `rust/crates/rustaxa-vdf/`
   - Touches block verification, proposal building, expiry/finalization cleanup, VDF proof work,
     and proposer scheduling.

3. **Vote manager, verified votes, and PBFT vote pipeline**
   - `libraries/core_libs/consensus/include/vote_manager/`
   - `libraries/core_libs/consensus/src/vote_manager/`
   - `libraries/core_libs/consensus/shims/vote_manager_shim/`
   - `libraries/core_libs/consensus/shims/verified_votes_shim/`
   - `libraries/types/vote/`
   - `rust/crates/rustaxa-consensus/src/pbft_vote_*.rs`
   - `rust/crates/rustaxa-bridge/src/pbft_vote_*.rs`
   - Covers validation, replay protection, 2t+1 thresholding, reward votes, vote generation,
     vote bundles, gossip effects, and slashing evidence payloads.

4. **Transaction manager, transaction queue, and gas pricer**
   - `libraries/core_libs/consensus/include/transaction/`
   - `libraries/core_libs/consensus/src/transaction/`
   - `libraries/core_libs/consensus/shims/transaction_manager_shim/`
   - `libraries/core_libs/consensus/shims/transaction_queue_shim/`
   - `libraries/core_libs/consensus/shims/gas_pricer_shim/`
   - `rust/crates/rustaxa-consensus/src/transaction_manager.rs`
   - `rust/crates/rustaxa-consensus/src/transaction_queue.rs`
   - `rust/crates/rustaxa-consensus/src/transaction_storage.rs`
   - Used by consensus for admission, proposal packing, finalized status, gas policy, and transaction materialization.
   - EVM gas estimation execution remains external.

5. **FinalChain and DPoS fact ports**
   - `libraries/core_libs/consensus/include/final_chain/`
   - `libraries/core_libs/consensus/src/final_chain/`
   - `libraries/core_libs/consensus/shims/final_chain_shim/`
   - `rust/crates/rustaxa-consensus/src/final_chain.rs`
   - `rust/crates/rustaxa-consensus/src/final_chain_execution.rs`
   - `rust/crates/rustaxa-bridge/src/final_chain.rs`
   - Internal for consensus-facing reads, DPoS snapshots, finalization planning, publication planning,
     rewards/state-root validation, and typed execution reports.
   - Arbitrary EVM execution and `StateAPI` mutation remain external.

6. **Storage ports used by consensus**
   - `libraries/core_libs/consensus/shims/storage_shim/`
   - `rust/crates/rustaxa-storage/src/dag.rs`
   - `rust/crates/rustaxa-storage/src/pbft.rs`
   - `rust/crates/rustaxa-storage/src/transaction.rs`
   - `rust/crates/rustaxa-storage/src/final_chain.rs`
   - Production consensus storage is mostly Rust-owned. Remaining storage-shim work should shrink as callers move to
     Rust-owned runtimes, read APIs, fixtures, or executor boundaries.
   - Storage admin, migration, broad query, and network compatibility shells are external support surfaces.

7. **Slashing manager**
   - `libraries/core_libs/consensus/include/slashing_manager/`
   - `libraries/core_libs/consensus/src/slashing_manager/`
   - `libraries/core_libs/consensus/shims/slashing_manager_shim/`
   - Internal for deterministic proof planning, duplicate-proof checks, normalized vote evidence, submitter selection,
     and calldata construction.
   - Transaction insertion, signing, gas bidding, and external StateAPI/finalization facts remain boundary work.

8. **Pillar chain and pillar votes**
   - `libraries/core_libs/consensus/include/pillar_chain/`
   - `libraries/core_libs/consensus/src/pillar_chain/`
   - `libraries/core_libs/consensus/shims/pillar_chain_manager_shim/`
   - `libraries/core_libs/consensus/shims/pillar_votes_shim/`
   - Internal for PBFT sync validation, pillar-vote inspection, vote aggregation, pillar-block planning,
     and Rust-owned deterministic relevance checks.
   - Network gossip, signing execution, and some FinalChain/state facts remain boundary work.

9. **Rewards and block stats**
   - `libraries/core_libs/consensus/include/rewards/`
   - `libraries/core_libs/consensus/src/rewards/`
   - `libraries/core_libs/consensus/shims/rewards_stats_shim/`
   - Internal for reward-vote facts, DAG/block stats, fee distribution planning, DPoS snapshots,
     interval cache behavior, and native finalization integration.
   - Legacy `BlockStats` carrier compatibility should keep shrinking.

10. **Key manager and signing boundary**
    - `libraries/core_libs/consensus/include/key_manager/`
    - `libraries/core_libs/consensus/src/key_manager/`
    - `libraries/core_libs/consensus/shims/key_manager_shim/`
    - Consensus depends on local vote signing, DAG/PBFT proposal signing, pillar signing, and transaction/slashing signing.
    - The decision of what to sign can be Rust-owned; the concrete signing executor may remain a boundary.

11. **Config, genesis, and hardfork parameters**
    - `libraries/config/`
    - Touches PBFT config, DAG config, genesis DAG block, gas limits, rewards distribution frequency,
      hardfork behavior, pillar intervals, Cacti lambda, and DPoS parameters.
    - These are consensus inputs and can be moved behind typed Rust config/fact DTOs as needed.

12. **Metrics and observability**
    - `libraries/metrics/include/metrics/pbft_metrics.hpp`
    - App and network logging surfaces.
    - Reads PBFT state and exposes consensus status. It is not a blocker, but Rust-owned status snapshots can replace
      direct C++ reads over time.

13. **Rust bridge and domain crates**
    - `rust/crates/rustaxa-bridge/`
    - `rust/crates/rustaxa-consensus/`
    - `rust/crates/rustaxa-types/`
    - `rust/crates/rustaxa-storage/`
    - `rust/crates/rustaxa-vdf/`
    - Rust-side consensus implementation, FFI DTO boundary, codecs, storage, and VDF/sortition implementation.

14. **Consensus tests and fixtures**
    - `tests/*pbft*`
    - `tests/*dag*`
    - `tests/*vote*`
    - `tests/*transaction*`
    - `tests/*final_chain*`
    - `tests/rust/consensus/`
    - `tests/rust/storage/`
    - `tests/test_util/`
    - Internal validation coverage and compatibility pressure. Tests should follow the rewrite path rather than preserve
      C++ decision authority.

## External Boundaries

External areas are out of scope for the current consensus rewrite. Rust consensus may return typed effects, facts,
requests, or DTOs for these boundaries, but should not absorb their ownership as part of consensus work.

1. **Network and tarcap transport**
   - `libraries/core_libs/network/`
   - Handles peer transport, packet wrapping, gossip fanout, send policy, known-peer marking, packet queues,
     disconnect/report mechanics, and sync traffic execution.
   - Consensus should expose ingress/event planners and typed egress effects instead of owning transport.

2. **External EVM, StateAPI, and state DB mutation**
   - `libraries/core_libs/consensus/include/final_chain/state_api.hpp`
   - `libraries/core_libs/consensus/src/final_chain/state_api.cpp`
   - `submodules/taraxa-evm/`
   - Includes arbitrary contract execution, state transition execution, receipt/log execution details, bridge-heavy
     state reads, and committed state mutation.
   - Rust consensus and FinalChain may plan requests and validate reports, but EVM execution stays outside consensus.

3. **RPC, GraphQL, plugins, and public query APIs**
   - `libraries/core_libs/network/rpc/`
   - `libraries/core_libs/network/graphql/`
   - `libraries/plugin/rpc/`
   - `libraries/plugin/light/`
   - Materializes DAG, PBFT, transaction, final-chain, and pillar data for external callers.
   - These should move to Rust read/query APIs where useful, but public API ownership is not consensus protocol work.

4. **App and node host lifecycle**
   - `libraries/app/src/app.cpp`
   - `libraries/app/include/app/app.hpp`
   - Owns construction, dependency wiring, daemon startup/shutdown, subscriptions, event dispatch infrastructure,
     and network attachment.
   - Consensus may provide runtime handles and planned lifecycle commands, but host mechanics stay outside consensus.

5. **Public object and compatibility materialization**
   - `libraries/types/dag_block/`
   - `libraries/types/pbft_block/`
   - `libraries/types/transaction/`
   - `libraries/types/vote/`
   - C++ `DagBlock`, `PbftBlock`, `PeriodData`, `Transaction`, `PbftVote`, `PillarVote`, bundles, and receipts remain
     necessary at public, network, test, EVM/executor, and compatibility edges.
   - They should not become authoritative consensus decision state again.

6. **Storage admin, migration, broad query, and compatibility shells**
   - `libraries/core_libs/storage/`
   - Includes lifecycle/admin behavior, storage migration, broad iterator/query APIs, and compatibility materialization
     for external callers.
   - Production consensus storage should keep using typed Rust storage ports instead.

7. **CLI, admin, and debug tooling**
   - `libraries/cli/`
   - RPC debug surfaces.
   - Reads raw DAG, PBFT, and storage data for administrative flows.

8. **Deployment and config templates**
   - `charts/taraxa-node/templates/*consensus*`
   - `charts/taraxa-node/templates/*transaction-generation*`
   - Do not own protocol logic, but configure and expose consensus-node behavior.

## Practical Summary

The remaining long-lived external executor boundaries are network/tarcap transport, external EVM/StateAPI execution,
public API/query materialization, and host lifecycle mechanics. Consensus-internal follow-up should focus on shrinking
compatibility surfaces around PBFT, DAG, votes, transactions, FinalChain facts, storage ports, pillar, slashing, rewards,
signing decisions, config inputs, and Rust-owned tests without re-centering behavior in C++.
