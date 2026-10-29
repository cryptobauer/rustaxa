# Residual VRF efficiency audit: 01a13836

Audit date2026-10-14. Latest implementation session identified from Git and artifacts:
`01a13836-67af-7033-9af2-6a6bcd375ba8`. Scope: root and all six descendants.
Logs were read only; no session/agent was resumed or contacted. Existing pending
source and user instruction edits were preserved. No implementation was changed.

## Verdict

**Useful accepted progress, but token efficiency has regressed.** Two separately
authorized ten-point rounds added138 VRF facts:490→628. They also implemented an
unaccepted fourth45-request profile across11 files. Three accepted source and three
documentation commits are local through68a7478c4; nothing was pushed.

The session used211.53M tokens and an estimated1407.42 historical credits. It
spent substantial work on evidence-format limits, inherited pin representation,
full failure-body retention, repeated correction/review and source acceptance.
Those are visible operational bottlenecks rather than proof that the state itself
cannot be read. Exact causal token cost for packaging is not measurable from these
logs; do not label every review token waste.

The shared pipeline still provides meaningful safeguards. The next improvement
should address growing evidence metadata and review context before repeating more
profiles, with an explicitly reviewed successor format. Keep proof authentication,
read bounds, single-use guards and independent final acceptance.

## Work and allowance

| Phase | UTC window, Oct14 | New accepted facts | Tokens | Estimated credits | Historical allowance |
|---|---|---:|---:|---:|---|
| R1 |02:20:52.601–05:28:28.788 |92 |98,696,306 |671.55068 |93%→83%,10 points |
| R2 |13:50:00.307–17:42:52.767 |46 |112,587,567 |723.62865 |83%→73%,10 points |
| Status answer |19:12:06.600–19:13:48.314 |0 |249,477 |12.24513 |Separate status turn |

The two work windows total7h00m29s, excluding idle time. Status adds1m42s.
Each response is allocated once by timestamp; concurrent agents and advance work
prevent treating phases as isolated per-profile bills. The two saved budgets
consumed20 observed points. Account-wide activity and delayed/rounded telemetry
limit billing inference; percentage points remain separate from tokens/credits.

First two residual profiles each accepted46 facts under97-proof full replay and
fresh71 prior proofs. The third accepted another46. Independent lead/reviewer
reports match, reverse controls qualify zero, and exact-command review preceded
one capture for each. All markers are consumed; no rerun is authorized.

Final accepted outcome is628/914 VRF facts.286 VRF plus1828 stake/jail facts remain,
with48 lambda selections,48 raw expected cache rows, producer/policy authority,
weights/cache equality, parent owner and real19-transfer transition still open.
These counts measure input coverage, not overall Go-EVM replacement completion.

Final authority: [R1 checkpoint](/home/fry/artifacts/n4-vrf-residual-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/checkpoint-final.md),
[R2 current checkpoint](/home/fry/artifacts/n4-vrf-residual-round2-01a13836-67af-7033-9af2-6a6bcd375ba8-20261014/checkpoint-current.md),
[accepted third record](../../n4_residual_vrf_third.md).
The broad repository checkpoint still describes the older490-fact run; current
residual artifact checkpoints take precedence.

## Counters and model costs

1753 unique response records across seven logs reconcile with both thread and
turn counters. No duplicates, conflicts or missing descendants.1910 cumulative
snapshots include167 repeats; snapshots were not summed. Ten compactions: three
lead, five Luna map, two Sol acceptance. All actual contexts used medium effort.

| Counter | Tokens |
|---|---:|
| Input including cached input |210,371,561 |
| Cached input, subset of input |205,206,528 |
| Uncached input |5,165,033 |
| Output including reasoning |1,161,789 |
| Reasoning, subset of output |454,533 |
| Total input plus output |**211,533,350** |

Historical October1 Standard rates match prior audits: per million uncached/cached/
output, Sol50/2.5/250; Luna2.5/0.25/12.5; Astra250/25/1250. The estimates are
not invoices or current prices. Cached input/reasoning are subsets, not extra tokens.
Actual model metadata supplies attribution, not the requested assignments.

| Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 1,214 | 157,942,216 | 154,607,232 | 722,365 | 158,664,581 | 733.85853 | 52.14% |
| gpt-6-luna | 386 | 33,920,983 | 32,610,560 | 377,186 | 34,298,169 | 16.14352 | 1.15% |
| gpt-6-astra | 153 | 18,508,362 | 17,988,736 | 62,238 | 18,570,600 | 657.42240 | 46.71% |

Total estimate1407.4244525 credits. Largest individual costs: Astra residual
contract488.45, Sol lead420.24, Sol acceptance235.16, and Astra45-profile authority
168.98. Luna map16.14 is a small financial target despite34.30M tokens.

Full counters/scripts: [summary.json](/home/fry/artifacts/token-audit-01a13836/summary.json),
[refinement.json](/home/fry/artifacts/token-audit-01a13836/refinement.json),
[audit.py](/home/fry/artifacts/token-audit-01a13836/audit.py),
[refine.py](/home/fry/artifacts/token-audit-01a13836/refine.py).

## Efficiency signal and limits

| Diagnostic proxy | R1 | R2 | Prior shared trial |
|---|---:|---:|---:|
| New accepted facts |92 |46 |47 |
| Estimated credits |671.55 |723.63 |150.77 |
| Credits per accepted fact |7.30 |15.73 |3.21 |
| Tokens per accepted fact |1.073M |2.448M |0.466M |
| Work-window minutes |187.60 |232.87 |68.21 |

