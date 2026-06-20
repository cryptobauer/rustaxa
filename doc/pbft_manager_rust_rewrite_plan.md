# PBFT Manager Rust Rewrite Plan

This plan tracks the remaining work needed to move PBFT manager behavior out of C++ shims and bridge compatibility
helpers and into Rust. It complements `doc/consensus_rewrite_tracker.md`; the tracker records current status, while this
file is the execution plan for retiring PBFT manager compatibility surfaces.

## Target Boundary

Rust owns deterministic PBFT behavior, durable consensus state, storage/query selection, restart normalization,
idempotency, state-machine transitions, validation decisions, sidecar metadata, and ordered effect planning.

C++ may remain only as an executor for these explicit boundaries:

- network/tarcap transport, packet wrapping, peer send policy, gossip fanout, peer marking, disconnect/report mechanics
- EVM and `StateAPI` execution, state DB access, receipts/log blooms that still require the EVM executor, contract calls
- temporary public API object materialization at the edge, until the owning subsystem exposes Rust-native views

All other PBFT manager shim and bridge code is temporary rewrite debt. In particular, `DbStorage`, `BridgeStorage`,
generic storage-shim batches, C++ PBFT scalar mirrors, C++ sidecar collections, and broad bridge query helpers should
not remain on production Rust-mode paths.

## Execution Workflow

PBFT manager rewrite work should be implemented through `$implement-rustaxa-consensus-slice`. Each slice must use that
workflow to pick a bounded change, inspect the existing Rust crates and bridge APIs, involve the relevant custom agents
when implementation or design review benefits from delegation, update this plan or `doc/consensus_rewrite_tracker.md`
when status changes, and close with targeted validation.

Slice work should prefer Rust ownership over preserving C++ compatibility for its own sake. C++ tests may be disabled,
removed, or retargeted when they become the reason a Rust-owned implementation cannot advance, but only under these
conditions:

- The disabled C++ test is tied to legacy C++ behavior, old object materialization, or shim scaffolding that the slice is
  intentionally retiring.
- Equivalent or stronger Rust module coverage exists for the moved behavior before the C++ test signal is dropped.
- If parity depends on the CXX bridge, a bridge-level Rust test or focused Rust-enabled shim test must cover the boundary.
- The commit or tracker entry explains why the C++ test no longer represents the target Rust-mode behavior.

The minimum bar is test parity for the Rust module that takes ownership. Disabling a C++ test is acceptable only as a
consequence of moving behavior to Rust, not as a way to hide missing Rust behavior.

## Current Starting Point

The migrated production storage routes are closed: Rust owns storage fact collection, write ordering, batch commit/drop,
and restart normalization for the audited PBFT finalization, vote, transaction, DAG/proposed-block, rewards, pillar, gas
pricing, and manager scalar families.

The remaining PBFT manager work is broader than storage:

- PBFT manager construction still receives `std::shared_ptr<DbStorage>` and seeds Rust runtimes through
  `db_->rustStorage()`.
- Startup and replay still use compatibility reads and C++ materialization for some proposed-block, queue, vote, and
  period-data paths.
- C++ still holds compatibility mirrors for state, timers, sidecars, queue payloads, cert-voted block objects, and public
  API materialization.
- Bridge APIs still expose broad `BridgeStorage` factories and `storage_shim_*` batch helpers used by compatibility
  surfaces.
- The C++ overlay still contains substantial manager orchestration around Rust planners.

## Slice 1: PBFT Runtime Root and Constructor Collapse

Goal: stop using `DbStorage` and `BridgeStorage` as the PBFT manager construction surface.

Status: complete.

Landed:

- `App::init` now creates the long-lived `BridgePbftManagerRuntime` before constructing `PbftManager`.
- The Rust-mode PBFT manager constructor receives that typed runtime handle directly and no longer calls
  `db_->rustStorage()` or accepts `BridgeStorage` for its core runtime.
- The PBFT manager overlay keeps `DbStorage` only as documented temporary compatibility for network/EVM/public
  materialization and lifecycle edges.
- Lazy runtime creation in `initialState()` is replaced with an explicit invariant failure because Rust-mode startup must
  supply the runtime root before the manager is used.

Scope:

- Introduce a Rust-owned PBFT runtime root or typed PBFT manager runtime handle that is created from the application
  storage root before constructing `PbftManager`.
- Change PBFT manager Rust-mode construction to receive typed Rust handles and query ports instead of
  `std::shared_ptr<DbStorage>` for consensus state.
- Keep `DbStorage` only where it is still needed for the explicit EVM/state DB boundary or legacy public API edge.
- Move startup scalar and status restoration behind the typed runtime root.

