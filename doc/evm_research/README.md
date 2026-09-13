# Taraxa execution modernization research

Preserve existing-network compatibility while replacing concrete EVM execution and state storage with maintainable
Rust components. The initial migration cannot change valid transaction behavior, historical execution, canonical
roots, receipts, or hashes to accommodate a library.

This research develops the [modernization proposal](../evm_state_db_rewrite_plan.md). Source-level findings establish
requirements and architectural constraints; they do not constitute execution parity or a deployment approval.

Start with the [architecture overview](architecture_overview.md) for the DAG/PBFT → FinalChain → execution/state
flow, current Go ownership, database roles, and what REVM concretely replaces.

The [implementation plan](08_implementation_plan.md) fixes the existing-layout compatibility constraint, records
the supplied mainnet snapshot, and assigns implementation slices to agents/models, including a focused Spark helper.

Implementation evidence now includes [S0 snapshot qualification](s0_snapshot_qualification.md),
[S1 contracts](s1_contracts.md), [S2 compatible reads](s2_compatible_reads.md),
[the bounded S3 execution core](s3_execution_core.md), and
[the first persisted S4 FinalChain path](s4_persisted_period.md).
The latter includes bounded interrupted-publication recovery. S5 adds the
[four original stateless helpers](s5_original_stateless.md) and
[native result application](s5_native_journal.md), plus a
[staged setCommission kernel session](s5_native_kernel_implementation.md).
Dispatcher integration, remaining native methods and the general ordered writer
remain separate implementation work.

## Research phases

| Phase | Deliverable | Status |
| --- | --- | --- |
| 1 | [Compatibility specification and reference evidence](01_compatibility.md) | Complete; bounded runtime evidence in phases 4–6 |
| 2 | [Engine and library assessment](02_engines.md) | Complete; interpreter/host selected in decision |
| 3 | [State ownership, migration, and validation design](03_state_migration_validation.md) | Complete source assessment and implementation acceptance gates |
| 4–6 | [Initial feasibility](04_feasibility.md), [creation frames](05_creation_frames.md), [native/storage/crypto](06_native_storage_crypto.md) | Complete bounded experiments; reproducible dual-reference corpus |
| 7 | [Direction decision](07_direction_decision.md) | Research closed for architecture selection; implementation and release gates explicit |

The evidence baseline is repository commit `922dd662502a0f932c033c7de142737b472af9a1` and EVM commit
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. The [baseline manifest](baseline.json) records content hashes and
configured network rules. External sources identify their revision or publication where available. Observations are
as of 2026-09-12; checked-in activation periods are not assertions about current live chain heads.

## Decision summary

Select REVM's interpreter with a Rustaxa-owned transaction/frame/host layer for implementation.
Reuse existing Rust consensus, FinalChain and native kernels. Retain RocksDB, the separate concrete state database,
and Taraxa's physical encodings and commitments initially. The [final decision](07_direction_decision.md) records
alternatives, ownership, implementation slices and stop conditions. The architecture feasibility question is resolved;
no production switch or protocol change is authorized by this research.

The decisive findings are:

- Taraxa has a mixed execution profile, wide nonces and unusual charging/rollback behavior. A named Ethereum fork
  configuration cannot express all of it. See the [compatibility specification](01_compatibility.md).
- REVM's full framework contains bounded transaction/account fields and CREATE logic; the executed lower-level interpreter/host
  probes support selecting that integration. Current evmone also has bounded CREATE nonce handling. See the
  [pinned engine comparison](02_engines.md).
- Existing Rust native kernels should be reused, but their projection checks allow historical encoding alternatives.
  The concrete writer must preserve unchanged bytes and produce exact mutations, rather than rebuild state from
  compact snapshots. See the [state ownership design](03_state_migration_validation.md).
- Cross-database commit/recovery needs explicit persistence ordering, including power-loss behavior. Public snapshots
  also need a defined Rust bootstrap path and paired provenance. See the
  [migration and recovery assessment](03_state_migration_validation.md).
- Modern crypto implementations are not automatically historical substitutes: the Falcon/FN-DSA version and encoding
  need direct compatibility evidence. See the [crypto assessment](02_engines.md).

## Bounded feasibility checkpoint

[Executable E1–E5 findings](04_feasibility.md) now accompany the source reports.
The isolated harness reproduces both pinned Go references, compiles REVM probes,
and independently verifies account/slot and native-mutation commitments. That historical checkpoint
recommended interpreter/host prototyping; the final decision below incorporates
the subsequent native and physical-node experiments. Production remains unchanged.

The [creation frame continuation](05_creation_frames.md) now executes a bounded
Rust frame driver against 16 dual-reference cases, including wide nonce growth,
collisions, nested rollback and exact account-state roots.

## Research closeout and remaining implementation gates

The [final experiments](06_native_storage_crypto.md) invoke the existing Rust native kernel during REVM calls,
match exact native mutations and combined roots, reopen physical node history, and execute a mixed gas/opcode profile.
Historical Falcon vectors match Rust verifier 0.3.0; 0.4.0 rejects all three historical valid signatures tested.
No upstream REVM patch was required by these probes.

All 12 isolated tests and the six-case native overlay comparison pass; both Go references reproduce exactly.
The [decision matrix](07_direction_decision.md#evidence-closure-and-implementation-acceptance) carries E1–E12 forward
as implementation and release acceptance criteria, with explicit evidence limits. The subsequent S4 implementation
adds an incremental writer and a bounded atomic persisted path. Complete execution, full historical replay, API
parity, full recovery/import/pruning and performance acceptance remain open. These are not a reason to
repeat engine-selection research.

The official endpoint attempts failed during research. A task-owner-supplied mainnet light snapshot is now available
locally with both database directories. [Qualification](s0_snapshot_qualification.md) on an independent copy
establishes matching mainnet genesis and paired period/root at 25,706,949, with bounded prior-period reads.
The owner identifies their lite node and likely producer commit; exact binary/capture provenance and full retained
historical coverage remain unresolved. The original snapshot is preserved.
The [head-input qualification](s0_head_replay_inputs.md) verifies the head period's
19 ordered transactions and receipts against the stored header roots, while
leaving configuration, reward-vote inputs and prior-state closure unqualified for replay.
Full historical replay remains an acceptance gap. No network parity, performance or delivery-duration claim is made.

## Evidence labels

- **Established:** directly supported by identified source or configuration.
- **Inference:** architectural consequence of established facts, requiring execution evidence where noted.
- **Proposed:** a design or acceptance criterion for future implementation.
- **Unresolved:** cannot be closed by the available source and documentation alone.

The three original reports contain source assessment only. Compilation and execution
evidence belongs to the linked experimental reports; no production cutover, benchmark
or state migration forms part of either assessment.
