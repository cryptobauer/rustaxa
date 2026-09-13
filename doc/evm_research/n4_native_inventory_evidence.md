# N4 authenticated native live-inventory evidence

Status: qualified for the bounded live storage trie of the DPoS account at the
retained mainnet head. This evidence does not qualify deleted history, semantic
catalog completeness, checkpoint adoption, or production routing.

## Input and identity

The probe opened this independently copied state database through RocksDB's
read-only API:

```text
/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy/db/state_db
```

No command was run against `/tmp/snapshot-litenode`. The exact pinned current
identity was:

- period: `25,706,949`
- state root:
  `b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`
- account: `0x00000000000000000000000000000000000000fe`
- account storage root:
  `c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504`

`ConcreteCheckpointReaders` required the current identity to equal the durable
state descriptor and authenticated the account path. Its inventory traversal
then resolved every hashed child under the account storage root, reconstructed
and checked every child and root hash, selected each leaf value at the pinned
period, and rejected missing dependencies, tombstones referenced as live
leaves, duplicate paths, malformed nodes, and inconsistent inline or hash
hints.

## Bounds and result

The run supplied these ceilings:

| Resource | Ceiling | Observed |
| --- | ---: | ---: |
| Decoded hashed or embedded nodes | 50,000 | 31,164 |
| Live leaves returned | 50,000 | 23,278 |
| Exact selected value bytes returned | 33,554,432 | 259,077 |

Crossing any ceiling returns an error without an inventory. These limits bound
decoded nodes and retained output, rather than the encoded size of one RocksDB
row or every temporary canonical buffer.

Entries are ordered lexicographically by their 32-byte hashed trie path. The
evidence digest encodes each entry as:

```text
hashed_path[32] || value_length_be_u64[8] || exact_value[value_length]
```

The encodings are concatenated in entry order and hashed with Keccak-256. The
result is:

```text
8183fbc4a15113b0b6b85ce5603e2ea449fc43b12bb531bdb48a9596776bd93f
```

The ignored qualified-copy fixture fixes the counts and digest and also checks
three supplied DPoS logical keys:

| Name | Logical key | Hashed trie path | Authenticated head result |
| --- | --- | --- | --- |
| `minted_tokens` | `d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873` | `cfb49530ec7471be164aff02dcea660bf2a32850593ff494d42ed3dc2dbb35b0` | selected physical tombstone and authenticated non-member; absent from live inventory |
| `total_supply` | `ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc` | `5f662fcb4dddda6fa419171c1ad89e94d79c2485371f7e75435f9d7724189d2d` | live exact bytes `237465dd4fbad4693966174c` |
| `yield` | `d33e25809fcaa2b6900567812852539da8559dc8b76a7ce3fc5ddd77e8d19a69` | `a93e930762556e73ce5ea92c5387f25fddc1c0f45d5bbe24b40c7501fe1a1516` | live exact bytes `83016db8` |

The logical keys are the native layout's `keccak256([field])` values for fields
6, 7, and 8. Trie paths apply the storage trie's second Keccak-256. The fixture
does not infer names for any other path.

## Reproduction

Run the bounded ignored fixture with the recorded copy path:

```bash
RUSTAXA_QUALIFIED_STATE_DB=/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy/db/state_db \
CARGO_TARGET_DIR=/tmp/rustaxa-bootstrap-recovery-target \
  cargo test --locked -p rustaxa-storage \
  --manifest-path rust/Cargo.toml \
  qualified_snapshot_fixture_has_verified_paths_and_bytes -- \
  --ignored --nocapture
```

The traversal implementation is commit `46c981560`. The ordinary focused
storage tests exercise complete results, all three resource failures, exact
identity rejection, and missing hashed sibling dependencies without requiring
the qualified copy.

## Qualification boundary

The successful traversal proves the complete set of live hashed paths and exact
selected values reachable from this one account storage root. Hashed paths are
not invertible into unknown logical keys. Live trie coverage cannot recover
deleted slots or establish the all-ever key catalog required by existing
restart lifecycle rules. It also does not prove reward/corruption history,
redelegation history, validator semantics, or native configuration semantics.

An independently supplied semantic DPoS snapshot could be encoded by the Rust
native layout and compared with every live path in this inventory. Unknown live
paths would have to reject that snapshot. Even an exact live match would still
need a separate explicit policy for deleted history and checkpoint catalog
lineage before offline adoption.
