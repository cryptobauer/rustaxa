# Full caller-source redelegation frames

Baseline: runtime `6b5228ad2`. This test/evidence derivative uses the existing
real EVM driver, journal and FinalChain session. Runtime is unchanged.

The [exporter](../../experiments/evm_feasibility/native_redelegate_full_source_frames_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_source_frames_reference.py)
capture eight actual cases from both unchanged Go pins: direct full swap-last,
direct full last-item, nested full, static full, parent revert full, two full
calls followed by parent revert, nested nonpayable and child funding79,999.
The selected source has d1/a1 principal1,000 each; destination has d1 principal
1,000. The two source choices preserve complete known semantic ordering, while
raw input is an actual touched-row map from first-read plus separate seed reads,
with explicit synthetic total-vote300/principal3,000. This is not a complete
physical native snapshot, and no unchanged other-delegator physical-row claim is
made. Both pins reproduce exactly; source/support/seed/output/diagnostic identities
are in the manifest.

Actual direct cases use101,912 transaction gas, nested102,678, static102,676.
The two-call parent revert uses183,422. Both children receive80,000 gas: first
succeeds and emits17 ordered writes; second reports Delegation does not exist
and emits no effects. The second CALL boolean is zero and the wrapper reverts
that exact result. Source deletion and caller membership removal remain in final
raw rows after parent revert; final logs remove the first child's native log.
Single last-item removal emits15 writes. Nonpayability/underfunding retain their
actual quote/funding/value/account and unused-action-gas behavior.

The [integration test](../../rust/crates/rustaxa-evm/tests/native_session_reference/redelegate_full_source_frames.rs)
compares all transaction gas/output/errors/logs/accounts/raw rows and the whole
per-native-call vector: sequence, quote, funding, caller/depth/input/value/static,
status/output/logs and ordered mutations before journal reduction. Final per-key
plans are checked separately; the caller source key has an explicit Delete
operation after success. In the two-call case the first fact is OuterFrameReverted
and the second remains OwnFrameReverted, with exact failure and empty effects.
These dispositions are Rust internal contract assertions, not Go output fields.
Committed validator stakes remain exact after disposing each private session.
The corrected deep-copy forwarding observer keeps per-call setup and execution
reads separate; its harness checks aggregate read/write coverage.

Astra medium accepted the bounded capture/rollback contract. Sol medium implements
directly; independent Sol medium frozen final review accepted all16 hashes without
blocking findings; see `full-source-frames-review.md` in artifacts. Final reproduction,12frame+18API regressions, affected clippy and serial
workspace fast pass. Runtime unchanged: accepted full-source
ON bridge12/all15 remains applicable. No failing correction batch; review corrected a copied module comment from
absent to existing caller destination. Direct private/other-delegator semantic
snapshots are separate accepted adapter evidence. An added
explicit source-delete assertion was validated before freeze. Commands/full
outputs/exit codes use `full-source-frames-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. No routing failure or quota billing
inference is claimed.

Actual complete-seed historical simulation, estimation and direct traces for full
caller removal remain next. Full+new destination, source-validator deletion,
reward-bearing/zero/new-validator and historical same-validator successes remain
excluded. N1–N6, Milestone10, real-history and production acceptance remain open.
No upstream C++, storage-module, fallback, broad gate or push is authorized.
