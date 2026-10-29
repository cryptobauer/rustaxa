# Token efficiency audit: 01a11b9c

Audit date: 2026-10-10. Session: `01a11b9c-47e5-7f92-a0f8-c4b9d7725b34`, including all 15 descendants. Local logs were read only. The audited session and its agents were not resumed or contacted. Existing repository edits were preserved.

## Result

The run improved cost and coordination. It closed the previous four-file pending slice and produced **22 accepted local commits: 21 code/test commits and one documentation commit**. No owned implementation, gate or review remains pending. Nothing was pushed.

The recorded budget moved **77% → 68% remaining: nine percentage points**, against a ten-point limit and 67% floor. The session stopped before a new profile because fresh reviewer capacity was unavailable under the active workflow. It later closed at the user's request. This was not account exhaustion or a demonstrated model usage failure.

Estimated credits per code/test commit fell about **33%**, and tokens per commit fell about **22%** against the previous audit. Agent coordination also fell. These are diagnostic proxies across different scopes and commit sizes, not a controlled benchmark. Active-window time per commit increased about **28%**, so there is no evidence here of improved throughput per commit.

The next improvements should target repeated correction work and lead context, rather than another model-routing change. Keep independent review and current validation.

## Deduplicated counters

| Counter | Tokens |
|---|---:|
| Input, including cached input | 91,940,737 |
| Cached input, subset of input | 89,229,184 |
| Uncached input | 2,711,553 |
| Output, including reasoning | 463,280 |
| Reasoning, subset of output | 122,662 |
| Total: input plus output | **92,404,017** |

There were 931 unique response usage records across 16 threads. No duplicates or conflicting records were found. Thread and turn counters reconcile. The audit did not sum cumulative snapshots: 956 snapshots include 30 repeated values. Five compaction records were found: four lead and one startup Luna.

Model and effort are taken from runtime metadata, not assignments or reviewer prose. All observed contexts used medium effort. Input includes cached input; output includes reasoning. Neither subset is added again.

Full evidence: [summary.json](/home/fry/artifacts/token-audit-01a11b9c/summary.json), [refinement.json](/home/fry/artifacts/token-audit-01a11b9c/refinement.json), [audit.py](/home/fry/artifacts/token-audit-01a11b9c/audit.py), [refine.py](/home/fry/artifacts/token-audit-01a11b9c/refine.py).

## Model cost profile

Historical Standard credit rates use the same October 1 assumptions as [the previous audit](token_usage_audit_01a10f48.md). Per million uncached input / cached input / output: Sol 50 / 2.5 / 250; Luna 2.5 / 0.25 / 12.5; Astra 250 / 25 / 1,250. These estimates are not billing records or a current price quote.

| Actual model | Threads | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 12 | 784 | 80,268,509 | 78,272,512 | 385,725 | 80,654,234 | 391.91238 | 69.39% |
| gpt-6-luna | 2 | 87 | 7,668,903 | 7,144,960 | 56,835 | 7,725,738 | 3.80653 | 0.67% |
| gpt-6-astra | 2 | 60 | 4,003,325 | 3,811,712 | 20,720 | 4,024,045 | 169.09605 | 29.94% |

Total estimate: **564.814965 credits**. Astra's estimated share fell from 41.96% to 29.94%. Sol supplied the lead and independent reviews. Luna supplied startup/resume checks at low estimated cost. No evidence supports removing useful independent review to save more.

## Work phases and Git correlation

These are execution windows within ONE saved budget. The resume did not create a new ten-point budget. Each response is allocated once by timestamp. Work inside a window includes implementation, validation, review and corrections; the logs do not support a precise causal bill for each activity.

| Window | UTC interval | Tokens | Responses | Estimated credits | Commits |
|---|---|---:|---:|---:|---:|
| Initial execution | Oct 8 13:03:11–16:14:57 | 72,356,042 | 679 | 397.29198 | 16 |
| Resumed execution | Oct 9 14:59:35–16:19:07 | 19,781,199 | 249 | 162.33291 | 6 |
| Requested wrap-up | Oct 10 04:12:43–04:13:07 | 266,776 | 3 | 5.19008 | 0 |

The first task ended with a recorded `cyber_policy` interruption. The audit cannot identify its trigger or attribute a separate token cost to it. Resume retained the saved budget. The three windows total **4h 31m 43s**; the intervening idle time is excluded. This duration includes gate waits and correction work, not just coding.

Git range: `5ff1f0fc5056656d78e5ea6d4a26333d84e6230c..b2f59e4baf991cbc3a4d1bb76206eec0ef47bf9e`. Types: 18 test, two feat, one fix and one docs. The accepted commit index agrees with Git history. Oracle and public-test work often have separate commits, so commit count is not feature count.

