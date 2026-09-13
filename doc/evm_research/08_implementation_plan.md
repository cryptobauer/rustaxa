# Compatible EVM/state rewrite implementation plan

Status: implementation plan; production routing and protocol changes remain unauthorized. This plan follows the
[direction decision](07_direction_decision.md) and incorporates the task owner's subsequent storage constraint,
mainnet snapshot and requested agent assignments. It supersedes the earlier suggestion to evaluate database
consolidation before implementation. Research selects REVM interpreter/host integration; the work below builds and
validates the complete backend.

## Fixed scope and compatibility contract

Preserve current application `db/` and concrete `state_db/` separation, RocksDB storage, general column-family/key
layout, historical encodings and commitments wherever possible. No database consolidation, automatic schema upgrade,
new transaction format or Ethereum fork activation is included. If a concrete implementation blocker requires a
layout change, document the smallest exception and its migration/rollback consequences for task-owner review before
making it. Existing compatibility bugs are reproduced under historical rules, not silently corrected.

Retain Rust consensus, DAG/PBFT ordering, FinalChain sequencing/publication, rewards planning and native business
kernels. Replace Go concrete execution/state behind the existing execution boundary. Preserve arbitrary-width account
nonces, exact native bytes, gas/fee/error ordering, receipts, roots and public query behavior. Physical compatibility
also requires testing database reopen with the intended reference binary; matching roots alone is insufficient.

The first target is a validated backend selectable only in isolated tests. Existing-network routing is a later,
separately authorized slice after the repository parity and smoke requirements pass. No silent production fallback.
The light snapshot supplies bounded real-state evidence; it does not narrow protocol support to its retained periods.

## Snapshot intake and provenance

The task owner supplied `/tmp/snapshot-litenode`, described as a mainnet light-node snapshot captured using RocksDB's
snapshot feature, mostly before the reported network hold and excluding the final blocks. Network status and capture
consistency are user-reported context, not independently established findings. The absence of the latest blocks does
not prevent codec/read/reopen tests at a verified older period.

Filesystem inspection found:

| Item | Observed |
| --- | --- |
| Application database | `/tmp/snapshot-litenode/db/db` |
| Concrete state database | `/tmp/snapshot-litenode/db/state_db` |
| RocksDB options version | 9.10.0 in both latest numbered OPTIONS files |
| Application column families | 36 listed, including genesis, period data, FinalChain headers and receipts |
| Concrete column families | `default`, `1` through `8` |
| Size and file count | About 13 GiB allocated by `du`; about 9.83 GB logical file bytes; application directory has 1,100,386 regular files, state directory 64 |

The [metadata inventory](snapshot_inventory.json) records exact counts, options/column names and SHA-256 hashes of
CURRENT, IDENTITY and the selected OPTIONS files. These hashes identify those small files only; they are not a full
snapshot checksum. Inspection did not open either database through RocksDB, decode head/root metadata, or change files.
The large application file count must inform copy space, inode and runtime planning.

A RocksDB checkpoint can be consistent for one database without establishing a common boundary across two databases.
The precise capture API, producer binary/revision, network genesis/configuration, paired period/root and retained
history remain to be established. An upstream Taraxa snapshot may lack Rustaxa's concrete lifecycle/provenance rows;
absence is a bootstrap case to design, not permission to manufacture authorization metadata or treat it as corruption.

S0 below must:

1. Preserve the supplied directory as evidence. Use an independent copy or copy-on-write clone for database opens and
   mutation tests; do not hardlink mutable files back to the supplied copy. Plan capacity before copying the file set.
2. Produce a full content manifest for the usable source/copy and record the copy method. Retain large manifests and
   database files outside Git; commit compact identities, coverage summaries and reproducible commands.
3. Open the copy with explicitly read-only tooling and compatible options, disabling create/repair/migration paths.
   Verify genesis/network identity, readable column families, producer clues, finalized head and concrete state descriptor.
4. Check matching header/state root and available previous state, code and native records. Record separately the retained
   transaction/receipt range and state range. File timestamps and RocksDB version do not establish chain period or node version.
