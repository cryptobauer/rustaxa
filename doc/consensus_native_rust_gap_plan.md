# Consensus Native Rust Gap Plan

This plan tracks the remaining non-network and non-EVM work needed before consensus shims and bridge compatibility code can be folded into native Rust modules. It starts from the completed PBFT manager protocol-runtime closeout in `PLAN.md` and the current status matrix in `doc/consensus_rewrite_tracker.md`.

## Target Boundary

Rust should own consensus rules, durable consensus state, restart normalization, storage/query selection, canonical payload retention, validation decisions, and ordered side-effect planning.

C++ may remain only for these explicit out-of-scope boundaries while this plan runs:

- network/tarcap transport, packet wrapping, gossip fanout, peer marking, report/disconnect execution, and packet queues
- EVM and `StateAPI` execution, state DB mutation, receipt/log bloom execution details, and arbitrary contract calls

Everything else that still exists only to support consensus behavior through C++ shims, broad bridges, legacy object materialization, lifecycle shells, or public compatibility surfaces is migration debt.

## Execution Workflow

Use `$implement-rustaxa-consensus-slice` for each implementation slice. Each slice should inspect the relevant Rust crates, bridge APIs, shim-owned handles, and existing storage/FinalChain/DAG/transaction/vote coverage before adding new C++ orchestration.

Test policy remains the repository-wide rewrite policy:

- Preserve existing test intent while it represents target behavior.
- C++ tests may be disabled, removed, or retargeted only when they are tied to retired legacy behavior, old object materialization, or shim scaffolding.
- Equivalent or stronger Rust module coverage must exist before dropping the old C++ signal.
- If parity depends on the CXX bridge, add bridge-level Rust coverage or a focused Rust-enabled shim test before removing the C++ test signal.

## Current Starting Point

PBFT manager protocol ownership is closed. The remaining blockers to deleting consensus shims and broad bridge code are outside the PBFT manager protocol brain:

- FinalChain/DPoS liveness and mutation observation that depends on non-empty PBFT finalization or external EVM/state execution
- remaining DAG public object materialization at API/network/test edges after DAG manager decision authority moved to Rust
- DAG block proposer executor mechanics that are intentionally outside the accepted Slice 4 boundary: worker threads/timers,
  async VDF execution, stale-proof sleep execution, key-manager signing execution, and compatibility add-block side effects
