# Restart checkpoint: existing-network milestone

Milestone 10 remains in progress and is not accepted. Milestone 09 is complete
only for its synthetic four-period scope; implementation plan 08 also remains
open. The latest N4 continuation is recorded at `a028b7a2a`. These recorded
results do not establish the current checkout's acceptance, a production backend
switch, existing-head adoption or full replay.

## Start here

Inspect branch, commit, worktree, ignored database paths and current artifacts
before edits. The expected work branch is `feat/rust/evm-state-db`; do not assume
the workspace is at the recorded N4 commit. Read `AGENTS.md` and this checkpoint,
then read only the relevant `PLAN.md`, [milestone 10](10_existing_network_milestone.md) and
[implementation plan 08 sections](08_implementation_plan.md#agent-and-model-assignments)
for the chosen slice. Use that linked section as the current model table; read
the [model/environment record](n6_model_setup_validation.md) only when routing
agents or recovering a previous session. Follow the Luna-first capacity policy.
Do not rewrite old model assignments as current routing evidence.

The restored N4 qualification and bounded probes are complete. Their detailed
evidence is in the [N4 report](n4_restored_snapshot_evidence.md), the
[four-key proof report](n4_head_sender_proofs.md), the
[independent reward inputs](n4_independent_reward_inputs.md), the
[reward planner comparison](n4_independent_reward_plan.md), the
[native-effects witness](n4_empty_native_effects.md), and the
[cold Go witness](n4_empty_native_go_cold.md). The corresponding task definition
is closed and preserved in [the N4 intake](n4_restored_snapshot_next_slice.md).
The original checkpoint is archived byte-for-byte in
[the October 1 history](n6_restart_checkpoint_history_2026_10_01.md); the dated
agent ledger retains assignment and runtime outcomes. Earlier slice details remain
in the [progress checkpoint](n6_progress_checkpoint.md) and dated
[handoff ledger](n6_agent_handoffs.md); these are historical sources, not new work
instructions.

## Current limits and authority

The supplied `data/` tree is evidence and must remain unchanged. Use only the
independent, requalified copy at `local/evm-state-db/snapshot-litenode-copy/` for
bounded read checks, after verifying its recorded identity and contents. The
recorded paired mainnet head is 25,706,949. Copy and qualification evidence does
not prove producer binary or capture provenance, full trie closure, or complete
native state.

Reward inputs and planner comparisons are bounded. Typed reward fields match in
the candidate comparison, while serialized RLP still differs in validator
ordering. Producer identity, hardfork/PBFT settings, reward transition, actual
Rust EndBlock, cache closure and reward-root execution remain open. The four
sender physical reads returned `history_unavailable`; separate authenticated
proofs establish NonMember for those four keys only. Neither result proves global
native completeness. The cold Go witness observes no backend writes in its narrow
fixture and does not execute warm state, transactions, reward distribution,
PrepareCommit or root calculation. Consult the linked reports for exact scopes.

Never give sparse reads or caches complete-snapshot, adoption or publication
authority. Adoption/recovery still requires complete DposSnapshot maps, global
principal/reward-graph invariants, snapshot/projection hashes, paired database
ownership, durable ordering and idempotent recovery. Full native/catalog
qualification, slashing reconstruction, producer policy, reward-inclusive real
replay and non-genesis adoption remain separate gates.

## Next action and validation

The [bounded transition closeout](n4_rust_native_transition_blocked.md) records
the slice from `896ac8886` under the [prompt](../../next_slice_prompt.md),
[contract](n4_rust_native_transition_next_slice.md) and
[review pilot](n6_token_usage_pilot.md). Rust EndBlock execution is **blocked**:
the complete FinalChain owner requires live storage, and terminal processing
requires raw vote/stake origins absent from the two-read cold Go fixture. Four
database-free rejection/classification tests passed in the existing raw/account
helpers. No Rust lifecycle execution, parity, zero-effect or acceptance result
is claimed. The full workspace test/hook gate remains outside the no-database
scope. Independent Astra review found no blocking findings. See the closeout
for focused checks, routing, source usage counters and the pending external
log audit. This checkpoint and the rejection tests are committed together.

The next executable slice needs authorized isolated complete synthetic owner
state and exact raw rows, or a separately reviewed database-free owner boundary.
Keep producer policy and reward-inclusive replay inputs distinct follow-up gates. Do not start
broad replay, expensive differential/fault campaigns, sparse publication, protocol
changes, original upstream C++ changes or production routing without the required
task-owner authorization.

For a slice, use the narrowest tier in
[`rewrite_validation_strategy.md`](../rewrite_validation_strategy.md). Require
the focused Rust package tests and reference parity; add the applicable Rust
smoke/subsystem checks for startup, sync, consensus, finalization or RPC paths.
For storage changes, build and run `rust_storage_tests` and affected C++ tests.
Ask first before expensive repository-wide or differential gates. Keep replay,
fault campaigns and sustained workloads within the authorized slice. Record
exact commands, data identity, results, failures and skipped required checks. An
aggregate pass does not cover skipped requirements.

The September 30 closeout sample was **65% used / 35% remaining** at 14:23 UTC.
The [separate usage audit](../../token_usage_audit.md) records a later **66% used**
sample. Keep both as time-specific observations; neither is a billing claim or a
hard quota limit. The owner's reserve remains at least 25% weekly usage, with no
new slice started below 30% remaining. Monitor telemetry and preserve closeout
capacity.

## October 1 bounded synthetic execution handoff

The [executed synthetic nonboundary report](n4_synthetic_native_transition.md)
records the complete two-validator Rust owner fixture, actual frequency-two
planner, bound session and successful `finish_rewards`, plus fresh pinned Go
cold Init/BeginBlock/EndBlock/Close execution. Both have zero terminal effects;
Rust returns zero reward and unchanged semantic state. Rust reads slots 4/5;
Go reads the DPoS account and absent jail list. All 27 Go raw genesis rows match
Rust; Rust has two extra empty-state representations. Exact read-set and full
raw snapshot identity are not claimed.

Origin and request/period/authority negatives pass. Focused Rust and
consensus/storage bridge checks pass; the fast workspace gate passes. Source,
fixtures and results were frozen before independent review. The report records
hashes, the same-route model-capacity interruption, unavailable usage telemetry
and correction groups. No commit or push was made by the implementation lead.
Producer qualification, historical/mainnet completeness, real-window acceptance,
publication and adoption remain open. Preparation and historical routing records
are preserved.

Independent Astra-medium review completed with no blocking findings. Root
reran consensus and storage bridge tests with retained logs (15/15 and 4/4).
The report records the compact slot-5 codec correction and two nonblocking
regression limits. The reviewed synthetic slice and this handoff are included
in a local Conventional Commit. No push was made. Pre-existing preparation
changes remain in the working tree.


## October 1 synthetic fixture-hardening handoff

The [fixture-hardening report](n4_synthetic_fixture_hardening.md) closes the two
harness regression gaps recorded at `214c87b58`. The Go runner now requires an
empty observed raw-write list. Both engines validate every field in the complete
shared synthetic manifest before setup or lifecycle execution. Each engine
rejects 306 missing, unknown, type, value and array drift controls. Positive
lifecycle behavior and raw-integrity/request/period/authority rejection coverage
remain intact. The input contract contains no expected execution effects.

Fresh Go/Rust outputs and the source/evidence freeze record are separate files;
old outputs, source pins and historical reports remain as history. Both engines
still have zero measured effects, Rust has zero reward and unchanged semantic
state, and all 27 Go raw genesis rows match Rust. Exact read sets and complete
physical snapshot identity remain different.

Targeted Rust tests pass (4/4 nonboundary and 17/17 rewards); Python raw-write
controls pass. Consensus/storage bridge build and tests pass (12 build jobs,
15/15 and 4/4 tests, `RUSTAXA_ENABLE=ON`). The workspace fast gate passes.
Commands, exit statuses and complete first-run logs include both corrected Go
harness failures. Independent Sol-medium review follows source/evidence freeze;
its result is recorded in the report. The lead implemented directly; bounded
Luna-medium input mapping completed. Runtime and current quota telemetry remain
unknown; no model, quota or billing claim is inferred from assignments.

Only fixture/test code and evidence changed. No production routing, supplied
database or historical snapshot access, replay, adoption or root derivation was
added. Producer qualification and the real-window gate remain open. This slice
is committed locally after review. No push is made. Pre-existing preparation
remains outside this commit.
