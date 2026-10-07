# Exact both-current count4 full transfer

This runtime slice uses the separately measured count4 oracle. The actual same
period sequence is callerAA partial300, ownerA1 partial100, owner partial100
again, then caller full700. The third operation already matches Rust without a
runtime change: both nodes become4 via earlier loaded3->persisted2->loaded4.
The full had no accepted staged route. It now has a separately named exact
count4 predicate and restores the earlier loaded source copy before destination
processing. Its source4->3->4 and destination4->3->5 match Go ordered writes.

The helper accepts exactly two ordered validators and two delegators, a common
retained validator owner distinct from caller/endpoints, four positive pairs,
full caller source principal with positive retained owner principal, current
heads and all four cursors, both nodes count4/index0, zero mirrors/pools, exactly
two live nodes, complete principal/same-validator histories and no corruption.
Post-fix Magnolia/Ficus and pre-Aspen2 are required. Stakes and owner identities
are dynamic; oracle constants are assertions, not runtime eligibility constants.
Missing/ineligible shapes return None; required graph or ledger integrity errors
are hard errors. Count2/count3 predicates, graph APIs and serializers retain
all previous behavior. No count>=4 route is added.

The staged route selects normal contract failures first. Only exact count4 full
success authenticates the extra common-owner identities, pairs, current cursors,
ordered validator/owner memberships and absent old0 rows. Its target has29
unique raw keys. The three prefixes have14,14,12 keys. Normal-failure ordering
and all other profile read streams stay unchanged. The kernel runs on a clone;
source/destination serialization completes before the session snapshot changes.
Hard errors abort the session without partial effects. The low-level kernel
still requires callers to discard failed staged state; no new frame rollback
policy or physical orphan enumeration is claimed.

New independent tests derive all three prefixes from actual oracle calls,
compare direct kernel and cold/warm staged ordered writes/logs/last bytes,
principal/membership/head/cursor/node facts, native80000gas and normal repeat.
Every prefix and target key gets corrupt bytes and a reader error, then a fresh
successful retry. Additional tests remove required rows or supply valid wrong
RLP/address/member bytes. Late destination serialization failure discards the
local restored source. Exact exclusions include unequal3/4 and4/3, higher count,
wrong identities/order/history/cursors/heads, extra nodes, reward indices/pools
and fork boundaries. Prior negative tests and corpora are unchanged.

Required gates are focused redelegation tests, consensus/EVM packages, checks,
Clippy, ON consensus bridge build with12 jobs/all15 tests, serial fast checks,
whitespace, and independent Sol medium frozen review. First test failure was an
invalid lower-count setup using restore_loaded_node; the new test preserves a
valid decremented graph instead. Initial format command lacked workspace --all;
corrected formatting is recorded in the run artifacts. No storage-owner change,
upstream C++, production routing, protocol/fallback, supplied-data change,
expensive broad gate or push is included. N1–N6 and cutover remain open.
