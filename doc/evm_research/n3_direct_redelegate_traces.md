# Direct redelegation structured traces

Baseline: `aab6231da`. Test/evidence-only N3 slice through unchanged trace,
journal and native session owners. The accepted bounded contract is recorded
in trace-contract-review.md under the run artifacts. No nested, JavaScript,
OpenEthereum, delayed-query, RPC routing or real-history acceptance is claimed.

The new `native_redelegate_trace_reference.{go,py}` exports eight actual default
TraceRunner sequences from both unchanged Go pins. Concrete committed state is
H=1 from the accepted two-validator seed; execution is H=2, with no H=2 reward
finalization. One live native storage/EVM view retains prefix and target changes.
Prefixes are untraced; only target JSON rows are returned. Nonces increase from
2^512 across the entire sequence, except the explicit stale zero. No DryRunner
nonce substitution occurs. Repeated traces are identical and full committed
rows/root/account/native facts remain unchanged. The manifest retains all source,
support and output hashes, including separate stderr files (both empty).
Native stdout diagnostics are retained per case (also empty for these cases).

Cases: one partial 300; prefix300/target200; two partial targets300/200;
prefix300/target650; missing destination then partial; low gas then partial;
nonpayable malformed then partial; stale nonce. Successful300 gas is101,912;
200 gas is101,848. The target650 fails only after prefix300 leaves700 and the
requested remainder50 is below minimum100; its gas is101,912. Native underfunding
uses21,912, nonpayable21,272 and stale nonce returns the supplied200,000 gas.
All returns and direct-precompile structLogs are empty. Structured JSON exposes
failed flags, not native error strings; none are invented in the evidence.

The Rust test uses one live MixedNativeExecutionPort with session execution2/
parent1 per sequence, so current H=2 nodes are authenticated absent initially
and warm overlay rows/semantic cursors are retained after the prefix. Actual
JSON, selected identity, full request nonce/value/input, invocation order and
single port drop match. Two physical reader reopen cycles across two semantic
owner constructions, with two repetitions each, give64 oracle-matched runs.
Eight injected prefix0/target0 hard failures verify precise stage/index and a
single drop; the target injection confirms the prefix actually completed.
Eight fresh retries match Go. Public committed reads verify each validator and
aa delegation remains1,000 with zero pending rewards; exact physical rows remain
unchanged. CompleteSeedReader absence authority is only for this synthetic seed.

First-run full commands/outputs/exits have `trace-` prefixes under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Actual dual-pin record/reproduction,
all15 API integration tests and affected-package clippy pass. Serial workspace
fast passes, and independent Sol medium review accepted all12 frozen hashes
without blockers; trace-review.md records acceptance in the artifact directory.
There are no runtime/bridge/storage-module changes; previously accepted runtime
ON bridge12/all15 applies. No failing correction batch occurred. Contract review
corrected proposed target600 to650 before exporter implementation. Requested and
confirmed routes: Sol medium direct lead, Astra medium bounded trace contract,
Sol medium independent final review; no routing failures. N1–N6 and Milestone10
remain open, as do excluded reward/full/new-destination/zero success branches.
