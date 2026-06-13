# Rust Rewrite Plan

This is the consolidated plan for progressively replacing C++ internals with Rust while keeping the node buildable, testable, and syncable with upstream.

## Objectives

- Keep upstream C++ sync practical through a pure-C++ validation gate.
- Preserve public C++ APIs while Rust implementations replace internals behind shims.
- Move high-value storage and FinalChain paths first.
- Use existing Rust rewrite implementations as aggressively as correctness allows, so each slice moves production routing
  toward the long-term goal of complete Rust ownership rather than rebuilding orchestration in C++.
- Keep Rust APIs type-safe, throughput-conscious, and unit-testable.
- Maintain C++ vs Rust behavioral parity with focused tests and conformance checks.

## Branch and Gate Model

| Branch | Role | Rule |
| --- | --- | --- |
| `upstream-main` | Upstream mirror | Clean 1:1 upstream C++ mirror; no local commits. |
| `cpp-reference` | Validation gate | C++ code plus integration hooks; verify with `RUSTAXA_ENABLE=0`. |
| `main` | Rewrite branch | Primary Rust rewrite branch and future source of truth. |

The normal upstream flow is:

1. Update `upstream-main` from upstream.
2. Merge `upstream-main` into `cpp-reference`.
3. Preserve shim/`#ifdef` structure while updating legacy C++ logic.
4. Build and test `cpp-reference` in pure C++ mode.
5. Merge `cpp-reference` into a temporary sync branch from `main`.
6. Port changed logic to Rust.
7. Validate Rust-enabled mode and merge to `main`.

When Rust feature work on `main` touches C++ intersection files, carry the relevant C++ side back to `cpp-reference`:

```bash
make cpp-intersection-list
make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>
```

## Architecture Direction

Use incremental shims at the C++ boundary and idiomatic Rust composition internally.

Core rules:

- Public C++ interfaces remain stable while implementation moves.
- C++ shims route selected APIs to Rust and keep legacy code available for validation.
- For upstream-owned C++ classes, use the overlay shim pattern by default (as in storage and FinalChain): header overlay + shim facade + legacy `*Old` compilation rename. Prefer this over scattered inline `#ifdef` edits to reduce upstream merge conflicts.
- Treat the full overlay shim as the first design step, not a later cleanup. Before adding Rust-mode behavior to an
  upstream-owned C++ class, create or extend the class overlay (`shims/<class>_shim/include/.../<class>.hpp`), compile
  the legacy implementation as `<Class>Old`, and keep Rust routing, shim-only helper methods, temporary stubs, and
  parity scaffolding in shim-owned files. Do not add Rust-only methods, `ForRust` hooks, bridge includes, or scattered
  `#ifdef` branches to original upstream headers/sources unless the task owner explicitly approves a temporary guarded
  hook. Closeout for touched upstream-owned files should include `git diff upstream-main -- <original C++ paths>`; the
  expected result is empty or an explicitly documented temporary exception.
- Hard rule: when a dependency or subsystem already has a Rust rewrite path, new rewrite work should leverage that Rust
  implementation directly instead of re-centering behavior in C++. Prefer extending Rust crates, bridges, and shim-owned
  Rust handles over adding C++ orchestration or C++ data materialization, unless a concrete blocker is documented and
  accepted by the task owner.
- Logging and observability are not architectural blockers for Rust ownership. Do not keep deterministic consensus
  behavior in C++ merely because the legacy implementation logs at that point. Rust planners may return typed statuses,
  telemetry facts, or executor reports that C++ logs temporarily, and logging can be moved, changed, or dropped in a
  later observability cleanup without affecting the ownership decision.
- Before selecting or implementing a rewrite slice, proactively inspect adjacent Rust crates, bridge APIs, shim-owned
  handles, and existing storage/FinalChain/DAG/transaction/vote coverage for reuse opportunities. Prefer connecting the
  new path to those Rust implementations, even if that makes the slice slightly larger, when it reduces future C++
  ownership and keeps behavior on the path to full Rust replacement.
- Future network ingress will use an application-owned arena/data pipeline: the latest tarcap payload bytes enter Rust
  once, receive a long-lived payload reference id, and then move through single-owner pipeline stages by payload
  reference id. Large derived facts may live in additional arenas and be referenced from small payload-reference
  metadata. Until that API lands, new consensus rewrite work should still be shaped for it: preserve canonical bytes,
  decode late, avoid unnecessary copies, avoid eager C++ object materialization, and return typed decisions/effects that
  a future network egress pipeline can execute.
  The planned initial CXX bridge entry point is:
  `pub fn ingest_network_packet(self: &mut BridgeNetwork, packet_type: u8, from_node: [u8; 64], data: Vec<u8>) -> Result<bool>;`.
  This latest-tarcap-only API reports ingestion success only: `true` means the payload bytes were accepted into the
  application arena/pipeline, while later protocol, consensus, peer, gossip, drop, or disconnect outcomes are emitted by
  downstream pipeline stages.
- The arena direction is not a single consensus pipeline. Current tarcap scheduling has three priority lanes, but the
  rewrite should model seven logical data pipelines over those lanes: peer status/sync control, transaction gossip and
  admission, DAG block gossip and admission, DAG sync, PBFT vote and round progress, PBFT chain sync/finalized-period
  intake, and pillar vote/bundle handling. Cross-pipeline impact must stay explicit in typed effects: deep PBFT sync
  filters most traffic, transaction ingress can peer-order block later DAG blocks from the same peer, DAG gaps trigger
  DAG sync, status can trigger PBFT or DAG sync, votes drive PBFT round/finalization progress and slashing, PBFT sync
  feeds the PBFT manager period-data queue, and pillar votes/data affect PBFT period validation.
  The intended stage shape is network ingress -> prefilter -> dispatcher -> pipeline-specific ring buffers -> effect
  executors. `NetworkEvent`, prefilter decisions, dispatcher classification, and ring-buffer allocation belong in the
  network crate or a dedicated pipeline crate. The consensus crate should define only consensus event/effect types such
  as `ConsensusEvent`, `PbftVoteEvent`, `ConsensusEffect`, and opaque ingress payload references used to decode arena
  bytes late. These consensus pipeline types are provisional scaffolding until the first routed pipeline lands; names,
  variants, and payload fields are expected to change as the design is validated.
- Consensus business logic should be expressed as deterministic protocol planners, not as async tasks, actors, or
  workflows that own data. A planner receives a consensus event or command plus explicit borrowed state views/facts and
  returns a protocol plan: validation outcome data, ordered state/write intents, follow-up consensus events, and external
  effects. The plan describes the protocol state transition, but the planner must not perform I/O, spawn Tokio work,
  write storage, send network messages, or mutate another pipeline directly. Runtime workers, actors, Tokio tasks, ring
  buffers, and effect executors may schedule and apply plans at the boundary; they should not hide consensus rules inside
  mailbox-local state.
- Hard rule for Rust-enabled paths: never forward, delegate, or rely on inherited behavior from legacy C++ implementations. Any not-yet-ported API must stay explicit in the shim as a documented stub/no-op/throw until Rust parity lands. If fallback is proposed, require explicit task-owner approval first.
- Hard rule: preserve existing test intent. Do not loosen or rewrite tests to accommodate Rust rewrite regressions; fix implementation parity first. Only change tests when product behavior is intentionally changed and documented.
- Documentation rule: whenever adding or changing rewrite code, document modules, types, and functions as complete units (purpose, inputs, outputs, invariants, and error or edge behavior), not just isolated lines.
- Rust domain modules define narrow ports for required capabilities.
- Infrastructure modules implement those ports over RocksDB, CXX bridges, or test fakes.
- Runtime/bootstrap code wires concrete implementations together.
- Avoid "everything bag" context structs and broad service locators.
- Keep dependency direction one-way: domain -> ports, infra -> port implementations, runtime -> wiring.
- Rust rewrite code should use clearer names than C++ when legacy names are ambiguous, unclear, overly abbreviated, or
  easy to misread. Preserve C++ compatibility at the shim boundary, but prefer descriptive Rust APIs internally.
- Rust modules should use `anyhow` for fallible APIs unless a narrower error type is explicitly needed at a domain
  boundary. Convert lower-level errors into `anyhow::Result` with useful context instead of leaking incidental backend
  details through domain APIs.

Dispatch guidance:

- Use generics/static dispatch for hot loops.
- Use `Arc<dyn Trait + Send + Sync>` only where runtime flexibility is needed.
- Collapse abstractions only when profiling or code clarity justifies it.

## Rust Type and Codec Policy

Introduce Rust domain types by semantic role, not by mirroring C++ class names.

Recommended module grouping:

- `rustaxa_types::pbft`: PBFT block metadata and future PBFT domain objects.
- `rustaxa_types::final_chain`: final-chain block, receipt, log, bloom, and execution-facing models.
- `rustaxa_types::dag`: DAG block and DAG-related models.
- `rustaxa_types::codec`: RLP and other wire/storage encoders and decoders.

Rules:

- Use `#[repr(transparent)]` newtypes for distinct scalar values such as `PbftPeriod`, `BlockNumber`, `TrxPosition`, `Gas`, and hashes.
- Keep domain fields private when invariants matter; expose validated constructors or `TryFrom`.
- Keep domain models separate from wire/storage representations.
- Avoid `TryFrom<&[u8]> for DomainType` when bytes could mean multiple formats; use codec-specific wrapper inputs.
- Decode bytes only when logic needs typed fields.
- Preserve canonical encoded bytes when repeated hashing or persistence would otherwise require re-encoding.
- Keep CXX bridge transport wrappers plain and separate from core domain models.

