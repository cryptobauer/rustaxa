# Synthetic lifecycle pilot: token and cost audit

Session: `01a0f7e8-d899-7453-8f1e-4b3fad23a54d`, including all three descendant
threads. This audit read local Codex logs. It did not resume the session, contact
its agents, rerun its tests, or change implementation code.

## Finding

The slice completed a useful synthetic execution gate at **9,178,448 tokens**
and **72.29021 estimated Standard credits**. Compared with the
[prior pilot](doc/evm_research/n6_token_usage_pilot_2026_10_01_audit.md), tokens
rose **71.9%**, while estimated credits fell **9.5%**. The prior pilot stopped
at four rejection tests. This pilot executed the real Rust lifecycle, built an
independent Go fixture, compared terminal effects, and ran required validation.
These are different task scopes; the figures do not establish a controlled
cost-per-equivalent-task improvement.

Model routing reduced estimated cost. Coordination still has room for improvement:
the Sol root delegated the main implementation to another Sol agent and made
24 wait calls. Messaging and waits consumed 28.4% of recorded tokens, up from
20.5% in the prior pilot. These figures include useful communication and recovery;
they are not a count of avoidable tokens.

## Usage by agent and model

All observed models ran with medium reasoning. Cached input is included in input;
reasoning is included in output. Do not add these subsets again.

| Agent / work | Actual model | Responses | Input | Cached input | Output | Reasoning | Total tokens | Estimated credits |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Root: coordination and closeout | GPT-6.1 Sol | 66 | 2,580,078 | 2,515,840 | 9,074 | 1,668 | 2,589,152 | 11.77000 |
| Fixture/API check | GPT-6 Luna | 18 | 774,304 | 704,256 | 3,449 | 891 | 777,753 | 0.39430 |
| Implementation and validation | GPT-6.1 Sol | 56 | 5,102,433 | 4,968,704 | 23,244 | 3,421 | 5,125,677 | 24.91921 |
| Independent final review | GPT-6 Astra | 15 | 682,417 | 620,928 | 3,449 | 116 | 685,866 | 35.20670 |
| **Total** | | **155** | **9,139,232** | **8,809,728** | **39,216** | **6,096** | **9,178,448** | **72.29021** |

By model: Sol **7,714,829 tokens / 36.68921 credits**; Luna **777,753 / 0.39430**;
Astra **685,866 / 35.20670**. Astra accounts for 7.5% of tokens and 48.7% of
estimated credits. Luna accounts for 8.5% of tokens and 0.5% of credits.

The estimate uses the October 1 rates recorded in the
[original audit](token_usage_audit.md): uncached input / cached input / output
per million tokens are Sol 50 / 2.5 / 250 credits, Luna 2.5 / 0.25 / 12.5, and
Astra 250 / 25 / 1250. Formula:
`((input - cached) * uncached_rate + cached * cached_rate + output * output_rate) / 1,000,000`.
This is an estimate, not an invoice or an allowance conversion.

## Work phases and time

| Phase, UTC | Tokens | Estimated credits |
| --- | ---: | ---: |
| Startup, before implementation spawn at 14:42:36 | 489,108 | 2.34164 |
| Implementation and validation, until review spawn at 15:02:38 | 6,868,103 | 29.86476 |
| Final review, until reviewer completion at 15:05:03 | 1,436,524 | 38.23388 |
| Closeout, until 15:07:09 | 384,713 | 1.84993 |

These are timestamp allocations, not isolated phase measurements. They include
overlapping coordinator and worker activity. Review-phase credits include more
than the reviewer alone.

The active task lasted **26m07.5s**; thread creation through completion lasted
**26m42.1s**. The prior active task lasted 10m14s. The reviewer ran **2m24.7s**,
from 15:02:38.932 to 15:05:03.638. The slice report's approximate five-minute
review duration overstates the logged interval. One Sol capacity interruption
recovered on the same thread; no model substitution occurred.

## Coordination, context, and Git

Responses associated with shell orchestration consumed 5,866,751 tokens;
messages 1,679,841; waits 929,810; follow-ups 86,510. Messages plus waits total
**2,609,651 tokens / 28.43%**; adding follow-ups gives 29.37%. Attribution uses
the tool calls emitted with each model response. It measures response context,
not the tokens inside a tool argument alone.

The root made 23 shell-orchestration calls, 24 waits, 11 sends, two follow-ups,
three spawns, and two agent listings. Wait intervals total approximately
14m28s, overlapping useful worker activity. Waiting itself does not charge
tokens per second; the model responses around the waits consume tokens.

Average input per response was approximately **58,963 tokens**, versus 54,769
previously. **96.4%** of input was cached. Shorter prose helps, but repeated
responses still resend accumulated context. The largest direct coordination
opportunity is the root's **2,589,152 tokens / 11.77 credits**.

