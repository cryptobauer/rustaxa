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
| 2 | Engine and library assessment | Pending |
| 3 | State ownership, migration, and validation design | Pending |

The evidence baseline is repository commit `922dd662502a0f932c033c7de142737b472af9a1` and EVM commit
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. The [baseline manifest](baseline.json) records content hashes and
configured network rules. External sources identify their revision or publication where available. Observations are
as of 2026-09-12; checked-in activation periods are not assertions about current live chain heads.

## Evidence labels

- **Established:** directly supported by identified source or configuration.
- **Inference:** architectural consequence of established facts, requiring execution evidence where noted.
- **Proposed:** a design or acceptance criterion for future implementation.
- **Unresolved:** cannot be closed by the available source and documentation alone.

No production implementation, compilation, execution tests, benchmarks, or state migration form part of these results.
