# Codex session token audit and efficiency analysis

Report date: 2026-10-01. Session: `01a0f259-680f-7a90-a823-bce110580fcc`.

The session and its five descendant agents recorded **112,071,893 tokens**, including
context compaction. The two EVM work phases account for **109,986,507 tokens**.
The work produced useful implementation, validation, and evidence, but the workflow
had substantial avoidable coordination and context overhead. The largest improvement
opportunities are scoped contexts, fewer coordination turns, and selective Astra use.

## Scope and boundaries

- Source: local Codex JSONL logs under `/home/fry/.codex/sessions/`, with
  `/home/fry/.codex/archived_sessions/` included in discovery when present.
- Coverage: the root session and all locally discoverable descendant agents, through
  the last recorded root response on **2026-10-01 at 01:20:20 UTC**.
- Forty local rollout files were scanned during discovery. Five descendants were
  found, all direct children of the root; no deeper descendants were found.
- The original session and its agents were not resumed, contacted, or changed.
- Log inspection and Git correlation were read-only. This report is the subsequently
  requested repository write; it does not modify the audited logs or implementation.
- Usage of this external audit conversation is outside the audited session totals.
- Model attribution comes from logged `turn_context.model`, rather than requested
  model names or an agent's self-description.
- Token counts, estimated credit costs, and subscription allowance percentages are
  separate measurements throughout this report.

## Accounting method and reconciliation

Count each `token_usage_record.payload.usage` once, deduplicated by
`(thread_id, response_id)`. Follow `session_meta.parent_thread_id` recursively to
identify descendants. Do not discover related threads merely by matching dates,
repository paths, or model names.

Use cumulative `thread_token_usage`, `turn_token_usage`, and displayed
`event_msg.token_count` counters for reconciliation only. Do not add them to the
individual response usage. Likewise, do not count copies of usage records embedded
in compaction metadata a second time.

The audit found:

| Accounting item | Count |
| --- | ---: |
| Unique response usage records | 958 |
| Duplicate response usage records | 0 |
| Displayed counter events | 1,058 |
| Distinct displayed counter snapshots | 955 |
| Repeated displayed snapshots | 103 |
| Additional compaction responses | 3 |

All ordinary displayed cumulative increments reconcile with the corresponding
response usage. The three compaction requests are present in individual usage records
and thread totals but omitted from displayed cumulative counters.

| Reconciliation | Tokens |
| --- | ---: |
| Sum of final displayed counters across six threads | 111,378,587 |
| Root compaction | 218,686 |
| Luna audit compaction | 242,286 |
| Astra review compaction | 232,334 |
| Total additional compaction usage | 693,306 |
| **Reconciled total** | **112,071,893** |

Compaction totals comprise 681,695 input tokens, including 665,088 cached input
tokens, and 11,611 output tokens. Their recorded reasoning output is zero.
Compaction itself represents approximately **0.62%** of all recorded tokens.

The Sol tools assignment ended at 13:58:47 UTC with a logged `server_overloaded`
model-capacity error. Its completed response usage remains included. The error is
not evidence of an account usage-limit failure or unrecorded token consumption.

## Token totals by model

| Model | Input | Cached input | Output | Reasoning output | Total |
| --- | ---: | ---: | ---: | ---: | ---: |
| gpt-6-astra | 84,720,340 | 83,288,192 | 144,818 | 33,828 | 84,865,158 |
| gpt-6.1-sol | 17,887,266 | 17,566,592 | 85,092 | 11,818 | 17,972,358 |
| gpt-6-luna | 9,204,088 | 8,885,760 | 30,289 | 10,403 | 9,234,377 |
| **Total** | **111,811,694** | **109,740,544** | **260,199** | **56,049** | **112,071,893** |

Cached input is already included in input. Reasoning output is already included in
output. Neither subset should be added again. Cache-write input tokens were zero.

**98.15% of input was cached**. Uncached input totaled **2,071,150 tokens**.
Total tokens equal input plus output; they are not a bill or an allowance conversion.

