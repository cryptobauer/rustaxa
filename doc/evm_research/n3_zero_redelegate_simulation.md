# Zero-success historical simulation

Accepted on 2026-10-06 over runtime8a6735c94 and frames752629e9c. This is one
settled complete synthetic H1 derivative, with unchanged Rust owners and prior
ten-case corpus. No production route or real-network/root-parity acceptance.

The sibling [exporter](../../experiments/evm_feasibility/native_redelegate_zero_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_zero_simulation_reference.py)
compose the existing complete StateTransition seed and run actual DryRunner twice
on both unchanged public/local pins. Caller aa has1000 on validators31/32; zero
amount/value, gas200000, price1 and supplied nonce2^512. Fix/Magnolia/Ficus/Cornus
are active; Aspen two inactive; rewards zero. Both outputs reproduce byte for byte
with separate hashed diagnostics. Actual H1 root:
`896c35f3da23eff33a5769399057ca81883044bbd4b7169e9e718997ac2c2982`.
All77 physical seed rows are retained. Actual gas101784, empty errors/return,
effective nonce2^264+6 and one zero-amount Redelegated event.

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
Starting allowance66%, target56%; the saved budget is unchanged. Estimation and
direct traces remain separate. Aspen-two failure, absent/zero pairs, rewards,
new validators, validator deletion and historical same-validator success remain
outside this contract. Milestone10 and N1–N6 stay open.
