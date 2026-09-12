# Taraxa execution modernization research

Preserve existing-network compatibility while replacing concrete EVM execution and state storage with maintainable
Rust components. The initial migration cannot change valid transaction behavior, historical execution, canonical
roots, receipts, or hashes to accommodate a library.

This research develops the [modernization proposal](../evm_state_db_rewrite_plan.md). Source-level findings establish
requirements and architectural constraints; they do not constitute execution parity or a deployment approval.

## Research phases

| Phase | Deliverable | Status |
| --- | --- | --- |
| 1 | [Compatibility specification and reference evidence](01_compatibility.md) | Complete source assessment; runtime evidence outstanding |
| 2 | [Engine and library assessment](02_engines.md) | Complete source assessment; integration depth requires experiments |
| 3 | [State ownership, migration, and validation design](03_state_migration_validation.md) | Complete source assessment and proposed acceptance gates |

The evidence baseline is repository commit `922dd662502a0f932c033c7de142737b472af9a1` and EVM commit
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. The [baseline manifest](baseline.json) records content hashes and
configured network rules. External sources identify their revision or publication where available. Observations are
as of 2026-09-12; checked-in activation periods are not assertions about current live chain heads.

## Decision summary

Proceed toward a Rust execution/state backend while preserving the existing Rust consensus and FinalChain boundaries.
REVM components are the preferred direction, with the integration depth still subject to feasibility experiments.
Keep RocksDB and the separate concrete state database initially. Defer Ethereum protocol upgrades and substantial
physical storage redesign until compatibility is established.

The decisive findings are:

- Taraxa has a mixed execution profile, wide nonces and unusual charging/rollback behavior. A named Ethereum fork
  configuration cannot express all of it. See the [compatibility specification](01_compatibility.md).
- REVM's full framework contains bounded transaction/account fields and CREATE logic; a lower-level interpreter/host
  integration may fit better. Current evmone also has bounded CREATE nonce handling. See the
  [pinned engine comparison](02_engines.md).
- Existing Rust native kernels should be reused, but their projection checks allow historical encoding alternatives.
  The concrete writer must preserve unchanged bytes and produce exact mutations, rather than rebuild state from
  compact snapshots. See the [state ownership design](03_state_migration_validation.md).
- Cross-database commit/recovery needs explicit persistence ordering, including power-loss behavior. Public snapshots
  also need a defined Rust bootstrap path and paired provenance. See the
  [migration and recovery assessment](03_state_migration_validation.md).
- Modern crypto implementations are not automatically historical substitutes: the Falcon/FN-DSA version and encoding
  need direct compatibility evidence. See the [crypto assessment](02_engines.md).

## Remaining experimental work

The source research defines twelve experiment groups in the [validation design](03_state_migration_validation.md),
covering engine feasibility, envelope/opcode behavior, native mutations, codecs, cryptography, fork replay, public APIs,
crash recovery, import/pruning, rollback and performance. First acquire reproducible reference fixtures and compare the
two REVM integration depths against wide values, nested frames and exact native writes. Verify one concrete commitment
path before expanding to a complete backend.

No final engine selection, implementation estimate, network parity claim or production cutover follows from source
inspection alone. Archive availability, representative runtime results and operational performance remain unresolved.
Those limitations are explicit decision gates rather than assumed successes.

## Evidence labels

- **Established:** directly supported by identified source or configuration.
- **Inference:** architectural consequence of established facts, requiring execution evidence where noted.
- **Proposed:** a design or acceptance criterion for future implementation.
- **Unresolved:** cannot be closed by the available source and documentation alone.

No production implementation, compilation, execution tests, benchmarks, or state migration form part of these results.
