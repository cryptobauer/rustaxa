# Milestone: persisted mixed-period parity

Status: implementation in progress; not accepted. The task owner authorized completion on
2026-09-13. Current contracts and evidence: [M1 handoff](m1_mixed_contracts.md). Planning baseline: `1c3f650b5` on `feat/rust/evm-state-db`.
This milestone groups reviewable implementation slices under the existing
[implementation plan](08_implementation_plan.md); it does not replace its S0–S8 gates.

## Outcome

Execute a reproducible sequence of signed, mixed contract/native transactions through
existing Rust FinalChain ownership, persist exact concrete state in the current separate
application/state databases, reopen, recover and continue with the same results as the
pinned reference for the named fixture configurations.

Completion requires contracts, native state, fees and rewards to agree in one composed
path. Passing isolated opcode, serializer or native-kernel tests does not close this
milestone. It advances S4, selected S5 behavior and bounded S6/S7 integration; it does not
claim complete native coverage, all historical rules or deployment readiness.

## Starting evidence and gaps

| Available at the baseline | Integration work still required |
| --- | --- |
| Signed transfer/CREATE/CALL through FinalChain, ordered concrete overlays, atomic commit and reopen | Contract deletion/revert and native effects in the same persisted corpus |
| SELFDESTRUCT and restored ordinary-slot flush match Go logical mutations | Exact incremental roots, retained physical rows and continuation for those effects |
| Real staged `setCommission` kernel, EVM native frame handling and rollback observations | Projection transcript, complete native catalog, period/session lifetimes and publication |
| Existing Rust delegation, undelegation and rewards business kernels | Narrow execution adapters and exact ordered raw/account serializers |
| Deterministic before-write/lost-acknowledgment recovery tests | The same interruptions with mixed native/ordinary state and nonzero economics |
| Paired light snapshot head and bounded input qualification | Full state closure, producer/capture verification and historical replay remain separate |

Use the linked slice evidence as the baseline: some older research tables and initial
slice documents describe gaps that later commits have closed.

## Required workload

Build a finite synthetic corpus with one declared post-Cornus configuration and exact
activation heights, genesis allocations/validators, reward inputs and signed transaction
bytes. Select the supported V2 undelegation family explicitly; do not treat V1 and V2 as
interchangeable. Add focused neighboring-height fixtures for rules actually exercised.

The main sequence must span enough consecutive periods to exercise the configured
undelegation delay and reward-reference transitions, with at least two close/reopen
boundaries and a final continuation transaction. Keep delay and stake parameters small
only where the real configuration permits; export them in the manifest.

Required behaviors:

- Transfer, CREATE, storage-changing CALL, nested ordinary revert, SELFDESTRUCT, and a
  later transaction observing account deletion. Preserve prior physical slot history.
- Native `setCommission`, payable `delegate`, `undelegateV2` and `confirmUndelegateV2`,
  routed through existing Rust kernels with ordered touched-row serializers. Include
  success, funded business failure, insufficient native gas and enclosing-frame revert.
- Multiple native calls in a transaction and across transactions in one period. Preserve
  period sequence, current raw state and the reference's distinct cache/snapshot lifetimes.
- Interleave an already implemented stateless precompile with consensus-native calls to
  prove their identities and effects remain separate under the real period adapter.
- Nonzero gas prices, validator/delegator state, fee distribution and at least one nonzero
  minted-reward effect using the existing rewards planner and kernels.
- Raw effects surviving ordinary revert, reverted ordinary values flushed because another
  mutation survives, custody-account balance changes and iterable insertion/deletion.

System-transaction inputs must come from the real planner and declared configuration.
An empty system stream is allowed only when explicitly proved for those periods. This
milestone does not require a new bridge/pillar system-action implementation.

## Delivery slices and order