- TransactionManager EVM gas-estimation executor edge and public/test/network materialization adapters
- vote, slashing, and pillar manager executor/materialization surfaces
- rewards stats legacy carrier ownership
- typed consensus storage ports that are still too narrow for some cross-subsystem side effects
- old public C++ object surfaces for `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, pillar objects, and reward/stat carriers
- app lifecycle, scheduler, timer, signing, event emission, and executor-result plumbing shells

## Slice 1: FinalChain and DPoS Fact Port Completion

Goal: remove consensus dependence on ad hoc C++ FinalChain/DPoS fact sourcing except for the explicit EVM execution boundary.

Status: complete. The first bounded route moved DAG proposer FinalChain-height and DPoS authorization collection into
a typed Rust `BridgeFinalChain::get_dag_proposer_final_chain_facts` port, replacing the C++ proposer shim's ad hoc PBFT
fact request used only to discover the latest finalized period. PBFT vote-weight collection now uses explicit Rust
PBFT-period DPoS fact methods that preserve the `last_finalized + delegation_delay` readiness boundary instead of
letting bridge callers interpret raw snapshot errors.

Open blocker found during validation: focused `pbft_manager_test` runtime coverage still does not execute non-empty PBFT
blocks, so delegated DPoS transactions never publish the FinalChain snapshots needed for later vote totals. The remaining
work is no longer a fact-port gap; it is PBFT non-empty block production/finalization and external-EVM publication
liveness before DPoS mutations can be observed by the Rust snapshot port. One bounded finalization-resume fact leak has
been closed: the PBFT manager shim now passes the actual `block_in_chain` value into Rust finalization intent planning,
so duplicate/resume handling cannot be misclassified as a first-time finalization.

The DAG proposer FinalChain fact port also no longer gates proposal-period DPoS authorization on
`last_finalized >= proposal_period`. Legacy DAG proposal asks DPoS for the proposal period directly and lets the
FinalChain delegation-delay/snapshot API decide whether the fact is available; preserving that contract avoids a circular
dependency where DAG blocks wait for a PBFT period that needs DAG blocks to become non-empty.

Validation after removing that gate exposed a separate non-empty block liveness issue: `PbftManagerTest.pbft_manager_run_single_node`
initially failed with zero non-empty PBFT blocks and zero executed transactions, so the trace moved through DAG block
creation, PBFT proposal/certification, and finalization admission rather than widening the FinalChain fact-port scope.

One adjacent DAG proposer compatibility issue has been closed during that trace: Rust transaction packing now uses the
legacy five-byte sender prefix for multi-shard DAG transaction selection. This restores compatibility with the C++
`sender.toString().substr(0, 10)` rule, but it does not explain the single-node runtime failure because that test uses
the default one-shard proposer configuration.

The single-node runtime blocker was then traced to DAG proposer VDF input sourcing. Bootstrap proposals map DAG level 1
to proposal period 0; legacy `DbStorage::getPeriodBlockHash(0)` returns the zero hash when no PeriodData exists, and the
proposer uses that zero hash in the VRF input. The Rust runtime had treated the missing period-0 PeriodData as
`DAG_PROPOSER_REASON_MISSING_VDF_INPUT`, so it retried forever and PBFT only proposed null anchors. The bridge proposer
runtime now preserves the legacy period-0 zero-hash contract while keeping nonzero missing PeriodData as a retry-only
missing VDF input.

The remaining DPoS eligibility runtime blocker was then traced to transaction packing gas estimation for DPoS mutating
transactions. The Rust FinalChain call path already owned DPoS snapshot queries and finalization-time mutation
application, but dry-run `call` only returned data for DPoS query selectors. Validator registration/delegation
transactions therefore returned gas `0`, were demoted by the Rust transaction pack planner, and never reached
finalization. Rust FinalChain now treats supported DPoS mutating selectors as read-only gas-estimation calls: it decodes
the ABI input, returns the selector's legacy gas cost with empty return data, and does not mutate the snapshot.

Closeout audit: remaining direct C++ `FinalChain` calls from Rust-mode consensus shims are classified. Transaction
packing still calls `FinalChain::call` only as the explicit EVM/gas-estimation executor boundary. PBFT finalization still
dispatches `FinalChain::finalize`, the legacy-compatible bounded `waitForFinalized` yield, and
last-height/delegation-delay lifecycle facts as live execution/lifecycle boundaries around Rust-owned planning. Pillar
block creation still reads bridge root/epoch through
the FinalChain shim because those are external bridge-contract/state execution facts. DAG proposer, DAG verification,
PBFT vote validation/generation, key-manager VRF lookup, transaction admission/cleanup, reward stats, and PBFT state-root
validation now source their consensus FinalChain/DPoS facts through typed Rust `BridgeFinalChain` ports or Rust-backed
FinalChain shim methods.

Scope:

- Inventory remaining consensus consumers that call C++ FinalChain, DPoS, slashing, validator, delegation, stake, or rewards fact APIs.
- Add or extend Rust fact/query ports for consensus-required facts.
- Move supported non-EVM DPoS/account/slashing facts and deterministic mutation planning into Rust snapshots or Rust storage-backed runtimes.
- Keep arbitrary contract execution, state DB mutation, and EVM receipt execution out of scope.

Acceptance:

- Consensus modules obtain validator/delegator/slashing/reward facts through Rust ports, not direct C++ FinalChain calls.
- Any remaining C++ FinalChain call from consensus is explicitly classified as EVM execution, state execution, public compatibility, or lifecycle wiring.
- Rust tests cover every moved fact family, including restart/reload behavior when persisted snapshots are involved.

Stop conditions:

- Stop if the slice requires arbitrary EVM contract execution parity.
- Stop if a fact family needs a broad state DB redesign rather than a consensus-facing Rust port.

## Slice 2: TransactionManager Public and Event Shell Collapse

Goal: make TransactionManager native Rust ownership complete enough that C++ only materializes public views or executes EVM gas estimation.

Status: complete. TransactionManager production shim paths now use Rust-owned runtime/query APIs for queue and sidecar
authority, canonical payload inspection, DAG persistence, finalized-status mutation, public admission command reports,
and event/log intent selection. Remaining `Transaction`/`PeriodData` objects are edge adapters for public/test/network
materialization or EVM gas-estimation execution.

Scope:

- Move public transaction construction and compatibility object creation behind Rust-owned payload/query APIs.
- Make event/log intent selection Rust-owned, with C++ only dispatching already-planned events while the app remains C++ hosted.
- Collapse remaining generic manager orchestration into Rust command sessions over canonical transaction bytes, hashes, sender facts, queue state, and finalized-location facts.
- Keep EVM gas-estimation execution as an explicit executor boundary, but make cache policy, result classification, and retry behavior Rust-owned.

Acceptance:

- TransactionManager no longer uses C++ maps or compatibility objects as decision authority.
- Public transaction objects are materialized only at public API/test/network edges from Rust-retained canonical payloads.
- Event/log dispatch is driven by Rust intents.

Stop conditions:

- Stop if the work expands into EVM execution semantics.
- Stop if public RPC/API shape changes are required beyond edge materialization adapters.

## Slice 3: DAG Manager Mirror and Materialization Collapse

Goal: remove remaining DAG decision authority from C++ graph mirrors, local cache cleanup, and live object materialization.

Status: complete. DAG manager consensus decisions now use Rust graph/runtime state, Rust storage, typed
TransactionManager payload/availability ports, and Rust-owned finalization/add/sync side-effect plans. Remaining C++
`DagBlock` and transaction materialization is executor/public compatibility at API, network, and test edges rather than
protocol authority.

Scope:

- Move remaining DAG graph mirror reads, cache cleanup decisions, and add/sync/finalization side-effect selection into Rust runtimes.
- Replace live transaction-pool decision inputs with typed TransactionManager/Rust payload and availability ports.
- Keep public DAG object materialization only at API/network/test edges.
- Use Rust storage-backed canonical DAG block payloads and compact graph facts as the authority for decisions.

Acceptance:

- DAG manager consensus decisions are made from Rust graph state, Rust storage, and typed transaction/finalization facts.
- C++ `DagBlock` materialization is not required to decide admission, ordering, expiry, finalization cleanup, or sync payload selection.
- Remaining C++ DAG code is executor/public compatibility, not protocol authority.

Stop conditions:

- Stop if the slice requires network pipeline ownership.
- Stop if the slice requires moving arbitrary EVM gas execution into Rust.

## Slice 4: DAG Block Proposer Lifecycle Shell Reduction

Goal: make proposer behavior native Rust except for explicit scheduler, async VDF execution, signing, and add-block execution edges.

Status as of 2026-06-21: implemented for the accepted proposer shell boundary.

- Rust already owns proposal attempt planning, worker-loop command selection, post-pack command selection, VDF wait
  cancellation decisions and poll cadence, stale-proof retry/reset decisions and delay cadence, block construction
  planning, durable per-wallet retry cursor storage, signing boundary progression, and add-block terminal classification.
- C++ still owns the worker thread/timer executor, VDF async execution mechanics, stale-proof sleep execution, signing
  execution, and compatibility `addDagBlock` side effects.
- The add-block executor now reports a typed outcome back to the Rust proposer session before the proposer shell records
  proposed-block telemetry or returns to the worker loop.
- The signing executor now reports a typed outcome before the session advances to add-block execution.

Scope:

- Move proposer worker command selection, retry lifecycle state, VDF request state, stale-proof handling, and block-submission intent planning into Rust.
- Keep async VDF compute and key-manager signing as executor effects with typed reports.
- Feed add-block through a typed executor result rather than letting C++ proposer code own the post-build control flow.

Acceptance:

- Rust owns proposer lifecycle state and command ordering.
- C++ proposer code executes VDF/signing/add-block effects and reports typed outcomes before Rust advances the session.
- Remaining proposer shell is small enough to delete when app lifecycle moves.

Stop conditions:

- Stop if this becomes a VDF crate rewrite.
- Stop if add-block execution requires broad network or EVM scope.

## Slice 5: Vote, Slashing, and Pillar Executor Surface Collapse

Goal: remove remaining vote, slashing, and pillar manager compatibility surfaces that exist only for PBFT manager or consensus-side decisions.

Status as of 2026-06-21: complete.

- Rust already owns PBFT vote-progress planning, verified-vote runtime mutation, pillar vote indexing/planning, and
  double-voting proof transaction planning.
- Slashing transaction insertion is still a C++ executor effect, but the executor now reports typed insertion outcome
  back to Rust before Rust mutates duplicate-proof state or classifies the submission result.
- Pillar vote lookup now has a Rust-retained payload path so C++ materializes returned `PillarVote` objects from Rust
  records at public edges instead of requiring live vote sidecars for selected votes.
- Pillar block finalization preflight is now a Rust-owned effect plan: C++ supplies compact current/latest facts plus
  Rust pillar-vote threshold and selected-weight facts, then only executes Rust-requested vote-bundle request, storage
  persistence, cleanup, event emission, and vote return effects.
- PBFT `VotesWithWeight` compatibility reconstruction now prefers Rust-retained weighted payloads; live PBFT vote
  sidecars are retained only as documented compatibility fallback for low-level helper paths that bypass runtime payload
  retention.
- PBFT vote admission and validation no longer hydrate, mutate, attach, or materialize the incoming live C++ `PbftVote`
  sidecar for weight ownership. The production admission path verifies Rust-retained weighted payload records directly;
  incoming live votes remain only network/public/executor payloads.
- Remaining `live_votes_` maps in PBFT and pillar vote shims are compatibility/test/public API projections only; production
  admission, finalization, slashing, and pillar threshold decisions route through Rust-retained payloads or compact facts.

Scope:

- Replace live `PbftVote`, `PillarVote`, and `PillarBlock` decision reads with Rust-retained payloads and compact facts.
- Move slashing transaction-submission planning as far as possible into Rust: submitter choice, gas bid policy, transaction payload facts, duplicate marking, and post-submit classification.
- Move pillar manager finalization orchestration and block/vote sidecar state behind Rust ports.
- Keep signing, network egress, and external transaction insertion as executor effects.

Acceptance:

- Vote and pillar C++ objects are materialized only for signing, network/public edges, or executor payloads.
- Slashing decisions and duplicate cache state are Rust-owned; C++ only executes transaction signing/insertion while that boundary remains.
- PBFT manager, DAG, and finalization paths consume typed vote/pillar/slashing ports instead of manager-internal sidecars.

Stop conditions:

- Stop if the slice requires network egress pipeline ownership.
- Stop if slashing work expands into unsupported EVM contract execution.

## Slice 6: Rewards Stats Carrier Ownership

Goal: move legacy rewards stats carrier ownership fully into Rust.

Status as of 2026-06-21: complete.

- Rust already owns rewards-stat planning, legacy-compatible `BlockStats` RLP encoding, interval cache write/clear plans,
  and Rust storage reload/clear for persisted block-reward stats rows.
- The C++ rewards-stats shim now treats Rust-produced RLP rows as the authoritative interval carrier and keeps decoded
  `BlockStats` only as a public/test compatibility view for `StateAPI::distribute_rewards` and legacy callers.
- Non-boundary rewards-stat cache writes in the C++ finalization path now append Rust-produced `BlockStats` RLP directly
  to the caller's Rust storage batch instead of committing through a separate rewards runtime storage handle.
- External-EVM FinalChain publication now previews rewards stats on cloned Rust runtime state and commits the rewards
  runtime/cache mirror only after Rust publication storage succeeds.
- Rust and Rust-enabled bridge coverage now covers legacy-compatible RLP shape, interval cache boundaries, storage
  reload/clear, fee attribution, Aspen DAG reward counting, duplicate cert-vote rejection, and external-EVM publication
  batch integration.

Scope:

- Replace legacy `BlockStats` carrier authority with Rust-owned types and canonical compatibility encoding.
- Keep old C++ views only as public/test adapters while callers still require legacy objects.
- Ensure interval cache, boundary cleanup, fee attribution, cert-vote weighting, and DAG reward counts round-trip through Rust-owned storage/reload paths.

Acceptance:

- Rewards stats decisions and persisted carrier state are Rust-owned.
- C++ `BlockStats` materialization is edge-only and removable.
- Rust tests cover codec compatibility, interval boundaries, restart/reload, and migrated finalization integration.

Stop conditions:

- Stop if reward distribution expands into arbitrary unsupported DPoS/EVM methods from Slice 1.

## Slice 7: Typed Consensus Storage Port Generalization

Goal: replace remaining broad bridge/storage compatibility shapes with subsystem-specific Rust ports.

Status as of 2026-06-21: complete.

- Rewards-stat finalization cache writes now use a task-specific Rust rewards storage port to append accepted
  `RewardsStatsProcessResult` cache rows to the surrounding Rust-owned finalization batch. The production rewards path no
  longer routes Rust-produced rewards-stat RLP through the generic storage-shim block-reward-stats appender; the legacy
  `saveBlockRewardsStats` compatibility API remains for public/reference storage callers.
- Direct reward-vote reset compatibility calls now route through a task-specific verified-votes reset storage request
  instead of exposing generic PBFT finalization storage stages. The broader PBFT finalization path still uses ordered
  storage stages for its multi-stage atomic batch.
- Remaining generic storage-shim appenders are classified as compatibility API surface for legacy/reference storage
  callers, tests, and public/admin shells. Rust-mode production shims audited for this slice either use subsystem
  runtime ports directly, ignore legacy `Batch&` compatibility arguments, or route through task-specific Rust ports.
- The storage-boundary guard now rejects newly added direct `storage_shim_*` calls and `BridgeStorageBatch` references
  outside the storage-shim/test compatibility allowlist.

Scope:

- Define task-oriented storage traits or runtime APIs for DAG, transaction, vote, pillar, rewards, finalization, and manager operations that still use generic bridge DTOs.
- Move each operation's atomic write group fully into Rust storage ownership.
- Delete obsolete generic storage shim appender/helper APIs once production callers move.
- Preserve compatibility storage APIs only for legacy/reference builds, tests, admin, public query shells, or explicitly classified lifecycle boundaries.

Acceptance:

- Consensus production routes do not depend on generic bridge storage batches or compatibility appenders.
- Each migrated operation has one Rust-owned atomic write group and documented restart/reload behavior.
- Storage-boundary guard covers the removed route class.

Stop conditions:

- Stop if a proposed port becomes a broad service locator instead of a task-specific operation.
- Stop before storage migration/admin/snapshot flows unless explicitly re-scoped.

## Slice 8: Public Object Surface and Compatibility Adapter Deletion

Goal: remove old C++ consensus object surfaces after all decision paths consume Rust-native payloads and facts.

Scope:

- Inventory remaining materialization of `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, pillar objects, and rewards carriers.
- For each family, classify callers as public API, test fixture, network edge, executor payload, or obsolete compatibility.
- Add Rust-native views or stable bridge DTOs for public/test callers where needed.
- Delete obsolete shim helpers, DTOs, sidecar maps, and compatibility tests that no longer prove target behavior.

