# S4 ordered observer storage evidence

Status: the bounded storage-owned ordered overlay is implemented and tested.
FinalChain composition with the general execution adapter remains a separate
integration gate. This work does not change production routing.

## Implemented boundary

`rustaxa-storage::ConcreteStateLifecycle::apply_observer_phase` accepts one
`ConcreteObserverPhaseDelta` after the existing durable execution marker has
been staged. Account and code keys remain unique. Slot operations remain in the
supplied order and may repeat a logical key, allowing an ordinary operation and
the later raw operation to affect the same trie member without being collapsed.

The first phase updates the durable prior. Every later phase updates the exact
preceding prepared root and reads its in-memory CF1-CF5 rows before RocksDB.
Content-addressed CF1/CF2/CF4 rows accumulate with byte-conflict checks. The
current period's CF3/CF5 rows keep the latest exact value or tombstone. Account
deletion does not remove earlier physical slot rows, and deleting a logical
nonmember does not create a tombstone over a retained orphan.

Each successful phase returns a sealed `ConcreteObserverPhaseOutput` containing
the derived identity and exact changed account rows. Private writer and sequence
fields bind it to one lifecycle and phase even when two phases have the same
root. `prepared_view` borrows the lifecycle and implements only
`ConcreteExecutionRead`: account paths authenticate against the prepared root,
physical slots select the overlay before the pinned prior, and code selects
immutable staged bytes before committed bytes. Missing prior physical history
continues to return `HistoryUnavailable`.

A failed phase leaves the preceding output and view valid. Advancing a phase or
discarding its exact marker invalidates older outputs. Ordered preparation and
the earlier cumulative fixture preparation cannot be mixed on one lifecycle.
`commit_observer_approved` consumes the latest sealed output and uses the
existing synchronous atomic rows/descriptor/provenance/catalog/marker commit.

## Independent reference evidence

`ordered_overlay_reference.py` exports and runs the same direct `TrieSink`
fixture in the public EVM revision
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local comparison revision
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. Both outputs are byte-identical at
SHA-256 `b838ca31b575f2b0338ea59bf7e9e71bb9bc9d6596040d5a266c0e0f39612b3c`.
The exporter SHA-256 is
`82ca898dcdabd30fc0b1129587c52545034eed51cb4a2b15b69494bc57354616`.

The Rust storage tests compare roots and complete prepared CF1-CF5-equivalent
maps with that fixture. Covered behavior includes:

- ordinary `Put(1)` followed by raw `Delete` of the same previously missing
  key, producing a physical tombstone and restoring the nil logical root;
- slot creation, account deletion, recreation with a nil storage root, and a
  later new root that retains the old physical slot as an orphan;
- delete of that nonmember orphan, which leaves the root and retained physical
  value unchanged while advancing the private phase token;
- stale output rejection, rollback after a rejected contradictory phase,
  unavailable physical-history propagation, final atomic commit, and reopen.

Targeted validation in the isolated implementation worktree:

```text
python3 experiments/evm_feasibility/ordered_overlay_reference.py
Both pinned ordered observer references executed and reproduced

cargo test -p rustaxa-storage
121 passed; 0 failed; 1 ignored

cargo clippy -p rustaxa-storage --lib -- -D warnings
passed
```

The ignored test requires the independently copied qualified mainnet snapshot.
The crate's strict all-target Clippy invocation still reports existing warnings
in unrelated legacy test helpers; it reports no ordered-overlay finding.

## Remaining boundary

This fixture drives `TrieSink` directly. It does not establish public RocksDB
same-block visibility, general EVM transaction-cache lifetime equivalence,
native semantic-cache lifetime, imported-state retention coverage, historical
replay, application publication, recovery fault behavior, or production
routing. The general adapter must still translate settled ordinary operations
before raw operations and validate each phase report through existing FinalChain
ownership. The earlier finite cumulative S4 composition remains in place until
that ordered composition passes its exact physical comparison.
