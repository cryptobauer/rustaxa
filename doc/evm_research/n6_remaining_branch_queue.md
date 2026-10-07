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

Accepted runtimea16acef1e adds separate exact BOTH-current count4 beside
unchanged count2/count3. Repeated owner partial is measured: AA300/O100/O100
leaves both nodes4; fullcaller700 preserves source4 and raises destination5.
Oracle0cea670f9, runtime370faults/freshretries46exclusions and sevenfourjournal
frames b4f62f303 are accepted. Canonical two-signer H1 with C0/O0/O1 is accepted
be90ad91f:108completeGo rows, publicfinalize/reopen/sameH1 simulation and complete
public owner physical inventory isolation. Preserve prior corpora/support/tests.

Count4 H1 estimate67f422b5e and retained-excess H2 trace05e92e685 are accepted.
Actual H2 partial300 leaves old1count2 for owner, then full400 uses unchanged
count2 helper. Actual default trace and a separate live witness prove state;
target authentication excludes old1/owner pairs. Required targeted/package/
bridge evidence, independent reviews and exact hashes are in the checkpoint.

Next ready dependency is the source-only
[positive-reward partial contract](n2_rewarded_partial_redelegate_contract.md).
Measure actual dual-pin P1 reward/commit and P2 partial300 first. Source predicts
payout396; no new oracle, positive-reward adapter or runtime acceptance exists.
Preserve all earlier zero-reward profiles and tests. Reuse existing reward graph
and account owners only after exact account/raw/log ordering is measured.

Run3 is closed: baseline99%, floor89%,10points consumed, reset1791948695.
No new starts with that saved budget. A later authorized run must save a fresh
baseline in a new directory. Previous closed budgets remain immutable.
General rewards, N1–N6, real-window/root parity and production cutover remain open.
Nothing was pushed.

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
