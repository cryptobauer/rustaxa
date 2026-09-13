# N4 retained-head replay preflight

This slice qualifies the smallest useful execution claim at the retained
mainnet head: period 25,706,949 can be decoded and executed in exact transaction
order against authenticated reads from period 25,706,948. The probe is
read-only. It opens only the independent copy at
`/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy`; the supplied
`/tmp/snapshot-litenode` path and its descendants are rejected.

## Contract

`replay_preflight` uses the existing Rust storage and execution APIs directly:

- `FinalChainRepository`, `PeriodRepository`, and `TransactionRepository` load
  the exact head, header, PBFT metadata, ordered signed transactions, and
  retained receipts with point reads.
- `ConcreteStateReader::open_historical_read_only` pins the current descriptor
  and prior header root. Every demanded account, code, and logical slot access
  goes through this reader. A pruning, unavailable-history, corruption, or I/O
  result aborts execution; the probe never substitutes an empty value.
- `decode_legacy_input`, `ExecutionJournal`, and the top-level execution driver
  execute the ordered stream. Settled account values are carried between
  transactions in memory. This overlay is semantic execution state, not a
  physical trie or publication handle.
- The retained five-field receipt shape is encoded from execution facts and
  compared byte-for-byte at every position.

Before executing, the probe requires exactly 19 signed calls, no system
transactions, empty calldata, non-native receivers, and receivers with no code.
Any storage, raw-storage, or code write fails the transfer-only bound. The
context uses the signed PBFT block's author and timestamp, chain ID 841, Cornus
envelope behavior, the post-Cacti profile, and mainnet's Cornus PBFT gas limit
`0x7d2b7500`. These transfers execute no bytecode and therefore do not observe
the block context; the producer binary remains unverified.

Run the probe with a fresh output path:

```bash
CARGO_TARGET_DIR=/tmp/rustaxa-state-qualification-target \
  cargo run --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin replay_preflight -- \
  /tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy \
  /tmp/replay_preflight.json
```

The committed compact report is `n4_replay_preflight.json`. It preserves each
transaction identity and decoded envelope, every authoritative base account
dependency with its exact physical RLP, any demanded code or slot dependencies,
and the per-position receipt comparison.

## Qualification boundary

Successful exact receipts qualify demanded prior-state closure for this one
transfer-only window. They do not qualify the final concrete state root,
rewards, writer bootstrap, snapshot adoption, or publication.

The observed result is exact within that boundary: all 19 retained inputs are
signed chain-841 calls with empty calldata to non-native accounts without code.
The historical reader authenticated 20 unique prior accounts and demanded zero
code and zero slot reads. Every transaction succeeded with 21,000 gas, every
five-field receipt matched byte-for-byte, and cumulative gas 399,000 matched the
header.

The next writer experiment has a specific missing contract. The existing
`ConcreteStateWriter` opens the committed current descriptor and prepares only
`current + 1`. Reproducing period 25,706,949 from the historical 25,706,948 root
requires an explicitly authorized disposable-copy adoption/bootstrap operation
or a read-only historical preparation API. That work must prove account encoding,
trie updates, descriptor/root pairing, and crash-safe publication without
changing the source snapshot.

The snapshot is also markerless for the Rust lifecycle/provenance records and
already has a nonzero application head. Current initial pairing is restricted to
head zero, so existing-head bootstrap is unsupported. Current multi-validator
Aspen2 native-session behavior is another explicit acceptance dependency; this
transfer-only period neither exercises nor qualifies it.

Reward closure is also open. Header `total_reward == 0` and the period's offset
from the 100-period distribution boundary are useful facts, but they do not
replace the producer-compatible reward request. Exact static reward settings,
the current block statistics, and the prior weighted vote/certificate payloads
must be reconstructed before claiming a reward transition or final root replay.

## Validation

The narrow validation for this experimental slice is:

```bash
CARGO_TARGET_DIR=/tmp/rustaxa-state-qualification-target \
  cargo test --locked \
  --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin replay_preflight
```

The live probe is a bounded set of point reads plus 19 transactions. No retained
range scan, state mutation, writer open, build-tree change, or production route
is part of this slice.