5. Select reproducible read fixtures and, where both prior state and complete inputs exist, replay windows. If the pair is
   skewed, identify a provably retained common boundary or request a corrected export; do not prune/repair it into agreement.

No further user feedback is needed to prepare these tasks. Ask only if required provenance cannot be recovered, a
layout exception becomes necessary, or a repository approval gate is reached. Full historical replay coverage remains
unresolved even if this snapshot passes all intake checks.

## Module ownership and contracts

Paths below are proposed ownership boundaries, finalized in S1 before parallel edits. Reuse nearby existing Rust types,
handles and storage APIs; do not create a second application manager or turn prototype clones into production journals.

| Boundary / proposed location | Contract | Owner |
| --- | --- | --- |
| Existing `rustaxa-consensus` FinalChain and `consensus_application_runtime.rs` | Supplies ordered period inputs; validates reports; authorizes and publishes one generation | Lead |
| New `rust/crates/rustaxa-evm/` | Taraxa envelope/profile, REVM instructions, frames, journal policy and execution result; no authority to publish or select consensus inputs | Execution worker |
| New concrete-state modules within `rustaxa-storage` | Compatible account/slot/code reads, codecs, trie updates, historical reads and prepared concrete commits in the existing separate database | State worker |
| Narrow native invocation contract and existing FinalChain kernels | Typed operation/period/context → outcome, logs and exact mutation facts; charge/value/nonce ownership explicitly assigned once | Lead, with execution-worker implementation tasks |
| Composition adapter in existing Rust application ownership | Connects execution ports to state and staged kernels; avoids application-lock reentry and dependency cycles | Lead |
| `experiments/evm_feasibility/` and focused conformance tooling | Independent pinned Go oracle, raw fixture inputs, byte/root comparisons and minimized regressions | Supporting worker; reviewer checks independence |

The execution crate may depend on existing domain types and define narrow task ports. Infrastructure implements access
through composition; do not make storage depend on the entire execution/application manager. Exact crate placement is
an S1 implementation detail, not grounds to relocate existing kernels into a new EVM-owned consensus subsystem.

Before implementation workers diverge, document these invariants as typed contracts and reference-based tests:

- Committed reader identity is fixed by period/root; absence, tombstone, pruning and I/O failure are distinct outcomes.
- Ordinary, native raw and transient state have explicit visibility and rollback lifetimes, including new-account
  deletion and overlapping keys. Original/current/new storage values support the correct gas/refund calculation.
- A native outcome serializes only touched rows in the correct historical representation, preserving untouched bytes
  and ordered iterable mutations. Projection-equivalence candidates are validation aids, not a database serializer.
- Frame settlement owns value, gas, code deposit, nonce increments and errors exactly once; all host operations use
  authoritative domain values. No bounded shadow nonce or blanket Ethereum `SpecId` defines Taraxa behavior.
- A prepared commit identifies prior/new period/root and exact intent. Publication follows proven persistence ordering;
  ambiguous writes are reconciled before retry. Query visibility advances only with the published generation.

## Implementation slices and exit criteria

All slices start as TODO except the S0 filesystem inventory. Each closes with its own evidence and Conventional Commit;
completion means the listed behavior and required validation pass, not merely that code compiles.

Implementation update (2026-09-13): [S1 contracts](s1_contracts.md) are implemented,
independently reviewed and pass focused tests plus the repository fast gate.
S0 has independently copied and content-hashed the supplied snapshot and verified
matching mainnet genesis and paired period/root at 25,706,949; retained trie/replay
coverage and capture provenance remain qualification work. The [S2 bounded reader](s2_compatible_reads.md) and [S3 core](s3_execution_core.md)
now pass their focused gates; full execution/persistence and broad historical
coverage remain open under the reviewed contracts. The [S4 integration map](s4_integration_map.md) identifies
existing application APIs. The [first persisted S4 path](s4_persisted_period.md)
executes signed transfer/CREATE/reopened-CALL inputs through those owners and
checks pinned Go results, roots, receipts and CF1–CF5 history. Full S3/S5 execution
coverage and later acceptance gates remain open. Bounded callback-interruption tests
also cover discard/retry before concrete commit and exact publication after lost
acknowledgment, using read-only lifecycle inspection and existing application recovery.
They do not close the S7 fault/import/pruning campaign. Production routing remains unauthorized.

