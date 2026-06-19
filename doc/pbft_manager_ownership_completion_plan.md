# PBFT Manager Ownership Completion Plan

This plan breaks the remaining PBFT manager ownership work into reviewable rewrite slices. It complements
`PLAN.md` and `doc/consensus_rewrite_tracker.md`; those files remain the higher-level source of truth for the
overall consensus rewrite.

Use `$implement-rustaxa-consensus-slice` when implementing any slice from this plan. That workflow supplies the
required repository rules, agent roles, slice-selection constraints, implementation rules, validation expectations, and
closeout checks for Rustaxa consensus rewrite work.

## Goal Prompt

Use this prompt to resume the track:

> Continue `doc/pbft_manager_ownership_completion_plan.md` with `$implement-rustaxa-consensus-slice`.

Keep the prompt short. This document supplies the required slice order, acceptance criteria, validation scope, and
closeout expectations.

## Target Boundary

The end state for this track is a Rust-owned PBFT manager protocol runtime behind the existing C++ public API. Rust owns
period/round/step state, manager transitions, proposal and vote decisions, sync admission, finalization planning,
storage intents, dynamic-lambda decisions, and ordered effect contracts. The Rust-mode C++ overlay remains only a
compatibility shell and effect executor.

The following stay outside this completion track unless a later task explicitly expands scope:

- network/tarcap transport, packet wrapping, peer queues, gossip fanout, disconnect/report execution, and send policy
- EVM and FinalChain execution, including transaction execution, receipt/log bloom construction, gas/state execution,
  and external contract execution
- daemon thread lifecycle, sleeps, timers, startup/shutdown wiring, and event emission mechanics
- temporary C++ materialization of `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, and pillar sidecars
  while public APIs still require those types

Logging is not a boundary. C++ may log Rust-returned statuses temporarily, but deterministic decisions should move to
Rust.

## Current Starting Point

The current Rust path already includes:

- storage-backed PBFT manager startup restore and scalar runtime snapshots
- cursor-managed daemon-tick sessions
- active state-action planning for proposal, filter, certify, first finish, and finish polling
- manager transition planning and transition storage commits
- proposed-block admission and leader-candidate ranking planners
- staged proposed-block validation checks for proposal and sync paths
- sync-period admission planning and transaction-finalization query planning
- finalization planning, staged finalization storage writes, dynamic-lambda calculation, and bounded duplicate/restart
  resume classification

The largest remaining C++ ownership is in live fact collection, side-effect execution, compatibility object resolution,
and duplicated glue around proposal, sync, finalization, broadcasting, and period advancement.

## Completion Slices

### Slice 1: PBFT Manager Effect Contract Inventory

Create an explicit effect catalog for `PbftManager` in Rust and map every Rust-mode C++ live action to one effect ID.
This is a documentation and type-shape slice before deeper routing changes.

Scope:

- Add or extend Rust enums/DTOs for PBFT manager effects: block lookup, proposed-block validation, vote generation,
  vote placement, vote gossip, reward-vote gossip, sync queue mutation, FinalChain wait, finalization dispatch, period
  advance, DAG cleanup, transaction cleanup, pillar finalization, peer report, and sleep.
- Add a table in this document or the tracker mapping current shim functions to Rust effects.
- Keep execution in C++, but remove ambiguous "call helper directly" paths from new Rust-mode code.

Acceptance:

- Every Rust-mode branch in `run()`, state actions, `processPeriodData()`, and `pushPbftBlock_()` has a named Rust effect
  or an explicitly out-of-scope executor reason.
- No new direct C++ decision logic is introduced while cataloging.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`
- `git diff --check`

Inventory status: `rustaxa_consensus::pbft_manager::PbftManagerEffectKind` is the first stable Rust effect catalog for
this slice. It intentionally catalogs executor boundaries without changing production routing yet.

Current Rust-mode shim mapping:

