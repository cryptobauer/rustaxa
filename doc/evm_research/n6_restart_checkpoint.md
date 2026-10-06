# Restart checkpoint: zero-amount direct traces in progress

Branch: `feat/rust/evm-state-db`. Accepted runtime/frame tip: `752629e9c`;
previous run closeout: `049ec5e73`. Preparation commit: `5a74db354`; accepted zero simulation: `1c5de6f75`.
No push, fallback or production routing. Milestone 10 and N1–N6 stay open.

Partial existing/new caller destination and full caller-source removal with a
retained validator have accepted adapter, frame, historical simulation, estimation
and direct trace evidence. Pre-Aspen-two zero amount with positive existing caller
pairs has accepted runtime and frame evidence; historical simulation is accepted; estimation is accepted and direct traces are in progress.
Details: [completed checkpoint](n6_restart_checkpoint_history_2026_10_03_post_redelegation.md),
[scorecard](../codex_slice_scorecard.md), [audit](../../token_usage_audit_01a0ff3b.md).
Artifacts: `/home/fry/artifacts/evm-redelegate-2026-10-03/`.

Current run: lead session `01a10f48-83ee-7a32-8bff-722d10c36359`, actual Sol medium.
Immutable quota budget:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/quota-budget.json`.
Baseline `2026-10-06T03:36:59.581Z`: 34% used / 66% remaining; target 56%.
Weekly reset `1791584594`. Latest observation 65% at `2026-10-06T03:47:26.190Z`.
Reuse the saved baseline on resume; do not initialize it again.

[Zero historical simulation](n3_zero_redelegate_simulation.md) passed actual dual-pin
reproduction, package check/tests/Clippy and serial fast; frozen independent Sol
review accepted. The unchanged runtime ON bridge evidence is retained. Extra
strict Clippy failed on existing consensus warnings; required Clippy passed.
Luna medium and both Sol review routes were confirmed. Starting worker telemetry
used the lead budget in error; own-session runtime checking corrected it.

[Estimation](n3_zero_redelegate_estimation.md) passed six actual dual-pin probes,
unchanged C++ search (104853), 48 Rust sessions, package gates and serial fast.
Frozen independent Sol review accepted without corrections. Commit is recorded in
Git history and scorecard. Direct trace source/oracle work has started in disjoint
sibling files; registration was held until estimation acceptance. Four sequences
cover single zero, partial prefix then zero, repeated zero and stale nonce. Targeted
checks, package gates, frozen review and commit remain pending. Keep actual H2 over
H1 and supplied increasing nonces; no DryRunner nonce substitution in traces.

Artifacts and first-run logs are in the current run directory. Serial fast uses
`RUST_TEST_THREADS=1` for existing temporary database locks. No push, production
route, broad gate or supplied-data mutation is authorized.

Producer executable identity, runtime overrides and capture command remain unknown.
Do not invent facts or repeat requests. Qualified real windows, signed-period/root
parity and N4–N6 remain open. Broad gates need prepared commands, identities,
bounds and required approval.
