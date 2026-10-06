# Next round preparation: 10-point quota budget

Prepared after [the October 3 audit](token_usage_audit_01a0ff3b.md).
Branch base: `049ec5e73`. Preparation changes remain uncommitted. No new EVM
implementation or audited-session resume is part of this preparation.

## Budget and next work

Measure fresh weekly allowance at the new session's startup, before delegation,
preparation commits or implementation. Save that observation once in a persistent
session-specific budget file. Stop after **10 percentage points** of allowance
have been consumed. Example: start at 80% remaining, stop at 70%; start at 57%,
stop at 47%. This replaces the fixed 80% reserve. It is not 10% of the remaining
balance and not a token or billing estimate.

The saved baseline must survive resume and retries. Do not overwrite it, pick an
old session observation or increase the budget after a weekly reset. Missing,
malformed, stale, wrong-session or changed-window telemetry stops new work after
one bounded refresh. The allowance is shared: concurrent account use also affects
the observed drop. Rounded telemetry and in-flight closeout mean the limit is
approximate. No separate agent budgets are used.

The next ready work is the settled one-case zero-amount historical simulation,
then estimation and supported direct traces. The
[next-round contract](doc/evm_research/n3_zero_redelegate_next_round.md) links the
accepted source map and historical contract and names owned paths and gates.
Completed partial/new/full adapter/API work and zero runtime/frames are historical.
Real-window qualification and N4–N6 stay open.

## Cost changes

- Keep direct Sol medium implementation and independent Sol for settled derivatives.
- Ask Astra only a named unresolved semantic question. Reuse settled contracts.
- Start fresh bounded reviewer contexts at semantic profile changes, not just
  contract-family changes. Preserve Luna capacity and record capacity blockers.
- Use complete assignments, result/blocker messages and correction handoffs;
  avoid routine polling and duplicated quota narration.
- Keep actual oracle read buffers isolated and require stateful-prefix effect
  witnesses where downstream rollback/failure depends on those effects.
- Preserve all required checks and independent reviews. Record comparable
  derivative scopes, tokens, estimated credits and correction batches separately
  from the account allowance drop.

## Launch

Select **gpt-6.1-sol, medium** in a new session in this repository and send:

> Execute `next_executable_slice_prompt.md`. Measure and save fresh starting
> weekly allowance, then work autonomously until 10 percentage points have been
> consumed. Keep required validation and independent review, make accepted local
> commits, and leave a compact checkpoint. Do not push.

The full [prompt](next_executable_slice_prompt.md) includes the exact quota-reader
command, preparation commit authority, budget resume rules and remaining limits.
Do not initialize the next run's budget in this preparation session.

Previous prompt and checkpoint are preserved byte-for-byte in linked history.
AGENTS.md now points to the active budget policy instead of a stale fixed reserve.

## Validation

All 26 quota tests pass. Coverage includes relative 80→70 and 57→47 budgets,
immutable resume baselines, exclusive initialization, stale/unknown telemetry,
session/window mismatch, same-window usage regression, contradictory values and CLI flag conflicts. Local
Markdown links, byte-exact archives and whitespace checks pass. Final test output
and command/exit metadata are saved under
`/home/fry/artifacts/codex-relative-budget-prep-2026-10-03/`.
No Rust/C++ runtime code changed. The next run's baseline has not been initialized.
