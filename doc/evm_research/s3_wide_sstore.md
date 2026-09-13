# Wide ordinary SSTORE gas compatibility

Status: bounded S3 host support with direct and nested dual-reference evidence.
This does not change production routing, protocols or the concrete database
layout.

Taraxa ordinary storage rows decode to positive `big.Int` values and are not
bounded to one EVM word. The pinned Go [`gasSStore`](../../submodules/taraxa-evm/core/vm/gas.go)
compares the full original, current and opcode-new values. It observes only:

- whether each value is zero;
- whether original equals current;
- whether original equals new; and
- whether current equals new.

The opcode-new value is still a 256-bit EVM stack word. A persisted value such
as `2^256 + 7` therefore differs from an opcode operand of `7`, even though
`SLOAD` projects both to the same stack word.

## Gas-only relation carrier

The pinned REVM `SStoreResult` is consumed by `sstore_dynamic_gas`,
`sstore_refund`, `sstore_state_gas` and `sstore_state_gas_refill`. At revision
`6014612c86f3690e4e9173a8c4deade396af398d`, those helpers also inspect only
pairwise equality and zero/nonzero predicates. `JournalHost` therefore uses the
following projection only when an original or current value is wider than 256
bits:

1. Numeric zero maps to `U256::ZERO`.
2. Equal full-width values map to the same representative.
3. Each distinct nonzero value among original, current and new maps injectively
   to one of `1`, `2` or `3`.

There are at most three nonzero equivalence classes, so this mapping preserves
every predicate consumed by the gas helpers. It maps the entire triple together;
mixing a wide representative with unchanged low words could create a false
equality. Narrow triples retain their exact words.

The returned `SStoreResult` is an internal gas-equivalence carrier. It is not
state data. [`ExecutionJournal::set_ordinary_storage`](../../rust/crates/rustaxa-evm/src/journal.rs)
receives and retains the actual opcode-new word independently. Existing concrete
reader errors and storage/account consistency failures still abort execution.
Any REVM revision change requires re-auditing every `SStoreResult` consumer
before retaining this proof.

## Independent oracle

[`wide_sstore_reference.py`](../../experiments/evm_feasibility/wide_sstore_reference.py)
archives and executes both pinned Go revisions with the additive
[`wide_sstore_reference.go`](../../experiments/evm_feasibility/wide_sstore_reference.go)
exporter. Both revisions produce identical bytes for six cases seeded with
`original = 2^256 + 7`:

| Case | Gas used | Refund before transaction reset | Final logical value |
| --- | ---: | ---: | ---: |
| wide to low `7` | 26006 | 0 | 7 |
| wide to zero | 13003 | 15000 | 0 |
| low, zero, low | 26418 | 0 | 7 |
| parent clear, successful delegate child writes `7` | 26932 | 0 | 7 |
| parent clear, reverting delegate child writes `7` | 13469 | 15000 | 0 |
| successful child followed by parent revert | 26938 | 0 | `2^256 + 7` |

The nested cases exercise the real Go EVM frame checkpoints and signed refund
ordering. Rust consumes the checked JSON directly and compares status, gas,
refund, output and logical storage. The fixture programs contain no `LOG`
opcode, and Rust separately verifies that their result logs are empty. A
separate predicate matrix includes distinct wide values with the same low word,
a wide nonzero value with low word zero, and every equality/zero relation.

The corpus is synthetic. It does not prove trie-row persistence, historical
snapshot coverage, native interaction, SELFDESTRUCT, production address
selection or production execution routing.

```sh
TARAXA_EVM_SOURCE=/path/to/taraxa-evm \
  python3 experiments/evm_feasibility/wide_sstore_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test wide_storage_reference --test host_driver_reference
cargo clippy --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --lib --test wide_storage_reference --test host_driver_reference \
  --no-deps -- -D warnings
```
