# Zero-success historical gas estimation

Accepted on 2026-10-06 after simulation `1c5de6f75`. The sibling
[exporter](../../experiments/evm_feasibility/native_redelegate_zero_estimate_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_zero_estimate_reference.py)
execute one accepted pre-Aspen-two zero request over the unchanged complete H1
StateTransition seed. Actual probes repeat on both unchanged Go pins. Complete
before/after state matches; separate diagnostics and source/input/output/support
hashes are retained. The old ten-case estimate corpus is unchanged.

Six actual gas probes: 200000, 150892, 126338, 114061, 107922, 104853.
The compiled unchanged C++ search consumes all six and returns 104853.
The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_zero_estimate.rs)
replays each probe through estimate_gas and fresh real native simulations.
Two semantic owners × two independent readers × two requests × six probes give
48 sessions. H1 finalizes once; restart does not refinalize. Checks cover selected
identity, gas/errors/output/logs, supplied nonce preservation, committed nonce+1,
probe order/exhaustion and typed result. Existing typed failure coverage stays
unchanged. Committed head, both stakes, ordered caller rows, zero pending rewards
and exact physical rows stay unchanged after disposal.

Actual dual-pin reproduction, targeted/package tests, package check, Clippy and
serial fast passed. Frozen independent Sol-medium review accepted without source
corrections. Runtime/storage/C++ owners stay fixed; retained accepted ON bridge
evidence applies. Lead/reviewer used confirmed Sol medium. No routing failure.

First-run logs, command/exit records, source freeze and
[review](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/zero-estimate-review.md):
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
Starting allowance 66%, target 56%; original budget remains fixed.
Direct traces remain separate. All excluded success profiles, real-network roots,
production routing and Milestone 10/N1–N6 acceptance remain open.
