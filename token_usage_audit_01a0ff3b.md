# Token audit: 01a0ff3b

Audited session: `01a0ff3b-ab12-7902-b7a9-164209e8305f` and all four
local descendant threads. Audit date: 2026-10-03. Logs were read only; no audited
session or agent was resumed or contacted. The report and derived audit artifacts
are new outputs, not changes to the original logs.

## Assessment

The run delivered the prepared redelegation chunk and several further scopes.
It completed **17 implementation slices**, plus one preparation commit and one
closeout commit. Elapsed time: **2 h 54 min 35.1 s**. The account observation
moved from 100% to 80% remaining, and new implementation stopped at the floor.
All accepted slices record required checks and independent acceptance. Nothing
was pushed. Historical network qualification and production acceptance remain open.

The changes improved review routing, but did not reduce measured total cost per
implementation slice. Sol reviewed settled derivatives and found two important
evidence defects. Astra still accounted for 66.8% of estimated credits. Long
review contexts and increased coordination remain the main opportunities.
The scopes differ from the earlier run, so commit-based averages are diagnostic
proxies, not a controlled benchmark or proof of equal work.

## Deduplicated token totals

| Counter | Tokens |
| --- | ---: |
| Input, including cached input | 90,499,522 |
| Cached input, a subset of input | 87,995,904 |
| Uncached input | 2,503,618 |
| Output, including reasoning | 397,437 |
| Reasoning output, a subset of output | 90,907 |
| Total input plus output | **90,896,959** |

There are **762 unique response records**. Deduplication uses
`(thread_id, response_id)`. No duplicate response usage records were found.
All five thread totals and all recorded turn totals reconcile against cumulative
counters. The 812 cumulative snapshots include 56 repeated values; these were
not summed. Six explicit top-level `compacted` records were found: root three, Luna two
and Astra one. The initial scanner missed this top-level log type; the refined
scanner corrects it. The preceding run also has two root compaction records,
and its earlier no-marker statement is corrected in that report. Token totals are unaffected.

## Model and agent costs

Every observed route used medium reasoning. Models come from actual
`turn_context` records, not assigned role names.

| Model | Responses | Total tokens | Estimated credits |
| --- | ---: | ---: | ---: |
| gpt-6.1-sol | 505 | 58,033,864 | 283.807440 |
| gpt-6-luna | 135 | 14,901,997 | 6.162273 |
| gpt-6-astra | 122 | 17,961,098 | 584.490700 |
| Total | 762 | **90,896,959** | **874.460413** |

| Agent / responsibility | Thread | Responses | Total tokens | Estimated credits |
| --- | --- | ---: | ---: | ---: |
| Sol direct implementation lead | 01a0ff3b-ab12-7902-b7a9-164209e8305f | 410 | 48,248,300 | 234.142560 |
| Luna bounded maps | 01a0ff3c-6c25-7aa3-b3c0-d79ca02d5bb3 | 135 | 14,901,997 | 6.162273 |
| Sol preparation / bounded review | 01a0ff3d-be28-7da3-b20d-bb1f568c5a26 | 18 | 999,302 | 8.380220 |
| Astra contract / risky review | 01a0ff46-bb77-7ec2-8ef5-1cccfda5d99a | 122 | 17,961,098 | 584.490700 |
| Sol settled API / frame review | 01a0ff61-5da4-7d82-a594-07a181d97cba | 77 | 8,786,262 | 41.284660 |

These are estimates using the same historical October 1 Standard credit rates
as the [previous audit](token_usage_audit_01a0f999.md). Rates per million uncached
input / cached input / output tokens: Sol 50 / 2.5 / 250; Luna 2.5 / 0.25 / 12.5;
Astra 250 / 25 / 1,250. This is a comparable historical-rate calculation, not a
current price quote or actual invoice. Cached input and reasoning output are
subsets and are never added a second time. Account allowance percentages are
not converted into credits or tokens.

## Comparison with the preceding run