Acceptance:

- C++ object materialization is edge-only and not required by consensus internals.
- Obsolete bridge and shim helpers are deleted instead of retained as unused scaffolding.
- Public API compatibility remains stable through Rust-backed adapters.

Progress:

- Removed the obsolete `PillarVotes` live `PillarVote` sidecar map. Rust now owns pillar vote payload retention and
  cleanup; C++ only materializes `PillarVote` objects from Rust-retained records for public/event compatibility.
- Removed the obsolete rewards-stats RLP sidecar map from the C++ shim. Rust owns authoritative rewards-stat bytes and
  storage payloads; C++ keeps only the decoded `BlockStats` view required by public/test and `StateAPI` edges.

Stop conditions:

- Stop if this requires changing external RPC/API semantics.
- Stop if a family still has active protocol decision consumers; send that work back to the relevant subsystem slice.

## Slice 9: Lifecycle, Scheduler, Signing, and Event Executor Shell Collapse

Goal: make the remaining C++ host shell a minimal executor around Rust sessions.

Scope:

- Move lifecycle command selection, scheduler/timer state, executor-result validation, and event intent planning into Rust sessions where they still affect consensus behavior.
- Keep actual OS threads, sleeps, app startup/shutdown wiring, key-manager signing, and event dispatch mechanics in C++ until the app host migrates.
- Standardize typed executor reports across proposer, DAG, transaction, vote, pillar, rewards, and finalization shells.

