# RETURNDATACOPY ordering and error parity

Status: implemented and targeted comparison passed; independent closeout review
remains pending. This is part of milestone 10's coupled N2/N3 work, not a
production route or a claim that all EVM opcodes have completed parity.

The pinned Go EVM validates the stack, computes a wrapping 256-bit memory end,
checks 64-bit word rounding, calculates/charges gas and expands memory before
checking return-data source bounds. The default REVM implementation checks
source bounds earlier. REVM's default static charge also precedes stack checks;
that changes the error when the opcode starts with no gas remaining.

The local Taraxa profile overrides opcode `0x3e` in all three declared phases.
Its table charge is zero, and the implementation charges the reference base
three plus the existing copy/memory kernels after the required checks. The
profile remains Istanbul with its existing local activation overrides.

- Memory end and rounding overflow produce `GasUintOverflow`, preserving Go's
  `gas uint64 overflow` instead of collapsing it into `OutOfGas`.
- Gas calculation/funding failure precedes invalid source bounds. Source bounds
  still apply when the copy length is zero; a wide destination alone does not.
- Go's wrapping memory arithmetic can pass sizing and later panic on the final
  destination slice. Rust returns an explicit `ReferenceInstructionPanic(0x3e)`
  from the driver, aborting/unwinding the session rather than producing a receipt.
  The opcode's internal fatal marker is recognized before nested frame settlement.

The oracle runs actual `EVM.Main` from both immutable Go pins. It uses the
existing MCOPY fixture input port and seeds return data through a real identity
precompile CALL. Twenty-three programs per phase produce 69 identical rows,
covering copies, exact/end/overflow source bounds, zero length, memory expansion,
64-bit rounding, quadratic-cost rejection, 256-bit wrapping, stack errors,
zero-gas precedence and observed reference panics. The harness catches panics
outside the unchanged Go entrypoint and labels them separately from results.

Rust compares every row through the full native-enabled frame driver, including
returned bytes, typed errors and total transaction gas. The manifest records
both reference revisions and source/artifact SHA-256 values. This corpus does
not establish every nested tracing detail or resource limit.

```sh
python3 -O experiments/evm_feasibility/returndata_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test native_driver_reference return_data_copy
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test native_driver_reference --no-deps -- -D warnings
```

Source anchors: pinned Go `core/vm/evm.go`, `instructions.go:opReturnDataCopy`,
`common.go:calcMemSize`, `gas.go:memoryCopierGas` and `memory.go:Memory.Set`;
pinned REVM `instructions/system.rs:returndatacopy` and its memory/gas kernels.
