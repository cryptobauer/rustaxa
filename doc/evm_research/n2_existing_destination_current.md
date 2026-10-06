# Bounded existing-destination current-node removal

Oracle preparation base `61d7b25`, accepted local commit `4ec89b7c1`.
The bounded runtime correction uses this accepted oracle. New producer
`native_redelegate_existing_current_reference.{go,py}` and separate fixture
`fixtures/native_redelegate_existing_current` preserve all earlier corpora and
observer/support code. Both unchanged Go pins and uninstrumented controls agree.
Frozen live-cache copies use immutable committed genesis0 fallback and preserve
tombstones; inspection does not warm execution caches. Complete committed rows
and account/fact transitions compare to controls independently of observer coverage.

Real genesis validators [31,32] each have caller d1 and owner a1 stake1000.
Genesis balances d1=5000/a1=3000, maximum supply8000, principal4000; after staking
balances d1=3000/a1=1000/native=4000. Chain666, minimum100, maximum1000000,
threshold100/step10/delay1, zero yield, Magnolia/Ficus/Cornus/fix/Aspen1 at0 and
Aspen2 disabled remain explicit in the complete configuration manifest.
At block1, actual partial300 from31 to existing32 at nonce0 creates BOTH current
nodes count2/index0 and caller cursors/heads1. Old nodes0 each retain count1.
Caller pairs become700/1300, source/destination totals1700/2300.

The nonce1 full700 target uses101912 gas, native quote80000, 15 ordered writes
and2 backend reads in the warm Go cache. It deletes the source caller pair,
changes caller membership [31,32] to [32], and restores SOURCE current count
2->1->2. Destination count changes2->1->3. Caller destination stake2000,
aggregate totals1000/3000 and totalprincipal4000. Owner pairs, old nodes and
absent33 facts stay unchanged. Target logs contain700; prefix logs contain300.
Nonce2 repeat returns normal missing-source failure, gas101912, no logs/writes,
no native-state change. Each admitted transaction advances normal caller nonce;
account balances remain unchanged. Final complete state has111 rows. Prefix
has12 writes/14 backend reads. Warm Go read counts are observations, not Rust
cache parity requirements.

Fresh Astra medium source contract predicted exactly source2/destination3 and
was confirmed by this real trace. Rust before this correction retained source1;
the new direct kernel and staged parity tests match source2/destination3. The old
absent-destination helper rejects retained==destination and remains unchanged.
A separate read-only companion captures the source node for exactly two positive
caller pairs ordered[source,destination], full positive source removal, retained
source validator, both current nodes count2/index0, matching current heads and
cursors, zero mirrors/pools, complete principal/history/graph and valid ledger,
post-fix/Magnolia/Ficus/pre-Aspen2. The kernel restores only source after removal and before destination work;
existing destination write_cursor already produces3. Full/existing staged calls
with a present source current node use the same companion eligibility. Other
current-source shapes return explicit unsupported scope; both-current-absent
existing-destination paths keep their prior behavior. Do not repair destination to2 or broaden
source-last, extra membership, reward, absent-node or later-state profiles.

The previous order-only source-last candidate is deferred: source review found
that the accepted full-source fixture already covers source-last removal.
A reordered identity adds no new semantic branch. It is not implemented.

Oracle acceptance passed dual-pin/control recording and reproduction, Python
compile, serial fast validation, whitespace and fresh frozen independent Sol
medium review (eight hashes). Four new Rust tests cover direct kernel, cold/warm
sessions and normal missing-source repeat, all ordered writes/deletes/logs and
frozen final rows; source cursor deletion and destination cursor1/count3; full
committed owner isolation. Prefix14 and cold/warm target15 unique Rust keys give
88 independent corruption/reader failures and88 fresh retries. Each failure
preserves staged state, sequence, raw backing and committed owner, clears
preparation, and poisons reuse. A direct serializer test fails after the local
source decrement/restoration and destination decrement, proving no raw, semantic
or sequence effects escape. Twenty-five predicate exclusions preserve supported
bounds. All57 redelegation regressions pass.

Initial compile failure (typed gas assertion) and invalid test mutation (count1
below two live references) remain saved; typed gas and a valid count4 exclusion
correct them without changing product expectations or old tests.
Runtime acceptance separately requires direct kernel and cold/warm
ordered parity, every authentication-key corruption/reader failure plus fresh
retry and isolation, predicate exclusions, package/check/Clippy regressions,
Rust-enabled bridge build12/all15 and serial fast, followed by fresh frozen
independent review. Gate results and frozen independent acceptance are saved separately from oracle
acceptance. No required runtime gate is implied solely by the oracle review.
Evidence is saved under
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`
with `existing-current-` prefixes and `existing-current-contract.md`.
No production route, storage/upstream C++ change, fallback, supplied-data change,
broad differential gate or push is authorized. Frames and historical APIs remain
separate gaps after bounded runtime acceptance.
