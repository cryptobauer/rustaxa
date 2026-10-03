# New-destination historical redelegation simulation

Baseline runtime: `de32bbc4f`; the separate actual frame slice is accepted at `4d745895f`.
This slice changes only test/evidence code. It uses the existing disposable
historical simulation, real FinalChain owner and concrete checkpoint reader.

The [exporter](../../experiments/evm_feasibility/native_redelegate_new_destination_simulation_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_new_destination_simulation_reference.py)
capture actual DryRunner.Apply from both unchanged Go pins over a complete
synthetic persisted H1. Source validator31 has aa principal1,000; destination32
has bb principal1,000, with no aa destination pair. Both metadata owners are aa.
Minimum100, maximum1,000,000, zero rewards and active Magnolia/Ficus/Cornus match
the accepted adapter. Empty H1 is committed through the public state transition.
The committed sender nonce is2^264+5; supplied nonce2^512 is preserved by the
request and replaced with committed+1 for execution. The exported account/native
trie and physical seed rows give non-membership authority only for this complete
synthetic seed. The earlier frame touched-row map is not used here.

Eleven fresh cases cover partial300, partial50 below minimum, missing source,
missing destination, insufficient source, source remainder below minimum,
same-validator rejection, malformed input, nonpayable malformed input, native
underfunding and intrinsic underfunding. Each actual probe is repeated with a
fresh disposable DryRunner state. Both pins are identical; physical identity,
all committed rows and state snapshots remain exact before/after.

The [Rust integration test](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_new_destination.rs)
compares all11 outputs, errors, gas and logs through88 fresh sessions: two semantic
owner constructions, two physical reader opens, two repetitions per case.
Initial owner construction finalizes one public empty H1; restart loads head1
without refinalization. Each success creates its destination from the original
committed source, rather than using a warm prior probe. Exact public assertions
retain both validator stakes1,000, aa only source31 and bb only destination32,
each with1,000 principal and zero pending rewards. Exact physical rows are checked
after disposal; total principal alone is not treated as disposal evidence.

Astra medium accepted the complete-seed historical contract before dependent
Rust tests. Sol medium implemented directly. Independent Sol medium frozen final review accepted all8 hashes without
correctness findings. Requested/confirmed runtime was gpt-6.1-sol medium; see
`new-destination-simulation-review.md` in the artifact directory. No routing failure or quota-to-billing inference is claimed.
First-run logs use `new-destination-simulation-` under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`. Actual dual-pin reproduction,
all16 API tests and package clippy pass. Shared corrected frame workspace fast
covers this frozen test source and passes. No failing correction
batch in this simulation slice. Runtime unchanged; accepted ON bridge12/all15
from the adapter remains applicable. Estimation and supported direct traces are
next. Real network history, N1–N6/Milestone10 and production acceptance remain
open; excluded successful branches, no fallback and no push remain in force.