Each new domain type should document:

- invariants
- canonical encoding shape
- C++ compatibility expectations
- allocation and throughput impact
- DB, codec, and CXX bridge conversion points
- byte-compatibility and malformed-input tests

## Storage Rewrite Plan

### Scope

In scope:

- `DbStorage` public API parity through Rust-backed shims.
- DAG, transaction, period-data, PBFT/vote, pillar, metadata/config/statistics read and write primitives.
- Batch semantics that are needed by active consensus and final-chain paths.

Out of scope for the current wave unless explicitly re-scoped:

- storage migration flows
- light-node / `plugin/light` maintenance and pruning flows
- snapshot lifecycle and recovery APIs
- low-level RocksDB admin APIs unless they block functional parity

### Current Storage Shape

`DbStorage` owns the main RocksDB database under `db/`. `state_db/` is a sibling database used by FinalChain state execution, not a column family inside `DbStorage`.

The main database uses column families grouped by domain:

- schema and metadata: `default`, `migrations`, `genesis`, `status`
- finalized period data: `period_data`, `pbft_block_period`, `dag_block_period`
- pending DAG and transactions: `dag_blocks`, `dag_blocks_level`, `transactions`, `trx_period`
- PBFT runtime and votes
- proposal/sortition/lambda/reward configuration
- pillar chain state
- system transactions
- final-chain indexes and receipts

The central storage lifecycle is pending-to-finalized:

1. DAG blocks and transactions live in pending columns.
2. `savePeriodData` writes the finalized bundle to `period_data`.
3. Reverse indexes map PBFT block hashes, DAG hashes, and transaction hashes back to period/position.
4. Historical reads use indexes plus `period_data`.

This makes `period_data` one of the highest-value Rust rewrite targets.

### Current Rust Storage Coverage

Rust-mode shim implementations live in `libraries/core_libs/consensus/shims/storage_shim/src/storage_shim.cpp`. Legacy `libraries/core_libs/storage/src/storage.cpp` remains legacy-only logic.

Current Rust-backed coverage includes:

- DAG read/index APIs.
- `period_data` primitives and finalized receipt-by-period reads.
- metadata/config/statistics reads.
- PBFT block-hash presence checks and PBFT manager/vote reads.
- pillar reads.
- transaction presence, location, count, retrieval, and finalized-state reads.
- write primitives for DAG, period data, transactions, PBFT manager/votes, pillar, and metadata/config/statistics APIs marked complete in the historical tracker.
- Rust-mode `createWriteBatch` and `commitWriteBatch` through a bridge-side batch registry.

Current Rust repositories include:

- `DagRepository`
- `PeriodRepository`
- `MetadataRepository`
- `PillarRepository`
- `PbftRepository`
- `TransactionRepository`

### Storage Gaps and Risks

- Batch-heavy write paths need parity and performance hardening, especially around consensus/final-chain hot paths.
- `saveDagBlock(..., Batch*)` explicit C++ batch accumulation remains intentionally unsupported in Rust mode because no external workspace callers were found.
- Snapshot, migration, admin, compaction, iterator, and `plugin/light` paths remain C++-owned and non-blocking for the current wave.
- `migrations` lookup interception is a known scope gap if shim lookup is used on migration paths.

### Storage Sequencing

1. Maintain parity for existing shimmed APIs.
2. Expand direct Rust coverage for externally used public APIs that are still in scope.
3. Harden batch behavior for write-heavy paths with tests and conformance fixtures.
4. Keep admin/snapshot/migration/light maintenance in C++ unless the scope changes.

Validation:

- Always run `rust_storage_tests` for storage changes.
- Run impacted C++ gtests or `ctest` subsets when C++ storage behavior changes.
- Add or update conformance tests for changed serialization, update, or read/write semantics.
- Run `scripts/storage_conformance_diff.sh` before closing larger storage refactors, after confirming with the task owner.

## FinalChain Rewrite Plan

### Scope

Goal: keep `final_chain::FinalChain` public API stable while moving implementation behind an additive shim and Rust-backed components.

The FinalChain shim uses a header overlay pattern and can be enabled with:

- `RUSTAXA_ENABLE_FINAL_CHAIN`

When enabled, legacy implementation compiles as `FinalChainOld`, and external call sites continue using `final_chain::FinalChain`.

### Current Implementation Status

- Rust-backed chain index reads:
  - `lastBlockNumber`
  - `blockNumber`
  - `blockHash`
- Rust-backed block, transaction, receipt, and bloom reads:
  - `blockHeader`
  - `transactionLocation`
  - `transactionCount`
  - transaction RLPs, transaction receipts, block receipts, and `withBlockBloom`