| Slice | Deliverable | Exit condition |
| --- | --- | --- |
| M1: exact workload and adapter contracts | Versioned fixture/config manifest; projection representability audit; staged native-to-reward API; explicit batch/observer cache lifetimes | Independent source review; no silently narrowed values, dropped required context, manager reentry or unqualified root reconstruction |
| M2: persisted ordinary lifecycle | Extend the real signed-period path with deletion, rollback, restored-slot flush, historical reads and continuation | Both Go pins match intermediate/final roots, receipts and named physical maps after reopen |
| M3: first persisted native period | Compose `setCommission` session, journal, ordered writer, complete native catalog and FinalChain projection validation | Multiple calls/transactions, parent revert and failure publish exact native/ordinary state; next period continues after reopen |
| M4: native custody lifecycle | Add delegate/V2 undelegate/confirmation adapters using existing kernels, full-width balances and operation-owned ordered serializers | Exact ABI/gas/errors/logs, custody balances, iterable rows and delayed confirmation match Go; no double transfer or nonce ownership |
| M5: mixed economics and period outputs | Integrate nonzero fees/rewards into the workload; derive post-transaction and post-rewards roots through the writer | Real planner inputs, reward accounting, exact receipts/header fields/hashes and native semantic state agree with appropriate reference sources |
| M6: mixed-state recovery and closeout | Apply bounded commit interruptions, reopen/query checks and continuation to the complete corpus; publish evidence matrix | No partial generation visibility or double native/reward application; replay and uninterrupted runs converge to identical committed results |

M1 fixes an explicit scenario matrix. Keep the full 21-frame/nine-lifecycle SELFDESTRUCT
corpus as targeted regressions; use representative deletion, outer revert, restored-slot
flush and retained-orphan cases in the persisted sequence rather than multiplying every
unit row into a separate chain fixture.

M1 is the first checkpoint. After it, M2 and native serializer work for M4 can proceed
independently. Integrate M3 before combining payable native work into M4. M5 depends on
M3/M4; M6 closes the composed behavior. Each row may produce several small reviewed
commits; a row is not complete merely because its scaffold compiles.

## M1 prerequisites that must be resolved before integration

Export canonical native genesis rows and the complete initial catalog from actual Go
initialization. Prove that they agree with the Rust semantic snapshot before starting the
first native session. Do not seed an independently invented five-row logical map and call
it an initialized native period.

M1 also proves that the selected valid configuration and finalized DAG/certificate facts
produce a nonzero reward. Use existing Rust reward tests to locate the kernels and construct
valid facts, but never copy their fake transaction inputs into the signed oracle corpus.
This feasibility check must precede committing to the main genesis/workload, rather than
waiting until M5 to discover that its economics are neutral or inconsistent.

The current native session exposes begin/prepare/invoke, while reward execution helpers
remain private FinalChain methods. Define a narrow staged native-to-reward phase API that
reuses those kernels and carries the accepted semantic state into rewards. Do not reset the
session at this boundary or reenter the application manager to reconstruct its state.

Existing projection replay has a 256-bit transaction-value boundary and does not encode
every S1 frame-context field. Audit each selected payable/nested path for representability
and validation. Retain exact widths and context where behavior requires them; document
which facts are implied by the admitted call/address constraints. Do not silently truncate
or drop required facts to fit the current adapter. Any needed interface/codec change needs
an explicit compatibility review within M1 before implementation.

The existing public batched lifecycle and concrete-observer mode can differ in native
cache lifetimes. Run the mixed/native reference corpus through both. Explain every
observable difference from source and record the exact execution mode being replaced.
An unexplained difference, or a Rust result that matches only an incompatible observer
mode, blocks the milestone; synthetic root agreement cannot waive existing-network
behavior. Differences limited to extra retained physical trie nodes may be accepted only
when explicitly separated from execution/root/receipt equality in the evidence.

## Ownership and composition

Use the assignments in the implementation plan, with one writer per assigned file:

- Lead (Astra): shared contracts, existing FinalChain/application authority, projection
  adapter, `persisted_period_reference.rs` integration, manifests, branch and commits.
- Execution worker (Sol): EVM/frame changes and assigned native-session/serializer modules.
  New payable operations must use task-oriented ports into consensus-owned kernels.
- State worker (Sol): concrete writer/read/lifecycle changes and focused persistence tests.
  Reuse sealed phase outputs and borrowed prepared views; no reader publication authority.
- Supporting helper (Spark when available): bounded fixture export/manifest tooling and
  mechanical wiring after inputs and expected semantics are settled. Semantic oracle
  design and approval remain with lead/reviewer; Terra can perform read-only maps.
- Independent reviewer (Astra or Sol): reference behavior, rollback/cache ownership,
  exact serialization, projection independence and commit/recovery evidence.

