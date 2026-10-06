# One-member full+new direct structured traces

Base: `78f228a9c`. New actual [Go exporter](../../experiments/evm_feasibility/native_redelegate_full_new_trace_reference.go)
and [harness](../../experiments/evm_feasibility/native_redelegate_full_new_trace_reference.py)
reuse the unchanged accepted full-new complete H1 seed/snapshot and default
TraceRunner capture owners. Exact79 rows/root/config/accounts match simulation.
Go TraceRunner selects H1 and runs real EVM.Main atH2 with supplied nonces;
Rust uses pending H2-over-H1, with no DryRunner nonce rewriting.

Three independently repeated actual sequences on both pins: single full-new,
full-success prefix then same-direction missing-source target, and stale nonce0.
Full-new target gas101912 succeeds. Missing-source target gas101912 fails normally.
Stale nonce target gas200000 fails consensus admission before native prepare.
All direct structLogs are empty. Diagnostics are retained and hashed separately;
complete committed before/after snapshots are equal. The manifest binds support,
seed input/root, exporter/harness/pins and output/stderr identities.

[Tests](../../rust/crates/rustaxa-evm/tests/native_simulation_reference/redelegate_full_new_trace.rs)
run24 normal trace sequences across semantic owner initialization/restart,
physical reopen and fresh repeats. Actual JSON is compared exactly. Detailed
native effects/error strings are Rust internal assertions, not Go trace fields.
Every successful full-new outcome has17 ordered operations; every expectation is
replayed against its preceding value. Caller item1 Delete then Put32, count0 then1,
source pair/reverse deletion, destination position1/principal1000/cursor2,
validator stakes1000/2000 and final zero-index/current and old-node counts are
explicit witnesses. Native quote80000, funding178088, period2, depth0, caller d1,
position/sequence/input/value and nonce preservation are checked.

Before the next target prepare, every reduced last prefix mutation is compared
with the live journal. This checks caller membership even though the missing-source
failure does not read it. Target outcome has no effects/logs/accounts. Stale nonce
has no prepared or completed native outcome. Committed head1, exact caller/other
principal/order/stakes and full physical bytes remain unchanged after disposal.

Seven hard-failure modes per semantic/reader open give28 failures and28 fresh
retries: prefix prepare, target prepare and each of5 target raw authentication
reads. A test-only NativeJournalRead wrapper forwards accounts/current rows and
injects only during the real inner target invoke. Backing-reader injection could
be hidden by the retained overlay, so it is not used for these reads. Exact reached
keys/order and propagated injected error are asserted. Successful prefix/reused
membership witness occurs before target injection. Each failure drops the port/
session/journal once, returns the exact prefix/target stage, publishes no partial
trace response, leaves committed bytes unchanged, and a fresh retry reproduces
actual JSON with one successful prefix and one normal failed target.

Corrections: initial compilation needed an explicit map import/closure type.
The strengthened validator stake witness first decoded the Magnolia envelope as
a scalar; it now reads the nested legacy row. First target/API failure logs remain.
Corrected target, check/Clippy/all27 API/package/serial fast/whitespace and actual
reproduction pass. Independent Sol review accepted all10 frozen hashes with no
findings (`full-new-trace-review.md`); final pending gates completed
(`full-new-trace-gate-closeout.md`).
Unchanged runtime retains staged ON bridge12/all15 evidence.

Records: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/`,
prefix `full-new-trace-`. Sol medium implements; Astra medium accepted the named
retained-prefix/target authentication contract with stale-nonce correction.
No reverse/current-node full-new success, nested/OE/delayed/RPC tracer scope,
real network history, production/fallback/C++/storage or broad gate is included.
N1–N6/Milestone10 remain open.