| Current shim action or helper | Effect kind | Executor boundary |
| --- | --- | --- |
| `pushSyncedPbftBlocksIntoChain()` | `ProcessSyncedPbftBlocks` | Rust owns the queue-drain sequence; C++ executes live queue payload and push/finalization effects. |
| `broadcastVotes()` | `BroadcastVotes` | Network send policy, packet wrapping, and gossip execution stay external. |
| `tryPushCertVotesBlock()` | `TryPushCertVotesBlock` | C++ resolves live cert-vote sidecars before finalization routing. |
| `vote_mgr_->determineNewRound()` | `DetermineNewRound` | VoteManager fact sourcing remains an executor call until next-round facts are Rust-owned. |
| `resetPbftConsensus()`, `setFilterState_()`, `setCertifyState_()`, `setFinishState_()`, `setFinishPollingState_()`, `loopBackFinishState_()` | `ApplyManagerTransition` | Rust owns transition planning and storage; C++ applies compatibility mirrors. |
| ineligible-wallet sleep and `sleep_()` | `SleepIneligiblePollingInterval`, `SleepUntilNextStep` | Node scheduling stays outside the PBFT manager breakthrough boundary. |
| `proposePbftBlock()` / `generatePbftBlock()` | `ConstructProposal` | C++ still materializes `PbftBlock` and wallet sidecars. |
| `getValidPbftProposedBlock()`, `validatePbftBlock()`, `validateFinalChainHash()` | `ValidateProposedBlock` | Rust plans validation order; C++ still answers live fact requests. |
| `identifyLeaderBlock()` | `ResolveLeaderBlock` | Rust owns ranking; C++ still resolves selected live block/vote objects. |
| `genAndPlaceVote()` / `placeStateActionVote()` | `GenerateVote`, `PlaceVote` | Rust owns vote bytes and admission; C++ keeps temporary sidecars. |
| `gossipVote()` / `gossipNewOwnVote()` / `gossipNewOwnVotesBundle()` | `GossipVote` | Transport and tarcap egress stay external. |
| FinalChain hash lookup and wait paths | `FinalChainFactOrWait` | FinalChain/EVM execution remains outside this track. |
| DAG order, sortition, anchor cache, and finalized-order mutation paths | `DagFactOrMutation` | DAG runtime owns migrated facts; C++ executes remaining live mutations. |
| transaction finalized-status and non-finalized lookup paths | `TransactionFactOrMutation` | Transaction runtime owns migrated facts; C++ executes remaining live shell calls. |
| pillar validation/finalization/post-processing paths | `PillarFactOrMutation` | Pillar manager orchestration remains a later consensus slice. |
| finalization primary/dynamic/executed storage stages | `ApplyFinalizationStorage` | Storage writes are Rust-owned and must commit before live mirrors change. |
| `finalize_()` / `final_chain_->finalize()` | `FinalizeFinalChain` | EVM/FinalChain execution remains outside the PBFT manager runtime. |
| dynamic-lambda live-field update | `ApplyDynamicLambda` | Rust owns calculation; C++ mirror assignment remains temporary. |
| `pbft_chain_->updatePbftChainForPbftFinalization()` | `UpdatePbftChain` | Rust validates post-state proof; C++ still updates live compatibility state. |
| `advancePeriod()` and startup replay mirror updates | `AdvancePeriod` | Rust should become the period-advance authority in a later slice. |
| sync peer report/clear outcomes | `ReportPeer`, `ClearCompatibilityCache` | Network reporting and compatibility cache mutation remain executor actions. |

### Slice 2: Unified Proposed-Block Validation Executor

Replace the remaining duplicated proposal/sync validation glue with one Rust-driven validation session. Rust should own
the full staged validation order and return typed fact requests; C++ should only answer requests from existing live
subsystems.

Scope:

- Fold `validateFinalChainHash`, DAG-order/gas fact handling, reward-vote, cert-vote, transaction, extra-data,
  pillar-block, and pillar-vote checks into one Rust session API.
- Reuse existing Rust FinalChain fact bundles where available instead of re-querying through C++ helpers.
- Return a single accepted/rejected/wait/report plan with stable reason codes.
- Keep C++ object lookup and expensive live checks as executor calls until those subsystem APIs move.

Acceptance:

- Proposal validation and sync-period validation consume the same Rust validation session.
- C++ no longer owns validation ordering or terminal status selection for PBFT block validation.
- Rejections and wait-for-finalization outcomes are Rust status codes.