## Token totals by agent

| Agent | Model | Input | Cached input | Uncached input | Output | Reasoning output | Total |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| root | gpt-6-astra | 35,374,136 | 34,657,536 | 716,600 | 82,937 | 23,419 | 35,457,073 |
| luna_n4_audit | gpt-6-luna | 9,204,088 | 8,885,760 | 318,328 | 30,289 | 10,403 | 9,234,377 |
| sol_n4_tools | gpt-6.1-sol | 9,730,699 | 9,551,104 | 179,595 | 44,688 | 7,007 | 9,775,387 |
| astra_n4_review | gpt-6-astra | 34,822,614 | 34,335,616 | 486,998 | 34,116 | 5,276 | 34,856,730 |
| astra_reward_contract | gpt-6-astra | 14,523,590 | 14,295,040 | 228,550 | 27,765 | 5,133 | 14,551,355 |
| sol_reward_rate | gpt-6.1-sol | 8,156,567 | 8,015,488 | 141,079 | 40,404 | 4,811 | 8,196,971 |

The root and independent reviewer together account for **70,313,803 tokens**, or
approximately **62.7%** of the total. The reviewer consumed about **1.94 times**
the tokens of the two Sol implementation agents combined.

## Work phases

Phases follow `token_usage_record.root_turn_id`. This attaches descendant work to
the root request that initiated it, rather than attributing it only by timestamp.

| Phase | Root turn ID | UTC date/time | Tokens |
| --- | --- | --- | ---: |
| Initial N4: working copy, reward diagnostic, sender scout | `01a0f259-a276-7c22-b257-c45d0605761e` | Sep 30, 12:45–13:01 | 13,348,269 |
| Remaining-work and allowance planning | `01a0f26c-e599-7361-a6aa-461868b84f5f` | Sep 30, 13:06–13:08 | 659,032 |
| Continued N4: proofs, reward inputs/planner, cold Go witness | `01a0f271-c055-74a1-bc53-c6f79a0eebd1` | Sep 30, 13:12–14:24 | 96,638,238 |
| Remaining-work status | `01a0f2d0-234d-7660-b976-de183ed31339` | Sep 30, 14:55 | 145,070 |
| Usage-analysis discussion | `01a0f508-279e-7863-adb5-727d262930ee` | Oct 1, 01:15–01:16 | 773,177 |
| External-audit discussion | `01a0f50b-a811-7601-a25d-2cdee255b845` | Oct 1, 01:19–01:20 | 508,107 |

| Agent | Initial N4 | Continued N4 | Other discussion | Total |
| --- | ---: | ---: | ---: | ---: |
| root | 7,411,419 | 25,960,268 | 2,085,386 | 35,457,073 |
| luna_n4_audit | 884,094 | 8,350,283 | 0 | 9,234,377 |
| sol_n4_tools | 1,888,210 | 7,887,177 | 0 | 9,775,387 |
| astra_n4_review | 3,164,546 | 31,692,184 | 0 | 34,856,730 |
| astra_reward_contract | 0 | 14,551,355 | 0 | 14,551,355 |
| sol_reward_rate | 0 | 8,196,971 | 0 | 8,196,971 |
| **Total** | **13,348,269** | **96,638,238** | **2,085,386** | **112,071,893** |

The two EVM implementation phases used 109,730,249 input tokens, including
107,855,360 cached input tokens, and 256,258 output tokens, including 54,812
reasoning output tokens. The two subsequent usage-analysis discussions used
**1,281,284 tokens** and are excluded from the EVM work subtotal.

## Subscription allowance observations

The root log reports account-level `codex` allowance snapshots in a 10,080-minute
window, with the same reset timestamp: **2026-10-03 at 17:30:41 UTC**.

| Observation | Used | Remaining |
| --- | ---: | ---: |
| First snapshot, Sep 30 12:46 | 32% | 68% |
| Initial N4 completion | 37% | 63% |
| Allowance-planning completion | 37% | 63% |
| Continued N4 completion | 66% | 34% |
| Remaining-work status | 66% | 34% |
| Final snapshot, Oct 1 01:20 | 66% | 34% |

