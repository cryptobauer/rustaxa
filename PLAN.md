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
- Rust domain modules define narrow ports for required capabilities.
- Infrastructure modules implement those ports over RocksDB, CXX bridges, or test fakes.
- Runtime/bootstrap code wires concrete implementations together.
- Avoid "everything bag" context structs and broad service locators.
- Keep dependency direction one-way: domain -> ports, infra -> port implementations, runtime -> wiring.

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

Rust-mode shim implementations live in `libraries/core_libs/storage_shim/src/storage_shim.cpp`. Legacy `libraries/core_libs/storage/src/storage.cpp` remains legacy-only logic.

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
4. Defer StateAPI, DPoS, bridge-heavy APIs, pruning, snapshots, and state-transition boundaries until there is a clear Rust/EVM integration strategy.

High-risk APIs:

- `finalize` / `finalize_`
- `prune`
- `updateStateConfig`
- `call`
- `trace`
- state and DPoS query surfaces that depend on EVM/state integration

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

## Validation Matrix

Use the narrowest validation that covers the changed behavior, then broaden for shared or high-risk paths.

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
