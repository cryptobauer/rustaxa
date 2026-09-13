# N3 API implementation evidence

Status: gas-search kernel implemented; ordinary simulation implementation and
reference qualification in progress. Full N3 remains open, including native
simulation, historical query composition, trace schemas and persisted API tests.

## Existing ownership and contracts

`ExternalEvmStateOwner` currently owns general StateAPI calls and tracing.
DPoS direct calls already use the Rust query client. This milestone adds isolated
Rust backend APIs; it does not change those production routes or original C++.

`ConcreteStateRead` pins committed period/root and distinguishes absence,
tombstone, corrupt data and unavailable history. `ConcreteExecutionRead` also
admits prepared views and therefore alone is insufficient for public historical
queries. Simulations must borrow the committed port and discard a new journal
per invocation/probe. Native simulations additionally need private staged native
state with the correct delayed reader; dropping ordinary writes alone is not
enough to prove isolation.

The pinned Go `state_dry_runner/dry_runner.go` reads the requested block's state
and replaces the caller's nonce with the full-width stored sender nonce plus one.
It retains gas-price/value/fee behavior and formats ABI revert reasons. Its trace
runner instead starts at the preceding block (zero stays zero), applies ordered
state-prefix transactions, and traces the target sequence without resetting it.
These different state/nonce lifetimes must not be collapsed into one API.

Public future-block behavior is operation-specific in the current owner:
account/call can select the latest concrete state for native-only periods,
storage/code return zero/empty beyond concrete head, and traces reject periods
beyond that head. Missing retained data is not permission to invent an empty
historical state. RPC defaults, block selection and error formatting remain
separate composition work and must retain their reference behavior.

## Gas estimation kernel

`rustaxa-evm::estimate` preserves the existing `Eth.cpp` search: execute at the
cap, start the lower bound at returned gas used, probe midpoints, raise the lower
bound on code failure, terminate on consensus failure, and stop within the
integer five-percent band. Initial code failure is terminal. Callback failures
remain infrastructure errors and exact execution error strings are retained.

The finite reference harness extracts the actual unchanged search body from
`libraries/core_libs/network/rpc/eth/Eth.cpp`, compiles it in a small C++20 harness,
and commits its SHA-256 plus eight callback scenarios. Rust compares exact
results, errors and probe transcripts against that output. Reproduction:

```sh
python3 experiments/evm_feasibility/estimate_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test estimate_reference
make rewrite-validate-fast
```

The eight focused Rust tests also cover callback errors, impossible gas overrun
and nonprogress. For an inconsistent callback with a successful cap below 20,
zero gas used and a failing one-unit midpoint, legacy C++ repeats forever. Rust
explicitly returns `NonProgress` instead of reporting invented parity or success.
Ordinary admitted Taraxa transactions have intrinsic gas at least 21,000; this
is a defensive callback-contract edge, not an execution/protocol change.

Luna implemented the kernel in `task/evm-api-estimate` from `68d23f19f`; the lead
corrected/reviewed edge expectations and supplied the independent C++ extraction.
Independent Astra review approved source/contracts and the reference comparison.
This proves search policy only, not EVM execution, RPC routing or probe isolation.
