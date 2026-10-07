# Both-current count3 oracle preparation

This separate producer/harness/corpus measures the named count3 gap. Runtime
072fc39fb still accepts only the earlier count2 profile and rejects this entry.
Fresh Astra medium source contract is both-current-three-contract.md in the run3
artifact directory. This preparation does not add Rust behavior or API acceptance.
Prior corpora, shared support, storage owners and upstream C++ are unchanged.

Real genesis has exactly validators31,32 with common ownera1, callerAA and owner
each1000 at both. Balances before staking are caller5000/owner3000; after staking
caller3000/owner1000/native4000, supply8000/principal4000. The complete chain
configuration is exported, including min100/maxstake1000000, zero yield, active
Magnolia/Ficus/Cornus/Aspen1, disabledAspen2/Cacti and locking2/3/7. CallerAA is
set consistently in genesis, execution and observations, rather than inheriting
the earlier D1 fixture identity.

Four actual period1 transactions run through the same live transition: caller
nonce0 partial30031->32; owner nonce0 partial10031->32; caller nonce1 full700
31->existing32; caller nonce2 repeat with normal missing-delegation failure.
Before the target caller700/1300, owner900/1100, totals1600/2400 and both current
node1 count3 have their head and both cursors. Both old node0 rows are absent.
Target sourcecount3->2->3 and destinationcount3->2->4 are measured in ordered
writes. Caller source disappears, destination becomes2000/cursor1, membership
becomes[32]; owner900/1100 and membership[31,32] remain fixed. Final validator
totals900/3100, principal4000. The normal repeat has no native writes/logs/raw
or semantic changes, with ordinary admitted nonce advance.

Actual gas/write/read counts are101912/12/14,101848/10/4,101912/15/2 and
101912/0/0. These backend read counts reflect live Go caching; they are not new
Rust authentication counts. The owner amount100 has one fewer nonzero calldata
byte than300 under the Ficus tariff. The first new harness failed by iterating
a null normal-failure write list; it now treats null as empty for comparison
without rewriting actual output. A later strengthened assertion incorrectly
expected ownergas101900 and12writes. It was corrected to the measured101848/10;
the initial source prediction remains historical evidence. First failures,
corrections and output/exit codes are retained.

Observation copies the live native cache without reads, then uses a separate
wrapper plus immutable genesis0. Tombstones suppress genesis fallback, including
both deleted old nodes and the deleted caller source pair. All attempts export
caller, actual nonce/input, frozen raw presence/bytes, semantic facts, accounts,
errors/gas/output/logs and complete ordered reads/writes. Raw/fact/account
continuity is exact across the two prefixes and target. Last writes match live
frozen bytes and final committed bytes; complete final committed rows total114.
Both pins run with observers and without read/write instrumentation. Full
account/native/fact/complete committed outcomes match; only observation arrays
and coverage-dependent convenience maps are excluded. Manifest binds source,
configuration, support, archive insertion targets/helpers and both control/output
hashes. No pending trie root or historical Rust identity is claimed.

Preparation requires dual-pin/control record/reproduction, Python compile,
serialfast/whitespace and fresh independent Sol medium frozen review. Records
use both-current-three prefixes under the run3 artifact directory. Rust is
unchanged, so accepted runtime072fc39fbONbridgebuild12/all15 is inherited
separately. A separate exact count3 helper, kernel/staged parity and exclusions,
every-prefix/target authentication fault/retry, late local rollback and required
Rust/bridge checks remain open. Do not replace count2 with count>=2 or change
old tests/corpora. No general reward graph, production route, protocol/fallback,
supplied-data mutation, upstream exception, broad differential gate or push.
