# Token usage audit: 01a10f48

Audit date: 2026-10-08. Scope: local logs for session `01a10f48-83ee-7a32-8bff-722d10c36359` and all 56 descendants. Logs were read only. The original session was not resumed. Existing EVM changes were left unchanged.

## Assessment

Model routing improved. The estimated cost share of Astra fell from 66.84% in the previous comparison run to 41.96%. Average input per response fell from about 118,766 to 92,852 tokens. Validation and independent review still found material gaps.

This session contains **four authorized 10-point runs: 40 observed allowance points**, rather than 20–30. Git history contains **80 commits: 65 code or test commits, 14 documentation commits and one preparation commit**. These are not 80 independent features. Four files from the final slice remain unaccepted.

The cost per code/test commit proxy fell about 21.5% against the previous comparison run. Tokens per commit rose about 5.7%, and elapsed time per commit was nearly unchanged. Scope and commit size differ, so this is evidence of better routing, not a controlled measurement of productivity or proven savings.

The next useful change is a fresh lead session with a compact handoff. Then finish validation of the pending slice before starting more implementation. Keep the current budget controls and independent review.

## Counters and method

| Counter | Tokens |
|---|---:|
| Input, including cached input | 365,838,256 |
| Cached input, subset of input | 356,170,752 |
| Uncached input | 9,667,504 |
| Output, including reasoning | 1,584,000 |
| Reasoning, subset of output | 296,821 |
| Total: input plus output | **367,422,256** |

The input cache share was 97.36%. Cached input still has an estimated cost; it is not free. Short final replies alone cannot remove the repeated input context that dominates this profile.

The audit counted 3,940 unique response usage records. No duplicate or conflicting response records were found. All thread and turn totals reconcile. The 4,161 cumulative snapshots include 236 repeated values; these snapshots were not summed. There were 15 compaction records: 14 in the lead session and one in a Luna thread.

Descendants were identified from local thread activity and spawn relationships. References back to the root were not counted as new agents. Runtime metadata supplies model and effort; requested assignments are not treated as billing evidence. All observed models used medium effort.

Phase allocation assigns each response once by its timestamp. It includes implementation, validation, review and closeout within each run. These logs do not support a precise split of every token into those activities. Git timestamps establish accepted commit counts, not an isolated token bill for each commit. Some command arguments are encrypted, and wrapped commands do not expose every underlying invocation.

Reproducible inputs and outputs: [audit script](/home/fry/artifacts/token-audit-01a10f48/audit.py), [phase refinement](/home/fry/artifacts/token-audit-01a10f48/refine.py), [full counters and agent records](/home/fry/artifacts/token-audit-01a10f48/summary.json), [phase and command evidence](/home/fry/artifacts/token-audit-01a10f48/refinement.json), [Git timestamps](/home/fry/artifacts/token-audit-01a10f48/git_commits.txt).

## Models and estimated cost

These are historical Standard credit estimates using the October 1 assumptions from [the previous audit](token_usage_audit_01a0ff3b.md). Rates per million uncached input / cached input / output tokens are Sol 50 / 2.5 / 250; Luna 2.5 / 0.25 / 12.5; Astra 250 / 25 / 1,250. They are not an invoice or a current price quote. Reasoning is already included in output.

| Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 3,346 | 327,816,696 | 320,094,848 | 1,327,646 | 329,144,342 | 1,518.24102 | 57.81% |
| gpt-6-luna | 176 | 14,144,647 | 13,550,080 | 95,552 | 14,240,199 | 6.06834 | 0.23% |
| gpt-6-astra | 418 | 23,876,913 | 22,525,824 | 160,802 | 24,037,715 | 1,101.92035 | 41.96% |

Total estimated cost: **2,626.22971 credits**. Luna supplied bounded mapping work at low estimated cost. Astra supplied contract and review work. Sol supplied the lead and most reviews.

| Agent group | Threads | Total tokens | Estimated credits |
|---|---:|---:|---:|
| Sol lead | 1 | 237,473,205 | 1,061.34973 |
| Sol reviewers | 32 | 91,671,137 | 456.89129 |
| Astra contract/review agents | 20 | 24,037,715 | 1,101.92035 |
| Luna mapping agents | 4 | 14,240,199 | 6.06834 |

The lead alone used 231,763,072 cached input tokens, estimated at **579.40768 credits**, about 22% of the total estimate. This is a larger cost target than the response contexts associated with waits. The estimate does not prove that all of this context could be removed safely.

