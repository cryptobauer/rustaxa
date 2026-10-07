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

Run4 accepted positive-reward partial oraclec652fc315, isolatedadapter4c82359,
actualframeoraclefff3c4e and publicRustframes e14922b. RealP1 mint800/sourcepool792/
commission8/native4800, P2 partial300 pays396 with12raw/2account/2logs. Public
FinalChain P1 finalize/reopen and boundP2 frames pass allrequiredchecks/reviews.
Go parentREVERT restores payout accounts/logs while native raw writes survive.
The corrected frame view has52unique keys, including actual minted_tokens and
absent total_supply; allrawvalues are resolved from complete physicalP1.
Prior zero/count2/count3/count4 corpora and guards remain unchanged.

Next: actual canonical positive-reward P1 same-H1 DryRunner measurement before
historical adapter work. Fresh Astra source contract and corrected Luna map live
in run4 artifacts. Go andRustsameH1 use literalblock/nativeperiod1; nonce is
separate. New historical admission needs explicit constructor origin and current1
selection, absentNODE1, exactgraph/auth/account authority. Do not silently admit
arbitraryperiods or inferhistory from request_id=None. Actualnewoutputs/root/rows
and per-vector effects are required before implementation/acceptance.

Run4 is active: savedbaseline89%,floor79%,reset1791948695;85% observed at
2026-10-07T23:33:08.937Z (4points used,6remain). Preserve this exact budget.
Runs1/2/3 are closed and must not resume. Generalreward/history shapes, N1–N6,
producer inputs, real-window/root parity and production cutover remain open.
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
