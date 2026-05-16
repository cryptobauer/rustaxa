# Rust Rewrite Plan

This is the consolidated plan for progressively replacing C++ internals with Rust while keeping the node buildable, testable, and syncable with upstream.

## Objectives

- Keep upstream C++ sync practical through a pure-C++ validation gate.
- Preserve public C++ APIs while Rust implementations replace internals behind shims.
- Move high-value storage and FinalChain paths first.
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

### Current Batch Status

- Batch 1 complete: additive shim scaffold plus Rust-backed chain index reads:
  - `lastBlockNumber`
  - `blockNumber`
  - `blockHash`
- Batch 2 complete: Rust-backed block header and transaction index reads:
  - `blockHeader`
  - `transactionLocation`
  - `transactionCount`
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
  - Rust finalization appends DPoS snapshots for finalized native-transfer blocks. Because Rust finalization is scoped to
    post-Magnolia execution, native transaction fees are assigned to validator commission rewards by finalized DAG block
    author and transaction hash.
  - non-genesis DPoS queries still throw when the queried block has not been finalized through Rust snapshot
    maintenance; unsupported state/EVM DPoS transitions remain explicit gaps.
  - selected DPoS precompile reads through `FinalChain::call` are Rust-backed for `getTotalEligibleVotesCount()` and
    `getValidator(address)`.
- Unimplemented public shim methods throw rather than falling back to `FinalChainOld`.

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

1. Keep read/index parity stable.
2. Migrate transaction, receipt, log query helpers, and bloom search parity.
3. Migrate finalization/write path pieces such as append-block, counters, and index writes.
4. Continue DPoS snapshot parity beyond native transfers: validator owner/metadata, delegation mutations, jailing,
   slashing, rewards distribution, and contract-call state transitions.
5. Defer broader StateAPI, bridge-heavy APIs, pruning, snapshots, and state-transition boundaries until there is a clear Rust/EVM integration strategy.

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
  transaction lookup and non-finalized recovery payload loading, Rust-planned finalized transaction filter/verification
  helpers, Rust-planned transaction verification and validated-insert admission, shim-owned live non-finalized/pool/count
  read helpers, and a Rust-backed `GasPricer` oracle for finalized-block history, minimum-price flooring, and percentile
  bid selection.
  The Rust-enabled `SlashingManager` overlay now routes deterministic double-voting proof planning, duplicate-proof
  cache decisions, submitter selection, and slashing contract calldata construction through Rust while C++ keeps live
  vote objects, account reads, gas bidding, transaction signing, and transaction-pool insertion.
- `rustaxa-types` contains shared Rust domain and codec types.
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
4. Port DAG graph operations before broader `DagManager` orchestration: pivot/tip availability, ghost path, ordering,
   counters, storage-facing queries, and deterministic `verifyBlock` reject decisions.
5. Define Rust ports for DPoS eligibility, eligible vote count, total vote count, and VRF key access. The current
   `DagManager` shim now gets those DPoS/VRF facts from a Rust FinalChain bridge bundle and routes embedded VRF proof
   verification, DAG VDF payload decode, difficulty calculation, legacy-modulus Wesolowski proof check, status-coded
   VDF/DPoS fact envelope, legacy VRF/VDF message construction, verify-side VDF denominator policy, and reject ordering
   through Rust. The Rust-mode `DagBlockProposer` overlay now routes proposer eligibility status decisions, legacy VRF
   input construction, and deterministic tip selection through Rust while preserving the C++ thread/network shell.
6. Replace the temporary `dposIsEligible` shim behavior once the eligibility port has a real implementation.
7. Finish the PBFT support slice by adding broader manager-level validation around the now Rust-backed primitives:
   `PbftChain` head updates, persisted-head preview, and next-block validation route through Rust under
   `RUSTAXA_ENABLE_PBFT_CHAIN`; proposed-block membership, validity flags, RLP snapshots, and cleanup planning route
   through Rust under `RUSTAXA_ENABLE_PROPOSED_BLOCKS`; period-data queue admission, effective size, pop vote-source
   decisions, and cleanup planning route through Rust under `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE`.
