# Fixture-hardening pilot: token and cost audit

Session `01a0f8a5-9d5f-7d00-91f3-cf46cd0ccde0`, including both descendant
agents. This audit read local Codex logs without resuming the session or
contacting its agents. It did not rerun validation or change implementation.

## Finding

The workflow changes worked operationally: Sol implemented directly, Luna did
one bounded map, and a separate Sol reviewed frozen evidence. The slice closed
both requested regression gaps at local commit `5bfdf494c`, with required checks
passing and no blocking review findings.

Usage was **2,983,551 tokens / 18.39287 estimated Standard credits**. Against the
[previous lifecycle pilot](token_usage_audit_01a0f7e8.md), tokens fell **67.5%**,
estimated credits fell **74.6%**, and active time fell from 26m07.5s to
**13m20.1s**. This was a narrower task over an existing fixture. Those changes
cannot all be attributed to routing; this is not a controlled model benchmark.

The direct evidence for improved coordination is strong: root waits fell from
24 to two, and messaging/wait response usage fell from 28.43% to **7.97%**.
The separate implementation-lead thread disappeared. Independent review and
validation remained intact.

## Deduplicated usage

All actual models ran at medium reasoning. Cached input is included in input;
reasoning is included in output. Neither subset is added again.

| Agent / work | Actual model | Responses | Input | Cached input | Output | Reasoning | Total tokens | Estimated credits |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Root: implementation, validation, integration | GPT-6.1 Sol | 38 | 2,226,338 | 2,146,048 | 15,472 | 2,916 | 2,241,810 | 13.24762 |
| Bounded input map | GPT-6 Luna | 8 | 222,512 | 182,528 | 1,970 | 424 | 224,482 | 0.17022 |
| Independent frozen review | GPT-6.1 Sol | 12 | 513,201 | 456,832 | 4,058 | 437 | 517,259 | 4.97503 |
| **Total** | | **58** | **2,962,051** | **2,785,408** | **21,500** | **3,777** | **2,983,551** | **18.39287** |

By model: Sol **2,759,069 tokens / 18.22265 credits**; Luna **224,482 /
0.17022**. No Astra agent ran. The reviewer accounts for 27.05% of estimated
credits, versus 48.70% for the prior Astra reviewer. Initial Sol review took
3m24.2s, versus about 2m24.7s for the prior Astra review; lower cost did not mean
a faster review, and the review scopes differed.

Credit rates are the October 1 Standard rates retained in the
[original audit](token_usage_audit.md): per million uncached/cached/output
tokens, Sol 50/2.5/250 and Luna 2.5/0.25/12.5. Formula:
`((input - cached) * uncached_rate + cached * cached_rate + output * output_rate) / 1,000,000`.
These are estimates, not actual charges or allowance conversions.

## Work phases

| Phase, UTC | Tokens | Estimated credits |
| --- | ---: | ---: |
| Startup, 18:06:41 to first source edit at 18:08:59 | 499,670 | 3.49666 |
| Implementation/validation, until reviewer spawn at 18:14:56 | 1,196,741 | 6.54713 |
| Review and coordinator work, until final reviewer completion at 18:18:49 | 966,460 | 6.90116 |
| Closeout, until 18:20:01 | 320,680 | 1.44792 |

These are timestamp allocations across all threads; activities overlap. The
review phase includes coordinator work, so it costs more than the reviewer alone.
Source/evidence froze at 18:14:34.463879, before review.

The initial review used ten responses, 408,814 tokens and 4.61330 credits.
A documentation correction used two responses, 108,445 tokens and 0.36173
credits over 16.5s. The reviewer had initially named the requested model as
`gpt-6-sol`; the explicit request and runtime both show `gpt-6.1-sol`. This was
a reporting error, not a model substitution. No frozen implementation or result
changed after review. Creation through final completion took 13m23.3s.

## Coordination and Git

The tree made 45 shell-orchestration calls: root 30, Luna six, reviewer nine.
There were two waits, two child messages, one follow-up, two spawns and two agent
listings. The root sent no separate `send_message` calls.

Messages and waits correspond to **237,852 tokens / 7.97%**. Including the
documentation follow-up gives **315,429 / 10.57%**. These figures attribute a
model response to its emitted tools; they do not measure argument text alone.
Required process waits during validation are distinct from repeated agent-status
polling. No unnecessary passing-check rerun was recorded.

Average input was **51,070 tokens per response**, versus 58,963 previously.
Responses fell from 155 to 58. Fewer responses were the larger improvement;
shorter context also helped. Cached input was 94.0% of input. Brief user-facing
text alone would not explain this reduction, because input includes instructions,
conversation, code and tool output.

