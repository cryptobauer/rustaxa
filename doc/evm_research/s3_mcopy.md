# S3 Ficus MCOPY profile

Status: bounded instruction-profile implementation and dual-pin EVM oracle. This
slice adds no historical fork-height selection, state behavior, native registry,
publication path, or production routing.

## Implemented contract

`rustaxa-evm/src/profile.rs` now names the cumulative Californicum, Ficus, and
Cacti instruction generations. `TaraxaProfile::new(false)` remains Californicum
and `TaraxaProfile::new(true)` remains Cacti for existing callers. New callers
may select Ficus with `TaraxaProfile::for_phase(TaraxaPhase::Ficus)`. The
application remains responsible for mapping an authoritative Taraxa hardfork
schedule to that phase.

Ficus installs MCOPY at opcode `0x5e` with static cost 3, and Cacti inherits it.
The implementation delegates the instruction mechanics to the pinned REVM
`memory::mcopy` routine. Because REVM guards that routine with Cancun, the local
wrapper temporarily admits Cancun only while MCOPY runs and restores Istanbul
after success, stack error, or gas error. The profile does not activate another
Cancun instruction, precompile, gas table, or transaction rule. Cacti continues
to install Taraxa's `0x5c`/`0x5d` transient aliases and remains the phase signal
used by driver integration for Cacti-era stateless registry additions.

## Reference evidence

The Go reference builds its Cacti table from Ficus and its Ficus table from
Californicum in `core/vm/jump_table.go`; `newFicusInstructionSet` invokes
`enable5656`, which installs MCOPY with three stack inputs and `gasMcopy`.
`core/vm/evm.go` selects the Ficus table for `Rules.IsFicus` and the inherited
Cacti table for `Rules.IsCacti`, while retaining `GasTableCalifornicum` in both.

`mcopy_reference.go` executes complete account-code programs through the Go EVM.
The Python harness archives and executes both pinned revisions:

- public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`;
- local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.

Their ten-row artifacts are byte-identical with SHA-256
`50746041f981a4a8441f9dd515f9aec5f1f446d04f78d097e6a3deed25523100`.
The exporter SHA-256 is
`30348a8c622bc1b21726c07328d1783ef0ef491deea228ba3993b74fc9042baf`.
The corpus covers Californicum rejection, direct Ficus activation, Cacti
inheritance, zero-length copying with a full-width destination, forward and
backward overlap, empty-memory expansion, a two-word dynamic copy charge,
expansion out of gas, and stack underflow.

The Rust differential test compares completion, exact returned bytes, and total
transaction gas for every row. It also verifies that the runtime spec is Istanbul
after every exit and pins the explicit phase API plus legacy boolean compatibility.
The existing profile test separately confirms that an unlisted later opcode such
as BASEFEE remains unavailable.

## Validation

```sh
TARAXA_EVM_SOURCE=/workspaces/rustaxa-evm/submodules/taraxa-evm \
  python3 -O experiments/evm_feasibility/mcopy_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test mcopy_reference --test profile_reference
cargo clippy --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test mcopy_reference --test profile_reference --no-deps -- -D warnings
```

Integration still must select the phase from an authoritative chain schedule and
exercise MCOPY through the production driver. Current-profile native contracts,
period replay, and reward application remain separate milestone gates.
