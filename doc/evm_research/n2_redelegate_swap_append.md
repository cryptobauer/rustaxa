# Source-first two-member removal and append

Base: `e289d9bda`. This staged extension keeps one-member full+new admission
and adds exactly two distinct positive caller members with source first and the
other member distinct from destination. No source-last or longer list is admitted.
The intermediate membership is the borrowed original tail, empty or one member,
after source swap removal. Destination serialization uses it in the same raw
trace; original snapshot retains delegation absence and reward authority. No
allocation, public type or kernel change is added by this membership derivation.

Actual profile: caller d1 has1000 on31/33, none32; a1 has1000 on all3 validators.
Caller order[31,33] becomes[33,32]; stakes2000/1000/2000 become1000/2000/2000,
total5000. Both validators remain, all current affected nodes are initially absent,
all indices/pools zero. Genesis balances5000/3000, supply8000, chain666,
threshold100/step10/minimum100/maximum1000000, active fix/Magnolia/Ficus/Cornus0,
Aspen part1 at0/part2 disabled. VRF44/55/66, ownera1, commission100 are pinned.

New [Go exporter](../../experiments/evm_feasibility/native_redelegate_swap_append_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_swap_append_reference.py)
execute separate instrumented and uninstrumented StateTransition controls on both
unchanged pins. Full config, accounts, native facts, committed rows, seed inspected
memberships for both owners/all validators and untouched third-validator rows are
captured. Original corpora/exporters stay unchanged. Actual success uses19 ordered
writes,16 Go reads, gas101912 and one1000 event. Same-direction follow-up reports
Delegation does not exist, no writes/logs and0 cached Go reads. Rust read-count
parity with Go is not claimed. Controls compare complete committed rows and facts.

Source moves reverse position33 from2 to1, replaces caller item1 with33, deletes
source reverse position and item2, then count2->1. Destination puts32 into freed
item2, adds reverse position32=2 and count1->2. Raw mutation expectations follow
every intermediate operation. Final item1=33/item2=32/count2, source reverse absent
and item3 absent; global validator order remains31/32/33. Old source node0 count
3->2->1/current1 count1; destination old0 count2->1/current1 count2. Third node0
count3 and its head/cursors remain unchanged; current third node1 stays absent.

[Tests](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_swap_append_tests.rs)
compare exact19 ordered writes/log/output, kernel clone with empty account port,
semantic principal/membership/cursors of both owners and all3 validators, final
physical presence/bytes and live reward nodes. Every19 cold and5 warm Rust key has
independent corruption/reader failure (48 failures), exact state/sequence isolation,
cleared prepare/poison and unchanged committed snapshot/head. Late read suffix is
explicitly bound to moved position33 and items1/2 after local source effects.
Ten exclusions cover missing/wrong/duplicate/longer/source-last orders, missing
retained pair, incomplete history and either affected current node. A direct
incorrect-membership helper failure drops partial local source/destination effects.

The first new harness still expected the old one-member/3000 facts and rejected
its fresh output. Corrected new-profile validation checks5000 and both complete
orders/other/third facts; first failure log remains. Actual record/reproduction and
all4 new targeted tests pass. Required consensus/EVM/redelegate/check/Clippy,
ON bridge12/all15, serial fast/whitespace passed. Independent Sol medium accepted
all11 frozen hashes with no blocking findings; final documentation status is
recorded separately in `swap-append-closeout.json`.

Artifacts: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`,
prefix `swap-append-`. Fresh Astra medium accepted the named swap/reused-slot
contract; fresh Sol medium review route is confirmed and accepted. Sol medium implements.
No frame/API/history/current-node/reward extension, real-network checkpoint,
production/fallback/C++/storage owner change or broad gate is accepted.
N1–N6/Milestone10 remain open.
