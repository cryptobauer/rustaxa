# One-member full+new historical estimation

Base: `028e316a0`. This settled derivative uses the accepted complete H1
full-new simulation seed and one request without runtime changes. Sender d1,
31->32 amount1000, value/price0, cap200000 and supplied2^512 remain exact.

New [Go probe exporter](../../experiments/evm_feasibility/native_redelegate_full_new_estimate_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_new_estimate_reference.py)
reuse unchanged H1 seed, capture and C++ search owners. Each actual probe executes
DryRunner twice independently on each unchanged pin. Complete committed snapshot
remains equal before/after. Manifest pins source/support/input/algorithm/output/
stderr identities. The harness compiles the unchanged C++ search body and requires
every transcript probe to be consumed at its actual requested gas.

Actual probes:200000,150956,126434,114173,108042,104977. Each succeeds with gas101912;
C++ consumes all6 and returns104977. The search algorithm is unchanged.

[Tests](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_full_new_estimate.rs)
assert exact accepted simulation seed/first probe identity, then compare every
probe through fresh real historical native sessions. Two semantic-owner opens,
two complete physical reader opens and two estimate runs give48 disposable Rust
simulations. Each checks supplied nonce preservation, committed nonce+1, selected
H1 identity, gas/status/output/logs and final estimate. Typed code/consensus error
arms remain explicit. Committed caller[31], a1[31,32], principal/stakes and head1
are unchanged; physical rows remain byte-identical after reader disposal.

Actual record/reproduction, target, EVM check/Clippy/all26 API/package/serial
fast/whitespace pass. Independent Sol review accepted all11 frozen hashes with
no findings (`full-new-estimate-review.md`); final pending gates completed
(`full-new-estimate-gate-closeout.md`).
Unchanged runtime retains staged ON bridge12/all15 evidence. Logs/records use
`full-new-estimate-` under
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
Sol medium implements/reviews this settled derivative. No trace, wider profile,
real network history, production/fallback/C++ change or broad gate is included.
N1–N6/Milestone10 remain open.
