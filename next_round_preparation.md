# Next round preparation

Prepared on 2026-10-03 after the
[multi-slice audit](token_usage_audit_01a0f999.md). Implementation baseline:
`327d15fa1`. These changes prepare a new run; they do not execute the next EVM
chunk or resume an audited session. Preparation changes remain uncommitted.

## Applied changes

- Default quota floor is **80% weekly allowance remaining**, or 20% used.
  The active workflow, prompt and checkpoint agree. The reader accepts a
  validated explicit override, but the next run must keep the authorized 80%
  floor. Missing, stale or wrong-session telemetry cannot permit a start.
- The lead coordinates quota decisions. Adjacent decisions can share a fresh
  observation; workers cannot extend an assignment on stale telemetry.
- Astra review contexts now stay within one contract family. Start a fresh
  bounded reviewer at family changes. Use independent Sol for settled
  derivatives and Astra for uncertain authority, authentication or rollback.
  All required validation and independent review remain in place.
- The startup checkpoint is compact again. Completed evidence and the prior
  prompt are preserved in linked history. AGENTS.md did not need more text.
- The next chunk is [staged redelegation](doc/evm_research/n2_redelegate_next_chunk.md),
  followed by ABI/frame and API composition in accepted commits. The settled
  contract, owned paths, invariants, evidence and exit checks are ready.

The review changes address the largest measured cost: Astra was 66.8% of
estimated credits in the last run. They are a measured workflow change, not a
claim that cheaper models have equal quality. See the audit for deduplicated
token totals and pricing assumptions; allowance percentages remain separate.

## Checks and launch

Ten quota tests pass, including the 80% boundary, explicit 20% override,
invalid floors, CLI behavior and existing telemetry protections. Whitespace,
local Markdown links and archived-content preservation are checked. No Rust or
C++ source changed, so no EVM build was needed for this preparation.

Fresh current-session telemetry at `2026-10-03T00:31:43.532Z` reported 100%
remaining and permitted startup. This is an account observation, not a current
guarantee or a task token budget. Read it again at launch.

Select **gpt-6.1-sol, medium**, start a new session in this repo, and send:

> Execute `next_executable_slice_prompt.md`. Continue the prepared redelegation
> chunk autonomously. Stop at about 80% weekly allowance remaining. Keep all
> required validation and independent review, make accepted local commits,
> and leave a compact checkpoint. Do not push.

The full [run prompt](next_executable_slice_prompt.md) authorizes the preparation
commit and accepted implementation commits. No production routing or broad
approval-required gate is authorized. Real-window qualification remains open.
