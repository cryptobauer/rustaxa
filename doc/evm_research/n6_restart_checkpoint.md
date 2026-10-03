# Restart checkpoint: staged redelegation next

Branch: `feat/rust/evm-state-db`. Use `git log -1` for the latest accepted commit.
Milestone 10 and N1–N6 remain open. No push or production routing is authorized.

The previous run completed metadata and escrow execution/API slices plus
redelegation observations, account-port wiring, kernel/write composition and
seven normal preflight failures. Targeted checks, required bridge checks and
workspace fast checks passed; independent reviews accepted their bounded scopes.
Details: [completed checkpoint](n6_restart_checkpoint_history_2026_10_02_post_branch_run.md),
[scorecard](../codex_slice_scorecard.md), [usage audit](../../token_usage_audit_01a0f999.md).
First-run logs and frozen evidence: `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

The authenticated staged partial redelegation adapter is validated under the
[settled contract and acceptance record](n2_redelegate_observations.md).
Next: selector-first redelegation ABI/admission, then actual frame and historical
API composition through existing Rust owners. Keep unsupported success branches
explicit. The Luna frame map is complete; do not repeat source discovery.
Current run artifacts: `/home/fry/artifacts/evm-redelegate-2026-10-03/`.
Direct lead runtime: Sol medium. Luna map and independent preparation Sol review
ran; fresh Astra medium adapter review found no blocking issue. Routing succeeded.
All required adapter checks pass; first-run failures and serial fast retry are
retained. The preparation commit is `5d4da5bc9`; record the adapter commit with
`git log -1` after closeout.

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