| Measure | Previous 01a0f999 | This run |
| --- | ---: | ---: |
| Accepted implementation slices | 14 | 17 |
| Preparation / closeout commits | 1 / 0 | 1 / 1 |
| Elapsed | 2 h 14 min 41.6 s | 2 h 54 min 35.1 s |
| Total tokens | 51,405,827 | 90,896,959 |
| Estimated credits | 581.970195 | 874.460413 |
| Credits per implementation-slice proxy | 41.57 | 51.44 |
| Tokens per implementation-slice proxy | 3.67M | 5.35M |
| Minutes per implementation-slice proxy | 9.62 | 10.27 |
| Average input per response | 109,313 | 118,766 |
| Responses associated with messages/waits: token share | 3.23% | 9.53% |
| Including follow-up assignments: token share | 9.39% | 14.88% |
| Astra estimated-credit share | 66.75% | 66.84% |

The proxies include all preparation, review, overlapping next-contract work and
closeout costs, divided by implementation count. On this basis credits rose
about **24%**, tokens about **46%**, and elapsed time per slice about **7%**.
The new run includes difficult authentication, absence, deletion and historical
composition work. These ratios cannot distinguish extra semantic difficulty from
avoidable overhead. They do show that cost improvement has not yet been proven.

Messaging figures count all tokens in responses containing the named tool calls.
They include useful context and other work, not just message text, and are not
a precise measure of waste or recoverable savings.

97.23% of input was cached. This reduces estimated cost but does not make growing
context free. Astra cached input alone accounts for **439.7152 estimated credits**,
about half the run total. Shorter output prose helps, but input context is the
larger cost target.

## Work phase and Git correlation

Nineteen successful root commit command events define the timeline boundaries.
All 762 response records allocate exactly once, including one final handoff
response. Each interval includes work by all agents during that period. Ahead
contracts, overlapping implementation and deferred commits mean these are
**timeline allocations, not isolated slice charges**. For example, most
new-destination simulation work ran before the frame correction commit; its
own commit interval is therefore unusually small.

| Timeline phase | Commits | Tokens | Estimated credits |
| --- | ---: | ---: | ---: |
| Preparation | 1 | 2,320,958 | 12.352405 |
| Partial existing destination: adapter/frame/API | 5 | 31,766,406 | 269.103196 |
| New caller destination: adapter/frame/API | 5 | 25,309,029 | 234.205594 |
| Full caller-source removal: adapter/frame/API | 5 | 21,973,477 | 231.256406 |
| Zero existing pairs: adapter/frames | 2 | 8,472,219 | 116.033670 |
| Checkpoint and next contract | 1 | 900,561 | 10.993610 |
| Final handoff after closeout commit | 0 | 154,309 | 0.515530 |

The largest individual interval is partial historical simulation at 96.43
estimated credits; it includes ahead contracts. New-destination adapter work is
85.42, and zero frames 77.75. Contract/review cost is attributed to actual agents
above; Git command count alone is not a measure of waste. Full commit intervals,
timestamps and per-model allocations are in
[refinement.json](/home/fry/artifacts/token-audit-01a0ff3b/refinement.json).

## Account allowance, separately

There were 126 completed quota-reader invocations: 125 JSON decisions and one
help invocation. Of the decisions, 103 permitted a start, 20 rejected stale
telemetry, one rejected a session-identity mismatch, and one stopped at the floor.
The lead ran 73 checks, Luna 18, Astra 19, and the Sol API reviewer 16. The longest
gap across reader invocations was 354.311 seconds, about 5 min 54 s, versus
8 min 41 s previously. Worker checks remain a source of duplicate/stale handling.

The first fresh decision at 00:48:51.318Z observed 100% remaining. The final
reader decision at 03:41:35.311Z observed exactly 80%, with snapshot age 26.6s,
and returned `do_not_start`. No later implementation commit occurred; only the
checkpoint commit and final message followed. The next historical contract was
in flight before this stop and completed; dependent implementation did not start.
Allowance is account-wide, rounded telemetry. The 20 percentage-point change
is not attributable billing for this task and must not be converted into tokens.

## Quality and autonomy

The direct Sol lead continued through partial-existing destination, new caller
destination, full caller-source removal with a retained validator, and pre-Aspen
zero amount with positive existing caller pairs. Each completed profile gained
its bounded execution/frame/API evidence; zero historical API work remains next.
See the [scorecard](doc/codex_slice_scorecard.md) and
[checkpoint](doc/evm_research/n6_restart_checkpoint.md).

