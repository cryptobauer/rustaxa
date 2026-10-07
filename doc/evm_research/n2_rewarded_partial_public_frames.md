# Positive-reward partial public Rust frames

The new redelegate_rewarded_partial_frames test composes the public EVM driver,
ordinary/raw journal lanes and existing mixed-native port with the accepted
[actual Go frame oracle](n2_rewarded_partial_frames_oracle.md). It adds no runtime
API or production route and preserves earlier helpers and empty-account rules.

Each of seven cases creates the two-validator31/32/common-ownerA1 chain with
callerD1/owner1000 principal on each validator, commission100, yield20,
blocks_per_year1, maximum1000000/minimum100, fix/Magnolia/Ficus/Cornus/Aspen1 at0
and Aspen2 disabled. Rust FinalChain takes ordinary balances after genesis
custody (D1=3000,A1=1000); Go configuration takes5000/3000 before custody.
The Rust mirror builds native principal4000 itself. Committee1 and one cert vote
for31 atP1 represent the independent oracle's declared TotalMaxVotesWeight1;
block-author/DAG caps are0, so the unrelated signed PBFT author gets no reward.
The fixture does not represent a real network certificate.

A signed empty PBFT period is stored, then public
finalize_block_with_rewards_facts_at publishes P1 with mint800. The test verifies
reward header and P1 balances, drops and reopens FinalChain, verifies head1, then
uses public begin_native_session_bound(request,2,1). No private snapshot advance
or field reset is used. This proves persisted FinalChain P1 hydration for this
case; external concrete EVM P1 publication/root parity remains outside scope.
FinalChain account/DPos snapshot and EVM StateRead are separate authorities.
FinalChain balance checks do not claim byte-identical native code/nonce metadata.

The EVM fixture is an independent logical view of actual Go committedP1, with
period1 and a declared synthetic root marker. It resolves the measured52 selected
keys forward from91 physical rows, and adds two actual owner rows needed by Rust
authentication but not read by Go's Run. Their values come from physical rows,
not guessed owner bytes. It retains actual account nonce/balance facts and adds
only the declared caller code overlay. It is not a public StateAPI snapshot import.

The existing MixedNativeExecutionPort converts real staged ordinary/raw effects;
a local recording wrapper only copies quote, invocation and returned outcomes.
Each case uses one fresh P2 journal/session/sequence. Assertions cover exact quote,
supplied gas, input/value/caller/depth/period/position/sequence/CALL or STATICCALL,
per-call status, logs, separate account and raw vectors, every intermediate raw
expectation, transaction gas/output/logs/status, invocation dispositions, final
accounts and all selected raw bytes. Two-call parent REVERT includes normal701
failure after successful300. Payout account/log effects roll back on parent REVERT,
while exact12 native raw writes remain. STATICCALL matches the actual payout.
All untouched rows, including both extra owner rows, remain fixed. Disposal leaves
public FinalChain P1 accounts, principal and validator stakes unchanged.

Initial compile API failures, mistaken pre-custody balances and omitted owner-row
authority are retained with first command/output/exit records. Corrections use
existing typed constructors and physical P1 facts. No prior test is weakened.
Required checks: focused seven-case test, full EVM package, consensus/EVM all-target
check and Clippy, serial rewrite-validate-fast, explicit tracked/untracked whitespace
and frozen independent Sol medium review. Unchanged runtime4c82359 retains its
successful Rust-enabled bridge build12/all15 evidence; no new C++ bridge behavior
is introduced by this test. Publication faults, general reward graphs, signed
historical API/estimate/trace parity and production acceptance remain open.
