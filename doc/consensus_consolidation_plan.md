# Consensus Bridge and Shim Reduction Plan

This plan replaces the completed slice-by-slice consolidation diary with the active plan for making the Rust-enabled
node stop exposing the legacy internal C++ architecture. `PLAN.md` owns strategic boundaries,
`doc/consensus_rewrite_tracker.md` is the only status and dependency queue, and
`doc/consensus_bridge_shim_audit.md` is the mechanically checked live inventory. Implementation history belongs in git,
tests, and the tracker’s concise completion evidence rather than in this file.

## Problem Statement

The bridge and consensus shims are approximately 71,000 lines: about 49,000 lines under
`rust/crates/rustaxa-bridge/src` and 22,000 lines of shim headers and implementations. The bridge crate contains native
application runtimes and their tests, while the shims reproduce broad C++ manager APIs and repeatedly materialize Rust
bytes into legacy object graphs. A fully classified surface is not necessarily a minimal surface.

The reduction target is architectural:

- `rustaxa-bridge` contains CXX declarations, plain boundary carriers, conversions, and thin calls into native Rust
  services; it does not own consensus application state or protocol runtimes.
- Rust-enabled production code has one supported application composition, not every historical partial-module topology.
- Internal C++ consumers use narrow application, query, transport, or executor APIs instead of concrete consensus
  manager classes.
- Canonical bytes, opaque identities, and client-specific views cross the boundary. Legacy `PbftBlock`, `PbftVote`,
  `DagBlock`, `Transaction`, `PeriodData`, and pillar object graphs are materialized only for named public clients.
- Pure-C++ behavior remains available on `cpp-reference`; preserving it does not require preserving the same internal
  class graph in Rust-enabled production.

## Cutover Decisions Now in Force

The task owner has selected aggressive Rust cutover. These are implementation constraints, not open questions:

1. Internal C++ manager APIs are not compatibility promises on Rust-enabled `main`. Only named external/public clients
   receive compatibility adapters.
2. Production Rust mode uses one supported feature bundle. Granular rewrite flags may remain as pure-C++/reference or
   short-lived test gates, but do not require partial Rust application services.
3. Tests do not by themselves justify a production CXX export, compatibility constructor, or shim. Behavioral tests
   move with the native runtime; bridge tests cover only ABI and conversion behavior.
4. Rust owns network consensus ingress, admission, routing, queueing, and effect decisions; tarcap is a leaf transport
   executor. Rust owns EVM/FinalChain orchestration and canonical payload/result validation; C++ `StateAPI` is a leaf
   concrete EVM/`state_db` executor.
5. Logging, timers, events, and public formatting are adapter concerns, not reasons to retain deterministic orchestration
   in C++.

## Non-Negotiable Boundaries

- Preserve the untouched pure-C++ implementations and validation route.
- Keep physical network transport mechanics in tarcap while implementing the now-authorized `CRW-N01` pipeline cutover.
- Keep concrete EVM execution and `state_db/` mutation outside Rust while implementing the now-authorized `CRW-E01`
  orchestration and adapter contraction. Moving the concrete executor itself still requires a later explicit decision.
- Preserve externally observable codecs, hashes, ordering, receipts, and error behavior with parity coverage.
- Do not edit upstream-owned C++ merely to make deletion easier; migrate consumers or use the established overlay/source
  selection strategy.

## Workstreams

### 1. Establish a measured deletion contract

- Generate checked metrics for bridge/shim lines, CXX functions and carriers, handles, shim directories, compatibility
  constructors, partial-service factories, and non-test C++ consumers.
- Give every retained facade a named client and deletion condition. “Public compatibility” without a named client is not
  sufficient.
- Add budgets that must decrease in each implementation slice. New exports, carriers, flags, or shim methods require an
  explicit tracker entry and compensating deletion.
- Extend the inventory guard to distinguish production callers from tests and to reject test-only CXX exports unless
  allowlisted as conformance boundaries.

### 2. Move application ownership out of `rustaxa-bridge`