The observed change is **34 percentage points**. Account-level observations do not
establish the session's exclusive allowance consumption, and percentages must not
be summed across agents or converted into token prices.

The continued-work completion message claimed **65% used / 35% remaining**.
Its contemporaneous logged counter recorded **66% / 34%**. The counter is the audit
value. The logged credit balance stayed at `2118.4016850000`; these logs do not
establish that the illustrative credits calculated below were actually charged.

## Git correlation and delivered work

The audited repository baseline was `5aa3febfee14a3ad59c2b24c15d94a49013fd170`.
The completed work ended at `a028b7a2a` on `feat/rust/evm-state-db`.

Git confirms six commits covering snapshot qualification, sender proofs, independent
reward inputs, planner comparison, native-effect gates, and the cold Go witness.
The net baseline-to-final diff contains:

| Category | Files | Added lines | Deleted lines |
| --- | ---: | ---: | ---: |
| Code/configuration | 21 | 4,042 | 144 |
| Documentation/evidence | 26 | 3,086 | 15 |

These are substantial deliverables. Line counts are context for the work, not a
quality or productivity score. Much of the documentation consists of reproducible
evidence and qualification limits required by the rewrite plan.

### Calendar intervals ending at commits

All timestamps below are UTC on 2026-09-30. The first interval begins at 12:45:00;
each subsequent interval begins at the preceding commit timestamp. Usage records
are allocated by their timestamp, across all six threads.

| Interval ending at commit | Deliverable | Tokens |
| --- | --- | ---: |
| 13:01:30 — `bf57aec01` | Qualify restored snapshot and complete sender scout | 12,984,546 |
| 13:20:35 — `3605bf909` | Authenticate bounded sender storage paths | 16,197,078 |
| 13:44:39 — `e63be522d` | Derive bounded reward inputs independently | 36,689,982 |
| 14:01:32 — `36d2efa9f` | Compare rewards with independent input artifacts | 20,226,287 |
| 14:11:52 — `399b3d8ae` | Qualify bounded parent native-effect gates | 13,914,769 |
| 14:24:02 — `a028b7a2a` | Witness cold native effects against pinned Go | 10,138,447 |

These are calendar intervals, **not exclusive costs per commit**. Work overlapped,
and the intervals include coordination, allowance planning, validation, and review.
They exclude subsequent closeout and discussion. Author timestamps have second
precision, so a response recorded later within the commit's second can fall into
the next interval. Use root-turn phases for exact work-request accounting.

### Git command profile

The following counts are textual command occurrences extracted from literal shell
commands in the logs. They are not a count of successful operations; a compound
command can contain several occurrences.

| Git subcommand | Occurrences |
| --- | ---: |
| diff | 78 |
| show | 56 |
| status | 45 |
| log | 12 |
| rev-parse | 11 |
| add | 6 |
| commit | 6 |
| push | 6 |
| branch | 2 |
| submodule, cat-file, merge-base, config, ls-files, rev-list | 1 each |

Responses containing Git commands account for **17,986,136 tokens across 147
responses**. Many also read source, edit documentation, or perform other work.
This association does not establish that Git was the cause of those tokens.

Git commands do not themselves consume model tokens. The expense comes from model
turns and returned output. Most status and diff checks were sensible for a shared
workspace and parity-sensitive rewrite. Batch compatible checks and return bounded
diffs rather than eliminating validation or repository-state checks.

Read-only commands used to correlate the history included:

```sh
git log --since='2026-09-30T12:40:00Z' --until='2026-09-30T15:00:00Z' \
  --format='%h %aI %s' --stat
git diff --numstat 5aa3febfee14a3ad59c2b24c15d94a49013fd170 a028b7a2a
```

## Response activity and context overhead

