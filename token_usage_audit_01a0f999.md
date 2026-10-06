# Autonomous branch run: multi-slice audit

Session `01a0f999-cdea-7203-b89b-100589964f42`, including all three descendant
agents. Audit date: 2026-10-02. Local logs were read without resuming the session,
contacting its agents, opening supplied databases, or rerunning validation.

## Result

The autonomous workflow delivered **14 EVM slices plus one preparation commit**
in **2h14m41.6s**, from baseline `5bfdf494c` through `327d15fa1`. The working
tree was clean at audit start. Nothing was pushed. The run continued through
multiple ready tasks while a historical-input question remained unresolved.

It consumed **51,405,827 recorded tokens / 581.97019 estimated Standard credits**.
Account allowance went from **29% to 21% remaining**. It stopped near the user's
approximately 20% floor after a completed test/review/commit, with a concrete
resumption plan. No recorded observation crossed the exact 20% guard.

This is a substantially larger and more difficult workload than the previous
2.98M-token fixture-hardening task. Its total is not evidence of an efficiency
regression. The main coordination improvement held: direct Sol implementation,
only three child threads, reused reviewers/helpers, no agent waits, bounded
milestone reports and local commits. The largest remaining cost is Astra review
context, not idle coordination.

## Deduplicated usage by agent and model

All actual models used medium reasoning. Cached input is included in input;
reasoning is included in output. Neither subset is added to totals again.

| Agent / work | Actual model | Responses | Input | Cached input | Output | Reasoning | Total tokens | Estimated credits |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Root: implementation, integration, validation, control | GPT-6.1 Sol | 294 | 33,725,514 | 32,720,000 | 196,843 | 42,635 | 33,922,357 | 181.28645 |
| Reused bounded maps | GPT-6 Luna | 56 | 5,322,800 | 5,118,208 | 27,701 | 16,569 | 5,350,501 | 2.13729 |
| Preparation and settled prerequisite reviews | GPT-6.1 Sol | 30 | 1,346,956 | 1,230,720 | 4,683 | 812 | 1,351,639 | 10.05935 |
| Contract, frozen review and correction review | GPT-6 Astra | 88 | 10,763,209 | 10,333,184 | 18,121 | 1,550 | 10,781,330 | 388.48710 |
| **Total** | | **468** | **51,158,479** | **49,402,112** | **247,348** | **61,566** | **51,405,827** | **581.97019** |

By model: Sol **35,273,996 tokens / 191.34580 credits**, Luna **5,350,501 /
2.13729**, Astra **10,781,330 / 388.48710**. Astra accounts for **66.75%** of
estimated credits; Luna accounts for **0.37%**. Assignments alone were not used
to infer runtime models or consumption.

Estimates use the October 1 rates retained in the
[original audit](token_usage_audit.md): per million uncached input / cached input
/ output, Sol 50/2.5/250, Luna 2.5/0.25/12.5, Astra 250/25/1250 credits.
Formula: `((input - cached) * uncached_rate + cached * cached_rate + output * output_rate) / 1,000,000`.
These are historical-rate estimates, not an invoice or an allowance conversion.

## Commit intervals and slice profile

The following allocates every response once by timestamp between successful
commit commands. These are **interval estimates, not isolated slice charges**:
contract review and helper work can run ahead of the next commit. For example,
the preparation interval includes early native-contract work, and the final
preflight interval includes the next redelegation adapter contract.

| Commit / accepted step | Tokens | Estimated credits |
| --- | ---: | ---: |
| `4148a1a4f` Preparation and quota-reader correction | 3,428,683 | 34.03633 |
| `86e74ff37` Staged validator metadata | 7,940,304 | 70.86708 |
| `8945453bc` Metadata selector-first ABI/admission | 3,967,717 | 30.12290 |
| `1fe3282f2` Metadata frame/API composition | 2,990,890 | 27.94629 |
| `abd7a111d` Metadata actual DryRunner | 3,311,490 | 30.83687 |
| `33e8002ac` Metadata gas estimation | 1,657,661 | 31.55137 |
| `1e49be922` Direct metadata structured traces | 5,331,139 | 57.67637 |
| `0cdef0e9b` Exact active escrow entry | 8,226,176 | 64.93095 |
| `c88907de3` Redelegation storage observations | 3,823,819 | 108.84939 |
| `a3951c811` Redelegation account-port dependency | 1,332,527 | 8.89775 |
| `d2207e387` Escrow actual DryRunner | 1,764,474 | 23.07130 |
| `fb3b56e74` Escrow gas estimation | 1,167,023 | 20.25515 |
| `b00f6ade2` Direct escrow structured traces | 1,766,015 | 25.53599 |
| `32fdf4dbc` Redelegation kernel/serializer composition | 2,170,742 | 30.21106 |
| `327d15fa1` Seven normal redelegation preflight failures | 2,356,229 | 16.49921 |
| Final handoff after last commit | 170,938 | 0.68218 |
| **Total** | **51,405,827** | **581.97019** |