Validation:

- Rust unit tests for all validation order branches and terminal statuses.
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`
- `cmake --build /build --target pbft_manager_test --parallel 12`

Implementation status: proposal validation and sync-period PBFT block validation now use the shared
`PbftManagerBlockValidationSession` API. Rust owns the validation cursor, pending check, fact mutation, wait/retry reset,
and terminal status selection. C++ still executes the requested live checks for PBFT-chain, FinalChain, reward votes,
extra data, pillar block, DAG order, and DAG weight, then reports those results back to the Rust session.

### Slice 3: Proposal Construction Planner

Move `proposePbftBlock()` and `generatePbftBlock()` decisioning into Rust while keeping C++ as the executor for wallet
sidecars, live DAG block materialization, key-manager signing, and network effects.

Scope:

- Rust chooses eligible proposer candidates from supplied DPoS/key facts.
- Rust chooses the DAG anchor/null-anchor rule, order hash inputs, previous PBFT hash, FinalChain hash requirement, and
  extra-data requirement.
- Rust returns a proposal construction command containing canonical facts needed to build or skip a PBFT block.
- C++ materializes the `PbftBlock`, calls Rust-backed vote generation, persists/adopts the accepted proposed block, and
  executes gossip.

Acceptance:

- C++ no longer decides eligible-wallet filtering, null-anchor fallback, or proposal skip reasons.
- Proposal status is a Rust code: no eligible wallet, missing FinalChain fact, no valid anchor/order, generated, or
  skipped.
- Existing vote generation stays Rust-backed and is not reimplemented in C++.

Validation:

- Rust unit tests for null-anchor, no-new-DAG, missing order, ineligible wallet, and multiple-wallet ranking cases.
- `cmake --build /build --target pbft_manager_test --parallel 12`
- Targeted proposal-related `pbft_manager_test` cases.

Implementation status: complete for the current ownership boundary. `PbftManagerProposalSession` now owns proposal
candidate filtering from supplied DPoS/sortition facts, FinalChain and extra-data skip status, null-anchor selection,
DAG anchor selection, gas clipping, closest-anchor recompute requests, and canonical order-hash calculation. The
Rust-mode PBFT manager overlay creates the proposal session, answers only requested DAG-order/gas facts, and materializes
the returned command into temporary `PbftBlock`/`PbftVote` sidecars through the existing Rust-backed vote-generation
path. C++ still executes live wallet sortition checks, FinalChain fact collection, extra-data materialization, DAG block
lookup, candidate leader sidecar adoption, and network effects as executor boundaries.

### Slice 4: State-Action Effect Executor Unification

Collapse `proposeBlock_`, `identifyBlock_`, `certifyBlock_`, `firstFinish_`, and `secondFinish_` into one C++ executor
loop over Rust-planned state-action effects.

Scope:

- Extend the existing Rust state-action planner so each branch returns ordered effects instead of one primary intent plus
  branch-specific C++ glue.
- Represent generated-vote, place-vote, persist-status, gossip, transition, no-op, and loopback actions uniformly.
- Route next-voted status persistence, own-vote cleanup, and broadcast-counter mutations through Rust transition/effect
  reports before live C++ mirrors update.

Acceptance:

- The five PBFT active-state methods become thin wrappers or are removed from the Rust-mode path.
- C++ only resolves block/vote sidecars and executes requested effects in order.
- Rust receives execution reports before advancing to follow-up effects or transitions.

Validation:

- Rust unit tests for each active state and report-driven branch.
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`
- `cmake --build /build --target pbft_manager_test --parallel 12`

Implementation status: complete for the current ownership boundary. The active-state planner now has an ordered
`PbftManagerStateActionEffectSession` surface. The Rust-mode PBFT manager overlay routes `proposeBlock_`,
`identifyBlock_`, `certifyBlock_`, `firstFinish_`, and `secondFinish_` through one shared C++ session executor that
requests one Rust-planned effect at a time, executes the live side effect, reports the result, and advances only after
Rust accepts the report. C++ still performs live block/vote sidecar resolution, vote generation, storage mutation, and
gossip as executor boundaries. Broader compatibility mirror reduction remains a later slice.

