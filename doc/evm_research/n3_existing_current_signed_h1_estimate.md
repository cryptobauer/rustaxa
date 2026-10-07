# Signed existing-current H1 estimation

This derivative uses the accepted [signed H1 simulation](n3_existing_current_signed_h1_simulation.md)
and its complete105-row physical identity. Public Rust history finalizes the
same canonical partial300 transaction once, then reopens the semantic owner.
Caller balance3000, ordinary ownera1 balance1000, native4000, supply8000,
both node1 count2 and the explicit fork/locking configuration are unchanged.
The runtime, old producers, corpora and shared support are unchanged.

The new Go producer executes actual DryRunner.Apply twice from fresh state for
each gas probe on both pinned revisions. It exports the complete H1 before and
after and requires exact equality. The harness requires equality across pins
and feeds the measured transcript to the unchanged C++ search body. Its
manifest binds input, producer, support, snapshot helpers/targets, stderr,
output and C++ algorithm identities. The eight actual probes are six successes
at200000,150956,126434,114173,108042,104977; minimum101912 succeeds;101911 fails
out of gas with used21912. The unchanged search returns104977 and101912, then
the exact out-of-gas error for the below-minimum request.

The Rust test compares each requested gas, effective nonce, admission/error,
used gas, return and event to those actual probes. Every callback starts a fresh
native session selected at committedH1. It checks the supplied request nonce,
public committed balances/delegations/stakes, owner H1 header and all complete
physical rows after each probe. Two owner lifetimes, two reader lifetimes each
and two repeated searches per case give64 disposable sessions and24 search
calls. Search consumption and results match the compiled C++ transcript.
No private poststate or physical-row hydration of the semantic owner is used.

Required checks are actual dual-pin recording/reproduction, Python compile,
targeted Rust estimation, EVM package/check/Clippy, serial rewrite-validate-fast,
whitespace and independent same-profile Sol medium frozen review. First-run
logs and hashes are retained under the run3 artifact directory with the
existing-current-h1-estimate prefix. Runtime remains072fc39fb; its accepted
Rust-enabled bridge build12/all15 is inherited separately, without a new run.

This is bounded same-H1 historical estimation, native invocation1 and effective
EVM context1. Direct H2 trace, real-window root parity and production acceptance
remain open. No Rust/Go root equality, supplied-data mutation, C++/storage owner
change, protocol/fallback, broad gate or push is included.

Run4 source correction: same-H1 Go DryRunner and public Rust simulation use literal
EVM block/native period1. Effective nonce2 in this older caller-nonce1 profile is
independent of block identity. The original prose bytes are frozen in run4
h1-context-before; previous checks, routing and historical report bytes remain
unchanged. The new source-only reward-h1-contract records exact source evidence.
