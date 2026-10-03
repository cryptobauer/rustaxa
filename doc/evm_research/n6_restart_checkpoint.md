# Restart checkpoint: staged redelegation next

Branch: `feat/rust/evm-state-db`. Latest accepted implementation: `327d15fa1`.
Use `git log -1` to record the new preparation baseline at startup.
Milestone 10 and N1–N6 remain open. No push or production routing is authorized.

The previous run completed metadata and escrow execution/API slices plus
redelegation observations, account-port wiring, kernel/write composition and
seven normal preflight failures. Targeted checks, required bridge checks and
workspace fast checks passed; independent reviews accepted their bounded scopes.
Details: [completed checkpoint](n6_restart_checkpoint_history_2026_10_02_post_branch_run.md),
[scorecard](../codex_slice_scorecard.md), [usage audit](../../token_usage_audit_01a0f999.md).
First-run logs and frozen evidence: `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

Staged redelegation remains unsupported. Implement the
[next chunk](n2_redelegate_next_chunk.md) under its linked settled adapter
contract, then add actual frame/API composition. Use existing Rust owners,
serializers and pinned fixtures. Direct Sol medium implementation; Luna bounded
map; independent Sol for settled derivatives and a fresh Astra review context
for authentication/rollback or unresolved semantics.

The active [prompt](../../next_executable_slice_prompt.md) and
[workflow](../codex_slice_workflow.md) now use **80% allowance remaining**.
Read fresh current-session telemetry before starting. The previous run's 21%
remaining is historical. If fresh allowance is at or below 80%, stop new work.
Unknown telemetry after one bounded refresh also stops new work.

Producer executable identity, runtime overrides and capture command remain
unknown. Do not invent them or repeat the request. N4 still needs qualified
complete inputs and real signed-period/root parity. Synthetic checks do not
close real-history or production acceptance. Broad replay/differential/fault
gates require prepared commands, data identities, bounds and required approval.