- Introduce a native Rust application/runtime crate or an equivalent non-bridge module boundary.
- Move PBFT service state and DAG/transaction/sortition service state, their construction, restoration, lock domains,
  and behavioral tests out of `rustaxa-bridge`.
- Keep bridge-owned wrappers as thin references to native application services.
- Move bridge-module unit tests to their native domain/runtime owners; retain only CXX conversion, error mapping, and
  boundary-lifetime tests in `rustaxa-bridge`.

Completion condition: removing CXX support from a native service would not require moving or rewriting its protocol
logic or behavioral tests.

Progress: the proposed-block and PBFT-chain sibling owners are the first bounded extractions. CXX-free
`rustaxa-consensus::proposed_blocks::ProposedBlocksService` owns its storage lifetime, restoration, lock domain,
durable-first mutation, snapshots, cleanup, and native behavioral coverage. Native `PbftService` embeds it and the
remaining proposed-block bridge methods are DTO adapters. CXX-free
`rustaxa-consensus::pbft_chain::PbftChainService` owns storage lifetime, restoration/default initialization, its lock,
head transitions, validation, and block lookup; the bridge chain-state struct and lock are deleted. Cross-domain PBFT
operations still borrow both native sibling guards. CXX-free
`rustaxa-consensus::pbft_vote_runtime::PbftVerifiedVotesService` now also owns verified-vote storage lifetime,
atomic restoration, and the shared admission-runtime mutex. The bridge owns neither the verified-vote runtime nor its
lock; it temporarily borrows the native guard for FinalChain, leader-selection, finalization, and DTO/effect
composition. Native `rustaxa-consensus::pbft_service::PbftService` now owns slashing configuration validation, coherent
restoration of every storage-backed PBFT sibling from one handle, complete root publication, and bootstrap readiness.
`BridgePbftService` is a one-field
CXX adapter and retains no sibling state, storage handle, mutex, or readiness flag; durable access comes from the native
sibling owner responsible for each operation. Native `rustaxa-consensus::slashing::SlashingProofService` owns slashing planner
configuration, duplicate-cache state, and its mutex; the slashing bridge now performs only DTO/status conversion around
task-oriented plan/report calls. Native `rustaxa-consensus::pbft_readiness::PbftServiceReadiness` also owns the
independent monotonic PBFT and pillar-bootstrap readiness atomics plus their acquire/release publication contracts; the
native application root retains those capabilities. Native
`rustaxa-consensus::pbft_manager::PbftManagerService` now owns the manager mutex and complete runtime/session container;
the native root retains the service and the bridge exposes only a short-lived native guard to DTO/effect adapters.
The native root also requires verified-vote, slashing, and pillar siblings by construction. C++ clients retain
null-service and pillar-readiness checks, but no longer probe for capabilities that cannot be absent from a published
service.
Native `rustaxa-consensus::pillar_chain_service::PillarChainService` now owns pillar storage and restoration,
`PillarVotes`, the canonical anchor snapshot, both preparation registries and finalization token sequence, the outer
serialization mutex, and bootstrap readiness. Native pillar-vote task methods own admission, relevance, weighted
bundles, payload/network lookup, and finalization preparation/acknowledgement. The temporary bridge guard and raw
`pillar_state` accessor are deleted; every FinalChain-composed path prepares natively, performs the external query
without a guard, and enters a generation-bound native apply.
The service also exposes native task APIs for current-data publication, own-vote persistence, startup bootstrap,
current-anchor decisions, consensus threshold, block creation/linkage planning, and latest-finalized lookup; the bridge
only maps those results to CXX carriers. Pillar protocol/state tests are native, while the bridge retains only FFI
conversion and FinalChain-unwrapping coverage. PBFT root restoration, shared-owner, failure, and readiness behavior is
native; remaining bridge orchestration/conversion tests and the DAG/transaction owner remain in this workstream.
The native pillar mutex guard, mutable state, state snapshot, and snapshot decoder are crate-private and no longer
re-exported. Bridge tests use public task behavior rather than pointer, generation, or token introspection; snapshot
relationship characterization now lives beside the native decoder.
Native `rustaxa-consensus::sortition::SortitionService` now owns restored sortition state, its serialization mutex,
and poison policy. The DAG/transaction bridge root contains this service as a required capability, so optional
sortition state, capability probes, and unavailable-domain branches are gone. A temporary native guard preserves the
DAG-then-sortition order for coupled cursor revalidation until the remaining DAG runtime moves native.
Native `rustaxa-consensus::transaction_packing_service::TransactionPackingService` now owns the complete transient
proposal-packing protocol: its mutex and poison policy, compatibility/DAG owner identity, canonical candidate/RLP
snapshot, shard cursor, planner, pending-estimate ordering, selected output, stop state, and selective abort. The
native `rustaxa-consensus::transaction_service::TransactionService` now owns that packing service together with the
complete transaction queue, sidecar/count/gas cache, gas oracle, proposal gas limit, durable storage handle, drop
observation, restoration, serialization mutex, and stable poison policy. Production publication follows successful
count/config/history restoration; the bridge borrows a short-lived native guard only for FFI-shaped task adapters.
The bridge snapshots queue/cache facts under the native transaction lock, releases every lock for external EVM work,
then applies typed demotion/cache effects and transfers selected payloads under the established DAG-then-transaction
order. Shared DAG/transaction batch composition and DAG state remain bridge-owned until the wider application root
moves native. The temporarily public native state/guard are explicit bridge escape-hatch debt and may not cross CXX or
external callbacks.

