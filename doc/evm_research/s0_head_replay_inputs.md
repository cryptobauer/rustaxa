# S0 head-period replay-input qualification

Status: period `25,706,949` has a complete bounded ordered transaction and
receipt bundle, a matching FinalChain header, and an available prior descriptor.
It is not yet replay-ready because exact producer configuration, weighted prior
certificate-vote inputs, and complete prior-state trie closure remain
unqualified. No replay or retained-range scan was run. The exact generated
manifest is [`s0_head_replay_inputs.json`](s0_head_replay_inputs.json).

## Method and preservation

`head_replay_inputs` opened only the existing independent copy at
`.snapshot-work/snapshot-litenode-copy`. The supplied
`/tmp/snapshot-litenode` was not opened. The helper canonicalizes the copy and
its application/state children, refuses the supplied path or symlink escapes,
requires an output outside both databases, and creates that output exclusively.
Both databases use `DB::open_cf_descriptors_read_only` with database and column
creation disabled.

The helper uses the existing Rust `PeriodRepository`, `TransactionRepository`,
`FinalChainRepository`, and `MetadataRepository` point-read APIs. It uses
`ConcreteStateReader::open_historical_read_only` for the prior descriptor. It
does not start a node, execute a transaction, mutate a database, enumerate a
retained period range, or infer coverage from key extrema.

Reproduction from repository base `7579f6af3`:

```bash
CARGO_TARGET_DIR=.snapshot-work/cargo-target cargo test --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin head_replay_inputs

CARGO_TARGET_DIR=.snapshot-work/cargo-target cargo run --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin head_replay_inputs -- \
  .snapshot-work/snapshot-litenode-copy \
  .snapshot-work/head_replay_inputs_rerun.json

cmp .snapshot-work/head_replay_inputs_rerun.json \
  doc/evm_research/s0_head_replay_inputs.json
```

The run linked `rocksdb` crate 0.24.0 / librocksdb 10.4.2. The executed helper
source SHA-256 is
`3e8d9c11e30149aa8994264e52ae368c60ce7bf6736b5959c1e3437e8f7161f2`.
The generated JSON SHA-256 is
`8f54b77461f9ab4583434bb64c443b51b03ce7c6cd65bb83305467016271ac85`.
The package version field is only the helper crate's `0.0.0`; the source hash
and repository base identify this qualification run.

The application database exposes 36 column families. It lacks the newer Rust
`finalized_reward_vote_cursor` sidecar, which this legacy snapshot and bounded
reader do not need. The helper requires only the existing columns it reads and
never creates the missing sidecar. All nine concrete-state column families are
present.

## Ordered inputs and receipts

The 7,286-byte head `PeriodData` row has SHA-256
`bcb4b16bed154c41156ed87f6d020f5ed7fad96f0c7562af778aeefd474de11b`.
It contains 19 regular transactions in storage order and no persisted system
transaction hash list. Every regular transaction:

- was returned byte-for-byte by the Rust period-position lookup;
- decoded as a valid signed legacy transaction with observed chain ID `841`;
- has a finalized location for period `25,706,949` at its exact list position;
- has its transaction hash, RLP byte count, and RLP SHA-256 recorded in the JSON.

The ordered transaction trie root is
`899f44457d384d45b5bd704ce5e091e966f00ed93dab3c62b73816cd867fe654`,
exactly the stored header's transaction root. The current concrete transaction
bundle rule, Keccak-256 over the RLP list of transaction byte strings, produces
`be29b0050585ee61785b87eebff229c2f81bfcbe2bbf5beae9cb6dda61f7acad`.

The 208-byte period receipt row has SHA-256
`a0cf47cce427e5517939c1eeaefc347184899e5fef60d6a580db109d30260aa1`.
It contains 19 ordered receipts, matching the input count. Its ordered trie root
is `84c077c745d619c706fb637ec0c68966e5a057ea63ed78e39e496d72f7a4f721`,
exactly the header receipt root. Each receipt's bytes and SHA-256 are recorded.

All 19 optional `final_chain_receipt_by_trx_hash` lookups are absent. This does
not make the ordered period receipt list incomplete: its count matches the
inputs and its exact bytes reproduce the header commitment. It does mean this
light snapshot cannot validate the by-hash receipt query index at the head.

## Header and prior descriptor

The stored head header is 399 bytes with SHA-256
`1bebb6391e97caa4e8d34db4336810ec71eae83ef975d48bfb05e0b56e81363f`.
The stored number-to-hash index value is
`e5b61be65207997d9d30536500b4bd8be3fc2bbe3ec00c38343f20282c6e5065`.
The helper does not recompute the contextual canonical block hash because that
requires PBFT and configuration facts outside this report. The header's parent
hash equals the stored block-hash index at period `25,706,948`. The header records
gas used `399,000`, zero total reward, and final state root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`.
That period/root pair exactly matches the current concrete descriptor.

The prior header is present and decodes at period `25,706,948`; its state root is
`926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`.
The prior root node is present, and the compatible historical concrete reader
constructor accepts the current descriptor and that prior period/root. This
establishes an available prior descriptor for future bounded reads. The
constructor does not attest the caller-supplied historical identity, and one
root node does not prove that every account, slot, code, or dependent trie node
a replay will touch is retained.

## Configuration facts and remaining replay gates

The database genesis hash exactly matches the documented mainnet hash
`8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377`.
The Rust metadata reads also find period lambda `1,500`, dynamic-lambda round
count `251`, the applicable 12-byte sortition-parameter row, and the 566-byte
head block-reward-stat row. Their exact hashes are in the JSON. These are stored
facts, not a complete node configuration.

This bounded pass did not recover or qualify the exact static execution profile,
activation schedule, block gas-limit policy, reward parameters, or native-kernel
configuration used by the producer. The current Rust FinalChain constructor also
needs weighted previous-certificate vote payloads that `PeriodData` alone cannot
reconstruct; this bounded pass did not qualify those side inputs. Finally, the
prior root has not undergone a complete touched-path inventory.

Therefore the answer is deliberately split:

- The head has complete ordered transaction inputs, complete ordered receipts,
  a matching header, and the required prior descriptor for a future replay.
- The snapshot is not yet authorized or qualified for that replay. A later
  bounded gate must supply the exact configuration and certificate-vote inputs,
  then verify every prior-state path requested during execution before comparing
  receipts and the final root.
