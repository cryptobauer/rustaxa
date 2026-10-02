# Escrow native gas-estimation parity

This slice composes the real historical native simulation with the existing
Rust gas search. The [harness](../../experiments/evm_feasibility/native_escrow_estimate_reference.py)
reuses the unchanged Go probe exporter and C++ compiler harness from metadata
estimation, with the five [actual escrow DryRunner requests](n3_escrow_dry_runner.md).
It hashes the reused sources and support harness, request fixture, search body
and complete outputs. The metadata corpus is unchanged.

The two pinned archives execute actual `DryRunner.Apply` for each candidate,
twice with a restored supplied nonce. The unchanged extracted upstream C++
`eth_estimateGas` body independently requests candidates and checks each one
against the actual Go transcript. It must consume every probe. This prevents
the exporter candidate loop from certifying its own gas-search decisions.

There are 26 probes across five cases. Each successful value (zero, one or 42)
uses eight probes and estimates 22,879 gas. The native-funding and intrinsic
failures each terminate on their first probe with `out of gas` and
`intrinsic gas too low`, respectively. These cases do not demonstrate midpoint
execution failure: the successful lower bound is already sufficient funding.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/escrow_estimate.rs)
runs each search twice across two physical reader reopens: 104 simulations.
Each callback creates a fresh real historical native session and compares the
exact candidate gas, nonce policy, error class/text, used gas, output and logs
with actual Go. Final estimates, errors and consumed probe counts match C++.
Exact committed rows remain unchanged. The active profile and fixed semantic
owner are the same bounded synthetic setup as the DryRunner slice.

## Checks and limits

The targeted estimation test passed on its first run. Dual-pin/C++ reproduction,
workspace fast checks passed. Frozen independent Astra medium review accepted
all eight source/fixture/report hashes and reused support hashes without findings.
Sol medium implemented directly; requested and confirmed routes match. There
were no routing failures or corrections.
Complete logs use the `escrow-estimate-` prefix under
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`. Runtime code is unchanged.

No escrow tracing, nested/delayed native trace, semantic-owner reopen, real
historical data, full-width value funding or production acceptance is claimed.
The wider N3 method matrix and Milestone 10 remain open.