There were **29 completed command invocations containing Git**: root 20,
implementation worker eight, reviewer one. This counts invocations, not every
Git subcommand inside a script. Startup checked status, baseline and ancestry.
Implementation checks at 14:47, 14:50 and 14:58 overlapped coordinator waits and
status checks. Closeout staged selected files, preserved existing preparation
edits, checked frozen evidence, and committed locally at 15:06:46.

The commit is `214c87b58`, with eight files changed, 1,416 insertions and eight
deletions. Git checks served baseline protection, review and selective staging.
There is no evidence that Git itself caused most usage. Command-associated
responses also contain previous instructions, conversation and tool output.
Avoid repeated mid-implementation status checks when they do not change a decision.

## Delivered scope and correction work

The [slice report](doc/evm_research/n4_synthetic_native_transition.md) records
real `plan_external_evm_rewards_stats -> begin_native_session_bound ->
finish_rewards` execution. Rust returned zero reward and mutations with unchanged
semantic state. Fresh pinned Go execution observed zero writes, mutation
attempts, backend puts and commits. All 27 generated Go raw rows matched Rust
canonical rows; two additional Rust rows and different read sets remain explicit.
This is bounded terminal-effect parity, not a complete historical snapshot or
production adoption result.

Recorded checks passed: focused Rust 3/3, rewards 16/16, consensus bridge 15/15,
storage bridge 4/4, and `make rewrite-validate-fast`. Independent Astra review
found no blockers. Frozen implementation and fixture outputs did not change
after review. The root reran both bridge binaries to retain complete logs;
capturing logs on their first run would remove this duplicate execution.

Four correction groups addressed fixture API types, period rejection and missing
Go genesis code, raw-write observation/input names, and shared-input checks and
representation labels. My preparation contract also incorrectly called slot 5
a 32-byte physical amount. Its encoding is compact big-endian U256; `0bb8` is
correct for 3000. I corrected that wording in the preparation contract. This is
a preparation defect, not an implementation parity failure.

## Next concrete changes

1. **Harden the existing evidence first.** Assert that the Go ordered raw-write
   list is empty and bind every relevant manifest input to both engines. These
   are the two nonblocking regression limits in the completed review. Preserve
   the observed outputs and oracle rules; rerun focused checks and independent
   review for changes. Do not expand into historical replay.
2. **Let the Sol driver implement directly.** Use Luna for one bounded check
   and a separate final reviewer. Spawn an implementation worker only for an
   independent scope that saves time. The current coordinator's 11.77 credits
   are a measured opportunity, not a guaranteed saving; some work transfers to
   the driver.
3. **Use milestone handoffs.** Report a concrete blocker, validation result,
   or source freeze. Remove short status polling and duplicate progress reads.
   Keep failure recovery and independent review.
4. **Reduce review cost for settled fixture work.** The existing model table
   permits an independent Sol-medium reviewer. At identical recorded review
   usage, Sol pricing would be 5.48902 credits instead of 35.20670: a 29.71768
   credit difference, or 41.1% of this session's total. This is a pricing-only
   counterfactual; equal review quality and token volume are not established.
   Keep Astra for unresolved authority, cryptographic, or historical semantics.
5. **Capture validation output once.** Save full logs and exit status on the
   first execution. Give reviewers paths, hashes, relevant source ranges and
   known differences; avoid repeated full reports and historical routing files.

The next measurement should record accepted scope, actual model routing,
responses, credits, corrections, validation coverage, and elapsed time. A lower
total that skips required checks is not an efficiency improvement.

## Allowance and audit method

Weekly account snapshots moved from **69% used at 14:41:09** to **70% used at
15:07:09**, leaving 30%. The reported balance stayed at 2,118.401685. These are
rounded account-level observations, separate from tokens and estimated credits.
The standing policy starts no new slice at 30% or less remaining; check current
telemetry before another slice. This historical observation is not a current
account reading or a task-specific charge.

The audit found **155 unique `(thread_id, response_id)` records**, zero duplicate
response records, **165 cumulative snapshots with ten repeated snapshots**, and
no compaction. It summed each response once, reconciled all five counter fields
against every thread's final cumulative usage, and did not add snapshots or
cached/reasoning subsets to totals. Runtime models came from `turn_context`,
not requested assignment alone. Descendants were discovered recursively from
logged agent activity. This audit's own usage is excluded.

Logs are under `/home/fry/.codex/sessions/2026/10/01/`. Thread IDs:

- Root: `01a0f7e8-d899-7453-8f1e-4b3fad23a54d`.
- Luna: `01a0f7ea-0637-79f0-bd8d-ea300bec1e63`.
- Sol implementation: `01a0f7ea-cfb2-7131-8de0-fa8df7c4fa29`.
- Astra review: `01a0f7fd-2a32-7e71-9bca-92949238b757`.

Reproducible [summary script](/home/fry/artifacts/token-audit-01a0f7e8/summarize.py)
and [derived JSON](/home/fry/artifacts/token-audit-01a0f7e8/summary.json) are
persistent local artifacts. Repository changes from this audit are this report
and the storage-width correction; neither is committed.
