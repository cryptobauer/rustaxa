Continue authorized Milestone 10 work on this branch. Read AGENTS.md, the compact
restart checkpoint, `doc/codex_slice_workflow.md`, and
`doc/evm_research/n3_zero_redelegate_next_round.md`. Do not resume audited sessions.
Use gpt-6.1-sol medium for direct implementation and confirm actual runtime.

Before delegation, preparation commits or implementation, select a unique
persistent run directory, measure fresh weekly allowance and save its baseline
once. A newly authorized run may share the session UUID; give it a unique suffix
and preserve every completed run budget. Reuse the same budget during this run:

```sh
rtk proxy python3 scripts/codex_quota.py --budget-file "${RUN_ARTIFACT_DIR}/quota-budget.json" --init-budget
```

Use the same path without `--init-budget` at all later checkpoints. If
CODEX_THREAD_ID is unavailable, obtain the current session UUID, select that UUID
for the directory and use explicit --session. Set RUN_ARTIFACT_DIR under
/home/fry/artifacts with that UUID and a unique run suffix. Never guess from the newest log.
Stop when **10 percentage points of weekly allowance have been consumed since
startup**. Example: 80% remaining initially means stop at 70%, not 72%. This
replaces the fixed 80% reserve. The shared account drop counts all agent activity
and concurrent account use. Do not reset the baseline on resume or after a retry.
Reject stale/unknown telemetry, missing/invalid budget, identity mismatch or a
changed weekly window. One bounded telemetry refresh is permitted, then stop new
work if unresolved. At the limit finish only an in-flight atomic step and minimal
checkpoint; do not commit unvalidated changes or claim unchecked gates passed.

Validate and locally commit approved preparation docs/tooling and audit reports
as a separate Conventional Commit. Preserve unrelated work. This prompt authorizes
accepted local commits, not a push. Confirm Luna first for one bounded next-seam
check; reuse the accepted map rather than repeating branch discovery.

The zero API round and bounded one-/two-member full/new derivatives are accepted.
Continue from the current checkpoint, without rerunning completed slices. Resolve
the source-current same-height historical simulation/estimation authority before
implementation; continue other ready approved queue work when that gap is blocked. Use existing Rust owners and actual pinned Go
outputs. Preserve prior corpora and runtime guards. Continue other ready approved
queue work if capacity remains; do not invent work to consume quota. Run required
checks, retain first-run output/exit codes, freeze source/evidence, obtain
independent review, correct findings and commit each accepted slice.

Use independent Sol medium for settled derivatives. Do not ask Astra to repeat
settled contracts. Request Astra only for a named unresolved authority,
authentication, gas or rollback question. Start fresh bounded review contexts at
semantic profile changes; do not carry old partial/new/full/zero history into the
next profile. Check capacity and preserve Luna's slot; record any routing blocker.
Send complete assignments and result/blocker/correction messages without routine
status polling. Workers use fresh timestamped lead quota decisions and do not
start follow-ups independently. The lead checks before starts, after closeout
milestones and about every five minutes; reuse adjacent fresh observations.

Keep required parity, package, bridge and serial fast checks. No push, production
routing, fallback, protocol change, supplied-data mutation or upstream C++ exception.
Prepare commands, data identities and bounds before approval-required broad gates;
continue independent approved work while awaiting a decision. Do not repeat missing
producer-fact requests or invent facts. Stop at the run budget, authorized work
completion or a blocker affecting all ready work. Record baseline, target, final
allowance, observed consumption and reset identity separately from tokens/billing.
Leave a compact checkpoint, accepted commits, checks, review and remaining limits.
