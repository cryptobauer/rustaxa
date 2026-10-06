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

Source-first/source-last current-node and block-one source-last both-absent
runtime/frames/signed H1 APIs are accepted. Bounded full/existing BOTH-current
runtime `072fc39fb` and publicframes `7531d4dc2` are accepted: sourcecount2 restored,
destinationcount3 retained, exactly two caller pairs ordered[source,destination].
The fresh signed-H1 authority contract is settled; newactualcanonicalpartial300
toexisting32 preservesbothnode1count2 atH1. Its oracle preparation measured105rows/
roote4781dc8.../101912gas/effective2 withcommittedH1unchanged; oracle preparation is accepted indd0c2f458;
closeout is recorded in the checkpoint. Next publicRust canonicalfinalize/reopen/session
proof must use that new physical identity, caller3000 AND ordinaryowner1000,
native4000, preservebothnodes2 and authenticate every newraw key/fault/retry.
No publicRust H1test exists for this profile yet. Do not reuse oldthree-validator
roots or claim H2/production/history acceptance from staged/frame/Go fixtures.
The order-only existing-destination source-last H2 candidate is deferred because
accepted single_full_last_item covers the branch. Preservepriorcorpora/support/
configurations and runtimeguards. The checkpoint records the closed56->46 budget;
start furtherimplementation only under a new authorized budget. N1–N6 remainopen.

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
