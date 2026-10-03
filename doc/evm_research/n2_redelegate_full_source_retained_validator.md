# Full caller-source redelegation with retained validator

Baseline: `ca84f142d`; accepted partial runtime is `de32bbc4f`.
The adapter now admits full removal of caller principal only when the destination
caller pair already exists and is positive, and source aggregate stake remains
strictly positive. All active/post-fix/distinct-validator, complete-history,
principal, zero-reward/node-count and membership authentication stays in place.
Full plus new destination and source-validator deletion remain explicit scope
errors. Normal authenticated failures still precede scope rejection.

The existing kernel runs on a clone with an empty account port. Existing source
and destination serializers produce ordered effects using the same invocation
trace. The source delegation/cursor and caller membership are removed; source
validator/order, other-delegator principal/cursor and total principal remain.
The source iterable removal authenticates count, source item/position and any
last item/position before effects escape. No semantic state advances until both
serializers complete. No upstream C++, storage-module or production route changes.

The [exporter](../../experiments/evm_feasibility/native_redelegate_full_source_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_source_reference.py)
capture two actual complete StateTransition executions from both unchanged Go
pins plus uninstrumented controls. Caller d1 has1,000 on31 and32; a1 has1,000 on
the selected source. Caller moves its full1,000. Source31 exercises swap-last
removal; source32 exercises last-item removal. Both retain source1,000,
destination2,000 and total3,000. Actual native writes number17 and15; each cold
Go action reads15 rows. Transaction gas is101,912 in both cases. Account balances
remain unchanged, and the caller nonce increases normally. A same-direction
follow-up actually reports Delegation does not exist with zero writes/logs and
Go cached reads0; its nonce advances to2. This is not a second full success.

Seed inspection exports ten actual iterable rows separately before execution;
they cover stronger Rust authentication rows not read by the Go action. Native
observation starts only afterwards. Final committed raw presence/bytes cover all
seed/read/write keys. Complete committed physical rows compare exactly to each
uninstrumented control, independently of observed key coverage. The manifest pins
source, support, observer targets, harness, control and output identities.

The [tests](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_full_source_tests.rs)
compare pending and synthetic historical adapter sessions against both actual shapes, exact ordered
writes/deletes, logs/output, semantic membership/delegation/cursors, validator
order and final Go physical presence/bytes/reward nodes. Every fresh Rust read
(17 swap-last/16 last-item) is independently corrupted and fails, then receives
an injected reader error; no effects/sequence/state advance escapes. A fresh
same-direction failure authenticates five rows without resetting retained state.
Existing validator-deletion exclusion tests remain. Full+new destination has a
new explicit exclusion test.

Corrections: the initial touched execution vector lacked two authentication rows;
export separate actual seed inspection instead of weakening authentication.
The first follow-up assertion used Insufficient delegation; capture the actual
Go follow-up and use Delegation does not exist. First failures remain recorded.
The historical helper installs a private synthetic snapshot/header; it does not
claim public finalization or actual DryRunner API parity.
All28 targeted redelegation tests,11frame+18API regressions, check/clippy and
actual dual-pin/control reproduction pass. ON bridge build12/all15 tests and serial workspace fast pass. Commands/full outputs/exit codes use
`full-source-` in `/home/fry/artifacts/evm-redelegate-2026-10-03/`.

Sol medium leads directly; Luna medium mapped this scope; Astra medium accepted
the bounded contract. Independent Astra medium frozen final review accepted all11 hashes without
blocking findings; see `full-source-review.md` in the artifact directory.
All routes ran without failure. No billing inference is made. Actual full-source
frames and historical API composition are next. Reward-bearing/zero/new-validator,
source-validator deletion/full+new-destination and historical same-validator
success remain excluded. N1–N6, Milestone10 and real-history/production acceptance
remain open. No push or broad gate is authorized.
