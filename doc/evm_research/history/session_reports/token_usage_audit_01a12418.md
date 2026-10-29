# Session audit and remaining work: 01a12418

Audit date: 2026-10-10. Scope: session `01a12418-dc34-7013-a51b-57b6ef22163b` and all13 descendants. Local logs were read only. No audited session or agent was resumed/contacted, and no implementation was changed.

## Assessment

The run completed **14 code/test commits and one documentation commit**, consumed the authorized **ten allowance points**, and stopped with **no owned source, checks, reviews or processes pending**. Accepted HEAD is `f80ed246985fd476c371b605ad96ad1813a21d4f`; nothing was pushed.

The shared full-owner snapshot helper is now accepted. Finite underfunded claim-all, H1/P0 trace, P2 trace and value1 nonpayable behavior gained measured parity and independent review. The final P0 ordinary-prefix trace commit is **Go oracle only**; Rust prefix admission remains the next gap.

Lead average input fell about12% against the prior run. Coordination remained low. Estimated cost for the round was nearly unchanged, while fewer commits were made and more runtime changes were involved. This is useful progress, but not proof of better cost per equivalent task. Repeated copied-test/API/nonce mistakes remain avoidable rework.

**Substantial milestone work remains. This is not the last few cleanup slices.** All six N1–N6 acceptance areas remain open at milestone level. The immediate next prefix slice is bounded; real-network qualification, replay/root parity and recovery are separate major work packages. A reliable completion percentage or number of quota rounds is not supported by the current acceptance inventory.

## Usage counters

| Counter | Tokens |
|---|---:|
| Input, including cached input | 79,507,244 |
| Cached input, subset of input | 77,149,952 |
| Uncached input | 2,357,292 |
| Output, including reasoning | 472,474 |
| Reasoning, subset of output | 118,757 |
| Total: input plus output | **79,979,718** |

The audit found875 unique responses across14 threads. There were no duplicate or conflicting response counters. Thread and turn totals reconcile. Cumulative snapshots were not summed:906 observations include36 repeats. Five compactions were recorded. Actual model/effort metadata shows medium throughout.

Scripts and complete per-agent records: [audit.py](/home/fry/artifacts/token-audit-01a12418/audit.py), [refine.py](/home/fry/artifacts/token-audit-01a12418/refine.py), [summary.json](/home/fry/artifacts/token-audit-01a12418/summary.json), [refinement.json](/home/fry/artifacts/token-audit-01a12418/refinement.json).

## Models and historical estimate

Use the same October1 Standard credit assumptions as [the previous audit](token_usage_audit_01a11b9c.md). Rates per million uncached/cached/output tokens: Sol50/2.5/250; Luna2.5/0.25/12.5; Astra250/25/1250. These are comparison estimates, not an invoice or current price quote. Assigned model names do not establish billing; runtime metadata supplies attribution.

| Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 794 | 75,398,027 | 73,352,832 | 443,433 | 75,841,460 | 396.50008 | 69.82% |
| gpt-6-luna | 17 | 746,966 | 683,008 | 4,166 | 751,132 | 0.38272 | 0.07% |
| gpt-6-astra | 64 | 3,362,251 | 3,114,112 | 24,875 | 3,387,126 | 170.98130 | 30.11% |

Total estimate: **567.864102 credits**. The Sol lead accounts for301.34329 credits; its60,238,976 cached input tokens alone account for150.59744 credits, about26.5% of the total. Astra's share is30.11%, nearly unchanged from29.94% in the previous run. Luna's bounded startup map cost estimate was0.382722 credits.

## Work phases, Git and quota

| Execution window | UTC interval, Oct10 | Tokens | Responses | Estimated credits | Commits |
|---|---|---:|---:|---:|---:|
| Initial work | 04:36:17–07:11:42 | 57,588,164 | 632 | 412.29188 | 9 |
| Resume | 11:56:24–13:02:42 | 22,391,554 | 243 | 155.57222 | 6 |

