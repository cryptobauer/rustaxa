# Measurement and allowance control

Read only the section needed for the requested task. Use the project's command
wrapper and persistent storage rules for the examples below.

## Allowance budget

Only enable a quota stop when the user requests one. Confirm its unit from the
request. A drop of 10 percentage points from 80% remaining stops at 70%; 10% of
that remaining balance would stop at 72%. State the interpretation when needed.
The default consumption option is 10 points, but pass the user's amount explicitly.

Before preparing a continuation, identify why work stopped. At a quota floor,
checkpoint; a new round needs explicit authority. For unknown telemetry, resolve
the exact freshness, identity or window issue without rebasing. For a project
resource limit, show the missing capacity; more quota does not fund it. For an
ordinary implementation failure, diagnose and continue within existing scope.
When the saved budget still permits work, prefer reusing it over proposing a new
baseline. A user's explicit new-round instruction takes precedence.

Use the bundled `scripts/codex_quota.py` with Python 3. Resolve its path from the
skill directory, not the project's working directory. For example, from this
skill directory, initialize the authorized 10-point budget:

```sh
python3 scripts/codex_quota.py --budget-file /persistent/run/quota-budget.json --init-budget --consume-percent 10
```

At later milestones, including resume:

```sh
python3 scripts/codex_quota.py --budget-file /persistent/run/quota-budget.json
```

Replace the example file path with the project's persistent run path. The reader
uses CODEX_THREAD_ID, or explicit `--session` / `--log` with current lead identity.
It does not select the newest log. Override `--sessions-dir` when the installation
uses another log directory. Never initialize a future run's budget from a planning
session or reset a saved baseline to gain more allowance.
On resume, keep the same lead session identity and saved budget file. A new
session or budget file is a new round, not a continuation; initialize it only
when the user explicitly authorizes that new round and its budget.

Initialization creates a schema-v1 JSON baseline exclusively, after fresh telemetry
validation. It saves the lead session, starting used/remaining percentage, observation
time, reset identity, consumption amount and derived remaining floor. Later checks
must use the same lead session and reset window. Other agents use the lead's fresh
budget decision; a child must not create its own budget or impersonate the lead.

Exit codes: 0 permits new work under the budget; 2 stops new work; 3 means unknown
or invalid telemetry/state. In budget mode, allow one bounded refresh for stale
telemetry, then checkpoint if unresolved. Never delete or rebase a mismatched file.
Reset timestamps may jitter by up to 10 seconds from the immutable saved baseline.
The reader never moves that anchor or pools tolerance across observations. Larger
window changes, expired saved windows and usage rollback still stop work. This
bounded telemetry tolerance does not change the saved allowance or quota floor.

The helper retains a legacy fixed-floor mode for compatibility; use `--budget-file`
for relative budgets and do not combine it with `--min-remaining-percent`.

The adapter supports fresh weekly primary-window telemetry (`window_minutes=10080`)
from local Codex JSONL logs. It rejects unsupported telemetry, changed windows,
identity mismatches and usage rollback. It is not a universal billing API or a
background process interrupter. If a destination uses another schema/window, report
that limitation and adapt/test the reader before relying on it.

The lead checks before new assignments/slices/large gates and at natural milestones
about five minutes apart, combining adjacent fresh checks. At the limit, start no
new task or delegation; complete only the in-flight atomic step and minimal handoff.
Do not commit unvalidated work. If starting allowance is below the requested budget,
exhaustion is the upper bound. Concurrent account activity counts toward the measured
drop, and rounded/delayed snapshots can overshoot an approximate stop.

Validate the bundled helper from the skill directory:

```sh
python3 -m unittest discover -s scripts/tests -p test_codex_quota.py
```

## Session audit

Identify the exact root log and recursively discover descendant thread IDs from
agent activity records. Do not treat every log in the same time range as a child.
Read logs without changing them or contacting their sessions. Keep derived files
outside the log directory. Record missing descendants or unsupported schema.

Inventory both top-level records and nested events before claiming response IDs
are missing. From this skill directory, run:

```sh
python3 scripts/usage_inventory.py --log /persistent/rollout.jsonl
```

Repeat `--log` for explicit discovered logs. The helper reports record coverage,
response-key duplicates/conflicts and cumulative snapshots without printing
conversation bodies or modifying logs. It does not discover descendants, validate
token arithmetic, attribute models or compute totals. It is a schema precheck,
not a completed audit; inspect gaps before choosing a fallback.

Prefer per-response usage records. Deduplicate by `(thread_id, response_id)`;
report conflicting duplicates instead of silently choosing one. Attribute each
record to the latest actual model/effort context in that thread. Reconcile sums
against final cumulative thread and turn counters. Cumulative snapshots are checks,
not additional charges. If response records are missing, report the coverage gap;
any cumulative-delta fallback must handle resets and be labelled separately.
Separate implementation, later audit/preparation and unrelated activity by explicit
time windows. Include the implementation handoff; exclude the current audit's own
usage from the audited task. Keep each response in one phase and report overlapping
work as timeline allocation, not isolated billing. Cross-check earlier audits from
raw records and preserve the original report alongside any correction.

Input includes cached input; output includes reasoning output. Total is input plus
output, not the sum of all four fields. Price uncached input, cached input and output
once using a dated, sourced rate table. Do not infer billing from assigned models,
credit balances, or allowance percentages. Leave unknown models/rates explicit.

Inspect top-level `compacted` records as well as nested events. Measure input growth
before and after compaction. For coordination estimates, tokens associated with
responses containing messaging/wait calls include useful context and other work;
they are not a direct measure of wasted message text.
Separate `functions.wait` calls that retrieve a running cell/process from
`agents.wait_agent` calls that wait for another agent. Report counts and elapsed
time by tool; do not count cell retrieval as agent-capacity waiting. Report mean
or median input tokens per response separately from total tokens per response.
The former describes typical prompt/context size; the latter is aggregate usage.

Correlate usage with successful completed Git commands, including `git -c ... commit`
and recorded wrappers. Distinguish actual command events from scripts that merely
contain Git text. Allocate each response once by timestamp if using commit intervals,
and retain final handoff usage. Label these timeline allocations: concurrent reviews
and ahead implementation prevent interpreting them as isolated task charges.

Report model/agent totals, work phases, reconciliation, elapsed time, accepted scope,
correction batches and their causes, required validation and review outcomes. State
the accepted behavior and exact evidence scope; do not use commit count alone as the
outcome. Keep allowance in a separate section. Compare like scopes; commits and lines
changed are weak throughput proxies. A smaller bill with missing checks is not an
efficiency improvement.

For capacity, distinguish a list of completed threads from proof that slots are
available. Record the current capacity observation and the result of each spawn
attempt. A failed spawn call is evidence of a rejection; a completed-thread list or
an unattempted delegation is not.

This section is an audit procedure, not a bundled log parser. Use a compatible
existing parser or build a bounded read-only extractor for the observed schema.

## Package provenance

The bundled quota helper and its tests were copied from this repository's tested
`scripts/codex_quota.py` and `scripts/tests/test_codex_quota.py`. The skill copy is
self-contained for transfer. When changing budget behavior, validate both copies
or explicitly record a version divergence; do not assume repository edits update
the skill package automatically. No historical audit logs, rates or project secrets
are bundled.
The separate usage-inventory helper and its tests are self-contained schema checks;
they do not replace the session-audit procedure.
