# Existing-destination current-node public frames

Accepted runtime base `072fc39fb`, oracle preparation `4ec89b7c1`.
New dual-pin producer/harness and separate frame corpus preserve prior files,
fixtures and support. Seven actual pinned Go EVM.Main cases use the accepted
initial frozen42-key native view, real partial30031->existing32, then full700
31->sameexisting32. Both current nodes are produced by the real prefix, not
inserted in the target seed. Prefix has12 writes; successful target has15 writes,
source2->1->2 and destination2->1->3. Native quote80000. Membership [31,32]
becomes[32] through source-first swap removal, no destination append. Repeated
second target returns missing-source with no effects.

The frame shell is explicit and differs from the top-level genesis balance
profile: senderaa and wrapperd1 nonce1/balance1000000; native nonce1/balance4000;
principal4000/votes400, both Aspen parts disabled/maxsupply1000000000,
params.TestChainConfig, period1/timestamp0/price0/value0 except nonpayable1,
transactiongas200000/blockgas1000000. The native zero-state snapshot is reused;
this is not complete genesis/account/trie or historical root acceptance.
Uninstrumented controls compare every parent output/error/gas/log/account/raw
state with instrumented runs, including real prefix/target continuity. Per-call
reads/writes/logs/STATICCALL route vectors and cumulative suffix boundaries are
copy-only observations; no log clearing or cache/commit reset occurs.

Actual gas: direct101912, CALL102678, STATICCALL102676, parent revert102678,
two-child revert183422, nonpayable29378 (quote0), underfunded22678 (quote80000,
Run not entered). Raw native mutations survive parent revert; native receipt
logs revert. Prefix logs stay in the prior transaction. Nonpayable child rolls
back child effects while ordinary parent value/account changes remain measured.

Rust tests use the existing public native session, one sequence across two
transactions and a new journal at the target boundary. They compare every call's
quote/funding/depth/route/input/output/status/logs/disposition, all ordered native
writes/deletes and expected intermediate bytes, final LAST writes including a
later normal failure, exact accounts/nonces and all actual raw rows. Explicit
swap removal witnesses bind count2->1, source-position deletion, destination
move toitem1/position1 anditem2 deletion. Committed FinalChain principal/stakes/
head stay unchanged. Production routing and native runtime remain unchanged.

Independent review found the new Rust frame shell inherited Aspen1=genesis
from Default while the producer explicitly disables it. The new test now sets
Aspen1=MAX; affected checks are repeated. Prior tests and runtime are unchanged.

Required checks: actual dual-pin/control reproduction, Python compile, all
native session frame regressions and EVM package/check/Clippy through serial
fast validation, whitespace and frozen same-profile independent Sol medium
review. Runtime bridge evidence is inherited unchanged from accepted
`072fc39fb`: RUSTAXA_ENABLE ON, build12 and all15 tests passed. Detailed gates,
first-run output and review are saved under the persistent run2 artifact folder
with `existing-current-frames-` prefixes. Signed H1 simulation/estimate/trace,
nonzero rewards, broader membership/later states and production acceptance are
separate gaps. No push, supplied-data/upstream C++/storage mutation, fallback
or broad differential gate is authorized.
