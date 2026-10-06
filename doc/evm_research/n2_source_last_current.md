# Bounded source-last current-node removal

Base `b5d61a8f1`. This slice follows the accepted signed-H1 derivatives and
preserves all prior corpora. Its new profile changes genesis
validator order to [33,31,32]. Actual pinned Go ApplyGenesis follows this slice
order, so caller membership is [33,31] and other/global membership [33,31,32].
No loaded semantic order or native poststate is edited to create the success.
The run artifact `source-last-current-contract.md` records fresh Astra medium
source authority and the exact runtime bounds.

Real partial300 from31 to33 creates source current node1/count2. Real full700
from31 to absent caller32 leaves [33,32], validator stakes1000/1700/2300 and
total5000. Both Go revisions plus uninstrumented controls agree. Prefix has
12 writes/14 Go reads; target has 16 writes/8 Go reads. Both use total gas101912,
native action gas80000, empty successful output and one exact event. Repeat
fails with actual `Delegation does not exist`, no writes/logs and unchanged
native state. The archive-only observers preserve control flow and tombstones.
The manifest binds new source/configuration and unchanged support/observer pins.

`bounded_redelegate_loaded_source_node` now selects the distinct retained member
from either two-member order. It still returns the exact original source
NodeKey/Node only for complete positive two-pair, source-retained, absent
destination, zero-reward post-fix/Magnolia/Ficus/pre-Aspen2 authority. Every
head/cursor/current/count/index/mirror/pool/history/marker check remains. Missing
required ledger/graph integrity is an error; structural nonmatches return None.
The kernel restores that earlier-loaded node after source removal and before
destination work. It does not recompute its count.

The custody path borrows original_members[..1] for source-last intermediate
membership. It requires the shared current-node predicate to return Some for
this order. Source-last with both current nodes absent remains unsupported.
Existing source-first both-absent behavior stays covered. Original before remains
pair/reward/absence authority; both serializers use one raw trace with exact
intermediate expectations. Item2 is present31 -> absent ->32; caller count is
2->1->2; source node is c28002->c28001->c28002. Retained item1 and position33 are
byte-identical and receive no writes. Generic serializers need no code change.

Five new tests use independent real genesis/prefix for direct kernel and staged
cold/warm target parity. They compare every ordered write, log, output, all
selected frozen/final bytes, graph node/cursor facts, preserved other/third rows,
principal totals and committed head/snapshot. Rust prefix has14 authenticated
keys and each target16. Independent corruption and reader error at every key
produce92 hard failures and92 fresh valid retries. The caller's successful
prefix state and sequence remain intact on target failure; prepared is cleared,
session is poisoned and no owner publication occurs. Wrong intermediate
membership separately proves local source deletion/restoration is discarded.

The original source-first exclusion shape12 now expects semantic helper Some
for reversed order: this is the intended product scope change. Its unchanged
source-first physical source-position1 must fail RawIntegrity against semantic
source-last position2, with full session atomicity. Original source-first
success/oracle cases remain unchanged. New source-last exclusions retain broader
order, duplicates, markers, node/pool/index/history bounds and both-absent denial.

Runtime code changes are confined to the shared predicate and custody order/gate;
new tests and module registration exercise them. No C++, storage implementation,
fallback, protocol, supplied data or production route changes occur. Tier2 bounded
subsystem validation requires actual dual-pin/control reproduction, package
check/Clippy/tests, old redelegation regressions, EVM package tests, ON bridge
build12/all15, serial fast, whitespace and independent frozen review. Results
must be saved before local acceptance. No broad/differential gate is claimed.

Run evidence:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`,
prefix `source-last-current-`. Frames and historical APIs for this order are next
separate authority/gate steps. Nonzero rewards, destination-current, longer
orders, source-validator removal and general current-node repair remain open.
