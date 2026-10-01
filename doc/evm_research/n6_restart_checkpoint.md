# Restart checkpoint: metadata ABI/admission complete

## Current state

Branch: `feat/rust/evm-state-db`. Preparation is committed at `4148a1a` after
separate Sol review and quota-reader corrections. Decoded metadata is committed
at `86e74ff`. The latest implementation commit includes the completed
[ABI/admission report](n2_validator_info_abi.md); use `git log` for its exact ID.
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
call-code and complete Rust frame/API composition remain open. Historical
simulation here is synthetic and does not certify real inputs.
Milestone 10 and N1–N6 remain open. Supplied data and historical copies were not
opened or changed. The earlier N4 hardening remains complete at `5bfdf494c`;
its reports and [checkpoint history](n6_restart_checkpoint_history_2026_10_01_post_hardening.md)
are retained.

## Next action

Continue the active [root prompt](../../next_executable_slice_prompt.md) and
[remaining queue](n6_remaining_branch_queue.md) under the
[slice workflow](../codex_slice_workflow.md). The next ready bounded metadata
dependency is actual Rust frame/API composition. Luna mapped real owner harnesses
in `rust/crates/rustaxa-evm/tests/native_session_reference.rs` and
`native_simulation_reference.rs`, with the real simulation adapter in
`tests/support/mixed_native.rs`. Add CALL, STATICCALL and parent-REVERT metadata
fixtures and compare actual driver/journal and simulation outputs. Do not use
the scripted-port tests as native business parity. Read only selected ranges.

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
