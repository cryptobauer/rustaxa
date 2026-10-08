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

Run4 accepts finite positive-reward P2 partial execution/frames, explicit-origin
same-H1 simulation and gas estimation, direct P2 public trace, and positive owner
commission trace. Accepted commits and exact evidence are in the compact checkpoint.
Canonical rewardP1 is measured and public FinalChain finalize/reopen works.
Literal H1 simulation/native period is1; trace executes2 over reader1 with an
unpublished pending session and unchanged wide nonce. Earlier nonce/context errors
and first failures are retained, not erased. General reward/history shapes stay open.

Next bounded ready profile: ClaimRewards(C,validator31) on the same canonical P1.
Actual Go direct trace is measured: gas61464/payout396/sixraw/one log; Rust tests,
required gates and fresh independent review remain pending. Existing claims adapter
is ready; no new seed or private graph advance is required. Follow the frozen
profile contract and quota in the run4 directory. Further claim frames/history and
sequences need their own measured witnesses; no production acceptance follows.

Run4 remains active with saved89% baseline/79%floor/reset1791948695. Latest
82% at2026-10-08T00:31:09.762Z means7points used,3remain; refresh before new work.
Runs1/2/3 remain closed. Producer inputs, qualified real-window/root parity,
recovery, integratedN4–N6 and production cutover remain open. Nothing was pushed.

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
