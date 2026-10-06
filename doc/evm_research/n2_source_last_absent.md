# Bounded source-last genesis removal

Base `a112028` (signed source-last estimation accepted). This new profile uses
real genesis order [33,31,32], caller [33,31], other/global [33,31,32], no prefix,
and full1000 from31 to absent32 at block1. Both unchanged pinned Go revisions and
uninstrumented controls agree. Target gas is101912, native quote80000, 17 ordered
writes and15 Go reads. The repeat has normal missing-source failure, no writes
or logs. Actual source old node0 decrements3->2->1; source current1 is created
with count1. Destination old0 changes2->1; current1 is created with count2.
Caller becomes[33,32]; other/third memberships and rows remain unchanged.

The new read-only custody predicate admits source-last only at invocation/current
block1 with complete principal/same-validator/graph histories, exactly two
positive caller pairs, full source removal, positive retained source validator,
absent destination pair, distinct validators, post-fix/Magnolia/Ficus/pre-Aspen2,
and zero rewards. All old heads/cursors are0; old counts are3/2/3 and indices0;
all three current nodes are absent. Source/retained caller mirrors are present
zero; destination mirror/cursor is absent. Semantic validator index maps are
absent-or-zero because real Rust genesis uses an empty map; exact graph indices
remain mandatory. Astra corrected its initial present-zero map requirement.
No genesis, shared loaded-current helper, kernel restoration or generic serializer
changes occur. Graph/ledger integrity errors propagate; topology nonmatches
return false. Existing raw authentication remains authoritative.

Removal uses original_members[..1], original before and one raw trace. Source
item2 goes31->absent->32, count2->1->2. Every repeated write remains ordered.
The target never writes retained item1/position33. No address/stake/VRF constants
are added to production logic. This is a test-only backend scope extension;
production routing, C++, storage owners, fallback and protocol remain excluded.

Five new tests bind direct kernel/staged cold success and warm failure to every
actual write/log/raw row and semantic graph/membership fact. Rust cold has17
unique authenticated keys and warm failure5. Corruption and reader errors at
every key give44 independent failures and44 fresh retries, checking unchanged
staged state, sequence, raw backing, committed snapshot/head, cleared preparation
and poisoned reuse. Wrong intermediate membership discards local serialization.
An explicit-zero validator index map matches genesis omission with the same ordered parity. Structural exclusions cover node/count/index/head/mirror/pool/marker/history,
member order and invocation bounds. Existing source-current both-absent denial
becomes an explicit wrong-old-count exclusion; new actual-corpus success supplies
the intended product scope extension. Existing source-first semantic-order-only
shape7 now requires RawIntegrity with full rollback against unchanged physical
positions. No old success corpus changes or weaker mismatch expectations occur.

Tier2 requires actual dual-pin/control reproduction, all redelegation regressions,
consensus check/Clippy/package tests, EVM package, Rust-enabled bridge build12/all15,
serial fast/pre-commit, whitespace and frozen independent Sol-medium review.
Full first-run output and corrections are retained under
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`
with prefix `source-last-absent-`. Acceptance requires saved gate results.
Frames, signed history, historical API/estimate/trace, nonzero rewards, later
blocks, broader membership, source-validator removal and general repair are
separate gaps. No push is authorized.
