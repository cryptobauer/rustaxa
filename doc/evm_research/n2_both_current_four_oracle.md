# Both-current count4 oracle preparation

This new producer, harness and corpus measure a repeated owner partial and the
caller full transfer that follows it. The source contract is
`current-owner-repeat-contract.md` in the persistent run3 artifact directory.
Rust runtime70399811f has no exact count4 full route. This preparation adds no
runtime behavior. All prior corpora and shared support stay unchanged.

The exact accepted count3 genesis and configuration are reused as inputs:
validators31/32, common ownerA1, callerAA and owner each1000 at both; initial
liquid balances5000/3000, post-stake3000/1000, native4000, supply8000;
min100/max1000000, zero reward indices/pools/yield, active
Magnolia/Ficus/Cornus/Aspen1 and disabledAspen2/Cacti, locking2/3/7.

Five real period1 transactions run in one live transition: AA nonce0 partial300,
A1 nonce0 partial100, A1 nonce1 partial100 again, AA nonce1 full700, AA nonce2
missing-source repeat. Both exact Go pins and uninstrumented controls agree.
The repeated owner partial writes each current node3->2->4. Before the full:
caller700/1300, owner800/1200, totals1500/2500, both current nodes4, heads and all
four cursors1, both old0 rows absent. The full source writes4->3->4 and
destination4->3->5. Caller source disappears, destination becomes2000, and
membership becomes[32]; owner pairs800/1200 and membership[31,32] remain fixed.
Final totals800/3200 and principal4000. The repeat has the normal missing-source
error, no native writes/logs/raw/fact changes, and an admitted nonce advance.

Measured gas/write/backend-read counts are101912/12/14,101848/10/4,
101848/10/0,101912/15/2,101912/0/0. Backend read counts reflect Go caching;
they do not define Rust authentication. Complete committed rows total114.
Every call exports actual input/nonce, accounts, facts, frozen native rows,
errors/gas/output/logs, and ordered writes/reads. Raw/fact/account continuity,
last-write reduction, target owner preservation and final committed bytes are
checked. Passive cache copies use immutable genesis0 fallback and suppress
fallback for tombstones. No pending trie root is claimed.

Manifest hashes bind both pins, producer/harness, unchanged support, observer
insertion targets/helpers and output/control bytes. Instrumented and control
executions match complete account/native/fact/committed results after excluding
observation arrays and coverage-dependent convenience maps. The actual third
partial is the prerequisite for the dependent fourth result; source predictions
remain separate historical evidence.

Acceptance requires record/reproduction, Python compile, serial fast checks,
whitespace, and independent Sol medium frozen review. Unchanged runtime bridge
validation70399811f ONbuild12/all15 is inherited separately. A new exact count4
predicate, staged/kernel parity, raw authentication, exclusions and atomic
failure checks remain open. Existing count2/count3 guards and negative tests
must stay unchanged. No general count bound, production routing, protocol,
fallback, storage owner, supplied-data change, broad gate or push is authorized.
