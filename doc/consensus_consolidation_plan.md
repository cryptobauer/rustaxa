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
lock; it temporarily borrows the native guard for FinalChain, finalization, and DTO/effect
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
native. Seven duplicated bridge session/startup/finalization transcripts are
deleted because native owner tests and `rust_consensus_tests` cover both
protocol behavior and the CXX boundary. Focused bootstrap gating, period-data
queue conversion, invalid-stage non-publication, ineligible-sleep mapping,
queue-drain mapping, and unknown-enum tests remain at the bridge; remaining
cross-domain orchestration/conversion tests stay in this workstream. Five
additional happy-path readiness, block-validation, candidate-admission, and
leader-selection planner transcripts are deleted from the bridge because the
native PBFT manager suite owns stronger behavior; the bridge retains only the
compact normal-path carrier sentinel and unknown-status error-mapping case for
that planner family.
Native `PbftService` now also owns the complete period-state cleanup task across
verified votes and proposed blocks: fixed sibling lock order, successor
validation, one Rust proposal-deletion batch, commit-before-memory publication,
typed counts, and retry-safe rejection. Its behavioral and failure-injection
tests are native. The later period-commit contraction removes the temporary
C++ cleanup action and its result conversion entirely: the bridge now exposes
only the final fallible native commit snapshot.
Native `PbftService` also owns the complete leader-selection task across
verified votes, proposed blocks, and finalized-chain membership. It prepares
deterministically ordered owned candidates under the manager-before-siblings
lock order, using the manager serialization domain to exclude finalized
membership storage writes while sibling locks protect live state. It
fingerprints the complete V1 snapshot, releases every native lock for retained
C++ block validation, then revalidates the exact snapshot and external report
set before atomically publishing planner-approved validity. Workflow,
multi-publication, and serialization tests are native; the bridge retains one
focused end-to-end prepare/finish and exhaustive status/payload conversion test
around the unchanged CXX boundary.
The remaining bridge-local reward-vote finalization transcripts are also
deleted. Native vote-runtime tests own ordered reward selection, durable
generation-bound cursor publication, idempotence, conflict rejection, and
restart behavior; native finalization tests own reset-bundle persistence,
authoritative stale-row deletion, serialization against in-flight admission,
and rejected write-set statuses. The bridge retains the unchanged production
DTO adapters plus one compact identity/payload/status conversion sentinel, but
no parallel reward-reset storage executor or protocol test suite.
Native `PbftVerifiedVotesService` now owns that production reward-finalization
task as well. It takes the single vote-runtime lock before deriving the exact
cert bundle, applies the storage-owned reset batch while admission is excluded,
and publishes the generation-bound live cursor only after a successful durable
result. It also owns coherent cursor/payload snapshots, ordered reward
selection, reset-stage preparation, combined-batch cursor acknowledgement, and
restart restoration. The unchanged CXX calls now perform only carrier
conversion for the temporary C++ finalization and vote materializers.
The native pillar mutex guard, mutable state, state snapshot, and snapshot decoder are crate-private and no longer
re-exported. Bridge tests use public task behavior rather than pointer, generation, or token introspection; snapshot
relationship characterization now lives beside the native decoder.
Native `rustaxa-consensus::sortition::SortitionService` now owns restored sortition state, its serialization mutex,
and poison policy. The DAG/transaction bridge root contains this service as a required capability, so optional
sortition state, capability probes, and unavailable-domain branches are gone. A temporary native guard preserves the
DAG-then-sortition order for coupled cursor revalidation.
Native `DagTransactionService` now also owns the finalized-period sortition
preview/commit task: canonical efficiency-count normalization, preview,
expected-change validation, clone-before-publish mutation, and the
lock-coherent threshold/history snapshot. The bridge no longer owns sortition
protocol behavior or exposes production guard access; it forwards native
requests and projects the result into the PBFT executor report. The two
bridge-only change carriers and the helper behavioral test are deleted. PBFT
chain-head/period-data preparation now also runs as one native PBFT-manager
task while the manager serialization domain is held. Native consensus rejects
caller-owned sortition stages, validates the chain successor and non-empty
size, decodes canonical period data, checks pivot/null-anchor consistency,
appends the exact storage stage, and retains the native DAG commit request
without bridge-side reconstruction. The bridge keeps only an
operation-specific delegate plus the finalization executor's conversion and
error mapping. Replacing the expected-change/at-most-once contract with a
portable full-state preview fingerprint remains explicit CRW-12 debt.
The post-storage sortition commit workflow is native as well. The lock-held
PBFT manager task owns current-step, cursor, and action validation; retained
plan/request consistency; manager-before-sortition commit; stable fatal error
projection; complete live-fact construction and validation; request
consumption; and runtime reporting. The bridge retains only an
operation-specific call, domain-step conversion, and the generic executor
drain shared with still-unmigrated finalization actions. Superseded unchanged,
drift, and stale-cursor bridge transcripts moved to native coverage; one
changed-commit CXX sentinel remains.
Reward-vote reset advancement now follows the same native cursor pattern.
`PbftManagerGuard` owns session/action/cursor checks, exact nonzero manager and
storage reset-generation provenance, the complete reward live report,
validation, native verified-vote cursor publication, and native runtime
reporting. The bridge retains only the reportless manager call, domain-step
mapping, generic drain, and boundary cleanup. The former C++ `VoteManager`
reset-report/cursor relay and its CXX carriers/export are deleted. The manager
executor state no longer exposes the internal reset generation to C++.
It also no longer exports bridge-only drain counts, storage status, or
per-action completion telemetry; C++ receives only state it actively consumes.
The drain adapter derives that retained state directly from the native manager
and next step rather than storing an intermediate bridge snapshot/status copy.
Finalized-transaction advancement now follows the same native cursor pattern.
The PBFT manager reads canonical `PeriodData` from its accepted plan, composes
directly with the native DAG/transaction owner under manager-before-transaction
lock order, applies storage/sidecar/queue/purge mutation, validates the accepted
count, and reports its runtime action without a C++ mutation relay. C++ supplies
only the recently-finalized retention window and account nonce facts from the
retained EVM query boundary. The per-transaction CXX payload carrier, mutation
report carrier, C++ hash/RLP inspection loop, and manager report relay are
deleted. The stable non-PBFT `TransactionManager` API remains a compatibility
client and sends one opaque canonical transaction-list RLP because partially
populated legacy `PeriodData` objects cannot always serialize a certificate
bundle; Rust still derives every transaction identity and owns all mutation.
The remaining generic owned-finalization drain is now native
`PbftManagerGuard` behavior. Under the manager serialization domain it drains
PBFT-chain publication, anchor-cache clearing, dynamic-lambda persistence and
live publication, and executed-status persistence/live publication, stopping
unchanged at DAG, transaction, sortition, vote, FinalChain/EVM, pillar,
period-advance, or network actions. Chain updates are projected and validated
before live publication; storage precedes the corresponding manager snapshot.
The bridge retains one argument-free delegate, next-step/snapshot compatibility
projection, and terminal session cleanup. The duplicate bridge drain state,
stage construction, behavioral loop, and behavioral transcripts are deleted.
Finalization executor start and resume now join that native ownership boundary.
`PbftService` holds the manager serialization domain while Rust clears stale
state, authenticates a resumable reward-reset generation, derives durable
resume state, installs the fresh cursor, prepares sortition through the native
DAG/transaction owner, applies primary storage, reports the result, drains the
first manager-owned action sequence, captures the manager snapshot, and clears
terminal or failed sessions. The bridge converts the start request and returned
boundary only; its duplicate workflow and behavioral transcripts are deleted,
with one CXX conversion/integration sentinel retained.
Every post-start executor advancement now uses the same native application
boundary. `PbftService` owns cursor/action validation, typed DAG, transaction,
sortition, reward-reset, FinalChain, pillar, and period report construction,
subsystem composition, subsequent owned-action draining, terminal/error
cleanup, and snapshot capture under one manager lock. External failure reports
also terminate natively. The eight CXX functions remain leaf-result adapters,
but their FinalChain, pillar, and advance-period entries no longer accept
manager-shaped report carriers: C++ passes only the observed FinalChain height
and pillar request period, while Rust derives blocks-per-year, processed period,
and the post-advance manager period from retained native state. The bridge no
longer owns a cursor protocol, generic mutation report, drain continuation,
cleanup policy, or behavioral test suite.
Finalized DAG-order advancement is now a composed native task rather than a C++
manager round trip. Rust validates the PBFT cursor before mutation, derives the
anchor, period, and ordered hashes from the retained plan, commits through
`DagTransactionService`, validates its native count, and advances the executor.
The PBFT-specific `DagManager` mutation/report facade and CXX `finalized_count`
relay are deleted; C++ retains only an explicit adapter that refreshes public
counter mirrors and evicts expired hashes from its temporary seen-block cache.
Reward-vote reset preparation is fully native inside PBFT executor startup.
`PbftService` passes its existing verified-vote sibling into the manager task;
fresh start rejects caller-owned reward stages, derives the exact certified
identity from the accepted write intent, prepares the canonical bundle under
manager-before-verified-votes lock order, and retains that vote guard through
sortition preparation and the atomic primary storage commit so concurrent
admission cannot invalidate the durable bundle. Preparation failures use the
executor's normal cleanup path. The PBFT shim, CXX preparation export, bridge conversions, and
reward-specific CXX storage-stage fields are deleted.
Same-process resume now also repairs the narrow post-primary-commit reward
publication window. The manager retains only a reset generation authenticated
against its shared storage owner, prepends the reward publication action only
when both reset intents are present, and lets the verified-vote service
revalidate the exact durable cert cursor and bundle before changing live state.
The C++ dispatcher permits that native lock-owning action during resume without
broadening replay for sortition, DAG, transaction, timer, or period mutations.
Concrete sortition parameter changes now have the same bounded recovery
contract: only a retained process-local preview whose exact change RLP is
present in durable storage can be prepended during same-process resume.
No-change previews remain non-replayable because they can advance hidden
efficiency-window state without a durable cursor. The C++ dispatcher releases
its DAG/transaction locks for this native manager-to-sortition task.
All seven remaining external finalization actions now share one CXX advancement
entrypoint; Rust decodes the action and routes to the existing typed native
leaf, while action-specific payloads remain ignored outside their matching
leaf. Six duplicated per-action exports and their bridge/shim wrappers are
deleted.
PBFT synced-period admission is now a native application task as well.
`PbftService` gates session creation on bootstrap readiness, while
`PbftManagerService` owns cursor allocation/replacement, check/report
validation, terminal cleanup, abort, and storage-backed sync egress loading.
The bridge module retains only CXX carrier/status conversion plus focused
cert-vote and end-to-end sync projection sentinels; its broad RocksDB-backed
session and egress behavioral tests are replaced by native service coverage.
The PBFT manager bridge suite likewise no longer repeats daemon ineligible
sleep, proposal ordering/build, broadcast-counter, or deadline-wait behavior.
Those rules remain covered by native manager tests; bridge coverage stays
focused on CXX projection, persistence, and external-executor boundaries.
Executed-block reset, next-voted status, and manager cursor-field persistence
transcripts now follow the same ownership rule. Native storage/runtime tests
own commit-before-publication, accepted-field, and rejection behavior; the
bridge retains one combined lifecycle-transition storage sentinel plus one
compact adapter status/error-mapping test instead of three parallel protocol
tests.
Broadcast-counter mutation and cached-anchor membership are likewise tested
only beside the native manager runtime. Their bridge RocksDB fixtures are
deleted because the live CXX wrappers are direct scalar/hash projections with
no conversion or error-lifetime policy of their own.
Lifecycle network-step presence validation now lives beside the native manager
cursor as well. The bridge retains only a compact unknown-kind rejection and
network-step projection/snapshot-preservation sentinel for its transition
request conversion.
Missing Cacti dynamic-lambda startup rejection and storage non-mutation
coverage now live with the native manager restore function rather than in a
bridge-owned RocksDB fixture.
PBFT period-state cleanup is no longer a separate C++ executor action.
`PbftService` validates committed-reset provenance and owns the final
cleanup-plus-period commit under manager, verified-vote, and proposed-block
guards. The bridge exposes only the fallible final snapshot commit; the cleanup
result carrier, bridge module, and shim-side count validation are deleted.
The remaining direct `VoteManager::resetRewardVotes` compatibility method no
longer materializes a broad finalization storage plan or exposes two
PBFT-finalization helper methods. It builds the existing narrow identity
request and delegates the complete reset task to the native verified-vote
owner; the legacy batch parameter remains inert for API compatibility.
Native `rustaxa-consensus::transaction_packing_service::TransactionPackingService` now owns the complete transient
proposal-packing protocol: its mutex and poison policy, compatibility/DAG owner identity, canonical candidate/RLP
snapshot, shard cursor, planner, pending-estimate ordering, selected output, stop state, and selective abort. The
native `rustaxa-consensus::transaction_service::TransactionService` now owns that packing service together with the
complete transaction queue, sidecar/count/gas cache, gas oracle, proposal gas limit, durable storage handle, drop
observation, restoration, serialization mutex, and stable poison policy. Production publication follows successful
count/config/history restoration. Lock-owning native tasks cover reads, admission, packing, cache/queue mutation,
finalization, recovery, and DAG transaction persistence; no production transaction guard escapes into the bridge.
The bridge supplies owned facts, executes retained FinalChain/EVM leaves only while native locks are released, and
converts native reports. Its former RocksDB-backed behavioral fixture is deleted, leaving one focused status-mapping
ABI test. Native publication preserves the established DAG-then-transaction
order. Native `rustaxa-consensus::dag_service::DagService` owns DAG graph/storage state,
proposer/verifier/add-block cursors, retry state, restoration, initial proposal-period mapping, serialization mutex,
and poison policy. Native `rustaxa-consensus::dag_transaction_service::DagTransactionService` is the complete
three-service application root: it constructs and restores transaction, DAG, and sortition siblings from one storage
owner, publishes only after all three succeed, and owns access to the canonical lock domains plus composed
DAG-then-transaction acquisition. It also owns the complete add-block cursor, canonical transaction validation,
single shared DAG/transaction persistence batch, post-commit live publication, finalized-order storage application,
and sibling transaction-sidecar cleanup. Its duplicated RocksDB-backed bridge add-block behavioral suite is deleted;
native tests cover durable commit/restart, failure-before-publication, cursor concurrency and retry safety, compatibility
object identity, supplied-transaction persistence, finalized-nonce filtering, and nonce-fact validation.
The native DAG application root also owns the proposer transaction-pack task:
it validates and advances the DAG cursor, snapshots and applies transaction
queue/cache effects, retains owner-bound estimate cursors across the unlocked
EVM interval, and cleans both matching cursors on failure. The bridge
retains conversion and external-executor wiring but no proposer-pack protocol
test; native tests own estimate/finalize, cache reuse, cursor cleanup, poison,
compatibility-owner, and malformed-payload behavior. The bridge
also routes the complete verifier task through the native root: Rust owns
transaction-source resolution, authorization/VDF cursor revalidation, native
VDF proof verification, and gas decisions, while C++ retains transaction
materialization, FinalChain lookup, and EVM estimation as unlocked leaves. The
duplicated bridge verifier-transaction behavioral suite is deleted; native
tests own all-supplied completion, canonical queue/sidecar resolution, missing
transaction rejection, finalized-nonce validation, and stale period/stage
cursor behavior. The
native root also owns the remaining DAG manager query, storage-lookup,
graph-status, and non-finalized-sync tasks and returns only owned domain
snapshots. Superseded direct proposer/verifier/query adapters and their bridge
behavioral tests are deleted. The DAG service, mutable state, guard, and root
DAG lock accessors are crate-private. Native transaction services also own the
complete read-task family, including queue/sidecar/storage views, status and gas
facts, and gas-estimation planning; those paths no longer borrow a bridge
transaction guard. Native transaction services additionally own compatibility
packing, gas-oracle/cache mutation, recently-finalized initialization,
non-finalized durable removal, and finalized-block queue expiry. Standalone DAG
transaction saving is also a lock-owning native task: the bridge converts owned
facts and the typed report but cannot borrow transaction state or a guard. Native
transaction services also own finalized-status persistence, recently-finalized
sidecar publication, queue-known/erasure mutation, and periodic account-nonce
purge as one lock-owning task. Native transaction services also own validated and public admission
as lock-owning tasks: C++ supplies retained FinalChain/account facts, while the
bridge performs carrier conversion and no longer plans or mutates admission
state.

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
5. ~~`gas_pricer_shim`~~ — retired after App, RPC/GraphQL, metrics, finalization, and slashing callers moved to
   `TransactionManager`
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
