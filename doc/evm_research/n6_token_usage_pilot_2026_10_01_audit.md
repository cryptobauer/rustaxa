# Review pilot audit — 2026-10-01

Session: `01a0f589-7bcf-7d70-8c7a-81252386af90`, plus four direct descendants.
The external audit read local Codex logs. It did not resume the session or contact
its agents. The work ended at `b279b0943bbe5c47942800bcb35b88005ecd0a22`.

## Recorded usage

| Agent / phase | Observed model / reasoning | Tokens | Estimated Standard credits |
| --- | --- | ---: | ---: |
| Lead, integration and closeout | Sol medium | 3,074,227 | 15.87951 |
| Owner mapping | Luna medium | 619,106 | 0.35232 |
| Contract review | Astra high | 660,513 | 35.48665 |
| Implementation | Sol medium | 613,657 | 4.77169 |
| Independent final review | Astra medium | 371,484 | 23.38880 |
| **Total** | | **5,338,987** | **79.87897** |

Input: **5,312,612**, including **4,976,384 cached**. Output: **26,375**,
including **3,253 reasoning**. Subsets are already included in their totals.
There were 97 unique `(thread_id, response_id)` usage records, no duplicate
response records, two repeated cumulative snapshots, and no compaction.
All five threads reconcile with their final cumulative counters.

The estimate uses the Standard credit rates cited in the [original audit](../../token_usage_audit.md),
observed on October 1. It is not an actual bill or allowance conversion.
The active task took 10m14s; thread creation through final response took 11m13s.
Weekly allowance snapshots changed from 67% used at 03:38:07 UTC to 68% used at
03:48:16 UTC, leaving 32%. These are account-level historical observations.

## Review and limits

Source froze at 03:44:56 UTC. Independent review started at 03:45:09 and ended
at 03:46:24. No final-review correction batch was required. The final reviewer
used nine responses and no waits. The root made two waits. Messages and waits
account for 1,096,278 tokens, or 20.53%, compared with 38.5% in the older batch.
Average input per response was 54,769, compared with 116,714 previously.
The logs show 24 Git subcommands across 15 tool invocations and one local
implementation commit. Git activity alone does not explain the token total:
the recorded input also includes accumulated conversation and tool output.

This was a smaller, blocked task: four helper rejection tests and a concrete
owner-state report. Actual Rust EndBlock and parity did not run. The full workspace
fast gate was excluded by the no-database scope. See the [closeout](n4_rust_native_transition_blocked.md).
The root also ran Sol rather than the planned Astra lead. Both scope and routing
changed, so this is not a controlled cost-per-equivalent-task comparison.

## Next pilot

Keep Sol medium as the prospective lead for the next bounded synthetic slice.
Keep review at defined milestones. Resolve the scope conflict by explicitly
allowing isolated synthetic owner databases and required validation databases
in the next prompt. Protect supplied data and historical snapshot copies.
Establish the complete fixture and common Go/Rust branch before implementation.
Do not present synthetic parity as mainnet replay, adoption, or acceptance.

Source logs are under `/home/fry/.codex/sessions/2026/10/01/`, identified by:
root `01a0f589-7bcf-7d70-8c7a-81252386af90`; Luna
`01a0f58a-d5ee-7761-8dcf-1cef9bc47100`; Astra contract
`01a0f58b-2482-7301-b08e-2d32e227e43d`; Sol implementation
`01a0f58c-c797-74c1-9a5c-40e51557c257`; Astra final review
`01a0f590-e908-74b1-a926-1594fb095fe4`.