Both windows used the SAME saved budget. Active-window time totals **3h41m44s**; idle time between windows is excluded. Each response is allocated once by timestamp. These windows include coding, gates, corrections, review and closeout; concurrent activity prevents an isolated causal cost per commit.

The first task ended with a recorded `cyber_policy` interruption. The resume completed the remaining work and preserved its quota baseline. A platform block does not establish a discovered vulnerability; the logs do not prove its trigger. There is no basis to label this as an unpatched vulnerability or to bypass the block. Durable checkpoints and retained outputs enabled recovery.

Git range: `b2f59e4ba..f80ed2469`. Types: ten test, four feat, one docs. The accepted index agrees with15 Git commits. Oracle preparation, runtime admission and public-test derivatives have different scope; they are not15 independent features.

Budget baseline:68% remaining at2026-10-10T04:36:25.690Z; floor58%; final58% at2026-10-10T13:01:52.301Z; reset1791948695. The final reader returned do_not_start. Ten account percentage points were consumed across the whole tree and any concurrent account activity. Percentage points are separate from token and credit units. The final58% is historical, not a claim about current allowance.

Authority: [final checkpoint](/home/fry/artifacts/evm-run-01a12418-dc34-7013-a51b-57b6ef22163b-20261010/final-checkpoint.md), [accepted index](/home/fry/artifacts/evm-run-01a12418-dc34-7013-a51b-57b6ef22163b-20261010/accepted-commits.json), [pending state](/home/fry/artifacts/evm-run-01a12418-dc34-7013-a51b-57b6ef22163b-20261010/current-pending.json), [final quota](/home/fry/artifacts/evm-run-01a12418-dc34-7013-a51b-57b6ef22163b-20261010/quota-final.json).

## Accepted scope and limits

- Shared test-only owner snapshots cover WAL, non-account rows, numeric comparators, empty columns and reopen. This closes the repeated observation-helper gap without production changes.
- Actual dual-pin gas-boundary oracles and bounded P1/H1/P2 runtime terminals are accepted, plus disposable estimation tests.
- Captured P0/H1 funded trace and finite gas-boundary traces are accepted. Captured P0 has82 rows; canonical P1 has92. Reader0/execution1 is distinct from reader1/execution1or2.
- P2 underfunded empty/ordinary-prefix trace tests and finite value1 Cornus nonpayable guard/initial-estimate behavior are accepted.
- Latest ordinary-prefix P0 oracle preserves both source snapshots across pins/repeats/controls. Its transaction1/nativeSeq0 composition does not yet have Rust admission.

Latest runtime evidence records consensus1538, public EVM81, package check/clippy, serial-fast and ON12-job bridge/all15. Exact unchanged runtime evidence is retained for later Go-only oracle work. The final oracle ran seven passing gates in47.35s and had independent final acceptance. No corrections were recorded in that last slice.

Nonpayable scope remains narrow: the Rust guard has an empty native callback lane, while actual Go observations include bootstrap callbacks. The closeout does not claim callback fault-timing parity. These finite adapters do not establish general native behavior, publication or real-network roots.

## Efficiency comparison

| Diagnostic measure | Previous01a11b9c | Current01a12418 |
|---|---:|---:|
| Code/test commits | 21 | 14 |
| Total tokens | 92.404M | 79.980M |
| Estimated round credits | 564.815 | 567.864 |
| Tokens per code/test commit | 4.400M | 5.713M |
| Estimated credits per code/test commit | 26.90 | 40.56 |
| Active-window minutes per code/test commit | 12.94 | 15.84 |
| Lead mean input per response | 117,361 | 103,097 |
| All-agent mean input per response | 98,755 | 90,865 |
| Send/agent-wait associated token share | 5.02% | 4.50% |
| Including follow-ups | 7.19% | 6.83% |

Raw tokens fell13.45%; estimated round cost rose0.54%. Lead mean input fell12.15%. Per-commit proxies worsened, but this run contains four runtime feature commits versus two previously. Different semantic scope, test workloads and split oracle/test commits prevent a like-for-like efficiency conclusion.

