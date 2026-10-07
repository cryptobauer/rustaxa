# Restart checkpoint: run3 closed

Branch `feat/rust/evm-state-db`; accepted runtime `a16acef1e`. No push.
N1–N6/Milestone10 and production cutover remain open.

Run3 session01a10f48-83ee-7a32-8bff-722d10c36359, actualgpt-6.1-sol medium.
Artifacts: `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r3-20261007`.
Saved quota-budget.json baseline99% at2026-10-07T12:55:05.674Z, stop89%,
reset1791948695. Floor reached at2026-10-07T16:08:04.993Z:99->89%,
10percentage points consumed. Saved floor observation at16:08:52.757Z confirms89%.
No new work, delegation or large gates started after the floor. Only the in-flight
source report, assigned review and minimal checkpoint closeout continued.
Preserve this baseline. A later authorized run needs a new run directory and fresh
baseline; do not reset or resume this closed budget. Nothing was pushed.
Run1 closed66->56/10points/19commits; run2 closed56->46/10points/20commits through
3b8d2e31e. Their budgets/reset1791584594 stay immutable. User authorized run3
after weekly reset; no prior budget was reset. Consumption is separate from billing.

Run3 accepted work:

| Scope | Commit | Evidence |
| --- | --- | --- |
| Count2 signed H1 simulation | bae1774ed | 105-row identity;248sessions/120faults120freshretries |
| Count2 H1 estimate | 8d2d7bcec | 8actualGo probes/64Rust probes/24search calls |
| Count2 H2 trace | 621596acc | 264sessions/128faults128freshretries;old1 deleted |
| Count3 oracle | 38b5a4498 | RealAA300/O100/full700;dualpin/control114rows |
| Exact count3 runtime | 70399811f | 322faults322retries44exclusions;consensus1510/ONbridge12all15 |
| Count3 public frames | e8e439ab0 | Sevenactualcases/threejournals;complete diagnostics ff4086218 |
| Count3 signed H1 simulation | 85f846521 | Two canonical signers/twotx/108rows;472sessions232faults232retries |
| Count3 H1 estimate | c9aa6ca72 | 8actualGo probes/64Rust probes/24search calls |
| Count3 retained-owner H2 trace | e9cc1b24d | 488sessions240faults240retries;old1count1 retained |
| Count4 repeated-owner oracle | 0cea670f9 | RealAA300/O100/O100/full700;dualpin/control114rows |
| Exact count4 runtime | a16acef1e | 370faults370retries46exclusions;consensus1515/ONbridge12all15 |
| Count4 public frames | b4f62f303 | Sevenactualcases/fourjournals;18frozen paths including8diagnostics |
| Count4 signed H1 simulation | be90ad91f | Twosigners/threetx/108rows;472sessions232faults232retries |

| Count4 H1 estimate | 67f422b5e | 8actualGo probes/64Rust probes/24search calls; full public-owner isolation |
| Count4 retained-excess H2 trace | 05e92e685 | 488sessions240faults240freshretries; old1count2 retained |

All named slices pass required actualdualpin reproduction/Python/targeted and
package/check/Clippy/serialfast/whitespace gates and independent frozen Sol
medium review. Runtime changes pass ON consensus bridge build12/all15; tests
inherit exact unchanged runtime bridge evidence separately. Closeout.json,
review.md and freeze.json records in run3 bind commands/exits/identities/failures.
Root is sole source/gate/producer/commit writer. Luna startup was confirmed;
Astra handles only named unsettled contracts; actual routing and earlier failures
stay in reports, without billing inference. Completed audited runs are not resumed.

Count4 profile: actual genesis31/32 with commonowner, caller+owner1000 each.
Three period1 partials caller300/owner100/owner100 leave caller700/1300,
owner800/1200, totals1500/2500, current nodes4, all heads/four cursors1, old0
absent. Fullcaller700 preserves source4 and raises destination5. Exact dynamic
count4 helper is separate from unchanged count2/count3; no general higher counts.
Public frames compare all four journals, ordered intermediate/LAST raw, gas/logs,
accounts and dispositions. Parent REVERT drops targetlogs while native raw effects
retain measured Go behavior. STATICCALL route is distinct from native read_only.

Canonical Ckey31/OkeyA1 derive77952ce83ca3cad9f7adcfabeda85bd2f1f52008 and
5d5c99edf529335160ff180fa141dd4967fc00d2. Signed noncesC0/O0/O1 finalize once
in H1. Cnonce1/Ononce2/native1, balances3000/1000/4000, supply8000, locking2/3/7.
Complete Go root a8ea1ecbd354137b6ed5d14084c673eb9bd959979175f110b24cd20a56903b71
has108rows. SameH1 DryRunner full700 uses native invocation1/EVMcontext2 and
private effective2, preserving supplied2^512. No Rust/Go root equality is claimed.
Review closed a new physical isolation gap: every public owner Column row and
all three receipts/metadata are checked after every attempt/disposal/reopen,
separately from complete Go seed rows. Initial wrong-path correction is retained.

Count4 H1 estimate is accepted: unchanged C++ search returns104977, minimum
101912, OOG101911. Every probe preserves all public owner column rows, three
receipts, metadata and complete Go H1 seed authority.
Count4 H2 trace is accepted: actual default JSON plus a separate identical-H1
live StateTransition witness. H2 partial300 reduces old1count4->3->2, retaining
owner800/1200, then full400 uses unchanged count2 helper. Target current2 source
count2->1->2/destination2->1->3; old1count2 stays unchanged. The 15 target
keys exclude old1 and owner pairs; separate state witnesses prove preservation.
No exported Go ordered H2 write-vector or broader runtime admission is claimed.

Next dependency: measure the positive-reward partial source contract in
[n2_rewarded_partial_redelegate_contract.md](n2_rewarded_partial_redelegate_contract.md).
It proposes yield20, P1 reward distribution/commit, then P2 partial300. Payout396
is a source prediction, not an actual measured result. Exact dual-pin reward,
account, raw-write, log and committed-state evidence comes before adapter work.
The source-only report and its review were assigned before the floor; actual
Astra startup was90%, Sol was assigned at90% and observed89% at startup. No
producer, runtime change or test follows from preparation alone. Review and
hash records are in the run3 directory. New execution needs a new authorized run.

General counts/rewards, producer configuration/override/capture inputs, qualified
real-window receipts/outputs/root parity, recovery and integratedN4–N6 remain
open. No supplied-data/upstreamC++/storageowner exception, protocol/fallback,
production route, broad/differential gate or push is authorized.
