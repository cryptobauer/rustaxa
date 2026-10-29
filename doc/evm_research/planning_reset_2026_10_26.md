# EVM planning reset

Prepared 2026-10-26 at the owner's request. This is a planning package, not an
implementation authorization or a replacement for an active contract. The old
implementation session was still active at preparation. Its final checkpoint must
replace the provisional facts below before a new implementer starts.

## Objective and diagnosis

Deliver a compatible Rust execution/state backend through the existing Rust
FinalChain owners. Judge progress by accepted behavior: reference-equivalent
execution, receipts, roots, APIs, persistence and recovery. Production cutover is
separate. Pruning remains deferred under [the scope update](n6_non_pruning_closeout_scope.md).

Keep the chosen architecture. The [bounded mixed-period milestone](09_mixed_period_milestone.md)
provides prior integration evidence. The [existing-network milestone](10_existing_network_milestone.md)
defines the open acceptance gates. Do not restart engine selection or discard
accepted implementations because recent qualification work has been slow.

The two sessions were a serial implementer (`01a14b66-bb49-7e22-8a8c-1cb04c03816e`)
and a review/prompt author (`01a0f50e-44cd-7a32-b1e1-553e0303c8e1`). The latter
repeatedly diagnosed low throughput but issued another prompt with substantially
the same development admission machinery. A new planner must be able to reject
that approach and propose a smaller, concrete successor.

Provisional baseline: [source190 review](history/session_reports/session_review_01a14b66_source190.md)
reports 61/81 constructors, no complete artifact, 0/16 fit routes and 805 accepted /
109 outstanding VRF facts. GEN30's local role60 correction was accepted; its full
producer had not run. The next round began on October 26. These are historical
qualification facts, not current results or percentages of EVM completion.

The [source183 audit](history/session_reports/token_usage_audit_01a14b66_source183.md) reports about
26.9 million tokens in one completed round, with review usage close to lead usage.
It does not measure this preparation or subsequent rounds. No new billing or full
token audit is needed to establish that repeated preparation has low throughput.

## Final handoff and workspace ownership

The owner will stop the old implementer. Do not send it messages, terminate its
processes, or assume it stopped because a checkpoint file exists.

Before new implementation, obtain its final checkpoint and confirm that its writers
and relevant child processes have stopped. While it is active, the planner can
read records and draft isolated documents; it must not edit active shared controls.

Record one compact handoff with:

- Final checkpoint path, timestamp, source commit and branch.
- Accepted scopes and exact evidence references; pending checks and first failure.
- Tracked diff and untracked source/artifact inventory, with sizes and hashes for
  the pending files that the next task needs. A commit alone does not preserve
  untracked code or generators stored under `/home/fry/artifacts`.
- Any live or uncertain operation, physical capture marker, reservation and
  unresolved failed-run record. Do not infer unused capture authority from a stop.
- Old budget identity and final observation as history, not a new session budget.
- Next executable dependency and its owning paths.

Do not reset, clean, stash, prune worktrees, or move work automatically. Preserve
the existing dirty checkout. Use `/home/fry/.codex/worktrees` if isolation is
needed, and explicitly transfer required pending files; a worktree from HEAD
does not include them. Keep manifests and recovery copies under
`/home/fry/artifacts` or the persistent checkout. Never read `AI.md`.

## Proposed process changes

These are concrete recommendations for the planner to refine against the final
state. They are not active waivers. Obtain one decision on the exact successor
after the proposal and required independent review are ready. Do not ask the owner
to approve an abstract plan before preparing the actual changes.

| Control | Proposed disposition | Required condition |
| --- | --- | --- |
| Go parity, exact state identity, missing-history errors, rollback and recovery | Keep | Preserve every applicable acceptance test and oracle boundary |
| Supplied data protection, physical capture attempts and runtime resource limits | Keep | No new capture, wider read authority or weaker runtime bound is implied |
| Complete command output, exit status and immutable first failures | Keep | Use the existing capture helper; retain unknowns without inventing results |
| Development-only cumulative read/unit/retention banks and manual actor-time transfers | Replace where separable | Propose one task resource envelope with measured process limits, finite output/disk bounds and user allowance; prove no runtime/capture consumer loses protection |
| Expiring review/launch bindings | Narrow | Keep genuine input/freshness checks; move slow review before final checks and derive related bindings from one source |
| Repeated semantic review of unchanged records | Reuse | Same source, input and accepted scope; independent review covers the changed assumption or instance |
| Historical failures in current review bodies | Link instead of copying where permitted | Preserve original bodies and all required semantic occurrences; review any format successor |
| Mandatory helper startup for every serial task | Make task-dependent in the successor policy | Explicitly amend the project rule before changing it; use a helper only for an independent deliverable |
| New round after every ordinary correction | Remove as a default pattern | Continue within the authorized task and budget; stop for a real bound, missing input, unsafe state or changed scope |

Do not make a second general-purpose accounting framework to implement this table.
Prefer a small change to existing ownership, configuration and record formats.
Do not convert old nonconforming outcomes into passes. Preserve old ledgers as
history; define an explicit transition for any unresolved live holds. Retiring a
development restriction is not a refund or a grant of physical capture authority.

