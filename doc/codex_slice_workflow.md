> Current run: the owner-approved [Rust EVM cutover addendum](evm_research/n6_rust_cutover_task_policy.md) supersedes conflicting scope and process rules below for root run 01a17dec. Historical run records remain unchanged.

# Bounded slice workflow

For the owner-approved orchestrated current-era and Cacti closeout only, the
[task policy](evm_research/n6_retained_commission_task_policy.md) replaces conflicting startup, allowance and
development-bookkeeping steps below. Independent review and all parity,
validation, source-data and runtime protections remain required. Fixed40
contracts are unchanged. No exception applies without owner approval.

Use this workflow for remaining EVM/state branch work. Review depth follows
semantic risk; the cheapest route is not mandatory. The current
[model table](evm_research/08_implementation_plan.md#agent-and-model-assignments)
and repository validation rules remain authoritative.

1. Inspect status and baseline once. Read `AGENTS.md`, the compact
   [checkpoint](evm_research/n6_restart_checkpoint.md), the selected task contract,
   and relevant source ranges. Read historical evidence only when a decision needs it.
   Final run artifacts supersede interim checkpoint facts. Select relevant scorecard
   rows and diagnostics; keep full outputs in artifacts. Inspect fixture keys, counts
   and hashes instead of large JSON dumps.
2. Check the current account allowance from the current session log:

   ```sh
   rtk proxy python3 scripts/codex_quota.py --budget-file "$run_budget" --init-budget
   ```

   The reader uses `CODEX_THREAD_ID` when available. Otherwise supply
   `--session <current-session-uuid>` or `--log <current-rollout-path>`.
   Do not select the latest log or reuse a completed session's observation.
   It reads one log without modifying it and prints allowance separately from
   tokens and billing. Set `run_budget` to a unique file under the persistent
   run artifact directory. Initialize it once, before delegation and preparation
   commits; thereafter use the same command without `--init-budget`.
   Save the fresh starting allowance and stop after **10 percentage points**
   have been consumed. Starting at 80% remaining means stopping at 70%, not 72%.
   Use one lead-session budget for all agents; no separate agent/model budgets.
   Do not overwrite or rebase it on resume. A missing or malformed budget,
   session mismatch, changed weekly reset or decreased cumulative used value
   cannot permit a new start. Record exact baseline and final observations.
   The lead checks at startup, before spawning, before each new slice or large
   gate, after validation/review/commit, and at natural milestones no more than
   about five minutes apart during long work. Combine adjacent checks while the
   same observation is fresh. Workers use the lead's timestamped decision for
   their bounded assignment, report completion or blockers, and do not start
   follow-up work without a fresh lead decision. Use a current snapshot no older
   than five minutes for a new slice or gate. If telemetry is unknown or stale,
   make one bounded refresh or session-identity retry. If it remains unknown,
   stop new work and write a durable checkpoint; do not consume allowance
   blindly. A rounded or delayed value cannot guarantee an exact floor.
   When 10 percentage points have been consumed, or allowance is exhausted,
   start no new slice, delegation or large gate.
   Finish only an in-flight atomic step, then checkpoint and hand off. Do not
   commit an unvalidated slice or claim unchecked gates passed. Just above the
   floor, prefer a small bounded step that leaves closeout capacity; do not add
   another start threshold.
3. Confirm Luna's bounded startup check. Sol medium implements a serial task
   directly; add another implementation agent only for an independent scope.
   Use one writer per module. Report milestones or blockers instead of polling.
4. Edit named code spans. After changing embedded JSON, parse it before expensive
   execution. Avoid replacements that cross code and fixture literals. Run the
   smallest relevant compile/contract check before full lifecycle checks.
5. Capture command, complete output and exit code on the first validation run.
   Retain full failure bodies once in immutable artifacts. Routine handoffs use
   exact path/size/hash, affected invariant, correction and final test result.
   Do not recursively copy logs or old reviews into later records. Existing
   embedded-body contracts require a reviewed representation successor before
   replacement; never omit required evidence or silently change charged counts.
   Precheck metadata fit, source pins and builder schema before expensive gates.
   Check exact test selectors and require a nonzero matched count. Check request
   bindings, expected read order and fixture validity before full gates. Complete
   owner-database preservation requires every column and logical row, including
   empty columns, correct comparators and WAL-visible state; selected accounts or
   the separate Go snapshot do not prove that claim. Prefer a tested shared helper.
   Follow all required package, parity, bridge and fast gates. Rerun after changes,
   failures or a concrete unresolved concern, not merely to obtain another log.
6. Freeze source, outputs and completed required checks. Request final review after
   gates finish; an early bounded source pass can finish separately with one later
   evidence handoff. An independent Sol-medium reviewer checks
   settled work; escalate unresolved authority or difficult semantics to Astra.
   Review summaries, hashes and relevant ranges without dumping full large artifacts.
   Reuse a review thread within one settled semantic family. A fixed manifest instance
   inside an accepted common contract is not automatically a new semantic family.
   At a semantic family change, start
   a fresh explicitly configured reviewer with `fork_turns="none"`; give it only
   the settled contract, source/configuration pins, current delta and evidence
   paths. Do not carry metadata, escrow and redelegation history into one Astra
   thread. Check capacity first and preserve Luna's slot. If a new thread cannot
   start, record the blocker and complete other authorized work; do not retry an
   unchanged failure. Use Sol for settled derivatives. Use Astra for new
   authentication, gas, rollback, trace or historical authority decisions.
   If capacity prevents fresh review at a new profile, checkpoint before dependent
   implementation. Record listed status separately from an actual spawn rejection.
   Send new assignments to an idle worker with followup_task; send_message steers
   an active worker and does not by itself start idle work.
   A new root needs explicit new-round authorization and its own baseline; never
   transfer or reset the old session-bound budget to bypass a capacity stop.
7. Record corrections, accepted scope and actual routing in the
   [scorecard](codex_slice_scorecard.md). Commit locally only when the task authorizes
   it; preserve unrelated work. Close completed slice contracts; keep the branch-run
   prompt active across slices. Replace checkpoint facts with the new state.
   Keep full handoffs as linked history instead of appending them
   to startup instructions.

These steps reduce repeated context and correction work. They do not reduce
required validation or turn synthetic results into historical/production acceptance.

The relative 10-point run budget replaces the previous fixed 80% reserve.
Always use `--budget-file` for this run. The reader's legacy bare-command floor
and `--min-remaining-percent` mode are not this run's budget; do not combine them.
If less than 10 points remain at startup, available allowance is the upper bound.
Account-wide use by other sessions also counts toward the observed drop. Rounded
or delayed telemetry cannot ensure an exact stop. A weekly reset invalidates the
saved window; checkpoint instead of creating a larger budget automatically.

The [recent audit](evm_research/history/session_reports/token_usage_audit_01a1470a.md) found poor accepted-result
throughput during repeated evidence-policy corrections. The next plan uses one
complete operation/resource preflight, compact current checkpoints and source-backed
fit before expensive gates. Reuse settled family contracts and exact scoped evidence;
Astra reviews only named changed assumptions, while Sol reviews exact instance,
source, command and proof deltas. Full pins, one-use guards, replay and acceptance
remain mandatory. Measure accepted behavior and distinguish one-time infrastructure
work from repeated execution cost. Cached input and short replies alone do not prove
efficiency.

## Autonomous execution

Work through the [remaining branch queue](evm_research/n6_remaining_branch_queue.md)
within the authorized milestone. Before each slice, name its inputs, owned files,
acceptance checks and excluded behavior. Finish implementation, corrections,
required validation, frozen independent review, and the authorized local commit.
Then update current checkpoint facts and continue to the next ready dependency.
Do not ask again for routine edits, targeted checks or local actions already
authorized by the task. Do not stop after each helper result.

Resolve uncertain contracts before dependent implementation. Ask for missing
external inputs while progressing independent authorized work. After repeated
identical failures, change the approach or obtain bounded diagnostic review;
do not repeat an unchanged retry. Leave failed or skipped gates explicit.

## Complete command evidence

All evidence-producing developer and reviewer commands use `scripts/capture_command.py`
with a new persistent output directory and admitted time/output bounds. Handoffs name
this execution path and require a real nonzero/stderr demonstration. A tested helper
that workers bypass gives no capture. Keep actual stdout/stderr/child exit distinct
from runner errors;never create literal empty stderr or exit0as command evidence.

Timeout,overflow,spawn/runnerfailure or missing status is not a complete pass. Output
cap retains a prefix,not a bound on a lost stream. stdin is closed by default;use a
retained script argument or explicit --stdin-file,not a wrapper here-document.
Require expected result/test counts. Bounded previews do not replace originals.
This runner is not a sandbox for deliberately detached processes. Use non-daemonizing
development commands only;native physical capture authority remains unchanged.

For serial correction work, the lead runs validation commands. Luna can check
bounded metadata or fixture structure without starting the validator. A delegated
command must name the exact wrapper, argv, capture path, child output paths and
one admitted occurrence. Check that all new output paths are unused before launch.
Do not retry on a collision, missing capture or runner error under the same hold.
Inspect the original status first: a child can start and import modules before
the test suite starts. Retain its charges even when zero tests ran. A new occurrence
needs complete prospective admission; a captured retry cannot repair lost streams.

When a review thread has accumulated large context, use a fresh independently
configured Sol reviewer at the next frozen delta. Supply the current accepted
contract, pins, delta, required checks and open obligations; link older evidence.
This does not waive required review or authorize another agent beyond capacity.

## Development recovery admission

Plan the complete remaining dependency chain once: implementation,source snapshots,
native input admission,failed/repeated outcomes,validation,independent review and
closeout. Use one reviewed finite operation/resource plan where the active contract
permits it. Keep per-operation admission and actual charging; an aggregate plan
does not waive either. Existing contracts bind until an authorized independently
reviewed successor replaces them. Amend that plan for a concrete changed assumption,
not as a routine permission exchange before each shell command. Preserve every
consumed charge,hold,UNKNOWN and immutable original; do not refund or reset history.

At a suitable independent review boundary, give a fresh reviewer the exact current
contract,source delta,pins,open obligations and relevant results. Link historical
evidence instead of inheriting the whole development conversation. Include every
required proof and read bound. Reuse that reviewer for its bounded family. Check
capacity and actual startup; fresh context grants no resource or quota authority.

For bounded development, reserve each operation's full outcome and closeout before
starting it. Include all in-flight tools and reviewers; a correction cannot consume
another pending operation's failure capacity. Use durable operation identity and
atomic exactly-once completion. Reconcile unknown/interrupted state before another
start. Keep observed failures even when they reveal an admission breach.

Use the accepted finite resource manifest and user quota for recovery admission;
do not invent a small correction/version count as a second autonomous-work stop.
Existing contract counts still bind until an explicitly authorized reviewed successor
replaces them. Development retries never grant another physical capture attempt.
Preserve full evidence, counts, read/time/allocation limits and independent acceptance.

A timeout is a failed gate. Diagnose its exact test and resource scope before retry;
never infer cause from elapsed time or accept completed subset lines as full success.
Bind each check to retained exact pre-run source, full output/exit and test roster.

## Defects and interruptions

Follow the skill's [recovery procedure](../.agents/skills/codex-efficient-work/references/defect-recovery.md)
when a concrete defect is found or work is interrupted. A platform block alone
does not establish a vulnerability. Record suspected findings locally, fix the
cause with a bounded regression test, and retain that test in the normal suite.
Require relevant parity checks and independent review before marking verified.
Continue permitted independent work; do not accept behavior that depends on an
unresolved invariant. Keep user updates to status, fix and validation results.

Save checkpoints before long gates and at acceptance boundaries. After a forced
interruption, inspect the live delta and command outputs on the next authorized
turn. Do not assume completion or bypass the block. Same-session resume preserves
its saved budget; a fresh session follows new-round authorization rules.

## Authorization boundaries

Ask only when required authority is missing: expensive broad/differential/fault
gates, production routing, legacy fallback, upstream C++ exceptions, or supplied
data changes. Prepare exact commands, datasets and bounds first. Push only when
explicitly authorized. A queue item is not authority to widen the milestone.
At the account-wide quota floor, stop new work and delegation. Finish only the
current atomic step, then retain a resumable checkpoint with next action,
relevant paths, input identities, validation state and remaining checks. If a
slice is blocked by missing authority or an external input, record the exact
blocker and continue other ready, approved queue work. Do not add work only to
consume allowance.
