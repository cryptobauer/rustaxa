# New-destination redelegation gas estimation

Baseline: accepted simulation `c4d2bf817` and runtime `de32bbc4f`.
This test/evidence derivative uses the same complete persisted synthetic H1,
separate aa-source/bb-destination delegation rows and existing simulation owner.
No runtime, storage or C++ source changes are included.

The [Go exporter](../../experiments/evm_feasibility/native_redelegate_new_destination_estimate_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_new_destination_estimate_reference.py)
execute21 actual DryRunner probes for11 requests from both unchanged pins.
Each probe is independently repeated and leaves the committed seed exact.
The harness compiles the unchanged upstream C++ search body through its existing
callback adapter; it checks requested gas and consumes every actual Go probe.
Algorithm SHA256 is7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759.
Source, request seed, support harness, algorithm and result identities are in the
fixture manifest. No upstream file is changed.

Partial300 estimates104,977 and partial50 estimates104,915. Each uses six actual
probes; the nine failure requests consume their initial probe. Native failures,
ABI/nonpayability, native out-of-gas and intrinsic-gas errors are preserved.
The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_new_destination_estimate.rs)
uses the existing estimator with168 fresh native simulations across two semantic
owner constructions, two physical reader opens and two repeated searches.
It checks exact requested gas, typed errors, output, logs and effective nonce for
every probe, then exact final estimates/errors and transcript consumption.
Committed per-validator/per-delegator principal/rewards and all physical rows
remain unchanged; each probe starts with the caller destination absent.

Actual dual-pin+C++ reproduction, all17 API tests and affected-package clippy
pass. Serial workspace fast passes; no failing correction batch.
Runtime unchanged: prior accepted ON bridge12/all15 remains applicable.
First-run command/output/exit records use `new-destination-estimate-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Sol medium leads directly;
independent Sol medium frozen review accepted all11 hashes without blocking
findings; report `new-destination-estimate-review.md` in the artifact directory. The existing settled estimate
contract is reused. There is no routing failure or billing inference.
Direct supported traces are next. This synthetic derivative does not close
real-history, production, N1–N6 or Milestone10 acceptance. Excluded success
branches, no fallback and no push remain in force.