The activity analysis associates each usage record with the response items issued
since the preceding usage record in that thread. The observed responses issued at
most one tool call each. Responses without tool calls form the final category.

This describes **what the response issued**, not the precise intellectual purpose
of every token. For example, a message can contain a valuable semantic finding,
and selecting a shell command can require substantial reasoning. Tool results
feed later responses and can be carried repeatedly in their input contexts.

| Response activity | Tokens | Share |
| --- | ---: | ---: |
| Issuing shell/tool execution (`exec`) | 60,836,693 | 54.3% |
| Sending agent messages | 32,405,425 | 28.9% |
| Waiting for agents | 10,741,866 | 9.6% |
| Assigning follow-up tasks | 2,582,002 | 2.3% |
| Listing agents | 770,610 | 0.7% |
| Spawning agents | 476,197 | 0.4% |
| Other responses, including compaction | 4,259,100 | 3.8% |

Messages and waits together account for **43,147,291 tokens**, or **38.5%** of
the total. They are a strong signal to investigate coordination overhead, not a
claim that all of those tokens were wasted.

### Context size by agent

| Agent | Responses | Average input per response | Median input | Maximum input |
| --- | ---: | ---: | ---: | ---: |
| root | 314 | 112,656 | 104,758 | 214,002 |
| luna_n4_audit | 79 | 116,507 | 110,006 | 239,927 |
| sol_n4_tools | 88 | 110,576 | 103,435 | 177,570 |
| astra_n4_review | 261 | 133,420 | 134,556 | 227,766 |
| astra_reward_contract | 117 | 124,133 | 116,383 | 201,616 |
| sol_reward_rate | 99 | 82,390 | 76,913 | 142,986 |

Even a short message or wait therefore carried a large accumulated context.
All agents loaded substantial source, plans, and evidence, and contexts persisted
across follow-up assignments. The repeated processing of large contexts matters
more than the direct cost of the three compaction requests.

### Coordination calls

| Agent | exec | send_message | wait_agent | list_agents | spawn_agent | followup_task |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| root | 182 | 74 | 18 | 7 | 5 | 21 |
| luna_n4_audit | 60 | 12 | 0 | 0 | 0 | 0 |
| sol_n4_tools | 62 | 23 | 0 | 0 | 0 | 0 |
| astra_n4_review | 106 | 86 | 63 | 1 | 0 | 0 |
| astra_reward_contract | 67 | 42 | 1 | 0 | 0 | 0 |
| sol_reward_rate | 60 | 31 | 3 | 1 | 0 | 0 |

The independent reviewer is the clearest optimization opportunity:

- It made **63 waits**, each requesting a one-minute timeout, and **86 message calls**.
- **30 waits timed out**. Responses issuing those waits account for **4,299,376 tokens**.
- Its other 33 waits account for 4,339,860 tokens.
- Its message responses account for **11,692,178 tokens**.
- Messages and waits together account for **20,331,414 tokens**, approximately
  **58% of that reviewer's usage**.

The reviewer identified issues and approved corrected artifacts, so independent
review had value. Keeping the reviewer active throughout implementation, with
frequent exchanges and waits, is the behavior to change.

### Additional polling and validation observations

| Overlapping activity tag | Responses | Associated tokens |
| --- | ---: | ---: |
| Process polling through `write_stdin` | 51 | 5,694,878 |
| Allowance polling through `check_usage.py` | 18 | 1,891,661 |

These tags can overlap with Git and other activities. Do not add them to the
exclusive activity table or to each other as if they were disjoint costs.

Targeted validation and repeated fast-gate executions were present. Exact repeated
commands included formatting, Clippy, whitespace checks, pushes, and allowance
polls. Repetition alone is not evidence of waste: new changes and review corrections
occurred between checks. The audit does **not** establish that required tests were
unnecessary. Preserve parity, smoke, targeted, and pre-commit gates.

## Illustrative credit costs

The following estimate uses the official **Standard credit rates observed on
2026-10-01**, rather than API dollar prices or a subscription allowance conversion.

