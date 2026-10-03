# Restart checkpoint: redelegation composition

Branch: `feat/rust/evm-state-db`. Latest accepted commit: `65ef42b5b`.
No push, fallback or production routing is authorized. Milestone10 and N1–N6 stay open.

The prepared adapter/ABI/frames/historical simulation/estimation/direct traces
chunk is complete and independently accepted. New caller destination partial
execution and all four compositions are also accepted. Full caller-source removal
with another delegator retaining the source validator is accepted at `6b5228ad2`;
its actual full-source frames are independently accepted.
Accepted commits, checks, corrections and model routes are in the
[scorecard](../codex_slice_scorecard.md). Current bounded scope and exclusions:
[N1 coverage](n1_existing_network_contracts.md),
[full source runtime](n2_redelegate_full_source_retained_validator.md),
[full source frames](n2_redelegate_full_source_frames.md).

Full-source complete historical simulation is independently accepted;
see [record](n3_redelegate_full_source_simulation.md). Next: estimation and direct
supported traces. The accepted historical profile has both validators
with aa/bb1,000 each (total4,000), permitting both caller removal shapes while
both validators stay positive. Separate complete actual Go H1 physical inputs
from prior touched-row frame maps. Use public H1 finalization once; semantic-owner
restart must load H1 without refinalization. Astra contract review is accepted.
Do not claim private synthetic adapter history is actual DryRunner API evidence.

Reference pins: public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`,
local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.
Artifacts: `/home/fry/artifacts/evm-redelegate-2026-10-03/`; retain first failures,
frozen hashes and independent reviews. Sol medium leads; Luna medium maps;
Astra medium reviews uncertain contracts/authentication, independent Sol medium
reviews settled derivatives. All requested routes ran without routing failures.
Required targeted/parity/ON bridge and serial workspace fast checks passed for
accepted runtime. Use `RUST_TEST_THREADS=1` for workspace fast (existing temporary
DB lock collision). No C++ or storage-module change is in this run.

Active [prompt](../../next_executable_slice_prompt.md) and
[workflow](../codex_slice_workflow.md) use80% weekly allowance remaining.
Latest fresh lead allowance:83% at2026-10-03T02:58:45.801Z. Refresh before starts;
at80% or less start no new work, finish only in-flight atomic closeout. Unknown
telemetry after one bounded refresh also stops new work.

Producer executable identity, runtime overrides and capture command stay unknown.
Do not repeat the request or invent them. Qualified complete real-window inputs,
real signed-period/root parity and N4–N6 acceptance remain open. Broad replay,
differential and fault gates require exact prepared commands/data/bounds and approval.
