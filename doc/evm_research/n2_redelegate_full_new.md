# Full caller-source removal into a new destination

Base: `e4c22cd7f`. This slice admits only a one-member caller whose full
source principal moves to an absent destination pair while both validator stakes
remain positive. Current-period reward nodes must be absent on both sides.
Existing active-fork, post-fix distinct-validator, complete-history, zero-reward,
exact node-count and raw authentication requirements remain. Normal authenticated
failures keep their precedence. Longer swap-remove/append orders, existing current
nodes, reward-bearing cases and source-validator deletion remain excluded.

Caller d1 starts with1000 on31 and no32 membership. Other delegator a1 has1000
on each validator. Aggregate stakes2000/1000 become1000/2000; total3000 stays.
Caller order [31] becomes [32]. Configuration records chain666, genesis balances
d1=4000/a1=2000, supply6000, minimum100, maximum1000000 and zero rewards.
Transactions use period1, gas200000 and zero value/price. The complete actual
configuration, accounts, native facts and committed physical rows are exported.

The new [Go exporter](../../experiments/evm_feasibility/native_redelegate_full_new_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_new_reference.py)
use unchanged public/local pins and independent uninstrumented controls. The first
transaction succeeds with17 ordered native writes, one event and gas101912.
The next same-direction transaction reports Delegation does not exist with no
native writes or logs. Go reads15 rows cold and0 warm because its block cache
retains facts. Actual committed presence/bytes compare with controls. Separate
seed inspection supplies physical authentication rows absent from the Go action
read prefix. Old exporters and corpora remain unchanged.

The source serializer removes caller item1, reverse position31 and writes count0.
Destination serialization receives only the intermediate empty caller membership;
all delegation absence, reward and validator authority still comes from the
original snapshot. It reuses item1 and writes count1. One invocation-local trace
preserves every intermediate expectation and repeated operation. The kernel runs
on a clone with an empty account port. Neither effects nor semantic state publish
until both serializers succeed. Other callers use the original serializer wrapper.

[Tests](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_full_new_tests.rs)
compare exact ordered effects, event/output, kernel/semantic state, complete caller
and other-delegator membership/principal/cursor facts, both retained validators,
and final observed physical rows/nodes. Every one of17 cold and5 warm Rust reads
has independent corruption and reader-failure checks (44 failures), with no
state/sequence advancement and a poisoned session. Seven excluded semantic shapes
cover absent, longer, duplicate and wrong caller membership, incomplete history,
and either existing current node. A local serializer test demonstrates disposal
of source effects when destination composition fails; it is not an owner-level
late read injection claim. Committed snapshot/head remain unchanged.

The old full+new exclusion assertion changes because the product scope changes:
a valid one-member semantic shape is now admitted, but that old fixture has
inconsistent destination raw rows and must fail raw integrity. Validator deletion
still fails scope checks. Existing normal failure, node/count, pool/index and
frame/API regression tests remain unchanged. A comment correction in the prior
zero-trace test explains that raw expectations follow intermediate writes within
each invocation; its behavior is unchanged.

Actual dual-pin recording/reproduction,35 targeted redelegation tests, consensus
check/Clippy/1483 tests, EVM package tests, Rust-enabled consensus bridge build
with12 jobs/all15 tests, serial workspace fast and whitespace all pass. First-run
logs and later bounded-node reruns are retained under
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
Independent frozen Sol medium review accepted all15 hashes without blocking
source findings (`full-new-review.md`). Final gate completion is recorded in
`full-new-gate-closeout.md`; the review's pending gate statements are resolved.

Sol medium implements directly; Astra medium accepted the named intermediate
membership/authentication contract. A fresh Sol medium reviews this semantic
profile independently. Route confirmation and results stay in the artifact
record. No production routing, fallback, upstream C++, storage-owner change,
public history, frame/API acceptance or broad gate is included. N1–N6 and
Milestone10 remain open. The next dependency is a bounded actual frame contract.