Accepted scope groups include selector-only malformed claim failure; commission and delegator sequences; mixed claim orders; positive, guarded and repeated claim frames; finite positive claim-all execution; claim-all trace; literal H1 simulation; and funded H1 estimation. These are finite qualification results. Real-network input qualification, general profiles, root parity, recovery, integrated N4–N6 and production cutover remain open.

Authority: [accepted commits](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/accepted-commits.json), [final closeout](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/final-closeout.json), [final checkpoint](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/final-checkpoint.md).

## Allowance and capacity

Startup observation: 2026-10-08T13:03:36.441Z, 77% remaining. Final run observation: 2026-10-09T16:17:47.558Z, 68% remaining, reset identity `1791948695`. The saved baseline and reset window remained unchanged. These account observations are separate from tokens and credit estimates; other account activity can affect them. The final observation is historical, not the present balance.

[Final quota record](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/final-quota.json) still allowed a start, with one point left. The next profile was not started. Closeout records seven listed threads, no close-thread tool, and no new spawn attempt. Thus the capacity stop followed the conservative repository rule; it was not an observed spawn rejection. Completed listings alone do not prove whether a slot has been released.

At a new profile requiring a fresh reviewer, prepare a compact handoff if capacity is unavailable. Do not repeatedly retry an unchanged failure or waive review. A fresh root session needs new explicit round authorization and a new baseline; the current quota reader binds a budget to its lead session. Do not copy or silently rebase a closed budget to another session.

## Comparison with the previous audit

| Diagnostic proxy | Previous 01a10f48 | Current 01a11b9c |
|---|---:|---:|
| Code/test commits | 65 | 21 |
| Tokens per code/test commit | 5.653M | 4.400M |
| Estimated credits per code/test commit | 40.40 | 26.90 |
| Active-window minutes per code/test commit | 10.14 | 12.94 |
| Lead average input per response | 119,380 | 117,361 |
| All-agent average input per response | 92,852 | 98,755 |
| Astra share of estimated cost | 41.96% | 29.94% |
| Send/agent-wait associated tokens | 14.87% | 5.02% |
| Including follow-ups | 18.12% | 7.19% |

The comparison includes setup, documentation, corrections and wrap-up. This run also paid to close work started in the prior run. Different contracts, test workloads and oracle/public commit splits limit attribution. The lower estimated cost is credible under the fixed historical rates; equivalent-task savings and faster throughput are not established.

## Improvements supported by evidence

1. **Use one tested owner-row snapshot helper.** Review again found a claim of full public owner preservation backed only by selected account/stake/header observations. The separate Go physical snapshot did not cover the Rust owner database. The correction added complete column/key/value observations after every attempt and reopen. A subsequent comparator failure was fixed with the public column descriptors. Two modules now define `owner_rows`. Extract a narrow test helper that retains column names, empty columns, correct comparators and WAL-visible rows. Prove it detects a changed row, including a non-account column, and works after reopen. Use it in new disposal tests instead of copying selected-value helpers. Keep source observations and public owner observations separate.
2. **Reduce broad lead reads.** The fresh session still averaged 117,361 input tokens per response, only 1.69% below the prior lead. Its cached input alone cost an estimated 174.50 credits, about 31% of the total. Early root commands returned roughly 62K characters from combined workflow/scorecard files, 32K from a combined startup read, and later 50K from a broad warning search. These are characters, not tokens or guaranteed recoverable cost. Read mandatory instructions, but select relevant scorecard rows, source spans and failure diagnostics. Parse fixture keys/counts/hashes rather than printing 30K-character JSON samples. Save full outputs as artifacts and inspect bounded summaries.
3. **Plan handoffs at profile boundaries.** Fresh-session startup helped coordination but did not remove large lead context. Lead input medians across response thirds were about 127K, 112K and 99K, with a final-third p90 near 188K. Use a compact checkpoint before a new semantic family or capacity block. Preserve closed evidence and baseline rules. Do not reset a budget to obtain more allowance.
4. **Run cheap contract checks before full gates.** Corrections included stale nonce indexes, expected read-count mismatches, a delegate request assumption and invalid negative-fixture configuration. Check named selectors, matched-test counts, request binding and expected ordered reads locally before full suites. Keep failed outputs. Do not replace actual Go measurements with predictions or reduce parity coverage.
5. **Keep review-after-gates and measured gate reuse.** The coordination change worked. Retain independent review after freeze and final checks. Reuse prior runtime/bridge evidence only when exact source hashes and required dependency scope remain unchanged. The final estimate derivative did this explicitly while running fresh public EVM/check/clippy/serial-fast/whitespace checks. Do not introduce blanket test batching or omit gates without authority.

The full-owner review correction is recorded in [first review](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/claim-all-trace-review.md) and [accepted correction](/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008/claim-all-trace-review-r2.md). The latest derivative recorded no first failure. These results support correcting repeated test patterns while preserving review.

## Coordination detail

Calls included 685 exec, 18 agent listings, 15 spawns, 29 sends, 14 follow-ups, 18 agent waits, 115 process/cell waits and one sleep. The 115 `wait` calls are tool/process waits, not agent coordination; do not add them to the comparable agent-wait measure.