- DPoS query boundary is partially Rust-backed:
  - genesis vote-count snapshot is derived in Rust from genesis validator stake.
  - `dposEligibleTotalVoteCount`, `dposEligibleVoteCount`, and `dposIsEligible` now preserve the `EthBlockNumber`
    argument through the C++ shim and Rust bridge.
  - `dposValidatorsTotalStakes` and `dposValidatorsEligibleVoteCounts` are Rust-backed and return address-sorted
    vectors for available Rust DPoS snapshots.
  - DagManager verification now receives DPoS authorization facts whose VDF sortition denominator is selected in Rust
    from genesis DPoS config plus the configured Magnolia boundary, instead of passing per-block hardfork or
    validator-max policy through the C++ shim.
  - DagBlockProposer now has a full Rust-mode overlay shim. C++ still owns thread/network orchestration, transaction
    packing, block construction, signing, and add-block wiring, while Rust owns proposer eligibility status decisions,
    legacy VRF input bytes, and deterministic tip-selection policy.
  - Rust finalization appends DPoS snapshots for finalized native-transfer blocks and the Rust-supported
    `registerValidator(address,bytes,bytes,uint16,string,string)`, `delegate(address)`,
    `undelegate(address,uint256)`, `undelegateV2(address,uint256)`, `confirmUndelegateV2(address,uint64)`,
    `cancelUndelegateV2(address,uint64)`, `reDelegate(address,address,uint256)`, `setValidatorInfo(address,string,string)`,
    and `setCommission(address,uint16)` DPoS contract subset. The Rust snapshot
    persists validator stake/vote aggregates plus a validator/delegator stake ledger seeded from genesis delegations so
    undelegation and redelegation ownership checks stay in Rust. It also persists validator insertion order and
    commission-change block numbers plus V2 undelegation queues with per-delegator IDs so paged validator/undelegation
    reads and owner commission rules remain restart-durable. Snapshots
    are persisted atomically with finalized block indexes, executed DAG/transaction status counters, and `lastBlockNumber`. Startup reloads persisted historical DPoS
    snapshots so PBFT, DAG, and pillar reads can reuse block-scoped Rust FinalChain facts after restart.
  - Rust finalization persists account snapshots atomically with finalized block indexes plus `lastBlockNumber`.
    Startup reloads persisted account snapshots and only serves latest account reads when the Rust account snapshot has
    caught up to the finalized head, so transaction purge and proposal filtering do not silently use genesis state after
    restart.
  - Because Rust finalization is scoped to post-Magnolia execution, native transaction fees are assigned to validator
    commission rewards by finalized DAG block author and transaction hash through the shared Rust rewards-stat planner,
    using bridged Magnolia/Aspen rewards configuration, DAG difficulty, `blocks_per_year`, and previous-block cert-vote
    facts. The native FinalChain path owns a long-lived Rust rewards-stats runtime, persists non-boundary interval cache
    rows in the finalized-block batch before `lastBlockNumber`, reloads cached stats on startup, and clears the cache at
    distribution boundaries after applying interval rewards. Native execution and reward account mutation now use a
    staged account map that is published only after finalization storage commits; post-Magnolia transaction fees credit
    the Rust DPoS contract account while the DPoS snapshot records per-validator commission ownership. Rust native
    finalization also distributes fixed-yield and Aspen part-two dynamic-yield minted block/DAG/vote rewards from decoded
    Rust reward stats into staged validator commission and delegator reward pools, credits the DPoS contract account with
    the minted total, migrates part-one minted tokens into durable total supply at the Aspen part-two boundary, and writes
    header `total_reward` from the Rust plan. Rust-backed FinalChain shim reads now expose DPoS total delegated, yield,
    total supply, and read-only delegator reward pages backed by Rust F1 reward cursors. Rust now executes delegator
    `claimRewards(address)`, validator-owner `claimCommissionRewards(address)`, validator-owner metadata/commission
    updates, current-ABI `claimAllRewards()`, and
    stake-mutation auto-claims by moving reward balances through staged Rust account/DPoS snapshots. Receipts for the
    supported native DPoS subset now carry Rust-generated legacy ABI logs for validator registration, delegation,
    undelegation, V2 undelegation creation/confirmation/cancelation, redelegation, direct claims, commission claims,
    validator info/commission updates, claim-all, and
    stake-mutation auto-claims, with the block header bloom derived from those logs. Supported DPoS owner validation
    failures now persist failed receipts without mutating DPoS state. Rust native finalization now accepts
    both the current `claimAllRewards()` ABI and the legacy pre-fix `claimAllRewards(uint32)` batch ABI, gates the batch
    selector on `fix_claim_all_block_num`, and charges claim-all gas from the staged Rust DPoS delegation view. The active
    Rust finalization path also persists the legacy two-level `final_chain_log_blooms_index` chunks with author-augmented
    blooms and routes `FinalChain::withBlockBloom` through Rust. Unsupported DPoS methods remain future work.
    Rust native finalization also executes the slashing `commitDoubleVotingProof(bytes,bytes)` precompile path for
    legacy PBFT vote RLPs: Rust decodes the calldata, recovers both vote signers, validates the double-vote facts,
    persists restart-durable jail blocks, jailed-validator order, and duplicate-proof keys in the DPoS snapshot, emits
    the legacy `Jailed(address,uint64,uint64,uint8)` log, and derives effective DPoS eligibility/total vote counts from
    the Rust jail state. Slashing read calls for `getJailBlock(address)` and `getJailedValidators()` are Rust-backed.
  - FinalChain native execution is now behind a Rust-owned `FinalChainExecutionRuntime` session boundary. The
    C++ FinalChain shim now builds the session request directly, asks Rust for the next execution step, and commits only
    when Rust returns a native commit action. Native value transfers plus the supported DPoS/slashing precompile subset
    still commit through the existing Rust FinalChain finalizer. Arbitrary EVM contract calls and contract creation now
    surface as typed external-EVM execution requests in the runtime session API rather than being treated as
    FinalChain-owned execution. When that boundary is needed, Rust now exposes the full ordered bridge-provided
    transaction stream in the EVM request rather than only the contract-call subset, while still reporting the count of
    transactions that require arbitrary EVM execution. EVM reports must cover that same full ordered request and validate
    request identity, transaction order, cumulative gas, typed receipt status, and basic receipt shape. External-EVM
    sessions now request bridge-contract system transaction facts before emitting the EVM request, plan canonical
    `finalizeEpoch()` system transaction RLPs in Rust from those facts, decode the planned RLPs with the fixed Taraxa
    system sender, append them after regular period transactions, and include them in the EVM request identity and
    transaction roots. A valid EVM report now advances to a Rust-owned
    rewards/state-root boundary, and a valid rewards report builds a non-mutating external EVM commit plan with
    transaction/receipt trie roots, header and indexed log blooms, receipt payloads, gas, post-rewards state root, total
    reward, regular/system transaction counts, and execution counters. Rust can also derive a non-mutating publication
    plan with stored/full header RLP, block hash, receipt payloads, transaction-location/receipt publication facts, and
    period system-transaction hash RLP. The publication plan has a deterministic plan id. Rust now validates a separate
    external EVM state-commit intent against the request id, plan id, post-execution root, post-rewards root, period,
    and publication block hash before C++ may call `StateAPI::transition_state_commit`; only after C++ reports a matching
    committed staged-state lifecycle does Rust store a typed ready-to-publish decision and expose an explicit
    storage-publication session action. The session-scoped Rust publication API consumes the stored plan and decision,
    recomputes the plan id, validates the current FinalChain head, and applies the external-EVM FinalChain storage rows
    in one Rust-owned batch: stored header, receipt-by-period, hash/number indexes, receipt-by-transaction hash,
    transaction locations, bloom-index chunks, executed counters, period system-transaction hashes, rewards-stat cache
    mutation, and `LAST_NUMBER` last. This publication API still does not execute EVM or call `StateAPI`. The Rust-mode
    C++ FinalChain shim now owns the temporary external-EVM executor adapter for arbitrary contract calls and contract
    creation: it collects bridge-contract system transaction
    facts through `StateAPI`, executes the ordered EVM request through the existing C++ `StateAPI`, reports EVM
    receipts/logs and rewards/state-root facts back to Rust, requests Rust's state-commit intent before committing the
    staged `StateAPI` state, and then reports only the external state-commit result status plus diagnostic text to Rust.
    Rust derives the lifecycle facts from the session-owned intent and commit plan, returns the ready-to-publish decision
    only for committed outcomes, clears the pending marker only for explicit discarded outcomes, and keeps rejected or
    ambiguous commit-call failures durable for startup recovery. The C++ shim calls the session-scoped Rust publication
    API only after Rust's next action asks for storage publication. Ready publication decisions carry a Rust-generated
    decision id derived from the post-commit lifecycle facts, so intent-shaped or hand-built decisions are rejected before
    storage mutation. Rust remains the authority for request identity, report validation, header/root/bloom derivation,
    state-commit intent validation, lifecycle decision validation, explicit discard/reject handling, rewards-stat cache
    persistence, and FinalChain storage publication. Rust persists a Rust-owned pending-publication marker before the C++
    `StateAPI` staged-state commit call; startup recovery compares that marker with
    `StateAPI::get_last_committed_state_descriptor()` and only replays publication when the committed period and
    post-rewards state root match exactly. Successful live or recovered publication clears the marker in the same
    Rust-owned storage batch as `LAST_NUMBER`. Rust now also exposes a read-only external-EVM publication audit for
    parity coverage; bridge tests use it after live publication, restart recovery, ambiguous rejected-then-recovered
    publication, representative call/create/failure receipt transcripts, and Rust-planned system-transaction publication
    to verify the stored header, full header hash, hash indexes, receipt rows, transaction indexes, bloom leaf, system
    transaction hash row, and pending-marker clearance match the Rust publication plan. Native Rust finalization
    now publishes transaction-location and receipt-by-hash indexes in the same Rust storage batch that publishes block
    visibility and `LAST_NUMBER`, closing the previous native crash window where a finalized head could appear before
    those indexes.
  - PBFT manager fact collection now connects directly to the Rust FinalChain runtime for PBFT final-chain hash lookup
    and validation, total eligible vote counts, per-wallet eligible vote counts, and wallet eligibility refresh. Missing
    delayed headers or DPoS snapshots are returned to PBFT as typed Rust facts instead of re-centering those consensus
    decisions in C++ FinalChain orchestration.
  - non-genesis DPoS queries still return typed errors or throw when the queried block has not been finalized through
    Rust snapshot maintenance; DPoS transitions beyond the supported validator-registration/delegation/owner-update/slashing subset and legacy databases without Rust
    account snapshots remain explicit gaps.
  - selected DPoS precompile reads through `FinalChain::call` are Rust-backed for `getTotalEligibleVotesCount()`,
    `getValidator(address)`, `getValidators(uint32)`, `getValidatorsFor(address,uint32)`,
    `getTotalDelegation(address)`, `getDelegations(address,uint32)`, `getUndelegationsV2(address,uint32)`,
    and `getUndelegationV2(address,address,uint64)`. These precompile reads
    use the exact finalized-block snapshot, while DAG authorization and explicit eligibility APIs still use the
    configured delegation-delay snapshot.
- Unimplemented public shim methods never fall back to `FinalChainOld`. `getAccountStorage`, `getCode`, `call`, and
  `trace` route to C++ `StateAPI` only for blocks whose external-EVM state has been committed by the Rust-mode executor
  adapter; otherwise they use the Rust FinalChain path where implemented or throw explicit Rust-shim gaps. `prune`,
  `getBridgeRoot`, `getBridgeEpoch`, and private `finalize_` remain explicit Rust-shim gaps. `waitForFinalized` remains
  a no-op because the Rust shim finalization path is synchronous and returns a ready future.

### FinalChain Storage Touchpoints

FinalChain currently depends on:

- `final_chain_meta`
- `final_chain_blk_by_number`
- `final_chain_blk_hash_by_number`
- `final_chain_blk_number_by_hash`
- `final_chain_receipt_by_period`
- `final_chain_receipt_by_trx_hash`
- `final_chain_log_blooms_index`
- `StatusDbField::ExecutedBlkCount`
- `StatusDbField::ExecutedTrxCount`
- period and transaction helpers from `DbStorage`
- batch writes and maintenance paths such as snapshots/compaction
- state execution through `StateAPI` / `taraxa-evm`

### FinalChain Sequencing

1. Keep Rust-backed read/index, transaction, receipt, bloom, account snapshot, DPoS snapshot, and PBFT fact-collection
   parity stable.
2. Continue finalization/write path parity beyond the currently supported native-transfer, DPoS mutation/read, rewards,
   bloom, slashing, and FinalChain execution-session subset. The external-EVM storage publication batch is now
   Rust-owned behind an explicit session action, and the Rust-mode compatibility finalizer now reaches it through the
   shim-owned C++ external-EVM executor adapter. The current EVM session plans period bridge-contract system transaction
   RLPs in Rust from C++ `StateAPI` facts and includes them in request/publication facts; Rust still needs
   bridge-contract state reads before C++ can stop collecting those facts.
3. Keep EVM execution outside FinalChain while completing the external executor port: request construction, report
   validation, Rust-owned system-transaction planning from bridge facts, rewards/state-root reporting, non-mutating
   commit/publication-plan derivation, two-phase state-commit intent/lifecycle validation, and session-gated one-batch
   storage publication are Rust-owned, including rewards-stat cache persistence and explicit committed/discarded/rejected
   state-commit result handling; the temporary C++ adapter still owns `StateAPI` fact collection, `StateAPI` execution,
   rewards distribution, and the actual staged-state commit call. The current shim persists a Rust pending-publication
   marker before the staged-state commit, reports only the external commit result status and diagnostic text, and lets
   Rust either publish, clear a discarded marker, or retain an ambiguous rejected marker for startup recovery based on the
   committed `StateAPI` descriptor. Rust bridge tests now audit live, recovered, and representative transcript
   publications against the persisted FinalChain rows, including call/create/failure receipts, log blooms, full-header
   bytes, transaction-location rows, and Rust-planned system transactions. A future broader C++ live-differential fixture
   can compare legacy executor output against the same Rust publication transcript API without moving EVM execution into
   Rust.
4. Continue DPoS and account snapshot parity for remaining DPoS contract methods, broader slashing surfaces, and broader
   state trie/code/storage recovery.
