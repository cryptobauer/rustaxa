# DAG Manager and Proposer Orchestration Plan

This plan tracks the next large consensus rewrite area after PBFT manager protocol ownership. It complements
`PLAN.md` and `doc/consensus_rewrite_tracker.md`; keep those documents as the higher-level status sources.

Use `$implement-rustaxa-consensus-slice` when implementing slices from this plan.

## Goal

Move DAG manager and DAG proposer orchestration from C++ shim glue into Rust-owned runtimes while preserving public C++
APIs. Network transport, EVM gas estimation, key signing, daemon/thread lifecycle, and public C++ object materialization
remain explicit executor or compatibility boundaries until their owning rewrite tracks move.

## Current State

Status: complete for this DAG manager/proposer orchestration boundary. Remaining C++ is limited to the executor and
compatibility surfaces that this plan intentionally excluded.

- `Dag` / `PivotTree` graph operations route through the Rust DAG domain.
- `DagManager` has Rust-backed verification/finalization decisions for tip uniqueness, proposal-period availability,
  expiry, transaction availability planning, VDF/VRF facts, gas-policy decisions, finalized-order application,
  non-finalized sync selection, selected DAG block RLP loading, sync transaction RLP lookup, finalized counter writes,
  expired DAG block deletes, and expired non-finalized transaction deletes.
- `DagManager::addDagBlock` and `addDagBlockRlp` enter the Rust runtime for duplicate/expiry/reference planning,
  persistence, graph mutation, event/gossip, and compatibility mirror effect selection.
- Proposed DAG blocks cross from proposer to manager as Rust-produced signed block RLP plus transaction payload facts;
  transaction persistence uses Rust-inspected canonical RLP payloads before any compatibility transaction
  materialization.
- `DagBlockProposer` has a full Rust-mode overlay, Rust proposer-eligibility status decisions, legacy VRF input bytes,
  deterministic tip selection, Rust `TransactionManager` pack-session integration, Rust-owned proposer-session skip and
  retry decisions, Rust-selected proposal timestamps, Rust block-intent/final signed-RLP construction, and signed-RLP
  manager submission.

Remaining C++ executor/compatibility boundaries:

- live `DagBlock` / `Transaction` object materialization for public API, event, and network compatibility
- live transaction-pool reads and EVM gas-estimation execution
- FinalChain/DPoS fact collection for DAG verification
- local cache cleanup, temporary legacy graph mirror updates, and live transaction-manager sidecar cleanup
- proposer thread/lifecycle behavior and live network throttle checks
- async VDF compute execution
- temporary key-manager signing
- network gossip/egress

## Target Boundary

Rust should own deterministic DAG protocol decisions, runtime cursors, operation plans, storage-backed selections, and
post-state validation. C++ should execute external effects and materialize compatibility objects only when a current API,
network path, or EVM/gas executor still requires them.

Do not move these in this track unless explicitly rescheduled:

- peer transport, packet wrapping, gossip fanout, peer reporting, and tarcap scheduling
- EVM execution and gas estimation internals
- key-manager signing
- daemon thread ownership and shutdown mechanics
- public API return-object compatibility

## Slice 1: DAG Manager Runtime Shell

Create a long-lived Rust `DagManagerRuntime` that owns compact DAG manager operation state and exposes ordered sessions
for manager operations.

Status: `complete` for this plan's target boundary.

Landed:

- `DagManager::addDagBlock` duplicate, expiry, and pivot/tip availability planning now enters the long-lived Rust
  `DagManagerRuntime` through compact block facts. The C++ shim executes the returned persistence, graph mutation,
  event, gossip, and compatibility mirror effects.
- `DagManager::verifyBlock` now opens an ordered Rust runtime session. Rust owns precheck, transaction-query planning,
  transaction availability, VDF/DPoS reject ordering, gas reject ordering, and terminal status selection while C++
  reports live transaction, FinalChain authorization, VDF verifier, and EVM gas-estimation facts.
