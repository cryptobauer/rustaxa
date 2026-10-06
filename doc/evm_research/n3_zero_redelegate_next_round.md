# Next round: zero-amount historical API

Base: `049ec5e73`; accepted runtime `8a6735c94`, frames `752629e9c`.
This is a settled N3 derivative, not a new staged adapter or historical authority.
The [accepted historical contract](/home/fry/artifacts/evm-redelegate-2026-10-03/zero-historical-contract-review.md)
and [bounded source map](/home/fry/artifacts/evm-redelegate-2026-10-03/zero-historical-api-map.md)
name exact owners, inputs and checks. Reuse them; their earlier account snapshots
are historical. Frozen final implementation review remains required.

## First slice: one-case simulation (accepted 2026-10-06)

Accepted evidence: [simulation](n3_zero_redelegate_simulation.md). Estimation is also accepted: [estimate](n3_zero_redelegate_estimation.md). Direct traces remain next.

Own a separate zero simulation exporter/harness/fixture under
`experiments/evm_feasibility/` and a sibling test under
`rust/crates/rustaxa-evm/tests/native_simulation_reference/`, with narrow registration
in `native_simulation_reference.rs`. Reuse `redelegate::{history,assert_committed}`,
materialization and `CompleteSeedReader`. Preserve runtime, storage, C++, shared
reader authority and the existing ten-case corpus. Workers are not alone; preserve
others' changes. The lead owns shared registrations, validation and commits.

Capture actual StateTransition H1 complete seed and DryRunner
`zero_before_aspen_two`: caller aa, validators 31 to 32, amount/value zero,
gas 200000, price 1, supplied nonce 2^512. Active fix/Magnolia/Ficus/Cornus,
Aspen two inactive, zero rewards, caller principal 1000 on both validators.
Use both unchanged pins:
public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`,
local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.
Repeat actual probes and preserve separate diagnostics, manifest identities,
complete before/after snapshots and exact committed H1 descriptor/root.

Construct semantic H1 by public finalization once, restart without refinalization,
and open independent concrete readers: two owners × two readers × two fresh
requests = eight disposable sessions. Use real `simulate_with_native` and
`begin_native_simulation` at H1. Compare actual gas/errors/output/log and effective
nonce (committed nonce+1); retain the supplied nonce unchanged. Check all physical
rows before/after disposal and both validator stakes, ordered delegation rows and
zero pending rewards through unchanged public assertions. No touched frame seed,
H2 relabel, guessed absence or aggregate-only disposal assertion.

## Follow-ups

After simulation acceptance, name narrow estimation and direct structured trace
slices using the same accepted profile and existing owners. Capture actual Go
probes and nonce/context rules; do not copy DryRunner nonce behavior into traces.
Preserve typed estimator failures and real prefix/target journal/disposal checks.
Require explicit effect witnesses when a stateful prefix matters. Clear per-call
oracle buffers and distinguish setup/execution observations on repeated calls.
If new semantics are unresolved, ask one bounded contract question before coding.
Do not repeat a settled Astra contract just to extend mechanical evidence.

## Gates, routing and limits

Each slice requires actual dual-pin reproduction, affected package checks/tests,
clippy, serial `RUST_TEST_THREADS=1 make rewrite-validate-fast`, immutable freeze
and independent Sol medium review. Runtime changes additionally require focused
ON bridge build with 12 jobs/all relevant tests; storage changes also require the
storage bridge gates. Retain unchanged accepted bridge evidence separately for
test-only changes. Save first-run command/output/exit codes under the new persistent
run directory. Record requested/actual routing and correction batches.

Sol medium implements directly. Luna medium performs a bounded check of the next
seam. Reserve its slot. Use a fresh Astra medium thread only for a named unresolved
authority/authentication/gas/rollback question; rotate review contexts at semantic
profile changes. Do not resume or contact audited agents.

Exclude zero with absent/zero caller pairs, full+new destination, validator deletion,
nonzero rewards, new validators and historical same-validator successes. Aspen-two
zero failure is a separate profile. No real-network/root-parity claim, production
cutover, fallback or supplied-data mutation. Broader work follows the authorized
queue after acceptance, within the baseline-relative 10-point quota budget.
