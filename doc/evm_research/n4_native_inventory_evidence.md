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

## Six-period historical availability

A second temporary read-only diagnostic used `FinalChainRepository` point reads
to obtain the exact H-5 through H headers, then opened all six identities in one
`ConcreteCheckpointReaders` handle. The diagnostic source SHA-256 was
`5a93eb2f5166da3f51d36e05d5b76fcaba042ee539c89fbc437832523c02cd2e`.
Its 121-line TSV output had SHA-256
`d5a6455cbaf22f9c5a5d2b0436f7760710f2e9c36a3eefa88383d290190cb425`.
The diagnostic opened only the same independently copied application and state
databases and refused the original snapshot path.

All six finalized headers were present and decoded to these identities:

| Period | State root |
| ---: | --- |
| 25,706,944 | `33199a435b8e4e4496fd7fd2bd8dd5bbb52263b775bc42b80f377a7e78b69014` |
| 25,706,945 | `387d37ea3df2ab3b69f78cffdb193fc6fee34565239988f6c7560eec621678b9` |
| 25,706,946 | `19c7e922abbc04acd3fae7e15437a84a9cd0ba502b3d116d6e57feb9031fb649` |
| 25,706,947 | `d2dc0b257de18518578497a8316087c2d10ec9e581b878795a9cc3930237dde7` |
| 25,706,948 | `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2` |
| 25,706,949 | `b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227` |

The checkpoint reader accepted every exact period/root pair. At every identity,
the DPoS account was present with storage root
`c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504`
and the slashing account was present with storage root
`d688bfddb847c241b44f78c048b41d7e423f138bc5ee8fe6c24e6f047b051a7a`.
The shared native roots show that finalized native state did not change across
this six-period window even though the overall state roots changed.

For each period the diagnostic authenticated a fixed 17-path sample:

- DPoS total votes, total stake, minted tokens, total supply, yield, and the
  validator-list count;
- validator positions 1 and 196, followed by each selected validator's record,
  reward, owner, and VRF rows; and
- the slashing jailed-validator list.

Every selected physical result agreed with its logical membership proof. There
was no unavailable-history, missing-node, corrupt-value, or identity error.
The validator count was 196 throughout. The first and final indexed validator
addresses and their sampled rows were unchanged. The jailed list was the
authenticated empty RLP list `c0` throughout. These point reads are a
deterministic availability sample, not complete historical leaf coverage.

Only the head DPoS trie was fully inventoried above. A separate head-only
slashing inventory used ceilings of 10,000 nodes, 10,000 leaves, and 4 MiB of
values. It completed with 71 nodes, 54 leaves, 128 value bytes, and deterministic
entry digest
`20bdbc5d9966419de9571790797bc0e2eade35012ade428884162c1bd0275be4`.
The value shapes were one `c0` jailed list, 32 one-byte `01` values, and 21 RLP
integer-shaped values. Source layout makes those shapes consistent with proof
flags and retained jail-block rows, but values alone do not recover their key
preimages or authorize a semantic classification.

Because the DPoS and slashing storage roots are identical at all six periods,
their content-addressed physical node sets are also identical. The audit did
not select every versioned leaf value separately at each historical period, so
it does not claim six exhaustive historical inventories. It establishes that
the headers, roots, native accounts, and every sampled historical dependency
needed by the proposed H-5 through H native view are present.

The Rust-owned `rustaxa:dpos_snapshot:*` and
`rustaxa:account_snapshot:*` sidecars were absent at all six periods. Those are
rewrite-derived caches that this pre-Rust snapshot was not expected to contain.
Their absence is an importer input, rather than evidence that the authenticated
legacy concrete rows were pruned.

## Owner-reported validator candidate

The snapshot owner supplied
`0xfbb85d00ca77b0d49da4f71a91de552bce88b083` as an uncertain candidate for
their producer validator address. This is owner-reported provenance, not a
verified producer identity. No private key is needed or used for the state
cross-check, and validator membership does not prove that this validator
created, ran, or exported the snapshot. The retained head PBFT author is the
different address `0xa6cec53b4d7920709f05ca1f7ac0d66338d29cb9`.

