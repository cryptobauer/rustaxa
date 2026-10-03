# Direct new-destination redelegation traces

Baseline: accepted estimation `1c480afa5`; runtime `de32bbc4f` is unchanged.
This derivative keeps the existing default structured trace runner, journal,
MixedNativeExecutionPort and complete synthetic H1 reader. Execution is H2;
source31 has aa1,000 and destination32 has bb1,000. Stored H1 head/cursor periods
are read exactly; they are not relabeled as H2 before execution.

The [exporter](../../experiments/evm_feasibility/native_redelegate_new_destination_trace_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_new_destination_trace_reference.py)
capture nine actual default TraceRunner sequences from both unchanged Go pins:
single300, prefix300/target200, two targets300/200, prefix300/target650 (remainder50
failure), missing-destination/partial, underfunded/partial, nonpayable-malformed/
partial, stale nonce0, and single50 below minimum. All other supplied nonces
increase from2^512 through each live sequence. Each sequence repeats exactly,
starts with the destination absent and keeps successful prefix effects live.
Actual direct precompile results have no opcode rows. Captured stdout diagnostics
and separate stderr are empty in this corpus and their identities are retained.
All committed physical rows and before/after state identity are exact.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_new_destination_trace.rs)
compares exact default JSON in72 runs across two semantic owner constructions,
two physical reader opens and two repetitions per sequence. The live session uses
begin_native_session(2,1), creates destination/current H2 rows in the first call,
and authenticates their retained overlay in later calls. Invocation IDs, periods,
caller/value/input and unchanged request nonces are checked. Eight injected hard
prefix/target failures dispose the port once; a successful mutating prefix is
confirmed before each target failure. Eight fresh retries start from committed
H1. Exact public validator/delegation/principal/reward assertions plus physical
row equality prove disposal, rather than only conserved total stake.

Astra medium accepted the H2 derivative contract before dependent tests. Sol
medium implements directly; independent Sol medium frozen review accepted all13 hashes without blocking
findings; report `new-destination-trace-review.md` in the artifact directory.
Actual dual-pin reproduction, all18 API tests and affected clippy pass.
Serial workspace fast passes. No failing correction batch. The accepted
ON bridge12/all15 from the runtime adapter remains applicable. Commands/full
outputs/exit codes use `new-destination-trace-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. No routing failure or billing
inference is claimed.

This covers direct synthetic default structured traces. Nested/delayed native,
OpenEthereum, RPC and real network windows remain separate. Full/zero/reward-
bearing/new-validator and historical same-validator successes remain excluded.
N1–N6, Milestone10 and production acceptance stay open. No fallback, upstream
C++ change, broad gate or push is authorized.
