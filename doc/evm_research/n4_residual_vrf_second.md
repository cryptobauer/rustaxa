# Accepted second residual VRF slice

Source commit `d5c9b8de2df9d6d23f571278db9b8dfa340b0584` adds the sealed
b92a profile to the shared residual engine. Independent acceptance adds 46 facts:
536 to 582. There are 332 residual requests and ten complete voters. Weight,
cache, producer, owner and production authority remain open.

Evidence is under
`/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014`.
Read `second-source/residual-authority-review.md`, `code-acceptance-review.md`
in that directory, and `residual-second/family-final-review.md`. The final seal
SHA256 is `0466b004e68244355bb51bd7feb4fe202123d4361cfc2dbd608602cee6c4b386`.

The fixed voter is `b92a29912b08b4fa7404b02dfa2b8a971abdeda1`. Its account is
`00000000000000000000000000000000000000fe`; its key is
`92f670429dc4076aa7771488f1512225a3bbf93eb17aa2e37a8cedcd463fff65`.
The 46 ordered requests use delayed indices 1 through 47 except 36. All 48
delayed identities remain in retention checks. The omission establishes no
missing, zero or complete-history result. First-D selection 12, parent row 38
and proof 63 bind the positive prior. All six raw inputs and all 71 prior proofs
are freshly authenticated before construction or opening the copy.

`ConcreteVrfResidualPlan::new_b92a(first)` consumes a validated first-period
plan. It derives only the fixed voter/key and period subset through the private
shared constructor. Invalid context or missing voter returns an error before
database access. It accepts no caller voter, root, key, count or index. The
first constructor and its omitted index 5 remain fixed. The voter accessor
returns the selected fixed instance. Capture and replay reuse RCV3 with exactly
46 payloads, 51 constructor proofs, 97 total proofs and exact EOF.

The shared qualifier adds fixed `residual_b92a_preflight`,
`residual_b92a_capture` and `residual_b92a_replay` wrappers and two fixed CLIs.
They retain outside-DB output guards, typed diagnostic failures, full prior
authentication and rejection without prefix qualification. The builder admits
only the named profile/run, 78 Rust/owner sources, two tools, exact preparation,
fixed 290 evidence occurrences and two binaries. Unknown or changed inputs
fail closed. The existing ten complete profiles and first residual API remain.

Source acceptance covers 189 package/all-bin tests, six focused tests with
ten complete and first residual saved-report equality, two full native/first-D
report comparisons, 37 Python tests, six actual invalid-argument cases,
canonical parity, format, clippy and all-bin builds. Storage has 181 tests,
including ten residual controls. The ON storage bridge built with 12 jobs and
passed four tests. Fast, manual hook and automatic source-commit hook each
passed 2140 tests. No C++ or production route changed.

Independent exact READY preceded one guarded capture. Lead and independent
replay reports are byte-identical; all 46 values are nonzero 32-byte Members
with matching Present lengths and hashes. The original complete profile rejects
at admission. The first residual profile rejects at payload index 0. Both
reverse controls have zero qualified payloads and freshly authenticate all 71
prior proofs. Equality of three fixed metadata records is narrower than full
database preservation.

All 13 ceilings remain. The command has 375 occurrences and 418995822 charged
bytes. The accepted outcome has 399 occurrences and 420013586 bytes. Its two
offline reverse controls have a separate 11-pin, 390653403-byte bound. All
failures, full commands, outputs and exits remain in the pinned records.

Named authority retains three accepted first-run records and permits six exact
historical representations through the retained first command and fact seal.
All 233 inherited occurrences retain their order and multiplicity. New source
and gate evidence remains direct. The binary pair additionally must fit 420 MiB
and all other command bodies 30 MiB. No cap increase, recursive expansion,
capture retry or first-only outcome packing extension is allowed.

The next e50b 46-request candidate is prepared offline under `luna-third`.
It has no source, capture or fact authority. The run budget remains the original
93% baseline and 83% floor; the checkpoint records the final quota and next
action. Local commits only; no push is authorized.
