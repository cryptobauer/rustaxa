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

Next ready is count4 H1 estimation with actual pinned probes, unchanged C++
search and per-probe isolation. Then new count4 retained-excess H2 trace can use
fresh source contract both-current-four-h2-contract.md: unchangedcount2 helper
admits old1count2 retained for owner after H2partial300, then full400. Actual H2
trace/state-witness measurement remains open. Target authentication does not
reauthenticate old1/owner pairs; preservation needs separate state authority.

See the compact checkpoint for accepted count2/count3 derivatives and exact
routing/check records. Active run3 baseline99->89, latest90at15:39:17.604Z;
9consumed1remains. Check fresh telemetry; preserve previous closed budgets.
General reward graphs, N1–N6 and production cutover remain open. No new prompt
is needed for ready authorized work within the saved budget.

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