5. Replace neutral placeholder shim methods with Rust implementations or explicit throwing stubs as their callers are
   migrated.
6. Defer broader StateAPI, bridge-heavy APIs, pruning, snapshots, and state-transition boundaries until the external
   EVM execution port is ready to carry those results without re-centering behavior in C++.

High-risk APIs:

- `finalize` / `finalize_`
- `prune`
- `updateStateConfig`
- `call`
- `trace`
- state and non-genesis DPoS query surfaces that depend on EVM/state integration

## FinalChain Domain Type Backlog

P0 done:

- `StoredFinalChainBlockHeader`
- `FinalChainBlockHeader`
- `BlockHeaderContext`
- `FinalChainBlockHeaderBuilder`
- `PbftBlockMetadata`
- `LegacyBlockHeaderRlpInput`
- `LegacyBlockHeaderRlp`

P0 todo:

- `NewBlock`
- `FinalizationResult`
- `BlocksBlooms`
- `LogEntry`
- `TransactionReceipt`
- `TransactionLocation`
- core State API payloads: `EVMBlock`, `EVMTransaction`, `LogRecord`, `ExecutionResult`, `TransactionsExecutionResult`, `RewardsDistributionResult`, `Account`, `StateDescriptor`, `Tracing`

P1 todo:

- `ValidatorStake`
- `ValidatorVoteCount`

P2 later:

- bridge-specific payload wrappers for `getBridgeRoot` and `getBridgeEpoch`
- optional trace/debug JSON adapters if the trace path is migrated

Recommended introduction order:

1. Receipt/location and block-header data model types.
2. Core state API execution result types.
3. Finalization result aggregation types.
4. DPoS validator query types.

## Consensus Rewrite Plan

### Scope

Goal: rewrite consensus internals while keeping existing C++ public APIs and node wiring stable.

Rules:

- Do not delegate Rust shim behavior back to legacy `FinalChainOld` or other old implementation methods.
- Temporary Rust-mode gaps must be explicit shim-local defaults, no-ops, or tracked unimplemented paths.
- Temporary guarded touches to upstream-owned C++ files should be removed once a complete shim can own Rust-mode routing;
  currently `pbft_manager.cpp` has a narrow early-return hook for sync pillar-vote bundle planning, with helper
  declarations supplied by the temporary `pbft_manager_shim` header overlay.
- Treat `dposIsEligible` and related vote-count methods as real consensus work, not permanent dummy behavior.
- Keep networking callbacks, thread orchestration, and broad node integration in C++ until the Rust domain services are stable.
- Ignore logging when deciding whether behavior can move to Rust. Logs are boundary observability, not consensus
  ownership. Keep temporary C++ logging only as an executor/reporting detail while moving the underlying decision,
  state transition, or persistence logic into Rust.
- Shape new ingress-adjacent consensus APIs for the upcoming application-owned arena pipeline even before the concrete
  API exists. Prefer functions that can consume raw payload bytes or compact arena-backed facts, produce small
  protocol plans/effects, and defer C++ `PbftVote`, `PbftBlock`, `DagBlock`, `PeriodData`, and `Transaction`
  materialization until a compatibility executor truly needs those objects.
- The first network-to-Rust call is expected to be
  `pub fn ingest_network_packet(self: &mut BridgeNetwork, packet_type: u8, from_node: [u8; 64], data: Vec<u8>) -> Result<bool>;`.
  Consensus code must not interpret its `bool` as payload validity or consensus acceptance; it only indicates whether
  the network ingress payload was accepted into the arena-backed data pipeline.
- Treat ingress processing as multiple logical pipelines, not one monolithic consensus loop. Ingress-adjacent Rust APIs
  should keep pipeline-specific facts and effects separate for peer status/sync control, transaction admission, DAG
  admission, DAG sync, PBFT vote progress, PBFT sync/finalized-period intake, and pillar votes. When one pipeline must
  affect another, return an explicit effect such as request-sync, block-peer-order, mark-known, admit, gossip, report
  malicious, enqueue-period-data, or drive-PBFT-progress instead of mutating another pipeline implicitly.
- Shape consensus pipeline units around ingress-payload-backed events. The dispatcher owns ingress-message routing outside
  the consensus crate, and passes typed consensus units such as `PbftVoteEvent` into consensus pipelines. Those units may
  carry compact facts or enrichment ids while preserving canonical bytes in the arena. Ring buffers should transfer
  ownership of these event units between stages instead of exposing shared mutable message objects. The current event
  names and payload shapes are deliberately open to change while the first production pipeline integration proves the
  right boundaries.
- Express consensus business logic as deterministic protocol planners over explicit state views. A planner receives a
  consensus event or command, compact facts, config/time inputs, and borrowed state views, then returns a protocol plan.
  The plan may include validation outcome data, ordered effects, storage/write intents, and follow-up consensus events.
  This is the concrete meaning of a protocol state transition: the observed consensus state plus an input maps to the
  next intended state and effects, while execution remains outside the planner. Tokio, actors, and ring-buffer workers
  belong around the planner as scheduling/execution machinery, not inside the consensus rule implementation.

### PBFT Manager Rust Ownership Boundary

Target state: the PBFT manager protocol brain moves to Rust. The Rust-mode C++ overlay should become a compatibility
shell and effect executor that supplies facts, calls a Rust-owned `PbftManagerRuntime`, executes returned effects, and
reports effect results back to Rust before the runtime advances.

Boundaries that should not move as part of the PBFT manager breakthrough:

- Network/tarcap transport: peer connections, packet wrapping, gossip fanout, send policy, known-peer marking,
  disconnect/report mechanics, and packet queue ownership stay outside the consensus manager migration. Rust may return
  typed egress, mark-known, sync-request, and peer-report effects for the existing network executor to perform.
- EVM/FinalChain execution: transaction execution, receipt/log bloom construction, gas execution, state transition
  execution, and external contract execution stay in the existing FinalChain/EVM boundary until that execution layer is
  migrated. Rust PBFT logic may plan finalization, validate facts, and request execution/finalization effects, but it
  must not absorb EVM execution into the PBFT manager.