### Slice 5: Broadcast and Rebroadcast Planning

Move `broadcastVotes()` decisioning into Rust while preserving network transport and packet wrapping in C++.

Scope:

- Rust tracks or derives period/round elapsed-time thresholds, broadcast counters, and rebroadcast counters.
- Rust returns egress intents for reward votes, own PBFT votes, pillar votes, soft 2t+1 bundles, and next-vote bundles.
- C++ resolves retained vote payloads or temporary sidecars and calls existing network methods.

Acceptance:

- C++ no longer compares lambda intervals or decides which vote family to broadcast.
- Broadcast counter updates happen only after Rust accepts executor reports.
- Packet wrapping, chunking, peer-known filtering, and send policy stay outside this slice.

Validation:

- Rust unit tests for first broadcast, rebroadcast, stuck-period, stuck-round, and no-op timing cases.
- `cmake --build /build --target vote_test --parallel 12`
- `cmake --build /build --target pbft_manager_test --parallel 12`

Implementation status: complete for the current ownership boundary. Rust now owns `broadcastVotes()` timing and counter
decisioning through `PbftManagerBroadcastFact`, `plan_pbft_manager_broadcast`, and
`report_pbft_manager_broadcast`. The planner selects no-op, period-vote broadcast/rebroadcast, or round-vote
broadcast/rebroadcast from elapsed-time, lambda, threshold, and counter facts, rejects invalid facts and counter
overflow explicitly, and only returns counter updates after C++ reports successful executor completion. The Rust-mode
PBFT manager overlay still executes packet wrapping, peer filtering, retained vote/sidecar resolution, and network send
policy as external effects.

### Slice 6: Sync Queue and Period-Data Intake Session

Convert `processPeriodData()` and `pushSyncedPbftBlocksIntoChain()` into a Rust-owned session over the existing
`PeriodDataQueue` runtime.

Scope:

- Rust owns queue pop/clean/clear decisions, sync stop conditions, malicious-peer report intent, wait intent, and
  accepted period-data admission status.
- Rust requests C++ only for live object facts that still cannot be represented by canonical bytes or existing Rust
  sidecars.
- Rust emits a final push/finalize command for accepted period data.

Acceptance:

- C++ does not choose clear-queue, peer-report, wait, accept, or drop terminal outcomes.
- Missing/not-checked facts remain explicit Rust session states, not implicit acceptance.
- Temporary `PeriodData` materialization remains only an executor compatibility step.

Validation:

- Rust unit tests for stale period, previous-hash mismatch, wait-for-FinalChain, invalid reward/cert/pillar votes,
  transaction warning, malicious-peer report, and accepted paths.
- `cmake --build /build --target pbft_manager_test --parallel 12`
- `cmake --build /build --target period_data_queue_shim_test --parallel 12` when available.

Implementation status: complete for the current ownership boundary. The existing Rust staged period-data runtime owns
candidate admission outcomes for stale period, previous-hash mismatch, FinalChain wait, invalid reward/cert/pillar
facts, transaction warnings, malicious-peer reporting, and accepted period data. `PbftSyncQueueDrainSession` now also
owns the outer `pushSyncedPbftBlocksIntoChain()` drain sequence: clean old queue data, pop/process a candidate, push
accepted period data, update sync state, continue after drops, stop on empty queue, and stop after push failure. C++ still
executes live `PeriodData` materialization, retained cert/reward/pillar vote sidecars, FinalChain waits, peer reporting,
network sync-state publication, and `pushPbftBlock_()` as executor boundaries.

### Slice 7: Finalization Live-Effect Runtime

Move the remaining `pushPbftBlock_()` live-effect choreography behind a Rust finalization runtime session. C++ should
only execute requested effects and return post-state proofs.

Scope:

- Rust emits ordered effects for pillar finalization, primary storage write, reward reset, DAG finalized-order mutation,
  transaction finalized-status cleanup, sortition commit, PBFT-chain head update, FinalChain dispatch, executed-status
  write, period advance, and pillar post-processing.
