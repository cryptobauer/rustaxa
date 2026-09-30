# Four-key authenticated sender proof — 2026-09-30

From `bf57aec01`, the new `--head-sender-proofs` mode completes the next bounded
N4 question using the existing Rust checkpoint owner. The
[report](n4_head_sender_proofs.json) establishes **NonMember for all four keys**
at H=25,706,949, root
`b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`.

The keys are the same fixed sender's delegation count, V1 undelegation count,
V2 validator count and V2 last-ID cursor. Each proof authenticates its path
through `ConcreteCheckpointReaders::verify_storage_path_at`; no child or full
inventory enumeration runs. Exactly four logical proof calls occur, with zero
separate `storage_at` scout calls. The owner performs its internal account/trie
reads; four calls does not mean four physical RocksDB reads.

The prior [physical scout](n4_head_sender_scout.json) remains unchanged at its
original commit. All four raw results were `history_unavailable`. The proof
report binds that artifact's SHA and retains the distinction: logical absence
is independently proven at this root, not inferred from missing physical rows.
The result does not establish historical/deleted-key coverage, no previous
participation, complete native state, adoption, publication or production routing.

The CLI shares exact copy/symlink/output guards, historical sender provenance
validation and fresh paired identity qualification. The same four-key derivation
feeds both modes. Member values retain exact bytes and must fit the declared
4/4/4/8-byte widths; proof unavailability/pruning stay unknown, while corruption,
I/O or malformed member values fail closed. Only the lead opened the qualified
independent copy; `data/` was not opened with RocksDB.

Command (exclusive new output):

```sh
cargo run --locked --manifest-path experiments/evm_feasibility/snapshot_qualifier/Cargo.toml \
  --bin native_inverse_coverage -- --head-sender-proofs \
  local/evm-state-db/snapshot-litenode-copy local/evm-state-db/reports/head-sender-proofs.json
```

Validation: 15 inverse binary tests passed, including three new proof tests for
exact keys/values, strict widths, four-call bounds, explicit nonmembership,
unavailable/pruned separation and fail-closed errors. Strict all-target qualifier
clippy (`--no-deps -- -D warnings`), fmt and whitespace passed. The existing
checkpoint owner and pinned Go path tests provide trie semantics; no storage
implementation, C++ or production route changed. Tier 1 applies. The lead explicitly
ran `.githooks/pre-commit`; workspace fmt/clippy/tests, both structural guards
and whitespace passed (local log `sender-proofs-pre-commit.log`).

Luna medium mapped APIs and references first; reused Sol medium implemented;
reused Astra medium independently reviewed source and report. No routing failure
or substitution occurred. Their prior runtime IDs remain applicable. A separate
Astra high read-only contract task maps the next reward state period because
historical delay/eligibility semantics require escalation; it did not implement
or approve this proof mode. This continuation began at 37% weekly usage used;
new slices stop at 70% used to reserve closeout before the user's 25% remainder.
