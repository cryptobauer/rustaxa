# S5 stateless frame integration

The opt-in CALL/CREATE entry points now execute the reviewed stateless helpers
at exact addresses 1–5 when the caller's full native classifier selects them.
They retain the existing consensus-native port and period sequence. The default
entry points still refuse native addresses; no production classifier, historical
fork selection or routing is added.

Consensus selection outside the full native set is rejected. Overlap between
the consensus subset and a reviewed stateless address is also rejected. Other
classified native addresses remain unavailable until their helpers are reviewed
and integrated.

Each top-level execution owns a checked stateless ordinal. Reached calls allocate
it after depth/funds admission, including calls whose action quote is underfunded.
Ordinary frame rollback does not rewind it. Stateless preparations own complete
typed invocations and quotes; they cannot enter `NativeExecutionPort` or advance
`PeriodConsensusSequence`. Returned gas and pure effects are checked before the
existing native frame settlement handles output, value transfer and rollback.

The shared driver test fixture adds six cases without changing earlier native
assertions:

- bounded direct calls from the original primitive and MODEXP corpora, checking
  output, intrinsic plus quoted gas, value transfer and absence of consensus facts;
- top-level quote underfunding, retaining unused action gas and reverting value;
- SHA, consensus, MODEXP, consensus interleaving followed by parent REVERT, with
  exact consensus IDs `0,1`, retained rollback observations, and a 204-gas
  difference between funded and underfunded MODEXP;
- classifier overlap rejection and continued refusal by default entry points;
- identity execution during CREATE initcode and installation of its returned byte;
- funds rejection without touching the stateless account or consensus sequence.

A private ordinal test verifies zero-based allocation and checked overflow.
These are composition checks against previously pinned primitive results and
source-defined frame rules. They are not a new exported Go transcript of the
mixed synthetic program or evidence of a persisted native period.

```sh
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test native_driver_reference
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm --lib --test native_driver_reference --no-deps -- -D warnings
make rewrite-validate-fast
```
