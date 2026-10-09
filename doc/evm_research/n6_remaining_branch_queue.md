# Remaining branch execution queue

Baseline: `5bfdf494c`, `feat/rust/evm-state-db`. This queue orders the remaining
[authorized milestone 10](10_existing_network_milestone.md#coupled-workstreams-and-acceptance)
dependencies. It does not replace its acceptance matrix or approve broad replay,
production routing, supplied-data changes or new publication authority.

## Execution order

| Work | Next acceptance gap | How to proceed |
| --- | --- | --- |
| N1: execution/profile contract | Source-reviewed activation matrix, native/API semantics and owner authority | First settle one declared profile and its next uncovered stateful-native path. Pin source/configuration and neighboring activation fixtures. Separate reference configuration from unknown producer facts; request missing inputs precisely. |
| N2 and N3: native execution and API parity | Stateful-native coverage and native historical simulation/trace parity | Implement agreed behavior through existing Rust owners. Pair execution and simulation using the same journal/frame semantics. Compare actual outputs, errors, gas, logs, rollback and mutations with pinned references. Parallelize only independent files/owners. |
| N4: qualified real window | Complete historical native/configuration/reward/system dependencies and real signed-period execution | Use identity-qualified inputs under the existing data rules. Resolve completeness and producer-policy gaps. A candidate one-period window is the minimum milestone exit; cold synthetic success cannot replace receipts, outputs and root parity. |
| N5: recovery and retained readers | Interrupted/uninterrupted convergence across N2–N4 | Integrate through existing owners. Prove no partial generation, double application or retained-root loss. Broad fault campaigns still require exact preparation and approval. |
| N6: integrated acceptance | Declared matrix, required gates and independent replacement-boundary review | Close only executed gates. State all remaining S0–S8 limits. Push only with explicit authorization; production cutover remains separate. |

N2/N3 work with settled contracts can progress while external N4 input requests
are pending. Do not claim the real-window gate is closed by another synthetic
fixture. Do not repeat the completed cold lifecycle or its hardening task.

## Next ready chunk

The original pending malformed-claim slice is closed as `e940e9419`. Subsequent
accepted work includes claim sequences and frames, finite pending claim-all,
its public trace, and finite literal H1 claim-all. Read the current
[checkpoint](n6_restart_checkpoint.md) and its run artifacts for exact commits,
required gates and limits. Old interim run 4 and earlier run facts are stale.

H1 runtime `23e859793` retains exact HistoricalSimulation 1/PendingPeriod 2
admission and has all 12 required gates plus frozen independent acceptance.
Funded H1 estimate oracle `b8a14fa44` measures caps 200000/111272/111273 and
5/1/1 probes with estimates 116817/111272/111273. The public estimation derivative
uses unchanged runtime and fresh disposable sessions for each callback.
Its closeout, when accepted, owns final test/review/commit status; otherwise
finish the exact pending files and checks in `current-pending.json` first.

After that derivative closes, the next distinct profile must first have an
available fresh reviewer context. Underfunded claim-all boundary measurement
can follow the settled source contract in `claim-all-underfunded-contract.md`.
The proposed probes are 21271/21272/21273/90000/111271/111272/111273. Actual
quote/run behavior must be measured before runtime design. Simply removing the
preparation gas check would permit scalar execution before a late failure.
A public H1 trace is another separate derivative; obtain its source contract
and actual dual-pin trace before implementation. Neither is already accepted.

The run budget remains 77% baseline and 67% floor, reset 1791948695. Latest
interim 69% means 8 points used and 2 left. Refresh the same saved budget before
new work. Final records supersede this observation. At the floor stop new
work, delegation and gates; checkpoint exact pending bytes and checks. No push.

## Contract work for later gaps

Use [N1 contracts](n1_existing_network_contracts.md) and relevant source ranges
to name one executable stateful-native profile gap, its Rust owner, independent
oracle and exit tests. Reuse settled decisions. If historical/authority semantics
remain unresolved, obtain bounded Astra contract review before dependent code;
record missing producer facts without inventing them. This is a focused contract
closure, not a new branch-wide inventory.

Follow the [slice workflow](../codex_slice_workflow.md): Sol implements serial
work directly; Luna checks bounded inputs; independent review follows freeze.
Update the [scorecard](../codex_slice_scorecard.md) for comparable tasks. Each
handoff names the completed gate, next ready dependency and concrete blocker.