Response contexts with sends or agent waits account for 4,635,295 tokens (5.02%); including follow-ups accounts for 6,647,201 (7.19%). They include useful context and work, not pure waste. Agent-wait contexts total 2,191,677 tokens and 6.73305 estimated credits, about 1.19% of total cost. Seventeen waits were at the lead and one at a reviewer. Further wait reduction is a small cost target compared with lead context.

## Next concrete step

Before another implementation round, prepare a current prompt using accepted HEAD `b2f59e4ba` and the final checkpoint. The previous prompt still names the now-closed malformed-claim work. First perform the bounded owner-row helper cleanup with targeted tests and independent review, then measure the underfunded claim-all boundary oracle from the settled contract. Actual dual-pin measurements must precede dependent runtime changes. Removing the gas guard alone is unsafe. New H1 trace work is another distinct profile with its own contract.

No implementation or instruction cleanup was applied during this audit. Model routing and quota controls can stay as they are. No further AGENTS.md pruning is justified by these counters alone.

## Agent totals

All observed runtime effort is medium. Full records include reasoning subtotals, source logs and counter reconciliation.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a11b9c-47e5-7f92-a0f8-c4b9d7725b34` | gpt-6.1-sol | 606 | 71,120,734 | 69,799,424 | 318,498 | 71,439,232 | 320.18856 |
| /root/startup_luna | `01a11b9d-074b-7372-9b31-56a1e5e63926` | gpt-6-luna | 76 | 7,341,208 | 6,857,728 | 54,363 | 7,395,571 | 3.60267 |
| /root/claims_failure_final_review | `01a11ba6-4b22-72a0-a0b1-f3627f8389ba` | gpt-6.1-sol | 9 | 357,540 | 297,728 | 4,390 | 361,930 | 4.83242 |
| /root/commission_sequence_review | `01a11baf-d988-78e3-b49d-6534f12a6eda` | gpt-6.1-sol | 19 | 871,891 | 816,384 | 7,854 | 879,745 | 6.77981 |
| /root/delegator_sequence_review | `01a11bca-8935-7a72-bac6-dc87709bf719` | gpt-6.1-sol | 22 | 1,187,894 | 1,129,344 | 6,005 | 1,193,899 | 7.25211 |
| /root/mixed_claims_review | `01a11bdc-1f2e-7ed2-b59d-c59862b47b53` | gpt-6.1-sol | 12 | 483,825 | 437,504 | 5,195 | 489,020 | 4.70856 |
| /root/claim_frames_review | `01a11bf5-f5f1-71b1-81d7-74ab19481048` | gpt-6.1-sol | 24 | 1,872,446 | 1,761,920 | 12,682 | 1,885,128 | 13.10160 |
| /root/claim_all_contract | `01a11c0e-48ca-7862-804a-9b02bcceeaf8` | gpt-6-astra | 40 | 2,989,923 | 2,867,200 | 13,888 | 3,003,811 | 119.72075 |
| /root/claim_all_review | `01a11c24-e4b0-7cb0-a31d-986b5620063a` | gpt-6.1-sol | 23 | 1,441,036 | 1,354,880 | 8,020 | 1,449,056 | 9.70000 |
| /root/checkpoint_record_review | `01a11c3c-efe3-7a83-9415-b95276385257` | gpt-6.1-sol | 9 | 331,517 | 295,808 | 2,929 | 334,446 | 3.25722 |
| /root/resume_luna | `01a1212e-5af6-7881-a7f1-5fd91bbd5c8a` | gpt-6-luna | 11 | 327,695 | 287,232 | 2,472 | 330,167 | 0.20387 |
| /root/claim_all_trace_review | `01a12131-9f83-7ba0-ac4e-10346e993eef` | gpt-6.1-sol | 22 | 1,019,829 | 948,480 | 6,383 | 1,026,212 | 7.53440 |
| /root/claim_all_h1_oracle_review | `01a1213d-f311-7933-b82a-c6b84da78c82` | gpt-6.1-sol | 10 | 389,156 | 344,192 | 4,111 | 393,267 | 4.13643 |
| /root/claim_all_underfunded_contract | `01a12149-4a2f-7080-b3ba-75ad79d54ca8` | gpt-6-astra | 20 | 1,013,402 | 944,512 | 6,832 | 1,020,234 | 49.37530 |
| /root/claim_all_h1_public_review | `01a12151-c770-7092-85a5-b495828d4e0b` | gpt-6.1-sol | 10 | 376,883 | 339,584 | 3,541 | 380,424 | 3.59916 |
| /root/claim_all_h1_estimate_review | `01a1215f-b5b2-7df0-87f5-be7306e93dba` | gpt-6.1-sol | 18 | 815,758 | 747,264 | 6,117 | 821,875 | 6.82211 |