## Runs, allowance and Git work

Allowance percentages are account telemetry. They are separate from token counters and estimated credits. Other account activity can affect them. Do not derive a token price from a percentage point.

| Run | UTC work window | Elapsed | Remaining start → stop | Tokens | Estimated credits | All commits | Code/test commits |
|---|---|---|---|---:|---:|---:|---:|
| 1 | Oct 6 03:36:51 → 05:50:56 | 2h 14m | 66% → 56% | 83,001,988 | 642.03887 | 19 | 18 |
| 2 | Oct 6 12:51:35 → 15:07:55 | 2h 16m | 56% → 46% | 79,839,960 | 690.58957 | 20 | 14 |
| 3 | Oct 7 12:54:51 → 16:17:52 | 3h 23m | 99% → 89% | 92,728,081 | 713.27417 | 21 | 16 |
| 4 | Oct 7 22:22:52 → Oct 8 01:28:32 | 3h 06m | 89% → 79% | 110,793,145 | 567.79552 | 20 | 17 |
| Between runs and final status | Separate user/status turns | Excluded from work windows | — | 1,059,082 | 12.53158 | 0 | 0 |

The four work windows total **10h 59m 5s**. This includes recovery after the run-three interruption. Summing only completed task intervals would omit most of that interrupted run and understate elapsed time. The full session spans about 47h 39m, including idle time.

Each run had its own authorized, immutable startup budget. Reset identity was `1791584594` for runs one and two, and `1791948695` for runs three and four. The identity changed and balance rose from 46% to 99% before the old advertised reset time. The logs do not establish why; this must not be called a normal weekly reset. Across the separately authorized windows, each recorded stop reached its ten-point limit.

Visible direct quota-reader command records: 447 completed commands, with 415 parsed JSON observations: 386 allow-start, 25 unknown and four do-not-start. Wrapped or non-JSON records account for the remainder. These are visible command records, not a count of every internal quota-reader call. Unknown observations require resolution; they do not authorize extra work.

Git range: `049ec5e73..5ff1f0fc5056656d78e5ea6d4a26333d84e6230c`. Types: 55 test, five feat, five fix, 14 docs and one chore. Per-run endpoints were `fd6a7313c`, `3b8d2e31e`, `28abfdec9` and `5ff1f0fc5`. No pushes were recorded.

Run four used the most tokens but had the lowest run cost estimate. Astra cost fell to about 130 credits there, versus 291–357 in earlier runs. Equal ten-point account movements therefore do not establish equal token or credit costs.

## Work and acceptance

| Run | Main work phase | Remaining limit |
|---|---|---|
| 1 | Zero/API cases; source/destination state transitions; current-source copy correction; frames and H2 evidence | Finite qualification only |
| 2 | Signed H1 simulation/estimation; source-last removal; existing destination and public frames | Latest H1 oracle validation carried into run three |
| 3 | H1/H2 existing-state cases; both-current and repeated-owner cases; retained excess and reward contracts | Real-network input and root parity still open |
| 4 | Positive rewards; canonical nonce/period correction; ordered writes; H1 runtime/simulation/estimation; owner/delegator traces; claim-failure Go oracle | Final malformed-claim runtime fix remains unaccepted |

Independent review had useful results. In run three, an H1 test compared Go physical rows but did not prove separate public Rust owner-database isolation. Review required physical row snapshots after faults, success, drop and reopen. The first correction used the wrong database path; the corrected helper and validation then closed that gap. This supports keeping review even when reducing coordination.

Run-four final authority is [final-closeout.json](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007/final-closeout.json), [final-quota.json](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007/final-quota.json) and [final checkpoint](/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007/final-checkpoint.md). An earlier repository checkpoint recorded 80% remaining; final telemetry records 79% and is authoritative.

Four pending files are outside accepted work:

- `rust/crates/rustaxa-consensus/src/final_chain/native_session.rs`
- `rust/crates/rustaxa-evm/tests/native_simulation_reference.rs`
- `rust/crates/rustaxa-evm/tests/native_simulation_reference/reward_claims_h1_failure.rs`
- `doc/evm_research/n3_reward_claims_h1_failure.md`

Focused public and unit tests each matched and passed one test, and formatting passed. The full gate stopped at the quota check before its first gate. Full consensus/EVM checks, clippy, bridge tests, serial fast validation, whitespace checks and final review are still required under a new authorized budget. A zero-test exact selection was correctly excluded as evidence. The accepted Go oracle does not establish acceptance of the pending runtime fix.

