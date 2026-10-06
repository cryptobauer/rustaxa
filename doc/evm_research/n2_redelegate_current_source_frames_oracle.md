# Actual source-current two-transaction frame oracle

Base f550858c6. This preparation records actual Go frame behavior only; Rust
frame parity remains open. Astra medium accepts the named seven-case design in
current-source-frames-contract.md. No runtime/bridge/old corpus changes occur.

The new native_redelegate_current_source_frames_reference Go/Python units export
and reproduce seven fresh cases on both unchanged pins, plus seven independently
executed controls per pin. Go runs one real EVM.Main prefix300 source31->existing33
and then target700 source31->absent32 on the same state/native cache. Initial raw
seed is the accepted current-source initial frozen42-row view plus explicit votes
500/principal5000; no prefix-after rows are injected. Manifest binds seed, support,
harness, exporter, pins, full context, outputs and controls. Frame context is
period1/price0/value0 except nonpayable1, gas200000/block1000000; aa/d1 balance1m
nonce1/nativebalance5000; Magnolia/Ficus/Cornus/fix0, Aspen disabled/supply1b.
This differs from the full StateTransition seed producer context.

Go EVM.Main returns cumulative logs. Each transaction exports its prior length,
full cumulative vector and exact suffix. Copy-only capture checks unchanged
prefix logs, result/state equality and zero refund before/after. No checkpoint,
commit, intermediate root, cache clear, log clear or refund reset occurs. Selected
raw maps use GetRawState while RawStorageDirty is live, with complete initial keys
plus all dirty keys independently enumerated in controls. These are dirty frame
rows, not committed trie or complete physical checkpoint facts. All bytes/maps/
logs/observations are copied before subsequent execution or rollback.

Prefix always succeeds, gas101912/12writes/sourcecurrentcount2. Target measured:
direct101912, nested102678, static102676, parentrevert102678, two-callparentrevert
183422, nonpayable29378, underfunded22678. First five emit18writes; sourcecurrent
count2->1->2, sourcepair deletion/moved33/reusedslot2 survive target parent revert.
Two-call second target normally fails with no writes. Nonpayable/underfunded0writes.
Success target suffix has one700event; parent revert target suffix empty and
cumulative vector keeps the one300prefixevent. Both Main calls keep refund0.
Direct final d1nonce3/aa1; nested final d1nonce2/aa2/native1. Nonpayable final
balancesaa999999/d11000001/native5000; all other balances unchanged.

Observer records every quote/funding/depth/caller/input/value and per-call
route_staticcall (actual wrapper opcode route; no Go native read_only flag is
exposed), setup versus Run
reads, output/error/nativeevent and ordered effects before parent rollback. It
resets vectors, never native state. Controls use direct EVMStateStorage and actual
native registration without forwarding/read/write observers. Comparison includes
all exported outer errors/gas/output, transaction/cumulative logs, accounts,
selected raw continuity and final dirty rows; only absent observer fields excluded.
Harness rejects missing cases/calls, per-call aggregation drift, log/refund/boundary
changes, changed12/18write shape, source-node stream drift or final-write mismatch.

First Go compile used wrong log/error types and an unused import. Corrected to
pinned vm.LogRecord/util.ErrorString; first failure output remains in artifacts.
Independent review requested the missing per-call route field; added with
explicit source-derived route labeling, prefix false/target wrapper route.
Corrected actual dual-pin/control record and reproduction/Pythoncompile pass;
serialfast/whitespace and independent frozen Sol review pass. All16corrected
file hashes accepted. Rust frame comparison has separate frozen review and final gates;
it is not accepted by the oracle review alone. Runtime ONbridge12/all15
from f550858c6 remains valid because runtime is unchanged. Rust derivative must
use one public session/period sequence across settled prefix and new targetjournal,
compare target suffix and cumulative receipt logs, and check cumulative last-write
witness before each quote plus every intermediate ordered operation. No private
snapshot injection or broader scope is accepted. N1–N6/Milestone10 remain open.

Records current-source-frames- under
/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/.
Sol medium lead and same-profile independent Sol medium review; Astra medium
contract. Routes confirmed earlier; no routing failure/billing inference/push.
