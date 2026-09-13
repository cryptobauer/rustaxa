# S3 SELFDESTRUCT and restored-storage flush evidence

The isolated execution driver now supports historical Taraxa SELFDESTRUCT.
Production routing, protocol rules and the existing database layout are unchanged.
The original supplied snapshot is not used by these synthetic tests.

## Reference behavior and implementation

Both pinned Go revisions execute the same 21 frame cases and nine direct
TransitionState lifecycle cases. `selfdestruct_reference.go` runs the actual Go
EVM and records account visibility before `CommitTransaction`, then uses a
synchronous logical output sink to record its exact account/slot mutations.
The Python runner archives each pinned revision into a disposable tree, verifies
identical results and records exporter/output identities in
`experiments/evm_feasibility/fixtures/selfdestruct_manifest.json`.

The Rust execution-only instruction wrapper validates the operand stack before
static protection, computes beneficiary emptiness from full-width metadata, and
charges 5,000 base gas plus the reference 25,000 empty-beneficiary/nonzero-balance
increment before applying journal changes. REVM's normal host call precedes its
gas check, so the host stages a read-only intent and the wrapper applies it only
on funded SELFDESTRUCT completion. This prevents out-of-gas execution from leaving
the historical RIPEMD beneficiary touch effect behind. The generic public
instruction table does not acquire this journal-specific mutation authority.

The journal transfers the full signed balance, including self-beneficiary
ordering, then zeroes and flags the source through ordinary undo. Nonce, code and
storage remain visible until transaction settlement. Settlement deletes a dirty
suicided account and suppresses its new code and storage writes. It does not
remove historical physical slot rows. Parent revert restores source balance and
flag; existing raw/transient lane lifetimes remain independent.

Go's `HasSuicided` is `IsNIL() && suicided`: every existing source reports false,
so repeated executions each add the historical 24,000 refund. This quirk is
preserved. A truly absent opcode source would dereference Go's nil account body;
Rust rejects that unsupported integrity boundary explicitly. The separate
journal-level suicide operation preserves Go's absent-source beneficiary touch.

The lifecycle corpus also exposed a pre-existing journal divergence: Go's SSTORE
undo retains the restored value in `StorageDirty`. It flushes that value if a
surviving raw or later nonce mutation makes the account update. Rust now retains
that dirty-map entry and gates slot emission on the account's actual update
path. A wholly reverted account emits nothing; undoing account creation removes
its ordinary/raw maps, while transient state survives until transaction reset.

## Covered cases and validation

- Zero, seven and 2^256 source balances against absent, empty and nonempty
  beneficiaries; self-beneficiary transfer.
- Funded and unfunded RIPEMD empty-beneficiary cases; unfunded new beneficiary;
  stack underflow with ample gas and with no execution gas.
- Nested success, repeated calls, parent revert and static rejection; top-level
  CREATE whose initcode self-destructs, including attempted address and deletion.
- Ordinary/raw/transient visibility and exact logical writes after commit/revert,
  absent source, later nonce modification, self-beneficiary undo and newly created
  account rollback/recreation.
- Rust-only regressions for metadata-only beneficiary quoting and signed balance
  transfer/undo. These additional cases are not claimed as exported Go fixtures.

Validation commands:

```sh
python3 experiments/evm_feasibility/selfdestruct_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm --lib --test selfdestruct_reference --no-deps -- -D warnings
make rewrite-validate-fast
```

All pass for this slice. The tests compare gas, refunds, status/error/output,
visible account fields and complete logical account/ordinary/raw mutation maps.
The helper is a logical sink, not a trie writer: no new persisted SELFDESTRUCT
root, physical-history closure, full period, historical replay or deployment
qualification is claimed. Those remain later integration/acceptance work.
