# Shared VRF trial audit: 01a1353c

Audit date2026-10-13. The latest implementation session was identified from current
Git history and its final checkpoint as `01a1353c-b1a0-71e3-a562-1d29c68efb96`.
Scope: root and all three descendants. Logs were read only; the audited session
and agents were not resumed/contacted. Existing edits were preserved.

## Result

**The shared-path trial succeeded and the cost profile improved substantially.**
It accepted the common sealed engine, deterministic builder/guard and47 new facts
for voter seven. Cumulative accepted VRF facts are349, with seven complete voters.
All six old accepted windows produced complete equivalent successor reports offline;
old physical captures were not rerun. Required checks, exact-command review and
independent byte-identical proof replay passed.

The run used **three observed quota points**,100%→97%, and stopped because the
specified trial was complete. It did not exhaust the ten-point budget. No owned
code remains pending. Source `3f1eb719d` and docs `3a02629a0` are local; no push.
The shared code admits seven fixed profiles. Voter eight is not yet admitted.

The historical estimate is150.771144 credits. Astra's share fell from74.14% in
[the prior session](token_usage_audit_01a12d5e.md) to32.82%. One common authority
thread and one independent Sol reviewer replaced repeated per-voter authority
threads. This is the intended routing improvement with acceptance checks retained.

## Counters and models

206 unique responses across four logs reconcile against final thread and turn
counters. No duplicate/conflicting usage or missing descendant logs.223 cumulative
snapshots include17 repeats and were not summed. No compactions were recorded.
Actual runtime metadata shows medium effort throughout; assignments alone are
not evidence of cost or routing.

| Counter | Tokens |
|---|---:|
| Input including cached input |21,788,240 |
| Cached input, subset of input |21,167,744 |
| Uncached input |620,496 |
| Output including reasoning |129,776 |
| Reasoning, subset of output |39,105 |
| Total: input plus output |**21,918,016** |

Historical Standard comparison rates are the same October1 assumptions used in
prior audits: per million uncached/cached/output, Sol50/2.5/250;
Luna2.5/0.25/12.5; Astra250/25/1250. These are not invoices or current prices.

| Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 170 | 19,871,487 | 19,398,656 | 114,547 | 19,986,034 | 100.774940 | 66.84% |
| gpt-6-luna | 18 | 932,720 | 856,576 | 8,192 | 940,912 | 0.506904 | 0.34% |
| gpt-6-astra | 18 | 984,033 | 912,512 | 7,037 | 991,070 | 49.489300 | 32.82% |

All206 responses fall inside the single shared-trial work window:
2026-10-13 12:28:54.272–13:37:06.849 UTC, **68m13s** elapsed. Child work overlaps
this interval. Tokens include common design, consolidation, validation, guard
corrections, command/fact review and closeout. The logs do not isolate their causal
costs. Do not price cached input or reasoning again as separate token charges.

Full data and reproducible scripts: [summary.json](/home/fry/artifacts/token-audit-01a1353c/summary.json),
[refinement.json](/home/fry/artifacts/token-audit-01a1353c/refinement.json),
[audit.py](/home/fry/artifacts/token-audit-01a1353c/audit.py),
[refine.py](/home/fry/artifacts/token-audit-01a1353c/refine.py).

## What improved

The old six adapters are thin wrappers around a crate-private sealed Profile engine.
Historical test bodies stay unchanged. Fixed instance data is derived/authenticated
through existing owners, rather than accepted as arbitrary caller voters/roots/keys.
The deterministic Python builder/guard is checked against exact source closures.

Gates passed:113 qualifier library/185 package tests, shared2, tool19, canonical
parity1,19 actual invalid-argv controls, format/clippy/all-bin, fast2130 and both
actual commit hooks2130. Unchanged storage172/ONbridge4 evidence was reused after
106 unchanged pin occurrences and exact source/cache/evidence checks. Changed
qualifier paths had fresh equivalence checks. All13 ceilings,47-payload command
scope, proof barriers, output/deadline caps and single-use markers remain.

Review found guard path/attempt-consumption problems. Before-fix regressions failed,
corrections retained the tests, and Astra's amendment plus independent final Sol
review accepted the fix. The stale correction-in-progress record remains history;
final accepted review owns the closed result. No database operations occurred in
those defect regressions. This is useful quality evidence, not a reason to bypass
review or claim a platform block proves a vulnerability.

