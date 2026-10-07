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

Accepted runtime70399811f adds exact BOTH-current count3 alongside unchanged
count2 runtime072fc39fb and its canonical105-row H1 simulation/estimation/H2trace.
Count3 real caller300+owner100+fullcaller700 oracle38b5a4498, runtime322faults/
freshretries44exclusions and sevenpublicframes e8e439ab0 are accepted. See the
checkpoint and[n2 count3](n2_both_current_three.md). Preserve all oldcorpora,
support, exclusions and runtime guards; count>=2/generalreward behavior is open.

The next named gap is two canonical signers inone committedH1 with count3, then
sameH1 callerfull700 simulation. Use Ckey31repeat32 and derived OkeyA1repeat32,
real300/100 signednonce0 prefixes, exactidentity-updatedgenesis/metadata/membership.
FreshAstra sourceauthority is complete in both-current-three-h1-contract.md.
Newdualpin count/gas/root/completeH1rows remain unmeasured. Then publicRust
finalize/reopen/semantic/concreteisolation can follow. Estimation/H2trace follow
only their separate measured authorities. Active run3 baseline99->89,latest95
at2026-10-07T14:16:49.043Z/4consumed6remain. Preserve prior completed budgets.
No new prompt is needed for authorized ready work within that budget; N1–N6 open.

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
