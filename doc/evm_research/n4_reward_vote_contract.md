# Bounded reward vote-input contract — 2026-09-30

The user authorized bounded input qualification after the restored pair and
sender proofs, with no production routing, broad replay, complete-snapshot
publication or source-database writes. This contract was independently mapped by
Astra high because delayed eligibility and historical VRF rules are consequential.
It is a diagnostic contract; actual results belong in separate execution evidence.

## Exact periods

For reward head H=25,706,949, its previous certificate certifies P=25,706,948.
Legacy `VoteManager::validateVote` requests voter count, VRF key and total at
Q=P−1=25,706,947. `rewards::Stats::getBlockStats` requests the same total at Q.
The candidate mainnet delegation delay is five, so Go's DPoS delayed reader uses
concrete state D=Q−5=25,706,942. Slashing applies that single delay from Q;
it must not subtract it again from D. Configuration version selection occurs at
Q; delayed eligibility and jail expiry evaluate D. H−1 or H−2 concrete roots do
not substitute for D.

The tool must obtain D's authoritative finalized header/root from the copied
application database and authenticate it with `ConcreteCheckpointReaders`, whose
committed descriptor remains H. Missing header/root paths stop dependent probes.
The fixed mainnet H qualification and exact `PeriodData` hash remain guards.

## Reads and fail-closed gates

Exactly two global keys are inspected first at D:

- DPoS `0xfe`: total count at Keccak(`04`).
- Slashing `0xee`: jailed-address list at left-padded 32-byte `02`.

Require the authenticated jail list to be empty before any voter loop. Go's
historical total reader subtracts counts for every listed address; the existing
Rust complete-snapshot helper subtracts active jails only. This diagnostic does
not silently assume those behaviors are equivalent for a nonempty list.

Each of the 19 independently recovered certificate voters then needs three keys:
DPoS validator Keccak(`00 00 || address`), DPoS VRF Keccak(`00 04 || address`),
and slashing jail block Keccak(`00 || address`). The bound is **59 distinct
logical keys**, each with one physical read and one authenticated proof: at most
118 top-level read/proof calls, plus owner-internal path reads. No inventory or
additional jailed-address enumeration is allowed in this slice.

For each key, keep physical and authenticated outcomes separate. A Member proof
must equal the raw Present bytes. NonMember plus missing physical history or a
tombstone establishes diagnostic logical absence, with the physical label
retained. NonMember plus raw Present is an orphan conflict and fails. Proof
failure, raw I/O/corruption and mismatched identities are not absence.
No production `ConcreteCheckpointNativeStateRead` behavior is changed.

## Kernel and provenance boundaries

Reuse the consensus-owned validator decoder and stake vote-count helper through
a narrow point-fact facade. Never construct a sparse `DposSnapshot` and call it
complete. Enforce stake no greater than the candidate configured maximum before
using the helper; historical Go and Rust behavior outside that bound differs.

Recover voters from canonical certificate bytes with the existing Rust inspection
API. Feed independently derived counts/VRF keys/total to
`validate_canonical_pbft_vote`, with strict VRF verification and
`has_preverified_weight=false`. Only after derivation may the tool compare weights
with retained target `BlockStats`. Retained expected weights cannot feed verifier
inputs. Preserve the existing raw serialized-order mismatch independently.

Bind the exact checked-in mainnet configuration bytes and relevant policy fields.
Even a complete match qualifies the observations only under that candidate
policy; it does not verify the producer binary, producer configuration, global
native state, adoption, reward execution or final roots.

Reference anchors (source references, not verified producer identity):

- Legacy C++ `upstream-main` at `a0e85fe31eb03573cd92c165a5f81035cec9907e`:
  `libraries/core_libs/consensus/src/vote_manager/vote_manager.cpp`
  (`validateVote`) and `libraries/core_libs/consensus/src/rewards/rewards_stats.cpp`
  (`getBlockStats`).
- Immutable Go reference `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`:
  `taraxa/state/contracts/dpos/precompiled/{api.go,reader.go,validators.go}`,
  `taraxa/state/contracts/slashing/precompiled/reader.go`, and
  `taraxa/state/state_db_rocksdb/db.go`. Current local EVM checkout is descendant
  `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`; do not label it the immutable pin.
- Existing Rust `pbft_vote_validation.rs`, `final_chain.rs::dpos_vote_count`,
  native-session validator decoder, and
  `ConcreteCheckpointReaders::{storage_at,verify_storage_path_at}`.

Exact relevant candidate DPoS values at Q are threshold
`500000000000000000000000`, vote step `1000000000000000000000`, and maximum
stake `80000000000000000000000000`, with delegation delay five. Magnolia and
Cacti are active at D; nested validator decoding uses Magnolia, not Cornus.
Strict VRF verification is this diagnostic's stronger policy; it does not claim
the original producer verified every certificate vote strictly. Persisted DPoS
config overrides may exist in concrete CF `8`; they are not yet qualified by
these checked-in JSON facts. No automatic Aspen overwrite of those settings
was found in the referenced source.