Acceptance:

- `PbftManager` Rust-mode construction does not call `db_->rustStorage()`.
- PBFT manager Rust-mode code does not require `BridgeStorage` to create its core runtime.
- Any remaining `DbStorage` member in the PBFT manager overlay is documented as network, EVM, or public API
  materialization debt.

Validation:

- `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`
- focused PBFT manager shim build/test target when touched
- storage-boundary guard

## Slice 2: Startup, Replay, and Restore Ownership

Goal: make Rust own PBFT manager restart state and replay classification without C++ compatibility reads.

Status: complete for the bounded startup/restore ownership pass. Remaining C++ `PeriodData` materialization is execution
compatibility for FinalChain replay and recent-transaction hydration and is tracked by later finalization, sidecar, and
sync slices rather than startup decision ownership.

Landed:

- Startup replay range selection is planned by Rust from compact height facts.
- Startup finalized-period replay loads canonical period data, finalized DAG hashes, and dynamic-lambda facts through the
  long-lived PBFT manager runtime storage handle.
- PBFT scalar, status, and cert-voted metadata restore is seeded from the Rust runtime snapshot.
- Proposed-block startup restore no longer falls back to `DbStorage::getProposedPbftBlocks()` or startup-time C++
  `PbftBlock` materialization; it hydrates the Rust-owned proposed-block index from Rust storage.
- Remaining C++ materialization during startup is limited to executor/public compatibility boundaries: FinalChain replay,
  vote validation side effects, transaction-manager recent-finalized hydration, and cert-voted block sidecar recovery.

Scope:

- Move proposed-block startup restore, cert-voted metadata restore, period-data queue metadata restore, and bounded
  finalization replay inputs into Rust runtime/query APIs.
- Replace C++ object scans with canonical RLP, hashes, periods, rounds, steps, and compact metadata.
- Keep C++ object materialization only for network or public API surfaces that still require old types.
- Delete PBFT manager startup reads through `DbStorage` once Rust restore APIs cover the same facts.

Acceptance:

- Startup decisions are derived from Rust runtime snapshots and typed storage queries.
- Proposed-block and queue restore paths do not materialize C++ `PbftBlock` or `PeriodData` just to compute PBFT manager
  control facts.
- Restart replay status remains explicit: complete, replay-needed, missing primary, conflicting primary, or unsupported.

Validation:

- Rust unit tests for restore/replay classification
- bridge tests for recovered PBFT manager runtime snapshots
- affected PBFT manager restart/gtest coverage

## Slice 3: State Mirrors, Timers, and Runtime Snapshot Authority

Goal: remove C++ scalar and timer mirrors as PBFT manager authority.

Status: complete for the bounded runtime-snapshot authority pass. Remaining direct writes to executor compatibility
fields are tracked by the later finalization and sidecar slices unless they affect planner input authority.

Landed:

- Public round and step getters read the Rust runtime snapshot when the runtime is present.
- Broadcast planning reads lambda and broadcast counters from the Rust runtime snapshot instead of using the C++ lambda
  mirror as planner authority.
- Deadline calculation reads the Rust runtime snapshot lambda when available; the C++ lambda field remains a temporary
  executor/public compatibility cache.
- Step persistence no longer applies local C++ exponential lambda backoff; lambda timing remains selected by Rust
  transition/runtime snapshots and mirrored back to C++ only for executor compatibility.

Scope:

- Move remaining round, step, state, lambda, next-step-time, broadcast counters, next-vote flags, cert-voted metadata,
  sync-period cursor, and related timing facts into the Rust runtime snapshot.
- Make C++ read these fields only from Rust snapshots when it must execute sleeps, network calls, or public getters.
- Replace direct C++ mutation helpers with Rust runtime commands that return post-state snapshots.
- Delete obsolete shim-local mirror helpers after their callers move.

Acceptance:

- C++ PBFT manager fields are caches for executor/public compatibility, not planner inputs.
- Every state transition that changes PBFT manager scalar state is committed through Rust first.
- Drift checks compare C++ compatibility caches against Rust snapshots and are removable once caches disappear.

Validation:

- Rust state-transition unit tests
- bridge tests for snapshot hydration and mismatch rejection
- focused PBFT manager transition tests

## Slice 4: State-Action and Daemon Control Flow Ownership

Goal: move the remaining branch-local PBFT manager control logic from the overlay into Rust planners.

Status: complete for the bounded daemon/session ownership pass. The overlay now acts as an executor for Rust-issued
runtime-session actions and state-action effects. The remaining stricter follow-up, if selected, is a separate
non-trivial contract for deriving advance-round candidates from Rust-owned VoteManager facts instead of C++ live vote
manager state.