The [mixed-period milestone](09_mixed_period_milestone.md) now completes M1–M6 for its
declared four-period corpus: contract/native custody, nonzero economics, exact persisted
history, reopen and deterministic recovery/retry through existing Rust owners. Its
[evidence matrix](mixed_period_evidence.md) records validation, the RPC rerun anomaly
and remaining E1–E12 limits. This advances S4, selected S5 and bounded S6/S7; it does not
complete this broader S0–S8 plan or authorize production routing.

| Slice | Dependencies / agent | Deliverable and exit criterion |
| --- | --- | --- |
| S0: qualify snapshot | State worker; Spark may build inventory/export helpers | Independent working copy, identities, pair/root checks, retained-range manifest, useful exported fixtures and explicit gaps. No original mutation |
| S1: contracts and test composition | Lead; independent reviewer | Exact module/file ownership, typed interfaces and rollback/publication invariants; unlinked Rust test composition using existing handles. Snapshot-dependent bootstrap details follow S0 |
| S2: compatible reads/codecs | S1; state worker, S0 for real-data gate | Account/slot/code/physical-node codecs and versioned reads; synthetic and qualified snapshot comparisons, corrupt/missing/tombstone cases; no layout changes |
| S3: transaction/profile/frame core | S1; execution worker | General wide-value envelope and mixed profile; calls/creation/value/gas/errors and journals. Compare pinned envelope, frame, storage and transient cases, then expand E1–E3 |
| S4: first complete persisted path | S2 + bounded S3; lead integrates workers | Simple transfer and storage-changing call in a complete synthetic period through existing Rust FinalChain test composition, exact receipts/root, incremental persistence, close/reopen and continuation. Include reward/system effects required by the chosen fixture |
| S5: native and precompile completeness | S3/S4 interfaces; lead assigns non-overlapping modules | Existing DPoS/slashing/reward kernels connected with exact serializers and historical exceptions; precompile ABI/gas/malformed/fork corpus. Match E4/E6 effects, logs, state and continuation |
| S6: complete period/API parity | S2–S5; lead + workers | Empty/native/contract/mixed periods and configured fork boundaries; headers/hashes/receipts/roots; account/storage/code queries, simulation, estimates, traces and delayed reads. Qualified real replay where data permits; synthetic coverage for missing eras |
| S7: recovery/import/pruning | S4 stable commits, S6 semantics; state worker + independent review | Paired bootstrap from existing layout, exactly-once recovery, interrupted writes/import/pruning, retained-root protection and reference rollback/catch-up on disposable copies. No blind marker adoption |
| S8: performance and cutover dossier | S6/S7 + required data/gates; lead | Comparable reference baselines, agreed regression budgets, sustained workload results, snapshot/recovery procedure and all gaps closed for claimed deployment scope. Produce reviewable routing change only when separately authorized |

S2 and S3 can proceed in parallel after S1. S0 qualification can continue alongside S1 and synthetic work. S4 is the
first integration checkpoint, before workers expand every opcode/native method independently. S5 can split into
native adapters and stateless precompiles once ownership is explicit. Recovery design starts in S1, and failure tests
start with S4; S7 completes them rather than postponing durability thinking until the end.

