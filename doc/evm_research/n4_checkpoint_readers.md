# Identity-pinned checkpoint readers

`ConcreteCheckpointReaders` opens one existing concrete RocksDB handle read-only
and pins its observed committed descriptor independently from an explicit list
of retained period/root identities. Every operation requires an exact listed
identity. Construction rejects duplicate periods, future identities, a missing
committed identity, descriptor disagreement and inaccessible root proofs.

Account reads authenticate the requested trie path. Physical storage history
reads remain distinct from logical storage membership proofs, and immutable code
reads retain the requested historical identity in missing-dependency errors.
Construction checks a deterministic account path for every root; it does not
prove complete trie closure or authenticate a historical period/root association
without the application's authoritative finalized header.

This reuses the existing Rust physical reader and version selection. It creates
no columns, writes no markers, exposes no publication capability and does not
convert sparse account observations into a complete snapshot. Authenticated
full live storage inventory, semantic DPoS reconstruction and checkpoint
adoption remain separate work.

Implementation: `4ff0cf8af`, authored by the resumed Sol state worker as
`b73a25da8`; independently source-reviewed by the resumed Astra reviewer.
Eight focused reader tests passed; one pre-existing external-fixture test
remains ignored. `make rewrite-validate-fast` passed. With
`RUSTAXA_ENABLE:BOOL=ON`, `cmake --build /build --target rust_storage_tests
--parallel 12` and all four storage bridge tests passed. No expensive
differential gate or original-snapshot access was performed.