Landed:

- PBFT daemon ticks are driven by a Rust runtime session with cursor-checked action reports.
- State-action branch selection is planned by Rust through effect sessions; C++ executes proposal, vote, transition,
  network, wait, and live-object side effects and reports typed outcomes.
- Certify and second-finish follow-up decisions use Rust session flags returned after executor reports.
- Transition eligibility and timing are selected through Rust transition plans before C++ mutates compatibility caches.
- Network transport remains an executor boundary; Rust returns action/effect intent rather than owning packet transport.

Residual:

- Advance-round candidate derivation still depends on C++ `VoteManager` live state before Rust validates the candidate.
  Moving that requires a new Rust VoteManager fact/query contract and is intentionally not folded into this bounded
  Slice 4 closeout.

Scope:

- Expand the daemon tick/session model so Rust selects all PBFT state-action branches and follow-up actions.
- Replace shim-local helper sequencing for proposal, filtering, certify, finish, finish-polling, round advance, rebroadcast,
  and sleep selection with ordered Rust effects.
- Keep C++ as executor for network sends, waits, local signing, and EVM-facing calls.
- Report executor outcomes back to Rust before the runtime cursor advances.

Acceptance:

- The C++ overlay no longer decides PBFT branch order or transition eligibility.
- C++ executes a Rust-returned effect list and reports typed results.
- Network effects remain boundary effects, not Rust-owned transport work.

Validation:

- Rust planner tests for action ordering and rejection paths
- bridge session tests for cursor mismatch and failed executor reports
- affected PBFT manager tests

## Slice 5: Object Sidecar and Materialization Reduction

Goal: replace C++ PBFT sidecar authority with Rust-owned canonical payloads and compact facts.

Status: planned.

Implementation plan:

1. Inventory sidecar authority.

- Audit PBFT manager overlay reads of `PbftBlock`, `PbftVote`, `PeriodData`, DAG block, transaction, and pillar sidecars.
- Classify each read as one of: protocol decision input, executor input, network/public materialization, EVM/FinalChain
  boundary materialization, or temporary compatibility cache.
- Treat protocol decision reads as Slice 5 candidates. Leave network/public/EVM materialization as allowed boundaries and
  mark them at the call site when they remain.

2. Move proposed-block decision facts first.

- Extend the Rust proposed-block/runtime APIs so PBFT manager decision paths can use compact proposed-block facts:
  period, block hash, pivot hash, validation flag, canonical block RLP availability, proposer identity facts when needed,
  and missing/corrupt status.
- Keep C++ `PbftBlock` reconstruction only when executing network/public/EVM-facing work or when a live validator check
  still needs the legacy object.
- Replace decision-side calls that fetch `getPbftProposedBlock()` just to read period, hash, pivot, or validation state
  with metadata/runtime queries.
- Preserve `getPbftProposedBlock()` as an explicit materialization edge and document why each remaining caller still
  needs a C++ object.

Landed in proposed-block metadata authority:

- Proposed-block admission lookup in the PBFT manager overlay reads Rust-owned compact metadata first and no longer
  materializes a C++ `PbftBlock` merely to decide whether the block exists or is already valid.
- C++ `PbftBlock` materialization remains only for the validation executor path and accepted vote-generation return.

Landed in cert-voted payload authority:

- First-finish cert-voted next-vote execution now treats the Rust runtime metadata and persisted cert-voted payload as
  authoritative when the temporary C++ `cert_voted_block_for_round_` cache is empty.
- The overlay rehydrates the C++ `PbftBlock` only as a vote-generation executor object, validates period, round, and hash
  against the Rust runtime snapshot, and reports missing or corrupt Rust payload as an invariant executor failure.

3. Move cert-voted sidecar authority.

- Keep the Rust runtime as the source of cert-voted period, round, hash, and persistence state.
- Add a Rust-retained canonical payload path for the cert-voted block so first-finish/next-vote planning can validate
  the compact metadata without relying on `cert_voted_block_for_round_`.
- Leave C++ `cert_voted_block_for_round_` only as a vote-generation/materialization cache until Rust vote generation can
  accept canonical payload references directly.
- Missing Rust-retained cert-voted payload for a Rust-planned cert-voted next vote must be an invariant error, not a
  silent skip.

4. Move period-data queue metadata before full payload ownership.

- Extend the Rust period-data queue/runtime contract so PBFT sync decisions use canonical period-data RLP and compact
  metadata: PBFT block period/hash/prev hash, pivot, cert-vote counts, previous-cert presence, transaction hashes, pillar
  facts, and finalized transaction warning facts.
