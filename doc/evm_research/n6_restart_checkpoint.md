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
The adapter commit is `93c4c2917`. Selector-first ABI/admission and actual frame
composition are also validated and independently accepted; see the
[frame record](n2_redelegate_frames.md). Use `git log -1` for their local commit.
Actual historical DryRunner is also tested over reopened physical readers and
reconstructed semantic owners; see [simulation record](n2_redelegate_simulation.md).
Estimation is also tested through actual Go probes and the unchanged C++ search;
see [estimation record](n3_redelegate_estimation.md).
Direct structured traces are also tested over actual Go live sequences; see
[trace record](n3_direct_redelegate_traces.md). The prepared bounded chunk is
complete and independently accepted. The new-destination partial adapter is
also tested against actual Go first/repeat writes and membership; see
[new destination record](n2_redelegate_new_destination.md).
New-destination actual eight-case frame composition is validated; see
[new destination frames](n2_redelegate_new_destination_frames.md). Frozen corrected review
accepted all14 files. Complete-seed historical simulation is independently accepted; see
[new destination simulation](n3_redelegate_new_destination_simulation.md).
Next: estimation and direct supported traces.
Keep excluded success branches explicit.
Current run artifacts: `/home/fry/artifacts/evm-redelegate-2026-10-03/`.
Sol medium leads directly; Luna medium maps inputs; fresh Astra medium reviews
the redelegation family. All routes ran, with no routing failures. Targeted,
actual pinned Go, ON bridge and serial workspace fast checks pass. First-run
failures remain recorded. The preparation commit is `5d4da5bc9`.
Latest lead allowance: 88% remaining at 2026-10-03T02:24:04.926Z; refresh before
new starts.

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
