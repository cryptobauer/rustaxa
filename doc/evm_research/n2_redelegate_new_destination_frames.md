# New-destination redelegation frames

Baseline: `de32bbc4f`. This test/evidence slice keeps the accepted partial
adapter and uses the existing real EVM driver, journal and FinalChain session.
No runtime, storage or C++ source changes are included.

The [exporter](../../experiments/evm_feasibility/native_redelegate_new_destination_frames_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_new_destination_frames_reference.py)
capture eight actual cases from both unchanged reference pins: direct300,
direct50, nested300, static300, parent revert300, two calls300+300 then parent
revert, nested nonpayable value1, and nested funding79,999. The fixture manifest
records source/support/seed and output/diagnostic identities. Both pins reproduce
identical output. The forwarding observer records one deep-copied entry for each
quote, including an underfunded attempt with Run=false; it keeps setup and Run
reads separate, and copies each call's ordered writes/logs before parent rollback.

The seed is the accepted new-destination first-read map plus explicit synthetic
total-vote200/principal2000 rows. It is a touched-row map, not a complete physical
native snapshot. The semantic genesis has caller d1 source principal1,000 and
other delegator a1 destination principal1,000. Ordinary sender is aa; wrapper is
d1. Magnolia/Ficus/Cornus are active at zero, rewards are zero, minimum is100,
maximum is1,000,000. Direct50 is valid despite being below the minimum because
redelegation has no destination minimum-deposit check. Other-delegator physical
rows are not exported by this corpus and no unchanged-row claim is made for them.

Actual direct300/direct50/nested300/static300 use101,912/101,848/102,678/102,676
transaction gas. Two CALL segments followed by REVERT use183,422 gas; both actual
children receive80,000 gas and succeed. Their ordered streams contain14 then10
raw operations, with one membership append. Both native logs are observed before
rollback and absent from final transaction logs. Parent-reverted native rows
remain advanced. Nonpayability quotes zero and records82,300 child funding with
no native effects; underfunding quotes80,000 and never calls Run. Exact accounts
include the outer sender-to-wrapper value transfer in the nonpayable case.

The [integration test](../../rust/crates/rustaxa-evm/tests/native_session_reference/redelegate_new_destination_frames.rs)
compares all eight transaction outputs, errors, gas, logs, accounts and selected
final raw rows. It compares every native quote/invocation, sequence, funding,
depth, caller, value/static context, result and ordered write/log stream before
journal reduction. Final per-key plans are checked separately and applied only
to the test fixture. Both two-call results have OuterFrameReverted dispositions,
while semantic state and the retained raw overlay agree across calls. Committed
per-validator stake remains unchanged by the private execution session.

First-run commands, full output and exit codes are under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`, prefix
`new-destination-frames-`. Evidence refinements added corpus/vector validation
and separate per-call read capture before freeze. Independent review found
Run reads carried into the next quote setup. The observer now copies then clears
the read buffer; the harness checks read concatenation and empty later setup.
The initial rejected review/freeze remains in artifacts. A diagnostic edit
script failed after saving the bounded correction; its saved output was inspected
before re-recording. Corrected reproduction/tests/fast are required before acceptance.
Final dual-pin reproduction, all11 frame tests, all15 API regression tests,
affected-package clippy and serial workspace fast pass. Runtime is unchanged;
the accepted ON bridge12/all15 gate from the adapter commit remains applicable.

Astra medium accepted the bounded capture contract. Sol medium implemented
directly. Independent Sol medium frozen corrected review accepted all14 hashes after
the read-isolation correction; corrected reproduction, all11frame tests and
corrected workspace fast pass. Report: `new-destination-frames-corrected-review.md`
in the artifact directory. All requested
routes ran without routing failures. No billing inference is made from quota.
Historical simulation, estimation and supported direct traces for the new branch
remain next. Full/zero/new-validator/reward-bearing and historical same-validator
success are excluded. Real-history and production acceptance, N1–N6 and
Milestone10 remain open. No push, fallback or production route is authorized.