Calls included701 exec,17 agent listings,13 spawns,30 sends,15 follow-ups,13 agent waits, five process/cell waits and47 sleeps. Send/agent-wait contexts total3,596,118 tokens; with follow-ups5,462,704. These include useful work, not recoverable waste. Agent-wait contexts alone total1,104,936 tokens/3.90272 estimated credits, about0.69% of the total. Keep process waits and sleeps distinct from collaboration waits. Sleep-associated response contexts total4,337,571 tokens/15.68325 estimated credits. Timing arguments do not establish the reason for each sleep, so this is not a measured gate-polling or recoverable-waste bill.

Repeated corrections came from invented or stale API names, wrong enum/struct shapes, noncanonical zero nonce encoding, copied fault expectations and omitted Tombstone handling. These are not evidence that tests should be weakened. Before writing each derivative, inspect the nearby actual constructor/type signatures and use canonical nonce constructors. Build a small test unit first. Reuse settled fault-injection and request helpers only after checking their invariants; separate historical DryRunner nonce behavior from Main/Trace behavior.

Keep the current model mix, bounded reads, final review after gates and quota handling. The accepted shared snapshot helper is a concrete efficiency improvement. Further gains should come from fewer failed test copies and better prioritization of milestone blockers.

## How much work remains

The [milestone acceptance contract](../../10_existing_network_milestone.md) defines completion. No total remaining task count or equivalent-task benchmark is maintained. A percentage based on commits, fixture count or reconstructed rows would be misleading. The current evidence supports this work map:

| Area | State | Remaining acceptance work |
|---|---|---|
| N1 profile/authority | Partial | Freeze the declared network/fork/native matrix and owner authority; distinguish candidate reference configuration from unverified producer policy. |
| N2 native execution | Many finite profiles accepted; milestone open | Close every path required by the declared profile, including relevant topology/history/gas/error/rollback/serialization combinations. Existing guarded fixtures do not prove general coverage. |
| N3 historical APIs | Many finite simulation/estimate/trace profiles accepted; milestone open | Complete required historical query/native composition and trace/RPC policy, including state identity, unavailable history and retained-reader behavior. Immediate gap is exact P0-prefix Rust trace binding. |
| N4 real-state adoption/replay | Major critical-path work open | Complete native semantic reconstruction and producer/reward/system dependencies; qualify/adopt a copied pair; execute at least one real signed window with exact receipts, outputs and roots. |
| N5 recovery/retention | Bounded lifecycle evidence; integrated work open | Prove paired bootstrap/publication interrupted versus uninterrupted convergence, exactly-once recovery and retained-root safety on qualified state. |
| N6 integrated acceptance | Open | Execute the declared matrix and required integrated gates, independent replacement-boundary review and authorized publication. Production cutover stays separate. |

The last documented N4 reconstruction explains2,282 of23,278 live DPoS rows;20,996 remain unexplained. This is a historical diagnostic of semantic reconstruction, **not a percentage of total implementation completed**. The present run did not close that gap. Reward diagnostics corroborate candidate policy but do not authenticate all producer settings or establish reward-root parity. See [reconstruction evidence](../../n4_native_seeded_undelegations.md), [reward input limits](../../n4_independent_reward_inputs.md) and [integrated checkpoint](../../n6_progress_checkpoint.md).

The broad checkpoint is historical in places; current finite slices are established by their final closeouts. These slices do not supersede the missing N4–N6 exit evidence. There is no basis to claim that one more ten-point run will finish the branch, or to give a reliable calendar ETA while input qualification and profile completeness remain unresolved.

### Immediate next chunk

1. Settle exact single ordinary-prefix binding through the existing Rust trace owner. The current native factory receives concrete identity/execution, not the prefix; transaction1/nativeSeq0 alone does not prove prefix contents or success.
2. Add only the measured P0-prefix admission, preserving transaction0/finite-gas origins and all current guards. Validate exact success, wrong prefix/request/source/sequence cases, faults/retries, full state preservation and old-profile regressions. Complete required gates and independent review.
3. After that finite slice, make the next selection against the N1–N6 acceptance matrix. Prioritize a concrete real-window dependency rather than extending synthetic combinations without a milestone need. A bounded N4 dependency check can proceed independently when authorized inputs and capacity are available.

