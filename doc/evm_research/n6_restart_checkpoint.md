# Restart checkpoint: full+new observation and staged contract next

Branch: `feat/rust/evm-state-db`. Accepted runtime/frame tip: `752629e9c`;
previous run closeout: `049ec5e73`. Preparation commit: `5a74db354`; accepted zero simulation: `1c5de6f75`.
No push, fallback or production routing. Milestone 10 and N1–N6 stay open.

Partial existing/new caller destination and full caller-source removal with a
retained validator have accepted adapter, frame, historical simulation, estimation
and direct trace evidence. Pre-Aspen-two zero amount with positive existing caller
pairs has accepted runtime and frame evidence; historical simulation is accepted; estimation and direct traces are accepted.
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

[Estimation](n3_zero_redelegate_estimation.md) and [direct zero traces](n3_direct_zero_redelegate_traces.md)
passed actual dual-pin reproduction and all required targeted/package/serial fast
checks. Frozen independent Sol review accepted both, including corrected trace
head checks. Commits are in Git history and the scorecard. All three bounded
pre-Aspen-two zero API derivatives are complete.

Next N2 question: full caller-source removal into a new caller destination while
another delegator retains both validators. Astra medium accepted a bounded staged
contract; original and corrected reports are in the current run directory. Final
profile uses d1/a1, source/destination 31/32, initial stakes 2000/1000, one caller
membership [31], full amount 1000, zero rewards. Actual Go observation/control
record and reproduction passed in disjoint new files; runtime guard stays unchanged.
Destination serialization needs explicit intermediate empty membership after source
removal. Restrict initial admission to one caller member; arbitrary swap-last+append,
other broader profiles and API/frame acceptance remain excluded. Next: implement
under the corrected contract, validate parity/package/ON bridge with 12 jobs/serial
fast, freeze, obtain independent review and commit only accepted scope.

Artifacts and first-run logs are in the current run directory. Latest allowance
64% at `2026-10-06T04:01:32.448Z`; baseline 66%, target 56%, same reset. Serial fast
uses `RUST_TEST_THREADS=1` for existing temporary database locks. No push, production
route, broad gate or supplied-data mutation is authorized.

Producer executable identity, runtime overrides and capture command remain unknown.
Do not invent facts or repeat requests. Qualified real windows, signed-period/root
parity and N4–N6 remain open. Broad gates need prepared commands, identities,
bounds and required approval.
