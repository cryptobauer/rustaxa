# Funded H1 claim-all public estimation

The public estimator composes `estimate_gas` with a fresh
`simulate_with_native` call for each probe. The test uses the accepted finite
HistoricalSimulation period 1 claim-all class from `23e859793`. It changes no
runtime code. The existing H1 simulation test and its 176 ports stay unchanged.

## Inputs and measured behavior

The dual-pin Go oracle and unchanged C++ search transcript are in
`experiments/evm_feasibility/fixtures/native_reward_claim_all_h1_estimate`.
Their accepted preparation commit is `b8a14fa44`. Public and local Go outputs
must match. Each probe uses the same complete 92-row P1 reader and a separate
public FinalChain owner at finalized P1. The supplied 512-bit nonce stays in
the original transaction; simulation selects nonce 1 and presents nonce 2 to
the native preparation callback.

| Gas cap | Probe gas limits | Estimate |
| --- | --- | --- |
| 200000 | 200000, 155636, 133454, 122363, 116817 | 116817 |
| 111272 | 111272 | 111272 |
| 111273 | 111273 | 111273 |

Every successful probe consumes 111272 transaction gas. Its native quote and
consumption are 90000. The test compares all 12 ordered raw mutations, both
balance changes, the positive 396 reward log, empty output, supplied native
gas, invocation identity and the measured final raw rows. It checks 14 raw
observations, 13 distinct keys, and 12 present observations. Each fresh port
has invocation sequence 0 and is dropped before the callback returns.

## Errors and disposal

For every probe position in each transcript, the test injects a read error at
each of the 14 raw observations. It also injects a corrupt count, a missing
first item, a preparation error, both native account read errors, and a factory
error. The existing direct H1 test retains the complete corruption and absence
matrix; this test checks their composition with the estimator.

Each error preserves its typed `SimulationError` through
`EstimateError::Callback`. The estimator makes no later probe and returns no
estimate. The failed native probe returns no completed outcome. Its observed
read prefix and invocation boundary must match the failure position. An
allocated port drops exactly once; a factory failure allocates no port. Each
failed estimate is followed immediately by a clean whole-estimate retry.

Fresh and reopened owners execute 572 estimate attempts, 1788 callbacks and
1774 allocated ports. After every callback, the test compares the complete
physical reader, all owner column names and logical rows, committed semantic
state, reader identity and original transaction. Reopen preserves the same
owner rows. These checks establish disposal for this finite simulation class;
they do not grant journal publication authority.

## Validation and limits

The required checks are the nonzero focused public estimation test, the full
EVM suite, affected package check and clippy, serial `rewrite-validate-fast`,
whitespace checks and frozen independent Sol medium review. The unchanged
runtime retains the accepted `23e859793` consensus suite (1526 tests), Rust ON
bridge build with 12 jobs and all 15 bridge tests. Actual dual-pin Go, C++
transcript reproduction and Python checks are recorded in the accepted
`b8a14fa44` oracle closeout. Run artifacts record results and source hashes.

This slice covers the three funded caps only. Underfunded and negative
estimation, other periods or shapes, nested calls, production routing,
publication, root parity, recovery and N4–N6 remain outside this acceptance.
