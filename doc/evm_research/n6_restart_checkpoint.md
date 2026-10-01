# Restart checkpoint: persisted metadata DryRunner parity complete

## Current state

Branch: `feat/rust/evm-state-db`. Preparation is committed at `4148a1a` after
separate Sol review and quota-reader corrections. Decoded metadata is committed
at `86e74ff`. The latest implementation commit includes the completed
[ABI/admission report](n2_validator_info_abi.md), at `8945453`. The latest local
commit adds [persisted DryRunner parity](n3_metadata_dry_runner.md); use `git log`
for its exact ID.
No push or production routing is authorized.

The [metadata report](n2_staged_validator_info.md) records the shared pending and
historical session adapter, actual dual-pin Go corpus, complete logs and frozen
independent Astra review. Five targeted tests, workspace fast checks, an ON
consensus bridge build with 12 jobs, and all 15 bridge tests passed. The first
owner-fixture failure and independent-review gas-funding correction are preserved
in `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

The subsequent ABI/admission slice passed seven targeted Rust tests, both pinned
21-case Cacti oracles plus diagnostic reproduction, workspace fast checks and a
rebuilt ON consensus bridge with all 15 tests. Frozen independent Astra review
accepted it without findings or corrections. Scope is staged metadata over
consistent snapshots, with exact ABI/admission and synthetic active-Cacti
evidence. Sparse owner/info distinctions, pre-Cornus session admission, delegate,
call-code and metadata estimation/trace parity remain open. Historical
simulation here is synthetic and does not certify real inputs.
Milestone 10 and N1–N6 remain open. Supplied data and historical copies were not
opened or changed. The earlier N4 hardening remains complete at `5bfdf494c`;
its reports and [checkpoint history](n6_restart_checkpoint_history_2026_10_01_post_hardening.md)
are retained.

## Next action

Continue the active [root prompt](../../next_executable_slice_prompt.md) and
[remaining queue](n6_remaining_branch_queue.md) under the
[slice workflow](../codex_slice_workflow.md). The frame/API slice passed all five
integration tests, dual-pin fixture reproduction and workspace fast checks.
Independent frozen Astra review accepted it without findings. It covers eight
actual frames and sixteen historical probes over synthetic in-memory readers.
The subsequent metadata DryRunner slice passed all six persisted simulation
tests, reproduced nine actual dual-pin cases and passed workspace fast checks.
Independent frozen Astra review accepted it without findings. Thirty-six Rust
metadata simulations cover two physical reader reopens and fresh sessions for
each probe. Matching semantic history remains fixed; no network adoption or
semantic-owner reopen is claimed. The first intrinsic-gas fixture discovery and
its retained/additional cases are in the report and complete logs.

The next ready bounded N3 dependency is native gas-estimation probe composition,
using actual Go DryRunner probes, the independently extracted C++ search and
fresh real Rust historical simulation ports. Luna is checking that candidate.
Reuse the existing complete seed and read only selected ranges. Other native
method and trace gaps remain in the milestone queue.

Current root log confirmed `gpt-6.1-sol` medium. Luna startup map ran on
`gpt-6-luna` medium; independent metadata review ran on `gpt-6-astra` medium.
There were no routing failures. Compatible reviewer/helper threads can be reused
in this session; fresh startup must confirm routing again. Account allowance was
27% after ABI validation and review. Read fresh telemetry at startup and
before new work; do not reuse this observation. At 20% remaining, stop new work
and finish only the in-flight atomic closeout. Unknown telemetry after one
bounded refresh also stops new work.

An asynchronous request is pending for the exact snapshot executable identity,
runtime overrides and capture command. The likely release commit is already
recorded; those missing producer facts remain an N4 blocker. Other ready N2/N3
work can continue. Broad replay/differential/fault gates still need the authority
required by repository policy. Nothing has been pushed.