### 3. Collapse configuration topology

- Define one Rust-enabled production feature bundle.
- Delete chain-only, pillar-only, transaction-only, and similar partial-service production factories after their tests
  use native fixtures.
- The pillar-only compatibility factory and Rust-mode manager compatibility constructor are deleted; pillar C++ tests
  now inject the same full PBFT application service used by production, while Rust unit tests use a private test helper.
- The storage-free gas-pricer partial factory and shim-owned compatibility service are deleted. Rust production uses
  the App-owned transaction service; native oracle tests own deterministic percentile/history behavior, and the
  untouched standalone C++ test remains reference-only.
- The transaction-only DAG/transaction partial factory is deleted. Every Rust-mode `TransactionManager` constructor now
  restores the same fully composed DAG/transaction/sortition service shape used by production; native bridge tests no
  longer preserve unavailable-domain behavior for a topology the application does not support.
- The PBFT-chain-only factory and Rust-mode `PbftChain(DbStorage)` constructor are deleted. The retained facade accepts
  only the full App-owned PBFT service, and C++/CXX tests use that same composition. No production or test-visible
  partial-service factory remains.
- Remove redundant module flags and CMake dependency matrices in dependency order.
- Continue to compile untouched legacy implementations in the all-Rust-disabled reference configuration.

Completion condition: Rust application construction has one production composition path and no compatibility-only
capability matrix.

### 4. Retire facades with no production authority

Start with surfaces whose audit already says they own no production state:

1. ~~`rewards_stats_shim` and `BridgeRewardsStatsRuntime`~~ — retired by the first `CRW-14` contraction slice
2. ~~`proposed_blocks_shim`~~ — retired after PBFT manager callers moved to the native service
3. ~~`verified_votes_shim`~~ — retired after VoteManager moved every Rust-mode operation to the shared native service
4. ~~`sortition_params_manager_shim`~~ — retired after native `SortitionService` ownership landed
5. `gas_pricer_shim`
6. `pbft_chain_shim`, after its named network/RPC readers use narrow APIs

For each facade:

- migrate named production clients;
- move or delete compatibility-only tests;
- delete its shim directory, CXX declarations, carriers, module flag, and construction path together;
- prove the pure-C++ build still selects the original implementation where required.

### 5. Replace legacy object materialization

- Inventory every boundary returning or accepting legacy consensus object graphs.
- Classify each as transport, executor, public query, event/plugin, or removable internal materialization.
- Replace internal crossings with canonical RLP, opaque IDs, borrowed Rust views, or operation-specific DTOs.
- Move JSON and RPC formatting behind the query API instead of retaining manager classes for formatting.
- Eliminate mirrored scalar state, sidecar caches, compatibility mutexes, and fingerprint/revalidation protocols once
  the corresponding C++ objects no longer survive across calls.

