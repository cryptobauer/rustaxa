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

See [checkpoint](n4_next_run_checkpoint.md):GEN29 passed61in actual generation,
61constructors completed,failed60. GEN30full60positive accepted with3exact prior
scopes;144required reads,review1,583,203under2MiB. Current190profile authenticated
for offline fixture only,not normal caller/fit acceptance. GEN30producer not run.

Stop:development time. NextB4,020seconds versus1,905.091remaining;oldSol expired.
B/C remain unconsumed352Mreads/64MiBretention/3072units. Old87-to67 closed76%,9unused.
[Prompt](../../next_executable_slice_prompt.md) proposes NEW20points,no pooling,
DEVELOPMENT32MiBretention/192MiBreads/43,200seconds/12,288units. User sends authority;
documents activate none. Reuse existing holds after full current admission.

Next:reuse accepted fixes,preflight actual profile/source/helper consumer pins and
exact pre-role60fixture state,coherent new bindings,full B/C/actor/failure/closeout
admission,long reviews first,fresh final pins last,GEN30execution/funded C.
No artifact,0/16fits,805/109unchanged;public guard/cold bridge open.
Preserve charges/UNKNOWNs/violations and hard limits. Pruning deferred;
no37/32capture/push/production/audit/prompt work.

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