Two independent Sol findings show useful quality control:

- New-destination frame evidence reused the previous call's execution reads as
  the next call's setup reads. The observer was corrected, actual dual-pin
  outputs regenerated, checks rerun and the corrected freeze accepted.
- Full-source structured traces lacked an explicit deletion/recreation effect
  witness. A success-only prefix could satisfy generic trace JSON assertions.
  The corrected tests now record outcomes and assert deletion, membership
  removal and reverse recreation before accepting the evidence.

The rejected freezes and first failures were retained. These were evidence
findings, not claims of production defects. Source changes were revalidated;
passing tests were not used to dismiss missing assertions. The initial workspace
fast gate encountered an existing temporary database lock collision; the accepted
serial rerun used `RUST_TEST_THREADS=1`. Runtime changes retain focused ON bridge
builds with 12 jobs and all 15 bridge tests. Test-only derivatives retain relevant
Rust checks and workspace fast gates. No EVM gates were rerun by this audit.

## Next improvements

1. **Bound reviewer history by profile as well as family.** The new Astra thread
   avoided old metadata/escrow history, but continued across partial, absent
   destination, full removal and zero profiles. Per-turn median input rose from
   49K to about 238K before an explicit compaction at 03:38:44.512Z reduced
   subsequent medians to 42K and 53K. That late reduction helped; waiting for
   a large automatic compaction still incurred substantial earlier costs. Start fresh bounded contexts at
   these transitions when capacity permits. Supply settled contracts, exact delta,
   pins and evidence paths; do not import the full branch conversation. Preserve
   Luna capacity and record any thread-capacity blocker.
2. **Keep Sol review for settled derivatives.** It caught both defects above.
   Keep Astra for new authentication/rollback and uncertain authority. Do not
   reopen settled contracts for mechanical corpus or test additions without a
   specific semantic question. This is not permission to omit independent review.
3. **Reduce coordination responses.** This run recorded 61 sends, 39 follow-ups,
   five agent waits and five sleeps, versus 19 sends, 25 follow-ups and zero agent
   waits previously. Some sends deliver required corrections and some waits are
   useful. Use one complete bounded assignment, result/blocker messages, and one
   correction handoff. Avoid routine status checks and repeated quota narration.
4. **Keep actual oracle observations clean.** For repeat-call captures, clear
   per-call buffers and assert concatenated observations. For stateful prefixes,
   assert the required effects before testing downstream failure/disposal. These
   focused checks reduce expensive correction/review batches.
5. **Measure comparable derivatives next.** Compare estimation/trace follow-ups
   with equivalent corpus and risk, not all commits as interchangeable tasks.
   Keep deduplicated tokens, historical-rate estimates, correction batches and
   elapsed time separate from account allowance. Do not cut validation to hit a
   cost target.

Implementation changes are not applied by this audit. The next ready work is
zero-amount historical simulation over the accepted complete H1 profile, then
estimation/traces. The run used the historical 80% reserve policy. The next run
uses the saved relative budget in `next_executable_slice_prompt.md`.
Real-window input completeness, root parity and N4–N6 remain open.

## Sources and reproducibility

- Root rollout: `/home/fry/.codex/sessions/2026/10/03/rollout-2026-10-03T00-48-15-01a0ff3b-ab12-7902-b7a9-164209e8305f.jsonl`.
- Derived script and reconciled records: [audit.py](/home/fry/artifacts/token-audit-01a0ff3b/audit.py),
  [summary.json](/home/fry/artifacts/token-audit-01a0ff3b/summary.json).
- Accepted commit list: [accepted-local-commits.txt](/home/fry/artifacts/evm-redelegate-2026-10-03/accepted-local-commits.txt).
- Corrected evidence reviews: [new-destination frame review](/home/fry/artifacts/evm-redelegate-2026-10-03/new-destination-frames-corrected-review.md),
  [full-source trace review](/home/fry/artifacts/evm-redelegate-2026-10-03/full-source-trace-review.md).

This report is uncommitted. Original logs remain unchanged.