The highest-cost interval was redelegation observation/contract work, at 108.85
credits, despite fewer tokens than the first metadata or escrow implementation.
Token count alone is a poor cost predictor when the model mix changes.

Within related API work, later escrow slices reused the metadata approach:
DryRunner cost 23.07 versus 30.84 credits, estimation 20.26 versus 31.55, and
traces 25.54 versus 57.68. Their elapsed intervals were also shorter. This is
consistent with useful reuse, but scopes and concurrent review work differ;
these are not controlled matched-task experiments.

## Work phases and review value

Root usage combines implementation, validation and orchestration; the logs do
not support attributing every token causally to a source edit or test command.
Luna completed four bounded mapping assignments. The reused Sol reviewer
completed six turns including preparation/correction and settled prerequisites.

A manual interpretation of Astra's 18 visible completion notes separates five
source-contract/feasibility assignments, 12 frozen reviews and one correction
recheck. The initial N1 source review is classified as contract work. Requested
follow-up messages are encrypted in these logs, so these phase labels are inferred.

| Astra phase | Responses | Tokens | Estimated credits |
| --- | ---: | ---: | ---: |
| Contract/feasibility | 30 | 2,976,746 | 102.14310 |
| Frozen review | 55 | 7,566,699 | 278.70195 |
| Correction recheck | 3 | 237,885 | 7.64205 |

Review found a real evidence defect: the metadata oracle recorded wrapper gas
for two direct transactions. It required observing actual native funding/depth,
regenerating evidence, and rechecking the corrected freeze. Sol preparation
review also exposed the quota reader's cross-thread identity weakness and improved
the CLI regression test. These are useful review results, not wasted effort.

Reports record required final checks passing. Runtime changes exercised the
ON consensus bridge with 12 build jobs and all 15 bridge tests; later test-only
steps used affected Rust tests and workspace fast checks. Metadata, escrow and
redelegation reports retain pinned reproduction, actual outputs, failures and
immutable review evidence. This audit checked records and scope, not a new full
correctness review of every changed line.

Some final reviews were conditional on a still-running fast gate. The recorded
gate subsequently passed before local closeout. This overlap reduces latency;
conditional review must remain pending until the condition resolves. A failed
gate or changed source requires correction and the relevant re-review.

## Coordination, context and Git

Recorded tools: **386 execution calls, 19 messages, 25 follow-ups, three spawns,
three agent listings, one user-input request, and zero `wait_agent` calls**.
Process waits inside execution calls are not the same as agent-status polling.
Messages used 1,660,133 response-associated tokens, **3.23%** of total. Including
follow-ups gives 4,829,253, **9.39%**. This preserves the low coordination share
of the prior hardening pilot (10.57% including follow-ups), and is much lower
than the original batch's 38.5% message/wait share.

Average input per response was **109,313 tokens**, versus 51,070 in the small
hardening pilot. Root input medians across response thirds were 132,551,
105,170 and 105,564: root context did not grow continuously. Astra's per-turn
median rose from **64,403 across its first three turns to 217,331 across its
last three**. Reusing a reviewer across metadata, traces, escrow and redelegation
saved setup, but carried a large prior context into later reviews.

Astra cached input alone costs **258.32960 estimated credits**, or 44.4% of the
entire run. This is the largest measurable context cost. Resetting the cheaper
Sol driver blindly is not the first optimization to make.

There were **111 distinct completed Git commands**, 72 from the root, including
15 successful local commits. Filtering excluded three Python commands that only
contained Git text. Git supported baseline checks, scoped diffs, frozen-state
verification and commits. There is no evidence that Git itself explains most
model usage; command-associated responses also carry prior context.

The total diff is 100 files, 7,617 insertions and 168 deletions, including earlier
approved preparation, fixtures and reports. This is not 100 independently
implemented features, and line counts are not a productivity metric.

## Autonomy, quota and remaining acceptance

The run asked once for missing producer facts. When the user said they might be
unrecoverable, it explained the acceptance limit and continued independent
native/API work. It did not invent producer facts or repeatedly request them.
The [checkpoint](doc/evm_research/n6_restart_checkpoint.md) retains their unknown
status and names the next bounded redelegation adapter.