Completion condition: internal C++ code cannot obtain mutable consensus-domain object graphs from Rust-owned services.

### 6. Contract PBFT and vote shims

- Route internal PBFT, vote, proposed-block, verified-vote, pillar, and chain consumers through the native PBFT
  application service.
- Split the remaining C++ surface into explicit adapters: lifecycle/timer, transport, signer, VDF, EVM/FinalChain
  executor, and public query/materialization.
- Delete manager-shaped methods after their last internal caller migrates.
- Activate `CRW-N01` when transport contraction reaches the point where network ingress/egress blocks further deletion.

Completion condition: `pbft_manager_shim` and `vote_manager_shim` are leaf executor adapters rather than alternate
application runtimes; completing `CRW-N01` makes them deletable or replaces them with a transport adapter.

### 7. Contract DAG and transaction shims

- Move worker-neutral orchestration behind the native DAG/transaction service.
- Keep C++ only for explicitly retained thread lifecycle, network execution, signing, VDF, EVM gas execution, and public
  submission/materialization.
- Replace manager-to-manager calls with service operations and typed leaf effects.
- Delete `dag_manager_shim`, `transaction_manager_shim`, and `dag_block_proposer_shim` when their remaining clients use
  application APIs.

### 8. Reduce storage to bootstrap/admin/query boundaries

- Separate application storage ownership from the public `DbStorage` compatibility class.
- Migrate remaining production constructors to native storage/application factories.
- Replace seven storage query-family handles with the query API or native test fixtures.
- Keep `BridgeStorageBatch` only if a named external compatibility client still requires the legacy `DbStorage::Batch`
  lifecycle; otherwise delete it with the storage facade.
- Keep storage-conformance helpers explicitly test-only and guard-confined.

### 9. Contract FinalChain and execution

- Separate public query methods from execution methods and native FinalChain ownership.
- Remove consensus callers of the broad `BridgeFinalChain` facade.
- Keep a narrow concrete external-EVM executor while the ready `CRW-E01` item moves orchestration and canonical
  payload/result ownership into Rust.
- If the retained C++ `StateAPI` contract still forces broad session/report/materialization plumbing, prepare the
  `CRW-E01` design and parity plan rather than adding another compatibility layer.

### 10. Final deletion and documentation closeout

- Delete newly obsolete shims, bridge modules, flags, carriers, constructors, tests, and audit rows in the same slices as
  their last callers.
- Require the live inventory, measured budgets, tracker, and `PLAN.md` to agree.
- Run the required validation and upstream-owned-file checks.
- Delete this plan when the reduction queue is complete; retain only strategic boundaries in `PLAN.md`, the live
  mechanical inventory, and any genuinely remaining queue items in the tracker.

## Slice Rules

Select work only from the tracker’s **Remaining Consensus Work Queue**. A slice must delete a complete ownership or
compatibility family, move application behavior out of the bridge, or unblock a named downstream deletion. Merely
renaming exports, combining DTOs, or reclassifying retained code is not sufficient.

Every slice must report:

- production callers migrated;
- lines, functions, carriers, handles, shims, flags, and compatibility constructors removed;
- native behavior/parity tests that replace deleted compatibility tests;
- retained boundary and its named client;
- exact Tier 1/Tier 2/Tier 3 validation;
- upstream-owned C++ diff evidence.

## Documentation Ownership

- `PLAN.md`: stable rewrite strategy and accepted external boundaries.
- `doc/consensus_rewrite_tracker.md`: only execution queue, status, dependencies, and concise completion evidence.
- `doc/consensus_consolidation_plan.md`: this active design and sequencing document; no implementation diary.
- `doc/consensus_bridge_shim_audit.md`: guard-parsed live inventory only; no slice history.
- `doc/rewrite_validation_strategy.md`: reusable validation policy.

Do not create another consensus gap, compatibility, cleanup, or slice tracker.