The planner must name each changed rule, its enforcing document/code consumer,
the replacement, preserved invariant, required test and approval status. If a
restriction cannot be separated safely, keep it and state why. Do not silently
treat every restriction as immutable merely because it appears in an old prompt.

Initial source locations for that bounded inspection:

- `experiments/evm_feasibility/snapshot_qualifier/tools/pa1_development.py`:
  development reservations and their dimensions.
- `experiments/evm_feasibility/snapshot_qualifier/tools/pa1_fixed40_runtime.py`:
  `RuntimeAdmission`, development checks and runtime consumers.
- `experiments/evm_feasibility/snapshot_qualifier/tools/pa1_fixed40.py`:
  controller state, complete constructor/read paths and profile bindings.
- `doc/codex_slice_workflow.md`, `AGENTS.md`, and the active task contract:
  procedure, authority and validation requirements.

These are inspection entry points, not a claim that all constraints are
development-only. The 2 MiB guard and capture/read limits must not be raised just
to make an existing failure pass.

## Evaluate the skill instead of inheriting it as policy

Read `.agents/skills/codex-efficient-work/SKILL.md` as an object of this review.
Keep compact handoffs, scoped evidence reuse, direct serial implementation,
independent review, meaningful checks and user-requested allowance stops.
Its detailed ledgers are expressly optional project policy. The project rules
and prompts made them mandatory here; changing the skill alone will not fix that.

Prepare exact proposed edits to the project workflow and, only where necessary,
the skill. Do not activate conflicting instructions while the old implementer is
running. Separate accepted policy from proposed policy in the output.

Add one decision rule to the successor workflow: after a round fails to close its
named gate, classify the cause as implementation, input, tooling, process or
resource. Repeated tooling/process failure requires an approach decision before
another equivalent prompt. This is a planning review point, not an arbitrary
retry counter imposed on the implementer's in-scope corrections.

## Select the next gate from the final state

Compare three paths once, using actual available inputs and named consumers:

1. Finish the current fixed40 path using accepted corrections. Estimate all
   remaining artifact, guard, fit, review and capture prerequisites; constructor
   completion alone is not its exit. Prefer this if the final run is near a
   complete accepted result without another large tooling expansion.
2. Make the minimum development-control correction above, then finish that path.
   Choose this if the same admission/evidence mechanism still prevents execution.
3. Advance one independent native/API acceptance gap while requesting a precise
   reference export for N4. Identify the exact missing fields, source revision,
   state/period identity and producer provenance. Do not assume a better export
   exists or that synthetic data can replace the real-window gate.

Recommend one primary path and, only if useful and independent, one secondary
task with disjoint ownership. Do not create a new broad inventory or fabricate
an overall completion percentage. Use the [remaining queue](n6_remaining_branch_queue.md)
and nearby implementation to verify a concrete N2/N3 candidate before naming it.

The longer-term integration target is a qualified real signed-period execution
with reference-equivalent receipts, outputs and roots, plus its required API and
recovery evidence. Finishing VRF inputs does not close other native, reward,
configuration, producer or state-adoption gaps.

## Deliverables of the new planning session

Produce one concise current plan and one first-implementer prompt using
[the template](implementer_prompt_template.md). The plan must contain:

1. Verified stop handoff and current accepted baseline.
2. Exact process successor proposal, including affected rules/code and the small
   set of decisions actually needed from the owner. A patch may be staged as an
   artifact; it must not silently become active policy.
3. Chosen next gate, why it is on the critical path, scope and dependency order.
4. Exact validation commands derived from existing targets and the changed paths.
5. A separate draft implementer prompt, clearly marked ready or blocked and why.

For a code-enforced policy change, the first prompt can implement and validate the
approved successor as its own finite deliverable. It must not also claim N4
acceptance. If no policy change is needed for the chosen slice, generate an
executable prompt without waiting for unrelated process work.

Do not invent a new implementation allowance or initialize one in the planning
session. The owner chooses the new implementer's allowance. A new session uses
its own fresh baseline; historical budgets and unused amounts do not transfer.
Planning usage and implementation usage must remain distinguishable.

Keep the startup reading set short: this package, the final checkpoint, the
selected contract, applicable instructions and relevant source. Consult original
evidence only for an unresolved claim. Do not load both complete session logs or
all prior reviews as default startup work.

## Validation and reporting

Use [the repository validation strategy](../rewrite_validation_strategy.md).
For Rust changes include the required fast gate and hook. For every storage change
include `cmake --build /build --target rust_storage_tests --parallel 12` and
`/build/bin/rust_storage_tests`. Select the affected subsystem/parity tests and
confirm Rust mode where required. Existing required tests cannot be weakened.
Broad differential/fault gates need existing authority or a concrete request.

Each task reports: acceptance gate before/after, changed behavior, checks and
failures, retained evidence, unresolved limits, actual allowance observation if
bounded, and next dependency. Record tooling progress separately from backend
acceptance. Do not run a full token audit after every slice.

No source, active prompt, budget, runtime controller or existing evidence was
changed by this preparation. The preparation itself requires document/link and
whitespace checks, not EVM or storage test runs.