- Rust validates post-state proofs for every live mutation before the cursor advances.
- Extend bounded resume so restart-adjacent replay can cover durable Rust-owned stages with explicit no-replay reasons
  for any side effect that still lacks a proof contract.

Acceptance:

- C++ no longer chooses finalization action order or terminal resume classification.
- All storage writes stay Rust-owned and committed before C++ live mirrors mutate.
- Ambiguous replay gaps are explicit Rust no-replay statuses with tests.

Validation:

- Rust unit tests for full finalization, duplicate complete, replay-needed, missing-primary, conflicting-primary, and
  failed post-state proof cases.
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize pbft_manager`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`
- `cmake --build /build --target pbft_manager_test --parallel 12`

Implementation status: complete for the current ownership boundary. Rust now owns the remaining `pushPbftBlock_()`
pillar ordering through two contracts: a pre-intent pillar preflight planner/report that runs before canonical
period-data RLP is built, and a `ProcessPillarBlock` finalization runtime action emitted after `AdvancePeriod`. The
preflight report validates the PBFT block identity, pillar block identity, executor success, and non-empty accepted
pillar-vote count before C++ attaches pillar votes to `PeriodData`. Normal finalization and FinalChain-tail resume
advance only through Rust-planned runtime actions. If a duplicate block requires pillar post-processing but Rust cannot
prove a safe replay from durable state, resume classification returns an explicit no-replay status instead of treating
the block as complete. C++ still executes FinalChain dispatch, pillar manager calls, period-advance mechanics, live field
mirrors, and compatibility object materialization as executor boundaries.

### Slice 8: Period Advance and Startup Replay Ownership

Move period advancement and startup replay decisions fully into the long-lived Rust manager runtime.

Scope:

- Rust owns advance-period effects: reset consensus cursor, reward-broadcast counters, wallet eligibility refresh
  request, proposed-block cleanup, period transition persistence, and startup replay range selection.
- Rust classifies startup replay periods and returns replay commands using Rust storage facts.
- C++ executes FinalChain waits, wallet eligibility fact collection, and compatibility mirror updates only when Rust asks.

Acceptance:

- `advancePeriod()` and startup replay logic become executor wrappers around Rust commands.
- C++ no longer independently computes replay ranges or period-advance side effects.
- Rust runtime snapshot is the authoritative source for manager scalar state after startup and period advance.

Validation:

- Rust storage-backed tests for startup replay range, missing period data, Cacti lambda lookup, and normalized cursor
  persistence.
- `cmake --build /build --target pbft_manager_test --parallel 12`
- Rust-enabled startup smoke if available through the current validation target.

Implementation status: complete for the current ownership boundary. Rust now owns startup replay range selection through
`PbftManagerStartupReplayRangeFact` / `PbftManagerStartupReplayRangePlan`, including empty-bootstrap handling,
FinalChain-ahead corruption rejection, FinalChain replay ranges, and recently-finalized transaction hydration ranges.
`advancePeriod()` now asks Rust for an ordered `PbftManagerAdvancePeriodPlan`: reset-consensus transition,
post-FinalChain executed-block reset, VoteManager period/round update, timer resets, reward-broadcast counter reset,
wallet eligibility refresh, vote cleanup, and proposed-block cleanup. The long-lived Rust manager runtime commits the
new period only after the C++ executor completes the ordered effects and rejects non-increasing period commits. C++ still
executes FinalChain waits, wallet eligibility refresh, VoteManager side effects, timers, proposed-block/vote cleanup,
startup `PeriodData` materialization, and recently-finalized transaction sidecar hydration as compatibility executor
boundaries.

### Slice 9: Compatibility Mirror Reduction

Remove or quarantine C++ mirrors that are no longer authoritative after the prior slices.

Scope:

- Audit `round_`, `step_`, `state_`, lambda fields, next-voted status mirrors, broadcast counters, own-vote caches,
  cert-voted block caches, proposed-block sidecars, and queue mirrors.
- Replace read paths with Rust runtime snapshots or Rust-owned views where public APIs allow it.
- Keep unavoidable mirrors documented as compatibility caches and update them only from Rust execution reports.

Acceptance:

