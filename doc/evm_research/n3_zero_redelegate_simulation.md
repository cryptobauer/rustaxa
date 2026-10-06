# Zero-success historical simulation

Accepted on 2026-10-06 over runtime 8a6735c94 and frames 752629e9c. This is one
settled complete synthetic H1 derivative, with unchanged Rust owners and prior
ten-case corpus. No production route or real-network/root-parity acceptance.

The sibling [exporter](../../experiments/evm_feasibility/native_redelegate_zero_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_zero_simulation_reference.py)
compose the existing complete StateTransition seed and run actual DryRunner twice
on both unchanged public/local pins. Caller aa has 1000 on validators 31/32; zero
amount/value, gas 200000, price 1 and supplied nonce 2^512. Fix/Magnolia/Ficus/Cornus
are active; Aspen two inactive; rewards zero. Both outputs reproduce byte for byte
with separate hashed diagnostics. Actual H1 root:
`896c35f3da23eff33a5769399057ca81883044bbd4b7169e9e718997ac2c2982`.
All 77 physical seed rows are retained. Actual gas 101784, empty errors/return,
effective nonce 2^264+6 and one zero-amount Redelegated event.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_zero.rs)
uses unchanged materialization, CompleteSeedReader, redelegate::history and
assert_committed. Public H1 finalization occurs once; restart does not refinalize.
Two semantic owners × two independent concrete readers × two fresh requests give
eight actual simulate_with_native/begin_native_simulation sessions. Checks cover
ABI, exact selected identity, supplied nonce preservation, committed nonce+1,
gas/errors/output/logs, both validator stakes, ordered caller rows, zero pending
rewards, unchanged head and complete physical bytes after disposal.

Targeted/package tests, package check, required Clippy, actual dual-pin reproduction
and serial fast all passed. Extra strict Clippy with -D warnings failed on existing
consensus warnings; that failed output is retained. No runtime/storage/C++ change
required a new bridge gate; accepted ON bridge evidence remains linked from the
prior runtime record. Source/evidence freeze and independent Sol-medium review
were accepted without required corrections.

Run artifacts and first-run command/output/exit records:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`.
[Frozen review](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/zero-simulation-review.md).
Lead and reviewer used confirmed Sol medium; Luna used confirmed Luna medium.
Starting allowance 66%, target 56%; the saved budget is unchanged. Estimation and
direct traces remain separate. Aspen-two failure, absent/zero pairs, rewards,
new validators, validator deletion and historical same-validator success remain
outside this contract. Milestone 10 and N1–N6 stay open.