Milestone 10 and N1–N6 real-network input qualification, root parity, recovery and integrated cutover remain open. Commit count must not imply those milestones are complete.

## Comparison and improvement priorities

| Diagnostic proxy | Previous 01a0ff3b | Current 01a10f48 |
|---|---:|---:|
| Code/test commits used as denominator | 17 | 65 |
| Tokens per code/test commit | 5.347M | 5.653M |
| Estimated credits per code/test commit | 51.44 | 40.40 |
| Work-window minutes per code/test commit | 10.27 | 10.14 |
| Average input tokens per response | 118,766 | 92,852 |
| Astra share of estimated credits | 66.84% | 41.96% |

These proxies include preparation, documentation, status and pending work costs. Different semantic scope and commit granularity prevent a like-for-like benchmark. They show improved model mix but no clear throughput gain per commit.

1. **Start the next budget in a fresh lead session.** Supply the current checkpoint, accepted HEAD, pending paths and hashes, invariants, required checks and the next bounded objective. Link historical evidence instead of embedding it. Measure average lead input, uncached input, cost estimate and accepted behavior per run. Do not reset a baseline within an active authorized run.
2. **Close the pending slice first.** Verify the four live files against frozen evidence. Use a new gate runner tied to the new budget, complete required checks, request independent final review and then commit if accepted. Do not reuse the closed run-four budget or imply its gate passed.
3. **Keep Sol as lead and use Astra for explicit unresolved questions.** A compact contract review should state the precise decision and evidence required. Preserve independent review where it found parity or isolation gaps. Track issues found and resolved, not reviewer token volume alone. Keep Luna for bounded mapping and routine evidence tasks when available.
4. **Request final review after required checks finish.** If early source review is useful, let that bounded pass finish, then send one evidence follow-up. Avoid leaving reviewers in repeated waits for gate results. Use completion notifications and concise evidence handoffs.
5. **Keep communication short and measurements local.** Put detailed evidence in artifacts. Send deltas, results and blockers. Query allowance at startup, required safety boundaries and closeout; preserve freshness checks before new work. Do not weaken quota handling to reduce command count.
6. **Require nonzero matched tests.** Check exact test selectors before expensive gates. Treat zero matches as missing evidence. Preserve relevant parity and runtime tests; do not reduce test coverage for a cheaper token profile.

Coordination recorded 452 sends, 239 waits, 88 follow-ups, 73 sleeps and 60 agent listings. Response contexts containing send/wait calls account for 14.87% of tokens; including follow-ups raises this to 18.12%. This includes useful work and repeated context, so it is not a recoverable waste estimate.

Of the 239 waits, 70 completed and 169 timed out. Review threads made 224 waits; the lead made 15. The paired response contexts total 16.205M tokens and about **48.44 estimated credits, 1.84% of total cost**. These are associated response costs, not a charge for waiting. Encrypted arguments prevent proof of every wait reason. Review records indicate that reviewers often waited for final gate evidence. Better scheduling can help coordination, but lead context and Astra scope offer larger cost opportunities.

Do not claim a drastic saving yet. Test a fresh-session handoff on the next bounded run, retain equivalent validation, and compare accepted behavior and defects resolved as well as credits and time. No further instruction-file cleanup is justified by this audit alone.

## Agent appendix