| Model | Credits per 1M uncached input | Credits per 1M cached input | Credits per 1M output |
| --- | ---: | ---: | ---: |
| gpt-6-astra | 250 | 25 | 1,250 |
| gpt-6.1-sol | 50 | 2.5 | 250 |
| gpt-6-luna | 2.5 | 0.25 | 12.5 |

Source: [official pricing and usage guidance](https://learn.chatgpt.com/docs/pricing).
That page explicitly distinguishes credit prices from included subscription usage.
Speed settings can also change applicable rates.

For each model, the calculation is:

```text
estimated Standard credits =
  ((input_tokens - cached_input_tokens) * uncached_input_rate
   + cached_input_tokens * cached_input_rate
   + output_tokens * output_rate) / 1,000,000
```

Reported compaction usage is included once. Reasoning output is already within
output. This is a **rate-card estimate, not an actual bill**. Historical speed
settings and applicable billing treatment were not established.

| Agent/work | Estimated Standard credits |
| --- | ---: |
| Astra root | 1,149.25965 |
| Astra independent reviewer | 1,022.78490 |
| Astra contract authority | 449.21975 |
| Sol tools implementation | 44.02951 |
| Sol reward-rate implementation | 37.19367 |
| Luna helper | 3.3958725 |
| **Total** | **2,705.8833525** |

| Model | Estimated Standard credits |
| --- | ---: |
| Astra | 2,621.26430 |
| Sol | 81.22318 |
| Luna | 3.3958725 |

Astra contributes approximately **97%** of the estimate. Cached Astra input alone
contributes **2,082.2048 credits**, about **77%** of the estimated total. A high
cache-hit rate is useful, but repeatedly processing cached context remains material.

### Model-routing sensitivity calculation

Repricing the root and reviewer's identical token counts at Sol rates produces:

| Scenario | Estimated Standard credits |
| --- | ---: |
| Recorded model assignments | 2,705.8833525 |
| Root at Sol rates | 143.20809 for the root |
| Reviewer at Sol rates | 118.71794 for the reviewer |
| Total with both substitutions and other assignments unchanged | 795.7648325 |

That is approximately **71% lower**. It does not prove Sol would achieve identical
quality, consume identical tokens, or avoid Astra escalation. It identifies model
routing as a high-value hypothesis for a controlled workflow experiment.

## Efficiency assessment

The session was productive, but its token profile was not lean. Useful evidence
includes six substantive commits, focused validation, independent review that
resulted in corrections, and explicit limits on what the evidence qualified.

The avoidable overhead is principally:

1. Persistent large contexts across many small responses.
2. Frequent agent-to-agent messages and reviewer waits during unfinished work.
3. Astra performing routine coordination and parts of incremental review.
4. Process and allowance polling that could be consolidated.

The logs do not establish an exact percentage of wasted tokens. Coordination can
resolve real dependencies, and review can prevent expensive correctness failures.
Similarly, six commits or a large line count do not prove optimal throughput.

Reducing required tests or merely shortening final answers would target smaller
parts of this profile and could damage the rewrite's correctness guarantees.
The better experiment changes orchestration, context scope, and model assignment
while retaining the existing acceptance gates.

## Strategies to improve throughput and cost per task

| Change | Concrete practice | Expected benefit |
| --- | --- | --- |
| Sol-led routine execution | Use Sol for settled implementation, integration, commands, and ordinary review; use Astra for difficult semantics and authority decisions | Largest potential credit reduction |
| Milestone-based review | Obtain early contract review when necessary; review implementation after source and evidence are frozen | Fewer waits and repeated partial reviews |
| Complete bounded assignments | Workers deliver a concise handoff and finish; reactivate related work for a specific new question | Less idle coordination |
| Scoped contexts | Reuse agents for closely related work; start fresh for materially different slices with a concise handoff | Smaller repeated input contexts |
| Consolidated messages | Send one handoff with paths, decisions, tests, hashes, and unresolved issues; send intermediate messages for blockers or contract changes | Fewer message-driven turns |
| Bounded tool output | Read relevant symbols/ranges; summarize large JSON with scripts; return failures and short test summaries | Less context growth |
| Batched deterministic checks | Run compatible independent reads/checks together; poll when the result can affect the next action | Fewer round trips |

### Proposed task lifecycle

1. Define a bounded slice with inputs, owned paths, acceptance criteria, required
   validation, and explicit architectural boundaries.
2. Launch and confirm the bounded Luna helper first when starting an EVM team,
   as required by repository policy. Use its output for a specific inventory,
   hash/link check, or mechanical task rather than ongoing broad commentary.
3. Resolve an ambiguous contract with Astra before workers diverge. Settled
   contracts should not require repeated approval messages.
4. Assign at most two Sol implementation workers with disjoint ownership when
   the scopes can proceed independently. Keep manifest/lockfile and database
   authority coordinated.
5. Workers return a compact handoff: changed paths, implemented behavior, tests,
   evidence identity, unresolved issues, and routing failures.
6. Freeze the source/evidence for independent review. Use Sol for routine review
   and Astra for high-risk semantic review, under the repository escalation criteria.
7. Correct findings and rerun affected validation. Repeat broader validation when
   new changes or unresolved concerns justify it, while satisfying required gates.
8. Integrate, record the checkpoint, and close out the slice. Finish completed
   assignments rather than keeping them in message/wait loops. Do not assume a
   completed thread releases an agent slot; use supported lifecycle operations.

This is a proposed future workflow, not a change to current configuration. Follow
the existing [implementation plan](doc/evm_research/08_implementation_plan.md),
[routing validation record](doc/evm_research/n6_model_setup_validation.md), and
[repository guidelines](AGENTS.md) until a prospective policy update is adopted.
Preserve historical model assignments and routing evidence.

Keep Astra escalation for unresolved historical behavior, cryptographic/trie
invariants, adoption/recovery/publication authority, and difficult failures.
Maintain independent review, Rust ownership, snapshot preservation, existing
test expectations, and the required parity/smoke gates. Optimization does not
authorize production routing or bypass architectural requirements.

### Context and documentation changes

- Keep always-loaded instructions concise while preserving architectural hard rules.
- Move detailed workflows to task-specific references or appropriately scoped
  instructions, so agents load what the task requires.
- Keep the restart checkpoint focused on current state and next actions. Preserve
  historical evidence in separately linked records.
- Prefer bounded `rg` searches and source ranges to repeated full-file reads.
- Select the necessary JSON fields instead of returning entire evidence reports.
- Summarize test output deterministically, retaining full local logs for inspection.
- Disable irrelevant connectors/integrations in coding sessions when supported,
  rather than carrying unnecessary tool descriptions.
- Reuse related agent threads; use fresh context for genuinely different work,
  rather than blindly restarting agents after every small change.

Official guidance supports smaller relevant source inputs, smaller `AGENTS.md`
files, fewer unnecessary integrations, and progressively loaded instructions:
[usage guidance](https://learn.chatgpt.com/docs/pricing) and
[instruction guidance](https://developers.openai.com/blog/rethinking-skills-and-prompts-for-gpt-6-astra).
These recommendations do not override this repository's validation requirements.

## Measurement plan

For the next three comparable slices, record:

| Metric | Purpose |
| --- | --- |
| Recorded tokens per accepted slice, including compaction | Measure context and response overhead |
| Rate-weighted credits per accepted slice | Compare model mixes without treating all tokens as equal |
| Time to passing validation and independent approval | Measure useful throughput |
| Model responses, messages, waits, and repeated reads | Identify orchestration overhead |
| Review corrections, rework, and later regressions | Detect quality loss from optimization |
| Account allowance movement, recorded separately | Track the practical subscription constraint |

Use **50% fewer recorded tokens and 60–75% lower estimated credits as experimental
targets, not promises**. Compare slices of similar risk and scope. Record actual
models, reasoning settings, speed modes, escalation, and capacity failures.
Do not declare success solely from fewer tokens if correctness, rework, or elapsed
time deteriorates. Do not sum separate projected savings without checking overlap.

The first three changes to trial are **Sol-led routine work, milestone-based
independent review, and compact handoffs between slices**. They address the largest
observed costs while preserving the checks that made the deliverables useful.

## Source logs

These paths are environment-specific; the logs are not copied into this repository.
All descendant agent paths below are under `/root/`.

| Agent | Thread ID | Rollout file under `/home/fry/.codex/sessions/2026/09/30/` |
| --- | --- | --- |
| root | `01a0f259-680f-7a90-a823-bce110580fcc` | `rollout-2026-09-30T12-45-40-01a0f259-680f-7a90-a823-bce110580fcc.jsonl` |
| luna_n4_audit | `01a0f259-fabf-7441-a34e-3e5c75b978a7` | `rollout-2026-09-30T12-46-18-01a0f259-fabf-7441-a34e-3e5c75b978a7.jsonl` |
| sol_n4_tools | `01a0f25a-56bc-7810-9bfa-62cb834404de` | `rollout-2026-09-30T12-46-41-01a0f25a-56bc-7810-9bfa-62cb834404de.jsonl` |
| astra_n4_review | `01a0f25b-32c3-7643-afd8-0a4067e369e4` | `rollout-2026-09-30T12-47-38-01a0f25b-32c3-7643-afd8-0a4067e369e4.jsonl` |
| astra_reward_contract | `01a0f274-2c9f-74b3-9c82-f85c6d19d763` | `rollout-2026-09-30T13-14-55-01a0f274-2c9f-74b3-9c82-f85c6d19d763.jsonl` |
| sol_reward_rate | `01a0f27a-c0a5-7112-b601-60f82329ed6b` | `rollout-2026-09-30T13-22-06-01a0f27a-c0a5-7112-b601-60f82329ed6b.jsonl` |

To reproduce the accounting, discover ancestry from session metadata, deduplicate
response records, sum their `usage`, attribute models from turn contexts and phases
from root turn IDs, reconcile compaction separately from displayed counters, and
correlate response timestamps with bounded Git history. Read the JSONL files
directly; opening or resuming the original session is unnecessary.

## Instruction cleanup and concise communication

The user requested a review of instruction growth and asked Luna to do the main
analysis. Requested route: `gpt-6-luna`, medium, through the configurable default
role, with no inherited conversation. The agent started and completed its read-only
assessment. No routing failure occurred. Its task was limited to relevant instruction
and checkpoint files. It did not change repository guidance or historical evidence.

The following file sizes were measured during this follow-up:

| File | Lines | Words | Recommended role and target |
| --- | ---: | ---: | --- |
| `AGENTS.md` | 189 | 2,164 | Required rules and links; target 1,400–1,700 words |
| `PLAN.md` | 844 | 8,973 | Architecture and roadmap reference; read relevant sections |
| `doc/evm_research/n6_restart_checkpoint.md` | 216 | 1,834 | Current state and next action; target 500–700 words |
| `doc/evm_research/n6_agent_handoffs.md` | 527 | 4,647 | Dated history; read selected entries |
| `doc/evm_research/08_implementation_plan.md` | 233 | 2,683 | EVM contracts, assignments, and acceptance gates |
| `doc/evm_research/n6_model_setup_validation.md` | 121 | 908 | Historical model observations and limits |
| `doc/evm_research/n4_restored_snapshot_next_slice.md` | 87 | 748 | Closed slice and its original contract |

### Cleanup priorities

1. Shorten `AGENTS.md` procedures and repeated explanations. Keep every required
   architecture rule, test requirement, permission boundary, and capacity rule.
   Put detailed commands and procedures in linked references. Make required reading
   explicit so that moving a rule does not make it optional.
2. Reduce the restart checkpoint to current state, next action, prerequisites,
   blockers, required checks, and evidence links. Move older detail to dated records
   only after confirming that its evidence and limits are preserved.
3. Keep `PLAN.md` and the EVM implementation plan as reference documents. Their size
   alone does not justify removing contracts or acceptance gates. Stop requiring a
   full read for every narrow task; identify the relevant section.
4. Keep the handoff ledger and model validation record as history. Link to them
   instead of copying their full content into new checkpoints. Preserve requested
   models, observed runtime, results, and failures.
5. Treat the restored N4 next-slice file as a closed historical handoff. Keep current
   next steps in the restart checkpoint.

The repository `AGENTS.md` examined here has no managed-block markers. The user
supplied managed storage, RTK, and communication rules through active instructions.
Those rules still apply. Changes to managed content belong in its management source;
do not copy or edit it as if the repository owns it. Do not read, write, search, or
ingest the excluded instruction file named in the active instructions.

### Proposed communication contract

Use ASD-STE100 Simplified Technical English, adapted to software terms. Keep technical
precision. Use short sentences, direct verbs, and exact names.

- Progress: state the result or blocker and the next action. Aim for 1–3 sentences.
- Agent messages: send contract changes, blockers, or a complete handoff. Avoid
  repeated status exchanges.
- Handoffs: give the result, changed paths, checks, evidence, remaining work, and
  requested and observed model information.
- Final answers: give the outcome, validation, and material limits. Link to detailed
  reports instead of repeating them.
- Keep required user updates. Keep enough detail to assess correctness and scope.

A compact task and handoff template is:

```text
Objective: one measurable result.
Base/reference: commit, configuration, or dataset.
Owner and paths: one owner and exact scope.
Dependencies and invariants: required inputs and rules.
Required checks: commands or evidence.
Scope limits: explicit exclusions required by the task.
Result: commit/artifact, checks and outcomes, unresolved issues.
Routing: requested model/reasoning, confirmed startup, observed result or failure.
```

Shorter text reduces new context and repeated input in later responses. However,
output was only **0.23%** of the audited token total. Shorter answers alone cannot
establish a large reduction in total usage. Combine concise text with smaller
working contexts, fewer messages, and review at defined milestones. Measure the
result with the existing per-slice metrics.

These are proposals. The current instruction files, model policy, and historical
records were not changed by this follow-up.

### Cleanup applied

The user then authorized implementation. The same Luna agent performed the main
documentation edits; the lead reviewed the rules and made final corrections.
The existing model routing policy and historical model records remain unchanged.

| File | Words before | Words after | Change |
| --- | ---: | ---: | --- |
| `AGENTS.md` | 2,164 | 1,620 | About 25% shorter; concise communication rules added |
| `doc/evm_research/n6_restart_checkpoint.md` | 1,834 | 648 | About 65% shorter; current state, limits, next action, and scoped reading |
| `doc/rewrite_agent_workflow.md` | — | 556 | Required reference for upstream C++ overlay work |
| `doc/evm_research/n6_restart_checkpoint_history_2026_10_01.md` | — | 1,834 | Exact archive of the original checkpoint |
| `doc/evm_research/n4_restored_snapshot_next_slice.md` | 748 | 773 | Link to the live checkpoint added; original intake retained |

The root guidance and live checkpoint together are about **43% shorter by word
count**. This is not a measured token or credit reduction. Required overlay details
remain in the linked workflow. Load them for relevant work. The archive preserves
history and does not need to be loaded for every task.

Review retained Rust ownership, the PBFT transport/execution boundary, task-owner
approval for exceptions, explicit missing-work behavior, the approved protected-state
pattern, shim TODOs, upstream diff checks, test integrity, parity and smoke checks,
storage validation, and agent capacity rules. Scoped reading replaces the checkpoint's
full startup document stack. The original 65% telemetry sample remains historical;
the later 66% sample is linked to this audit.

Validation: whitespace checks, local-link checks, and exact byte comparison of the
checkpoint archive against the original Git version. No code or runtime tests were
needed for these documentation changes. No commit or push was made.
