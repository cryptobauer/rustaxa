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

Continue with a bounded Rust-native EndBlock/transition contract through existing
Rust kernels. First define its authority boundary, exact inputs/outputs, reference
oracle, tests and non-goals; then implement only the reviewed slice. Keep producer
policy and reward-inclusive replay inputs distinct follow-up gates. Do not start
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
