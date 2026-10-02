# Actual escrow DryRunner parity

The [Go exporter](../../experiments/evm_feasibility/native_escrow_simulation_reference.go)
uses actual `DryRunner.Apply` over the unchanged complete synthetic H=1 seed.
The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/escrow.rs)
materializes its exact persisted rows and runs the real historical native session
through `simulate_with_native`. Exact active escrow entry uses the shared pure
kernel; account transfer and gas charging remain in the EVM frame.

## Profile and evidence

The existing seed activates Phalaenopsis, Magnolia, Ficus and Cornus at zero;
Aspen part two and Cacti remain inactive. Only the new Rust test chooses zero
Phalaenopsis activation. Existing native-history tests retain their original
maximum activation through a wrapper with the same default behavior.

Five actual Go cases cover values zero, one and 42, insufficient native gas,
and insufficient intrinsic gas. The input is exactly `44df8e70`. Successful
calls use 22,272 gas, with empty return and logs. Native gas failure uses 21,272
gas and `out of gas`; intrinsic admission failure uses all 21,063 supplied gas
and `intrinsic gas too low`. The reference's intrinsic cost is 21,272, so the
22,063 supplied native-failure case is below total funding for the 1,000 quote.
Do not substitute an Ethereum-only intrinsic-cost calculation.

Both pinned archives execute each request twice with its supplied 512-bit nonce
restored before repetition. Actual DryRunner replaces that nonce with the
persisted sender's next nonce. Both outputs are byte-identical, repetitions are
identical, and exact before/after committed snapshots match.

Rust runs each request twice per physical reader reopen, with two reopens and
a fresh historical native session for every probe: 20 total simulations. It
compares nonce replacement, consensus/execution error distinction and exact
text, gas, output and logs. Exact persisted rows remain unchanged after each
reader is disposed. The semantic history owner is fixed across physical reopens;
this does not prove semantic-owner restart or real historical inputs.

## Checks and limits

The complete persisted native simulation target passed all ten tests, including
the new escrow test, on its first run. Actual dual-pin fixture reproduction,
workspace fast checks passed. Frozen independent Astra medium review accepted
all eight source/fixture/report hashes without findings. Sol medium implemented
the slice directly; both routes were requested and confirmed. No routing failures
or validation corrections occurred.
Complete logs are under `/home/fry/artifacts/evm-branch-2026-10-01-2233/` with
the `escrow-dry-` prefix. No runtime implementation changed in this slice.

Existing frame tests separately cover escrow transfer and parent rollback.
This slice does not close escrow estimation or tracing, nested historical API
calls, inactive-selector presentation, pre-Cornus admission, full-width value
funding boundaries, native business rows, real-window or production acceptance.
Milestone 10 and N1–N6 remain open.
