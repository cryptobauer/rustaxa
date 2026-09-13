# S0 mainnet light-snapshot qualification

Status: qualified for bounded codec/read development at period `25,706,949`; not qualified for full replay, historical
coverage, reference-binary reopen, or production adoption. The compact machine-readable evidence is
[`snapshot_qualification.json`](snapshot_qualification.json).

## Evidence preservation and identity

The supplied `/tmp/snapshot-litenode` was never opened through RocksDB. `cp -a --reflink=always` first failed because
the overlay filesystem does not support reflinks; the incomplete destination was removed. The usable copy was then
created with `cp -a --reflink=never` under the task worktree's local untracked `.snapshot-work/` directory. Source and copy
have distinct `CURRENT` inodes and no regular file with link count greater than one.

The deterministic full-content manifests cover 1,100,450 regular files and 9,826,980,174 logical bytes apiece. Both
ordered JSONL manifests have SHA-256
`d6f7d58c7e9ff8d7d734ea5104fc2bdd593cbfbcecf949f1ccec7a5ec259fccb`. The approximately 125 MB manifests and
database copy remain outside Git. The committed manifest helper makes the comparison reproducible.

## Read-only open and paired boundary

The isolated Rust qualifier opened all 36 application and nine state column families on the copy with
`DB::open_cf_descriptors_read_only`. Create-if-missing and create-missing-column-families were false; the tool contains
no write, repair, migration, or compaction path. It refuses the canonical supplied path and descendants. It linked
the Rust `rocksdb` crate 0.24.0 / librocksdb 10.4.2; success therefore proves those read-only bindings can read the
copy, not that the intended 9.10 reference binary can reopen it.

The database genesis hash exactly matches the mainnet `GenesisConfig` hash produced by a small helper linked from the
existing Rust-enabled `/build` configuration:
`8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377`.

At FinalChain head `25,706,949`, the application header and concrete descriptor both contain state root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`. Its root node is present. The period
`25,706,948` header identifies root `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`, whose root node is also present.
These checks establish a paired metadata boundary and a bounded prior-root clue. They do not establish complete trie
closure or validate every path beneath either root.

## Observed retention and fixtures

The following are observed key extrema, not claims of contiguous coverage:

| Data | Rows | Observed period minimum | Observed period maximum | Additional evidence |
| --- | ---: | ---: | ---: | --- |
| FinalChain headers | not counted | 0 | 25,706,949 | head header decoded |
| Period data | not counted | 25,645,235 | 25,706,949 | head row hashed |
| Receipts by period | not counted | 21,552,343 | 25,706,949 | head row hashed |
| Transaction locations | 758,544 | 25,644,235 | 25,706,949 | zero period-field decode failures in full CF scan |
| Main value versions | 3,641,535 | 0 | 25,706,949 | zero malformed keys; zero tombstone rows |
| Storage value versions | 11,107,986 | 0 | 25,706,947 | zero malformed keys; 258,133 tombstone rows |

The state database also contains 2,059 code rows, 1,393,005 main-node rows and 3,621,667 storage-node rows.

The exported fixture is the DPoS system account `0x00000000000000000000000000000000000000fe` at the head and prior
period. Both select the physical account version at period `25,706,900`. The account's storage-root node is present;
its 3,000-byte code exists, its Keccak-256 equals the account's code hash, and its byte length equals the declared code
size. The fixture also preserves exact account RLP and the exact selected physical rows for `minted_tokens`,
`total_supply`, and `yield`, including the selected `minted_tokens` tombstone. This is reproducible bounded data for
S2 codec tests. Membership proof paths from the pinned roots have not yet been validated, so it is not evidence of
complete state availability or a general absence rule.

The initial qualifier hashed the head period-data, receipt and rewards-stat rows plus the prior root as a candidate
replay bundle. A subsequent [bounded head-input qualification](s0_head_replay_inputs.md) decoded all 19 referenced
regular transactions and their locations, verified signatures and transaction/receipt roots, and found no system
references. That closes the selected period's transaction/receipt input check. Configuration, reward-vote inputs and
complete prior-state closure remain unverified; no replay was run.

## Producer clues and remaining gaps

Both database logs report RocksDB 9.10.0, Git SHA `0`, and compile date `2026-01-19 16:17:19`. Their session IDs
`Q6EXN31JYCZ49V3C8FYJ` and `Q6EXN31JYCZ49V3C8FYG` share a prefix, which is a pairing clue rather than a capture
guarantee. The database identities are committed in the JSON evidence. Neither the logs nor the state configuration
encode a Taraxa application revision.

The task owner identifies their lite node as the producer and reports that it likely ran node commit
`a0e85fe31eb03573cd92c165a5f81035cec9907e` (release/v1.14.1), whose EVM gitlink is
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`. This remains a candidate, not verified binary identity. At that source
revision, FinalChain invokes an application RocksDB checkpoint followed serially by the state checkpoint at the same
finalized period. The snapshot metadata cannot prove that this invocation path produced the supplied directories or
record the interval between those calls.

The unavailable producer binary identity, exact capture invocation/timing, continuity, trie closure, complete replay
inputs, intended reference-binary reopen, and full historical/activation coverage remain explicit qualification gaps.
They constrain later claims but do not block the bounded S2 physical codec/read work recorded here.

## Post-run tool review

The evidence retains the qualifier source hash from the original run committed in `297d28843`.
Subsequent guard fixes resolve database child paths, reject report/manifest outputs within either input tree,
and create new outputs exclusively. The genesis helper also refuses outputs within its build inputs or the
supplied snapshot. These guards were checked with small synthetic filesystem tests; the full content scans
were not repeated. Invalid integer-comparator key widths now abort explicitly instead of applying an
incompatible fallback order. Transaction-location evidence describes only decoding the period field;
complete location row/key/position/system-flag validation was not performed.


The owner additionally suspects that the producing validator used public address
`0xfbb85d00ca77b0d49da4f71a91de552bce88b083`, but explicitly cannot confirm it.
This is an unverified provenance clue. A matching validator record in the copied
network state can provide a cross-check, but cannot identify which node produced
the snapshot or establish its build/capture procedure. No private signing keys
are required for this check or native-state reconstruction.
