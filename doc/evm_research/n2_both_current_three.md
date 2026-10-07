# Bounded full/existing both-current count3 restoration

This slice uses the accepted [actual count3 oracle](n2_both_current_three_oracle.md)
and fresh Astra medium placement/authentication contract. It adds a separate
read-only eligibility helper, shared by the staged boundary and semantic kernel.
The count2 helper, old tests/corpora and generic reward graph remain unchanged.

The helper requires exactly two validators in source/destination order with one
common owner distinct from caller and validators. Each has exactly caller and
owner positive principal rows; both accounts have ordered[source,destination]
memberships. The caller transfers its full source principal while owner source
stake remains positive. Histories and principal ledger are complete/valid, no
same-validator markers exist, graph period equals invocation, fix/Magnolia/Ficus
are active and Aspen2 is inactive. Both nodes are exactlycount3/index0 with
current heads and caller/owner cursors, present zero mirrors and absent-or-zero
pools. The graph has exactly those two live nodes. Other shapes returnNone;
required ledger/graph integrity failures remain hard errors.

The kernel captures the earlier-loaded source before any mutation, then uses
the existing restore placement after source removal and before destination work.
Destination retains its existing independently loaded increment. This preserves
actual Go source3->2->3 and destination3->2->4 rather than normalizing counts.
Serialization is unchanged. The new staged branch authenticates common owner
keys and both owner pairs/cursors, owner ordered membership, validator count and
forward slots, and old node0 absence. These extra reads apply only when the new
predicate matches. The semantic two-live-node bound does not claim enumeration
of every unreferenced physical orphan row.

Separate Rust tests construct actual caller300 and owner100 prefixes. Cold entry
uses their successful staged snapshot and the actual Go raw before view; warm
entry retains the real session. No node count is inserted as positive authority.
The fixture's complete114 Go rows retain immutable genesis0 physical owner
versions. Test raw readers obtain the previously unenumerated owner keys from
those exact retained versions with existing public storage codecs, and prove
no actual prefix/target write changes them. No owner bytes are guessed, no
accepted corpus is changed and no storage API is extended.

Five tests cover direct kernel parity; staged cold/warm exact ordered12/10/15
writes and events300/100/700; final source/destination totals900/3100 and cursor
counts3/4; caller membership[32], retained owner900/1100 and principal4000;
normal missing-source repeat with no native effects. Native quote/outcome gas
is80000. Exported physical native before/after/final rows match actual Go.
Go total gas101912/101848/101912 remains separately measured envelope authority.

Actual Rust authentication reads are14 at each prefix and29 at target, cold and
warm. All are distinct. Every key receives corruption and reader failure; each
fault preserves backing rows, staged snapshot, sequence and committed owner,
clears preparation and poisons reuse. Every fault has a fresh successful retry
with exact Go writes/logs/facts. This gives228 faults and228 retries. A separate test adds94 missing-required-row
and valid-wrong-value faults with94 fresh retries in cold/warm entry. It covers
owner identity, stake/cursor, membership and validator counts/slots/reverse
positions, positive old nodes and current node count/index. Missing zero pools
and removal of existing empty tombstones remain valid, so are not claimed as
missing-required-row failures. Total coverage is322 faults and322 retries. Forty shape
exclusions plus four fork exclusions cover count4/asymmetriccounts, owner cursor
history/missing identity, third identities, ordering, mirrors/pools/indices,
stale heads, incomplete history/graph, ledger mismatch, markers, old nodes and
period/fork mismatch. Explicit zero pools remain equivalent to absent pools.
Late destination serialization failure checks local source restoration without
publishing trace/backing/snapshot changes.

First failures and corrections are retained: direct kernel snapshot must come
from the period-advanced session constructor; storage codecs are public crate
exports rather than a public module; graph deletion helpers take borrowed keys;
the late witness expects restoredcount3. Added physical-fault test first needed
an explicit RLP import and then excluded valid missing zero pools and empty
tombstones from required-row failures; first outputs are retained. No existing test was weakened.

Required checks are five targets, all redelegation regressions, consensus and EVM
packages, consensus check/Clippy, Rust-enabled consensus bridge build12/all15,
serial rewrite-validate-fast, whitespace and independent frozen Sol medium review.
Capture records use both-current-three-runtime prefixes in the run3 directory.
No upstream C++, storage module, general reward graph, supplied-data or protocol
change is included. Public frames/H1 APIs/H2 trace, rewards/generalcounts,
real-window root parity and production routing remain separate. No fallback,
broad differential gate or push.