- Live C++ API compatibility: temporary `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, and
  pillar sidecar materialization may remain in the overlay while public APIs and remaining callers require those types.
- Node lifecycle and scheduling: daemon threads, sleeps, timers, startup/shutdown wiring, event emission mechanics, and
  key-manager signing may stay as effect execution until the surrounding application pipeline owns them.

Everything else inside `PbftManager` is in scope for Rust ownership: period/round/step state, daemon-tick control flow,
proposal/certify/finish-polling transitions, sync-period admission, proposed-block selection and cleanup planning, vote
and reward-vote selection, finalization planning and bounded resume, dynamic-lambda decisions, DAG/transaction cleanup
planning, PBFT-chain head advancement plans, storage/write intents, cross-pipeline effects, and ordered side-effect
contracts. Logging is explicitly not a boundary; it can stay temporarily in the C++ executor as reporting derived from
Rust statuses and telemetry.

### Current Consensus Shape

The C++ consensus area includes:

- DAG graph and proposal logic: `Dag`, `DagManager`, `DagBlockProposer`, `SortitionParamsManager`.
- PBFT state and proposal flow: `PbftManager`, `PbftChain`, `PeriodDataQueue`, `ProposedBlocks`.
- Voting and eligibility: vote manager, vote bundles, DPoS eligibility, eligible and total vote counts.
- Transaction flow: `TransactionManager`, `TransactionQueue`, gas pricing, and transaction proposal selection.
- Pillar chain, rewards, slashing, and final-chain state/query integration.

The current Rust starting point is intentionally small:

- `rustaxa-consensus` contains early FinalChain read/index logic, Rust-backed DAG graph state, Rust-backed
  sortition efficiency/threshold runtime state, Rust-backed PBFT chain head/validation state, and Rust-backed
  proposed PBFT block cache, period-data queue metadata state, and DagManager `verifyBlock` deterministic reject
  decisions for prechecks, transaction availability, DAG VDF payload/embedded-VRF/difficulty/proof verification,
  legacy DAG VRF/VDF message construction, DPoS authorization ordering, gas policy, Rust-backed transaction queue
  metadata/order/limit state, Rust-backed `TransactionManager::packTrxs` deterministic packing decisions, Rust-owned
  DAG transaction persistence planning plus Rust-storage batch commits, Rust-storage-backed `TransactionManager`
  transaction lookup and non-finalized recovery payload loading, runtime-backed finalized transaction
  filter/verification helpers, Rust-planned transaction verification and runtime/FinalChain-backed validated admission,
  shim-owned live non-finalized/pool/count
  read helpers, a canonical PBFT vote event fact boundary, a Rust-owned validation-backed PBFT vote admission runtime
  that composes canonical validation, event-fact derivation, verified-vote mutation, threshold planning, retained
  storage/slashing vote payload sidecars, and typed executor intents for peer-known marking, proposed-block sidecar
  routing, gossip, and PBFT progress. The Rust-mode `VoteManager` shim exposes those intents through a temporary
  admission report consumed by latest-tarcap vote handlers, so single-vote and bundle paths mark peers/votes known,
  report slashing, and gossip only after Rust admission has accepted the vote. Generic legacy snapshot and 2t+1 APIs on
  the `VerifiedVotes` overlay now materialize temporary `PbftVote` sidecars from Rust-retained weighted payload bytes
  instead of skipping missing live sidecars. Reward-vote validation and materialization now enter the same runtime:
  Rust builds preferred-round and reverse-period candidates from Rust-owned verified-vote metadata and returns selected
  retained weighted records in PBFT-block requested order. Metadata-only compatibility/test helper inserts may still fall back to
  `live_votes_` until those helpers are removed, but production-admitted votes treat missing retained payloads as
  invariant errors. The crate also contains a side-effect-free PBFT vote-progress protocol planner
  plus a Rust-owned PBFT vote pipeline session that stages
  verified-vote insertion reports into typed
  known/admit/slashing/gossip/progress intents, a side-effect-free PBFT vote ingress planner for deterministic
  single-vote and bundle relevance/window/sync-hint/drop decisions, and exposes operation-specific CXX bridge surfaces for Rust-mode
  `VoteManager::addVerifiedVote` execution, Rust-owned PBFT vote validation
  planning with replay-cache storage,
  canonical PBFT vote RLP inspection, signed/unsigned vote hashing, signature recovery, VRF proof verification,
  Rust-computed received-vote weight, sortition-threshold calculation, Rust-owned PBFT `2t+1` threshold cache, local
  proposer-sortition screening, Rust-owned local PBFT vote byte generation/signing for canonical signed and weighted
  vote payloads with shim-side parity checks against temporary C++ live sidecars for the Rust-mode `VoteManager`
  overlay, and Rust-owned optimized PBFT vote-bundle construction from retained weighted payload records for
  get-next-votes egress. C++ still owns peer-known filtering, tarcap packet wrapping, splitting, send policy, and
  known-vote marking at the network boundary. The crate also contains a Rust-backed
  `GasPricer` oracle for finalized-block history, minimum-price
  flooring, and percentile bid selection.
  The Rust-enabled `SlashingManager` overlay now routes deterministic double-voting proof planning, duplicate-proof
  cache decisions, unweighted vote evidence payload normalization, submitter selection, and slashing contract calldata
  construction through Rust; the PBFT vote admission route now passes Rust-normalized unweighted payload records, while
  C++ keeps account reads, gas bidding, transaction signing, transaction-pool insertion, and the live-vote overload for
  remaining compatibility callers.
- `rustaxa-types` contains shared Rust domain and codec types, including the legacy transaction envelope used by
  Rust-enabled transaction-manager shims to decode canonical RLP bytes, hash transactions, recover/validate senders,
  compute intrinsic gas coverage, and surface deterministic nonce/gas/value/cost facts without calling C++
  `Transaction` getters for those fields.
- `rustaxa-types` now contains Rust pillar type and codec parity for `PillarBlock`,
  `ValidatorVoteCountChange`, `PillarVote`, `PillarBlockData`, optimized pillar-vote bundles, and current pillar data
  storage shape. Pillar-vote author recovery now lives on the Rust `PillarVote` type with C++ parity coverage for the
  recoverable-signature path. Pillar signing, broader manager orchestration, and production routing remain later
  consensus slices.
- `rustaxa-consensus` now contains a Rust pillar-vote aggregation domain for already-verified pillar vote facts:
  period initialization, per-validator uniqueness, weighted per-block aggregation, deterministic threshold subset
  selection, and stale-period cleanup. The `RUSTAXA_ENABLE_PILLAR_VOTES` overlay routes the C++ `PillarVotes` API
  through Rust for deterministic aggregation while C++ keeps live `PillarVote` sidecars. The PBFT sync pillar-vote
  bundle path now calls a stateless Rust bundle planner for period/block validation, duplicate-safe unique-weight
  threshold accounting, deterministic rejection statuses, accepted vote weights, and Rust-recovered voter identities.
  The PBFT shim resolves only Rust-accepted vote hashes back to live C++ sidecars and inserts them through a temporary
  planned-insertion hook that does not re-run manager validation, recover voters in C++, or re-query DPoS weights.
  `PillarChainManager::isRelevantPillarVote` now uses a shim-owned Rust relevance planner for period/block/known-vote
  decisions under `RUSTAXA_ENABLE_PILLAR_VOTES`.
  `PillarChainManager::validatePillarVote` now inspects pillar-vote RLP in Rust, uses the Rust-recovered
  `(period, vote_hash, voter)` identity for uniqueness, uses the recovered voter for DPoS eligibility, and avoids C++
  signature or voter recovery in Rust mode. `PillarChainManager::addVerifiedPillarVote` now also runs through Rust
  inspection, uses the Rust-recovered voter for C++ `FinalChain::dposEligibleVoteCount`, and inserts with
  `addVerifiedVoteWithRecoveredVoter` to avoid re-querying C++-side voter identity. Pillar signing and the full
  `PillarChainManager` overlay remain later slices.
- `rustaxa-storage` contains storage repositories that consensus should use through narrow ports.

### Consensus Sequencing

1. Inventory consensus APIs, dependencies, tests, and current Rust shim gaps.
2. Create a tracker that classifies each item as Rust-backed, shim-stubbed, C++-owned temporary, or out of scope.
3. Introduce pure Rust domain types for PBFT rounds, steps, votes, DAG ordering metadata, and eligibility outputs.
4. Add ingress-compatible Rust inspection/planning surfaces as adjacent slices are touched. PBFT vote, DAG block,
   transaction, pillar vote, and PBFT sync work should accept canonical bytes or compact facts, optionally create
   enrichment records, and return route/admit/drop/gossip/request-sync/peer-action intents rather than depending on
   network handler objects or eager C++ materialized objects. The first PBFT vote-progress runtime now routes
   Rust-mode `VoteManager::addVerifiedVote` through a validation-backed Rust admission runtime: C++ supplies
   FinalChain/key facts, Rust validates the canonical PBFT vote RLP, carries the validation result and calculated
   weight into compact progress facts, mutates the single Rust-owned `VerifiedVotes` runtime, retains weighted storage
   payloads and unweighted slashing evidence payloads, and returns one terminal executor report with Rust-planned
   peer-known, proposed-block sidecar, gossip, storage, slashing, threshold, and PBFT-progress intents. Latest-tarcap
   single-vote and bundle handlers now execute those peer-known, slashing, and gossip intents from a shim-owned
   admission report after Rust admission accepts the vote; bundle rebroadcast in Rust mode is limited to accepted votes.
   PBFT vote packet ingress now has a compact-fact Rust planner for relevance, period/round/step windows, proposed-vote bundle rejection,
   bundle identity consistency, and PBFT/next-vote sync hints. Latest-tarcap packet handlers currently call this planner
   through guarded temporary hooks in Rust-enabled builds, while C++ still decodes packets, supplies live peer/sidecar
   facts, and executes network effects until the network/tarcap pipeline overlay owns those routes. The guarded latest-
   tarcap method signature changes are temporary network-hook debt, not the target API shape.
   Replay protection and PBFT `2t+1` threshold caching now live in the same Rust `VerifiedVotes`/admission runtime that
   owns verified-vote mutation. PBFT vote validation now also has Rust planner surfaces for compatibility/testing:
   C++ supplies DPoS/key facts, while Rust owns canonical vote-byte inspection, signature/VRF facts, Rust-computed
   received-vote weight, final accept/reject statuses, replay-marker timing and storage, local proposer-sortition
   screening, and the sortition-threshold formula. C++ still performs the temporary `PbftVote::weight_` live sidecar
   mutation and parity-checks it against the Rust weight.
5. Port DAG graph operations before broader `DagManager` orchestration: pivot/tip availability, ghost path, ordering,
   counters, storage-facing queries, and deterministic `verifyBlock` reject decisions.
6. Define Rust ports for DPoS eligibility, eligible vote count, total vote count, and VRF key access. The current
   `DagManager` shim now gets those DPoS/VRF facts from a Rust FinalChain bridge bundle and routes embedded VRF proof
   verification, DAG VDF payload decode, difficulty calculation, legacy-modulus Wesolowski proof check, status-coded
   VDF/DPoS fact envelope, legacy VRF/VDF message construction, verify-side VDF denominator policy, and reject ordering
   through Rust. The Rust-mode `DagBlockProposer` overlay now routes proposer eligibility status decisions, legacy VRF
   input construction, and deterministic tip selection through Rust while preserving the C++ thread/network shell.
7. Replace the temporary `dposIsEligible` shim behavior once the eligibility port has a real implementation.
8. Finish the PBFT support slice by adding broader manager-level validation around the now Rust-backed primitives:
   `PbftChain` head updates, persisted-head preview, and next-block validation route through Rust under
   `RUSTAXA_ENABLE_PBFT_CHAIN`; proposed-block membership, validity flags, RLP snapshots, and cleanup planning route
   through Rust under `RUSTAXA_ENABLE_PROPOSED_BLOCKS`; period-data queue admission, effective size, pop vote-source
   decisions, and cleanup planning route through Rust under `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE`.
9. Continue shrinking the Rust-mode `PbftManager` overlay into Rust services for candidate validation and ordered
   state-action scripts. The first grouped leader-candidate planner owns proposal candidate status derivation,
   mark-valid commands, and deterministic leader ranking; the C++ overlay only supplies live block lookup/validation
   facts, applies returned mark-valid effects, and materializes the selected vote/block. `getValidPbftProposedBlock`
   now runs through a Rust-owned proposed-block admission planner that requests sidecar lookup, live block validation,
   mark-valid mutation, acceptance, or rejection. PBFT manager state-action phases now consume that admission planner
   through one shim helper for Rust-planned proposal/filter/certify/finish block hashes instead of duplicating
   phase-local lookup and validation branches. Rust-planned state-action vote intents now also go through one shim
   executor helper that centralizes live vote placement and next-vote status mirror persistence while Rust continues to
   own the intent selection. PBFT proposed-block validation also has a Rust-owned staged planner for
   the proposal path: Rust requests PBFT-chain, FinalChain hash, reward-vote, extra-data, pillar-block, DAG-order, and
   DAG-weight facts in legacy order, then returns accept/reject or wait-for-finalization decisions while C++ still
   supplies the live object checks. `processPeriodData` now reuses that shared planner for the overlapping sync-path
   FinalChain, reward-vote, and extra-data checks before handing sync-only cert-vote, transaction, pillar-vote, and
   peer/queue side effects back to the PBFT sync runtime planner. The next removal target is to replace the remaining
   `validateFinalChainHash`, DAG-order/gas fact, cert-vote, transaction, and pillar-vote decision glue with shared Rust
   executor intents that consume existing Rust FinalChain bundles. After the shared planner owns deeper sync acceptance,
   `identifyLeaderBlock`,
   `proposeBlock_`, `identifyBlock_`, `certifyBlock_`, `firstFinish_`, and `secondFinish_` should collapse further into
   hash/object resolution plus vote/sign/gossip/storage effect execution.
10. Port transaction queue behavior before transaction manager orchestration. The Rust-mode `TransactionQueue` overlay
   now routes deterministic queue metadata, per-account nonce ordering, same-nonce replacement, non-proposer expiry
   planning, pool limits, gas-price threshold accounting, queued transaction RLP payload retention, known-transaction
   cache expiry, overflow/drop observation state, and finalized-account purge planning through Rust while C++
   materializes `Transaction` objects on demand. Finalized-account purge fact sourcing now reads accounts from the Rust
   FinalChain runtime in both TransactionManager runtime cleanup and standalone `TransactionQueue::purge()`. The
   Rust-mode `TransactionManager` packing shim now routes proposal candidate
   snapshotting, candidate scan, Rust-inspected envelope facts for candidate EVM input, declared-gas fit checks,
   invalid-estimate demotion mutation, accepted output ordering, accepted gas accumulation, and stop rules through a
   Rust runtime pack session. C++ drives packing through a narrow Rust step protocol that either asks for a required EVM
   estimate or returns the final selected payloads and clears the session; declared-gas and gas-estimation-cache hits are
   consumed inside Rust without a C++ callback. The stale standalone planner FFI and explicit pack-finalize surface are
   removed in favor of this session contract. A shim-owned guard prevents concurrent C++ callers from racing the single
   Rust runtime session while EVM execution is outside the transaction lock. Rust also owns `estimateTransactionGas` and
   `estimateTransactions` declared-gas shortcut decisions plus the bounded `(transaction hash, proposal period)` opaque
   `ExecutionResult` cache, while C++ keeps EVM execution, public transaction construction, final selected transaction
   materialization, and lifecycle/finalization orchestration. The shim-only `TransactionQueue::demoteToNonProposable`
   API has been removed because pack demotion mutates the Rust runtime queue directly.
   The TransactionManager shim now owns an opaque Rust runtime handle for live queue metadata/payloads, known-cache
   state, non-finalized and recently-finalized transaction sidecars, and the authoritative transaction count. DAG
   transaction persistence now derives transaction hashes, senders, nonces, gas facts, costs, and canonical RLP payloads
   through the shared Rust legacy transaction envelope before sending facts to the Rust runtime. Rust sources latest
   account nonces from the Rust FinalChain runtime and owns sidecar membership checks, duplicate filtering,
   nonce-gated finalized-storage
   lookup, accepted ordering, count planning, the storage batch, accepted non-finalized sidecar insertion, and accepted
   queue erasure before returning a typed DAG-save command report that C++ consumes only for logging. Finalized transaction status
   updates now send finalized hashes and RLP payloads to Rust; Rust plans count increments, retention eviction, periodic
   queue cleanup, recently-finalized sidecar insertion, non-finalized sidecar removal, known-cache marking, and queue
   erasure while persisting `TrxCount` before returning typed finalized-status command buckets that C++ logs. Periodic
   finalized-account purge now executes inside the Rust finalized-status command report by sourcing account facts from
   Rust FinalChain and mutating the Rust queue. Block-finalized queue cleanup now calls the Rust runtime mutator directly,
   so C++ no longer receives count mirrors or switches on raw lifecycle notice IDs for these mutation paths.
   Non-finalized recovery now asks Rust to delete stale finalized rows, inspect survivor legacy envelopes, validate key
   hash and sender facts, and insert survivor sidecar payloads into the Rust runtime without returning count mirrors or a
   C++-applied recovery input list.
   `excludeFinalizedTransactions` and `verifyTransactionsNotFinalized` now inspect legacy transaction envelopes in Rust
   for identity facts, then call runtime-backed Rust bridge APIs for sidecar membership, latest FinalChain account nonce
   sourcing where required, finalized-storage checks, and deterministic filtering/short-circuit decisions; the older
   storage-only and sidecar-only finalized filter/verification CXX entry points have been removed. `verifyTransaction`,
   `insertTransaction`, and `insertValidatedTransaction` now inspect the transaction envelope in Rust and call typed
   Rust admission command reports for exact verification reasons, legacy public insertion result text,
   latest FinalChain account sourcing, public insertion result mapping, staged known-fast-path prechecks, finalized-location mapping, Rust storage-completed admission
   support, and fused proposable/non-proposable admission with Rust-owned live queue mutation. Public
   `insertTransaction` now enters one Rust runtime operation that owns known precheck, verification decisioning,
   FinalChain-backed account/finalized lookup, queue mutation, and event/log intent before C++ maps legacy public error
   strings. Direct standalone validated-insert planner CXX entry points have been removed, so public and validated
   insertion paths must go through the Rust runtime command-report APIs. Known-hash insert decisions now route through the Rust runtime precheck instead of a shim-local early return,
   and `isTransactionKnown` now includes Rust sidecar membership checks alongside queue-known state. Rust now returns
   typed DAG-save, finalized-status, and admission command reports instead of generic lifecycle/action reports. These
   reports now carry direct hash receipts for the remaining C++ log/event sinks without redundant transaction-count
   fields, so shim code no longer rebuilds input hash vectors or revalidates Rust command bucket indexes before logging
   side effects. The Rust-mode facade now owns the public `transaction_added_`
   event surface and emits it from shim-owned code after Rust accepts a proposable queue mutation. Transaction read helpers no longer infer source
   order in C++: `getTransaction`, `getTransactions`, `getBlockTransactions`, `getNonfinalizedTrx`, and
   `getPoolTransactions` now consume Rust-owned transaction views that preserve request order and duplicates while
   resolving queue, non-finalized sidecar, recently-finalized sidecar, pending storage, finalized regular storage, and
   finalized system storage sources. The Rust runtime state exposes the authoritative Rust-mode transaction count and
   drives count reads after persistence/finalization commits. Rust FinalChain now exposes block-scoped account snapshots,
   and proposal transaction views verify stored transaction RLP hashes, inspect legacy sender/nonce identity in Rust, and
   apply proposal-period finalized-account nonce filtering before C++ materializes returned payloads. Remaining live-shell
   gaps are EVM estimation execution, event/log mechanics, public transaction object construction, final materialization,
   and broader lifecycle orchestration. With transaction account-fact sourcing owned by Rust, the first PBFT
   orchestration storage slice now restores proposed-block metadata directly from Rust storage and removes stale
   proposed-block storage keys through Rust-batched cleanup while C++ keeps daemon threads, networking, timers,
   finalization side effects, and live object dispatch. A full Rust-mode `PbftManager` overlay now owns PBFT startup and
   sync-validation routing so upstream `pbft_manager.cpp` stays merge-clean; the copied overlay is deliberate PBFT
   orchestration scaffolding and should be reduced over time by moving round/step/status planning into a Rust-owned PBFT
   manager runtime. The first PBFT orchestration slice now routes `processPeriodData` sync-period admission through a
   side-effect-free Rust planner: C++ still sources PBFT chain, FinalChain, reward/cert vote, transaction, and pillar
   facts and still performs waits, queue clears, peer reporting, live object dispatch, and temporary log emission, while
   Rust owns the deterministic accept/drop/wait/clear decision table. Logging is not a reason to leave the decision
   table in C++. Missing or finalized transaction facts remain warn-only for
   compatibility and do not reject synced period data. Rust now also plans the sync-period transaction-finalization query:
   C++ extracts live DAG and period-data transaction hashes, Rust de-duplicates DAG references, removes hashes already
   supplied by period data, and returns the ordered finalized-storage lookup list before C++ performs the live
   TransactionManager query. The PBFT sync runtime now has a staged Rust planner for the full `processPeriodData`
   validation order: Rust returns the next required live C++ check for FinalChain, reward votes, cert votes,
   transactions, pillar data, or pillar votes until all required facts are present, then returns accept/drop/wait/report
   side-effect intent. This keeps sleeps, queue mutation, peer reporting, live vote/transaction managers, and
   `PeriodData` materialization in the shim while making `NotChecked` facts explicit runtime work rather than implicit
   acceptance. Rust now also owns the first PBFT manager daemon-tick runtime session: the overlay supplies current
   state/period/round/step, network sync status, and post-prestate eligible-wallet reports, and Rust returns a cursor-managed script for
   synced-block processing, optional vote broadcast/cert-block push, round advance, ineligible-wallet sleep, the current
   PBFT state action, state transitions, and final sleep. C++ still executes each live action, but must report the result
   back before Rust advances the cursor; cert-push and round-advance progress complete the session with a restart-loop
   intent, while certify and second-finish branches are selected from explicit reported flags. Rust also now owns the
   active PBFT state-action branch planner for value proposal, filtering, certify, first finish, and finish polling: the
   shim supplies compact vote/timing/status facts, Rust returns typed proposal, soft-vote, cert-vote, next-vote, finish,
   or no-op intents, and C++ only materializes live blocks/votes, storage mutations, and network effects. Rust now also
   owns PBFT leader proposal ranking and selection: the shim supplies proposal vote identity, VRF credential, recovered
   public key, weight, live candidate validation status, and pivot facts, while Rust computes the legacy
   `getVoterIndexHash(credential, voter, index)` ranking over RLP bytes, applies the duplicate-rank overwrite rule,
   skips already-in-chain/missing/invalid candidates, preserves the null-anchor fallback rule, and returns selected
   vote/block hashes for C++ materialization. The shim-local `getProposal()` helper has been removed. C++ still owns
   live `PbftBlock`/`PbftVote` object resolution, proposed-block validation fact collection, signing, storage mutation,
   and gossip execution. Rust now also
   owns PBFT manager cursor transitions for round reset, filter/certify/finish/finish-polling phase changes, finish
   loopback, polling delays, exponential lambda backoff, next-voted status resets, cert-voted sidecar cleanup, own-vote
   clearing, and candidate round-advance validation. Rust storage now owns the transition persistence apply path for
   manager round/step fields, next-voted status resets, cert-voted block cleanup, and latest own-vote cleanup in one
   committed batch, dropping rejected batches before C++ live mirrors change. The shim only applies Rust-planned live
   mirror updates after that Rust storage apply succeeds, while keeping actual timers, FinalChain waits, VoteManager
   period/round side effects, network effects, and compatibility objects in C++. Rust now also owns the long-lived PBFT
   manager scalar runtime handle used by the overlay: startup restore reads persisted round/step/lambda/status facts
   through Rust storage, applies legacy-compatible default and step-normalization rules, persists normalized startup step
   state through Rust before C++ mirrors are updated, rejects missing Cacti dynamic-lambda facts explicitly, and advances
   the runtime cursor only after a Rust-owned transition storage batch commits. C++ mirrors remain temporary compatibility
   state until the remaining live side effects move behind Rust protocol plans. PBFT
   finalization
   execution now has a Rust-planned intent contract as well: the shim supplies accepted
   block, PBFT head, anchor, pillar-finalization, and dynamic-lambda facts, and Rust returns explicit cleanup/finalize/
   advance-period flags before C++ applies the existing DB, DAG, transaction-manager, PBFT-chain, FinalChain, and timer
   side effects in the legacy order. Rust also now plans the native-ready PBFT finalization storage write set: the shim
   supplies PBFT head key, canonical period-data RLP, ordered finalized DAG hashes, reordered transaction hashes,
   certified-vote identity, and lambda facts, and Rust returns primary-batch write flags, positioned DAG/transaction
   index writes, period-lambda persistence, executed-status persistence, and `blocks_per_year`. Rust now appends the
   PBFT head, PBFT hash-to-period, period-data RLP, finalized DAG indexes, transaction indexes, and pending-row deletes
   to the shim's Rust-backed storage batch. The PBFT finalization persistence bridge now exposes one staged Rust appender
   for primary finalized-period writes, post-live-mutation dynamic-lambda persistence, and post-FinalChain-dispatch
   executed-status persistence; compatibility wrappers remain for the older appender entrypoints. The C++ shim calls the
   staged API while preserving the existing batch/commit boundaries. Rust now also owns the storage-batch lifecycle for
   PBFT finalization persistence stages: the bridge creates, appends, commits, or drops Rust storage batches for the
   primary finalized-period/reward-reset/sortition group, dynamic-lambda persistence, and executed-status persistence,
   while the PBFT overlay still owns live FinalChain, timer, period-advance, and remaining object-materialization side
   effects in the legacy order. Rust now owns a side-effect-free finalization runtime stepper and the Cacti
   dynamic-lambda calculation for PBFT finalization: it returns the ordered mixed-executor action list, block-period
   lambda, reward `blocks_per_year`, post-adjust rounds count, post-adjust dynamic lambda, and increase/decrease
   telemetry flags. The PBFT overlay consumes those Rust outputs and no longer calls the C++ dynamic-lambda adjustment
   routine from the Rust-mode finalization path. Rust now also owns a finalization runtime session cursor: the overlay
   asks Rust for each action, executes the temporary C++ live effect, and reports success/failure back before Rust
   advances the session. PBFT finalization sortition update is now two-phase: the sortition shim previews the Rust
   threshold transition without mutating live state, the emitted change is included in the existing primary finalization
   storage batch, and only after that batch commits does the shim commit live sortition runtime state and report the
   action back to the Rust cursor. Reward-vote reset metadata commit, DAG finalized-order mutation, transaction
   finalized-status sidecar cleanup, sortition runtime commit, and PBFT-chain live head updates now run behind shim-owned
   executors that return structured post-state proofs back to Rust before the cursor advances: Rust validates the
   finalized DAG count, finalized transaction count, finalized period, PBFT block hash, anchor hash, reward-vote
   period/round/block metadata, stale extra-reward-vote cleanup, sortition change period/current-threshold facts,
   PBFT-chain size, and PBFT-chain head/anchor state against the accepted finalization plan.
   The bridge also exposes a storage-backed PBFT
   finalization resume classifier for duplicate or
   restart-adjacent blocks: Rust inspects the durable hash-to-period, period-data, finalized DAG/transaction indexes,
   optional period-lambda, executed-status, and FinalChain height facts and returns complete, replay-needed,
   missing-primary, or conflicting-primary classifications instead of letting the shim treat `pbftBlockInDb` as a blind
   duplicate. The duplicate path now consumes that classification through a Rust-owned resume runtime session for the
   storage-proven tail only: when primary finalization and dynamic lambda are already durable and FinalChain is exactly
   one period behind, the overlay replays FinalChain finalization, persists executed status through Rust, sets the live
   executed flag, advances the PBFT period, and reports each action back to Rust before the cursor advances. Dynamic
   lambda gaps, missing/conflicting primary facts, and complete duplicates remain explicit no-replay paths. This
   classifier/session does not yet replay ambiguous C++ live side effects because timers, reward metadata, sortition
   live-state replay after duplicate admission, pillar post-processing, and broader startup recovery still lack durable
   Rust-owned replay contracts. The
   Rust-mode `VoteManager` overlay now uses the
   approved temporary protected-state hook to inherit unported behavior from `VoteManagerOld` while owning reward-vote
   reset persistence handoff in shim code: it selects the live cert-vote bundle in C++, passes the stage-4 Rust storage
   facts into the Rust-owned finalization apply batch, and mutates live reward metadata only after Rust commits the
   stage. The same overlay now routes deterministic verified-vote live
   state methods through the Rust-backed `VerifiedVotes` facade instead of `VoteManagerOld`: insertion/uniqueness,
   vote presence and snapshots, proposal-vote selection, cleanup, 2t+1 block/bundle lookups, next-round detection,
   current round persistence of non-cert bundles, and network t+1 step reads use Rust-owned metadata. `addVerifiedVote`
   now enters a validation-backed Rust PBFT vote admission runtime: the shim collects only FinalChain/key-manager
   facts, Rust owns canonical validation, replay-marker intent, calculated weight, event/progress fact construction,
   insertion gating, authoritative verified-vote mutation and threshold decisions, duplicate/conflict classification,
   retained slashing payload pairs, and storage-ready extra-reward/current-round 2t+1 payloads. C++ keeps
   live `PbftVote` sidecars, temporary weight hydration for legacy sidecar compatibility, slashing transaction
   construction/submission from Rust-normalized payloads and deferred network effects. Temporary C++ logging around this
   executor boundary is allowed but is not an ownership constraint. PBFT
   vote persistence for own verified votes, extra reward votes, finalized reward-vote resets, and latest-round 2t+1
   bundles now routes through VoteManager-specific `rustaxa-storage` bridge operations and Rust-owned vote payload
  builders: Rust constructs the weighted storage RLP records, raw weighted vote-bundle RLP for persistence, optimized
  PBFT vote-bundle RLP for get-next network egress, and normalized unweighted slashing evidence RLP from canonical
  signed vote bytes plus the authoritative calculated weight. Rust owns the immediate vote-progress write batch and the
  caller-owned own-vote cleanup batch appender, and the Rust slashing planner consumes the normalized evidence before
  building calldata. The shim mutates temporary live sidecars only after Rust accepts the durable operation. `validateVote`,
   `voteAlreadyValidated`, `getPbftTwoTPlusOne`, and
   `genAndValidateVrfSortition` now route away from `VoteManagerOld`: Rust owns validation/replay planning, canonical
   received-vote RLP inspection, signed and unsigned vote hash derivation, recovered voter identity, signature and VRF
   proof checks, Rust-computed received-vote weight, the replay cache, PBFT sortition-threshold formula, Rust-owned
   `2t+1` threshold lookup/current-period cache, and local proposer-sortition screening, while C++ temporarily supplies
   FinalChain/key-manager facts and performs only the live `PbftVote::calculateWeight` sidecar mutation after a Rust
   parity check. Rust now also
   generates local PBFT vote bytes in a side-effect-free bridge API: it derives the VRF proof/output, signs the legacy
   unsigned vote hash, returns canonical signed or weighted `PbftVote` RLP plus hashes/identity facts, and reports
   zero-stake, zero-total-DPoS, and zero-weight outcomes as stable statuses. The shim now materializes local
   `PbftVote` sidecars directly from Rust-generated signed or weighted RLP, hydrates the temporary C++ VRF credential
   cache with local VRF verification, and persists locally generated own votes through Rust storage using the
   Rust-generated weighted bytes. `checkRewardVotes` now calls the Rust verified-vote runtime to build reward-vote
   candidates from Rust metadata, evaluate the preferred round and reverse period scan, and resolve selected retained
   weighted records in the PBFT block's requested hash order; C++ only materializes those records into temporary
   sidecars when callers request copied votes. Rust-retained weighted payloads now back verified-vote snapshots,
   reward-vote materialization, and 2t+1 bundle reads, so missing C++ live sidecars no longer produce partial generic
   snapshot, reward, or 2t+1 results. C++ still owns the temporary live sidecar type,
   FinalChain fact sourcing, reward-vote sidecar mapping, and broader PBFT manager/network orchestration; logging may
   remain temporarily at this boundary but should be ignored when choosing what logic moves to Rust. Rust owns PBFT
   finalization sortition-change persistence: the sortition shim now previews the live Rust runtime transition, returns
   the emitted threshold change for storage staging, and commits the same transition only after the PBFT staged storage
   appender has committed the primary batch. C++ still owns dynamic-lambda live-field assignment from Rust output,
   FinalChain
   dispatch, and live PBFT runtime mutation until those sidecar APIs move across the bridge.
   Reward-vote reset live metadata is still physically mutated in the vote-manager shim, but the action is now an
   explicit Rust-validated executor report instead of an unproved PBFT-manager side effect. The full Rust-mode
   `PillarChainManager` overlay now keeps original pillar
   manager files clean while Rust
   owns deterministic pillar-vote relevance/inspection/insertion and the first pillar-block planning slice: validator
   vote-count deltas are planned in Rust from C++-supplied FinalChain snapshots, and pillar-block first/period/parent
   linkage is validated by Rust before C++ materializes or persists `PillarBlock` objects. C++ still owns bridge
   root/epoch facts, DPoS reads, live `PillarVote` sidecars, signing, storage writes, event emission, network requests,
   and finalization orchestration.
   `DagManager::getNonFinalizedBlocksWithTransactions()` now consumes a Rust-storage-backed sync payload: Rust
   selects non-finalized hashes, loads selected DAG block RLPs, decodes transaction references, de-duplicates transaction
   lookups, and returns transaction RLP results while C++ only reconstructs legacy return objects. The Rust-mode
   `DagManager::setDagBlockOrder()` now calls one Rust apply operation that resolves the anchor level from Rust storage,
   computes the candidate finalization state, applies finalized-block counter updates, expired DAG deletes, and expired
   non-finalized transaction deletes through one Rust storage batch, commits Rust state, then returns only local cache
   and live transaction-manager sidecar cleanup facts to the shim. The Rust-mode `GasPricer` overlay now routes
   finalized-block history restoration through Rust storage, live finalized-block gas-price updates through Rust, and
   pool-mode minimum-price flooring through Rust. Pool mode requires the Rust-backed transaction queue so
   `TransactionManager::getMinGasPriceForBlockInclusion()` reads Rust queue metadata rather than legacy queue state.
10. Port deterministic rewards, remaining slashing-manager/runtime behavior, and pillar calculations after DPoS and final-chain query
    ports are real. The `rewards::Stats` surface now has a Rust-mode overlay: Rust accepts finalized-period facts,
    computes legacy-compatible `BlockStats` RLP, tracks interval cache/distribution boundaries, appends non-boundary
    cache writes to the caller-owned Rust storage batch, and mirrors post-commit interval clears without changing the
    legacy FinalChain ordering. The active Rust `FinalChain` native finalization path now owns a long-lived
    rewards-stats runtime, builds finalized-period facts with bridged previous-block cert votes, persists/clears
    interval cache rows in the finalized-block batch, reloads cached stats on startup, applies interval-boundary
    fee commission rewards from the Rust planner to staged Rust account/DPoS snapshots, and now handles fixed-yield plus
    Aspen part-two dynamic-yield minted block/DAG/vote distribution, total-supply migration, Rust-backed supply/yield and
    delegator reward-page reads, validator paging reads, owner metadata/commission updates, claim balance/cursor updates,
    commission reward claims, claim-all dynamic gas plus legacy batch ABI compatibility, and header `total_reward`
    natively. Moving unsupported DPoS event receipt parity and legacy
    `BlockStats` carrier ownership fully into Rust remain future work. Double-voting proof planning, Rust FinalChain
    double-vote jailing, slashing read calls, and already-verified pillar-vote aggregation are Rust-backed; slashing
    transaction construction/signing, pillar signing/recovery, and
    `PillarChainManager` orchestration still depend on future FinalChain/state ports.

### First Implementation Slice

Start with a consensus rewrite tracker, then implement Rust DAG graph logic as the first code slice.

Tracker: `doc/consensus_rewrite_tracker.md`

The tracker should list:

- consensus classes and public APIs
- direct dependencies on storage, final-chain state, networking, transaction pool, and config
- current tests that cover each area
- proposed ownership: Rust domain, Rust infra adapter, C++ shim, or deferred legacy path

The first Rust code slice should focus on `Dag` graph operations because the domain is narrower than `PbftManager` and gives PBFT/finalization work a stable base.

### Risks

- `PbftManager` is the largest and most coupled consensus class; port it only after DAG, vote, and chain primitives are stable.
- DPoS eligibility depends on FinalChain/state surfaces; genesis-only DPoS query support must stay temporary and visible until
  Rust finalization maintains block-keyed snapshots.
- Finalization crosses DAG, PBFT, storage, rewards, and state execution; port finalization decisions only after the read/query ports are real.
- Consensus behavior is latency-sensitive and persistence-sensitive, so byte compatibility and deterministic ordering tests matter.

### Consensus Validation

Use targeted validation before broad integration runs:

- Rust consensus changes require `cargo fmt --manifest-path rust/Cargo.toml`, `cargo clippy --manifest-path rust/Cargo.toml`, and `cargo test --manifest-path rust/Cargo.toml`.
- DAG changes should run relevant DAG tests such as `dag_test` and `dag_block_test`.
- DAG proposer-routing changes should run Rust validation plus `rust_consensus_tests`, `dag_block_test`, and proposer-path
  PBFT or full-node coverage when thread/network orchestration changes.
- Transaction queue and transaction-packing changes should run Rust validation plus `transaction_queue_shim_test`,
  `transaction_manager_shim_test`, queue/packing-focused `transaction_test` cases, and `gas_pricer_test` when gas-price
  threshold behavior is touched.
- Sortition parameter changes should run `rust_consensus_tests`, `sortition_test`, and the
  `sortition_params_manager_shim_test` overlay check when `RUSTAXA_ENABLE_SORTITION_PARAMS` is enabled.
- PBFT chain/proposed-block/period-data-queue changes should run `rust_consensus_tests`, the corresponding shim test,
  and targeted `pbft_chain_test` or `pbft_manager_test` cases; broader PBFT changes should also run relevant
  `vote_test` coverage.
- Pillar vote aggregation or PBFT sync bundle validation changes should run Rust validation plus `rust_consensus_tests`
  and `pillar_votes_shim_test` when `RUSTAXA_ENABLE_PILLAR_VOTES` is enabled; manager-path changes should also run
  targeted `pbft_manager_test`/`pillar_chain_test` coverage and any affected final-chain or full-node tests.
- Rewards-stat planner changes should run Rust validation plus the Rust rewards-stat unit tests, `rust_consensus_tests`,
  and `rewards_stats_test`; add final-chain/full-node coverage when the C++ rewards stats overlay or reward distribution
  routing changes.
- Shim startup behavior should be validated with a Rust-enabled node smoke test when consensus shims change.

## Validation Matrix

Use the narrowest validation that covers the changed behavior, then broaden for shared or high-risk paths. The dedicated
strategy and repeatable Makefile targets live in `doc/rewrite_validation_strategy.md`.

| Change area | Minimum validation |
| --- | --- |
| Rust-only type/codec changes | `cargo test --manifest-path rust/Cargo.toml` |
| Rust storage bridge | `cmake --build /build --target rust_storage_tests` and `/build/bin/rust_storage_tests` |
| C++ storage behavior | affected C++ gtests or relevant `ctest --output-on-failure` subset |
| Larger storage refactor | storage tests plus `scripts/storage_conformance_diff.sh` after owner confirmation |
| FinalChain read shim | targeted FinalChain/RPC/gtest coverage plus Rust tests for moved logic |
| Finalization/write path | targeted unit tests, conformance where available, and broader CTest/Python integration coverage |
| Upstream sync | pure C++ validation on `cpp-reference`, then Rust-enabled validation on the sync branch |

## Near-Term Priorities

1. Keep storage shim parity green while hardening Rust batch semantics.
2. Add or update conformance coverage for any storage behavior that changes.
3. Continue FinalChain execution-runtime migration: native-supported finalization already routes through Rust, and
   external-EVM orchestration now has Rust-owned request, plan, lifecycle-result, recovery, and publication decisions.
   Publication-row audit coverage now verifies live, recovered, and representative transcript Rust publication against
   persisted FinalChain rows through the normal `rustaxa-bridge` test suite. Next slices should reduce temporary C++
   `StateAPI` fact collection and, if needed for pre-merge confidence, add a live legacy-vs-Rust external-EVM transcript
   fixture without moving EVM ownership into FinalChain.
4. Introduce missing P0 FinalChain domain types with byte-compatible codecs.
5. Keep `cpp-reference` synchronized for C++ intersection changes so upstream sync remains viable.
