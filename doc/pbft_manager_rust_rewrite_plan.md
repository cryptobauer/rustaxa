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