A temporary read-only point probe with source SHA-256
`ba6ef974297a2f6a6257b07a931c66bdffa0f2c16a5ebe9dd53a5542629d7063`
opened only the qualified copy and pinned period 25,706,949 and state root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`.
The global validator iterable map authenticated the candidate at one-based
position 128. Its reverse and forward iterable rows agreed. The exact logical
keys were:

- position by validator:
  `5a48c40f7a0db86e77facc73a338b7d303d0ca37bc9ab0076e0b1c7f2e0ec6be`;
- validator at position 128:
  `634cff04eb2d3d263206e30d949eef029317db184545f623c4ae5465078992f2`;
- validator record:
  `5b8e315f16952aff7ca13007c4813d87be8a96a165e5217c930f49996b1ac91a`;
- owner:
  `904a7f86687f97a6a2fc08030315526545fd4fa4c4857f94d8feeee843eb1b37`;
  and
- VRF key:
  `2d008eec95c11cd911bf30bdfeee52a733937c8dc06ba59948cc0b05d3460ce9`.

Every DPoS row was both physically selected and authenticated as a trie member.
The exact extended validator record was
`d8d6893635c9adc5dea000008201f483e30e5684014dadb580`. Its nested legacy
fields decode to stake `0x3635c9adc5dea00000`
(`1,000,000,000,000,000,000,000`), commission 500, last commission-change
period 14,880,342, and reward-reference head 21,867,957. The extended
undelegation count is zero. The owner row is
`6165c85193ab6b67daa21863d8bfa07b14ed8293`, and the 32-byte VRF row is
`a3e7ec4f11a739e610fc2b54737788f0e68768522292591796b86611fc3593fb`.

The candidate's derived slashing jail logical key is
`69cc4d2688c3064ff2b2bad547e85886110492bfdd5c3ca3eb1d6bfe8a1013ae`.
Its logical trie path authenticated as a non-member at the head. The raw
version selector reported `HistoryUnavailable` because no physical version row
exists for that key; the authenticated path result establishes current
non-membership without converting the raw-history error into a fabricated
absence.

## Semantic reconstruction audit

The Rust canonical projection describes how a complete `DposSnapshot` maps to
raw rows, but there is no inverse importer from concrete rows. Some current
state can be reconstructed from public indexes:

- the global validator iterable map enumerates all 196 current validators;
- each known validator address derives its record, metadata, reward, owner, and
  VRF keys; and
- global vote, stake, supply, and yield keys are fixed.

That enumeration is insufficient for a complete snapshot. Delegation and
undelegation indexes are scoped by delegator, while no global current-delegator
index supplies every delegator preimage. Public getters such as validators-for,
delegations, and undelegations therefore require an address already known to
the caller. The account trie and the 23,278-leaf DPoS inventory expose hashed
paths and cannot recover those addresses. Candidate addresses from retained
transactions or logs would not prove completeness on a pruned light-history
database.

Slashing has the same inversion boundary. `getJailedValidators` exposes only
the current list, which is empty here, while `getJailBlock` requires a supplied
validator. Persisted jail-block rows intentionally outlive list cleanup.
Double-voting proof rows use proof hashes as logical keys and have no iterable
index. The live inventory applies a second trie hash, so neither historical
validator addresses nor proof hashes can be inverted from its paths.

Consequently the observed blocker is not a demonstrated absence of H-5 through
H roots or sampled native state. It is the lack of authoritative logical-key
preimages and a reviewed inverse decoder that can prove every live path is
represented. A producer/reference native snapshot export, an authenticated
logical-key manifest, or complete historical evidence capable of deriving the
same preimages would close that input. After reconstruction, the Rust canonical
projection can compare every candidate row with the live inventory and reject
unknown or missing paths.

The remaining boundary separates evidence from implementation:

| Category | Current finding |
| --- | --- |
| Qualified source facts | Six headers and roots, both native account roots, all sampled historical paths, the complete head DPoS live inventory, and the complete head slashing live inventory are readable. |
| Facts still missing | Authoritative logical preimages for every delegator-scoped DPoS row, retained slashing jail row and proof row; evidence for deleted-history/catalog lineage and redelegation-corruption history completeness; and producer configuration/reward inputs tracked by the replay workstream. |
| Rust work not implemented | A fail-closed inverse native-state decoder, exhaustive candidate-to-live-inventory comparison, concrete-backed checkpoint account port, and sidecar/bootstrap publication through existing owners. |
| Historical-retention status | No actual dependency is missing in the bounded H-5 through H sample. Exhaustive historical leaf-version availability remains untested rather than failed. |

## Qualification boundary

The successful head traversal proves the complete set of live hashed paths and exact
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


## Owner clarification: candidate participation

The owner reports that the node associated with the suspected address
`0xfbb85d00ca77b0d49da4f71a91de552bce88b083` lacked enough stake to produce
blocks or vote. The authenticated validator-index entry establishes registration
only; it does not establish eligibility or active consensus participation.
The address remains an uncertain attribution to the node that created the
snapshot. Creating a snapshot does not require producing consensus blocks.