- No Rust-mode production decision reads a C++ mirror as authoritative when the Rust runtime has the state.
- Remaining mirrors are either public API materialization caches or executor-local temporary state.
- Obsolete bridge helpers and shim helpers are removed in the same slice.

Validation:

- Targeted searches for removed helpers and authoritative mirror reads.
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge`
- `cmake --build /build --target pbft_manager_test --parallel 12`

Implementation status: in progress. The first compatibility mirror reduction cut makes the long-lived Rust manager
runtime authoritative for round, step, state, current-lambda, next-step-time, and executed-block scalar inputs after
startup. `getPbftRound()` and `getPbftStep()` now read `PbftManagerRuntimeSnapshot` whenever the runtime handle exists,
the daemon tick fact is seeded from a fresh Rust runtime snapshot instead of C++ scalar mirrors, and transition-planner
facts for delay/reset/filter/certify/finish/finish-polling/loopback use snapshot scalar values. State-action facts and
the runtime action mismatch guard also read `state_` only through a fresh Rust runtime snapshot; the C++ `state_` field is
now a compatibility mirror updated by Rust transition/snapshot application helpers rather than a production decision
source. PBFT period remains PBFT-chain-derived for now because normal finalization advances the PBFT chain before Rust
commits the new runtime period. Dynamic-lambda compatibility mutation has been narrowed: the obsolete shim-local
`adjustDynamicLambda()` helper was removed, and the Rust-mode finalization path now builds the dynamic-lambda storage
stage from the Rust planner output before updating C++ lambda mirrors only after Rust storage accepts the stage. The
dynamic-lambda read cut adds a Rust runtime commit point for accepted dynamic-lambda stages, makes `getRoundLambda()` read
round-one lambda from `PbftManagerRuntimeSnapshot`, and feeds finalization dynamic-lambda planner inputs from the same
snapshot instead of `rounds_count_dynamic_lambda_` / `dynamic_lambda_`. The next-voted flag cut makes successful
next-vote status persistence return an authoritative Rust runtime snapshot, applies that snapshot to compatibility
mirrors, removes the direct C++ bool writes, and feeds state-action planner facts from snapshot flags. The
broadcast-counter cut makes `PbftManagerRuntimeSnapshot` own the four live broadcast/rebroadcast counters,
seeds accepted startup snapshots with one-based counters, resets round counters from committed reset-consensus
transitions, routes reward-counter reset and force-broadcast through a Rust runtime counter update, builds
`broadcastVotes()` facts from a fresh runtime snapshot, and applies accepted broadcast reports by hydrating C++
compatibility mirrors from Rust. The cert-voted metadata cut makes `PbftManagerRuntimeSnapshot` own whether a
cert-voted block exists plus its period, round, and hash; startup restore records metadata after loading the Rust-owned
recovery row, successful cert-vote storage writes update runtime metadata before the C++ sidecar changes, transition
reset clears metadata in Rust, and transition/state-action planner facts read the runtime metadata instead of the C++
optional. The C++ `cert_voted_block_for_round_` object remains a temporary materialization sidecar for vote placement
and proposed-block APIs. The DAG-order cache-membership cut makes the Rust runtime own the compact set of anchor hashes
with materialized DAG-order data; proposal and sync validation facts now read `dag_order_cached` from that runtime
metadata while C++ keeps the temporary `DagBlock` vector cache only as a FinalChain/finalization materialization sidecar.
The sync queue tail-metadata cut makes the Rust-backed `PeriodDataQueue` own the queued PBFT block hash alongside entry
id, period, processable size, pop planning, and cleanup metadata. `lastPbftBlockHashFromQueueOrChain()` now reads that
compact Rust queue fact instead of materializing the last queued `PeriodData.pbft_blk` only to read its hash. C++ still
owns queued `PeriodData`, `PbftVote`, and peer `NodeID` payloads for processing and public compatibility. The derived
syncing-period cut moves the queue-aware network status period calculation into the Rust-backed `PeriodDataQueue`; C++
now supplies only the PBFT-chain size executor fact. The queue hash fallback cut also moves the
queued-block-hash-versus-chain-hash decision into Rust queue metadata: C++ supplies the chain-derived PBFT period and
chain last-hash executor facts, and Rust returns the hash to use without the PBFT manager reading queue period metadata
directly. The proposed-block metadata cut makes the Rust-backed `ProposedBlocks` index retain compact pivot-hash and
cached-validity facts. PBFT leader-candidate ranking now reads those facts without reconstructing `PbftBlock` sidecars
for already-valid candidates, materializing the selected block only when an executor or public API still needs the C++
object. The sync pop metadata cut makes the Rust-backed `PeriodDataQueue` return popped block period/hash/previous-hash
and pivot-hash metadata with the live compatibility payload. `processPeriodData()` now builds PBFT sync admission and
block-validation facts from that Rust pop metadata instead of reopening the popped `PeriodData.pbft_blk` sidecar for
chain-link facts. The sync transaction-hash metadata cut makes queued Rust metadata retain transaction hashes referenced
by finalized DAG blocks and hashes supplied in the period-data transaction list; the pop metadata carries those compact
lists into `processPeriodData()`, which now builds PBFT sync transaction-query facts from Rust-owned queue metadata
instead of scanning the popped `PeriodData` DAG blocks and transaction sidecars for those hashes on every runtime plan.
The sync previous-cert metadata cut also makes the Rust queue retain whether the popped payload carried previous-block
cert votes and whether the first cert vote already had weight; `processPeriodData()` feeds those compact flags into the
Rust admission planner without reading `PeriodData.previous_block_cert_votes` only to decide reward-vote replacement.
The sync pillar-presence metadata cut makes the Rust queue retain whether the popped payload carried pillar-vote
sidecar data; `processPeriodData()` now uses that compact flag for required/not-required pillar-data admission status
instead of reading `PeriodData.pillar_votes_` solely for presence. Full pillar-vote validation and materialization remain
temporary C++ compatibility payload work. The sync block-fact metadata cut makes the Rust queue retain the popped PBFT
block's final-chain hash and extra-data/pillar-block-hash presence. `processPeriodData()` now validates FinalChain hash
and PBFT extra-data status from those Rust-owned compact facts instead of reopening `PeriodData.pbft_blk` for those
fields; reward-vote, cert-vote, actual transaction, pillar-vote, and finalization payloads remain live compatibility
sidecars. The sync transaction-identity cut makes the Rust queue retain period-data transaction hash/sender/nonce facts
derived from Rust legacy-transaction inspection when the payload is queued. `processPeriodData()` now feeds those
pre-inspected facts to the Rust-backed TransactionManager finalized-status checker instead of reopening
`PeriodData.transactions` only to derive finalized-warning inputs; actual transaction objects remain live compatibility
payloads for finalization execution. The proposed-block mark-valid command cut adds a compact period/hash mutation
surface to the Rust-backed `ProposedBlocks` shim and makes PBFT manager admission/leader-selection mark-valid commands
use Rust-returned identities instead of reusing materialized C++ `PbftBlock` objects as the mutation authority. The
sync invalid-state-root cleanup uses the popped Rust queue final-chain-hash fact for sync rejection logging instead of
reopening the live PBFT block sidecar after Rust queue metadata already supplied that block fact. The sync cert-vote
block-identity cut makes cert-vote validation consume the Rust queue's popped PBFT period/hash facts directly, so the
helper no longer reopens the live PBFT block sidecar solely to compare vote period/hash, choose strict-validation
intervals, or log the block identity; live `PbftVote` validation, weight accumulation, and verified-vote insertion
remain compatibility payload work. The sync reward-vote hash cut makes the Rust-backed `PeriodDataQueue` retain the
popped PBFT block's requested reward-vote hashes. `processPeriodData()` now validates reward votes through those compact
queue facts plus VoteManager's Rust verified-vote runtime instead of reopening `PeriodData.pbft_blk` solely to read the
requested hashes; copied selected `PbftVote` objects remain temporary compatibility payloads for previous-cert
replacement.
Remaining Slice 9 work: proposed-block sidecars that still require validation/API materialization and sync payload
materialization for actual transaction objects during finalization execution, votes, and pillar data.

### Slice 10: Rust-Mode PBFT Manager Parity Gate

Add a focused parity and smoke gate for the Rust-owned PBFT manager runtime before considering this track complete.

Scope:

- Add C++ vs Rust transcript tests for daemon ticks, state-action scripts, sync admission, finalization sessions, and
  period advancement.
- Add restart/replay fixtures that load from Rust storage and verify runtime snapshots plus compatibility mirrors.
- Add a guard or test assertion that Rust-mode PBFT manager routing does not call `PbftManagerOld` or original
  upstream-owned PBFT manager implementation paths for production decisions.

First guard cut landed: `pbft_manager_test` now includes a Rust-mode compile-time assertion that the shim-owned
`PbftManager` does not inherit from `PbftManagerOld` when the PBFT manager overlay is active. Daemon tick transcript
coverage is present in `rust_consensus_tests`: the PBFT manager runtime session records the value-proposal action order,
restart-on-cert-progress behavior, advance-round reset effect, certify-to-finish transition, and cursor mismatch
rejection.
State-action transcript cut landed: `rust_consensus_tests` now records the CXX bridge transcript for a finish-polling
state-action effect session, proving Rust emits the ordered current-soft-value next vote before the null-block next vote
and accepts one executor report per effect before completing the script.
Sync-admission transcript cut landed: `rust_consensus_tests` now records the staged `processPeriodData()` runtime
checks through final-chain validation, reward votes, cert votes, transaction queries, pillar data, and pillar votes
before Rust returns the accept action.
Finalization-session transcript coverage is present in `rust_consensus_tests`: the finalization runtime session records
the mixed-executor action order from primary storage through advance-period and requires matching executor reports before
completion; the finalization resume runtime records bounded duplicate/restart tail-replay actions, and the storage-backed
resume inspector classifies not-persisted, dynamic-lambda-needed, FinalChain-replay-needed, and complete crash windows.
Period-advance transcript cut landed: `rust_consensus_tests` now records the Rust-planned period-advance effect order
from reset-consensus application through vote/proposed-block cleanup. Startup snapshot coverage landed:
`rust_consensus_tests` now seeds Rust storage manager fields/statuses, restores the PBFT manager runtime through the CXX
bridge, verifies the Rust-owned runtime snapshot, and requires the normalized startup step to be persisted back to Rust
storage. Slice 10 is complete for the current Rust-owned PBFT manager boundary; remaining PBFT manager ownership work is
tracked under Slice 9 compatibility payload/materialization.

Acceptance:

- PBFT manager deterministic decisions have transcript coverage across proposal, certify, finish, sync, finalization,
  duplicate/restart, and period-advance paths.
- Rust-mode production routing has no silent legacy fallback.
- `libraries/core_libs/consensus/src/pbft/pbft_manager.cpp` stays clean versus `upstream-main`, except for any explicitly
  approved guarded hook.

Validation:

- `make rewrite-validate-consensus`
- `git diff upstream-main -- libraries/core_libs/consensus/src/pbft/pbft_manager.cpp`
- `git diff --check`

## Suggested Order

The safest order is:

1. effect contract inventory
2. unified proposed-block validation executor
3. state-action effect executor unification
4. proposal construction planner
5. sync queue and period-data intake session
6. finalization live-effect runtime
7. period advance and startup replay ownership
8. broadcast and rebroadcast planning
9. compatibility mirror reduction
10. Rust-mode PBFT manager parity gate

Broadcast planning can move earlier if it blocks state-action cleanup, but it should not pull network transport into the
PBFT manager runtime.

## Per-Slice Closeout Checklist

- Keep original upstream PBFT manager files clean unless an approved guarded hook is documented.
- Keep new Rust/C++ bridge APIs based on canonical bytes, compact facts, stable hashes, and typed effects.
- Do not add C++ decision logic when Rust can own the planner.
- Do not silently forward Rust-mode production behavior to legacy C++.
- Add Rust unit coverage for each moved decision table.
- Run the narrowest C++ target that exercises changed shim behavior.
- Remove obsolete bridge helpers, shim helpers, and compatibility DTOs once a Rust route becomes authoritative.
