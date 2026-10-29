# Effectiveness audit: 01a12d5e

Audit date2026-10-13. Scope: local session `01a12d5e-f42f-74f0-86fb-26bc2c433942`
and all19 descendants. Logs were read only; audited sessions/agents were not
resumed or contacted. No implementation or workflow was changed.

## Verdict

**Effective evidence closure, but an expensive process that should be improved
before repeating more voter adapters.** The session accepted302 VRF facts across
three separately authorized rounds. Six voters now have all48 required periods.
Each physical command had reviewed bounds, one guarded capture and independently
replayed proof evidence. No production behavior or real replay is claimed.

Estimated cost is1952.30285 historical credits. Astra supplies74.14% of that cost,
versus about30% in recent audits. Repeated per-voter authority threads and nearly
identical adapter implementations are the main visible scaling problem. Lower
commit counts alone are not evidence of waste, but copied implementations and
repeated authority work are concrete improvement targets.

Do not remove proof authentication, old-profile rejection, bounds, single-use
commands, required tests or independent review. Replace repeated construction
with a reviewed shared implementation and deterministic profile artifacts.

## Outcomes and allowance

| Authorized round | Historical allowance | New accepted VRF facts | Complete voters, cumulative | Tokens | Estimated credits |
|---|---|---:|---:|---:|---:|
| 1 |64%→56%,8 points |67 |1 |51,305,021 |472.70387 |
| 2 |55%→45%,10 points |141 |4 |71,174,776 |757.04369 |
| 3 |45%→35%,10 points |94 |6 |57,042,404 |701.92618 |
| Status/user turns outside work windows |Separate observations |0 |— |614,443 |20.62911 |

The three budgets account for**28 observed points**, not one ten-point run.
The full64%→35% balance difference is29 points: one additional point sits between
round-one final56% and round-two baseline55%. Account telemetry can include other
activity and status work; it is not a token bill. Baselines/reset identities stayed
in their saved files across each round. No conversion from points to tokens is made.
Round one stopped with its accepted scope closed and the next implementation not
READY; later rounds reached their floors under new user authorization.

Main execution windows:
- Oct11 23:49:22.063→Oct12 01:26:06.107.
- Oct12 18:37:03.422→20:36:53.516.
- Oct13 00:34:58.568→02:09:53.949.

These total5h11m30s, excluding idle time and short status turns. All paired lead
turns total5h13m00s. Timestamp allocation assigns every response once but cannot
isolate coding, review and gates as causal token charges.

Git range `3e070d0c4..28c62a714` contains10 commits: seven source commits and
three reviewed documentation commits. The two first storage commits establish
first-period20 and the initial47-row window. Later qualifier commits close five
more47-row windows. Nothing was pushed. All seven physical guards are consumed;
do not rerun them. The seventh-voter design/guard/metadata preparation is reviewed,
but its workspace implementation, gates, full command freeze/READY and capture
remain unstarted.

Final authority: [round-one checkpoint](/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/checkpoint.md),
[round-two checkpoint](/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/checkpoint.md),
[round-three final checkpoint](/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/checkpoint-final.md).
The round-three frozen quota record captured nine points; the later stop
observation/final checkpoint correctly supersedes it with ten. Preserve both.

## Deduplicated usage

| Counter | Tokens |
|---|---:|
| Input including cached input |179,048,488 |
| Cached input, subset of input |174,280,832 |
| Uncached input |4,767,656 |
| Output including reasoning |1,088,156 |
| Reasoning, subset of output |353,510 |
| Total: input plus output |**180,136,644** |

There are1791 unique response records across20 threads; no duplicate/conflicting
usage records or missing descendant logs. Thread and turn counters reconcile.
1943 cumulative snapshots include160 repeats; snapshots were not summed.
Nine compaction records were found: three lead, five Luna overlap-worker and one Luna startup child. No Astra compaction was recorded. Models and effort come from runtime metadata;
all observed contexts used medium effort. Assignments/reviewer prose do not supply
billing evidence. Cached input and reasoning are subsets, not additional totals.

Evidence and reproducible scripts: [summary.json](/home/fry/artifacts/token-audit-01a12d5e/summary.json),
[refinement.json](/home/fry/artifacts/token-audit-01a12d5e/refinement.json),
[audit.py](/home/fry/artifacts/token-audit-01a12d5e/audit.py),
[refine.py](/home/fry/artifacts/token-audit-01a12d5e/refine.py).

## Model cost profile

Use the same historical October1 Standard credit assumptions as
[the previous audit](token_usage_audit_01a12418.md). Rates per million uncached /
cached / output: Sol50/2.5/250; Luna2.5/0.25/12.5; Astra250/25/1250.
These are comparison estimates, not invoices or current prices.

| Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits | Cost share |
|---|---:|---:|---:|---:|---:|---:|---:|
| gpt-6.1-sol | 927 | 101,393,399 | 99,085,952 | 496,009 | 101,889,408 | 487.08948 | 24.95% |
| gpt-6-luna | 452 | 40,218,534 | 38,860,544 | 381,409 | 40,599,943 | 17.87772 | 0.92% |
| gpt-6-astra | 412 | 37,436,555 | 36,334,336 | 210,738 | 37,647,293 | 1,447.33565 | 74.13% |

