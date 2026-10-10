Start a NEW session on feat/rust/evm-state-db using gpt-6.1-sol medium.
Work autonomously on the next N4 historical native-state reconstruction slices
for another TEN PERCENTAGE POINTS of weekly allowance from a fresh startup baseline.
This prompt authorizes the bounded implementation, read-only probes, isolated test
owners, validation, independent review and local commits described below. Do not
start the run while merely preparing this prompt. Do not push.

Read AGENTS.md and /home/fry/.codex/RTK.md before shell commands. Prefix commands
with rtk. Apply codex-efficient-work, doc/codex_slice_workflow.md and the current
model table in doc/evm_research/08_implementation_plan.md. Never ingest AI.md.
Keep worktrees under /home/fry/.codex/worktrees and useful artifacts under
/home/fry/artifacts or the persistent checkout.

Accepted implementation/evidence base: 1838694c6d9089c497a824a82c120044183553ee.
Verify branch, HEAD and status once; reconcile newer preparation-only commits.
Preserve unrelated edits, including existing workflow, queue, old checkpoint,
skill and audit changes. There are no pending owned implementation, gates,
reviews or processes. Read doc/evm_research/n4_next_run_checkpoint.md first.
It supersedes stale P0 next-action text in the older restart and queue records.

Read these current records, then selected source ranges rather than full history:
- doc/evm_research/n4_light_snapshot_availability.md.
- doc/evm_research/n4_v1_14_1_policy_basis.md.
- doc/evm_research/n4_owner_input_restart_contract.md, interpreted with those updates.
- /home/fry/artifacts/light-snapshot-check-20261010/checkpoint.md and review.md.
- /home/fry/artifacts/n4-v1.14.1-policy-20261010/checkpoint.md and review.md.

Before agents, preparation commits, gates or implementation, identify THIS lead
using CODEX_THREAD_ID or an explicit current UUID, never the newest log. Create a
unique RUN_ARTIFACT_DIR under /home/fry/artifacts. Record fresh allowance once:

```sh
rtk proxy python3 .agents/skills/codex-efficient-work/scripts/codex_quota.py --budget-file "${RUN_ARTIFACT_DIR}/quota-budget.json" --init-budget --consume-percent 10
```

Reuse the same budget without --init-budget for every later check and same-session
resume. This new round is explicitly authorized; old budgets remain immutable.
Do not resume the old lead sessions or their agents, transfer their budgets, or
infer current allowance from historical readings. Ten points means a drop from
the startup remaining percentage by 10, not 10% of that remainder. Account-wide
activity counts. Check before assignments/slices/large gates, after closeout and
about every five minutes during long work. Unknown/stale telemetry gets one
bounded refresh; unresolved telemetry, identity or reset-window mismatch stops
new work. At the floor finish only in-flight atomic work and minimal closeout.

Established input basis:
- H=25,706,949; P=25,706,948; session eligibility E=25,706,944;
  certificate eligibility D=25,706,942. Keep E and D distinct.
- All six authenticated live DPoS/slashing inventories at P/E/D succeeded.
  Each DPoS inventory has 23,278 rows; each slashing inventory has 54.
  The respective native tries match across these periods. This does not prove
  all accounts, deleted/all-ever keys, logical key catalogs or complete semantics.
- Existing partial inversion explains 2,065 DPoS rows and leaves 21,213 undecoded;
  those unexplained live bytes are present, not missing. No Rust DPoS sidecars.
  Parent vote/stake raw origins are present. Minted-token tombstone is not zero.
- Retained cache has 49 decoded rows at 25,706,901..25,706,949. Excluding H gives
  the 48-period candidate parent subset under frequency 100; historical runtime
  and generation authority still need reviewed reconstruction.
- Owner directs default mainnet configuration and reports likely v1.14.1.
  Release commit a0e85fe31eb03573cd92c165a5f81035cec9907e pins public EVM
  6c7e5338b22d5e596cc2365a88d1f94840e1ee1b. Seven relevant release files equal
  the checkout; all 12 retained DPoS scalars match. Local Go oracle pin
  bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418 is distinct.
  Use this supplied reference basis without requesting another config upload.
  Actual producer binary/capture remain unverified; retain that limit in evidence,
  but do not use it as a generic blocker to reference reconstruction work.
- P0 ordinary-prefix Rust binding is accepted at c182cdc4e25e218ef24587bde64dd01eb892d4c0.
  Do not redo that slice or broaden prefix admission.

Work in order:
1. Check capacity, then launch and confirm gpt-6-luna medium first with a bounded
   read-only map of the existing native inverse decoder, Go storage layouts and
   Rust snapshot import/session seams. Give exact paths and questions; reuse its
   map. Reserve its slot. Lead implements serial slices directly. Use configurable
   roles, explicit models/reasoning and fork_turns=none; verify runtime when available.
