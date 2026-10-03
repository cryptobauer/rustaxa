# Restart checkpoint: redelegation composition

Branch: `feat/rust/evm-state-db`. Latest accepted implementation: `752629e9c`.
Run base: `327d15fa1`. Stop:80% weekly allowance remains at2026-10-03T03:41:08.680Z.
No push, fallback or production routing. Milestone10 and N1–N6 stay open.

The prepared partial adapter/ABI/frames/historical simulation/estimation/direct
trace chunk is complete. Partial new caller destination and full caller-source
removal with another delegator retaining the validator also have independently
accepted staged, actual frame, complete historical simulation, estimation and
direct structured trace composition. See [scope](n1_existing_network_contracts.md)
and [accepted slices/checks/routes/corrections](../codex_slice_scorecard.md).

Pre-Aspen-two zero amount is accepted only with two positive existing caller
pairs: [staged runtime](n2_redelegate_zero_existing_pairs.md) `8a6735c94`,
[actual frames](n2_redelegate_zero_frames.md) `752629e9c`. Two actual zero calls
succeed with22 ordered writes; parent revert removes both logs and retains raw
state. Per-call/final account parity is measured for the new frame cases only.
No complete physical or public historical authority comes from touched frame maps.

Next ready slice: one actual zero-success DryRunner case over the existing complete
2,000-principal H1 profile, then public historical simulation, estimation and traces.
The in-flight historical contract was accepted before closeout; implementation
and gates did not start. Resume after a fresh allowance check permits new work.
Map: `zero-historical-api-map.md`; settled contract/checks:
`zero-historical-contract-review.md` in the artifact directory below.
Use unchanged `redelegate::{history,assert_committed}`: public H1 finalization once,
owner restart without refinalization, complete actual Go seed rows, independent
reader reopen and eight fresh simulations. Preserve stored heads/cursors and
DryRunner effective committed nonce+1; no frame seed/root transplant.
Keep zero+absent/zero caller pairs, full+new destination, source-validator deletion,
nonzero rewards, new validators and historical same-validator successes excluded.

Reference pins: public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`,
local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.
Artifacts: `/home/fry/artifacts/evm-redelegate-2026-10-03/`; retain first failures,
commands/full outputs/exit codes, reviewed freezes and independent review reports.
Sol medium led directly; Luna medium mapped; Astra medium reviewed contracts/auth,
independent Sol medium reviewed settled derivatives. All requested routes ran.
Accepted affected check/clippy/tests, actual dual-pin parity/control checks,
ON bridge build12/all15 and serial workspace fast passed. Use
`RUST_TEST_THREADS=1` for fast (existing temporary DB lock collision).
No C++ or storage-module change; no new broad/differential/fault gate.

Active [prompt](../../next_executable_slice_prompt.md) and
[workflow](../codex_slice_workflow.md) use80% remaining; at or below this floor
start no new work. Producer executable identity, runtime overrides and capture
command remain unknown. Do not repeat requests or invent facts. Qualified real
network windows, signed-period/root parity and N4–N6 acceptance stay open.
