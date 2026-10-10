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

Accepted local source HEAD is `c07d5495a`. The shared owner snapshot helper,
underfunded P1 boundaries/runtime/H1 estimates, H1/P0 funded and gas trace,
P2/P1 gas trace and value1 nonpayable oracle/runtime are closed finite scopes.
Use [the current checkpoint](n6_restart_checkpoint.md) and named run closeouts;
do not repeat those slices or infer general profile/root acceptance.

The same-profile H1 initial-cap nonpayable estimate derivative is accepted at
`c07d5495a` after all11 frozen gates and independent Sol ACCEPT. Public EVM81,
serial fast507.46s and exact retained consensus1538/ON12all15 are recorded.
Its unchanged C++ search consumes actual accepted Go initial responses;
each cap stops at one failed probe, with no midpoint or successful estimate.

Then update and independently review the compact checkpoint/scorecard facts in
a separate docs commit. Preserve user-owned workflow, prompt, skills and audits.
Use the current run's immutable baseline68/floor58 and fresh telemetry; do not
start a new gate or delegation at the ten-point limit.

The next uncovered P0 ordinary-prefix claim-all trace is a separate profile.
Name its source/context and first-native transaction binding, settle the source
contract, confirm fresh reviewer capacity and measure both pins before code.
Preserve exact178728 and empty-prefix gas origins. Do not remove guards or infer
prefix effects from source predictions. Real producer facts remain a separate
N4 dependency; do not repeat requests or invent missing inputs.

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