Every thread is listed below. Medium effort applies throughout. Input includes cached input; output includes reasoning. The linked full counter record includes reasoning subtotals, reconciliation and source-log paths.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a10f48-83ee-7a32-8bff-722d10c36359` | gpt-6.1-sol | 1,981 | 236,491,028 | 231,763,072 | 982,177 | 237,473,205 | 1,061.34973 |
| /root/luna_zero_seam | `01a10f49-79b9-7451-8cd3-81853bdfc248` | gpt-6-luna | 18 | 877,409 | 776,960 | 7,182 | 884,591 | 0.53514 |
| /root/sol_prep_review | `01a10f4a-2304-7b91-a615-446522919a9a` | gpt-6.1-sol | 16 | 763,603 | 695,680 | 5,212 | 768,815 | 6.43835 |
| /root/sol_zero_api_review | `01a10f50-ebac-7c70-9128-584b75c46fc2` | gpt-6.1-sol | 26 | 1,645,432 | 1,557,376 | 13,271 | 1,658,703 | 11.61399 |
| /root/astra_full_new_contract | `01a10f5a-bdab-74f2-b12b-8b8bf824bc86` | gpt-6-astra | 36 | 2,454,420 | 2,340,992 | 16,145 | 2,470,565 | 107.06305 |
| /root/sol_full_new_review | `01a10f6b-00c4-7e12-945a-473f6fbd0a3b` | gpt-6.1-sol | 47 | 4,503,498 | 4,350,720 | 18,114 | 4,521,612 | 23.04420 |
| /root/astra_swap_append_contract | `01a10f81-f452-71f3-b52f-5a43454b6fab` | gpt-6-astra | 11 | 436,032 | 392,192 | 4,136 | 440,168 | 25.93480 |
| /root/sol_swap_append_review | `01a10f82-957d-78d3-808b-0a034c197091` | gpt-6.1-sol | 43 | 3,467,222 | 3,336,832 | 13,553 | 3,480,775 | 18.24983 |
| /root/astra_current_node_contract | `01a10f92-875b-74b3-a5b4-4991b5b9dcf1` | gpt-6-astra | 50 | 3,844,778 | 3,703,168 | 23,928 | 3,868,706 | 157.89170 |
| /root/sol_current_source_review | `01a10fa5-51e7-7971-9657-de0731989053` | gpt-6.1-sol | 77 | 7,538,909 | 7,380,992 | 22,132 | 7,561,041 | 31.88133 |
| /root/luna_r2_next_seam | `01a11145-b33e-77c2-807e-70f7a46e1880` | gpt-6-luna | 39 | 2,463,722 | 2,323,968 | 23,444 | 2,487,166 | 1.22343 |
| /root/astra_r2_current_history_contract | `01a11148-4867-7ad1-a31e-83a821525d4a` | gpt-6-astra | 20 | 982,972 | 906,880 | 5,129 | 988,101 | 48.10625 |
| /root/sol_r2_prep_review | `01a11148-dd0a-7a90-b7fa-cb096bf71e34` | gpt-6.1-sol | 65 | 4,648,576 | 4,507,392 | 15,714 | 4,664,290 | 22.25618 |
| /root/sol_r2_signed_h1_review | `01a1115b-879a-76c0-8fcb-0f506dfd30c4` | gpt-6.1-sol | 28 | 1,489,717 | 1,424,768 | 8,008 | 1,497,725 | 8.81137 |
| /root/astra_r2_source_last_contract | `01a11160-4cec-7660-aa43-534523e558d1` | gpt-6-astra | 27 | 1,458,702 | 1,393,408 | 8,762 | 1,467,464 | 62.11120 |
| /root/sol_r2_source_last_review | `01a11167-69e9-7111-bbc0-92613ea0ff30` | gpt-6.1-sol | 37 | 2,953,664 | 2,849,024 | 11,676 | 2,965,340 | 15.27356 |
| /root/astra_r2_source_last_h1_contract | `01a1116f-1f42-7b52-ba36-45f2a3dbbb68` | gpt-6-astra | 12 | 546,176 | 504,704 | 4,539 | 550,715 | 28.65935 |
| /root/sol_r2_source_last_h1_review | `01a11176-a202-7b33-959b-35d6ca3f4512` | gpt-6.1-sol | 31 | 1,676,031 | 1,605,504 | 8,175 | 1,684,206 | 9.58386 |
| /root/astra_r2_source_last_absent_contract | `01a11178-1036-7e72-bb03-0f73a10b46b7` | gpt-6-astra | 39 | 2,982,228 | 2,868,992 | 16,836 | 2,999,064 | 121.07880 |
| /root/sol_r2_source_last_absent_review | `01a11186-18c3-7113-8841-7550065f9967` | gpt-6.1-sol | 39 | 2,839,977 | 2,741,248 | 12,140 | 2,852,117 | 14.82457 |
| /root/sol_r2_source_last_absent_h1_review | `01a11194-2d0f-7e41-80ab-df45c6cb5c7e` | gpt-6.1-sol | 25 | 1,312,172 | 1,251,328 | 8,087 | 1,320,259 | 8.19227 |
| /root/astra_r2_existing_destination_source_last_contract | `01a1119a-0bd1-70c1-b1fe-b62836970d73` | gpt-6-astra | 12 | 537,440 | 474,496 | 4,803 | 542,243 | 33.60215 |
| /root/astra_r2_existing_current_contract | `01a1119e-3d11-7a81-b69d-6bb8d8ddda5d` | gpt-6-astra | 14 | 612,073 | 570,752 | 4,809 | 616,882 | 30.61030 |
| /root/sol_r2_existing_current_review | `01a111a6-6b85-7f00-a67c-a4b448fd204c` | gpt-6.1-sol | 45 | 3,925,240 | 3,811,328 | 17,238 | 3,942,478 | 19.53342 |
| /root/astra_r2_existing_current_h1_contract | `01a111b4-ae69-7a03-938c-7cd8511b1704` | gpt-6-astra | 15 | 619,027 | 571,392 | 5,404 | 624,431 | 32.94855 |
| /root/sol_r2_existing_current_h1_oracle_review | `01a111bb-85e5-7891-a0c1-5b1544471ef6` | gpt-6.1-sol | 26 | 1,229,563 | 1,172,864 | 6,479 | 1,236,042 | 7.38686 |
| /root/luna_r3_existing_current_h1_seam | `01a1166f-315e-7f62-b20c-69c3f10cff57` | gpt-6-luna | 27 | 1,169,803 | 1,101,056 | 9,879 | 1,179,682 | 0.57062 |
| /root/sol_r3_existing_current_h1_review | `01a11677-8470-7b63-8a47-3f0d75ce5674` | gpt-6.1-sol | 29 | 1,533,490 | 1,452,288 | 8,268 | 1,541,758 | 9.75782 |
| /root/astra_r3_existing_current_trace_contract | `01a11679-dbfb-7831-9704-5380fc4fc3bc` | gpt-6-astra | 15 | 728,306 | 658,688 | 6,384 | 734,690 | 41.85170 |
| /root/sol_r3_existing_current_h2_trace_review | `01a11684-59f6-74f0-bbc6-505d0e3fd0ab` | gpt-6.1-sol | 25 | 1,366,689 | 1,290,880 | 8,109 | 1,374,798 | 9.04490 |
| /root/astra_r3_both_current_three_contract | `01a11685-ff07-7b62-aab6-c367cd162f6c` | gpt-6-astra | 32 | 1,916,018 | 1,846,400 | 10,922 | 1,926,940 | 77.21700 |
| /root/sol_r3_both_current_three_review | `01a1168f-f454-77d3-ba87-3dd6771003da` | gpt-6.1-sol | 53 | 4,939,668 | 4,803,584 | 16,372 | 4,956,040 | 22.90616 |
| /root/sol_r3_both_current_three_frames_review | `01a116b1-f56a-70d1-90ac-0500a7c9a916` | gpt-6.1-sol | 35 | 1,579,887 | 1,527,168 | 6,542 | 1,586,429 | 8.08937 |
| /root/astra_r3_both_current_three_h1_contract | `01a116b5-b5de-7391-bc8f-ac305d508d25` | gpt-6-astra | 14 | 657,054 | 609,408 | 5,863 | 662,917 | 34.47545 |
| /root/sol_r3_both_current_three_h1_review | `01a116bc-bb5a-78e1-b3f7-04699ee15de8` | gpt-6.1-sol | 59 | 3,290,480 | 3,223,168 | 9,437 | 3,299,917 | 13.78277 |
| /root/astra_r3_both_current_three_h2_contract | `01a116c2-d6c6-7091-b78c-e745cc2d0c57` | gpt-6-astra | 13 | 512,549 | 477,056 | 5,335 | 517,884 | 27.46840 |
| /root/sol_r3_both_current_three_h2_review | `01a116c9-c21d-7bf3-9c7c-060b0f2f236f` | gpt-6.1-sol | 47 | 1,633,220 | 1,581,056 | 4,748 | 1,637,968 | 7.74784 |
| /root/astra_r3_current_owner_repeat_contract | `01a116d6-60db-7f23-9312-b6ca42cf117d` | gpt-6-astra | 21 | 1,195,295 | 1,104,768 | 7,717 | 1,203,012 | 59.89720 |
| /root/sol_r3_both_current_four_review | `01a116e2-92a4-7770-8e69-b8c5ce95942e` | gpt-6.1-sol | 49 | 4,010,095 | 3,897,856 | 14,333 | 4,024,428 | 18.93984 |
| /root/sol_r3_both_current_four_h1_review | `01a116f9-29d1-7502-8d64-2a3c69b35aec` | gpt-6.1-sol | 61 | 4,218,205 | 4,068,096 | 11,609 | 4,229,814 | 20.57794 |
| /root/astra_r3_both_current_four_h2_contract | `01a11701-2433-7d50-ba48-1dc1289b57a0` | gpt-6-astra | 13 | 644,793 | 585,216 | 6,280 | 651,073 | 37.37465 |
| /root/sol_r3_both_current_four_h2_review | `01a11710-f7ff-7030-9379-8bccbbd107bb` | gpt-6.1-sol | 34 | 1,515,842 | 1,425,664 | 4,058 | 1,519,900 | 9.08756 |
| /root/astra_r3_rewarded_partial_contract | `01a1171d-c0c5-7761-8f45-9a3afac96b51` | gpt-6-astra | 18 | 938,057 | 877,568 | 6,476 | 944,533 | 45.15645 |
| /root/sol_r3_rewarded_partial_contract_review | `01a1171e-bd6b-7bd0-97ff-c10b572690e0` | gpt-6.1-sol | 26 | 926,790 | 822,784 | 5,173 | 931,963 | 8.55051 |
| /root/luna_r4_rewarded_partial_seam | `01a11876-d5ae-79a2-8e5a-0fd64b369533` | gpt-6-luna | 92 | 9,633,713 | 9,348,096 | 55,047 | 9,688,760 | 3.73915 |
| /root/astra_r4_reward_auth_contract | `01a1187b-0011-7750-9ef6-80004c12c303` | gpt-6-astra | 21 | 1,306,360 | 1,215,872 | 6,946 | 1,313,306 | 61.70130 |
| /root/sol_r4_reward_oracle_review | `01a1187b-eea4-7840-8441-07ca94e74385` | gpt-6.1-sol | 60 | 4,994,949 | 4,865,792 | 14,899 | 5,009,848 | 22.34708 |
| /root/sol_r4_reward_frames_review | `01a118a0-b9b8-7f60-a7ba-41d9448b576a` | gpt-6.1-sol | 85 | 6,483,821 | 6,364,160 | 18,820 | 6,502,641 | 26.59845 |
| /root/astra_r4_reward_h1_contract | `01a118b3-8766-7931-8c87-2aa7e1883ada` | gpt-6-astra | 22 | 1,053,388 | 1,001,600 | 6,069 | 1,059,457 | 45.57325 |
| /root/sol_r4_reward_h1_review | `01a118bf-611e-77f3-ac78-64d80560f876` | gpt-6.1-sol | 66 | 5,705,887 | 5,579,136 | 20,040 | 5,725,927 | 25.29539 |
| /root/astra_r4_reward_trace_contract | `01a118cf-30e7-7ea0-b35e-8ee8cbb38ca5` | gpt-6-astra | 13 | 451,245 | 422,272 | 4,319 | 455,564 | 23.19880 |
| /root/sol_r4_reward_trace_review | `01a118d8-98a8-7641-b23d-8527d7d80b0f` | gpt-6.1-sol | 44 | 2,200,651 | 2,139,520 | 10,853 | 2,211,504 | 11.11860 |
| /root/sol_r4_commission_trace_review | `01a118e4-88b2-7821-8cce-6d5bef93b656` | gpt-6.1-sol | 28 | 1,141,284 | 1,068,672 | 5,397 | 1,146,681 | 7.65153 |
| /root/sol_r4_delegator_trace_review | `01a118ec-4e4e-7350-a43c-285d735e6cd4` | gpt-6.1-sol | 32 | 1,375,551 | 1,323,648 | 5,766 | 1,381,317 | 7.34577 |
| /root/sol_r4_delegator_h1_review | `01a118f2-bae1-7873-8858-1779af248dbf` | gpt-6.1-sol | 43 | 2,054,598 | 1,989,888 | 7,483 | 2,062,081 | 10.08097 |
| /root/sol_r4_commission_h1_review | `01a118fe-1951-7982-aa34-3777c78b6ffc` | gpt-6.1-sol | 39 | 1,666,432 | 1,608,064 | 6,321 | 1,672,753 | 8.51881 |
| /root/sol_r4_claims_failure_review | `01a11913-ac07-7b53-85e8-958ac23128ed` | gpt-6.1-sol | 45 | 2,694,525 | 2,615,296 | 7,442 | 2,701,967 | 12.36019 |
