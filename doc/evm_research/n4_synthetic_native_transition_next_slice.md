# N4 synthetic native transition: bounded next slice

Completed at `214c87b58`; see the [execution report](n4_synthetic_native_transition.md).
This contract is retained as slice history. Use the
[current prompt](../../next_executable_slice_prompt.md) for the next task.

## Goal and boundary

Execute the existing Rust-native `FinalChainNativeSession::finish_rewards` path
on a complete, explicitly synthetic, two-validator genesis state. The candidate
branch is period 1 with distribution frequency 2: a real rewards-stats plan
contains no distribution rows, but carries the normal cached-period intent.
Compare its terminal effects with a fresh execution of the pinned Go cold
`Init -> BeginBlock -> EndBlock -> Close` path using the same synthetic inputs.
This is a synthetic parity exercise; it does not qualify a historical producer,
reconstruct the mainnet state, or close the real-window acceptance gate.

The prior report [N4 cold Go witness](n4_empty_native_go_cold.md) is not a
complete snapshot: it has two reads and caller-supplied identity labels. Keep it
unchanged. Its pinned Go revision is `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`.
If the existing Go harness cannot accept the same synthetic inputs, add a
separate, observer-instrumented synthetic case. Do not claim exact read-set
parity: the Go witness observes account and jail-list reads; Rust validates raw
DPoS counter rows at slots 4 and 5. Record the actual ordered reads and writes
on each side.

## Existing Rust path to use

The positive baseline is
`empty_distribution_stream_is_a_zero_effect_nonboundary_phase` in
`rust/crates/rustaxa-consensus/src/final_chain/native_session/rewards.rs`. Its
current `plan(Vec::new())` is a test-only hand-built plan. Replace that plan
source in the synthetic case with the real
`FinalChain::plan_external_evm_rewards_stats` API in
`rust/crates/rustaxa-consensus/src/final_chain.rs`.

Use the existing full owner fixture constructor
`with_reward_chain_and_jail_policy` in `native_session/rewards.rs`: it creates a
real temporary `Storage`, two `GenesisValidator`s, DPoS/rewards configuration,
and a `FinalChain` at genesis. Parameterize its test setup to set
`rewards_distribution_frequency` to 2. Build a valid empty-vote
`FinalizedRewardsPeriodFact` for period 1, then call, in order:

1. `chain.plan_external_evm_rewards_stats(request_id, fact)`;
2. assert `distribution_stats.is_empty()` and that the cache intent is present;
3. `chain.begin_native_session_bound(request_id, 1.into(), GENESIS)`;
4. create `RewardState::from_snapshot(&session.dpos_state)` and call
   `session.finish_rewards(&plan, &state)`.

`RewardsStatsRuntime::process_period_result` in
`rust/crates/rustaxa-consensus/src/rewards_stats.rs` defines the frequency-two
nonboundary behavior. `reopened_cleanup_scheduler_matches_both_go_pins_through_native_rewards_and_install`
and its `current_reward_chain` helper in
`final_chain/current_rewards_scheduler_reference_tests.rs` are additional
examples of full synthetic `Storage`/`FinalChain` construction and real native
reward completion. Do not use its publication simulation for this cold case.

## Fixture integrity and assertions

Start from the complete `FinalChain`-owned `DposSnapshot` produced by the
two-validator genesis fixture. Do not label it as a complete historical or
mainnet snapshot. Derive canonical raw origin rows from that snapshot and assert
that `finish_rewards` reads the DPoS raw-storage keys for slot 4 (compact u64
vote count) and slot 5 (compact big-endian U256 delegated amount). Instrument `RewardState` or
an equivalent test adapter to record actual reads. A successful empty
nonboundary outcome must have zero account mutations, zero raw mutations, zero
reward, unchanged semantic snapshot, and no writes. Assert the exact observed
read keys and values; do not inject expected outcome fields into the plan.

Add negative cases with slot 4 or 5 absent and malformed. They must fail closed
with the established raw-integrity error and expose no successful effects.
Retain or extend the existing plan/request/period binding tests in
`native_session.rs`; verify identity and capability mismatches are rejected
before mutation. Missing origin data must never be treated as zero. Keep all
existing tests intact unless an intended product behavior changes; fix
implementation or parity wiring before changing an assertion.

For Go, use a disposable archive at the pinned revision and observer-only
instrumentation. Construct the same explicitly synthetic two-validator input,
run actual EndBlock, and record real terminal effects. Do not inject expected
results. Record equivalent validator state, genesis balances, configuration,
period and reward inputs in a shared fixture manifest. Verify that both engines
select the same nonboundary branch before claiming parity. Report any mismatch
as a failure; do not alter the oracle or assertions to obtain zero effects.
Separate fixture setup writes from measured lifecycle writes. Synthetic fixture
setup may persist the owner state required by each engine; this is not real-state
publication. Do not open `data/` or `local/evm-state-db/snapshot-litenode-copy`, or run
historical replay, transactions, reward distribution, PrepareCommit, root
calculation, publication, or adoption. A test-only DB used by the full Rust
owner fixture is allowed. No production routing or sparse production API is in
scope.

## Exit and validation

Success requires executed `finish_rewards`, zero effects as above, fail-closed
origin and authority negatives, and a truthful side-by-side Go/Rust synthetic
effect report. Keep producer qualification, complete historical snapshot,
mainnet real-window, and publication/adoption flags false. If a required owner
invariant cannot be satisfied, report the exact fixture/API blocker and tests;
do not call a planner-only result execution or parity.

Run the affected Rust package tests and applicable bridge/storage tests, then
`make rewrite-validate-fast`. Select any additional tier from
`doc/rewrite_validation_strategy.md`; CMake builds use 12 jobs. Honor existing
standing validation authorization and ask only for gates that existing policy
marks as requiring approval. Freeze source, fixtures, outputs, and test results
before independent review. After review and checks, make a local Conventional
Commit; do not push.
