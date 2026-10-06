# Source-last current-node public frames

This derivative retains the bounded source-last runtime and its exact source-last
genesis order [33,31,32]. The new producer seeds only actual initial native rows,
executes real partial30031->33, then full70031->32 through actual EVM frames.
It does not import desired prefix-after state. Old producers/support stay intact.
Fresh same-profile Astra medium settled the seven-case rollback contract in
`source-last-current-frames-contract.md` under the run2 artifact directory.

Actual cases are direct native call, nested CALL, STATICCALL, child success then
parent REVERT, two child calls then parent REVERT, nonpayable value1, and native
funding79999 below quote80000. Both pinned revisions and independent controls
must reproduce these observations. No DELEGATECALL evidence is claimed.

Each Go case uses two real Main transactions on the same state/native cache,
without clearing logs, committing or constructing a root between them. Prefix
has12 ordered native writes. Successful target has16; the repeated second child
has normal missing-source failure and no further writes. Source current node is
2->1->2. Source-last target deletes item2 then appends32 and rewrites count2->1->2.
Retained item1=33 and position33=1 are unchanged across both transactions and
receive no target write. Producer fields and harness explicitly check these rows.

The observer copies per-call RequiredGas setup/Run reads, write and log suffixes
before parent rollback. Independent controls call actual native without the
observer and compare all non-observer output, gas, account, raw and cumulative
log facts. Native dirty raw effects survive parent rollback; target logs revert,
while the prefix log remains. Empty failure cannot erase an earlier raw witness.
Final raw comparison uses LAST write per key; ordered streams stay unreduced.
These are live dirty-row facts, not complete checkpoint/trie/root authority.

The full manifest frame context records price0, timestamp0, period1, txgas200000,
blockgas1000000, aa/d1 balances1000000 and nonce1, native5000/nonce1, direct target
nonce2, prefix caller d1, wrapper d1, TestChainConfig, zero refund, threshold100,
step10, minimum100, maximum1000000, blocks/year1, votes500/principal5000, fix and
Magnolia/Ficus/Cornus0, both Aspen disabled and maxsupply1000000000. This context
differs from StateTransition oracle genesis and is disclosed separately.

Rust constructs real semantic genesis in the same order. One public native
session and sequence serve the real prefix journal, settled disposable state,
and a new target journal. RecordingPort checks cumulative prior LAST effects
before each prepare. Exact per-call ordered outcomes match Go suffixes; replay
checks every intermediate expected value against actual transaction prior rows.
Target settlement plans equal that transaction's reduced effects, including
parent rollback. Retained rows have no target-plan entries. Accounts, gas/status,
output, native dispositions, receipt log suffix and cumulative logs match actual
frames. Committed head0, genesis principal and validator stake remain unchanged.

Producer/control reproduction, Python syntax, frozen independent review, new
Rust target and all frame regressions, EVM check/Clippy/package, serial fast and
whitespace are required before acceptance. This fixture/test-only Tier1 change
inherits accepted unchanged source-last runtime ON bridge evidence. Runtime or
bridge changes would require another bounded subsystem gate. No production,
RPC, historical API, nonzero reward, broader order, storage or C++ scope is added.
Run evidence:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`,
prefix `source-last-current-frames-`.