2. Obtain bounded independent gpt-6-astra medium authority review before changing
   native completeness or historical-owner boundaries. Escalate to high only for
   a named difficult invariant. Settle one actionable reconstruction contract:
   logical-key discovery from justified address/index/history sources, raw-byte
   authentication, missing/absent/tombstone rules, ordering, reward graph/history
   authority, cache reconstruction and a safe parent test owner. Separate a narrow
   decoder contract from larger owner design so ready decoding can proceed.
3. Implement the smallest useful decoder/catalog family under that contract using
   existing Rust owners and codec modules. Prefer retained indexes, justified key
   derivation and bounded retained transaction/history evidence. Classify every
   examined row as explained, unresolved or invalid; preserve raw bytes and root/
   period provenance. Partial coverage must not become a complete DposSnapshot.
   Keep delegation principal, iterable ordering, V1/V2 undelegations, graph nodes/
   cursors, slashing records and history markers explicit. No brute-force key search,
   fabricated zeros, policy facts, unknown keys or missing-history assumptions.
4. Use only local/evm-state-db/snapshot-litenode-copy for read-only source probes
   through the existing path and identity guards. Reports must be new files outside
   both trees. The supplied data tree and independent input copy remain immutable.
   Reuse accepted evidence by exact hashes. Do not repeat full inventories merely
   to restate availability. Any new scan has fixed identities, account/column/key
   sources and explicit node/row/byte/operation limits reviewed before execution.
   Existing inventory ceilings are 50,000 nodes, 50,000 leaves and 32 MiB per account;
   reward-cache acceptance ceilings are 128 rows and 1 MiB with overflow detection.
   Stop on exceeded bounds, unavailable history or identity drift. Prepare broad
   or differential campaigns concretely and ask before running those gates.
5. Build an isolated historical test owner only after complete reconstruction and
   its authority contract are reviewed. Keep account/storage/EVM execution in the
   existing owners. Do not rewrite the retained head, write LAST_NUMBER to bypass
   constructor checks, weaken completeness guards or add a production fallback.
   New writable test databases belong under the persistent run artifacts and must
   be initialized through a reviewed path from authenticated inputs. Diagnostic
   coverage improvements may be accepted independently while owner construction
   is still blocked by a precisely named semantic requirement.
6. Execute the real one-period reference only when dependencies are complete:
   the same 19 signed transfers, request-bound native session, reviewed parent cache,
   derived system schedule and one finish_rewards, with unpublished output.
   Derive rewards, effects and final root independently; compare them with stored
   receipts/results. Never inject the expected root or treat transaction-only root
   equality, empty-distribution c0 or synthetic success as full transition proof.
   Retain the owner-supplied policy assumption in every real-window acceptance claim.
7. Continue ready N4 slices within the budget. Do not stop after one useful commit
   if another reviewed bounded slice is ready. Do not add optional synthetic work
   merely to consume quota, or request a full snapshot without identifying specific
   unavailable records. If all useful work is blocked, record the exact missing
   source/authority and the attempted bounds, not a generic database/consensus gap.

Validation and acceptance:
- Follow doc/rewrite_validation_strategy.md. For each behavior change, run focused
  malformed/missing/tombstone/order/history tests and the smallest compile checks
  before broader required gates. Use C++/Go versus Rust parity for routed behavior.
  Preserve existing tests and old profiles; do not weaken assertions to pass.
- Run affected package fmt/check/clippy/tests and required fast/bridge/smoke checks
  when their scope applies. CMake builds use 12 jobs. Any storage-module change also
  builds and runs rust_storage_tests. Ask before expensive repo-wide/differential
  campaigns; routine required targeted checks remain authorized. Capture commands,
  full outputs, exits and nonzero matched test counts on the first run.
- Snapshot preservation claims need complete logical-row/column comparisons with
  correct comparators and WAL-visible state, not selected-value checks. Use the
  accepted shared owner snapshot helper for disposable owner fault/retry tests.
- Freeze source and evidence after checks, then obtain independent Sol medium
  final review for settled decoder work; Astra reviews new historical authority.
  Never approve own implementation. Fix findings and rerun affected checks before
  local Conventional Commits. Document changed modules/types/functions fully.

Use fresh reviewer contexts at semantic profile changes. Record model requests,
confirmed runs/results and routing failures; do not infer quota from model choices.
If required review capacity cannot start, checkpoint before dependent work and do
other ready independent work. Do not retry unchanged capacity failures or silently
substitute models. Workers must preserve others' edits. No production route,
protocol/fallback change, supplied-data mutation, upstream C++ exception, push or
unapproved broad campaign is authorized. Read rewrite_agent_workflow.md before any
upstream-owned C++ change; favor Rust and existing overlay shims.

At budget, authorized completion or a blocker affecting all ready work, leave a
compact checkpoint with accepted HEAD/commits, owned pending paths/hashes,
checks/reviews/results, remaining invariants, actual routing, source/evidence pins
and exact next action. Keep baseline/floor/final/reset separate from token/billing
claims. Link detailed history. Do not leave an unvalidated slice marked accepted.