Document actual assignments and isolated worktree bases before implementation. If a model
is quota-limited, record the limitation and reassign explicitly; do not mislabel another
model. The lead owns the shared `/build` tree. Avoid concurrent mutable database opens.

Continue to use `execute_final_chain_application_task`, the existing staged native and
reward kernels, `ConcreteStateLifecycle` and existing projection/receipt codecs. Keep
consensus/storage free of a production dependency on the EVM crate. No new C++ orchestration,
parallel chain manager, production fallback or changes to original upstream C++ are planned.

## Evidence and acceptance

Every successful reference run must derive state from actual Go execution and incremental
TransitionState/TrieSink updates, beginning at the declared prior state. A reconstructed
final map alone cannot establish persisted roots. Export both public-batched and concrete-
observer behavior, including native/cache and transaction-to-reward phase effects.
The Go row-model comparison does not
establish Go RocksDB/reference-binary reopen; label those evidence classes separately.

For each transaction/phase/period, compare:

1. Admission, status/error/return bytes, gas/refund, nonce/value/fee effects, surviving logs
   and stored receipt bytes, with the exact signed ordering from FinalChain inputs.
2. Native invocation identities, original logs, rollback dispositions and ordered raw
   mutations; the complete catalog, including untouched entries and deletion markers.
3. Intermediate, post-transaction and post-reward roots, exact physical account/slot/code
   bytes and retained CF1–CF5 history under the concrete-observer boundary. Explicitly
   identify any latest-view or metadata rows outside the reference comparison.
4. Rust publication descriptors, provenance and pending-marker transitions; headers and
   hashes use the appropriate existing FinalChain reference codec/source, not a fabricated
   Go EVM header oracle. No expected root may be substituted into executor output.
5. Current and retained historical reads through existing reader/query ports before and
   after reopen. Full RPC simulation/estimate/trace and pruning semantics are separate work.

Use the existing callback interruption boundaries: after durable application intent but
before concrete write, and after concrete commit with acknowledgment lost. Close owners,
recover through the existing application entry point, repeat recovery to prove idempotence,
and execute the continuation. These deterministic tests are not a disk-fault campaign.

M6 acceptance also requires negative tests for stale/missing native catalog entries,
incorrect invocation order/disposition, mismatched prior identities and inconsistent
prepared commit intents. Invalid reports must fail before either committed descriptor
advances. Retry a valid report from clean/reopened owners and prove that the rejected
report cannot contaminate publication.

## Validation and authorization boundaries

Per code slice: run the narrow affected package/test targets and `make rewrite-validate-fast`.
Reproduce only the relevant bounded Go exporter. Every storage-module change also requires:

```sh
cmake --build /build --target rust_storage_tests --parallel 12
/build/bin/rust_storage_tests
```

Choose the applicable subsystem tier in `doc/rewrite_validation_strategy.md` for any changed
FinalChain or bridge boundary; record exact targets before running, verify Rust mode where
required, and never count skipped targets as passes. New focused test/exporter names belong
in the M1 handoff and each slice's evidence document. Documentation-only planning needs
link/whitespace checks, not runtime tests.

Implementation and targeted validation retain the existing authorization. Expensive
repository-wide/differential gates, broad replay, fault campaigns and sustained workloads
still require a concrete command/dataset/scope proposal and task-owner approval under
AGENTS.md. No such campaign is needed merely to write this plan.

The original `/tmp/snapshot-litenode` remains untouched. Its likely producer commit and
unknown capture command stay recorded; the synthetic milestone does not depend on resolving
them and cannot qualify the snapshot for import or full replay.

Full precompile/native completeness (including BLS/P256/Falcon, slashing and other DPoS
methods), all historical activation combinations, reference-binary reopen, import/pruning,
operational budgets and production routing remain outside this milestone. A layout or
protocol exception, silent legacy fallback, or loss of existing Rust ownership requires
stopping that design and escalating; it is not an implementation shortcut.

## Definition of done

All M1–M6 exit conditions pass for the declared corpus, independent review is complete,
required checks have no unexplained failures/skips, and reproduction/evidence plus reviewable
commits are pushed. Remaining E1–E12 gaps are listed explicitly. Partial native support,
neutral rewards or reconstructed-only roots cannot be reported as completion of this milestone.