The lead used81,029,004 total tokens over654 responses, with mean input123,314
per response. That is about20% above the103,097 lead mean in01a12418. Its runtime
reported a475K context window, but a larger window alone does not establish why
more context was used. The lead estimate366.90732 credits is18.8% of total.

Eight VRF Astra authority threads plus a lambda authority thread cost1447.33565
credits. Four individual window threads cost about185–247 credits each. Luna
processed40.60M tokens for17.88 credits; optimizing its token volume first would
miss the largest financial target. There is no basis to claim all authority work
was unnecessary, especially during initial family design and discovered defects.
Once the shared semantics are settled, repeating that work for each fixed voter
needs a narrower route.

## Throughput and repeated work

Round two accepted141 facts for757.04 credits, about5.37 credits/fact. Round three
accepted94 for701.93, about7.47/fact: roughly39% higher. This includes preparation
for the next family, different corrections and gates; it is not a controlled
benchmark. At the latter pace,612 remaining VRF facts would be roughly6.5 similar
rounds, versus4.3 at round-two pace. Neither rate predicts the cost of the other
input families or total N4 completion. Changing the repeated workflow is preferable
to budgeting many identical rounds from these weak estimates.

Observed source duplication is concrete: fifth/sixth modules are591/622 lines,
with95.5% line similarity after normalizing profile names. Both contain the same
selection/prior/authority/preflight/capture/replay structure and similar tests.
There are seven VRF modules and fourteen thin live/replay binaries at the inspected
HEAD. Each profile adds source closure, command artifacts and review work.
Historical source copies reached345 and the final offline verification covered644
pins/423MB. Immutable history must remain intact, but historical evidence need not
be reread into agent context or reimplemented for every new voter.

A fifth-family capture took about28s and replay26s; the sixth capture also took28s.
These are examples, not full gate timing. They show that database capture time
alone does not explain the many minutes spent accepting a family.

Coordination recorded239 sends,77 follow-ups and57 agent waits, plus six process
waits and40 sleeps. Send/agent-wait response contexts account for29,416,808 tokens
(16.33%); including follow-ups39,917,040 (22.16%). These are associated contexts,
not pure waste. Agent-wait contexts alone cost an estimated73.56 credits,3.77%
of total. Review/correction duplication is a larger target than waits alone.

Quality checks caught useful problems: sixth review required an actual all-bin
build; guard/helper PATH/reverse-profile/closed-pipe behavior and a hanging-child
deadline regression were corrected; tuple-shape precheck stopped before starting
replay; closeout publication stopped on a missing checkpoint. Keep those protections.
One early Luna startup child produced no useful work; record startup once, then
use the existing worker for bounded related tasks.

## Recommended next change

1. **Review a common sealed VRF engine before another copied adapter.** Keep a
   fixed accepted request universe, typed immutable profile selection, root/key/
   prior-proof binding, ordering, full-consumption and reverse-profile rejection.
   Keep47-payload command bounds and all13 ceilings. Do not turn this into an
   arbitrary voter/root/period API or expand a physical command.
2. **Make profile manifests and guard metadata deterministic.** A tested builder
   should emit the exact source/executable/input/output pins and consume marker,
   with collision/size/timeout/publication controls. Preserve old frozen commands
   and archives; never rewrite historical acceptance or rerun consumed captures.
3. **Settle authority once for unchanged family semantics.** Astra reviews shared
   construction and named new authority questions. Independent Sol can review
   deterministic voter-profile deltas, exact command facts and offline replay once
   the common contract explicitly permits that route. A changed retention, source,
   cap, proof or consumer rule still needs fresh authority review. This is a proposed
   contract change, not permission to bypass current per-profile requirements.
4. **Use a fresh lead at the next authorized round.** Supply the current checkpoint,
   settled shared contract and exact delta, not all previous voter history. Preserve
   old budgets and use a newly authorized baseline. Reduce correction follow-ups
   with complete assignments and machine-checked profile builders.
5. **Measure accepted dependencies, not copies or commits.** Compare facts per
   credit/time after equivalent validation; track corrections and consumer coverage.
   Continue other bounded lambda/cache/stake/jail preparation when VRF is blocked,
   rather than spending the entire budget drafting another unavailable command.

Do not apply a broad refactor without a bounded contract, regression controls,
independent review and applicable storage/bridge gates. No implementation or skill
changes were applied during this audit. There is no new failure that justifies
loosening completeness, proof or read constraints.

## Remaining milestone work

Accepted302/914 VRF facts is33.0% of this input family, not overall replacement
progress. Six complete voters are closed;612 VRF and1828 stake/per-voter-jail rows
remain (2440 total voter rows). Also open:48 lambda selections,48 raw expected cache
RLP rows and producer/configuration/historical-policy authority. Voter weights,
cache equality, authoritative parent owner, full19-transfer reward transition,
real-network roots/recovery and production cutover remain downstream.