R2 credits per accepted fact rose about115% against R1. R2 also paid for the
unaccepted45-profile implementation, so the denominator understates its completed
engineering work. The residual contracts required new storage/proof semantics,
while the prior shared trial covered a different47-row class and consolidation.
These are not equivalent tasks or proven like-for-like regressions. Nevertheless,
absolute costs, context growth and artifact corrections show concrete overhead
that should be fixed rather than forecasting more rounds at this pace.

Lead mean input128,689 tokens/response versus120,912 in the shared trial, about6%
higher. The lead made757 responses and used97.85M total tokens. Reusing a long
root across rounds preserved state but increased repeated context; shorter final
messages alone cannot resolve this cost. Cached lead input alone estimates239.94
credits, about17% of the session total.

Coordination:259 sends,67 follow-ups,62 agent waits,84 sleeps,21 capacity listings,
six spawns,1169 exec. Send/wait-associated contexts account for37.04M tokens
(17.51%); including follow-ups46.81M (22.13%). They include useful work/context,
not direct waste. Agent-wait contexts alone estimate75.63 credits,5.37% of total.
Idle-message routing/scheduling mistakes are explicitly retained in checkpoint
records; new assignments to an idle agent should use followup_task, not a status
message that does not trigger work.

## Evidence bottleneck

The first outcome hit400 occurrence pins; second/third399. Third preparation
projected401 occurrences and needed a specific reviewed representation decision.
Later work required review/freeze documents below128KiB while carrying complete
older failure bodies. The pending source closeout calls for an82602-byte Python
failure body, multiple further failure/decision bodies and a749065-byte retention
successor. These are measured artifact requirements, not hypothetical warnings.

This caused a cycle of projection, representation, cap negotiation, code corrections,
full gate runs and more retained evidence. Strict physical proof limits are useful;
embedding growing historical logs in every authority/review record is a separate
format choice. The current contract requires it, so an agent must not simply omit
records, deduplicate charged history or raise caps to pass.

Useful defects were fixed and preserved: bad helper tuple/path assumptions, fixture/
schema mismatches, current source pin updates and wrong-profile reverse helper
handling. Required storage/bridge/parity checks passed for accepted work. The pending
source has passed listed gates too, but **gate success is not source acceptance**.
Final frozen-source review and command authority remain absent.

## Recommended fixes

1. **Close the pending45-profile first under fresh authorization.** Verify its current
   11-file freeze and complete gates. Finish independent source acceptance/local
   commit before exact-command READY, capture and independent proof replay. Do not
   rebuild or reset already passed work merely because old checkpoints are verbose.
   Do not claim45 facts before final acceptance.
2. **Review a bounded evidence-format successor before more profiles.** Store full
   first-failure logs once as immutable artifacts. Use exact path/size/digest and
   concise finding/status records for review, rather than repeatedly embedding all
   failure bodies. Separate mandatory raw proof/source/command authority from a
   bounded historical provenance index. Specify deterministic verification and
   finite parent depth/closure, no arbitrary embedded paths or unbounded traversal.
   Retain and authenticate all required history; no automatic cap increase or
   silent change to current occurrence accounting. Astra reviews the changed
   authority/representation once; Sol reviews implementation and instance deltas.
3. **Keep common residual logic shared.** Different cardinality/omission profiles
   need explicit semantic review, but should become sealed descriptors once that
   behavior is accepted. Avoid another large copied pipeline or repeated complete
   analysis of unchanged builders. New proof/retention assumptions still escalate.
4. **Use compact fresh-root handoffs between authorized rounds.** Include accepted
   HEAD, pending source hashes, settled contracts and exact next gates. Link history.
   New root/new budget needs explicit authorization; preserve every old budget.
5. **Make prechecks machine-readable.** Before full suites, validate fixed candidate,
   tuple shape, source pins, artifact-fit accounting and guard builder. Use the same
   tested builder/verifier for reviewer evidence. Schedule idle workers with an
   actual follow-up task and retain independent review after gates/freeze.

Do not reduce proof checks, full-consumption, single-use semantics or regression
coverage. This audit identifies improvements; it does not authorize a new evidence
schema or acceptance of pending code. No workflow or implementation changes were
applied during the audit. The report is the only new repository file.

## Agent appendix

All actual effort is medium. Input includes cached input; output includes reasoning.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a13836-67af-7033-9af2-6a6bcd375ba8` | gpt-6.1-sol | 757 | 97,417,337 | 95,975,552 | 432,836 | 97,850,173 | 420.23713 |
| /root/residual_map | `01a13837-10ad-7ab3-b0f1-0ae523a859fa` | gpt-6-luna | 386 | 33,920,983 | 32,610,560 | 377,186 | 34,298,169 | 16.14352 |
| /root/residual_authority | `01a1383a-a5fa-7480-8be2-54adf7bd0752` | gpt-6-astra | 110 | 14,106,704 | 13,719,552 | 38,936 | 14,145,640 | 488.44680 |
| /root/residual_acceptance | `01a1383c-1aca-7b12-9c70-41b033e0d6b5` | gpt-6.1-sol | 284 | 47,059,193 | 45,794,816 | 229,830 | 47,289,023 | 235.16339 |
| /root/residual_storage | `01a1383e-7a87-7aa1-a5dd-a513cc5d49ac` | gpt-6.1-sol | 65 | 5,201,238 | 4,827,136 | 28,765 | 5,230,003 | 37.96419 |
| /root/residual_second_qualifier | `01a1389a-1d8d-7d01-9314-c8367aa963cb` | gpt-6.1-sol | 108 | 8,264,448 | 8,009,728 | 30,934 | 8,295,382 | 40.49382 |
| /root/residual_45_authority | `01a13af5-d29a-72b0-b11b-fdb625861868` | gpt-6-astra | 43 | 4,401,658 | 4,269,184 | 23,302 | 4,424,960 | 168.97560 |
