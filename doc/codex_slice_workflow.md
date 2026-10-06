# Bounded slice workflow

Use this workflow for remaining EVM/state branch work. Review depth follows
semantic risk; the cheapest route is not mandatory. The current
[model table](evm_research/08_implementation_plan.md#agent-and-model-assignments)
and repository validation rules remain authoritative.

1. Inspect status and baseline once. Read `AGENTS.md`, the compact
   [checkpoint](evm_research/n6_restart_checkpoint.md), the selected task contract,
   and relevant source ranges. Read historical evidence only when a decision needs it.
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
   Follow all required package, parity, bridge and fast gates. Rerun after changes,
   failures or a concrete unresolved concern, not merely to obtain another log.
6. Freeze source, outputs and checks. An independent Sol-medium reviewer checks
   settled work; escalate unresolved authority or difficult semantics to Astra.
   Review summaries, hashes and relevant ranges without dumping full large artifacts.
   Reuse a review thread within one settled profile. At a profile/family change, start
   a fresh explicitly configured reviewer with `fork_turns="none"`; give it only
   the settled contract, source/configuration pins, current delta and evidence
   paths. Do not carry metadata, escrow and redelegation history into one Astra
   thread. Check capacity first and preserve Luna's slot. If a new thread cannot
   start, record the blocker and complete other authorized work; do not retry an
   unchanged failure. Use Sol for settled derivatives. Use Astra for new
   authentication, gas, rollback, trace or historical authority decisions.
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

The [latest audit](../token_usage_audit_01a0ff3b.md) supports these review
changes: Astra still used 66.8% of estimated credits and grew to about 238K input
tokens before compaction within one family. Use focused profile contexts,
independent Sol for settled derivatives, complete assignments and correction
handoffs without routine status polling. This is local evidence, not proof of
model equivalence.
The official [OpenAI context guidance](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra)
also recommends removing stale instructions and loading focused guidance.

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
