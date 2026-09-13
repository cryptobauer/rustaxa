# Milestone: existing-network execution and API parity

Status: authorized implementation, in progress. Baseline `48dd3173b` on
`feat/rust/evm-state-db`, 2026-09-13. This follows the completed bounded
[mixed-period milestone](09_mixed_period_milestone.md) and groups open slices in
the [implementation plan](08_implementation_plan.md); it does not replace S0–S8.

## Outcome and fixed boundaries

Starting from qualified, independently copied existing-network state, execute a
declared contiguous replay window through existing Rust FinalChain ownership,
answer historical queries and simulations, and reopen/recover interrupted
publication with reference-equivalent results.

Preserve existing-network behavior and current separate application/concrete
database layout. Reuse Rust consensus, storage, FinalChain and native kernels.
The EVM backend remains isolated from production routing. No protocol changes,
silent legacy fallback, original upstream C++ edits or automatic schema changes.
The original `/tmp/snapshot-litenode` remains untouched. Its likely producer
`a0e85fe31eb03573cd92c165a5f81035cec9907e` is owner-reported; the exact binary
and capture command remain unknown and must not be upgraded to verified facts.

## Coupled workstreams and acceptance

| Slice | Coupled deliverable | Required exit evidence |
| --- | --- | --- |
| N1: profile and contracts | Fix execution/native activation matrix, API semantics, existing-state qualification inputs and reader/publication authority | Source-reviewed manifest and concrete per-module ownership; explicitly separate reference configuration from unknown producer facts |
| N2: execution and native coverage | Complete frame/native/precompile adapters for the declared profile, including historical behavior exercised by activation fixtures | Independent pinned-reference results for ABI, gas, errors, logs, rollback and ordered mutations; no unsupported path in the declared profile |
| N3: historical reads and simulation | Account/storage/code queries, call, gas estimation and traces over committed historical readers and private execution overlays | Reference-compatible outputs and errors, exact state identity, no committed-state mutation, reopen and unavailable-history cases |
| N4: existing-state bootstrap and replay | Qualify copied state paths, recover complete configuration/reward/system inputs, adopt a proven pair through existing owners and execute real signed periods | At least one contiguous real window with exact receipts, period outputs and roots, qualified prior-state dependencies and reproducible input identities |
| N5: publication, recovery and retention | Combine N2–N4 with interrupted bootstrap/publication, repeated recovery and retained-reader guarantees | Interrupted and uninterrupted runs converge; no partial generation, double application, blind marker adoption or loss of retained roots |
| N6: integrated acceptance | Reproduce the declared matrix and review replacement boundary | Required targeted gates and independent review pass; evidence and reviewable commits pushed; remaining S0–S8 limits stated |

N1 starts with mainnet head period 25,706,949 and its prior descriptor as the
candidate smallest real window. This is a candidate, not replay qualification.
The current mainnet profile and relevant neighboring activation fixtures must be
enumerated before N2 closes. Missing snapshot inputs trigger further bounded
qualification or a concrete export request; synthetic fixtures cannot substitute
for N4's real-window exit. A one-period window may establish the minimum real
replay result; synthetic multi-period continuation still exercises richer mixed
behavior. Broader replay requires separate validation approval.

Keep execution/native and historical API work paired: the same journal and frame
semantics must serve period execution and simulation. Keep bootstrap paired with
recovery: both must establish authoritative descriptors and persistence ordering.
Integrate small slices continuously through the existing differential harness;
do not leave four independent implementations for a final integration step.

## Assignments and ownership

The task owner approved Luna as the bounded Spark fallback because Spark quota
is exhausted. The assignments in 08 apply with these milestone responsibilities:

- Astra lead: shared contracts, manifests, composition, `PLAN.md`, branch/commits,
  integration validation and publication authority.
- Sol execution owner: EVM/frame/native implementation and reference semantics.
- Sol state owner: compatible reader/bootstrap/recovery implementation and state
  qualification on independent snapshot copies.
- Luna helper: narrowly scoped code maps, fixture tooling, settled adapters and
  targeted tests. Medium reasoning for maps, high for bounded implementation.
- Independent Astra/Sol reviewer: original-reference and contract review before
  integration; helper-created expectations never approve their own semantics.

Initial read-only assignments at baseline `48dd3173b`: `milestone_execution`
(Sol high) audits execution/native coverage; `milestone_state` (Sol high) audits
bootstrap/replay dependencies; `milestone_api_map` (Luna medium) maps existing
query/simulation/estimate/trace ownership. Implementation handoffs must record
isolated worktree bases, owned files, inputs, invariants and targeted checks.
One writer per module. Workers must accommodate other edits and never revert
them. Lead alone operates shared `/build`; no concurrent writable database opens.

## Validation and evidence

Each slice uses the narrowest applicable tier in the
[validation strategy](../rewrite_validation_strategy.md), affected Rust tests
and `make rewrite-validate-fast`. Storage implementation changes additionally
build/run `rust_storage_tests` with CMake `--parallel 12`. FinalChain/bridge
changes require the applicable Rust-enabled subsystem checks and boundary parity.
Reproduce the relevant bounded reference exporters; record source/config/input
identities and compare actual outputs, never injected expected roots.

For APIs compare admission, returned bytes/errors, gas/estimate behavior, trace
structure and state identity. For persistence compare receipts, roots, retained
physical bytes and publication/recovery outcomes. Label Go row-model evidence
separately from reference-binary reopen, and logical state equality separately
from physical retained-node differences.

Implementation and targeted validation are authorized. Before expensive broad
replay, repository-wide differential validation, fault campaigns or sustained
workloads, prepare exact commands, datasets and bounds and ask under AGENTS.md.
Exhaustive pruning/power-loss campaigns, sustained performance/resource budgets
and production cutover remain later operational acceptance. Any required gate
that has not run remains open; a milestone is not complete because time, quota
or a single snapshot limits the evidence available.

## Progress

- N1: in progress; [contracts and source inventory](n1_existing_network_contracts.md)
  record actual worktrees, profile/native gaps and existing-state constraints.
- N2: in progress; P-256 primitive and frame integration implemented, BLS and
  Ficus MCOPY implemented; Falcon integration and remaining stateful-native
  coverage remain under validation. Ordered Aspen2 reward phases implemented.
- N3: [gas-search and ordinary simulation evidence](n3_api_parity.md) implemented;
  ordinary persisted historical reads/simulation/reopen now have bounded evidence;
  native simulation and full historical API/trace acceptance remain open.
- N4: [copied-head transaction preflight](n4_replay_preflight.md) passes for 19
  transactions, exact receipts and transaction-only derivation of the retained
  root; bootstrap and qualification of the terminal rewards/system inputs remain open.
- N5–N6: open; no integrated milestone completion claimed. The
  [progress checkpoint](n6_progress_checkpoint.md) records validated chunks,
  interrupted assignments and remaining acceptance work.
