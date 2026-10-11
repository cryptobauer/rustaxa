Start the next N4 slice on feat/rust/evm-state-db with gpt-6.1-sol medium.
Work autonomously through the bounded offline parent-input handoff described below.
Commit accepted work locally; keep documentation commits separate. Do not push.
This prompt is preparation only until invoked; do not start a new run merely to
prepare or validate it.

Read AGENTS.md and /home/fry/.codex/RTK.md before shell commands; prefix all shell
commands with rtk. Never read, write, search or ingest AI.md. Apply
codex-efficient-work, doc/codex_slice_workflow.md, the applicable rewrite workflow
and doc/rewrite_validation_strategy.md. Start from:
- doc/evm_research/n4_next_run_checkpoint.md (accepted storage closeout)
- doc/evm_research/n4_next_10point_plan.md (current next-slice scope)
- /home/fry/artifacts/n4-binding-resume-20261011/storage-final-closeout-review.md
- /home/fry/artifacts/n4-next-prompt-20261011/next-input-pins.json

Accepted implementation HEAD is8a4a613dbbc4c82dcf5d608efb6f65d9d288f048.
Accepted storage checkpoint commit is9539294ba99f6be60f54095b3b0518eabf32ace1.
A later documentation-only prompt commit may follow it. Inspect actual Git state
and reconcile newer accepted work; do not reset to these references. Preserve
unrelated workflow, N6 queue/checkpoint, .agents/ and token-audit changes.

Quota: the latest user instruction is “continue and ignore budget for now”.
Quota stopping is suspended until the user changes it. Preserve the existing
/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/quota-budget.json
unchanged, SHA256bd5664c3f082e5b961e5fb67deffde261e0bf72ea60112233af44c007fe7695a.
Historical baseline90%, floor80%, last valid83%; fresh resumed telemetry was
unresolved. Do not present83% as current, initialize a budget, rebase the old
file, impersonate its lead or infer billing. Record fresh current-lead telemetry
if available and include account-wide side use; unknown telemetry stays unknown.
Actual account/model unavailability still stops dependent work. No new ten-point
round is authorized by this prompt.

1. Check capacity and confirm Luna-medium first for a bounded input/hash/consumer
   map. Follow the current model table in doc/evm_research/08_implementation_plan.md
   and n6_model_setup_validation.md. Configure models explicitly through a
   configurable role, with independent context for fresh agents. Reserve Luna's
   slot. Sol implements serially; Astra reviews the new authority profile. Record
   requested model/reasoning, successful startup, results and routing failures.
2. Verify the333-file accepted closeout without a database open:

   rtk proxy python3 -E /home/fry/artifacts/n4-binding-resume-20261011/verify-final-closeout.py

   Storage capability, delayed roots and the fixed96 total/jail proof family are
   closed. All48 totals for D25706894..25706941 are534879 and all48 jail
   preconditions passed independent replay. Candidate Aspen policy is accepted
   separately under the default-mainnet assumption. Do not rerun their consumed
   live commands, recapture, change frozen artifacts or inject observed constants.
3. Prepare the exact offline parent-input admission/join contract from the six
   pinned artifacts in the current plan. Keep original parent/schema validators
   unchanged. Bind48 source periods25706901..25706948 to D=s−7 and complete roots.
   Obtain independent Astra READY for the implementation contract before coding.
   The next-slice plan does not by itself supply READY executable authority.
4. Implement the smallest additive offline handoff under snapshot_qualifier
   ownership. Reuse the accepted raw-chain, witness verifier, canonical native
   decoder and Aspen derivation. Reports are provenance/comparison inputs; claimed
   success booleans never replace authentication. Preserve all13 witness ceilings.
   Settle aggregate artifact-read/source-profile/output limits and parser
   allocation exclusions before implementation; count repeated loader reads.
   Use the admitted bundle with the storage verifier; blindly calling the replay
   CLI/helper again would exceed the proposed artifact-read count/bytes.
   No database opens, new physical reads, cursor use or inventory is authorized.
5. Validate and freeze the handoff, independently replay it offline, obtain final
   review, and commit accepted work. Then prepare one exact remaining voter or
   lambda authority contract using the original recovered-voter manifest and
   consumer rules. No new live probe follows automatically. If justified live
   evidence is required, first settle its complete semantic/read contract and
   independent exact-command READY, including fixed inputs/identities/keys,
   byte/operation/output limits, executable/source pins and failure stops.

Remaining inputs:2742 unqualified voter value/proof rows for914 delayed-period/
voter pairs and20 distinct voters;48 lambda selections;48 raw expected cache
rows; producer/configuration authority. Native total/global jail success does
not qualify voter weights or every per-voter jail key. Do not repeat the69
earlier unavailable tuples without changed evidence, expand the manifest, sweep
addresses or keys, or assume missing/tombstone/unavailable means zero.

Run narrow tests first and require nonzero matches. Preserve existing tests and
complete first-failure logs. Run affected package checks/tests, format, production
lint, focused canonical/selection parity, required fast checks and commit hooks.
If storage changes, confirm RUSTAXA_ENABLE=ON, build rust_storage_tests with12 jobs
and run it. Obtain approval before expensive repository-wide/differential
campaigns, after preparing exact commands and data. Freeze sources, inputs,
executables, outputs and gates; independent final review/replay precede acceptance.

Full reward-cache construction, writable owner creation,19-transfer execution
and production adoption require their separate complete authority. Do not route
production, alter supplied trees, weaken completeness checks, introduce C++
ownership or fallback, or treat diagnostics as public finality/producer proof.
Keep H25706949/P25706948/session E25706944/certificate D25706942 distinct.

Continue ready work in this scope. At completion, actual capacity/account limit
or all-ready-work blockage, leave a compact checkpoint: accepted commits and
implementation HEAD; pending paths/hashes; checks, corrections and independent
reviews; actual routing; original/latest quota with the user override; exact
remaining binding, wiring or authority/data blockers; next executable action.
Do not push.
