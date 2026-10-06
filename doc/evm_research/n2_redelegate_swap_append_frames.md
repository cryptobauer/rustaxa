# Source-first two-member full source/new destination frames

Base: `e9c726834`. This derivative changes only tests and actual reference
capture. The accepted [staged slice](n2_redelegate_swap_append.md) owns runtime.

[Exporter](../../experiments/evm_feasibility/native_redelegate_swap_append_frames_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_swap_append_frames_reference.py)
execute seven cases on unchanged public/local Go pins: direct, nested, static,
parent revert, two calls then parent revert, nonpayable value1 and funding79999.
The source seed is first-attempt actual swap-append read rows plus seed inspection;
it is a touched-row map. All three validators remain; caller d1 has1000 on31/33;
a1 has1000 each31/32/33. Explicit synthetic vote500 and principal5000 rows are added.
No complete physical checkpoint or unchanged unexported a1 physical rows are claimed.

The frame context differs from the StateTransition seed configuration. Outer
sender aa and wrapper d1 each have balance1000000/nonce1; native custody has
balance5000/nonce1. Period1, block gas1000000, transaction gas200000 and price0;
fix/Magnolia/Ficus/Cornus active0, both Aspen parts disabled, supply1000000000,
minimum100/maximum1000000/threshold100/step10. Manifest records the separate
frame context, seed/source/support/harness/output/diagnostic hashes and pins.

Actual direct/nested/static gas is101912/102678/102676. Parent revert uses102678;
two calls plus revert uses183422. The first call succeeds with19 ordered writes
and one event. The second same-direction call reports Delegation does not exist
with no writes/logs. Nonpayable uses29378 gas; underfunded uses22678 and never
calls Go Run. Observations copy each quote and Run stream before parent rollback,
keep setup/Run reads separate and clear the read buffer between calls.

[Integration test](../../rust/crates/rustaxa-evm/tests/native_session_reference/redelegate_swap_append_frames.rs)
compares exact transaction gas/output/error/logs/accounts and each quote/funding,
sequence/depth/caller/value/static context, native status/output/logs and ordered
stream. It replays every expectation against the prior intermediate value with
accepted empty/delete logical equivalence. Caller item2 Delete then Put32 and
count1 then count2 are explicit assertions; source reverse position is deleted,
destination reverse position2 inserted. Moved third position33=1 and item1=33
are also explicit assertions. Final reduced native plan preserves these
results, source delegation deletion, stakes and actual selected final raw bytes.
Before the second quote, every reduced first-call effect is checked in the live
journal. Its source-missing failure confirms private semantic progress for that
failure prefix. Complete private semantic/a1 facts remain staged evidence.

Parent revert preserves native raw changes, removes the native event from final
logs and follows ordinary account rollback. First/second Rust dispositions are
OuterFrameReverted/OwnFrameReverted. Value1 nonpayability matches actual outer
sender-to-wrapper transfer. Applying the plan is confined to the disposable
fixture. Public committed validator stakes/principal and head remain unchanged;
no private accessor or runtime hook is added.

Actual record/reproduction, targeted test, all15 frame/all27 API/EVM package
tests, check/Clippy/serialfast/whitespace passed. Independent same-profile Sol
medium accepted all12 frozen hashes with no blocking findings. Final status and
doc hash are in `swap-append-frames-closeout.json`. Runtime is unchanged; retain staged ONbridge12/all15 evidence.

Evidence: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`,
prefix `swap-append-frames-`. Sol medium implements; The accepted staged contract and settled
full-new frame contract govern this mechanical derivative. No production routing, fallback, C++, storage
change, complete historical API, rewards/longer-list/current-node extension or
broad gate is included. N1–N6/Milestone10 remain open.