- Keep C++ `PeriodData` construction only where the current finalization executor or public/network boundary still
  requires legacy objects.
- Replace queue decision code that reopens `PeriodData` only to compute status with Rust-inspected metadata and typed
  status reports.

5. Move vote sidecar decision facts opportunistically.

- Route PBFT manager reads of proposal, soft, cert, next, reward, and pillar vote metadata through existing Rust vote,
  reward-vote, and pillar-vote runtimes where available.
- Do not broaden the slice into a VoteManager rewrite. Use narrow query/fact APIs that return canonical vote bytes,
  voter, period, round, step, type, block hash, weight, and validation status.
- Leave local signing, network gossip, and public object returns as executor/materialization boundaries.

6. Delete obsolete compatibility helpers as each family moves.

- After each family is routed through Rust facts, search for shim helpers, bridge structs, and tests that only existed to
  materialize C++ objects for that decision path.
- Delete obsolete rewrite-owned helpers in the same commit when they are no longer needed for restart/reload, parity, or
  public API compatibility.
- If a helper must remain, add a TODO at the call site naming the allowed boundary and the later slice that removes it.

Proposed commit slicing:

- Commit 1: proposed-block metadata authority. Replace PBFT manager decision reads with Rust proposed-block metadata and
  document remaining `PbftBlock` materialization edges.
- Commit 2: cert-voted payload authority. Make Rust-retained cert-voted metadata/payload the planner source and leave the
  C++ sidecar only as a vote-generation cache.
- Commit 3: period-data queue metadata authority. Move queue decision facts to Rust-inspected canonical RLP and keep
  `PeriodData` construction only at finalization/public boundaries.
- Commit 4, only if still bounded: vote/pillar/reward sidecar fact cleanup plus deletion of obsolete helpers discovered
  by the first three commits.

Stop conditions:

- Stop before changing network/tarcap transport, peer gossip, packet wrapping, or disconnect/report execution.
- Stop before moving arbitrary EVM/FinalChain execution or state DB mutation into Rust.
- Stop if proposed-block metadata replacement requires a broad `PbftBlock` Rust model rewrite rather than compact facts.
- Stop if period-data queue work expands into the full sync/finalization executor collapse; that belongs to Slices 6 and
  7.
- Stop if vote sidecar cleanup requires reworking `VoteManager` ownership rather than adding narrow Rust fact/query
  ports.

Scope:

- Move remaining `PbftBlock`, `PbftVote`, `PeriodData`, DAG block, transaction, and pillar sidecar state that is used for
  PBFT decisions into Rust runtimes or typed query APIs.
- Store canonical RLP and compact identity facts in Rust; materialize C++ objects only at network/public/EVM boundaries.
- Continue moving reward-vote, cert-vote, proposed-block, queue, and pillar-vote selection to Rust-retained payloads.
- Delete live sidecar maps once no PBFT manager decision reads them.

Acceptance:

- PBFT manager decisions never reopen C++ objects merely to read hash, period, round, step, voter, pivot, transaction, or
  pillar metadata.
- Temporary materialization call sites state which allowed boundary still needs the old C++ type.
- Missing Rust-retained payloads are invariant errors, not silent partial results.

Validation:

- Rust payload/codec tests for each moved sidecar family
- bridge tests proving C++ materialization is edge-only
- targeted sync/finalization tests

## Slice 6: Finalization Executor Collapse Except EVM

Goal: make Rust own the finalization plan and all non-EVM post-commit state updates.

Scope:

- Move finalization post-commit ordering, PBFT-chain head advancement, DAG finalized-order commit reports, transaction
  finalized-status reports, reward metadata updates, sortition commits, gas/rewards side effects, and pillar pre/post
  processing reports behind one Rust finalization runtime contract.
- Keep arbitrary EVM execution and state DB mutation as explicit executor effects.
- Ensure Rust validates every executor report before advancing the finalization cursor.
- Delete storage-shim batch calls that remain only for finalization compatibility.

Acceptance:

- Non-EVM finalization side effects are Rust-planned and Rust-validated.
- C++ cannot independently mutate PBFT manager finalization state after Rust accepts a plan.
- Any EVM effect is explicit in the Rust plan and has a typed report path.

Validation:

- Rust finalization runtime tests
- bridge tests for ordered executor report validation
- `rust_storage_tests`
- affected PBFT/final-chain/gtest coverage

## Slice 7: Sync and Period-Data Intake Without C++ Decision State

Goal: make Rust own PBFT sync admission and period-data queue processing from canonical bytes and compact facts.

Scope:

- Move remaining sync-period admission, period-data validation, reward/cert vote checks, transaction warning
  classification, pillar-data checks, and peer-report decisions into Rust.
