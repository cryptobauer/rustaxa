# S2 compatible concrete reads: current-head milestone

Status: current committed-head account/storage/code reads implemented and validated in isolation. Historical identity
selection, writer/update codecs, prepared commits, pruning evidence and production routing remain open parts of S2 and
later slices.

`rustaxa-storage` now opens the existing separate `state_db` through RocksDB's read-only API. Construction requires the
descriptor to exactly match a FinalChain-supplied period/root. It never creates a database or column family and does
not adopt a moving latest descriptor. A future expected period is distinct from an identity mismatch.

The account codec preserves exact five-field physical RLP and arbitrary-width values emitted by the reference writer.
The shared nonce domain rejects noncanonical leading-zero nonce bytes, and code-size decoding is likewise stricter than
the permissive Go read helper; this milestone does not claim equivalence for physical rows the reference writer cannot
produce. The four-field
commitment codec reuses the physical nonce, balance and present hash strings after validating their decoded record;
only absent storage/code hashes are normalized to the reference empty hashes. This retains the reference decoder's
leading-zero balance and short-hash behavior without weakening the shared canonical nonce domain. Account and storage
version keys match the existing Keccak/prefix plus big-endian-period layout.

Account reads verify one persisted physical-node path against the pinned state root. The verifier reconstructs Taraxa's
16-child physical branches, canonical 17th empty branch value, cached embedded nodes and leaf hash-value encoding.
Missing nodes or referenced values return `HistoryUnavailable`; hash, shape and value mismatches return `Corrupt`.
Authenticated non-membership is the only current source of `ConcreteRead::Absent` for accounts. No unproved miss is
reported as pruning.

`ConcreteStateRead::storage` follows its physical-history contract: it returns the selected live row or tombstone at
or before the pinned period independently of current account/root reachability. This preserves valid orphan rows after
account replacement. A missing physical predecessor returns `HistoryUnavailable` because logical non-membership does
not prove physical-history coverage. `ConcreteStateReader::verify_storage_path` separately reports authenticated
logical membership/non-membership when a current account storage root exists; it never changes the raw read result.

Focused synthetic tests cover wide account values, exact commitments, current storage/code, live and tombstoned orphan
rows, non-membership, unavailable history, identity mismatch and node/code corruption. A persisted branch/extension
fixture emitted identically by both pinned Go references covers membership, non-membership and a missing path node.
An opt-in test against the
independent qualified snapshot copy verifies the complex DPoS account path, its referenced 3,000-byte code and the
`total_supply` raw row/path at period `25,706,949`.

The current constructor intentionally accepts only the database descriptor identity. The prior root observed at
period `25,706,948` is not exposed as a general historical reader until retained path/value coverage can be established.
Code-row absence also remains `HistoryUnavailable`. The module is isolated from application routing and has no write,
repair, migration, publication or protocol behavior.

The focused commands used for this milestone are:

```bash
cargo fmt --all --manifest-path rust/Cargo.toml -- --check
cargo clippy --locked -p rustaxa-storage --manifest-path rust/Cargo.toml --lib -- -D warnings
cargo test --locked -p rustaxa-storage --manifest-path rust/Cargo.toml concrete_state
RUSTAXA_QUALIFIED_STATE_DB=/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy/db/state_db \
  cargo test --locked -p rustaxa-storage --manifest-path rust/Cargo.toml \
  concrete_state::reader::tests::qualified_snapshot_fixture_has_verified_paths_and_bytes \
  -- --ignored --exact
```

The synthetic run passed all non-ignored concrete-state tests, and the explicit copied-snapshot test passed. The
repository's pre-existing storage test-only Clippy findings prevent an all-target `-D warnings` claim; the changed
library passes the strict library Clippy command above.