Acceptance:

- C++ lifecycle code does not choose consensus behavior; it only executes Rust-planned commands and reports results.
- Remaining shell code is small, mechanical, and deletable with the future app-host migration.
- Cross-subsystem executor report validation is consistent and covered by Rust tests.

Stop conditions:

- Stop if the work becomes a full application runtime rewrite.
- Stop if network or EVM ownership is required.

## Suggested Order

1. Slice 1 first, because FinalChain/DPoS fact ownership unblocks DAG, transaction, vote, pillar, and rewards cleanup.
2. Slice 2 is complete: TransactionManager payload/public/event ownership now removes a major source of C++ sidecars.
3. Slice 3 is complete: DAG manager decision authority moved to Rust-backed graph, storage, transaction, and
   finalization/add/sync planning.
4. Slice 4 is complete for the accepted proposer shell boundary: Rust owns proposer lifecycle state and command
   ordering while C++ executes scheduler/timer, async VDF, signing, and add-block effects.
5. Slice 5 is next, because vote/pillar/slashing executor collapse benefits from the stronger FinalChain, transaction,
   DAG, and proposer ports.
6. Slice 6 follows once rewards inputs and FinalChain facts are Rust-owned enough to remove the legacy carrier.
7. Slice 7 should run after the major subsystem routes are known, so storage ports are task-specific instead of speculative.
8. Slice 8 follows after decision paths no longer require C++ objects.
9. Slice 9 remains last, because lifecycle and executor-shell deletion is easiest after subsystem sessions are uniform.

## Closeout Definition

This plan is complete when consensus production behavior outside network/tarcap and EVM/state execution no longer requires C++ shims or broad bridge compatibility code as decision authority. Remaining C++ should be limited to public API adapters, app-host lifecycle mechanics, signing/executor mechanics, and network/EVM boundaries that are explicitly outside this plan.