- Keep peer identity and network report execution in C++ until the network pipeline owns it.
- Replace C++ queue mutation and live `PeriodData` sidecar reads with Rust queue runtime commands.

Acceptance:

- PBFT sync decisions are made from Rust-inspected period-data payloads and Rust-owned queue metadata.
- C++ does not mutate the PBFT sync queue outside Rust commands.
- Peer reporting remains an executor effect with typed Rust reasons.

Validation:

- Rust sync admission tests
- bridge queue-drain/session tests
- affected PBFT sync tests

## Slice 8: Vote, Pillar, and Slashing Boundary Tightening

Goal: remove PBFT manager reliance on vote and pillar C++ managers except for explicit network/public execution.

Scope:

- Route reward-vote selection, cert-vote lookup, next-vote facts, pillar-vote bundle checks, and slashing proof facts
  through Rust-owned runtimes.
- Make `VoteManager` and `PillarChainManager` expose typed Rust ports for PBFT manager instead of C++ object sidecars.
- Keep network egress, local signing, and external transaction submission as executor effects.

Acceptance:

- PBFT manager does not consume live C++ `PbftVote` or `PillarVote` sidecars for protocol decisions.
- Vote and pillar manager C++ overlays only materialize old objects for allowed edge boundaries.
- Slashing submission is planned from Rust-normalized evidence.

Validation:

- Rust vote/pillar planner tests
- bridge tests for selected reward/cert/pillar payloads
- affected vote and pillar manager tests

## Slice 9: Storage Shim and Bridge Surface Deletion

Goal: delete PBFT manager dependency on generic storage compatibility APIs.

Scope:

- Remove `DbStorage` methods that only remain for PBFT manager compatibility after slices 1-8 move their callers.
- Remove `BridgeStorageBatch` and `storage_shim_*` functions that no production Rust-mode caller needs.
- Replace broad `BridgeStorage` factories with typed runtime construction from the Rust application storage root.
- Keep only storage APIs needed by legacy/reference builds, tests that intentionally validate compatibility, network
  boundary materialization, or EVM/state execution.

Acceptance:

- PBFT manager Rust-mode code has no dependency on `DbStorage`, `BridgeStorage`, or storage-shim batches except documented
  network/EVM edge paths.
- Targeted searches show no stale PBFT manager references to deleted methods.
- The storage-boundary guard remains green and is stricter where possible.

Validation:

- `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge storage pbft`
- `rust_storage_tests`
- focused C++ shim tests
- `git diff --check`

## Slice 10: Overlay Shrink and Upstream-Sync Cleanup

Goal: reduce the PBFT manager overlay from copied orchestration to a thin executor surface.

Scope:

- Delete copied PBFT manager branches whose behavior is now Rust-planned.
- Keep shim-owned methods only for public API compatibility and allowed executor effects.
- Remove obsolete TODOs, stale bridge DTOs, compatibility tests, and duplicate helper functions.
- Verify original upstream-owned files stay clean or have documented temporary exceptions.

Acceptance:

- The PBFT manager overlay is small enough to audit as an executor/public API adapter.
- No Rust-enabled production behavior forwards to `PbftManagerOld`.
- Remaining C++ code is classified as network, EVM, public API materialization, lifecycle wiring, or temporary test
  compatibility.

Validation:

- targeted PBFT manager tests
- rewrite storage boundary guard
- `make cpp-intersection-list`
- focused diff against `upstream-main` for touched upstream-owned paths

## Sequencing Notes

- Slices 1 and 2 should happen before broad deletion work because they expose the real PBFT runtime root and restart
  contract.
- Slices 3 and 4 reduce C++ decision authority and should be kept in small commits because state-machine regressions are
  high risk.
- Slices 5 through 8 can be split by sidecar family when needed, but each sub-slice must end with Rust as the authority
  for the moved facts.
- Any slice that disables or removes C++ test coverage must land Rust module or bridge parity coverage in the same commit.
- Slice 9 should not start until targeted searches show the deleted storage shim methods have no PBFT manager production
  callers.
- Slice 10 is cleanup after behavior has moved; it should not carry new PBFT semantics.

## Closeout Definition

This plan is complete when Rust-mode PBFT manager production behavior no longer depends on `DbStorage`, `BridgeStorage`,
generic storage-shim batches, C++ scalar mirrors, or C++ protocol sidecars, except where a call site is explicitly part of
network/tarcap execution, EVM/state execution, or public API materialization. At that point the C++ PBFT manager overlay
should be an executor and compatibility adapter around a Rust-owned PBFT runtime.