The audit found **12 distinct Git-related completed command events**: root 11,
reviewer one, versus 29 previously. These are command invocations, not all
embedded subcommands. Startup checked status and the baseline. Mid-slice diff
checks occurred at 18:12:34. Closeout verified frozen hashes, staged only this
slice, checked whitespace, and committed at 18:19:43. The reviewer inspected the
baseline diff once. No Git operation suggests a major waste driver. Selective
staging preserved unrelated preparation.

## Delivered work and validation

The [hardening report](doc/evm_research/n4_synthetic_fixture_hardening.md) and
[independent review](doc/evm_research/n4_synthetic_fixture_hardening_review.md)
record both completed objectives:

- The Go runner and Rust parity test require an empty observed raw-write list;
  absent/malformed evidence and nonzero effect counters fail.
- Both engines check a complete fixed input contract before setup or lifecycle
  work. Each rejects **306** missing, unknown, type, value and array drift cases.

Fresh positive execution retained zero effects. Historical outputs and source
pins stayed unchanged. This remains a synthetic cold nonboundary result; it
does not qualify a historical producer, real-window replay, publication or adoption.

Recorded final passes: Python harness test 1/1; Rust nonboundary 4/4; rewards
17/17; consensus bridge 15/15; storage bridge 4/4; both bridge builds with 12
jobs; and `make rewrite-validate-fast`. First-run logs, commands, exit status,
failed attempts and the 36-entry freeze record were retained. Review verified
the evidence without rerunning passing tests.

Two Go failures required correction: a uint64/uint32 modulo mismatch, then a
mechanical code edit that also corrupted embedded JSON literals. Both retries
were justified after corrections. One duplicate report-table row was removed
after review. No oracle or expected-result relaxation occurred.

The commit changes 11 files with 4,797 insertions and nine deletions. Much of
that size is retained negative-control evidence: the new Go JSON is 3,760 lines.
Line count is not a measure of implementation complexity or efficiency.

For the whole lifecycle-plus-hardening delivery, the two sessions total
**12,161,999 tokens / 90.68307 estimated credits / 39m27.6s active time**.
The latest slice alone should not be presented as the total cost of constructing
and hardening the fixture. The earlier blocked pilot and these audits are separate.

## Keep and improve

Keep the direct Sol lead, bounded Luna map, independent Sol review and first-run
log capture for comparable settled work. This run preserved the gates while
reducing coordination. Retain Astra escalation for unresolved authority or
difficult semantics; this one result does not prove interchangeable reviewers.

Next improve mechanical edit safety: change a named code span, then parse any
embedded JSON before a full Go execution. Avoid broad replacement across code
and data literals. This targets the two observed correction cycles.

Keep full drift-control evidence as an artifact and review it with structured
counts, assertions and hashes. Do not repeatedly print the full large JSON.
Keep concise milestone updates; do not chase a smaller response count by skipping
required validation or evidence.

The startup telemetry path needs repair. The lead searched for a callable quota
tool and recorded `unknown`. Local logs nevertheless recorded **70% used / 30%
remaining** at both 18:06:48.007 and 18:20:01.772, with the same reported balance
of 2,118.401685. The project rule starts no new slice at 30% or less remaining.
The available external observation was at that threshold; the agent's startup
check did not resolve or enforce it. This does not establish knowing misconduct.
Use a targeted current-session local-log read when the tool API lacks telemetry,
and check current allowance before another slice. Historical percentages do not
establish today's live balance. Unchanged rounded percentages and balance do
not prove that this task was free.

## Method and reproducibility

The audit discovered descendants recursively from recorded agent activity and
read only their local rollout logs. It found **58 unique `(thread_id,
response_id)` records**, no duplicate usage records, **61 cumulative snapshots
with three repeats**, and no compaction. It reconciled input, cached input,
output, reasoning and total sums against each thread's final cumulative counters.
Snapshots and subset counters were not added again. Models came from actual
`turn_context` records. This audit's own usage is excluded.

Logs are under `/home/fry/.codex/sessions/2026/10/01/`:

- Root: `01a0f8a5-9d5f-7d00-91f3-cf46cd0ccde0`.
- Luna input map: `01a0f8a6-1810-70a3-ad37-171f7f0c0a2b`.
- Sol review: `01a0f8ad-38ba-7190-9dc4-4d6dea64a16a`.

Persistent [audit script](/home/fry/artifacts/token-audit-01a0f8a5/summarize.py)
and [derived JSON](/home/fry/artifacts/token-audit-01a0f8a5/summary.json) retain
the detailed counters and command correlation. This report is uncommitted.
