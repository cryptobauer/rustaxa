# Accepted first residual VRF slice

Source commit `958127d777bed4045d37c293a39b6c88875cf6b3` adds one sealed
partial-period diagnostic. Independent acceptance adds 46 VRF facts: 490 to
536. There are 378 residual requests and ten complete voters. Weight, cache,
producer, owner and production authority remain open.

The run evidence is in
`/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014`.
Read `residual-authority-review.md`, `code-acceptance-review.md` and
`residual-first/family-final-review.md`. The final outcome seal has SHA256
`04badad6795a837ee5a9c2e313d4854f5dfd40eb850236a498bdaca281c13019`.

## Fixed scope and invariants

The corrected offline map subtracts 490 accepted full tuples from the original
914 requests. It preserves all original references and order, and rejects
duplicates and conflicting tuples. The resulting set has 424 requests before
this capture.

The selected voter is `03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b`.
The fixed account is `00000000000000000000000000000000000000fe`; the key is
`89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b`.
Payloads use delayed indices 1 through 47 except index 5. All 48 delayed
identities remain in the constructor and retention checks. The omitted period
does not establish a missing, zero or complete-history result.

The first-D prior uses selection 0, parent row 2 and proof 51. The shared owner
authenticates all six raw inputs and freshly consumes all 71 prior proofs before
constructing the residual plan or opening the copy. The new RCV3 family has
46 payloads and exactly 97 proofs: 51 constructor proofs and 46 payload proofs.
Replay consumes the complete bundle and exact EOF. No prefix qualifies after
failure. The ten complete profiles, their 47-payload APIs, and RCV1/RCV2 stay valid.

## Changed interfaces

`ConcreteVrfResidualPlan::new(first)` consumes a validated first-period plan.
It binds only the fixed voter, derives the ordered 46 tuples, and retains the
full context. It accepts no voter, key, root, count or period override. Invalid
context or missing voter returns an error before database access. Its read-only
accessors return the fixed context, voter and tuples.

`capture_vrf_residual_run` and `replay_vrf_residual_bundle` share the existing
storage witness engine. Capture uses the read-only diagnostic owner and produces
the complete RCV3 bundle. Replay has no database input and returns all logical
and physical dispositions plus usage, or a typed failure without a qualified
prefix. Shape validation alone does not authenticate external votes or roots.

The qualifier's fixed `residual_first_preflight`, `residual_first_capture` and
`residual_first_replay` wrappers return `Result<()>`. Their exclusive outside-DB
outputs use the existing path, source, authority and publication guards. The
two fixed CLIs accept no caller profile. The Python builder admits only the
reviewed profile, preparation, source closure, run and exact command. Its marker
is permanently consumed before admitted offline or physical failures.

## Checks and outcome

Source acceptance covers 187 qualifier package/all-bin tests, four focused tests
with ten complete saved-report comparisons, two native/first-D report comparisons,
32 Python controls, six actual invalid-argument cases, canonical parity, format,
clippy and all-bin builds. Storage has 178 tests, including omitted-period history
and missing-node failures with zero payloads. The Rust-enabled storage bridge
build used 12 jobs and all four bridge tests passed. Fast validation, the manual
hook and the source commit's actual hook each passed 2137 tests.

Independent exact READY preceded one guarded capture. Lead and independent full
replays match byte for byte. All 46 values are nonzero 32-byte Members with matching
Present lengths and hashes. The original-profile reverse stops at admission with
zero payloads. Equality of the three fixed metadata records is narrower than full
database preservation. No production route or C++ behavior changed.

All 13 ceilings remain. The command charges 375 occurrences and 418519918 bytes.
The accepted outcome charges 400 occurrences and 419590799 bytes. Named authority
permits one fixed five-part full-byte control record and full failure records in
the final review. Originals, complete outputs, first failures and repeated parent
occurrences remain intact. The separate offline reverse has its own seven-pin
bound. No guard retry, cap increase or recursive evidence expansion is permitted.

The fresh quota baseline is 93%, with a stop floor of 83%. The same budget file
applies throughout this run and any resume. The run checkpoint records the final
allowance, routing, timing, next subset and remaining limits. Local commits only;
no push is authorized.
