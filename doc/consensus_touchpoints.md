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
- `NetworkEffectResult`
  - `effect_id`
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
  - It accepts only latest vote and vote-bundle packet ids (`kVotePacket = 1`, `kVotesBundlePacket = 3`) into a
    bounded Rust-owned ingress arena.
  - It exposes empty effect-drain and effect-result-reporting contracts, but no production tarcap handler is rerouted
    yet.
  - It does not call consensus shims, C++ consensus managers, `DbStorage`, peer transport, packet wrapping, or gossip.

First useful routes:

- PBFT vote and vote-bundle ingress.
- DAG block ingress and DAG sync intake.
- PBFT sync and finalized-period intake.
- Transaction gossip admission.
- Pillar vote and pillar-vote-bundle ingress.

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
  - whether Rust storage publication, pending markers, committed `StateAPI` descriptor, transaction hash rows,
    receipts, blooms, and `LAST_NUMBER` agree for the requested period.

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
