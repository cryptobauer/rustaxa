# Direct zero-success structured traces

Accepted on 2026-10-06 after estimation `aa6ad522b`. The sibling
[Go exporter](../../experiments/evm_feasibility/native_redelegate_zero_trace_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_zero_trace_reference.py)
execute actual default TraceRunner twice on both unchanged pins. Four sequences
cover single zero, a partial 300 prefix then zero, two zero targets, and stale
nonce. Complete H1 seed and before/after physical snapshots match. Trace executes
H2 over H1, retaining supplied nonces 2^512 plus sequence index, or stale zero.
DryRunner nonce substitution does not apply. Per-call capture buffers, separate
diagnostics and manifest identities remain explicit. Old corpora are unchanged.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_zero_trace.rs)
runs 32 normal fresh sequences, eight injected failures and eight clean retries
across semantic restart and independent concrete reader reopen. One live port and
journal owns each sequence. Checks cover whole structured target JSON, selected
identity, direct empty opcode rows, context, invocation order, nonces and disposal.
Prefix effects prove caller pairs become 700/1300; zero retains those principals.
Before the next invocation, including injected target failure, both live journal
pair bytes match prior effects. Unprefixed zeros retain 1000/1000. Failure tests
check stage/index, prefix completion/effects, single disposal, cleared outcome
buffers and fresh retry. Exact physical bytes, committed head 1, both stakes,
ordered delegation rows and zero pending rewards stay unchanged.

Corrections: convert NativeRawValue to bytes for the witness; distinguish
committed-base effect expectations from the live overlay; add head assertions
after normal runs, failures, retries and reader cycles. Failed runs and initial
review are retained. No existing test changed. Actual dual-pin reproduction,
package check/tests/Clippy and serial fast passed. After the review correction,
check/Clippy, all 24 API tests and serial fast passed again. Independent Sol-medium
review accepted the frozen corrected source. Runtime/storage/C++ stay unchanged;
accepted prior ON bridge evidence applies.

First-run command/output/exit records, freezes and
[corrected review](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/zero-trace-head-review.md):
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
Lead and reviewer used confirmed Sol medium. Baseline 66%, target 56%; the saved
budget remains fixed. This closes the three bounded pre-Aspen-two zero API
derivatives. Other success profiles, Aspen-two failure, nested/OE/RPC traces,
real-network roots, production routing and Milestone 10/N1–N6 remain open.