The audit found **109 actual quota-helper invocations**, not the 614 recursive
captures in the raw summary: root 71, Astra 28, Sol six, Luna four. There were
98 fresh allow-start results and 11 stale-observation rejections. The last helper
check at 00:47:52.772 saw 21% remaining with an observation age of 107.4s.
The largest interval between helper invocations was **8m40.7s**, longer than the
intended roughly five-minute cadence. No recorded sample crossed the floor.

First/final account observations show **71% to 79% used**, or **29% to 21%
remaining**. Reported credit balance stayed at 2,118.401685. This does not make
the task free, and eight allowance percentage points must not be converted into
tokens or billed credits. Agent activity and other account usage can overlap.

Stopping at 21% was a near-floor judgment after a complete local slice, not an
exit-2 hard-guard trigger. That fits the user's approximate target and leaves
the larger adapter for a new run. All N1–N6/Milestone 10 acceptance remains open;
the new native/API evidence does not establish historical native completeness,
real-window replay, production cutover, publication or adoption.

## Next improvements

1. **Keep the autonomous driver pattern.** It delivered 14 substantive steps
   without rebuilding a team or pausing after each slice. Preserve targeted and
   fast validation, independent review, honest limits and clean local commits.
2. **Bound expensive review contexts by contract family.** Reuse a reviewer
   within metadata or escrow work; use a fresh explicitly configured review
   context when moving to a substantially different family. Supply settled
   contracts, pins, relevant source ranges, a scope/delta summary and evidence
   paths. Check capacity and preserve Luna's slot; do not spawn a full team.
3. **Use Sol review for settled derivatives.** Keep Astra for authentication,
   gas/rollback semantics, new tracing behavior and unresolved historical authority.
   Already specified parity adapters and ordinary kernel/serializer tests may use
   independent Sol. Do not switch every semantic review merely to lower cost.
4. **Coordinate quota checks through the lead.** Maintain the shared-account
   floor, critical preflight checks and freshness rule. Avoid redundant adjacent
   checks; workers need extra checks when they run long or lack fresh lead data.
   Fix stale-session handoffs and long gaps rather than adding status polling.
5. **Keep startup state compact.** The checkpoint grew back to a commit table
   plus detailed adapter contract. Keep its next action and limits, link the
   scorecard/history for completed slices, and link the full settled contract.

At identical Astra token volume, Sol pricing would be 51.86446 instead of
388.48710 credits, a difference of 336.62264. This is a pricing-only ceiling,
not a forecast or proof of equivalent review quality. The practical experiment
is a few settled derivative reviews, not blanket replacement of Astra.

The next implementation is the already settled staged redelegation adapter:
authenticate required rows before the existing kernel, preserve cold normal
error prefixes, use one raw trace and explicit unsupported-success scopes,
then prove pending/historical and frame/API behavior. Producer qualification
and the real-window gap must stay separate.

## Reproducibility

The audit found **468 unique `(thread_id, response_id)` records**, no duplicates,
**490 cumulative snapshots with 24 repeats**. A follow-up audit on October 3
found two top-level root `compacted` records, at `2026-10-01T23:16:42.529Z` and
`2026-10-02T00:08:58.841Z`. The initial scanner missed this top-level type.
This corrects the earlier no-marker statement; usage totals are unchanged.
Every thread and turn reconciles against its cumulative counters. The root has
three runtime-context records, all Sol medium; those are not evidence of three
separate user-started tasks. Displayed snapshots were not added to usage totals.
This audit's own usage is excluded.

Logs span 2026-10-01 and 2026-10-02 under `/home/fry/.codex/sessions/`:

- Root: `01a0f999-cdea-7203-b89b-100589964f42`.
- Luna: `01a0f99a-4f18-7c73-8dd0-5193e757818c`.
- Sol reviewer: `01a0f99a-cf2a-78e2-bc75-d25b962372d6`.
- Astra reviewer: `01a0f99d-1b71-7400-925c-53440309e280`.

Persistent artifacts: [summary](</home/fry/artifacts/token-audit-01a0f999/summary.json>),
[audit script](</home/fry/artifacts/token-audit-01a0f999/audit.py>),
[slice allocation](</home/fry/artifacts/token-audit-01a0f999/slice_allocations.json>),
[slice generator](</home/fry/artifacts/token-audit-01a0f999/summarize_slices.py>),
[quota/context refinement](</home/fry/artifacts/token-audit-01a0f999/refinement.json>),
and [inferred Astra phases](</home/fry/artifacts/token-audit-01a0f999/astra_role_allocations.json>).
Only this audit report was added to the repository; it is uncommitted.
