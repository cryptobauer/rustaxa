# Restart checkpoint: synthetic fixture hardening

## Current state

Checkout baseline: `5bfdf494c4ea84a3982903a4d46cb8113fc0d79b` on
`feat/rust/evm-state-db`. The N4 synthetic fixture-hardening slice is complete
and committed at this baseline. The working tree also has uncommitted
preparation edits; inspect status and preserve them.

Milestone 10 and plan 08 remain open. Milestone 09 is complete only for its
bounded synthetic four-period scope.

The [hardening report](n4_synthetic_fixture_hardening.md) and
[independent review](n4_synthetic_fixture_hardening_review.md) record a
successful synthetic Rust lifecycle and pinned Go run. Both engines validate
the full shared input contract and reject 306 drift cases. The Go runner and
Rust parity test require an empty observed raw-write list. Targeted Rust,
Go/Python, bridge and workspace fast checks passed. See the report for first-run
failures, corrections, logs and source freeze details.

This result covers only the bounded synthetic cold nonboundary case. It does
not qualify the historical producer or establish complete historical state,
real-window acceptance, publication, adoption, replay or root derivation.
Supplied `data/` and historical copies remain protected.

## Next startup

Read `AGENTS.md`, this checkpoint, and only the relevant plan and validation
sections for the selected task. Follow
[`doc/codex_slice_workflow.md`](../codex_slice_workflow.md). Check current
allowance with [`scripts/codex_quota.py`](../../scripts/codex_quota.py) at
startup and at the workflow milestones. The account-wide floor is 20% remaining
for all agent activity. If current telemetry remains unknown after one bounded
refresh, checkpoint before starting new work. Historical quota samples and
model-routing audits do not establish current allowance or routing.

The [remaining branch queue](n6_remaining_branch_queue.md) orders N1 contract
closure, paired N2/N3 coverage, qualified N4 replay, then N5/N6 acceptance.
Continue ready authorized work; keep external input blockers explicit.

The [completed prompt](n4_fixture_hardening_completed_prompt.md) preserves the
exact prior task. The root
[`next_executable_slice_prompt.md`](../../next_executable_slice_prompt.md) is the
active autonomous branch-run prompt. The [slice scorecard](../codex_slice_scorecard.md) records this
completed task and leaves three rows for comparable future slices.

The prior checkpoint is preserved byte-for-byte in the
[post-hardening history](n6_restart_checkpoint_history_2026_10_01_post_hardening.md).
