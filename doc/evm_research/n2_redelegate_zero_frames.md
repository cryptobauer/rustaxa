# Actual zero-existing-pair frame composition

Baseline/runtime: accepted staged zero `8a6735c94`. This test/evidence derivative
uses existing SessionPort, EVM driver and journal; runtime/C++/storage are unchanged.
Separate [exporter](../../experiments/evm_feasibility/native_redelegate_zero_frames_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_zero_frames_reference.py)
select the original actual zero_before_aspen_two seed by name. Both validators
and caller pairs have1,000, total principal2,000, synthetic votes200 and native
balance2,000. Touched seed rows are not a complete physical checkpoint.

Eight actual cases execute twice from both unchanged pins: direct zero, nested
zero, STATICCALL zero, single parent revert, two zero calls then parent revert,
nested nonpayable, nested underfunded and direct Aspen-two zero. Per-call quote,
funding/caller/depth, errors/output, ordered reads/writes/logs and account views
are copied before reduction. Completed Run reads are cleared before later setup;
aggregate vectors must equal concatenated call vectors. Exact source/support/
seed/output/stdout/stderr identities are in the manifest; prior corpora stay exact.

Direct gas is101,784; nested/single-revert102,550 and static102,548. Both nested
zero calls actually succeed with80,000 quotes:183,294 total gas and12+10=22
ordered operations, two invocation logs and zero parent logs. Go's warm second
call has zero storage reads; Rust still authenticates its retained overlay.
Aspen-two zero retains its normal error with zero reads/writes/logs. Underfunded
Go Run is uncalled; Rust typed admission remains observed. Nonpayability and
outer value transfer keep their actual separate error/account behavior.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_session_reference/redelegate_zero_frames.rs)
compares all actual parent results, per-call effects before journal reduction,
quotes/funding/context and final per-key publication plan. Both positive caller
rows remain1,000 with cursor1; exact touched raw values/membership remain checked.
Per-call ordinary account views from the Rust journal match the actual Go views;
input/final nonce/balances also match. Native account effects are empty. Successful
zero-price/value balances stay unchanged while outer nonce advances. The
nonpayable nested case captures outer value transfer separately. Each success
in the two-call parent revert has internal OuterFrameReverted disposition;
both logs disappear while all22 raw operations survive. Committed per-validator
semantic stakes remain exact. Dispositions are Rust internal journal assertions,
not fields claimed in the Go output.

Luna medium mapped the frame seam. Astra medium accepted the bounded contract
and accepted all15 final frozen hashes without blocking findings; see
`zero-frame-review.md` in artifacts. Sol medium implements directly.
Both pinned actual repeated reproduction, all13 frame tests,21 API tests and
package clippy pass. Serial workspace fast passes. No failing correction
batch; per-call Rust account views were strengthened before freezing. Unchanged
runtime retains accepted ON bridge12/all15 from the preceding staged slice.
Evidence prefix `zero-frame-` under `/home/fry/artifacts/evm-redelegate-2026-10-03/`.
No routing failure or billing inference is claimed.

Measured account parity is now established for these exact synthetic frames;
it does not retrofit missing balance fields into the old staged corpus. Actual
public historical zero simulation, estimation and direct traces are next.
Zero+absent/zero pairs, full+new destination, source-validator deletion, rewards,
new validators and historical same-validator successes remain excluded.
N1–N6/Milestone10, real network and production acceptance stay open. No fallback,
broad gate or push is authorized.