The oracle is already accepted; do not regenerate it without changed inputs or an unresolved concern. This is one known bounded feature with contract, integration and acceptance work still needed, not a forecast for the whole branch. Final repository checkpoint/queue facts still name interim c07d5495a/59%; next preparation must update them from final f80ed2469/58% and avoid resuming the closed budget.

## Agent appendix

All actual contexts used medium effort. Input includes cached input; output includes reasoning. Full records include reasoning subtotals and reconciliation.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a12418-dc34-7013-a51b-57b6ef22163b` | gpt-6.1-sol | 596 | 61,445,778 | 60,238,976 | 361,623 | 61,807,401 | 301.34329 |
| /root/luna_helper_map | `01a12419-46ab-7d93-baa6-51f56df24bc3` | gpt-6-luna | 17 | 746,966 | 683,008 | 4,166 | 751,132 | 0.38272 |
| /root/sol_helper_review | `01a12424-312c-7ce0-8604-d36026e2fcc7` | gpt-6.1-sol | 8 | 220,123 | 157,184 | 2,046 | 222,169 | 4.05141 |
| /root/astra_quote_bootstrap | `01a12428-d1f5-7bb0-bcc9-fd8f8c8f4f4d` | gpt-6-astra | 13 | 575,024 | 508,416 | 5,182 | 580,206 | 35.83990 |
| /root/sol_boundary_review | `01a1242c-6e02-75a2-a523-4331a0ff7052` | gpt-6.1-sol | 11 | 413,264 | 363,520 | 5,651 | 418,915 | 4.80875 |
| /root/sol_underfunded_runtime_review | `01a12443-b9fc-7303-ade6-8f2a1e1060be` | gpt-6.1-sol | 16 | 792,343 | 733,696 | 6,231 | 798,574 | 6.32434 |
| /root/astra_h1_trace_source | `01a12448-67fa-7f11-a207-8b00800216a3` | gpt-6-astra | 27 | 1,763,610 | 1,682,048 | 11,675 | 1,775,285 | 77.03545 |
| /root/sol_h1_trace_contract_review | `01a1244a-30db-7f33-8454-7410903b494b` | gpt-6.1-sol | 32 | 2,122,376 | 2,000,128 | 14,063 | 2,136,439 | 14.62847 |
| /root/sol_h1_underfunded_trace_review | `01a1246e-707f-7690-9385-1afe6a4c12d6` | gpt-6.1-sol | 27 | 1,905,222 | 1,804,928 | 13,623 | 1,918,845 | 12.93277 |
| /root/sol_underfunded_p2_trace_review | `01a12490-7f04-7542-964c-2644a8395a12` | gpt-6.1-sol | 28 | 2,010,802 | 1,823,744 | 13,468 | 2,024,270 | 17.27926 |
| /root/astra_claim_all_nonpayable_source | `01a1249f-8730-7fd1-8898-7c1920c0ed58` | gpt-6-astra | 13 | 584,178 | 543,488 | 3,926 | 588,104 | 28.66720 |
| /root/sol_claim_all_nonpayable_review | `01a124a2-830b-7ef3-b0c1-1015a1e82dc0` | gpt-6.1-sol | 51 | 5,033,280 | 4,849,792 | 18,252 | 5,051,532 | 25.86188 |
| /root/astra_p0_prefix_trace_source | `01a125de-247a-78b1-b02b-4c1996df1be0` | gpt-6-astra | 11 | 439,439 | 380,160 | 4,092 | 443,531 | 29.43875 |
| /root/sol_p0_prefix_trace_review | `01a125e0-34c3-76a0-a35c-3d78b7b890b0` | gpt-6.1-sol | 25 | 1,454,839 | 1,380,864 | 8,476 | 1,463,315 | 9.26991 |
