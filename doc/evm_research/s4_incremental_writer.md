# S4 incremental concrete writer prerequisite

Status: bounded storage prerequisite implemented and targeted locally; durable
publication and application composition remain open, so S4 is not complete.

## Implemented boundary

`rustaxa-storage::ConcreteStateWriter` opens an existing concrete-state RocksDB
with the reference column families and requires its descriptor to equal the
FinalChain-supplied prior identity. `prepare` accepts account, logical storage,
and immutable code mutations for exactly the next period. It authenticates all
touched prior account paths and touched storage roots, updates only touched trie
paths, derives storage and main roots, and returns a writer-bound
`PreparedConcreteState` with private rows. Every content-addressed node loaded
during mutation, including a surviving hashed sibling resolved for branch
collapse, is rehashed before its topology or value is trusted.

`prepared_account` exposes one exact five-field account row from the latest
prepared overlay over the same RocksDB handle, or falls back to an authenticated
prior row. It preserves present, tombstone, and proved-absent outcomes and
validates writer/prior/sequence binding. This gives the executor composition an
account projection view without opening a second conflicting RocksDB handle or
presenting prepared data as published state.

`persist_contents` atomically stages only compatible CF1-CF5 content, node, and
version rows. It does not change `last_committed_descriptor`, provenance,
catalog, pending-publication, or application metadata. Prepared values are
bound to one writer/prior/sequence; stale, foreign, duplicate, conflicting, and
already-persisted values fail closed. Once a handle stages rows, it rejects
another preparation while its descriptor remains at the prior generation.
Existing content-addressed rows must have identical bytes, and existing
next-period version rows are rejected.

The incremental trie encoder preserves the reference distinctions:

- a canonical branch hashes a 17-child list while its physical row stores 16;
- short nodes retain their compatible physical embedding and leaf hash hints;
- values remain in period-versioned CF3/CF5 rows and deletes write empty
  tombstones only when the deleted trie member existed; a missing path leaves
  retained physical history untouched;
- main and per-account storage roots use the existing CF2/CF4 node rows;
- ordinary words and native raw slots share the logical `ConcreteStorageKey`
  boundary, before the reference path and physical-prefix hashes;
- account nonce/balance/code physical fields are preserved when a touched slot
  causes storage to replace only the derived storage-root field;
- account storage roots always come from authenticated prior state plus slot
  mutations, and live code references must resolve to staged or existing bytes
  with the declared hash and size;
- a contradictory account-delete plus storage-mutation batch is rejected rather
  than presented as the Go account flush behavior.

No EVM crate, C++ path, new column family, metadata key, database bootstrap, or
production route was added.

## Bounded evidence

The trie unit test replays all eight insert/update/delete/reinsert stages in
`experiments/evm_feasibility/fixtures/public.json`. At every stage it compares
the root and the complete accumulated physical-node map byte-for-byte with the
pinned public Go writer at EVM revision
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`.

A disposable RocksDB test exclusively creates the reference columns and a
known empty generation-zero descriptor. It stages a real account, slot and code
row, advances the descriptor only through a private test helper, closes every
handle, reopens through `ConcreteStateReader`, then continues with a second
writer. The seed and update roots and their CF2-CF5 physical rows match the
`reverse-existing-commit` prior/final rows in the pinned public journal fixture:

- prior root `e8269fee2b975af999fca4ae0f98390c2b86a727e060065ecaf870aea2cf951b`;
- updated root `b9d139c10bb0fe50e36d00dd6235a3b21b9c7ad871cee485b9af2efefffe10c9`;
- raw slot bytes change from `11` to `0044` with exact account/storage root
  propagation;
- a historical reader pinned to the first root still returns the old row after
  the second root is staged and test-published.

The journal oracle manifest pins matching public/local fixture SHA-256
`28c17da4b2e73a63339c6557d8d65477b1ca90ca451ee144a5c46bbdf0d83cc3`.
Another fresh fixture uses distinct ordinary and native-raw logical keys and
verifies both after close/reopen. This tiny exclusively-created database gives
the test complete knowledge of its rows; no generic trusted-coverage switch or
claim about imported snapshot history is introduced.

Targeted validation:

```text
cargo test -p rustaxa-storage concrete_state::
13 passed; 0 failed; 1 ignored (qualified copied-snapshot test)
```

The documented Spark helper was attempted for the bounded mapping task but was
unavailable due its service usage limit. Mapping and implementation continued
with the assigned lead model; this does not weaken the pinned byte comparisons.

## Remaining S4 work

FinalChain still needs to consume the accepted execution/journal plan, bind it
to `PreparedConcreteState`, and atomically authorize concrete descriptor,
provenance, catalog, and pending-publication transitions through the existing
durable lifecycle. The private descriptor helper exists only inside the
disposable reopen test and is not callable by production code.

Imported database raw-history coverage remains unresolved. The writer does not
turn a missing retained physical row into semantic zero or add markerless
bootstrap provenance. Full journal projection, per-transaction roots,
native/reward application, C++ bridge validation, recovery, fault injection,
and differential replay remain outside this prerequisite.

Because this prerequisite intentionally adds no pending-generation metadata, a
process restart after `persist_contents` cannot distinguish abandoned future
rows from lifecycle-authorized staging. Callers must not reopen and prepare a
different generation from the same prior after unpublished staging. The durable
S4 lifecycle must keep prepared rows in memory until its existing atomic
CF-row/descriptor/provenance/catalog/marker commit, eliminating that interim
staging ambiguity.
