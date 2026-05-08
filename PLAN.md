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
  proposed PBFT block cache and period-data queue metadata state.
- `rustaxa-types` contains shared Rust domain and codec types.
- `rustaxa-storage` contains storage repositories that consensus should use through narrow ports.

### Consensus Sequencing

1. Inventory consensus APIs, dependencies, tests, and current Rust shim gaps.
2. Create a tracker that classifies each item as Rust-backed, shim-stubbed, C++-owned temporary, or out of scope.
3. Introduce pure Rust domain types for PBFT rounds, steps, votes, DAG ordering metadata, and eligibility outputs.
4. Port DAG graph operations before `DagManager`: pivot/tip availability, ghost path, ordering, counters, and storage-facing queries.
5. Define Rust ports for DPoS eligibility, eligible vote count, total vote count, and VRF key access.
6. Replace the temporary `dposIsEligible` shim behavior once the eligibility port has a real implementation.
7. Finish the PBFT support slice by adding broader manager-level validation around the now Rust-backed primitives:
   `PbftChain` head updates, persisted-head preview, and next-block validation route through Rust under
   `RUSTAXA_ENABLE_PBFT_CHAIN`; proposed-block membership, validity flags, RLP snapshots, and cleanup planning route
   through Rust under `RUSTAXA_ENABLE_PROPOSED_BLOCKS`; period-data queue admission, effective size, pop vote-source
   decisions, and cleanup planning route through Rust under `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE`.
8. Split `PbftManager` into Rust services for round/step transitions, proposal handling, vote thresholding, and finalization decisions.
9. Port transaction queue behavior before transaction manager orchestration.
10. Port deterministic rewards, slashing, and pillar calculations after DPoS and final-chain query ports are real.

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
- Sortition parameter changes should run `rust_consensus_tests`, `sortition_test`, and the
  `sortition_params_manager_shim_test` overlay check when `RUSTAXA_ENABLE_SORTITION_PARAMS` is enabled.
- PBFT chain/proposed-block/period-data-queue changes should run `rust_consensus_tests`, the corresponding shim test,
  and targeted `pbft_chain_test` or `pbft_manager_test` cases; broader PBFT changes should also run relevant
  `vote_test` coverage.
- Pillar/reward/eligibility changes should run `pillar_chain_test` and any affected final-chain or full-node tests.
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
