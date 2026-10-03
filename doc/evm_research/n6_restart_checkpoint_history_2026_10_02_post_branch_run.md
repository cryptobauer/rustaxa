# Restart checkpoint: redelegation prerequisites complete

## Current state

Branch: `feat/rust/evm-state-db`. Run baseline: `5bfdf494c`.
Use `git log -1` for the checkpoint commit's exact ID. All completed slices below
have local Conventional Commits. Nothing was pushed. No production routing,
upstream C++ changes, supplied-data opens or historical-data mutations occurred.
Milestone 10 and N1–N6 remain open.

The latest accepted step adds seven normal redelegation kernel failure cases;
all 12 targeted tests and workspace fast checks passed. Frozen independent Sol
medium review accepted it without findings, conditional on the fast gate that
subsequently passed. The new test passed on its first run.

## Completed branch work

| Work | Local commit / evidence |
| --- | --- |
| Approved prep docs/tooling and quota-reader correction | `4148a1a4f`; separate Sol review, six Python tests and fast checks |
| Staged validator metadata | `86e74ff37`; [report](n2_staged_validator_info.md) |
| Metadata selector-first ABI/admission | `8945453bc`; [report](n2_validator_info_abi.md) |
| Metadata frame/API composition | `1fe3282f2`; [report](n2_validator_info_frames.md) |
| Actual metadata DryRunner | `abd7a111d`; [report](n3_metadata_dry_runner.md) |
| Metadata gas estimation | `33e8002ac`; [report](n3_metadata_estimation.md) |
| Direct metadata structured trace facade | `1e49be922`; [report](n3_direct_metadata_traces.md) |
| Exact active escrow entry | `0cdef0e9b`; [report](n2_staged_escrow_transfer.md) |
| Actual redelegation observations | `c88907de3`; [report](n2_redelegate_observations.md) |
| Redelegation account-port dependency | `a3951c811`; same report |
| Actual escrow DryRunner | `d2207e387`; [report](n3_escrow_dry_runner.md) |
| Escrow gas estimation | `fb3b56e74`; [report](n3_escrow_estimation.md) |
| Direct escrow structured traces | `b00f6ade2`; [report](n3_direct_escrow_traces.md) |
| Redelegation kernel/write composition | `32fdf4dbc`; same redelegation report |
| Seven normal kernel preflight failures | checkpoint commit; same report |

Each linked report records exact scope, checks, corrections and independent
review. Runtime changes passed relevant ON consensus bridge builds with 12 jobs
and all 15 bridge tests. Later test-only steps used affected Rust tests and
workspace fast checks. Escrow API evidence covers 20 DryRunner simulations,
104 estimation probes and 28 trace runs across physical reader reopens, with
fixed semantic owners. Do not infer semantic-owner restart or real-history parity.

Complete commands/output/exit files, first failures, source-hash freezes and
immutable review copies remain in
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`. Slice measurements and routing
are in the [scorecard](../codex_slice_scorecard.md). Quota samples are account
observations, not inferred token usage or billing.

## Next executable slice

Continue the active [root prompt](../../next_executable_slice_prompt.md) and
[remaining queue](n6_remaining_branch_queue.md), using the
[slice workflow](../codex_slice_workflow.md). Staged redelegation is still
unsupported. Its two kernel account parameters now accept `DposAccountPort`.
The existing kernel plus source/destination serializers match actual Go's
12/10 writes across two partial calls, including repeated-key expectations,
logs and zero account effects. Seven normal preflight errors also match Go.

Astra medium settled the [bounded adapter contract](n2_redelegate_observations.md#settled-bounded-adapter-contract):
fresh authentication per invocation over consistent complete snapshots, without
Go cache/read-count parity. Use one raw trace. Preserve cold normal failure
prefixes before success-scope guards. Authenticate head/cursor/current reward
nodes and both membership positions before the kernel. Allow only active
Magnolia/Ficus, post-fix distinct validators, positive partial principal,
existing positive destination delegation, zero relevant pools/indices and
retained positive validators. Run the existing kernel on a clone, permit no
account access/effects, serialize source then destination, advance only on
complete success. Integrity/reader errors abort; excluded successes are explicit
scope errors. Malformed ABI and historical same-validator branches stay outside.

Own `rust/crates/rustaxa-consensus/src/final_chain/native_session.rs` admission,
`native_session/custody.rs` dispatch and a narrow redelegation child module/tests.
Reuse the existing helpers and actual fixtures. Add focused pending/historical
session tests for repeated authentication, 0/1/3/4/5 normal failure prefixes,
bad node/membership rows, reader failures and unsupported successful branches,
all without partial effects or semantic advance. Add real frame/API composition
before claiming that boundary. Required closeout: affected Rust package tests,
actual pinned parity reproduction, focused ON consensus bridge build/tests,
workspace fast, immutable freeze, independent review and local commit.

## Capacity and remaining inputs

Last fresh root observation: 21% remaining. This is near the user's requested
approximately 20% reserve; the current atomic test/review/commit is complete.
Read fresh telemetry before resuming. The repository's exact no-new-work floor
remains 20%; unknown telemetry after one bounded refresh also stops new work.

Confirmed runtime: root `gpt-6.1-sol` medium; Luna bounded startup/map
`gpt-6-luna` medium; independent reviews `gpt-6-astra` medium and
`gpt-6.1-sol` medium. No routing failures. Reuse existing threads where available;
fresh startup must confirm routing/capacity and Luna again.

The user reports that exact producer facts may be unrecoverable. Keep executable
identity, runtime overrides and capture command unknown unless evidence is found.
Do not invent facts or repeat the request. The likely release and checked-in
reference configuration support bounded checks, but do not prove producer setup.
N4 still needs qualified completeness and real signed-period parity. Broad
replay/differential/fault gates need the authority required by repository policy.
Push and production cutover remain separate. Earlier N4 hardening and its
[checkpoint history](n6_restart_checkpoint_history_2026_10_01_post_hardening.md)
remain complete and preserved.