The work stayed on a required N4 dependency. It was effective technically, but the
current fixed-voter implementation process will scale poorly across the remaining
inputs. The next round should invest in the shared bounded workflow before treating
all remaining voters as separate new implementation families.

## Agent appendix

All actual contexts are medium. Input includes cached input; output includes reasoning.

| Agent | Thread | Actual model | Responses | Input | Cached input | Output | Total tokens | Estimated credits |
|---|---|---|---:|---:|---:|---:|---:|---:|
| /root | `01a12d5e-f42f-74f0-86fb-26bc2c433942` | gpt-6.1-sol | 654 | 80,647,464 | 79,175,808 | 381,540 | 81,029,004 | 366.90732 |
| /root/vrf_overlap | `01a12d5f-6fc6-7c30-a06b-9d4067b84ff8` | gpt-6-luna | 448 | 40,117,432 | 38,787,584 | 380,777 | 40,498,209 | 17.78123 |
| /root/vrf_authority | `01a12d60-15aa-7d22-a016-601ae1459ff8` | gpt-6-astra | 47 | 3,838,543 | 3,715,840 | 20,803 | 3,859,346 | 149.57550 |
| /root/vrf_code_review | `01a12d68-7481-74a0-b692-2942392b093b` | gpt-6.1-sol | 26 | 1,775,273 | 1,670,272 | 8,847 | 1,784,120 | 11.63748 |
| /root/vrf_voter_window_authority | `01a12d7e-0484-7663-a4fb-5bd6b328d677` | gpt-6-astra | 40 | 2,808,761 | 2,720,256 | 20,238 | 2,828,999 | 115.43015 |
| /root/vrf_voter_window_code_review | `01a12d8b-4ec9-7112-ba61-a4191e8cd14b` | gpt-6.1-sol | 38 | 2,438,419 | 2,349,568 | 12,851 | 2,451,270 | 13.52922 |
| /root/lambda_authority | `01a12da6-5ce0-7b12-b084-334c9956df9f` | gpt-6-astra | 9 | 314,045 | 289,152 | 3,915 | 317,960 | 18.34580 |
| /root/vrf_next_window_authority | `01a12dab-3897-7ea1-9e78-b701b28fc83d` | gpt-6-astra | 51 | 4,665,334 | 4,481,920 | 23,684 | 4,689,018 | 187.50650 |
| /root/vrf_second_window_code_review | `01a13169-f48a-7b21-b733-ada59a8be246` | gpt-6.1-sol | 30 | 1,611,870 | 1,526,656 | 12,496 | 1,624,366 | 11.20134 |
| /root/vrf_third_window_authority | `01a13179-f3b5-76d1-a895-3dc975774d50` | gpt-6-astra | 51 | 4,947,204 | 4,834,560 | 28,626 | 4,975,830 | 184.80750 |
| /root/vrf_third_window_code_review | `01a13181-4fbe-78c2-9449-dea47e264154` | gpt-6.1-sol | 28 | 1,856,924 | 1,733,888 | 14,347 | 1,871,271 | 14.07327 |
| /root/vrf_fourth_window_authority | `01a131a0-17e0-7420-9948-8169d4a605a4` | gpt-6-astra | 64 | 7,020,668 | 6,884,736 | 32,625 | 7,053,293 | 246.88265 |
| /root/vrf_fourth_window_code_review | `01a131a5-2848-7e72-be7d-7a0d374d567b` | gpt-6.1-sol | 42 | 3,590,362 | 3,479,424 | 18,138 | 3,608,500 | 18.77996 |
| /root/vrf_fifth_window_authority | `01a132b1-ec4f-73b3-bde9-721663364f2c` | gpt-6-astra | 70 | 6,613,632 | 6,466,176 | 34,795 | 6,648,427 | 242.01215 |
| /root/vrf_fifth_window_code_review | `01a132b6-c94e-7d20-b071-62c900206e49` | gpt-6.1-sol | 23 | 1,439,751 | 1,343,872 | 11,969 | 1,451,720 | 11.14588 |
| /root/vrf_sixth_window_authority | `01a132bd-cb92-7951-9fb3-e99fd9047f52` | gpt-6-astra | 47 | 4,885,378 | 4,686,208 | 28,748 | 4,914,126 | 202.88270 |
| /root/vrf_sixth_window_code_review | `01a132c4-3ba2-71c1-81d4-450b0011720d` | gpt-6.1-sol | 64 | 6,754,700 | 6,606,592 | 25,424 | 6,780,124 | 30.27788 |
| /root/vrf_seventh_window_authority | `01a132dc-691c-7350-9158-b52bb40eddb6` | gpt-6-astra | 33 | 2,342,990 | 2,255,488 | 17,304 | 2,360,294 | 99.89270 |
| /root/vrf_seventh_window_code_review | `01a132e1-5850-7212-8a36-f312686592c6` | gpt-6.1-sol | 22 | 1,278,636 | 1,199,872 | 10,397 | 1,289,033 | 9.53713 |
| /root/vrf_overlap/luna_startup_pins | `01a13168-53ca-7970-8703-e0982d186581` | gpt-6-luna | 4 | 101,102 | 72,960 | 632 | 101,734 | 0.09650 |