Map the existing [E1–E12 acceptance matrix](07_direction_decision.md#evidence-closure-and-implementation-acceptance)
to slice evidence. A single light snapshot cannot close every activation replay or demonstrate live-network catch-up.
Do not broaden a validated slice's claim beyond the actual reference configuration and retained history.

## Next grouped milestone

The task owner selected [persisted mixed-period parity](09_mixed_period_milestone.md)
as the next larger milestone at baseline `1c3f650b5`. Its M1–M6 slices compose
contract lifecycle, staged native/custody operations, nonzero fees/rewards and
bounded reopen/recovery through existing Rust ownership. The document fixes the
workload, acceptance evidence and exclusions; it is a plan, not a completion claim
for S5/S6/S7 or authorization for production routing.

## Agent and model assignments

Start with one lead and two implementation workers; use an independent reviewer at contract and integration boundaries.
A supporting worker is added only for a task that can run independently. These are initial project assignments, not
Rustaxa model benchmark results or a commitment to maximum concurrency.

| Role | Model / reasoning | Assignment |
| --- | --- | --- |
| Lead / integration owner | `gpt-6-astra`, high; increase for a demonstrated hard problem | S1, shared contracts, composition, sequencing, task/branch ownership, final review and integration |
| Execution worker | `gpt-5.6-sol`, high | S3, assigned S4/S5 execution modules and E1–E4 behavior |
| State worker | `gpt-5.6-sol`, high | S0/S2, incremental writer, persistence and S7 recovery |
| Independent reviewer | `gpt-6-astra` or `gpt-5.6-sol`, high | Reference-to-implementation checks, hidden narrowing/rollback defects, serialization, durability and missing tests |
| Focused coding helper | `gpt-5.3-codex-spark`, supported default/medium setting | Fixture/report tooling, small adapters with settled contracts, localized compiler fixes and repetitive wiring |
| Exploration/documentation helper | `gpt-5.6-terra`, medium; Luna for clear repetitive tasks | Read-only code maps, evidence inventories, documentation and well-specified supporting changes |

Spark is appropriate when inputs, owned files, expected behavior and targeted checks are already explicit. Do not
assign it sole ownership of historical semantics, trie design, cryptographic compatibility or recovery decisions.
Sol/Astra reviews consensus-sensitive helper output against the reference. Escalate ambiguous tasks rather than
letting the helper invent a new policy. All models have the same test requirements; a faster model does not imply
permission to skip tests. Choose models by observed completion quality and rework, not assumed token-price savings.

Official [model guidance](https://learn.chatgpt.com/docs/models) describes Spark as a text-only research preview for
fast coding iteration and notes client/account-dependent availability. The
[subagent guidance](https://learn.chatgpt.com/docs/agent-configuration/subagents) supports per-agent model/reasoning
choices. Check the active tool's model/role support when dispatching: if Spark is exposed only through a fixed
Rust/C++ specialist role, use it for a fitting task; otherwise report the unavailable selection and use the assigned
Sol/Terra worker. Do not silently label another model as Spark. This plan does not modify global Codex configuration.

## Branching, integration and validation

Continue planning/evidence on `feat/rust/evm-state-db`. Before implementation, record the integration base SHA and
create task branches/worktrees from that feature lineage. Use one writer per owned module and keep global manifests,
lockfiles, shared interfaces and `PLAN.md` under lead control. Workers must account for other changes and never revert
them. No concurrent writable opens of the same test database or unsynchronized reuse of the shared CMake build tree.

Each task handoff names: objective, input commit/reference/config, owned paths, dependencies, required tests, explicit
non-goals and expected report. Workers return commits, evidence and unresolved findings; the lead integrates small
reviewed slices and pushes them. Independent review compares original reference behavior as well as implementation;
agent agreement is not a substitute for differential evidence. Experimental branches remain documented and pushed
when they hold useful work; do not route them into production merely because tests compile.

Use the [repository validation strategy](../rewrite_validation_strategy.md) at each slice. Rust changes run the fast
repository gate plus narrow package tests. Every storage-module change includes building/running `rust_storage_tests`;
direct CMake builds use `--parallel 12`. C++ bridge/shim changes need focused target validation and the upstream-diff
audit. No existing tests may be weakened to accommodate rewrite divergence.

Before production routing, require Go execution parity plus applicable C++/Rust boundary parity and a Rust-enabled
subsystem/smoke gate. Ask the task owner before expensive repository-wide or storage differential gates, broad replay,
fault campaigns or sustained operational workloads. Prepare exact commands, dataset identity and scope first. Record
failed and skipped requirements explicitly; aggregate exit success cannot stand in for an unavailable required test.

The immediate next implementation work is S0 qualification and S1 contracts, followed by parallel S2/S3. Layout
consolidation and further engine surveys are not prerequisites. Snapshot qualification, remaining historical coverage
and production acceptance remain explicit gates; no duration or throughput estimate is claimed by this plan.