- Finalized-order application, non-finalized sync selection, expiry cleanup, proposer frontier facts, proposal-attempt
  planning, and block construction planning route through the same Rust runtime/storage boundary.

Scope:

- Add Rust runtime/session APIs for `verifyBlock`, `addDagBlock`, finalized-order application, non-finalized sync
  selection, expiry cleanup, and missing transaction handling.
- Reuse existing Rust DAG graph, storage, transaction, and FinalChain fact paths instead of adding new C++ decision
  logic.
- C++ supplies only requested live facts: transaction availability/RLPs, FinalChain DPoS facts, EVM gas-estimate
  results, and network/report execution results.
- Return typed executor effects and require C++ reports before advancing session cursors.

Acceptance:

- No new DAG manager production decision logic is added in C++.
- Existing Rust-backed graph/order/storage decisions are reachable through one runtime boundary.
- `doc/consensus_rewrite_tracker.md` documents the remaining C++ effects as executor-only boundaries.
- Any unported operation is an explicit shim-local stub or documented compatibility route, not silent legacy fallback.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag`
- `cmake --build /build --target dag_test dag_shim_test rust_consensus_tests --parallel 12`
- Targeted DAG manager tests that exercise changed behavior.
- `git diff --check`

## Slice 2: DAG Block Materialization and Payload Reduction

Reduce C++ object materialization in DAG manager paths by routing decisions through canonical DAG block RLP, transaction
hashes/RLPs, and compact facts.

Status: `complete` for this plan's target boundary.

Landed:

- Proposed DAG blocks can now cross the proposer-to-manager boundary as Rust-produced canonical signed block RLP plus
  transaction hash/RLP payloads. Rust decodes compact manager facts from the signed RLP for add-block planning and graph
  mutation; C++ materializes the accepted `DagBlock` only inside the manager for the remaining cache, legacy mirror,
  event, and network compatibility surfaces.
- Proposed DAG transaction payloads now persist through a payload-based `TransactionManager` shim entry point. Rust
  inspects canonical transaction RLPs, verifies the supplied hashes, and owns DAG transaction storage/sidecar mutation
  before C++ materializes transactions only for the remaining network-gossip compatibility path.

Scope:

- Route DAG block verification/addition from canonical RLP plus Rust-inspected block and transaction facts.
- Make sync and finalized-order paths return payload/fact records instead of requiring eager `DagBlock` or
  `Transaction` materialization.
- Keep C++ materialization only for public API return values, EVM/gas executor calls, and network compatibility.
- Search for obsolete shim helpers after each moved fact source and remove them in the same slice.

Acceptance:

- Sync selection and finalized-order cleanup do not reopen C++ block objects for facts Rust already owns.
- Missing transaction and expired transaction decisions use Rust-owned transaction/storage views.
- Temporary materialization sites are documented as API/EVM/network compatibility, not decision authority.
- Restart/reload and sync payload parity are covered by focused tests.

Validation:

- Rust DAG and storage-backed DAG tests.
- `cmake --build /build --target dag_test dag_block_test rust_consensus_tests --parallel 12`
- A focused C++ shim/manager test for changed sync/finalized-order paths.
- `git diff --check`

## Slice 3: DAG Proposer Policy Runtime

Move proposer orchestration decisions into Rust while leaving signing, thread lifecycle, and network submission in C++.

Status: `complete` for this plan's target boundary.

Landed:

- The Rust-mode `DagBlockProposer::selectDagBlockTips` compatibility method now routes through a storage-backed Rust
  DAG manager runtime tip-selection plan. Rust loads tip metadata from canonical stored DAG block RLP, skips missing
  tips, applies unique-proposer priority, descending-level ordering, gas-limit enforcement, and max-tip enforcement; C++
  only translates hashes for the legacy API.
- `DagBlockProposer::proposeDagBlock` now opens a Rust-owned proposer session. Rust owns attempt skip reasons,
  transaction-pack command selection, VDF input/message bytes, VDF wait/cancel decisions, stale-proof decisions,
  add-block completion outcome, missing VDF input status, and retry-cursor updates while C++ reports live transaction
  packing, async VDF executor, compatibility sleep, signing/materialization, and add-block facts.
- Transaction-pack throttling is now an explicit Rust proposer-session report and reason instead of being collapsed into
  an empty eligible transaction pack.
- Production proposal block-intent planning now selects the wall-clock timestamp inside the Rust bridge before temporary
  C++ signing.
- Proposed blocks now enter `DagManager` as signed block RLP plus transaction payloads; proposer-side `DagBlock` and
  `Transaction` materialization was removed.

Remaining:

- The proposer path still uses temporary C++ signing before Rust finalizes the signed block RLP.
- `DagManager` still materializes accepted proposed blocks for compatibility cache, legacy mirror, event emission, and
  network gossip; accepted proposed transactions are materialized only for network gossip.
- The live network throttle check itself still runs in the temporary C++ executor shell until proposer worker/network
  lifecycle ownership moves.

Scope:

- Add a Rust proposer runtime/session that owns proposer loop decisions, skip reasons, VDF input/payload planning,
  tip policy, candidate transaction packing command selection, block-construction commands, and report-driven cursor
  advancement.
- Reuse the Rust `TransactionManager` pack session for transaction selection.
- C++ executes VDF compute if still external, key signing, final `DagBlock` materialization, `DagManager::addDagBlock`,
  and network gossip.
- Return stable proposer statuses for no eligible proposer, no tips, no packable transactions, missing VDF input,
  constructed block, and executor failure.

Acceptance:

- C++ no longer decides proposer skip reasons, tip policy, VDF payload shape, or transaction-packing flow.
- Rust receives executor reports before advancing the proposer session.
- Remaining C++ proposer code is lifecycle/effect execution plus compatibility materialization.

Validation:

- Rust proposer runtime tests for each skip/construct/report branch.
- `cmake --build /build --target dag_block_test pbft_manager_test rust_consensus_tests --parallel 12`
- Transaction packing tests when pack-session behavior is touched.
- `git diff --check`

## Slice 4: DAG Side-Effect Executor Cleanup

Delete or quarantine obsolete shim helpers after DAG manager/proposer sessions become authoritative.

Status: `complete` for this plan's target boundary.

Landed:

- Removed obsolete standalone DAG proposer bridge DTOs and functions for post-pack, retry-reset, VDF-wait, and
  stale-proof planning. The ordered proposer session is now the bridge surface for those executor reports; the underlying
  Rust consensus planners remain crate-local implementation details used by that session.
- Classified remaining DAG manager/proposer C++ surfaces as explicit executor or compatibility boundaries in this file,
  `doc/consensus_rewrite_tracker.md`, and `PLAN.md`.

Scope:

- Remove duplicated C++ helper logic for DAG verification, sync selection, cleanup, proposer policy, and now-obsolete
  bridge DTOs.
- Update `doc/consensus_rewrite_tracker.md` and `PLAN.md`.
- Classify remaining live materialization as public API, network, signing, or EVM compatibility only.

Acceptance:

- DAG manager/proposer tracker rows classify every remaining C++ surface as executor or compatibility work.
- Remaining gaps are concrete external boundaries: network, signing, EVM gas execution, daemon lifecycle, temporary
  mirror/cache cleanup, and public API/event/network objects.
- No stale bridge structs/functions or roadmap text remain for replaced routes.

Validation:

- Targeted searches for removed helper names and old route APIs.
- `cargo fmt --manifest-path rust/Cargo.toml --all --check`
- Relevant Rust DAG/proposer tests.
- `cmake --build /build --target dag_test dag_block_test dag_shim_test rust_consensus_tests --parallel 12`
- `git diff --check`

## Tracking Notes

- Keep slice commits small enough to review independently.
- Prefer canonical bytes, hashes, scalar facts, and typed executor effects over eager C++ objects.
- Do not move network transport, EVM/gas execution, or signing into this track.
- Update this file and `doc/consensus_rewrite_tracker.md` after every slice.
- Commit after each landed slice.