One new guarded capture ran. Complete98-proof lead and independent replay reports
match byte for byte; old first profile rejects at Identity/Payload0 with zero
payload tuples. All47 values are32-byte nonzero members with Present provenance.
Fixed metadata equality is narrower than whole-database byte preservation.

Evidence: [final checkpoint](/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/checkpoint-final.md),
[shared implementation record](../../n4_shared_bounded_vrf.md),
[code acceptance](/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/code-acceptance-review.md),
[timing records](/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/timing-record.json).

## Comparison limits

| Diagnostic measure | Prior round3 | Shared trial |
|---|---:|---:|
| New accepted VRF facts |94 |47 |
| Additional reusable result |Per-voter adapters |Shared engine/builder and old-window equivalence |
| Total tokens |57.042M |21.918M |
| Historical estimated credits |701.926 |150.771 |
| Estimated credits per new fact |7.47 |3.21 |
| Tokens per new fact |606,834 |466,341 |
| Active elapsed |94m55s |68m13s |
| Historical allowance consumed |10 points |3 points |

Estimated credits per new fact fell about57%; tokens per new fact fell about23%.
The new trial includes one-time consolidation and guard fixes. The previous round
had two newly accepted voter instances and preparation for the next. These are
not equivalent workloads. The time per new fact did not improve in this comparison;
shared-code investment is part of the current duration. Do not announce a proven
57% future saving or predict facts per allowance point from these windows.

The next same-family instance is the cleanest measurement of the recurring route.
Retain comparable required checks and count corrections, review time and new facts.

## Remaining improvements and next action

1. Keep the common contract and deterministic builder. Extend only the fixed eighth
   sealed instance after its required source/authority delta review. Do not create
   another large copied adapter or repeat the entire common authority analysis.
   Changed selection eligibility still needs reviewed coverage; current code admits
   seven, not every remaining voter automatically.
2. Continue independent Sol review of settled code/instance/exact command/facts.
   Astra handles named new authority questions. One guard per command and independent
   complete replay remain mandatory. Keep old source archives and consumed markers.
3. Bound lead context. Mean lead input was120,912 tokens/response versus123,314
   previously: only about2% lower. Input medians rose61K→123K→177K across thirds.
   Fewer calls and better routing drove the visible cost improvement; there is no
   evidence that long-context cost is solved. Use compact contracts, fixed pin maps
   and delta summaries for the next run, not accumulated voter history.
4. Preserve checkpoint truth. The trial stopped with seven points left because
   the specified scope was complete. Its reader still allowed starts; this is not
   a quota failure. Further work needs an explicit next scope and reviewed eighth
   authority. Do not reset the same saved budget to obtain more allowance. A newly
   authorized fresh session can use a fresh baseline under normal rules.

Coordination recorded23 sends, seven follow-ups, one agent wait, four capacity
listings and three spawns. Send/wait associated contexts total2,192,553 tokens
(10.00%); including follow-ups3,356,894 (15.32%). These include useful work. The
one agent-wait response cost proxy is0.25452 credits, not a wait-service charge.
No process/cell waits or sleeps were recorded. Avoid treating coordination context
as wholly recoverable waste.

Remaining565 VRF+1828 stake/jail facts, lambda48/raw-cache48/producer authority,
voter weights/cache equality, parent owner and real19-transfer replay stay open.
This advances a real N4 dependency; it does not close N4 or production replacement.

## Per-agent totals

All observed effort is medium. Full records include reasoning subtotals and source logs.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a1353c-b1a0-71e3-a562-1d29c68efb96` | gpt-6.1-sol | 105 | 12,695,785 | 12,411,008 | 86,791 | 12,782,576 | 66.964120 |
| /root/luna_map | `01a1353d-acf1-78b2-97df-40c8c7f726b0` | gpt-6-luna | 18 | 932,720 | 856,576 | 8,192 | 940,912 | 0.506904 |
| /root/astra_authority | `01a13540-4320-7161-9ab5-4e938a703f13` | gpt-6-astra | 18 | 984,033 | 912,512 | 7,037 | 991,070 | 49.489300 |
| /root/sol_review | `01a1354b-ece7-7121-b769-ab62811b6393` | gpt-6.1-sol | 65 | 7,175,702 | 6,987,648 | 27,756 | 7,203,458 | 33.810820 |
