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

The fresh run closed the pending malformed-claim slice first, then commission,
delegator and mixed claim sequences, positive claim CALL/STATICCALL/revert
frames, read-free low-gas/nonpayable guards and repeated two-call frames.
Exact accepted commits and gates are in the [checkpoint](n6_restart_checkpoint.md)
and its run directory. Old run4 is closed; its interim pending facts are stale.

Positive noarg claim-all oracle preparation and runtime/public parity are
accepted separately at3ded1a88a590/546ac3eeb129. The runtime closeout has all11
fresh gates and independent frozen acceptance. It preserves count-only gas
preparation, ordered two-item traversal, shared private scalar views, twelve
writes and one396payout. Public parity covers168ports across fresh/reopened
owners. Broader claim-all failure/topology/call profiles remain excluded.

The next settled derivative is a direct public API trace over canonicalP1 with
empty/ordinary self-transfer prefixes and original wide nonce. Luna's bounded
`claim-all-next-trace-map.md` identifies the existing producer, trace runner and
public owner. Confirm actual dual-pin measurement before implementation. A
prior native prefix, historical period1, nested/static or repeated claim-all
requires separate scope/source authority and new evidence. No new producer
input is missing for the selected synthetic derivative.

Fresh budget77%,floor67%,reset1791948695 is immutable. Latest interim72% means
5points used/5left; refresh the same run budget before further work. Run final
records supersede interim observations. Stop new work at the floor; checkpoint
pending bytes and gates without claiming acceptance. No push.

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
