# Direct full caller-source structured traces

Baseline: accepted estimation `ca9cea696`; runtime `6b5228ad2` is unchanged.
The accepted complete4,000 H1 seed has aa/bb1,000 on both validators31/32.
Execution is H2 from unchanged stored H1 heads/cursors, through one live native
session and journal per sequence. Public H1 finalization occurs once; semantic
owner restart loads H1 without refinalization.

The [exporter](../../experiments/evm_feasibility/native_redelegate_full_source_trace_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_source_trace_reference.py)
capture nine actual default TraceRunner sequences from both unchanged Go pins:
full swap-last, full last-item, full prefix then missing delegation, two full
targets then missing delegation, full prefix then reverse partial300, missing
destination then full, low gas then full, nonpayable malformed then full, and
stale nonce0. Other supplied nonces increase from2^512 across each live sequence.
Each sequence repeats exactly. Exact physical committed rows and before/after
identity stay unchanged. Direct native results have no opcode rows; stdout
and separate stderr identities are retained. Reverse300 recreates the removed
caller pair through accepted partial-new-destination behavior; it does not
extend the excluded full+new-destination branch.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_full_source_trace.rs)
compares exact default JSON in72 runs across two semantic owner constructions,
two physical reader opens and two repetitions. Invocation IDs, H2/H1 periods,
caller/value/input and supplied nonces are exact. Eight injected hard prefix
or target failures dispose the native port once; target failure follows a
confirmed successful full prefix. Eight fresh retries match the actual trace.
Committed public validator/delegation/principal/reward assertions and exact
physical rows prove disposal separately from total principal4,000.

Astra medium accepted the bounded H2 contract. Sol medium implements directly;
independent Sol medium corrected frozen review accepted all12 hashes; see
`full-source-trace-review.md` in artifacts. Actual dual-pin reproduction,
all21 API tests, affected clippy and serial workspace fast pass. No producer correction
batch apart from independent review correction below. Runtime unchanged: accepted ON bridge12/all15 remains applicable.
Independent review required retained outcome witnesses: full-prefix pair/position
Delete and count1; reverse recreation expects absent/tombstone pair, principals
300/1,700 at cursor2 and appended membership/count2. Normal runs, target hard
failure and fresh retries now check those internal effects. First rejected review
and closure type-inference compile failure are retained. Corrected tests pass;
corrected clippy/fast pass.
Commands/full output/exit records use `full-source-trace-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. No routing failure or billing
inference is claimed.

This is direct synthetic default structured trace evidence. Nested/delayed,
OpenEthereum/RPC and real network windows remain separate. Source-validator
deletion, full+new destination, zero/reward/new-validator and historical same-
validator successes remain excluded. N1–N6/Milestone10 and production acceptance
stay open. No fallback, broad gate, upstream C++ change or push is authorized.
