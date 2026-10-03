# Redelegation gas-estimation composition

Baseline: `656a5e314`. Test/evidence-only N3 slice. Existing `estimate_gas`,
`simulate_with_native`, native session, reader and journal owners remain unchanged.
The accepted two-validator committed H=1 seed and ten DryRunner request cases
are reused. No production routing, supplied data or upstream source is changed.

`native_redelegate_estimate_reference.go` executes each candidate twice through
actual pinned DryRunner.Apply. Its harness archives both unchanged pins, includes
the exact accepted seed sources and compiles the unchanged `eth_estimateGas`
search body from upstream Eth.cpp using the existing C++ reference harness.
That callback verifies every candidate gas and requires all Go probes consumed.
The manifest records source, request, support-harness, algorithm and output hashes.
Both pins produce identical outputs. Fifteen probes cover ten cases: six for
partial success and one for each initial native/consensus failure. C++ returns
104,977 for partial success; it retains exact normal/native/ABI/payability and
intrinsic errors for the others. Full state rows/root before/after are identical.

The Rust integration test performs 120 actual fresh native simulation probes:
two independently constructed semantic owners (one restart), two physical
reader reopen cycles each and two repeated estimates over all 15 probes.
Every request gas, effective nonce, concrete identity, output, error, log and
consumed transcript length matches Go/C++. The request nonce remains unchanged.
Typed estimator consensus versus code errors are checked separately. Exact
physical rows remain unchanged, and public semantic reads verify both validator
stakes and aa delegation rows remain 1,000 with zero pending rewards.

First-run commands, full outputs and exit codes use `estimate-` prefixes in
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Actual record/reproduction,
all 14 API integration tests and affected-package clippy pass. Serial workspace
fast passes. No runtime/bridge/storage-module changes occur;
prior accepted ON bridge12/all15 covers the unchanged runtime. No correction
failure occurred; typed error assertions were strengthened before final checks.
Independent Sol medium frozen review accepted all 11 hashes without blockers;
the report is estimate-review.md in the artifact directory. Implementation
route is confirmed Sol medium; the historical contract and API owners are settled.
This synthetic estimation evidence does not close real-history or RPC routing.
Supported direct structured traces follow; N1–N6 and excluded adapter successes
remain open.
