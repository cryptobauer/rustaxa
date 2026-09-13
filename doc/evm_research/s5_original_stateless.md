# S5 original stateless primitives

Status: bounded implementation of ECRECOVER, SHA-256, RIPEMD-160 and identity.
This is a primitive helper for test composition, not a complete native dispatcher,
historical registry or production route.

`rustaxa-evm/src/stateless.rs` uses the individual functions from the existing
REVM pin `6014612c86f3690e4e9173a8c4deade396af398d`. It does not select an Ethereum
`SpecId` or precompile table. Exact addresses 1–4 occur in each inspected Taraxa
registry; unsupported addresses return an explicit boundary error rather than
being classified as ordinary bytecode by this helper.

`PreparedStatelessCall` owns the complete immutable invocation and quote. It
performs no crypto work or journal reads during preparation. Consuming it either
reports insufficient gas before invoking the primitive, or charges exactly the
quote and returns output with no ordinary/raw/log effects. Invalid ECRECOVER
inputs succeed with empty bytes, and high-S recovery remains accepted. The
dispatcher retains period/sequence validation and the frame retains value movement,
gas forwarding/return and transaction settlement. This helper does not implement
that dispatcher's `NativeExecutionPort` or enable native frame routing.

The independent Go exporter calls actual `RequiredGas` and `Run` methods at both
public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. Its 35 cases cover empty/binary inputs
at word boundaries and 1,025 bytes, valid low-S/high-S recovery, right padding,
ignored trailing bytes, invalid recovery-byte padding and zero/order scalars.
The Python verifier compares both emitted artifacts and committed manifest bytes
with explicit error checks that remain active under `python -O`.

The Rust differential test checks exact quotes and outputs against those Go
fixtures. It additionally checks gas just below, exactly at and above each quote,
and preservation of all four call-kind context values without narrowing full-width
value. These are helper inputs, not execution of CALL-family opcodes; native frame
gas/rollback integration needs its own evidence.

Targeted reproduction:

```sh
python3 -O experiments/evm_feasibility/stateless_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test stateless_reference
cargo clippy --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --lib --test stateless_reference --no-deps -- -D warnings
make rewrite-validate-fast
```

MODEXP, BN254, BLAKE2F, the Ficus/Cacti BLS maps, P-256 and Falcon remain outside
this helper. Their gas schedules, malformed-input behavior and activation tables
require individual reference comparisons. See the [source inventory](s5_stateless_inventory.md)
and [native session map](s5_native_kernel_map.md) for separate expansion work.