8. Split `PbftManager` into Rust services for round/step transitions, proposal handling, vote thresholding, and finalization decisions.
9. Port transaction queue behavior before transaction manager orchestration. The Rust-mode `TransactionQueue` overlay
   now routes deterministic queue metadata, per-account nonce ordering, same-nonce replacement, non-proposer expiry
   planning, pool limits, gas-price threshold accounting, queued transaction RLP payload retention, known-transaction
   cache expiry, overflow/drop observation state, and finalized-account purge planning through Rust while C++
   materializes `Transaction` objects on demand and keeps FinalChain account reads for purge fact sourcing. The Rust-mode `TransactionManager` packing shim now routes proposal candidate
   sizing, declared-gas fit checks, invalid-estimate demotion decisions, accepted gas accumulation, and stop rules
   through Rust while C++ keeps `estimateTransactionGas`, estimation caching, and lifecycle/finalization orchestration.
   The TransactionManager shim now owns an opaque Rust runtime handle for live queue metadata/payloads, known-cache
   state, non-finalized and recently-finalized transaction sidecars, and the authoritative transaction count. DAG
   transaction persistence sends transaction/account facts to Rust; Rust owns sidecar membership checks, duplicate
   filtering, nonce-gated finalized-storage lookup, accepted ordering, count planning, the storage batch, accepted
   non-finalized sidecar insertion, and accepted queue erasure before C++ logs removals. Finalized transaction status
   updates now send finalized hashes and RLP payloads to Rust; Rust plans count increments, retention eviction, periodic
   queue purge, recently-finalized sidecar insertion, non-finalized sidecar removal, known-cache marking, and queue
   erasure while persisting `TrxCount` before C++ logs side effects.
   `excludeFinalizedTransactions` and `verifyTransactionsNotFinalized` now collect only hash/nonce facts in the shim,
   then call Rust for sidecar membership, finalized-storage checks, and deterministic filtering/short-circuit decisions.
   `verifyTransaction`, `insertTransaction`, and `insertValidatedTransaction` now collect transaction/config/cache/account
   facts in the shim and call Rust planners for exact verification reasons, public insertion result mapping, and
   proposable/non-proposable admission before C++ mutates the live queue; known-hash insert decisions now route through
   the Rust insert planner instead of a shim-local early return, and `isTransactionKnown` now includes Rust sidecar
   membership checks alongside queue-known state. Rust now returns explicit validated-insert queue actions and finalized
   known-cache/pool mutation actions so shim code applies side effects directly from Rust-planned intent instead of
   inferring local action intent. The Rust-mode facade now owns the public
   `transaction_added_` event surface and emits it from shim-owned code for Rust-planned proposable admissions before
   live queue insertion, matching legacy event timing. Live pool helpers remain shim-owned under the existing
   transaction mutex and no longer forward to `TransactionManagerOld`; they now materialize from Rust runtime queue or
   sidecar RLP. The Rust runtime state exposes the authoritative Rust-mode transaction count and drives count reads
   after persistence/finalization commits. Remaining live-shell gaps are Rust ownership of FinalChain purge fact
   sourcing and estimation/lifecycle orchestration.
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
10. Port deterministic rewards, remaining slashing behavior, and pillar calculations after DPoS and final-chain query
    ports are real. Double-voting proof planning and already-verified pillar-vote aggregation are Rust-backed; broader
    slashing state transitions, pillar signing/recovery, and `PillarChainManager` orchestration still depend on future
    FinalChain/state ports.

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
3. Continue FinalChain read/query migration before write/finalization migration.
4. Introduce missing P0 FinalChain domain types with byte-compatible codecs.
5. Keep `cpp-reference` synchronized for C++ intersection changes so upstream sync remains viable.
