# Bounded both-current count3 public frames

This settled derivative uses runtime70399811f and the actual count3 oracle
38b5a4498. New producer, harness, corpus and test are separate from all earlier
profiles. No runtime, accepted corpus, support, storage API or upstream C++ changes.

CallerAA is the wrapper; outer senderD1 and common ownerA1 are distinct. The
initial native42-key view is actual immutable genesis authority from the accepted
AA oracle. Real same-period1 transactions first transfer caller30031->32, then
owner10031->32. Both current nodes become count3 and both old0 nodes disappear.
The fullcaller700 target enters this measured state. No positive node count is
inserted. The owner pairs remain900/1100; successful target totals are900/3100,
principal4000 and caller membership[32], with source3->2->3 and destination3->2->4.

Go uses live TransitionState with three Main calls; Rust uses one public native
session and PeriodConsensusSequence across three journals. Prefix callers AA/A1
have supplied nonce1; direct target AA has nonce2, nested outerD1 nonce1. Fixture
sender/wrapper balances1000000 and owner1000 are execution context, not signed
historical or genesis account/root parity. Native escrow is4000. Both languages
use fix/Magnolia/Ficus/Cornus0, Aspen1=0, Aspen2 disabled, blocksperyear1, zero
yield, maxsupply8000, min100/maxstake1000000, eligibility100/step10 and period1.
Timestamp and gas price/value are0 except the named nonpayable control.

Seven actual dual-pin cases cover direct CALL, nested CALL, STATICCALL route,
parent REVERT, two native calls then parent REVERT, nonpayable value1 and native
funding79999. The observer only copies per-call reads/writes/logs; separate
uninstrumented controls compare outcome/raw/accounts/logs/gas. STATICCALL is an
observed opcode route; Go does not expose a native read_only flag. Cumulative
logs preserve both real prefix events; receipt suffixes compare each transaction.
No clear/commit/reset occurs between calls. Refund remains zero.

Prefixes use101912/101848 gas and12/10 ordered native writes. The target writes15
when invoked successfully. Actual total target gas is101912 direct,102678 nested
or parent REVERT,102676 STATICCALL,183422 two-call parent REVERT,29378 nonpayable
and22678 underfunded. Native quote is80000. In two-call control, the second full
removal has normal `Delegation does not exist` failure with no native writes.
Parent REVERT removes target logs and labels successful native facts
OuterFrameReverted; native raw effects retain the exact measured Go behavior.
These are explicit frame facts, not a claim that every effect is rolled back.

The Rust test compares every envelope output/error/gas, supplied context,
per-call quote/disposition/status/output/logs, ordered writes and intermediate
expected raw values. LAST effective native writes remain visible across both
prefixes and later calls. Final write reduction, memberships, source load-copy
stream, raw rows, ordinary balances/nonces and cumulative receipt logs match Go.
Committed public principal, validator totals and owner height stay unchanged.
Previously unenumerated common-owner keys use actual immutable genesis0 versions
from the oracle's complete114 rows via existing public storage codecs, with proof
that none of the actual prefix/target writes modifies them. They are not guessed
bytes. The frame corpus itself claims exported live raw authority, not complete
physical committed rows or trie/root parity.

Required checks: actual dual-pin record/reproduction and uninstrumented controls,
Python compilation, all native-frame regressions, affected EVM package/check/
Clippy, serial rewrite-validate-fast, whitespace and independent frozen Sol
medium review. Unchanged runtime consensus/bridge ON build12/all15 gates remain
separately inherited from70399811f. First outputs are retained in the run3
artifacts. Production routing, general counts/rewards, signed historical APIs,
H2 traces, real-window root parity and broad differential gates remain open.
No fallback, supplied-data mutation or push.
