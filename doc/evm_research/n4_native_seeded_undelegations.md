# N4 seeded undelegation inverse evidence

## Scope

This bounded diagnostic extends the authenticated current-head DPoS inverse
using only the 290 validator and owner addresses already recovered by the
seeded delegation pass. It derives V1 and V2 undelegation iterable keys,
objects, and V2 last-ID cursors. Every live result must match the same complete
authenticated inventory byte-for-byte. The checked-in exact report is
[`n4_native_seeded_undelegation_coverage.json`](n4_native_seeded_undelegation_coverage.json).

The address sample is not a global delegator catalog. The result does not prove
semantic snapshot completeness, deleted or historical-key coverage, checkpoint
adoption, state publication, or production routing.

## Codec sources

The derived key order and value codecs match the existing Rust native custody
implementation in
[`native_session/custody.rs`](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody.rs)
and both actual Go fixture pins recorded by
[`native_cancel_custody/manifest.json`](../../experiments/evm_feasibility/fixtures/native_cancel_custody/manifest.json):

- public Go pin `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`
- local Go pin `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`
- byte-identical `undelegations.go` SHA-256
  `0712bb7dad0940be0ff2e03cc31373f03b61634dd17c3f1884d139068cf86aa2`
- byte-identical `iterable_map.go` SHA-256
  `564727217952482fe59c2b6061849023896f1c55d9f20f5ec281998ca47ff0d3`

The extension covers V1 `[3,1] || delegator` validator indexes and
`[3,0] || validator || delegator` objects. It also covers V2
`[3,2] || delegator` validator indexes, nested
`[3,3] || delegator || validator` ID indexes,
`[3,0] || delegator || validator || id_le` objects, and
`[3,4] || delegator` last-ID rows. Iterable items and reverse keys use the
observed aliased discriminator `2`; V2 IDs and last IDs are exact eight-byte
little-endian values. V1 objects are canonical RLP `[amount,block]`; V2 objects
are canonical RLP `[[amount,block],id]`, with the embedded ID required to equal
the key ID. Last IDs are monotonic allocation cursors and are not required to
equal the greatest current live ID.

## Bounded read-only run

The worker ran exactly once against the independent qualified copy:

```bash
timeout 300s env \
  CARGO_TARGET_DIR=/tmp/rustaxa-n4-native-inverse-target \
  cargo run --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin native_inverse_coverage -- \
  /tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy \
  /tmp/n4_native_seeded_undelegation_coverage_20260914.json
```

The process completed successfully in 23.8 seconds. The application database
and `ConcreteCheckpointReaders` opened read-only. The tool accepts only the
canonical qualified-copy path, rejects the original snapshot, and creates its
output outside both snapshot paths with `create_new`.

The existing inventory limits remained 50,000 trie nodes, 50,000 live leaves,
and 32 MiB of live values. The extension independently caps authenticated seed
addresses, aggregate V1 entries, aggregate V2 validator groups, and aggregate
V2 IDs at 4,096 each. Those ceilings allow at most 49,152 derived point reads;
the fixed 290-address input allows at most 37,734 and the observed run used
1,017.

The report selected FinalChain head 25,706,949, state root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`,
and DPoS storage root
`c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504`.
The authenticated inventory remained 23,278 rows and 259,077 value bytes with
SHA-256 `a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c`.
The original coverage object remained unchanged at 2,065 matched rows and
35,694 bytes.

| Result | Before extension | After extension |
| --- | ---: | ---: |
| Known exact live rows | 2,065 | 2,282 |
| Known exact value bytes | 35,694 | 37,971 |
| Unexplained current live rows | 21,213 | 20,996 |
| Unexplained current live value bytes | 223,383 | 221,106 |

The 217 newly explained rows contain 2,277 exact value bytes:

| Family | Live rows |
| --- | ---: |
| V1 validator counts | 60 |
| V1 validator items / reverse positions / objects | 45 / 45 / 45 |
| V2 last IDs | 5 |
| V2 validator counts | 5 |
| V2 validator items / reverse positions | 2 / 2 |
| V2 ID counts / items / reverse positions / objects | 2 / 2 / 2 / 2 |

Of the 60 live V1 count rows, 47 were zero and 13 enumerated 45 current
objects. The other 230 V1 count reads returned `HistoryUnavailable`. Of the
five live V2 count rows, four were zero and one enumerated two validator groups
with one ID each. The other 285 V2 count reads returned
`HistoryUnavailable`. Five V2 last-ID rows were live, with values
`[6,1,22,1,4]`; the other 285 returned `HistoryUnavailable`. Unavailable reads
remain typed unavailable observations and are never converted to absent rows or
zero counts.

The observed source run returned an exact current-live row and byte partition.
Its residual current-live path SHA-256 is
`54809f424fe27f9679395fb72189d6a3cac20f8fc5dd2a2ca6b8eca712a2b314`;
the residual path/value SHA-256 is
`b53d39a3528f100e922ce9dda13367d3b6f6edff57c06fb716874ca3ea84f5f8`.
Independent review reconstructed all 217 new logical preimages and canonical
values, verified their disjointness from the base inverse, and checked the
reported totals. It did not reopen the database or independently recompute the
residual digest from physical storage.

The aggregate SHA-256 over `native_inverse.rs`, `native_inverse_coverage.rs`,
and the adjacent seeded-undelegation helper is
`593f94c3ffb3bdd310f32272b61f32967c79ef1637007ca9b797f010b9c094f2`.
The pretty-printed exact report SHA-256 is
`4eec7a3c2bf71c440869efef866cf206768cc4d2e2593059471d414d6630e0d6`.

Focused validation passed eight binary tests covering actual-Go V1/V2 bytes,
canonical zero and nonzero integers, malformed RLP, forward/reverse mismatch,
V2 key/object ID mismatch, typed unavailable and tombstone handling,
cap-before-item rejection, base overlap, and inventory-byte mismatch.
`cargo fmt --check`, focused `cargo clippy -- -D warnings`, and
`git diff --check` also passed. Independent source review approved the bounded
operation before execution and independently approved the resulting artifact.

## Remaining reconstruction boundary

This extension classifies current live rows reachable from the same 290 seeds.
It does not discover undelegators outside that set. The remaining 20,996 current
live rows may include other delegation and undelegation maps, non-head reward
graph nodes and cursors, or other unresolved native families; hashed paths alone
do not establish their type. Deleted and all-ever historical keys remain outside
the current-live inventory and require separate evidence.
