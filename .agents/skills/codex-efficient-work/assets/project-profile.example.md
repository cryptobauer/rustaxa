# Project profile for Codex Efficient Work

Copy this template into the destination project only when preparing its workflow.
Fill it from current project rules and user authorization. This is configuration
for the agent, not an executable policy or a grant of permissions.

| Setting | Project value |
| --- | --- |
| Current objective and accepted scope | Link the active task or contract. |
| Acceptance gap and consumer | Name the dependency this task closes, who uses its result, and the stop/report condition if inputs are unavailable. |
| Checkpoint and ready queue | Give paths and current base/reference. |
| Ownership and exclusions | Name shared files, writer boundaries and excluded work. |
| Required focused checks | List exact commands for the affected modules. |
| Broader validation or approval gates | Preserve the project's existing requirements. |
| Implementation / helper / reviewer routes | Set supported model and effort only where required. |
| Delegation and capacity | Record whether delegation is authorized and available slots. |
| Review capacity and handoff | Record active reviewers, observed open slots, who owns final review, and the handoff if capacity is full. Completed threads do not prove a slot is free. |
| Commit / push / external actions | State existing user authorization; do not infer it. |
| Persistent artifact directory | Use storage that survives environment replacement. |
| Optional account budget | Record the explicitly authorized unit and amount, such as 10 percentage points. A new session or budget needs explicit new-round authorization. |
| Run budget file | Choose one persistent file for the lead session; reuse the same lead identity and file on resume. |
| Optional project resource policy | Link an existing required ledger or limits. Leave unset otherwise; this skill adds no resource caps, grants or per-command approval loop. |
| Audit log location | Current installation's session directory; never guess the current thread. |
| Cost estimation | Dated rate source, units and model mapping; leave unknown rates unpriced. |
| Evidence scope | Name exact source revisions, fixtures/artifacts, test cases, expected observations and exclusions. |

Suggested handoff: objective; base and reference; owned paths; dependencies;
invariants; exact evidence scope; required checks; exclusions; expected result;
acceptance gap and downstream consumer; actual routing; active reviewers,
observed capacity and final-review handoff.

Suggested measurement row: accepted scope and behavior; session tree; actual models;
unique input/cached/output/reasoning totals; mean/median input tokens per response;
estimate and rate date; elapsed time; correction causes and outcomes; required
checks/review; process/cell waits separate from agent waits; start/end allowance.

A quota budget is optional. Without an explicit budget request, do not add a
quota stop or block otherwise authorized work because telemetry is unavailable.
Keep project-specific models, runtime limits, resource units and capture rights in
this profile or linked contracts. Do not copy them into the transferable skill.
