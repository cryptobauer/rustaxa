# Restart checkpoint: native metadata estimation complete

## Current state

Branch: `feat/rust/evm-state-db`. Preparation is committed at `4148a1a` after
separate Sol review and quota-reader corrections. Decoded metadata is committed
at `86e74ff`. The latest implementation commit includes the completed
[ABI/admission report](n2_validator_info_abi.md), at `8945453`. The latest local
commit adds [native estimation](n3_metadata_estimation.md); use `git log`
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
call-code and metadata trace parity remain open. Historical
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

Native metadata estimation also passed targeted tests, actual dual-pin/C++
reproduction and workspace fast checks; independent frozen Astra review accepted
it without findings. The unchanged C++ search consumes 24 exact actual-Go probes,
and Rust runs 96 fresh simulations across reader reopen with matching results.
The next ready bounded N3 candidate is direct native structured tracing with
retained prefix/target state. Read the real Go TraceRunner lifetime before design;
its delayed native factory uses live block state rather than DryRunner history.
Do not infer full delayed-query/native trace parity from metadata-only coverage.

Current root log confirmed `gpt-6.1-sol` medium. Luna startup map ran on
`gpt-6-luna` medium; independent metadata review ran on `gpt-6-astra` medium.
There were no routing failures. Compatible reviewer/helper threads can be reused
in this session; fresh startup must confirm routing again. Account allowance was
26% after estimation validation and review. Read fresh telemetry at startup and
before new work; do not reuse this observation. At 20% remaining, stop new work
and finish only the in-flight atomic closeout. Unknown telemetry after one
bounded refresh also stops new work.

The user reports that exact producer facts might not be recoverable. Record
executable identity, runtime overrides and capture command as unknown unless
evidence is found; do not invent them or repeat the request. The likely release
commit and checked-in reference configuration remain available for bounded
checks, but do not prove the producer setup. Qualified real-window acceptance
remains limited by those unknowns and completeness; other ready N2/N3
work can continue. Broad replay/differential/fault gates still need the authority
required by repository policy. Nothing has been pushed.
