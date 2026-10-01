# Milestone-review token pilot

Use this pilot for the bounded N4 Rust-native transition slice. It measures the whole review batch without changing the established lead, model routing, or approval gates. Launch Luna first for bounded mapping; keep the Astra lead/contract role, Sol implementation role, and an independent Astra or Sol reviewer as assigned. Use initial contract review only when the owner map or authority is uncertain. Finish each reviewer assignment before starting the next dependent assignment. Freeze source, evidence, and test results before independent final review; if corrections are needed, make discrete batches and review them again.

The weekly allowance reserve remains at least 25%. Start no new slice at 30% or less remaining. Sample available telemetry at slice start, before implementation, after validation, and at closeout. If telemetry is missing, record `unknown`; do not estimate it from history or infer billing/quota from assignment. The prior **110M EVM-token** figure covers the full earlier batch and is not a comparable per-slice baseline.

Record model/reasoning requested and observed separately, plus elapsed wall time, token counts when available, correction/review quality, check results, and any remaining blocker. Report Standard credit estimates as a separate estimate; do not treat them as weekly allowance usage. Keep progress updates short. Send agent messages for contract changes, blockers, or complete handoffs; reviewers return findings and finish rather than staying in status/wait loops. At closeout, record the session ID and prepare the external audit request below. Missing audit results remain `pending`, not a pass.

```text
Audit session <ID> and all descendant agents using local Codex logs. Keep it
read-only and do not resume the session. Deduplicate response usage by
(thread_id, response_id); count compaction once and reconcile cumulative counters.
Report by model, agent, phase, and activity. Separate token counts, Standard credit
estimates with dated rates, and allowance percentages. Compare the pilot's elapsed
time, review corrections, and accepted scope; identify missing evidence.
```

| Measure | Start | Contract review | Implementation | Validation | Final review / closeout |
| --- | --- | --- | --- | --- | --- |
| Weekly used / remaining sample + time |  |  |  |  |  |
| Model and reasoning requested / observed |  |  |  |  |  |
| Session / descendant IDs |  |  |  |  |  |
| Wall time |  |  |  |  |  |
| Tokens in / out / total (source or unknown) |  |  |  |  |  |
| Standard credit estimate (separate) |  |  |  |  |  |
| Corrections, review findings, tests / result |  |  |  |  |  |

Closeout review: compare work quality and corrections with the time/token record; identify missing telemetry and tool-routing failures; note whether the slice met its bounded objective. Keep observations descriptive. One slice does not establish model equivalence, future availability, account quota, or billing.

## October 1 bounded slice measurement

The [closeout](n4_rust_native_transition_blocked.md) records exact thread IDs,
requested/observed runtime, cumulative source counters, checks, corrections and
the prepared external audit request. Rust transition execution is blocked;
four real helper rejection/classification tests pass. Independent Astra medium
review found no blocking findings. No correction batch followed final review.

| Phase | Weekly used / remaining sample (UTC) | Model / reasoning requested and observed | Result / wall time |
| --- | --- | --- | --- |
| Start / mapping | 67% / 33%, 03:37 | Luna medium, same observed | Map complete; thread 03:38:31–03:40:05 |
| Contract | 68% / 32%, 03:40 | Astra high, same observed | Blocked contract; thread 03:38:51–03:40:57 |
| Implementation | 68% / 32%, 03:40 | Sol medium, same observed | One import correction; thread 03:40:39–03:43:52 |
| Validation | 68% / 32%, 03:43:47 | Root observed Sol medium; historical policy named Astra medium | Focused checks passed; full DB test gate outside scope |
| Final review / closeout | 68% / 32%, 03:46:24 | Astra medium, same observed | No blocking findings; total elapsed to 03:46:44 capture: 9m 41s |

Root runtime differs from the historical lead policy; no model switch or silent
substitution is claimed. Deduplicated token totals by phase, Standard credit
estimates and full-session totals are `unknown`; the external log audit is
`pending`. Raw cumulative counters and first/last samples are saved under
`/home/fry/artifacts/n4-transition-2026-10-01/`. These measurements do not prove
model equivalence, future access, billing or milestone acceptance.
