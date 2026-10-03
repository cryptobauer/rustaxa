# Full caller-source historical simulation

Baseline: frames `65ef42b5b`, runtime `6b5228ad2`.
This test/evidence derivative uses the existing real disposable historical
simulation, FinalChain owner and checkpoint reader; runtime is unchanged.

The [exporter](../../experiments/evm_feasibility/native_redelegate_full_source_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_source_simulation_reference.py)
create a separate complete actual Go H1 seed. Both validators31/32 have aa/bb
principal1,000 each, aggregate2,000 each and total4,000. Rewards are zero, all
required activations are at zero, minimum100 and maximum1,000,000. Empty H1
commits through the public state transition. Full aa transfer1,000 in either
direction retains source1,000 and destination3,000; bb principal/cursors remain.
The complete actual native/account trie, rows and root are exported. Prior3,000
frame rows/root/counts are not transplanted. Absence authority applies only to
this complete synthetic seed, not a real network checkpoint.

Twelve actual DryRunner.Apply cases are full swap-last and full last-item, plus
the ten accepted existing-pair partial/normal/ABI/admission failures. Each is
independently repeated from committed H1. The supplied nonce2^512 stays in the
request; execution uses committed sender nonce2^264+5 plus1. Both pins reproduce
identical outputs/errors/gas/logs and exact before/after committed state.

The [Rust test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_full_source.rs)
compares96 fresh sessions across two semantic owner constructions, two physical
reader opens and two repetitions per case. Initial owner construction finalizes
one public empty H1; owner restart loads head1 without refinalization. Exact public
committed assertions retain both validators2,000 and both aa/bb two delegation
rows of1,000 with zero pending rewards. Every disposed physical reader is followed
by exact row equality. Total4,000 alone is not treated as disposal evidence.

Astra medium accepted the4,000-profile historical contract. Sol medium implements
directly; independent Sol medium frozen final review accepted all10 hashes without
correctness findings; see `full-source-simulation-review.md` in artifacts. Actual dual-pin
record/reproduction, all19 API tests and affected clippy pass. Serial workspace
fast passes. No failing correction batch. Runtime unchanged: prior
accepted full-source ON bridge12/all15 remains applicable. Commands/full outputs/
exit codes use `full-source-simulation-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. All requested routes ran without
failure; no billing inference is made.

Full-source estimation and direct supported traces are next. Full+new destination,
source-validator deletion, reward-bearing/zero/new-validator and historical
same-validator success remain excluded. N1–N6/Milestone10, real-history and
production acceptance remain open. No fallback, broad gate or push is authorized.
