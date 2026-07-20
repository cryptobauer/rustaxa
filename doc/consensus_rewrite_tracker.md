# Consensus Rewrite Tracker

This tracker expands the consensus section in `PLAN.md`. Keep it current as consensus code moves from C++ into Rust.
Consensus validation policy is defined in `doc/rewrite_validation_strategy.md`; subsystem-specific minimum coverage and
unresolved gaps are tracked in the **Validation Matrix** and **Current Open Items** below.

## Status Legend

| Status | Meaning |
| --- | --- |
| `not-started` | No Rust port exists yet. |
| `rust-domain` | Pure Rust domain model or deterministic algorithm exists without C++ runtime wiring. |
| `rust-backed` | C++ public API is backed by Rust for this behavior. |
| `partial` | Some Rust-backed behavior exists, but documented gaps remain before the API can replace C++ semantics. |
| `shim-stubbed` | Rust mode has an explicit temporary default/no-op. Must not delegate to legacy C++. |
| `cpp-owned` | Intentionally left in C++ for now. |
| `deferred` | Out of current consensus rewrite scope. |

## Hard Rules

- Do not delegate Rust shim behavior back to legacy C++ implementation methods.
- Keep public C++ APIs stable unless a task explicitly changes the API.
- Main-only files referenced from upstream-owned C++ files must be included only behind `RUSTAXA_ENABLE=1` or a narrower Rustaxa feature guard.
- Treat DPoS eligibility and vote-count behavior as required consensus functionality. Temporary defaults must stay visible in this tracker.
- Keep network callbacks, daemon threads, peer gossip, and full-node orchestration in C++ until the Rust domain services are stable.
- Logging is not a reason to keep consensus behavior in C++. Treat logs as boundary observability that may stay in a
  temporary C++ executor, be replaced by Rust-returned status/telemetry facts, or be cleaned up later. Ownership
  decisions should be based on state, protocol decisions, persistence, network effects, object materialization, and
  external execution dependencies instead.
- C++ tests may be disabled, removed, or retargeted only when they block an intentional move away from legacy C++ behavior,
  old object materialization, or shim scaffolding. The Rust module taking ownership must already have equivalent or
  stronger coverage, and CXX boundary parity must be covered by bridge-level Rust tests or focused Rust-enabled shim tests
  before the C++ test signal is dropped.
- New consensus rewrite APIs must be shaped for the upcoming application-owned arena/data pipeline even before the
  concrete pipeline API lands. Prefer canonical bytes, compact facts, ingress-payload-addressable enrichment, and typed
  protocol plans/effects over eager C++ object materialization.

## Planned Arena/Data Pipeline Direction

The parallel Rust network rewrite will land an application-owned arena pipeline before this consensus feature branch is
merged. The first ingress point is expected to accept latest-tarcap payload bytes from
`TaraxaCapability::interpretCapabilityPacket`, write them into an application-global ingress arena, and report only
whether ingestion succeeded. Network-level outcomes such as drop, disconnect, mark-malicious, gossip, or sync request
will be emitted later by pipeline stages as egress events; they are not part of the ingress success contract.

The planned initial CXX bridge entry point is:

```rust
pub fn ingest_network_packet(
    self: &mut BridgeNetwork,
    packet_type: u8,
    from_node: [u8; 64],
    data: Vec<u8>,
) -> Result<bool>;
```

This API is latest-tarcap-only. Its `bool` reports ingestion success only: `true` means the payload bytes were accepted
into the application arena/pipeline. Broken payload, protocol rejection, consensus rejection, peer action, gossip, sync,
drop, and disconnect outcomes are produced later by downstream pipeline stages and network egress events.

Consensus rewrite slices should assume this future direction:

- Pipeline communication is by payload reference id. A stage may wrap the ingress payload reference id with small
  route/source/priority metadata, but large derived data should live in ingress or enrichment arenas and be referenced by
  payload reference id.
- Each ingress event is handled by only one thread at a time. Pipeline stages should model ownership transfer of an event
  carrying a payload reference, not fanout over shared mutable ingress state.
- The intended stage sequence is network ingress -> prefilter -> dispatcher -> pipeline-specific ring buffers -> effect
  executors. `NetworkEvent`, prefilter decisions, dispatcher classification, and ring-buffer allocation belong in the
  network crate or a dedicated pipeline crate. The consensus crate should define only consensus event/effect types and
  opaque ingress payload references. Dispatchers outside consensus route accepted ingress events into typed consensus
  units, such as `PbftVoteEvent`, before those units enter the owning pipeline's ring buffer. These type names, variants,
  and payload shapes are provisional and should evolve as the first routed pipeline validates the design.
- Materialization should be delayed. Ingress-adjacent Rust code should inspect raw bytes, produce compact facts or
  enrichment records only when useful, and avoid constructing C++ `PbftVote`, `PbftBlock`, `DagBlock`, `PeriodData`, or
  `Transaction` objects unless a temporary compatibility executor requires them.
- Consensus domain identity remains hash/period/round/step/voter/level based. Ingress payload references are data-plane
  handles and should not become consensus identities.
- Intermediate modules between network ingress and consensus, such as prefilters or route classifiers, are expected.
  Consensus-facing APIs should be callable from those stages with raw bytes, ingress-payload-backed views, or compact
  facts instead of requiring network handler objects.

The arena-backed data plane should expose multiple logical pipelines rather than one monolithic consensus pipeline.
Today's C++ scheduler has three priority lanes, but consensus behavior is better represented as seven logical pipelines:

| Logical pipeline | Current ingress message kinds | Cross-pipeline effects to keep explicit |
| --- | --- | --- |
| Peer status and sync control | `StatusPacket` | Peer readiness, peer PBFT/DAG progress, PBFT sync start, DAG sync request, next-vote request |
| Transaction gossip and admission | `TransactionPacket` | Peer-known transaction marks, transaction admission, peer-order blocking for later DAG blocks from the same peer |
| DAG block gossip and admission | `DagBlockPacket` | DAG verification/admission, missing transaction or tip responses, DAG sync request, PBFT-sync-aware DAG acceptance |
| DAG sync | `GetDagSyncPacket`, `DagSyncPacket` | Missing DAG/transaction recovery, peer DAG synced state, possible PBFT sync restart on period mismatch |
| PBFT vote and round progress | `VotePacket`, `VotesBundlePacket`, `GetNextVotesBundlePacket` | Vote admission, known-vote marks, proposed-block sidecar admission, slashing proof submission, PBFT round/finalization progress, gossip |
| PBFT chain sync and finalized-period intake | `GetPbftSyncPacket`, `PbftSyncPacket`, `PbftBlocksBundlePacket` | Deep-sync traffic filtering, sync-peer validation, period-data queue admission, malicious-sync-peer reporting, PBFT sync completion, DAG sync request |
| Pillar vote and bundle handling | `PillarVotePacket`, `GetPillarVotesBundlePacket`, `PillarVotesBundlePacket` | Pillar-vote admission, known-vote marks, pillar block/vote facts used by PBFT period validation |

Future Rust ingress-adjacent APIs should keep these pipeline facts and effects separate. Cross-pipeline impact should be
represented as typed effects, such as request-sync, block-peer-order, mark-known, admit, gossip, report-malicious,
enqueue-period-data, or drive-PBFT-progress, rather than hidden mutation of another pipeline's state.

## Consensus Protocol Planner Model

Consensus business logic should be expressed as deterministic protocol planners over explicit state views. A planner
receives a consensus event or command, compact facts, config/time inputs, and borrowed state views, then returns a
protocol plan. The plan describes the protocol state transition implied by the input: validation outcome data, ordered
state/write intents, follow-up consensus events, and external effects.

The planner itself must stay side-effect-free. It should not own long-lived consensus data, write storage, send network
messages, spawn Tokio tasks, block on async I/O, or mutate another pipeline directly. Runtime workers, actors,
ring-buffer stages, Tokio tasks, and effect executors may schedule planners and apply returned plans at the boundary, but
consensus rules should not be hidden inside mailbox-local actor state or async task choreography.

The term "transition" means the consensus system's protocol state is transitioning, not that each input produces only a
small transformation. A single PBFT vote event can legitimately plan known-vote marking, vote admission, proposed-block
sidecar admission, threshold updates, slashing proof emission, gossip, and PBFT progress triggers as one protocol plan
when those are the deterministic consequences of the event and current state view.

## Native Rust Consensus Gap Closeout

The former native Rust consensus gap plan is complete and folded into this tracker plus `PLAN.md`. The closeout boundary
is:

- Rust owns consensus rules, durable consensus state, restart normalization, storage/query selection, canonical payload
  retention, validation decisions, scheduler/timer policy, ordered side-effect planning, and typed executor-result
  validation.
- Remaining C++ is accepted only for public API adapters, app-host lifecycle mechanics, OS threads, condition variables,
  actual sleeps, signing execution, async VDF execution, network/tarcap transport and gossip, EVM/StateAPI execution,
  state DB mutation, receipts/log blooms, arbitrary contract calls, and explicitly classified temporary materialization
  edges.
- C++ objects such as `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, pillar objects, and reward/stat
  carriers may still be materialized at public/test/network/EVM/executor boundaries, but they are no longer accepted as
  consensus decision authority.

Completed closeout slices:

- FinalChain/DPoS fact ports moved consensus-facing facts to typed Rust ports, with EVM/state execution left as the
  explicit boundary.
- TransactionManager production paths moved runtime/query authority, canonical payload inspection, finalized-status
  mutation, public admission reports, and event/log intent selection to Rust.
- DAG manager production decisions moved to Rust graph/runtime state, Rust storage, typed transaction ports, and
  Rust-owned finalization/add/sync plans.
- DAG proposer lifecycle and scheduler policy moved to Rust sessions and worker-command planners while C++ executes VDF,
  signing, add-block, thread, timer, and network effects.
- Vote, slashing, and pillar decisions consume Rust-retained payloads, compact facts, and typed plans; remaining C++
  paths are signing, transaction insertion, network, public, or temporary sidecar executors.
- Rewards stats carrier authority moved to Rust-owned encoding, interval cache state, storage/reload/clear behavior, and
  finalization integration.
- Typed consensus storage ports replaced broad production storage shim/batch authority with task-specific Rust storage
  ports or runtimes.
- Obsolete public object/materialization adapters were removed where Rust DTOs were sufficient; remaining materialization
  is classified by boundary.
- PBFT next-step sleeps, ineligible-wallet polling, startup finalization waits, eligible-wallet readiness waits, and DAG
  proposer worker retry delay are Rust-planned. Remaining lifecycle shell code is accepted host/executor mechanics.

Future consensus cleanup should be deletion-oriented: remove adapters, broad CXX bridge DTOs, sidecar maps, and shim
helpers once their public, network, EVM, test, or app-host consumers move. New unclassified production consensus fallback
to legacy C++ is a rewrite blocker.

FinalChain facade closeout: the Rust-mode `final_chain::FinalChain` overlay is self-contained and no longer imports,
compiles, or exposes `FinalChainOld`. Rust production builds exclude the untouched legacy `final_chain.cpp`; pure-C++
reference builds retain it. The remaining C++ `ExternalEvmStateApiClient` and `StateAPI` work is the classified external
EVM/state executor boundary, not legacy FinalChain delegation.

External facade closeout: `ConsensusNetworkApi`, `ConsensusExecutionApi`, and `ConsensusQueryApi` are the dedicated
network/tarcap, external-EVM/StateAPI, and public-read boundaries. They expose typed operations and DTOs without exposing
consensus managers or mutable internal state. Their remaining C++ code is transport execution, EVM/state execution,
public formatting/materialization, or explicitly tracked compatibility glue. Strategic ownership and invariants live in
`PLAN.md`; exact APIs live with the Rust facade modules and bridge tests.

## PBFT Manager Breakthrough Boundary

The intended PBFT manager end state is a Rust-owned protocol runtime behind the existing C++ compatibility surface. The
Rust runtime should own long-lived manager protocol state and expose command/event entry points that return ordered
protocol plans. The C++ overlay should supply facts, execute effects that still belong outside consensus, and report
effect results back before Rust advances the runtime cursor.

Do not include these boundaries in the PBFT manager breakthrough slice:

- Network/tarcap internals: peer transport, packet wrapping, gossip fanout, send policy, peer-known marking, disconnect
  and report mechanics, and queue ownership stay outside the consensus manager migration. Rust should return typed
  network effects for an executor instead of owning the transport.
- EVM/FinalChain execution: transaction execution, gas execution, receipt/log bloom construction, external contract
  execution, and state transition execution stay behind the existing execution boundary. Rust PBFT code may plan
  finalization and validate execution facts, but not move EVM execution into the PBFT manager.
- Live compatibility objects: temporary C++ materialization of `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`,
  `Transaction`, pillar sidecars, and public API return values may remain until callers no longer require those types.
- Node lifecycle mechanics: daemon threads, sleeps, timers, startup/shutdown wiring, key-manager signing, and event
  emission mechanics may remain as executor responsibilities around the Rust protocol runtime.

Everything else in `PbftManager` is a Rust ownership candidate: period/round/step state, daemon-tick control flow,
proposal/certify/finish-polling transitions, sync-period admission, proposed-block selection and cleanup planning,
vote/reward/cert selection, finalization planning and bounded resume, dynamic-lambda decisions, DAG/transaction cleanup
planning, PBFT-chain head advancement plans, storage/write intents, cross-pipeline effects, and ordered side-effect
contracts. Logging is not a boundary; temporary C++ logs should be treated as executor reporting from Rust statuses or
telemetry.

## Current Rust Starting Point

| Area | Rust location | Status | Notes |
| --- | --- | --- | --- |
| FinalChain read/index helper | `rust/crates/rustaxa-consensus/src/final_chain.rs` | `rust-backed` for selected FinalChain reads | Exists because FinalChain work started before consensus. It is not a PBFT/DAG port yet. |
| Consensus crate root | `rust/crates/rustaxa-consensus/src/lib.rs` | `rust-owned` for DAG graph | Exports the native DAG graph model owned by `DagManagerState`; standalone CXX graph wrapping is retired. |
| Consensus event pipeline scaffold | `rust/crates/rustaxa-consensus/src/consensus_pipeline.rs` | scaffold | Defines only consensus-layer pipeline vocabulary: logical pipeline kinds, opaque ingress payload references, event origins, typed consensus events, the first `PbftVoteEvent`, typed cross-pipeline effects, and the provisional `ConsensusPlan` envelope returned by side-effect-free protocol planners. These names, variants, and payload fields are provisional and open to change until production routing proves the boundaries. `NetworkEvent`, prefilter decisions, dispatcher classification, and ring-buffer allocation are intentionally excluded and should live in the network crate or a dedicated pipeline crate. This scaffold does not decode payload bytes, mutate consensus state, or route production handlers yet. |
| PBFT vote ingress/progress planners | `rust/crates/rustaxa-consensus/src/pbft_vote_ingress.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_admission.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_event.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_progress.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_pipeline.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_runtime.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_ingress.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_admission.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_event.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_progress.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_pipeline.rs`, `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `partial` | Side-effect-free protocol planners for PBFT vote ingress and vote pipeline decisions plus a Rust-owned admission runtime for Rust-mode `VoteManager::addVerifiedVote`. Rust now owns the deterministic single-vote and bundle ingress gates for relevance, period/round/step windows, proposed-vote bundle rejection, bundle identity consistency, and PBFT/next-vote sync hints. The production shim opens admission from canonical vote RLP plus explicit validation facts, so Rust carries canonical validation output, explicit replay mutation facts, and the validation-calculated weight into compact progress facts, mutates the single Rust-owned `VerifiedVotes` runtime, retains weighted storage payloads and unweighted slashing evidence payloads, owns PBFT `2t+1` threshold cache lookup/update, and commits any required progress rows before publishing, and returns one terminal executor report with typed peer-known, proposed-block sidecar, gossip, slashing, threshold, and PBFT-progress intents. Retained weighted payload views now back legacy snapshot, reward-vote materialization, and 2t+1 materialization APIs, and missing retained payloads for Rust-owned selected votes are invariant errors instead of partial results. The Rust runtime also builds PBFT reward-vote candidate facts from its own verified-vote metadata and resolves selected reward records in PBFT-block requested order. The Rust-mode `VoteManager` shim now exposes the peer-known, gossip, slashing, and PBFT-progress portions as a shim-owned admission report; latest-tarcap single-vote and bundle handlers execute those effects only after Rust admission accepts the vote, and Rust-mode bundle rebroadcast includes only accepted votes. The older admission session, pipeline session, standalone progress planners, weight-supplied constructor, and low-level verified-vote mutators remain compatibility/test scaffolding rather than production state-transition authority. Required extra-reward and current-round 2t+1 rows commit under the vote lock before the live mutation is published; persistence failure restores the bounded replay/round/payload checkpoint and exposes no executor effects. C++ still decodes network packets, supplies peer/live-sidecar facts, executes peer/network effects and proposed-block sidecar handling, and temporarily hosts guarded packet-handler ingress hooks until the network/tarcap pipeline overlay owns those routes. The latest-tarcap method signature changes are temporary hook debt, not the target pipeline API. |
| PBFT vote validation planner | `rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs`, `rust/crates/rustaxa-consensus/src/pbft_thresholds.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_validation.rs`, `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `partial` | Rust owns received-vote validation decisions, stable rejection statuses, replay-marker timing, canonical PBFT vote RLP inspection, signed/unsigned vote hash derivation, recovered voter identity, signature and VRF proof checks, Rust-computed received-vote weight, the PBFT sortition-threshold formula, runtime-owned replay protection, runtime-owned `2t+1` threshold lookup/current-period caching, and local proposer-sortition screening. The Rust-mode `VoteManager` shim routes `validateVote`, `addVerifiedVote`, `voteAlreadyValidated`, `getPbftTwoTPlusOne`, and `genAndValidateVrfSortition` through service-owned verified-vote methods instead of `VoteManagerOld`; `addVerifiedVote` collects typed Rust `BridgeFinalChain` DPoS/key facts and lets Rust validation-backed admission produce the authoritative weight, verified-vote mutation, replay mutation, threshold decisions, and post-mutation executor effects. The older standalone `BridgePbftVoteValidationRuntime` and `BridgeVerifiedVotes` CXX handles have been deleted; production and bridge coverage route validation replay/threshold state through `BridgePbftService`. C++ still supplies proposer sortition object materialization and temporary live sidecar mutation, but consensus DPoS/VRF facts come from Rust FinalChain ports. It only calls `PbftVote::calculateWeight` to hydrate the legacy sidecar after checking the hydrated sidecar weight matches Rust's calculated weight. |
| PBFT vote generation planner | `rust/crates/rustaxa-consensus/src/pbft_vote_generation.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_generation.rs` | `partial` | Rust owns side-effect-free local PBFT vote byte generation: it validates vote type/step compatibility, derives the expected voter/VRF public key from ephemeral wallet secrets, creates the legacy PBFT VRF proof/output, signs `PbftVote::sha3(false)`, returns canonical signed or weighted `PbftVote` RLP, and reports zero-stake, zero-total-DPoS, and zero-weight outcomes as stable statuses. The Rust-mode `VoteManager` shim now materializes `generateVote` and `generateVoteWithWeight` sidecars directly from Rust-generated RLP, hydrates the temporary C++ VRF output cache through local VRF verification, and checks Rust hashes, signing hash, recovered identity, VRF proof, weight, and exact RLP bytes before returning the sidecar. Locally generated own-vote persistence therefore stores Rust-generated weighted vote bytes through Rust storage. C++ still owns the temporary `PbftVote` sidecar type and PBFT manager/network orchestration; PBFT vote DPoS and VRF facts are sourced through Rust FinalChain/key ports. Logging around these calls is temporary observability, not an ownership blocker. |
| PBFT vote payload builders | `rust/crates/rustaxa-consensus/src/pbft_vote_payload.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_payload.rs` | `partial` | Rust owns legacy-compatible PBFT vote payload construction for post-admission side effects. It derives weighted storage RLP records from canonical signed vote bytes plus the authoritative calculated weight, builds raw weighted vote-bundle RLP for latest-round 2t+1 persistence and finalized reward-vote reset stages, builds optimized PBFT vote-bundle RLP for get-next network egress from retained weighted records, and normalizes unweighted signed vote RLP for slashing evidence so storage weights do not leak into slashing calldata. The Rust admission runtime now retains these payloads for accepted votes, persists admission progress internally, and returns only slashing-ready records to `VoteManager::addVerifiedVote`; the bridge also exposes plan/build status codes so C++ can peer-filter and chunk optimized egress without materializing `PbftVote` sidecars. C++ still selects live vote sidecars for remaining reload/finalization compatibility paths, supplies period/round/block metadata where Rust has not already returned it, executes Rust storage batches, wraps tarcap packets, marks sent vote hashes known, and submits signed slashing transactions. |
| Shared DAG/PBFT types | `rust/crates/rustaxa-types/src/{dag.rs,pbft.rs}` and codec modules | partial | Useful for future consensus domain types, but not yet a full consensus model. |
| Storage ports | `rust/crates/rustaxa-storage/src/{dag.rs,pbft.rs,pillar.rs}` | closed for migrated consensus storage; compatibility shell remains | `rustaxa-storage` is the durable backend for migrated production consensus storage rows. PBFT finalization, VoteManager persistence, TransactionManager consensus storage, DAG/proposed-block storage, rewards stats, pillar storage, PBFT-manager residual storage, gas-pricer storage, and consensus FinalChain fact ports now route through Rust-owned runtimes or storage sessions rather than production `DbStorage`/`getDB()`/`rustBatchId` authority. `BridgeStorage` and `DbStorage` remain for CXX construction, storage-shim internals, public query/network/admin/test compatibility, and temporary sidecar materialization. New routes must not add unclassified C++ storage authority. |

## Storage Boundary Status

The consensus storage migration is closed for the migrated production Rust-mode routes. `rustaxa-storage` is the durable
backend, and `rustaxa-consensus` or subsystem runtimes now own storage fact collection, write ordering, idempotency,
restart normalization, and commit/drop behavior for the audited consensus storage families. The storage-boundary guard and
post-migration audit found no remaining unclassified production consensus route that depends on `DbStorage`, direct
`getDB()`, public `rustBatchId`, or bridge-batch appenders as the storage authority.

Two compatibility layers remain intentionally visible:

- `BridgeStorage` is a temporary CXX-safe handle/constructor adapter and compatibility batch registry around the shared
  Rust storage handle. It should not become a new consensus storage API.
- `DbStorage` is the C++ lifecycle, public API, legacy/reference, storage-shim, query, network, test, and admin shell.
  Remaining references must stay classified as storage-shim internals, marked query/network compatibility, FinalChain/EVM
  boundary work, sidecar materialization, or app/admin lifecycle behavior.

CRW-06 closes the production-authority audit; future shrink work is caller-owned compatibility retirement rather than
storage migration. Remove `BridgeStorage`, storage-shim, and `DbStorage` code only when the caller has moved to a
Rust-owned runtime, read API, fixture, or executor boundary. Re-plan
before moving network/tarcap transport, packet wrapping, gossip fanout, arbitrary EVM execution, receipt/contract
execution, or public API materialization into the consensus storage cleanup.

### Compatibility Surface Cleanup Slices

- Slice 1 — Inventory and classification is complete: all migrated and remaining compatibility storage call sites and
  shells were inventoried and classified.
- Slice 2 — Runtime handle collapse is complete: C++ consensus shims now receive typed Rust runtime/query handles where
  storage ownership remains active; `DbStorage` and bridge batch registries are not used as production storage authority.
- Slice 3 — Batch registry cleanup is complete: generic bridge batch appender and compatibility batch lookup/deletion routes
  were removed or reduced to typed storage operations where production code still needs compatibility staging.
- Slice 4 — Query and materialization split is complete: read paths for DAG, PBFT, Pillar, Transaction, rewards, and
  final-chain/period lookups moved onto typed query handles; remaining reads are explicitly classified as compatibility
  boundaries.
- Slice 5 — Header and FFI pruning is complete: stale `BridgeStorage`/`storage_shim` header and CXX FFI declarations
  were removed after their callers moved to typed Rust storage/session APIs.
- Slice 6 — Storage/admin/query compatibility classification is complete: remaining storage-shim admin, migration, and
  generic iterator/existence boundaries are explicitly marked as `RUSTAXA_ADMIN_COMPAT_UNSUPPORTED`,
  `RUSTAXA_ADMIN_COMPAT_LEGACY_ONLY`, or `RUSTAXA_QUERY_COMPAT_READ`. This keeps snapshot/range/compaction/migration and
  broad iterator shells visible as compatibility debt without treating them as production consensus storage blockers.
- Slice 7 — CXX carrier minimization is active: the transaction finalized-check identity input moved from generated
  CXX into a shim-owned C++ carrier, and four transaction/DAG staging DTOs moved from `ffi.rs` into private Rust module
  types. `CRW-02` then replaced `BridgePbftManagerRuntime` and `BridgePbftChain` with one application-owned
  `BridgePbftService`, deleted the finalization chain report carrier/API, and moved the obsolete
  `PbftManagerStartupFact` out of CXX into a Rust-private test fixture. The remaining production exports have callers,
  while the one test-only FinalChain seed helper is an enforced storage-conformance boundary. Further reduction follows
  each service composition item rather than a no-caller export sweep. The first `CRW-03` sub-slice then deleted
  `BridgeProposedBlocks`, its factory and explicit restore exports, the storage-shim-owned live handle, and the C++
  facade mutex after all production callers moved to the PBFT service or stateless compatibility functions.

No separate file now tracks this cleanup; `doc/consensus_rewrite_tracker.md` is the active tracking location.

### Remaining Consensus Work Queue

This is the dependency-ordered execution queue for the remaining consensus rewrite. `PLAN.md` defines the ownership
boundary, `doc/consensus_consolidation_plan.md` provides slice design and historical implementation detail, and
`doc/consensus_bridge_shim_audit.md` is the mechanical bridge/shim deletion inventory. Update this queue whenever a
slice changes status or exposes a new dependency; do not create another consensus gap or cleanup tracker.

Statuses are `ready` (can be selected now), `blocked` (a named dependency or scope decision is missing), `active`
(incremental work is in progress), and `complete` (the completion condition and required validation have landed).
Activating an item still requires a bounded implementation slice with the validation and review required by
`doc/rewrite_validation_strategy.md` and `.codex/skills/SKILL.md`.

#### Required consolidation and parity work

| ID | Status | Work | Depends on | Complete when |
| --- | --- | --- | --- | --- |
| `CRW-01` | `complete` | Select the minimal Rust application-service composition boundary. Start with a PBFT-cluster-only root unless code mapping proves a shared DAG/transaction/pillar root removes more active compatibility surface without creating a service locator. | None | The selected ownership graph, retained external facades, constructor/bootstrap path, and first deletable handles are recorded in this tracker and the bridge audit. |
| `CRW-02` | `complete` | Compose `BridgePbftManagerRuntime` and `BridgePbftChain` behind one application-owned PBFT service; migrate app/bootstrap, `pbft_manager_shim`, and `pbft_chain_shim` production callers. | `CRW-01` | One application-owned service now owns PBFT manager and chain lifetime/state; Tier 1/Tier 2/Tier 3 evidence and classified pre-existing full-CTest harness/environment failures are recorded in the consolidation plan, and independent review approved the slice. |
| `CRW-03` | `complete` | Absorb PBFT-private state handles into the PBFT application service, starting with proposed blocks and verified votes, then close their combined leader, admission/persistence, and period-cleanup crossings. | `CRW-02` | Proposed-block and verified-vote state is service-owned; obsolete independent handles are deleted; leader selection revalidates one coherent snapshot; vote admission persists before publication; and period advance cleans both owners through one atomic service action. |
| `CRW-04` | `complete` | Compose transaction/gas and DAG graph/manager/proposer runtimes behind application-owned Rust services with native FinalChain/storage ports. | `CRW-01`; coordinate shared dependencies with `CRW-02` | C++ shims no longer pass internal bridge handles between transaction, DAG, PBFT, FinalChain, or storage services; they perform input conversion, explicit EVM/network execution, and public materialization only. |
| `CRW-05` | `complete` | Compose pillar, slashing, sortition, and rewards planning/state behind their Rust application owner rather than standalone internal handles. | `CRW-01`; `CRW-02` where PBFT owns the lifetime | Remaining C++ code is limited to FinalChain/DPoS fact execution, signing, transaction insertion, tarcap/event execution, lifecycle/executor work, and public materialization; internal bridge handles and cross-shim lookup paths are deleted. |
| `CRW-06` | `complete` | Delete storage compatibility scaffolding after runtime consumers move: `BridgeStorage`, `BridgeStorageBatch`, storage query-family handles, broad storage-shim calls, and related `DbStorage` compatibility access. | Relevant consumer migrations in `CRW-02` through `CRW-05` | No production consensus route uses broad storage handles or C++/bridge batch authority. Retained admin, migration, test, network, and public-query behavior is narrow, explicitly classified, or explicitly unsupported in Rust mode. |
| `CRW-07` | `active` | Continue CXX carrier/export, module-flag, shim, and compatibility-test minimization after every consumer migration. | Runs alongside every consolidation item | The bridge exposes only `BridgeConsensusQueryApi`, `BridgeConsensusNetworkApi`, `BridgeConsensusExecutionApi`, application/bootstrap handles, and demonstrably necessary public compatibility handles. The inventory guard has no undocumented or stale entries, and tests protect behavior rather than retired scaffolding. |
| `CRW-08` | `active` | Close remaining FinalChain/DPoS behavior parity: required contract methods outside the currently supported mutation subset and full failed-contract receipt parity for older supported paths. | A bounded method/receipt family and expected legacy behavior must be selected per slice | All required current-ABI DPoS/slashing behaviors in the selected family execute through Rust account/DPoS state with byte-compatible receipts, logs, blooms, persistence, restart behavior, and targeted legacy-vs-Rust parity coverage. |
| `CRW-09` | `ready` | Introduce missing P0 FinalChain domain types/codecs and reduce temporary C++ `StateAPI` fact collection while preserving external EVM/state execution as an explicit adapter. | Select one type family or execution transcript per slice | Consensus-internal request, recovery, publication, and audit data remains Rust-owned; C++ `StateAPI` supplies only the external execution/committed-state operations allowed by `PLAN.md`, with byte-compatible codec and transcript coverage. |
| `CRW-10` | `blocked` | Perform final consensus consolidation closeout: delete newly obsolete code/docs, reconcile the audit, run required Rust/C++ validation, and synchronize applicable upstream-owned C++ intersections to `cpp-reference`. | `CRW-02` through `CRW-09`, excluding work explicitly scope-gated below | No actionable unclassified consensus ownership or compatibility-deletion item remains; retained C++ surfaces match the declared network, EVM, lifecycle, signing/VDF, and public-materialization boundaries, and the tracker/audit/plan agree. |

`CRW-08` current slice closes native DPoS delegate transaction receipt/state parity. Rust now charges top-level
transaction intrinsic gas in addition to action gas for every Rust-native DPoS/slashing transaction while preserving
legacy precompile out-of-gas behavior: insufficient intrinsic gas consumes the gas limit, and insufficient action gas
consumes intrinsic gas, advances the sender nonce, and does not transfer value or mutate contract state. Genesis
validator `total_stake` is credited once to the DPoS precompile account, any explicit precompile balance is preserved,
and its genesis nonce is one; persisted account snapshots replace this derived state on restart. The dual-mode delegate
fixture proves byte-compatible receipt RLP, status, 62,680 gas, cumulative/header gas, logs, bloom, balances, nonce,
stake/vote state, and restart state. The new Tier 3 `make rewrite-validate-final-chain-parity` gate passed from both a
fresh and retained pure-C++ build cache. Existing Rust databases created before this correction may contain deficient
DPoS account snapshots; migration versus rebuild remains an explicit deployment decision and this slice does not
silently top up persisted state. `CRW-08` remains active for the next bounded method/failed-receipt family.

The next bounded `CRW-08` slice closes the native `delegate(address)` contract-failure family. Missing validators,
first delegations below `minimum_deposit`, and delegations that would exceed the validator maximum now produce legacy
status-zero receipts instead of aborting FinalChain. Native contract execution applies each transaction against
the block-local account and DPoS state in order. Expected status-zero outcomes are validated before mutation; payable
value and claim-gas state commit only after success, while gas and nonce remain charged on failure. This preserves
same-block ordering without cloning the complete account and DPoS state per contract call. Mutable DPoS execution now
starts from the immediately preceding finalized snapshot even when delegation reads use a nonzero historical delay, and
native contract fees flow through the common reward-accounting path for successful and failed receipts. Existing
delegators may still make a below-minimum top-up, matching the legacy contract rule. The dual-mode missing-validator
fixture proves exact 61,464 gas, receipt RLP, empty logs/bloom, gas-only balance charge, nonce, unchanged
escrow/stake/votes, and restart state. Focused Rust coverage also proves nonzero-delay multi-block persistence through
restart, pre-Magnolia fee rewards for both receipt statuses, and failed-delegate same-sender continuation. The Tier 1,
FinalChain Tier 2, and current-source `make rewrite-validate-final-chain-parity` Tier 3 gates passed with both focused
delegate fixtures and the complete Rust-enabled and pure-C++ FinalChain suites. No bridge/export surface changed, so
`CRW-07` has no inventory delta. `CRW-08` remains active for the next bounded method or failed-receipt family.

The next bounded `CRW-08` slice closes the direct `claimRewards(address)` missing-delegation failure path. Legacy DPoS
returns `ErrNonExistentDelegation` as a contract execution failure, but Rust previously propagated the missing pair as
an `anyhow` error and aborted finalization. The Rust contract boundary now checks the caller/validator delegation pair
before reward cursor or account mutation and emits a status-zero outcome when it is absent; cursor ordering, arithmetic,
contract-balance, storage, and codec faults remain hard invariants. Focused Rust retains a separate absent-validator case,
while the same-block/restart Rust fixture and dual-mode C++ fixture target a registered validator delegated only by a
different account. Together they prove both missing-pair branches, exact 61,464 gas, canonical persisted receipt RLP,
empty logs and bloom, gas-only sender charge, nonce advancement, unchanged DPoS stake/vote/reward state, same-block
continuation by the sender, and restart persistence. The reusable pure-C++ parity filter includes the new fixture. No
bridge/export surface changes, so `CRW-07` again has no inventory delta.
`CRW-08` remains active for registration, undelegation, redelegation, and remaining reward/method failure families.

The next bounded `CRW-08` slice closes current-ABI native `registerValidator(address,bytes,bytes,uint16,string,string)`
business validation. Legacy validates the validator-address proof, minimum stake, endpoint and description byte lengths,
VRF-key length, commission, duplicate registration, and maximum stake before mutation; each rejection is a normal
status-zero contract outcome. Rust previously discarded the proof, rejected some well-formed business-invalid payloads
during ABI decoding, omitted several checks, and aborted finalization for duplicate or over-maximum registrations. Rust
now preserves proof and VRF bytes through decoding, verifies the legacy 27/28 recoverable proof over the validator
address, classifies the full selected family before mutation, and keeps inconsistent snapshot rows and arithmetic/codec
faults hard. Successful registration now records the funding caller as delegator, and zero-value registration no longer
creates a phantom delegation or reward cursor. Focused Rust boundaries and the dual-mode 11-transaction C++ fixture
cover valid success, wrong proof, below-minimum stake, 50/51-byte endpoint, 100/101-byte description, 31/33-byte VRF,
10,000/10,001 commission, duplicate validator, exact/over maximum stake, exact gas and receipt RLP, log/bloom ordering,
gas-only failure charges, nonce, value rollback, same-sender continuation, state facts, and restart. Structurally malformed
ABI and invalid-UTF-8 string handling remain an explicit cross-method DPoS decode-failure family. No bridge/export surface
changes, so `CRW-07` again has no inventory delta. `CRW-08` remains active for that decoder family plus undelegation,
redelegation, and remaining reward/method failure families.

The next bounded `CRW-08` slice closes that cross-method native DPoS ABI decode-failure family. Rust now classifies the
mutation selector before decoding arguments, so malformed known fixed-gas methods produce normal status-zero receipts
with their legacy action gas instead of aborting FinalChain. Malformed legacy `claimAllRewards(uint32)` preserves its
legacy successful no-op with intrinsic-only gas. Short and unknown inputs and hardfork-disabled selectors also consume
intrinsic gas only but fail normally. Current claim-all and well-formed legacy batches accept trailing calldata; the
legacy batch decoder rejects `uint32` overflow, while other retained `uint16`/`uint32`/`uint64` decoders use the
low-width bytes like the legacy Go ABI. Cornus
nonpayable calls with value fail before decode with zero action gas, while pre-Cornus successful DPoS calls retain legacy
value-transfer behavior. Finalized validator description and endpoint fields are byte-native, preserving invalid UTF-8
through ABI queries and snapshot RLP without changing the CXX/genesis string carrier or existing RLP layout. Old
valid-UTF-8 snapshots remain byte-compatible, but rollback to a pre-slice Rust binary is unsafe after invalid metadata is
finalized because that binary cannot decode the new payload. The dual-mode fixture covers malformed fixed/dynamic calls,
short/unknown and hardfork-disabled selectors, Cornus nonpayability, claim-all overflow/trailing behavior, same-sender
continuation, invalid-byte registration, query bytes, and restart. No bridge/export surface changed, so `CRW-07` again
has no inventory delta. `CRW-08` remains active for undelegation, redelegation, and remaining reward/method failures.

The next bounded `CRW-08` slice closes four ledger-derived, pre-mutation failures for native V1
`undelegate(address,uint256)`: a missing validator, a missing caller/validator delegation, an amount greater than the
delegation, and a nonzero remainder below `minimum_deposit`. A fallible Rust preflight classifies those expected business
rejections as contract failures while preserving aggregate-stake corruption as a hard invariant error. Each selected
failure now produces the legacy status-zero receipt with 60,000 action gas, advances the sender nonce, charges gas,
rolls back value, emits no logs or bloom, and leaves account, delegation, stake, vote, and reward state unchanged; the
dual-mode fixture also proves same-sender block continuation and restart stability. Existing V1 undelegation requests,
V1 request persistence and confirm/cancel/query behavior, zero-amount requests, and successful escrow release remain
outside this slice. No bridge/export surface changed, so `CRW-07` has no inventory delta. `CRW-08` remains active for
the V1 undelegation lifecycle, redelegation, and remaining reward/method failures.

The next bounded `CRW-08` slice completes the native V1 undelegation custody lifecycle as one deployable family. A
successful `undelegate(address,uint256)` now atomically persists an ordered `(delegator, validator)` request, rejects a
duplicate before reward or stake mutation, preserves the legacy zero-amount request, and uses the base/Cornus/Cacti
locking-period priority. Rust serves `getUndelegations(address,uint32)` with legacy 20-entry paging, end flags,
validator-existence facts, and per-item gas. `getValidator(address)` retains the legacy pre-Magnolia zero pending count
and reports the combined V1/V2 queue count at Magnolia and later. Rust also
executes `confirmUndelegate(address)` and `cancelUndelegate(address)` with legacy action gas, status-zero missing/locked
failures, reward and stake restoration on cancel, escrow payout on confirm, exact V1 logs/blooms, and Magnolia deletion
only after zero stake, zero commission rewards, and a zero combined pending count. Pre-Magnolia zero-stake validator
deletion remains compatible while its pending V1 request stays confirmable. At Magnolia and later, Rust deliberately
derives the count/deletion guard from both queues, fixing the legacy `ValidatorV1` blind spot that omitted pre-Magnolia
requests and could delete a validator while principal was still pending. The snapshot schema appends an ordered V1 queue
as item 21 and continues to decode the prior 20-item form; rollback to a pre-slice binary is unsafe after a new
snapshot is finalized. Restart-backed dual-mode coverage exercises create/query/cancel, duplicate and missing failures,
locked confirmation, and successful payout. No bridge/export surface changed, so `CRW-07` has no inventory delta.
`CRW-08` remains active for redelegation and the remaining DPoS method/failure families.

The next bounded `CRW-08` slice closes native `reDelegate(address,address,uint256)` for normal calls and the configured
historical correction transcript. Rust applies the legacy ordered failures for same-validator post-fix calls,
Aspen-part-two zero amounts, missing validators or source delegation, enabled destination maximum, excess source amount,
and a nonzero below-minimum remainder before mutation; aggregate-stake corruption remains hard. Success claims source
then existing-destination
rewards, moves only the delegation ledger, leaves DPoS escrow and aggregate delegated amount unchanged, permits new
destination pairs below `minimum_deposit` (including a zero pair before Aspen), emits only reward logs followed by
`Redelegated`, and deletes an empty source validator only under the pre-/post-Magnolia pending-queue rule. A configured
maximum stake of zero is treated as unlimited. Rust deliberately reproduces the historical same-validator stale-stake,
ordered vote-delta, and restored pre-claim reward-pool writes through `fix_redelegate_block_num`, then applies the
ordered configured corrections after rewards and transaction effects and before snapshot publication. Corrections
subtract only the configured amounts, derive each validator's vote count from resulting stake, and preserve the legacy
global eligible-vote total even when another same-validator call at the fix block leaves an unconfigured gap; later
same-validator calls persist status-zero receipts.
Repeated reward-bearing same-validator calls after a snapshot already contains stale reference state require the legacy
reward-state reference graph, which the current scalar reward snapshot cannot represent. Rust now persists a
complete-history bit plus an explicit validator corruption marker on every successful pre-fix same-validator call,
including zero-amount calls that leave no stake gap, and recognizes every older stake/principal mismatch. Markerless
5-through-21-item snapshots remain history-incomplete across re-encoding: a reward-bearing pre-fix same-validator call
requires rebuild/replay instead of silently assuming no zero-amount history. Ambiguous, marked, or mismatched calls fail
finalization before mutation; representable repeated zero-pool calls on complete-history snapshots remain available for
the configured correction transcript. Full graph replay remains active `CRW-08` work.
Restart-backed Rust and dual-mode coverage protect exact action gas, receipt/log/bloom behavior, rollback, source
deletion, fix-1/fix/fix+1 state, configured-versus-new fix-block gaps, duplicate reward-pool payouts, Aspen zero handling,
below-minimum/new-pair behavior, maximum handling, and correction persistence. `FinalChainRewardsConfig` gains the fix
block and ordered correction entries, so `CRW-07` records a carrier field delta but no new handle or export. `CRW-08`
remains active for the remaining DPoS method/failure families.

The next bounded `CRW-08` slice closes the terminal validator lifecycle for
`claimCommissionRewards(address)`. After validating the owner, paying the exact commission pool, zeroing it, and
emitting `CommissionRewardsClaimed`, Rust deletes a zero-stake validator before Magnolia or, from Magnolia onward, only
when the combined V1/V2 pending-undelegation count is zero. Pending requests remain custody/history state and stay
confirmable even when pre-Magnolia deletion removes the registration; V2 last-ID state is likewise retained. Deletion
clears every validator-owned snapshot row plus the Rust-only same-validator corruption marker, so restart and fresh
same-address registration cannot inherit stale metadata, VRF, ordering, reward, vote, or redelegation history. Registration
after a clean deletion remains supported; a marker left without validator rows by an older binary is instead a hard
snapshot inconsistency that requires explicit repair, migration, or replay/rebuild. A
metadata row without the corresponding stake row is classified as hard snapshot corruption before payout rather than
as a terminal zero-stake validator. Focused Rust coverage protects pre-/post-Magnolia queue rules, V1 and V2 retention,
failure rollback, restart, and re-registration. A dual-mode `native_dpos_*` FinalChain fixture protects exact current-ABI
action gas, receipt/log/bloom, payout, and durable validator retention while a V2 request remains pending. The pure-C++
reference keeps its observable pending counter after V2 confirmation in this path, whereas Rust intentionally uses the
corrected combined live-queue view, so terminal deletion and fresh same-address registration remain Rust-focused coverage
rather than a misleading common transcript. The legacy Magnolia-to-Phalaenopsis
interval's conditional zero-reward persistence bug is not claimed as replay parity; modeling that interval would require
a separate hardfork carrier. No CXX handle, carrier, export, shim, or module flag changes, so `CRW-07` has no inventory
delta. `CRW-08` remains active for the remaining DPoS method/failure families and the explicit historical reward graph.

The next bounded `CRW-08` slice closes two native V2 undelegation custody details. Before Magnolia, a successful full
`undelegateV2(address,uint256)` now removes a zero-stake, zero-commission validator after reward checkpointing and stake
removal, while retaining the V2 request and last-ID cursor as custody/history state. The request uses the active locking
period in Cacti, Cornus, then base priority and remains queryable and confirmable after registration deletion; confirmation
removes the request and releases the exact escrowed principal. At Magnolia and later, validator retention/deletion keeps
using the corrected combined live V1/V2 queue view. The staged same-block claim-gas snapshot now removes V2 requests on
confirmation and, on successful cancellation, restores both delegation membership and principal before a later
`claimAllRewards()` gas calculation. Failed calls still leave that staged view unchanged. Focused Rust coverage protects
snapshot round-trips, retained IDs/queues, pre-Magnolia terminal deletion, and confirm/cancel gas-view transitions.
Restart-backed dual-mode FinalChain fixtures protect exact action gas, receipts, logs, blooms, Cornus lock selection,
confirmation payout, cancel restoration, claim-all gas, and durable state. Nonzero `delegation_delay` claim-all gas is
closed by the later live-membership slice below; the known legacy Magnolia persisted-counter divergence remains outside
this transcript. No CXX handle, carrier, export,
shim, or module flag changes, so `CRW-07` has no inventory delta. `CRW-08` remains active for the remaining DPoS
method/failure families and the explicit historical reward graph.

The next bounded `CRW-08` slice restores the exact four-byte Phalaenopsis DPoS escrow-transfer action
`0x44df8e70`. Before the configured activation period, or when any trailing calldata is present, Rust preserves the
legacy unknown-selector status-zero receipt with intrinsic gas only and no value transfer. At activation and later the
action remains payable even after Cornus, charges 1,000 action gas, and uses the common successful contract-payment path
to move only the transaction value from the sender account into DPoS escrow. It emits no logs, returns no payload, and
does not mutate validator, delegation, vote, reward, supply, or other DPoS snapshot state. Account/receipt publication
remains atomic and restart-durable. `FinalChainRewardsConfig` gains the Phalaenopsis activation period sourced from
genesis hardfork configuration, so `CRW-07` records a carrier-field delta but no new handle, export, constructor, shim,
module flag, or snapshot schema. Historical databases finalized by the prior Rust path with this selector require
replay/rebuild or a separately designed migration; this slice does not infer or top up escrow. `CRW-08` remains active
for the remaining DPoS method/failure families and the explicit historical reward graph.

The next bounded `CRW-08` slice closes nonzero-`delegation_delay` claim-all gas parity. Legacy
`claimAllRewards()` and `claimAllRewards(uint32)` price the caller's live contract membership; the delay applies to
eligibility and historical authorization reads, not mutation gas. Rust native execution now seeds its staged claim-gas
view from the immediately preceding finalized DPoS snapshot, the same live state used for transaction mutation, and then
continues applying successful same-block membership changes before each later claim. Delayed eligibility APIs retain
their existing historical snapshot selection. A zero-gas-price, restart-backed dual-mode fixture creates a delegation
after genesis with delay two, then proves a current claim-all receipt charges one exact 45,000-gas item while total stake
uses live state and eligible votes remain delayed. It also protects empty logs/bloom, nonce, balances, receipt RLP, and
restart persistence. No carrier, bridge handle/export, shim, module flag, or snapshot schema changes, so `CRW-07` has no
inventory delta. `CRW-08` remains active for the remaining DPoS method/failure families and the explicit historical
reward graph.

The next bounded `CRW-08` slice restores value custody for native slashing
`commitDoubleVotingProof(bytes,bytes)`. The legacy EVM transfers call value before invoking the precompile, and the
slashing contract has no generic or Cornus nonpayability check, so a successful proof keeps that value at the slashing
account despite the ABI's nonpayable metadata. Rust now defers both sender debit and contract credit until the proof
outcome succeeds, initializes the slashing account nonce to one on its first successful jail write, and never increments
that nonce again. Duplicate, malformed, invalid, and otherwise failed proofs retain their action gas and sender nonce but
roll back value and slashing state. A restart-backed dual-mode fixture submits a valid value-bearing proof followed by a
duplicate value-bearing proof in one block and protects exact receipt RLP, gas/cumulative gas, `Jailed` log/bloom,
success-only slashing balance, sender balance/nonce, eligibility effect, account nonce, duplicate rollback, and restart.
No carrier, bridge handle/export, shim, module flag, or snapshot schema changes, so `CRW-07` has no inventory delta.
Rust-finalized history that previously encountered a valid value-bearing proof contains a failed receipt, missing jail
state, and missing slashing-account custody; recovery requires replay/rebuild from before the first affected proof or a
separately designed migration rather than an inferred balance edit. `CRW-08` remains active for precompile read
transactions, the remaining DPoS method/failure families, and the explicit historical reward graph.

The next bounded `CRW-08` slice closes native slashing read transactions for `getJailBlock(address)` and
`getJailedValidators()`. At Magnolia and later, both recognized selectors charge 5,000 action gas and succeed without
logs or jail-state mutation; `getJailedValidators()` retains the legacy acceptance of trailing calldata, while a
malformed jail-block argument is a status-zero contract failure with the same action gas. Legacy EVM call ordering makes
these ABI-view methods payable in actual transaction execution, so successful reads move value into the slashing
account while malformed and action-out-of-gas calls roll it back. Read success never initializes the slashing account
nonce: that storage-initialization side effect remains exclusive to a successful double-voting-proof write. The read
view remains frozen at the delayed previously committed snapshot and cannot observe a proof earlier in the same block.
A pre-Magnolia call retains ordinary empty-account semantics: intrinsic-only success, value transfer only when nonzero,
and no materialized receiver account for a zero-value call.
A restart-backed dual-mode fixture protects exact receipt RLP, gas/cumulative gas, empty logs/bloom, successful-only
value custody, sender continuation, account nonces, and persisted receipts. No carrier, bridge handle/export, shim,
module flag, or snapshot schema changes, so `CRW-07` has no inventory delta. Rust-finalized history that previously
treated these selectors as unsupported contains different receipts, cumulative gas, balances, and potentially later
transaction affordability; recovery requires replay/rebuild from before the first affected read or a separately
designed migration rather than an inferred balance edit. `CRW-08` remains active for DPoS precompile read transactions,
the remaining DPoS method/failure families, and the explicit historical reward graph.

The next bounded `CRW-08` slice closes the complete fixed-gas DPoS eligibility-read family:
`isValidatorEligible(address)`, `getTotalEligibleVotesCount()`, and
`getValidatorEligibleVotesCount(address)`. Rust direct calls now serve all three from the delayed eligibility snapshot,
evaluate stake thresholds, vote schedules, Cacti jail state, and jail expiry at the same delayed block, and charge the
legacy 20,000 action gas; this corrects the prior 22,000 total-count estimate. Native finalized transactions use one
frozen preceding-head delayed view for the whole block, emit no logs, and do not expose earlier same-block DPoS or
slashing mutations. Before Cornus the reads retain legacy value custody at the DPoS account. At and after Cornus they
are nonpayable before ABI decoding, so value-bearing calls fail with intrinsic gas only and roll value back. Recognized
malformed address calls retain fixed action gas before Cornus, unknown selectors remain intrinsic-only failures, and
action-out-of-gas calls charge intrinsic gas. The shared native transaction state machine now also follows the Cornus
fork rule for intrinsic-out-of-gas nonce advancement while retaining the pre-Cornus unchanged nonce. A restart-backed
dual-mode fixture protects exact receipt RLP, gas/cumulative gas, empty logs/bloom, successful-only value custody,
pre-/at-Cornus intrinsic-gas nonce behavior, same-sender continuation, DPoS account state, and persisted receipts. No
carrier, bridge handle/export, shim, module flag, or snapshot schema changes, so `CRW-07` has no inventory delta.
Historical Rust receipts for these selectors, the prior 22,000 call estimate, and Cornus intrinsic-out-of-gas native
transactions may differ in gas, cumulative gas, nonce, balances, and later affordability; recovery requires
replay/rebuild from the first affected transaction or a separately designed migration rather than inferred account
repairs. `CRW-08` remains active for the remaining static and dynamic DPoS read transactions, remaining DPoS
method/failure families, and the explicit historical reward graph.

The next bounded `CRW-08` slice closes the remaining fixed-5,000 singleton DPoS read family:
`getValidator(address)` and Cornus-gated `getUndelegationV2(address,address,uint64)`. Rust direct calls normalize
recognized malformed and missing-record outcomes into contract errors instead of escaping as executor failures, while
retaining orphaned validator-owned rows, including the persisted same-validator corruption marker, as a hard snapshot
inconsistency. Native finalized transactions read the exact
live block-local DPoS snapshot. Consequently a V2 read observes a
successful `undelegateV2` earlier in the same block rather than the frozen delayed eligibility view. Before Cornus,
`getValidator` accepts successful call value into the DPoS account and the V2 selector remains unsupported with zero
action gas. At Cornus and later both methods reject value before ABI decoding with intrinsic gas only. Active valid,
missing, and malformed calls retain 5,000 action gas; action-out-of-gas calls charge intrinsic gas, and the shared
pre-/at-Cornus intrinsic-out-of-gas nonce rule remains unchanged. Successful reads emit no logs, do not mutate DPoS
state, and leave the DPoS account nonce at one. A restart-backed dual-mode fixture protects trailing calldata,
pre-Cornus value custody and V2 rejection, same-block V2 creation/read visibility, missing-record and Cornus value
failures, exact receipt RLP and gas, log/bloom isolation from the preceding mutation, balances, nonces, and persisted
receipts. No carrier, bridge handle/export, shim, module flag, or snapshot schema changes, so `CRW-07` has no inventory
delta. Historical Rust receipts for these native selectors may differ in status, gas, cumulative gas, value custody,
balances, and later affordability; recovery requires replay/rebuild from the first affected transaction or a separately
designed migration rather than inferred account repairs. `CRW-08` remains active for dynamic validator pages,
delegation and undelegation reads, remaining DPoS method/failure families, and the explicit historical reward graph.

The next bounded `CRW-08` slice executes the dynamic validator-page family natively:
`getValidators(uint32)` and `getValidatorsFor(address,uint32)`. Both read exact live block-local DPoS state, so a page
observes successful registration or terminal deletion earlier in the same block. `getValidators` charges 5,000 action
gas per returned entry, capped at 20, with one 5,000-gas storage read for an empty or out-of-range page;
`getValidatorsFor` retains the legacy fixed 100,000 action-gas scan charge regardless of matches. Recognized malformed
calldata has zero action gas. Before Cornus, successful value-bearing reads retain value in the DPoS account; from
Cornus onward both selectors reject value before ABI decoding. Execution preserves legacy narrow ABI behavior and
wrapping `uint32 batch * 20` page offsets even though gas calculation widens the batch before multiplication. Validator
deletion now mirrors the legacy iterable map by moving the last validator into a removed middle slot, making durable
page order match after terminal deletion. Partially deleted validator-order entries remain hard snapshot corruption
rather than disappearing from output. Successful reads emit no logs and do not mutate DPoS state. No carrier, bridge
handle/export, shim, module flag, or snapshot schema changes are required, so `CRW-07` has no inventory delta.
Historical Rust native page transactions were finalized as unsupported, and Rust snapshots after a non-tail deletion
may retain stable-shift rather than legacy swap-remove order. Status, gas, receipt roots, value custody, balances,
ordering, and later affordability can therefore differ; exact recovery requires replay/rebuild from the first affected
read or deletion, or a separately designed migration rather than inferred repair. `CRW-08` remains active for
delegation and undelegation reads, remaining DPoS method/failure families, and the explicit historical reward graph.

The next bounded `CRW-08` slice executes the two custody-only undelegation page reads natively:
`getUndelegations(address,uint32)` and Cornus-gated `getUndelegationsV2(address,uint32)`. Both use the exact live
block-local DPoS snapshot, so successful earlier same-block create, cancel, and confirm transitions affect page output
and gas. V1 charges 5,000 action gas for each returned request, capped at 20, with a 5,000 minimum for an empty or
out-of-range page. V2 charges 5,000 for each validator and request-index storage read: two reads per visited validator
group plus two per returned request, including scanned groups before an out-of-range page. Execution retains legacy
narrow ABI decoding, trailing calldata, iterable swap-remove order, and wrapping `uint32 batch * 20` offsets while gas
calculation widens the batch first. V1 is active throughout history; V2 remains unsupported before Cornus. Before
Cornus a successful value-bearing V1 read retains value in DPoS custody, while Cornus rejects value for both active
selectors before ABI decoding. Successful reads emit no logs and do not mutate DPoS state. No carrier, bridge
handle/export, shim, module flag, or snapshot schema changes are required, so `CRW-07` has no inventory delta.
Historical Rust native page transactions may differ in status, gas, receipt roots, value custody, balances, ordering,
and later affordability; exact correction requires replay/rebuild from the first affected transaction rather than an
inferred state edit. `CRW-08` remains active for delegation reads and their explicit historical reward reference graph,
plus the remaining DPoS method/failure families.

The next bounded `CRW-08` slice executes `getTotalDelegation(address)` natively without coupling principal accounting
to the remaining historical reward graph. The method sums only the delegator's live authoritative validator-membership
rows, charges 5,000 action gas per membership with zero action gas for an empty delegator, and observes successful
earlier same-block delegate, undelegate, cancel, redelegate, and terminal-validator transitions. Duplicate or dangling
membership, principal wider than `uint256`, and sum overflow are hard snapshot corruption. Legacy narrow address
decoding and trailing calldata remain accepted; malformed input has zero action gas. Before Cornus, successful value is
retained in DPoS custody; at Cornus and later value is rejected before ABI decoding. Direct calls continue to use the
requested finalized snapshot, while native execution uses transaction-point live state and emits no logs or mutations.

The internal DPoS snapshot codec gains a twenty-third item, `delegation_ledger_history_complete`, independent of the
same-validator reward-corruption marker. Genesis/current and schema-seven through schema-22 snapshots are complete;
direct schema-five/six snapshots remain incomplete across re-encoding and this read rejects them pending replay/rebuild.
A schema-five/six snapshot already rewritten as schema 22 by an older binary cannot be distinguished retroactively.
This is an internal storage-schema delta with no CXX carrier, bridge handle/export, shim, module flag, or `CRW-07`
bridge-inventory change. Historical Rust native total-delegation transactions may differ in status, gas, receipt roots,
value custody, balances, and later affordability; exact correction requires replay/rebuild from the first affected read
rather than an inferred principal edit. `CRW-08` remains active for `getDelegations(address,uint32)`, its explicit
historical reward-per-stake reference graph, and the remaining DPoS method/failure families.

The first bounded reward-reference-graph foundation is now isolated in `rustaxa-consensus` without routing
FinalChain reads or changing the persisted DPoS snapshot schema. It models arbitrary-width reward-per-stake nodes keyed
by validator and block, explicit validator heads and delegation cursors, persisted reference counts, incomplete-history
provenance, and the legacy stale-head correction boundary. Clone-staged mutations reproduce legacy load-copy-write
ordering, including same-key count inflation, node resurrection, and positive orphan counts, without recomputing
persisted counts. The canonical seven-field RLP codec rejects trailing bytes, non-list or unsorted tables, duplicate
keys, noncanonical integers, dangling references, and undercounted nodes. Reward arithmetic keeps exact `BigUint`
intermediates and applies `uint256` truncation only at the ABI boundary.

Validation passed all 20 focused reward-graph tests, all 738 `rustaxa-consensus` tests, normal-policy clippy with the
unchanged 46-warning crate baseline, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the bridge inventory
guard, skill validation, the repository pre-commit hook, Rust formatting, and whitespace validation.

The follow-up integrates that graph as item 24 of the persisted DPoS snapshot. Genesis and validator registration create
the exact head/cursor topology; transaction-time checkpoints move only the validator head; claims and every supported
stake mutation move or remove only their operation-specific delegation cursor; terminal validator deletion preserves
the distinct legacy force-delete and decrement/orphan outcomes. Block reward distribution still only grows the reward
pool and never creates a checkpoint. Exact graph nodes are now the reward-per-stake and cursor authority used by reward
claims, while the scalar rows remain derived compatibility data.

Schemas 5, 6, 7, 9, 11, 14, 15, 17, 20, 21, 22, and 23 decode with explicitly incomplete graph provenance and hard-fail
graph-dependent reads or mutation pending replay/rebuild. Schema 24 round-trips arbitrary-width accumulators, counts,
heads, cursors, stale markers, and current block across restart. Pre-fix same-validator redelegation now preserves fresh
and repeated partial counts `3 -> 5`, fresh and repeated full counts `2 -> 3`, stale live-or-missing heads, and the
configured count-neutral correction conflict rule. No CXX carrier, bridge handle/export, shim, module flag, or `CRW-07`
inventory change is introduced. `CRW-08` remains active for native/direct `getDelegations(address,uint32)` routing and
its dual-mode restart fixture; that read must consume the graph without scalar fallback or filtered corruption.

Integration validation passed all 23 focused reward-graph tests, all 746 `rustaxa-consensus` tests, normal-policy clippy
with the unchanged 46-warning crate baseline, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the bridge
inventory guard, skill validation, the repository pre-commit hook, Rust formatting, whitespace validation, and an
independent configured-reviewer `APPROVED` verdict. The future `getDelegations` C++ fixture remains outside this commit.

The follow-up routes direct and native `getDelegations(address,uint32)` through the persisted reward graph. Genesis
membership preserves validator insertion order, pages use the legacy wrapping `uint32` offset calculation and
swap-last removal order, and each returned row carries graph-authoritative pending reward without scalar fallback.
Action gas deliberately retains the legacy widened batch calculation, including its large-batch divergence from the
wrapped output page. Malformed recognized input has zero action gas; narrow ABI words and trailing calldata remain
accepted. Before Cornus successful value is retained in DPoS custody, while Cornus rejects value before decode.

Duplicate membership, missing principal or validator rows, incomplete graph history, and missing graph references are
hard errors. Only selected page rows are reward-resolved, and zero aggregate validator stake returns zero pending reward
without consulting an older cursor. Rust unit coverage proves native same-block membership visibility through read gas.
Restart-backed Rust-enabled and pure-C++ fixtures protect ordering, page/end flags, large-batch behavior, decoder
variants, and persistence. No CXX carrier, bridge
handle/export, shim, module flag, snapshot schema, or `CRW-07` inventory delta is introduced. Historical Rust native
reads may differ in status, gas, receipt roots, value custody, balances, ordering, and later affordability; exact
correction requires replay/rebuild from the first affected read. `CRW-08` remains active for the next bounded
method/failure family selected from the queue.

Read integration validation passed the focused Rust page test, all 747 `rustaxa-consensus` tests, the focused
Rust-enabled and pure-C++ restart fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust and changed-line C++ formatting, whitespace validation, and independent configured review.

The next bounded `CRW-08` slice closes finalized native owner-update parity for
`setValidatorInfo(address,string,string)` and `setCommission(address,uint16)`. Rust preserves the legacy ordered
business failures: endpoint then description byte limits and owner for validator info; owner, maximum commission,
frequency, then delta for commission. Exact limits pass, successful commission writes the current block, and the live
block-local update affects that block's minted commission/delegator reward split. Both methods charge 20,000 action gas.
Before Cornus successful value remains in DPoS custody while business failures roll it back; from Cornus onward value is
rejected before decode with intrinsic-only gas. Success emits exact `ValidatorInfoSet` or `CommissionSet` logs and bloom,
while failures emit none. Metadata without canonical stake and a future persisted commission-change block are hard
snapshot inconsistencies after legacy user-error precedence; clean absence remains a status-zero contract failure.
Restart-backed dual-mode coverage protects ordered failures, gas, value, logs, metadata/commission state, nonpayability,
receipt persistence, and restart, while focused Rust coverage protects corruption ordering, exact frequency/delta
boundaries, and same-block reward effects. No CXX carrier, bridge handle/export, shim, module flag, or snapshot schema
changes are introduced, so `CRW-07` has no inventory delta.

All 25 current Solidity DPoS ABI methods now have Rust selector/decode and native apply/read routing. `CRW-08` remains
active because general DPoS mutation simulation through `FinalChain::call` is still incomplete: that read-oriented Rust
surface recognizes mutation ABI/gas but returns empty success without applying business rules. Close that cross-method
gap through one shared ephemeral native executor rather than setter-only simulation, and continue auditing any
remaining failed-receipt transcript before declaring `CRW-08` complete.

Owner-update validation passed the focused maximum-height frequency regression, all 753 `rustaxa-consensus` tests,
the focused Rust-enabled and pure-C++ restart fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust and changed-line C++ formatting, whitespace validation, the repository pre-commit hook, and
independent configured review.

The next `CRW-08` foundation slice extracts all finalized DPoS mutation dispatch behind one staged-snapshot kernel.
Finalized execution still owns gas, fees, nonces, payable-argument value injection, value custody, rollback, receipts,
reward planning, cleanup, and publication; the kernel owns only the deterministic DPoS transition result. Its
caller-owned account and DPoS maps make the eventual dry-run boundary explicit without routing `FinalChain::call`
prematurely. Focused registration/delegation success and failure tests protect value injection and failed-transition
isolation while the existing finalized transaction suite protects behavior across the full mutation family.

The legacy audit confirms that call closure must be atomic across all 16 mutation branches. The follow-up transient
envelope must load exact requested-block account and DPoS snapshots, apply intrinsic plus action gas, reserve combined
gas and value, stage precompile value with rollback, preserve V2 and legacy batch outputs plus logs, and carry exact
business errors. It must not advance rewards, create receipts, run end-block cleanup, or publish snapshots. Blocks that
lack complete Rust account or DPoS snapshots remain a separate replay/migration or explicitly retained hybrid-routing
gap.

Kernel-foundation validation passed the focused register/delegate success and failure tests, the same-block
register-then-claim-all live-gas regression, all 757 `rustaxa-consensus` tests, `rewrite-validate-fast`,
`rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy
baseline, the bridge inventory guard, skill validation, Rust formatting, whitespace validation, and the repository
pre-commit hook, followed by independent configured review.

The next result-carrier slice preserves mutation success output without changing finalized receipts. Successful
`undelegateV2(address,uint256)` now retains its ABI-encoded `uint64` request ID alongside the existing log. Legacy
pre-fix `claimAllRewards(uint32)` retains the exact ABI `bool is_end` word for empty, non-final, exact final, partial
final, out-of-range, and wrapping-`uint32` page offsets, while current `claimAllRewards()` continues to return no bytes.
Established widened finalized page selection remains unchanged, including no mutation for wrapping batches that are
out of range on the native path; correcting selection has separate historical replay consequences and stays pending.
The finalized transaction boundary still discards contract return bytes after execution and publishes the same status,
gas, and logs. Exact typed business errors remain the next kernel-result requirement before atomic call routing.

Output-preservation validation passed the focused V2 ID and claim-all end-flag boundary tests, all 758 `rustaxa-consensus`
tests, `rewrite-validate-fast`, `rewrite-validate-final-chain`, `rewrite-validate-final-chain-parity`, the unchanged
46-warning consensus clippy baseline, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook, followed by independent configured review approval.

The typed-result slice now preserves exact legacy DPoS mutation business errors separately from ABI return bytes.
All reachable mutation validation branches carry a `DposContractError`; claim-all failures retain the exact validator
context and sequential first-error order without cloning canonical successful state, and registration proof recovery
matches the pinned no-CGO btcec compact-signature behavior, including dynamic recovery errors, zero-S and identity
recovery, and non-invertible-R hard failure. Delegate validation now preserves legacy maximum-before-minimum ordering,
while V2 aggregate-stake underflow, reward-graph faults, account inconsistencies, and impossible arithmetic remain hard
errors. Missing, unknown, retired, or otherwise ABI-unrecognized inputs remain untyped; genuine Cornus-gated method
rejections retain `Method not supported`. Finalized receipts still publish only the unchanged status, gas, and logs and
discard the typed reason for successful and ordinary typed business outcomes. Documented hard classifications instead
abort without a receipt, including V2 aggregate-stake underflow and non-invertible registration proof recovery. The
atomic transient call envelope is the next remaining DPoS mutation-call prerequisite.

Typed-result validation passed eight focused error, ordering, recovery, dispatch, and rollback regressions, all 766
`rustaxa-consensus` tests, `rewrite-validate-fast`, `rewrite-validate-final-chain`,
`rewrite-validate-final-chain-parity`, the unchanged 46-warning consensus clippy baseline, the bridge inventory guard,
skill validation, Rust formatting, whitespace validation, and the repository pre-commit hook, followed by independent
configured review approval.

The atomic mutation-call slice now routes all 16 DPoS mutation selectors through the shared kernel under a transient
transaction envelope. Calls clone the exact requested finalized account and DPoS snapshots, enforce legacy full
gas-cap affordability, validate intrinsic gas before combined value affordability, charge action gas, reject Cornus
nonpayable value before mutation, stage payable value and mutation-local reward/account changes, and return exact typed
business errors, ABI outputs, and logs. Every staged account and DPoS change is dropped for success, typed failure, and
hard failure; call execution does
not advance rewards, create receipts, run end-block cleanup, mutate nonces, or publish storage. Legacy dry-run nonce
replacement remains unobservable, so no nonce request field was added. Historical blocks missing either Rust snapshot
fail closed and remain a replay/migration or explicitly designed hybrid-routing gap. `CRW-07` records one narrow call-log
result-carrier delta reusing the existing EVM-log DTO, with no new handle or request export. `CRW-08` remains active for
the remaining older failed-contract receipt/parity families rather than call simulation.

Mutation-call validation covers all 16 selectors, exact historical selection and missing-snapshot failures,
gas-cap/value affordability and their legacy failure ordering, intrinsic and action out-of-gas, staged rollback, typed
errors, transient logs, V2 request ID output, both claim-all output shapes, and unchanged persisted snapshots. The Rust
consensus suite contains 768 passing tests, and all 56 `rust_consensus_tests` bridge cases pass, including the call-log
round trip. `rewrite-validate-fast`, `rewrite-validate-final-chain`, the authorized Tier 3
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook pass with the unchanged warning baseline, followed by independent
configured review approval.

The next bounded `CRW-08` evidence slice closes Cornus `undelegateV2(address,uint256)` pre-mutation business-failure
parity without changing production logic. Missing validators fail before delegation lookup; missing delegations fail
before amount validation; amounts above the delegation and nonzero remainders below `minimum_deposit` both retain the
legacy `Insufficient delegation` outcome. Every well-formed call charges calldata intrinsic gas plus the fixed 60,000
action gas, advances the sender nonce, publishes a status-zero receipt with empty logs/bloom, and rolls back DPoS
custody, stake, votes, reward state, V2 queues, and request IDs. A later same-sender transaction still executes.
Focused Rust tests protect the exact typed-error order and combined finalization transcript, while a restart-backed
dual-mode fixture protects receipt RLP, cumulative/header gas, gas-only balances, nonces, unchanged DPoS state, empty
header bloom, continuation, and durable receipts. There is no production, bridge, carrier, handle, module-flag,
snapshot-schema, migration, or `CRW-07` inventory delta. `CRW-08` remains active for V2 confirm/cancel failure
transcripts, the larger slashing-invalid-proof matrix, and any other older failed-contract receipt family established by
legacy audit.

V2 failure-evidence validation passed both focused Rust tests, all 770 `rustaxa-consensus` tests, the focused
Rust-enabled C++ fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the authorized Tier 3
`rewrite-validate-final-chain-parity`, the bridge inventory guard, skill validation, Rust formatting, whitespace
validation, and the repository pre-commit hook with the unchanged warning baseline, followed by independent configured
review.

The next bounded `CRW-08` evidence slice closes Cornus V2 request-consumption failure parity for
`confirmUndelegateV2(address,uint64)` and `cancelUndelegateV2(address,uint64)` without changing production logic. Both
methods first index the request by caller, validator, and ID, so a missing ID or wrong caller returns
`Undelegation does not exist`. Confirmation then rejects a still-locked request with
`Undelegation is not yet ready to be withdrawn`; cancellation instead checks the retained validator and returns
`Validator does not exist` when a pre-Magnolia terminal deletion left the request as custody history. Confirmation
intentionally does not require the validator and can still succeed after unlock.

The restart-backed dual-mode transcript creates one full V2 undelegation that deletes its zero-stake validator, then
proves missing and locked confirmations at intrinsic plus 20,000 gas, missing and validator-absent cancellations at
intrinsic plus 60,000 gas, and a successful later same-sender transfer. All four failures advance the sender nonce,
charge gas only, publish exact status-zero receipt RLP with empty logs/bloom, preserve escrow and the request/last-ID
state, and leave the validator absent across restart; all block-two receipts are reloaded and compared after restart.
A compact Rust test protects the exact typed-error order, wrong-caller alias, legacy text, and unchanged staged state.
No production, bridge, carrier, handle, module-flag, snapshot-schema, migration, or `CRW-07` inventory delta is
introduced. Corrupt Rust-only duplicate V2 keys remain a separate invariant-validation question rather than a
legacy-representable receipt branch. `CRW-08` remains active for the larger slashing-invalid-proof matrix and any other
older failed-contract receipt family established by legacy audit.

Validation passed the focused Rust request-consumption test, all 771 `rustaxa-consensus` tests, the focused restart-backed
dual-mode C++ fixture, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the Tier 3
`rewrite-validate-final-chain-parity` differential gate, `rewrite-bridge-inventory-guard`, skill validation, whitespace
validation, and the repository pre-commit hook. The configured `reviewer` returned `APPROVED` after the restart fixture
also reloaded and compared the exact block-two header, gas usage, and DPoS escrow balance.

The next bounded `CRW-08` evidence slice closes the ordinary well-formed semantic failure matrix for native
`commitDoubleVotingProof(bytes,bytes)` without production changes. It covers identical votes; separate period, round,
and step mismatches; equal block hashes with distinct unsigned vote hashes; both odd-next-step mixed zero/nonzero
orientations; invalid first and second signatures; distinct recovered validators; and a valid common signer absent from
the delayed validator view. Duplicate-proof behavior remains protected by the existing value-custody fixture.

The restart-backed dual-mode transcript submits every selected failure with nonzero value from consecutive sender nonces,
then executes an ordinary transfer. Each failure publishes exact status-zero receipt RLP at intrinsic plus 20,000 gas,
rolls value back, emits no logs, and preserves proof, jail, validator, DPoS, and slashing-account state. Cumulative/header
gas, gas-only sender debit, nonce continuation, accounts, receipts, and the exact block header remain identical after
restart. Compact Rust tests protect verifier branch coverage and delayed membership selection. There is
no production, bridge, carrier, handle, export, module-flag, snapshot-schema, migration, or `CRW-07` inventory delta.

Exact diagnostic text and combined-invalid precedence remain a separate production question: legacy consults stored
duplicate state before most semantic checks, Rust verifies the proof first, and finalized receipts publish neither
diagnostic. Malformed inner vote or sortition RLP is also excluded because legacy treats it as a must-decode hard boundary
while Rust currently normalizes the verifier failure to status zero. `CRW-08` remains active for those audited boundaries
and any other required FinalChain/DPoS family selected from the canonical queue.

Validation passed both focused Rust slashing tests, all 773 `rustaxa-consensus` tests, the focused fixture in both
Rust-enabled and pure-C++ binaries, `rewrite-validate-fast`, `rewrite-validate-final-chain`, the Tier 3
`rewrite-validate-final-chain-parity` differential gate, `rewrite-bridge-inventory-guard`, skill validation, whitespace
validation, and the repository pre-commit hook. The configured `reviewer` returned `APPROVED` for the final scoped diff.

The next bounded `CRW-08` slice restores the legacy must-decode boundary for malformed nested
`commitDoubleVotingProof(bytes,bytes)` payloads. The outer Solidity ABI envelope remains ordinary user input: malformed
offsets, lengths, or tails consume the recognized method gas and publish a status-zero receipt. Once both dynamic byte
arguments are extracted, however, legacy Go constructs each three-field `Vote` and its four-field embedded
`VrfPbftSortition` through `rlp.MustDecodeBytes`; malformed list shape, fixed-width signature or proof fields,
noncanonical framing, and trailing nested bytes panic across the EVM boundary and abort FinalChain without publishing a
receipt or block state. Rust now preserves that distinction through a typed `OuterAbi` / `NestedRlp` / `Semantic`
classification. Only `NestedRlp` propagates from the native transaction decoder as a hard finalization error; valid-RLP
slot, hash, signature, signer, and validator-membership failures remain ordinary status-zero contract outcomes.

Focused Rust tables cover outer framing, too-short and too-long vote and sortition lists, four-item weighted votes,
signature/proof widths, nested trailing bytes, and semantic rejection. The dual-mode FinalChain fixture proves an outer
failure receipt plus hard nested failures leave the finalized head, header, receipt, sender/slashing accounts, jail
block, and jailed-validator list unchanged. Exact duplicate-versus-semantic diagnostic precedence remains excluded:
legacy checks the durable duplicate key earlier, but both orders publish the same status, gas, empty logs/bloom, and
unchanged state, and diagnostics are not receipt data. No bridge handle, carrier, export, shim, module flag, snapshot
schema, migration, or `CRW-07` inventory delta is introduced. `CRW-08` remains active for any other required
FinalChain/DPoS family established by the canonical audit.

Validation passed both focused Rust classification tables, all 775 `rustaxa-consensus` tests, the focused
Rust-enabled FinalChain fixture, the standalone pure-C++ legacy death test, `rewrite-validate-fast`,
`rewrite-validate-final-chain`, and the Tier 3 `rewrite-validate-final-chain-parity` differential gate. The standalone
reference test uses GoogleTest re-exec mode and constructs its database and FinalChain only inside the child, preserving
the legacy process-abort assertion without forking live RocksDB or executor threads.

The next bounded `CRW-08` evidence slice closes the finalized `claimCommissionRewards(address)` ordinary-failure
transcript without changing production logic. A missing validator and a registered validator called by the wrong owner
both retain the legacy `This account is not owner of specified validator` contract failure. At Cornus and later,
zero-value well-formed calls reach those business checks and consume calldata intrinsic gas plus the fixed 20,000 action
gas; positive value is rejected earlier by the already-covered nonpayable boundary and therefore is not used to prove
owner-check ordering.

The dual-mode restart fixture submits both failures, selector-only malformed calldata, and a same-sender continuation.
The malformed recognized method also consumes intrinsic plus 20,000 action gas. Together they prove exact status-zero
receipt RLP, cumulative and header gas, gas-only balance changes, nonce advancement, empty logs and bloom, unchanged
DPoS account/validator/stake/delegation state, durable receipts, and restart-stable state. Focused Rust assertions
preserve the typed `WrongOwnerAcc` result for both owner branches, prove absent metadata takes precedence over a missing
stake row, verify staged account/DPoS rollback, and protect malformed-call continuation.

There is no production, bridge, carrier, handle, export, module-flag, snapshot, migration, or `CRW-07` inventory delta.
The audit separately identifies a generic pre-Cornus payable-envelope risk: legacy reserves the full gas cap and credits
call value before executing a payable DPoS mutation, while Rust currently checks value after used-gas charging and
applies commission-claim business logic before crediting incoming value. Normal fully backed state is equivalent, but
marginal affordability or undercollateralized/corrupt state needs its own bounded parity decision. `CRW-08` remains
active for that demonstrated envelope boundary or another older failed-contract family established by canonical audit;
all current-ABI methods are already Rust-routed.

Validation passed all six focused Rust commission-claim tests, all 776 `rustaxa-consensus` tests, the focused fixture in
Rust-enabled and pure-C++ binaries, `rewrite-validate-fast`, and the Tier 3
`rewrite-validate-final-chain-parity` target. The broad Rust-enabled `final_chain_test` phase continues to report the
pre-existing unrelated `native_dpos_redelegate_correction_applies_only_at_fix_block` dangling-cursor failure; the new
commission fixture and the complete pure-C++ suite pass.

The next bounded `CRW-08` slice closes the generic native payable-contract envelope divergence exposed by that audit.
After intrinsic validation, Rust now matches legacy EVM reservation ordering: a sender that can fund the full gas cap
but not the cap plus call value receives a status-zero receipt, consumes the full gas limit, advances its nonce, and
never enters the DPoS/slashing kernel. Executable calls stage sender-to-contract value before kernel dispatch so balance
checks observe the same state as the legacy precompile; an ordinary typed contract failure rolls back only that staged
payment while retaining nonce and gas effects. Hard errors remain block-atomic through the existing local snapshots.

A standalone dual-mode `delegate(address)` transcript fixes the marginal boundary at exactly
`gas_limit * gas_price + value - 1`. It proves full-cap gas and cumulative/header accounting, empty logs/bloom, no value
or delegation/stake mutation, nonce continuation, sender/recipient balances, durable receipt RLP, and restart state in
both Rust-enabled and pure-C++ modes. Existing payable-success and business-failure rollback fixtures continue to cover
the other two envelope branches. A Rust maximum-value case proves the affordability comparison cannot overflow into a
hard finalization error. No bridge, carrier, handle, export, shim, module flag, snapshot schema, migration, or
`CRW-07` inventory change is introduced. `CRW-08` remains active for any other required failed-receipt boundary found by
the canonical audit.

Validation passed all 777 `rustaxa-consensus` tests, the standalone fixture in Rust-enabled and pure-C++ binaries,
existing payable-success and typed-failure rollback fixtures, `rewrite-validate-fast`, and the Tier 3
`rewrite-validate-final-chain-parity` gate. The broad Rust-enabled FinalChain phase retains only the documented unrelated
redelegation-correction dangling-cursor failure; the full pure-C++ suite and parity target pass.

The next bounded `CRW-08` slice closes native full-gas-cap affordability and Cornus nonce parity. Legacy checks the
arbitrary-precision `gas_limit * gas_price` reservation before nonce, intrinsic-gas, or precompile decoding. An
underfunded sender is charged only `floor(balance / gas_price)` gas; value and contract state remain untouched. Before
Cornus the nonce does not advance, while Cornus advances every non-stale transaction to `transaction_nonce + 1`,
including skipped nonces; stale nonces remain unchanged. Rust now preserves that ordering for native DPoS/slashing
calls, treats a bounded `U256` multiplication overflow as unaffordable instead of aborting finalization, skips malformed
or state-dependent precompile decoding on the earlier failure, continues the transaction stream, and removes a newly
created empty pre-Cornus sender in line with EIP-161. The standalone dual-mode fixture proves pre-Cornus, equal-nonce,
and skipped-nonce receipts, affordable gas charging, no receiver mutation, persisted receipt RLP, and restart state.
Focused Rust coverage additionally proves stale nonce, multiplication overflow, absent-sender cleanup/materialization,
malformed-slashing precedence, and same-block continuation. No bridge, carrier, handle, export, shim, module flag,
snapshot schema, migration, or `CRW-07` inventory changes. `CRW-08` remains active for the next demonstrated parity
family; arbitrary-width account/transaction nonce domain types remain `CRW-09` debt.

The next bounded `CRW-08` slice closes the configured redelegation correction at its exact EndBlock activation order.
The activation-block same-validator transaction now executes against the still-corrupt legacy state before the
configured correction subtracts the historical inflation. Rust reward-graph checkpointing retains a stale-head node
when another delegation cursor still references it, then rebinds the validator head without moving that unrelated
cursor or changing stored reward counts. The one canonical activation replay may observe a regressed reward-per-stake
delta; matching legacy signed arithmetic, that transaction pays zero reward and still advances its affected cursor,
while every normal reward path continues to reject unsigned reward regression.

The existing standalone dual-mode fixture proves the pre-fix success, activation-block transaction-before-correction,
post-fix typed failure, corrected stake/votes, and restart persistence. Rust integration coverage adds a nonzero reward
pool plus distinct owner and redelegator cursors, proving the exact `333`, `0`, and `312` reward sequence and durable
reward-graph topology. This slice changes no CXX carrier, bridge handle/export, shim, module flag, snapshot schema,
migration, or `CRW-07` inventory. `CRW-08` remains active pending the broader failed-contract receipt and historical
snapshot/replay completion audit.

`CRW-04` completion record: the transaction/gas composition slice embedded the native gas-price
oracle and proposal gas limit in private transaction state. Production pool bids now inspect the Rust-owned
queue directly, storage-backed construction restores transaction count and gas history before publishing the service,
and the standalone `BridgeGasPricer` handle and its five exports are retired. The stable C++ `GasPricer` facade remains
for app/RPC/slashing consumers and delegates production work to `TransactionManager`; its storage-free combined runtime
exists only for standalone compatibility tests. The follow-up bounded cleanup retired `BridgeDagGraph`, `dag_shim`, and
their bridge-mechanics tests because the production DAG state already owns both native graphs. Direct legacy
`Dag`/`PivotTree` behavior remains pure-C++ reference coverage; Rust mode keeps native `DagGraph` unit coverage and all
production `DagManager` tests. The next manager/proposer slices made session startup derive and fingerprint frontier,
proposal-period, and period-hash observations inside the DAG runtime state, then moved storage-backed block
construction, timestamped unsigned-intent state, and canonical signed-RLP assembly into that keyed session. C++ now
collects external FinalChain/sortition facts, executes VDF, signs only the Rust-returned hash, and executes the returned
add-block payload. The standalone construction planner and unsigned/signed-intent bridge operations are retired.
The latest ownership slice replaced the standalone transaction-manager and DAG-manager bridge handles with one
App-owned `BridgeDagTransactionService`. Private sibling transaction and DAG states share Rust storage, production
`TransactionManager` and `DagManager` retain the same C++ RAII holder, and full construction restores both domains plus
the initial proposal-period mapping before publication. Transaction-only compatibility services reject DAG calls with
`DAG_SERVICE_UNAVAILABLE`; all service receivers are shared references, with mutation protected by the sibling mutexes.
The follow-up pack slice deleted the proposer request/report carriers and C++ sharded-payload relay. The service now
derives proposal/shard limits from the private DAG cursor, owner-binds the single transaction pack session, returns only
required EVM estimate candidates, and transfers selected canonical hash/RLP/gas payloads directly into the DAG cursor.
Composite transitions lock DAG before transaction and release both before C++ executes EVM estimates;
`TransactionManager::pack_mutex_` serializes that unlocked interval against public compatibility packing. Network
throttle observation, EVM execution, VDF, signing, and add-block execution remain explicit C++ boundaries. A subsequent
cleanup internalized finalized-DAG transaction expiry: Rust now commits DAG/storage cleanup and
then infallibly clears matching private transaction sidecars under the same DAG-then-transaction lock epoch. The expired
transaction hash list, C++ conversion/set construction, and DagManager-to-TransactionManager call are deleted; C++ sees
only finalized count and expired DAG hashes. Factory-only restore and initial proposal-mapping CXX exports are also
retired. The next composition step internalized verify-block transaction availability: query hashes remain private, Rust
prepares queue/sidecar/storage views without advancing, and a private C++ adapter materializes them and reads each sender
at the exact proposal period before a cursor-bound Rust completion applies finalized filtering and advances under
DAG-then-transaction locking. The old query-hash carrier, transaction report, public `getTransactions` relay, and report
export are deleted; a private friend adapter constructs cursor-bound, exact-proposal-period FinalChain account-nonce
facts without widening TransactionManager's public API. Proposer session start now also snapshots transaction queue and
non-finalized sidecar sizes directly from the sibling Rust runtime under DAG-then-transaction locking. The two count
fields and public TransactionManager getter relays are deleted from the CXX begin path; wallet/configuration and external
executor inputs remain explicit. Accepted DAG persistence is now also composed: a cursor-bound prepare/complete flow
revalidates the add plan and commits transaction rows/count plus DAG block/index/counters in one shared storage batch
before publishing graph or transaction runtime state. The C++ transaction-save, DAG-save, graph-add, and add-order
relays are deleted. DagManager now resolves the cursor's latest FinalChain nonce requests directly through its existing
FinalChain facade, so the DagManager-to-TransactionManager account-fact relay and its private helper surface are also
deleted; post-commit logs/materialization/events/gossip remain external. Verify-block gas completion now keeps tip
hashes and conditional canonical tip-gas loading inside the Rust verification session. C++ reports only the externally
executed block gas, aggregate transaction weight, and configured DAG/PBFT limits; its `needs_tip_gas` branch, the
`DagTipGas` CXX carrier, the gas-report tip vector, and the standalone tip-gas lookup export are deleted.
The final production-boundary audit found no remaining internal handle or state relay. Retained DAG-to-transaction calls
perform only public transaction materialization, narrow FinalChain fact collection, explicit EVM/network execution, or
logging; compatibility-only factories remain classified in the bridge audit. Current selection moves to `CRW-05`, the
next dependency-ready application-owner composition item, while `CRW-07` continues alongside each deletion slice.
`CRW-07` is cross-cutting and must be updated in the same commit whenever another item deletes or narrows bridge/shim
surface.

`CRW-05` current slice absorbs slashing planning and duplicate-proof state into the application-owned
`BridgePbftService`. The standalone `BridgeSlashingProofPlanner` handle and factory are deleted. The C++ slashing facade
keeps only FinalChain account-fact collection, gas-price lookup, transaction construction/signing, and transaction-pool
insertion, and reports the executor result back to the same PBFT-service state used by verified-vote admission.

The next `CRW-05` slice absorbs sortition runtime state into the application-owned `BridgeDagTransactionService`.
Full production construction restores DAG, sortition, and transaction state from one Rust storage owner before
publication; transaction/gas compatibility services expose neither DAG nor sortition capability. The standalone
`BridgeSortitionParamsManager` handle and factories are deleted, while the C++ facade retains only compatibility
materialization and the existing typed PBFT preview/stage/commit boundary. DAG verification and proposer sortition-fact
relays remain the next narrowing work for this owner.

The latest `CRW-05` slice absorbs external-EVM rewards-stat planning and publication state into Rust `FinalChain`.
Validated execution reports now build a session-owned rewards plan bound to the exact request, period, prior FinalChain
head, and rewards-runtime generation. Rust supplies only canonical distribution-stat RLP to the external C++
`StateAPI::distribute_rewards` adapter, attaches the storage mutation to its own publication plan, verifies matching
already-applied publications against durable rewards rows, and monotonically binds the live runtime to a head-stable
storage snapshot after durable publication or recovery. Planning fails closed while durable head and runtime head differ.
The production FinalChain overlay no longer constructs `rewards::Stats`, relays a
`FinalChainExternalEvmRewardsStatsUpdate`, or acknowledges/clears a second runtime. The standalone `rewards::Stats` and
`BridgeRewardsStatsRuntime` surfaces remain compatibility-test-only. The following `CRW-05` slice completes the
PBFT-owned pillar lifetime target.

The latest `CRW-05` lifetime slice moves the complete pillar runtime behind the application-owned `BridgePbftService`.
Full-service construction restores exactly one private pillar state; `PillarChainManager` replays startup data on that
same state and completes a pillar-specific readiness transition before live calls. Chain-only services fail with
`PBFT_SERVICE_PILLAR_UNAVAILABLE`, while a narrowly named partial service exists only for compatibility constructors and
tests. Production App injects its existing service, `BridgePillarChainRuntime` and its factory are deleted, and all pillar
receivers use sibling service locking without crossing C++ FinalChain, signing, network, event, or materialization
effects. PBFT's four pure current-anchor decisions now call the shared service directly; the public manager wrappers
remain compatibility adapters. `BridgePillarChainStorage` remains separately classified as a stable `DbStorage`
compatibility implementation and is not production pillar storage authority.

The next bounded `CRW-05`/`CRW-07` slice internalizes the PBFT-finalization sortition commit across the two
application-owned services. C++ still derives the finalized DAG/transaction counts needed by the Rust operation and
keeps the previewed optional change in the primary Rust storage batch, but it no longer commits sortition through the
compatibility facade or returns a six-field live-state report to PBFT. `BridgePbftService` validates the active cursor
and retained storage-stage change before `BridgeDagTransactionService` atomically publishes a cloned next sortition
state; stale cursors and preview mismatches leave sortition unchanged. The CXX report carrier and the rewrite-only C++
commit helper are deleted. A post-primary preview/stage divergence is fatal rather than retryable because duplicate
resume does not replay protected sortition mutation. DAG verification and proposer sortition-fact relays remain the next
narrowing work for this owner.

The latest `CRW-05`/`CRW-07` slice internalizes DAG VDF verification across the private DAG and sortition siblings of
`BridgeDagTransactionService`. The DAG cursor retains its full signed-block hash, cursor identity, action generation,
proposal period, and normalized vote counts. The composed operation snapshots those facts under the DAG lock, loads
historical sortition parameters under the sortition lock alone, verifies the proof without either lock, and advances
only after revalidating the unchanged cursor. C++ now supplies only the signed block payload/level and the external PBFT
period-hash and FinalChain VRF-key facts. The direct verifier DTO/export, VDF report carrier/export, explicit sortition
parameter lookup, and vote-count relays are deleted. Proposer sortition-fact relays remain the next narrowing work for
this owner.

The latest `CRW-05`/`CRW-07` slice internalizes the remaining DAG proposer sortition lookup and planning fact. The
proposer reports only the requested external FinalChain period/authorization facts. `BridgeDagTransactionService`
snapshots the keyed proposer cursor, loads the cursor period's historical sortition parameters without the DAG lock,
then reacquires `DAG -> sortition`, revalidates the exact cursor/action/observation/period, repeats the indexed lookup,
and compares every parameter field before planning. Lookup/decode/capability failures clean only the matching cursor;
parameter drift returns a stable stale-retry error without advancing. The exact Rust-selected legacy parameters are
retained in the cursor and exposed only on `StartVdf` as inputs to the accepted asynchronous C++ VDF executor. The C++
sortition facade lookup, inbound parameter relay, conversion helper, and external-facts API names are deleted. This
closes the named DAG/proposer sortition relays; the PBFT sortition-facade accessor/preview path now requires a separate
retirement audit before `CRW-05` can be marked complete.

The final `CRW-05`/`CRW-07` slice closes that audit and completes CRW-05. App injects its existing
`BridgeDagTransactionService` holder directly into `PbftManager`. On fresh finalization, Rust validates the service-owned
PBFT head, decodes canonical period-data RLP, derives the next non-empty chain size, rejects caller-owned sortition
stages, previews sortition, appends its own optional primary-storage stage, and retains the exact facts through durable
commit. The later cursor-only composed operation publishes the cloned sortition state or raises the existing fatal
post-storage invariant without publication; resume never replays preview or commit. The facade accessor/preview methods,
C++ fact and stage relays, commit request carrier, direct preview/commit exports, and bridge-mechanics tests are deleted.
Only classified public compatibility methods remain on the C++ sortition facade. With CRW-02 through CRW-05 consumer
migrations complete, the final CRW-06 storage-authority audit became dependency-ready; CRW-07 continues alongside it.

The bounded CRW-06 closeout found no remaining unclassified production consensus storage route. Native consensus and
storage crates contain no bridge-shaped storage handles, `rustBatchId` has no code call sites, and remaining
`BridgeStorage`, `BridgeStorageBatch`, storage-query-family, `BridgePillarChainStorage`, and `DbStorage` references are
limited to typed application/bootstrap construction, stable public compatibility, network/query, external FinalChain/EVM,
admin/migration, conformance, and test boundaries. `BridgeStorageBatch` is an opaque carrier inside the stable
`DbStorage::Batch` lifecycle; C++ compatibility callers still sequence typed append operations, while Rust owns
validation, key/column selection, batch storage, and atomic commit.
Standalone `rewards::Stats::processStats(..., Batch&)` remains public/test compatibility because replacing its append
semantics would break caller atomicity without reducing production authority. Further deletion belongs to the owning
caller migrations, not CRW-06. The storage-boundary and bridge-inventory guards, targeted symbol searches, skill/prompt
drift checks, and whitespace validation passed; configured `code-mapper` and `architect-reviewer` agents independently
confirmed the classification-only closeout. CRW-06 is complete.

#### CRW-01 selected composition boundary

`CRW-01` selected a PBFT-cluster-only Rust application service. Current code mapping did not find a wider DAG,
transaction, pillar, FinalChain, gas, or slashing root that would delete more active compatibility surface in the same
slice. Those runtimes have independent non-PBFT consumers and remain the separately ordered work in `CRW-04` and
`CRW-05`; collecting them behind one root now would create a service locator without reducing ownership ambiguity.

The `CRW-02` ownership graph is:

```text
C++ App bootstrap
└─ one shared Rust PBFT application service
   ├─ PBFT manager runtime, queue, and operation sessions
   ├─ PBFT chain head/state and storage-backed block lookup
   └─ one shared Arc<Storage>
      ├─ proposed-block state follows in CRW-03
      └─ verified-vote/admission state follows in CRW-03
```

The service is the only owner of manager and chain state. Its bridge handle must be held through one safe shared C++
lifetime owned by `App` and passed to the `PbftManager` and `PbftChain` compatibility facades; neither facade may borrow
an unowned nested Rust reference or independently restore state. Rust owns synchronization with separate manager and
chain lock domains and a documented lock order. Do not put the complete service behind one coarse lock, expose raw
manager/chain/storage handles, or add an arbitrary subsystem lookup API.

Bootstrap becomes one fallible service construction path. Rust restores the PBFT chain first, derives the manager's
current period and Cacti activation from the restored head plus immutable configuration, restores the manager runtime,
and returns a coherent service. `App` supplies durable lambda, hardfork, step, deadline, and polling configuration; it
no longer reads the C++ chain facade to populate `current_period` or `cacti_active_at_chain_size`. Startup replay remains
an explicit service bootstrap phase while replay still needs classified C++ executors, and live manager commands must
fail closed until that phase completes.

Existing manager and chain operation DTOs should move onto the service receiver before being redesigned. Chain reads,
legacy JSON projection facts, canonical block lookup, manager snapshots, lifecycle/proposal/sync/finalization commands,
and typed executor reports all address the same owner. Operation identities or generations continue to reject stale or
duplicate reports. PBFT-chain finalization mutation and manager-to-chain reads become native service operations: the
current `UpdatePbftChain` C++ dispatch followed by a report back into the manager runtime is an internal round trip, not
an accepted external executor boundary.

The public C++ `PbftManager` facade remains for app-host threads, timers, sleeps, network wiring, signing/VDF execution,
external effect execution, logging, and compatibility materialization. The public C++ `PbftChain` facade remains as a
narrow read/materialization view for DAG, votes, network/tarcap, RPC, stats, and tests. Its existing
`updatePbftChain(...)` method remains a public compatibility/test mutation adapter until direct callers migrate, but the
mutation executes against the service-owned chain rather than an independently authoritative handle. It is distinct
from the finalization-specific mutation/report bounce deleted below. `BridgeConsensusNetworkApi`,
`BridgeConsensusExecutionApi`, and `BridgeConsensusQueryApi` remain separate external facades. DAG, transaction, pillar,
FinalChain, gas, and slashing runtimes remain sibling services or typed executor ports and are never fetched from the
PBFT service.

The first `CRW-02` deletion/narrowing set is:

- production `create_pbft_manager_runtime_from_storage` and `create_pbft_chain_from_storage` construction routes;
- independent CXX ownership of `BridgePbftManagerRuntime` and `BridgePbftChain`, including
  `PbftManager::pbft_manager_runtime_` and `PbftChain::rust_chain_`;
- the standalone manager-runtime constructor parameter, `App`'s production use of the chain facade's direct `DbStorage`
  construction path, and the app-side chain-size/Cacti startup derivation;
- standalone `pbft_manager_runtime_*` and `pbft_chain_*` exports as their live callers move to the service;
- `PbftChainFinalizationUpdateReport`, `PbftChain::updatePbftChainForPbftFinalization`, and
  `pbft_manager_runtime_advance_finalization_pbft_chain` after finalization mutation is drained internally.

The public `PbftChain(addr_t, std::shared_ptr<DbStorage>)` and `PbftChain::updatePbftChain(...)` signatures remain stable.
Direct test and compatibility callers continue through explicitly classified construction and mutation adapters backed
by the service-aware path rather than an independently authoritative `BridgePbftChain`; remove those adapters only after
their callers are intentionally migrated. Lower native factories may remain Rust-private or `cfg(test)` when unit
coverage still needs them. The C++ `PbftChain` class itself is not a `CRW-02` deletion target because its external
compatibility consumers are still live. Normal finalization, duplicate resume, storage failure/crash recovery,
concurrent chain reads, shared-handle teardown, and bootstrap rejection are required targeted coverage for the composed
service.

#### CRW-02 PBFT application service implementation

`CRW-02` implemented the selected boundary as one exported `BridgePbftService`. The service owns the manager runtime,
period-data queue, sync/proposal/finalization sessions, PBFT chain state and block lookups, and the shared native storage
handle. Separate manager and chain locks preserve concurrent public reads; operations requiring both follow the
manager-before-chain order and do not retain a Rust guard across a C++ executor call.

Production construction is now one chain-first `create_pbft_service_from_storage` call. Rust restores the chain, derives
the manager period and Cacti activation from the restored head plus immutable `PbftServiceConfig`, then restores manager
state. The production service starts behind an explicit bootstrap gate. `PbftManager` completes the gate only after
startup replay, `initialState`, wallet eligibility, and pillar restart processing; daemon, proposal, and PBFT sync
session entry points fail closed before that transition.

`App` owns one shim-local `PbftService` RAII holder and shares it with the retained C++ `PbftChain`, `PbftManager`, and
`VoteManager` facades. The app bootstrap edit is an intentional guarded `RUSTAXA_ENABLE` exception in upstream-owned `app.hpp` and
`app.cpp`; pure-C++ construction remains unchanged and no main-only header is required when Rust mode is disabled. The
public `PbftChain(addr_t, std::shared_ptr<DbStorage>)` constructor remains a chain-only compatibility adapter using
`create_pbft_chain_service_from_storage`, and public `updatePbftChain(...)` mutates service-owned state. Both are covered
as compatibility behavior rather than independent production authority.

The exported `BridgePbftManagerRuntime` and `BridgePbftChain` handles, their production constructors, app-side startup
derivation, facade-owned boxes, `PbftChainFinalizationUpdateReport`,
`PbftChain::updatePbftChainForPbftFinalization`, and
`pbft_manager_runtime_advance_finalization_pbft_chain` are deleted. Finalization now drains the chain update inside the
service, validates the resulting head against the accepted finalization plan, and advances the existing manager cursor
before returning the next true external effect. The obsolete CXX `PbftManagerStartupFact` test carrier is also deleted;
Rust-only tests use a private fixture, while C++ bridge/storage tests seed a durable chain head and exercise the production
service constructor.

Focused coverage proves chain-first period/Cacti derivation, invalid configuration failure, bootstrap rejection and
one-way completion, shared chain visibility/lifetime, chain-only compatibility, internal finalization drain, Rust bridge
manager/sync behavior, the C++ public chain facade, single-node/null-anchor finalization, consensus bridge fixtures, and
storage lifecycle behavior. Required Tier 1, consensus Tier 2, bridge inventory, storage bridge, smoke, and upstream-file
diff evidence is recorded in the consolidation plan closeout for this slice.

#### CRW-03 PBFT-private state absorption

`CRW-03` is complete after dependency-ordered implementation sub-slices. The first moved proposed-block state into the
application-owned `BridgePbftService`, migrates production callers in the PBFT and vote-manager facades, replaces the
storage shim's independent proposed-block handle with storage-only compatibility operations, and deletes
`BridgeProposedBlocks`. Tentative wallet candidates remain a non-persisted Rust-local candidate set until leader
selection; they must never enter the authoritative proposed-block index before selection.

The second ownership sub-slice is implemented: verified-vote/admission state is restored into the same service before
publication, `BridgeVerifiedVotes` and its factory are deleted, and the retained C++ vote/network facades are service
clients. Coherent state, step, and current-reward materializers use owned snapshots from one vote-lock epoch. Proposed-
block absorption landed first so this change did not recreate an independently owned cross-shim handle.

The current service uses sibling Rust lock domains for manager, verified votes, proposed blocks, and chain state.
Production construction restores all four state families before bootstrap publication; chain-only compatibility
services carry no vote runtime and reject vote receiver calls explicitly. No service guard crosses C++ validation,
network, FinalChain/EVM, logging, or gossip callbacks.

The proposed-block sub-slice is implemented. Service construction restores canonical proposals before publication;
push and cleanup serialize durable-first mutation plus live publication under the Rust proposed-block write lock.
`ProposedBlocks` is now a lock-free C++ client view over the shared service, `VoteManager` no longer accepts a
`ProposedBlocks&`, and the former temporary `ProposedBlocks(DbStorage)` candidate index is replaced by one ordered
non-persisted Rust batch lookup. `DbStorage` save/snapshot compatibility uses stateless Rust storage functions, so no
second process-local index exists. `VerifiedVotes` likewise owns no runtime, storage handle, or mutex; Network still
reaches vote state through the stable `VoteManager` facade.

The final combined-operation debt is closed. Period advance now emits one `CleanupPeriodState` action and calls one
service operation with `finalized_chain_size` and its exact successor. The operation locks verified votes before proposed
blocks, plans both removals without mutation, commits all proposed-block deletes in one Rust batch, and only then prunes
both in-memory owners. Rejected validation or storage commit leaves both owners unchanged; empty cleanup publishes a
typed storage-free no-op. The former manager-only VoteManager cleanup wrapper and second planner action are deleted,
while individual cleanup APIs remain classified compatibility/test routes.

#### Scope-gated follow-up work

These items are tracked so they are not mistaken for forgotten consensus gaps, but they do not block the current
non-network/non-EVM consensus closeout. They require an explicit task-owner scope decision before implementation.

| ID | Status | Work | Unblock condition | Complete when |
| --- | --- | --- | --- | --- |
| `CRW-N01` | `blocked` | Implement the application arena network ingress/egress pipelines, finish PBFT gossip effect-drain integration, and fix the deferred vote packet duplicate-with-block delivery gap in the rewrite-side network path. | Explicitly start the network/tarcap rewrite boundary. | Rust owns packet inspection, consensus admission/routing, typed effects, and result validation; C++ tarcap owns only transport execution, wrapping, peer mechanics, and queue scheduling. |
| `CRW-E01` | `blocked` | Move concrete EVM/state execution, receipt execution details, arbitrary contract calls, or `state_db/` mutation into Rust. | Explicitly expand the accepted external-EVM boundary in `PLAN.md`. | A separately approved design and parity plan replaces the current `ConsensusExecutionApi` executor boundary without moving consensus authority back into C++. |

PBFT manager compatibility removal is tracked in the consolidated PBFT ownership boundary in `PLAN.md`. That plan treats
network/tarcap and EVM/state execution as the only long-lived C++ executor boundaries; all other PBFT manager shim and
bridge compatibility should move into Rust-owned runtimes, typed ports, or explicit public API materialization edges.

## Module Inventory

Note: inventory rows may mention C++ logging where the current shim emits legacy diagnostics. That is not a retained
ownership boundary. Treat those mentions as temporary observability only; deterministic decisions, state transitions,
payload construction, persistence, and protocol planning should still move to Rust when their real dependencies allow it.

Current PBFT scaffold status: the standalone feature-on facade no longer imports the original manager header, and
feature-on source selection excludes `pbft_manager.cpp` rather than compiling it as `PbftManagerOld`. Module-disabled
and pure-C++ builds still select the untouched original implementation.

| Module | Primary files | Approx size | Status | Proposed ownership | Notes |
| --- | --- | ---: | --- | --- | --- |
| DAG graph | `dag/dag.hpp`, `dag/dag.cpp`, `rustaxa-consensus::dag::DagGraph` | 424 legacy lines | `rust-owned` | Native graphs inside `DagManagerState`; legacy C++ graph is reference-only | `BridgeDagGraph`, `dag_shim`, and their CXX compatibility tests are retired. Rust-enabled production owns total-DAG and pivot-tree state privately inside `BridgeDagTransactionService`; native tests cover graph semantics. Rust builds continue excluding untouched legacy `dag.cpp`, while direct `Dag`/`PivotTree` cases compile only in pure-C++ reference mode. |
| DAG manager | `dag/dag_manager.hpp`, `dag/dag_manager.cpp`, `dag_manager_shim/*` | 1048 lines | partial | Private DAG state in `BridgeDagTransactionService` plus C++ executor/compatibility shell | The application service owns storage-backed private DAG graph/index state, persistence-backed startup restore and block loading, finalized-order application, non-finalized sync payload selection, runtime-derived proposer observations, accepted-block atomic DAG/transaction persistence, signed-RLP add-block fact decoding, and verify-block session ordering. Proposer session begin atomically derives frontier, proposal period, period hash, transaction pressure, and a fingerprint; external FinalChain/sortition reports are rejected as stale when the DAG observation changes, while VDF polling and stale-proof resume derive the live proposal level internally. The old direct frontier/proposal-period/attempt/count CXX inputs are deleted. `DagManager::addDagBlock` supplies compact block payloads and requested FinalChain nonce facts; Rust commits transaction rows plus DAG storage and graph state before returning only compatibility counters, log facts, and event/gossip intents. The standalone facade imports neither the original manager header nor `DagManagerOld`, and feature-on builds exclude the original `dag_manager.cpp`. Proposed DAG blocks enter through canonical signed RLP plus transaction payloads; Rust decodes block facts before planning and persists accepted transaction payloads before C++ materializes compatibility objects only after acceptance. `DagManager::verifyBlock` opens a Rust-owned session that owns precheck, transaction-query planning, transaction availability, VDF/DPoS reject ordering, gas reject ordering, and terminal status selection while C++ reports live transaction, FinalChain authorization, VDF verifier, and EVM gas-estimation facts. Tip gas facts for the PBFT aggregate gas check, DAG block knownness, and the legacy `pivotAndTipsAvailable` compatibility API load compact facts from private Rust state/storage instead of materializing C++ `DagBlock` tips or consulting compatibility caches. Remaining C++ surfaces are executor/compatibility boundaries: FinalChain/DPoS fact collection, EVM gas execution, network gossip, public/event `DagBlock`/`Transaction` and compatibility-cache materialization, counter mirroring for retained public views, and logging. |
| DAG proposer | `dag/dag_block_proposer.hpp`, `dag/dag_block_proposer.cpp`, `dag_block_proposer_shim/*` | 576 lines | `partial` | Standalone C++ executor facade with Rust proposer session | The Rust-mode overlay is a self-contained facade: feature-on builds exclude the untouched original source and contain no `DagBlockProposerOld` scaffold, while pure-C++ builds retain the original implementation. C++ still owns thread/network lifecycle, temporary signing, live add-block execution, and live network throttle checks. Transaction packing enters the Rust-owned TransactionManager pack session, and Rust owns proposer eligibility status decisions, atomic DAG observation/revalidation, legacy VRF input bytes, historical sortition selection, deterministic tip-selection policy, production proposal timestamps, canonical signed block RLP finalization, the legacy `selectDagBlockTips` compatibility surface through storage-backed Rust runtime planning, and the ordered `proposeDagBlock` session for skip reasons, transaction-pack command selection, transaction-pack throttle reporting, runtime-derived VDF wait/cancel and stale-proof decisions, add-block completion outcome, missing VDF input status, and retry-cursor updates. C++ no longer independently reads DAG or sortition storage or echoes those facts back to Rust; it collects only requested external FinalChain facts, then consumes Rust-selected parameters solely as an asynchronous VDF executor instruction. The proposer hands signed block RLP plus transaction payloads to `DagManager` instead of materializing `DagBlock`/`Transaction` objects locally. Remaining proposer gaps are explicit executor boundaries: temporary C++ signing, live add-block side effect execution, and worker/network lifecycle ownership. |
| Sortition params | `dag/sortition_params_manager.hpp`, `dag/sortition_params_manager.cpp` | 331 lines | `rust-backed` | Private sortition state in `BridgeDagTransactionService` behind a C++ compatibility facade | Deterministic efficiency/threshold runtime state and persistence route to `rustaxa-consensus::sortition` and native `rustaxa-storage` in master Rust mode. Full application construction restores sortition beside DAG and transaction state before publication; compatibility services fail fast when that capability is absent. PBFT finalization uses a two-phase contract: preview the Rust threshold transition without publishing live state, persist any emitted `SortitionParamsChange` inside the primary Rust-owned finalization batch, then commit the live Rust runtime only after storage succeeds and validate the emitted change matches the preview. The standalone Rust handle/factories and facade-owned box are deleted. The public `pbftBlockPushed(..., Batch&, ...)` signature remains for cross-mode API compatibility; Rust ignores the batch and persists through the service-owned storage handle. The C++ facade owns only the shared `SortitionParamsChange` carrier, canonical RLP codec, and typed compatibility crossings; the untouched legacy source is pure-C++-only, and the `SortitionParamsManagerOld` scaffold and redundant module flag are retired. |
| PBFT chain | `pbft/pbft_chain.hpp`, `pbft/pbft_chain.cpp`, `pbft_chain_shim/*` | 259 lines | `rust-backed` | PBFT application-service state behind a C++ compatibility view | `BridgePbftService` owns startup restore/default initialization, the in-memory head, block lookup, head updates, finalization mutation, and next-block validation. Production `App` shares one service between the manager and chain facades; the stable public `PbftChain(DbStorage)` constructor remains a chain-only compatibility/test adapter. C++ retains JsonCpp formatting and temporary `PbftBlock` materialization. Feature-on builds import or compile no `PbftChainOld`; pure-C++ builds retain the untouched original implementation. |
| Proposed blocks | `pbft/proposed_blocks.hpp`, `pbft/proposed_blocks.cpp`, `proposed_blocks_shim/*` | 178 lines | `rust-backed` | PBFT application-service state behind C++ materialization facade | `BridgePbftService` owns restore, membership, compact pivot metadata, validation flags, canonical RLP payloads, storage-first publication, and atomic stale-period cleanup behind a sibling Rust lock. `BridgeProposedBlocks`, its factory, the storage-shim-owned live handle, and the C++ facade mutex are deleted. `DbStorage` compatibility uses stateless storage functions; tentative wallet candidates use an isolated Rust-local batch lookup. The retained facade materializes temporary `PbftBlock` values only. |
| Period data queue | `pbft/period_data_queue.hpp`, `pbft/period_data_queue.cpp`, PBFT service queue API | 168 lines | `rust-backed` | Rust metadata owned by the PBFT application service | Admission rules, queued block-link/reward/pillar/cert-vote metadata, transaction metadata and payloads, previous-cert metadata, processable-size/period tracking, pop decisions, cleanup planning, and clear semantics route through `BridgePbftService`. The standalone queue CXX handle, shim overlay, and module flag are retired. C++ keeps live `PeriodData`, compatibility vote/transaction materialization at external boundaries, pillar-vote sidecars, and peer `NodeID` ownership. |
| PBFT manager | `pbft/pbft_manager.hpp`, `pbft/pbft_manager.cpp`, `pbft_manager_shim/*` | 3267 lines | `partial` | PBFT application service plus C++ lifecycle/executor facade | Rust-enabled builds exclude the untouched upstream manager source and expose only the shim-owned `PbftManager` facade. `BridgePbftService` owns manager scalar state, daemon/action/session cursors, period-data metadata, sync admission, proposal planning, block validation, transition persistence, PBFT chain lifetime/state, proposed-block state, and verified-vote state. Production construction restores chain, proposed blocks, verified votes, and manager state before publication and remains bootstrap-gated until C++ replay/restart work completes. Authoritative leader selection uses a service-owned vote/proposed/chain snapshot plus fingerprint revalidation around the external C++ block validator; no separate proposal-vote snapshot, per-vote proposed lookup, or C++ chain callback remains. Vote admission commits any required progress batch under the vote lock before publishing the transition and restores a bounded checkpoint on failure. Period advance cleans vote and proposed-block state through one storage-first service action. Finalization drains and validates the chain mutation internally; C++ executes only the remaining typed external effects for FinalChain/EVM, DAG, network, timers, signing, events, and compatibility materialization. |

PBFT manager proposal note: `PbftManagerProposalSession` now owns proposal candidate filtering from supplied
DPoS/sortition facts, FinalChain and extra-data skip status, null-anchor selection, DAG anchor selection, gas clipping,
closest-anchor DAG-order recompute requests, and canonical order-hash calculation. The Rust-mode overlay answers only
requested DAG-order/gas facts before materializing the returned proposal command through temporary `PbftBlock`/`PbftVote`
sidecars and the existing Rust-backed vote generation path. C++ still owns live wallet sortition checks, extra-data
materialization, DAG block lookup, candidate leader sidecar adoption, and network effects; FinalChain facts are collected
through typed Rust ports.

PBFT manager broadcast note: Rust now owns `broadcastVotes()` timing and counter decisioning through a typed broadcast
planner/report contract. The Rust-mode overlay supplies elapsed-time, lambda, threshold, and counter facts, executes the
selected period-vote or round-vote broadcast as a network executor boundary, and applies broadcast counters only after
Rust accepts the executor report. C++ still owns retained vote/sidecar resolution, packet wrapping, peer filtering, and
network send policy.

PBFT manager sync intake note: Rust now owns the outer synced-period drain sequence through
`PbftSyncQueueDrainSession`. The Rust-mode overlay asks Rust whether to clean old queue entries, pop/process another
candidate, push accepted period data, update network sync state, continue after drops, stop on an empty queue, or stop
after push failure. Candidate admission remains Rust-owned through the staged period-data runtime, including stale block
drop, previous-hash mismatch clear/report, FinalChain wait, reward/cert/pillar rejection, transaction warnings, and
accepted period data. C++ still materializes temporary `PeriodData`, vote sidecars, peer IDs, FinalChain waits, network
effects, and `pushPbftBlock_()` as executor boundaries.

PBFT manager period-advance/startup replay note: Rust now owns startup replay range selection and the ordered
`advancePeriod()` effect plan. The Rust-mode overlay supplies live height/finalization facts, executes only
Rust-requested replay loops and period-advance effects, and commits the long-lived runtime period snapshot only after
those executor effects complete. Empty bootstrap is an accepted no-replay plan, FinalChain-ahead startup facts are
rejected explicitly, and non-increasing runtime period commits are rejected. C++ still owns FinalChain waits, wallet
eligibility refresh, VoteManager side effects, timer fields, compatibility cleanup calls, `PeriodData` materialization,
and recently-finalized transaction sidecar hydration. PBFT manager compatibility mirror reduction is closed for the
current protocol-runtime boundary; remaining live sidecars belong to executor/public API compatibility or subsystem
model-port work.

PBFT manager compatibility mirror note: the first Slice 9 cut now reads the long-lived Rust manager runtime snapshot for
round, step, state, current-lambda, next-step-time, and executed-block scalar inputs after startup. `getPbftRound()` and
`getPbftStep()`, daemon tick facts, and transition-planner facts for delay/reset/filter/certify/finish/finish-polling
and loopback no longer use the C++ scalar mirrors as authority. State-action facts and the runtime action mismatch guard
now source `state_` through fresh Rust runtime snapshots, leaving the C++ `state_` field as a compatibility mirror updated
by Rust transition/snapshot application helpers. PBFT period remains PBFT-chain-derived while finalization can advance
the chain before the Rust runtime period commit. Dynamic-lambda compatibility mutation has also been reduced: the
obsolete shim-local `adjustDynamicLambda()` helper is gone, and Rust-mode finalization stages use Rust planner lambda
outputs before updating C++ lambda mirrors only after Rust storage accepts the dynamic-lambda stage. The Rust manager
runtime now records accepted dynamic-lambda stage outputs in its snapshot, `getRoundLambda()` reads round-one lambda from
that snapshot, and finalization dynamic-lambda planner inputs no longer read `rounds_count_dynamic_lambda_` /
`dynamic_lambda_` as authority. Successful next-vote status persistence now returns a Rust runtime snapshot that hydrates
the C++ compatibility flags, and active state-action facts read next-voted flags from runtime snapshots instead of the
C++ bool mirrors. Broadcast/rebroadcast counters now live in `PbftManagerRuntimeSnapshot`: startup seeds them as
one-based counters, committed reset-consensus transitions reset round counters in Rust, reward-counter reset and
force-broadcast route through a Rust runtime counter update, `broadcastVotes()` builds facts from runtime counters, and
accepted broadcast reports hydrate C++ compatibility mirrors from Rust snapshots. Cert-voted block metadata now also
lives in `PbftManagerRuntimeSnapshot`: startup records period/round/hash metadata from the Rust-owned recovery row,
successful cert-vote storage writes update runtime metadata before C++ changes its temporary block sidecar, transition
reset clears metadata in Rust, and transition/state-action planner facts read runtime metadata instead of
`cert_voted_block_for_round_`. The C++ cert-voted block object remains a temporary materialization sidecar for vote
placement and proposed-block APIs. DAG-order cache membership now also lives in the Rust manager runtime: proposal and
sync validation facts query Rust for cached-anchor metadata, while C++ keeps only the temporary materialized `DagBlock`
vector sidecar used by FinalChain/finalization execution. Sync queue tail metadata now also lives in the Rust-backed
`PeriodDataQueue`: queued entries carry the PBFT block hash, and PBFT manager chain-link facts read the last queued hash
from Rust metadata instead of materializing the last queued `PeriodData.pbft_blk` only to read its hash. C++ still owns
queued `PeriodData`, `PbftVote`, and peer `NodeID` payloads for processing and public compatibility. The queue-aware
syncing-period calculation now also routes through the Rust-backed queue metadata, with C++ supplying only the PBFT-chain
size executor fact. The queued-block-hash-versus-chain-hash fallback decision now also routes through Rust queue
metadata, with C++ supplying only the chain-derived PBFT period and last PBFT-chain hash executor facts. Proposed-block
pivot-hash and cached-validity metadata now also live in the Rust-backed `ProposedBlocks` index: PBFT leader-candidate
ranking reads those compact facts without reconstructing `PbftBlock` sidecars for already-valid candidates, and
materializes only the selected block or blocks that still require validation/executor/public API handling. Popped sync
queue candidates now also return Rust-owned period/hash/previous-hash/pivot-hash metadata, and `processPeriodData()`
uses those compact facts for PBFT sync admission and block-validation planning instead of reading the popped
`PeriodData.pbft_blk` sidecar for chain-link facts. Popped sync queue metadata now also carries Rust-owned
DAG-referenced transaction hashes and period-data transaction-list hashes, so `processPeriodData()` builds
transaction-query facts from compact Rust queue metadata instead of scanning the popped `PeriodData` DAG/transaction
sidecars for those hashes on each runtime plan. The popped queue metadata also carries previous-cert presence,
first-vote weight flags, and pillar-vote sidecar presence, so `processPeriodData()` no longer reads
`PeriodData.previous_block_cert_votes` solely to decide whether Rust should request reward-vote replacement or
`PeriodData.pillar_votes_` solely to classify required/not-required pillar data. The same popped queue metadata now also
carries the PBFT block final-chain hash and extra-data/pillar-block-hash presence, so `processPeriodData()` validates
FinalChain hash and PBFT extra-data admission from Rust-owned compact facts instead of reopening
`PeriodData.pbft_blk` for those fields. Popped queue metadata now also carries Rust-inspected period-data transaction
hash/sender/nonce facts, and `processPeriodData()` passes those facts to the Rust-backed TransactionManager
finalized-status checker instead of reopening `PeriodData.transactions` only to build finalized-warning inputs.
The invalid-state-root sync log now uses the popped Rust queue final-chain-hash fact instead of reopening the live block
sidecar after Rust queue metadata already supplied that fact. Sync cert-vote validation now also consumes the popped
Rust queue PBFT period/hash facts directly instead of reopening the live block sidecar only to compare vote period/hash,
choose strict-validation intervals, or log block identity.
Sync reward-vote validation now consumes popped Rust queue reward-vote hash metadata plus the Rust verified-vote runtime
instead of reopening the live block sidecar only to read the requested reward hashes; copied selected `PbftVote` objects
remain temporary previous-cert replacement payloads.
Sync pillar-vote validation now consumes popped Rust queue pillar-vote RLP bytes for Rust inspection and deterministic
bundle planning, while live `PillarVote` sidecars remain only for accepted insertion side effects.
Sync transaction finalization payloads now consume popped Rust queue transaction RLP bytes: after admission accepts,
`processPeriodData()` rematerializes temporary `Transaction` sidecars from those queued bytes and verifies them against
Rust-owned queued transaction-hash metadata before dispatching finalization. Sync cert-vote payloads now also consume
popped Rust queue PBFT vote RLP bytes: Rust selects either the next queued entry's previous-cert payloads or the final
queued block's cert-vote payloads, and C++ materializes temporary `PbftVote` sidecars only for VoteManager
validation/insertion and finalization dispatch. Remaining `PbftVote`, `PillarVote`, pillar-data, `PbftBlock`,
`DagBlock`, `PeriodData`, and `Transaction` materializations are executor or public API compatibility caches rather
than authoritative PBFT manager decision state; deleting them belongs to the VoteManager/PillarChainManager,
FinalChain/EVM execution, proposed-block public API, network/tarcap, and model-port tracks.

Slice 9 executor-shell closeout: PBFT next-step sleeps, ineligible-wallet polling, startup finalization waits, and
eligible-wallet period readiness waits are Rust-planned. DAG proposer worker retry delay is Rust-planned through the
worker-command planner. C++ remains the accepted host executor for OS threads, condition-variable wakeups, actual sleeps,
network/tarcap effects, key-manager signing execution, public-query compatibility loops, and EVM/FinalChain execution
dispatch until the future app-host, network, and EVM migrations.

| Verified votes | `vote_manager/verified_votes.hpp`, `vote_manager/verified_votes.cpp`, `verified_votes_shim/*` | 384 lines | `rust-backed` | PBFT application-service state behind C++ materialization facade | Unique-voter checks, voted-value weight aggregation, 2t+1 block mappings, period cleanup, and round t+1 markers route to `rustaxa-consensus::verified_votes` through `RUSTAXA_ENABLE_VERIFIED_VOTES`. Production `BridgePbftService` restores and owns `PbftVoteAdmissionRuntime` behind its vote mutex; the chain-only service has no vote runtime, and storage-free construction is private Rust test scaffolding. `BridgeVerifiedVotes`, its factory, the facade-owned box, and the C++ mutex are deleted. C++ reconstructs transient `PbftVote` objects only from owned compatibility results; full state, step buckets, current rewards, mutation outcomes, and authoritative leader candidates are captured under one vote-lock epoch. Leader validation reports are accepted only after the vote/proposed/chain fingerprint is rebuilt unchanged. Admission checkpoints only its replay delta, touched round, and payload entry; required progress rows commit before publication, and failure restores the checkpoint with all executor effects suppressed. Period cleanup is planned and published atomically with proposed-block cleanup by the service. Get-next-votes egress uses a Rust plan/build API returning ordered vote hashes and optimized bundle RLP bytes. The standalone C++ facade owns exact compatibility carriers but no authoritative state; Rust-enabled builds do not import or compile `VerifiedVotesOld`. |
| Vote manager | `vote_manager/vote_manager.hpp`, `vote_manager.cpp`, `vote_manager_shim/*` | 1145 lines | `partial` | Rust domain for validation/aggregation/storage/generation; C++ network/live sidecar shell | The Rust-mode `VoteManager` overlay now routes deterministic verified-vote state methods through the Rust-backed `VerifiedVotes` facade: `addVerifiedVote`, vote presence/uniqueness, verified-vote snapshots and counts, period cleanup, proposal-vote selection, next-round detection, 2t+1 block/bundle lookup, step snapshots, current period/round bundle persistence, reward-vote selection/materialization, get-next optimized egress planning/building, and network t+1 step reads. `addVerifiedVote` now enters a validation-backed Rust PBFT vote admission runtime: the shim collects FinalChain/key facts, Rust validates canonical PBFT vote RLP, returns explicit replay mutation facts and the authoritative calculated weight, derives compact progress facts, mutates the Rust-owned verified-vote index, applies threshold decisions through the same runtime handle, retains weighted storage payloads and unweighted slashing payloads, and commits required progress rows before publication, and returns one terminal executor report with Rust-owned peer-known, proposed-block sidecar, gossip, slashing, threshold, and PBFT-progress intents. C++ validates those intents against the temporary live sidecar and executes slashing/network effects through existing boundaries, may emit temporary logs from Rust statuses, and leaves peer/network execution with current packet/PBFT-manager callers until the ingress pipeline moves. VoteManager PBFT vote persistence now routes through VoteManager-specific `rustaxa-storage` bridge operations and Rust vote payload builders: locally generated own votes write Rust-generated weighted RLP into `latest_round_own_votes` through Rust before mutating the live own-vote sidecar, accepted-vote progress persists optional extra reward votes and latest-round 2t+1 bundles inside the admission service call before live publication, finalized reward-vote reset stages receive Rust-built raw weighted vote-bundle RLP, and own-vote cleanup appends exact vote-hash deletes to the caller-owned Rust batch before clearing live sidecars. Startup restore is owned by the storage-backed Rust runtime, which returns only compact facts needed to materialize the remaining public C++ sidecars. `validateVote`, `voteAlreadyValidated`, `getPbftTwoTPlusOne`, and `genAndValidateVrfSortition` no longer forward to `VoteManagerOld`: Rust owns validation/replay planning, canonical received-vote RLP inspection, recovered voter identity, signature and VRF proof checks, Rust-computed received-vote weight, replay-cache storage, sortition-threshold calculation, Rust-owned threshold lookup/current-period caching, and local proposer-sortition screening through the `VerifiedVotes` runtime facade. `generateVote` and `generateVoteWithWeight` now call Rust signed/weighted vote generation once and directly materialize the returned sidecar from Rust-generated RLP; the shim verifies local VRF output to hydrate the temporary C++ credential cache needed by PBFT proposer selection. `checkRewardVotes` now calls the Rust verified-vote runtime to build preferred-round and reverse-period reward candidates from Rust metadata, evaluate the PBFT block's requested hashes, and return selected retained weighted records in requested order; C++ only materializes those records when copied votes are requested. Verified-vote snapshots, reward-vote reads, and remaining 2t+1 bundle reads no longer skip missing live sidecars because the shim materializes from Rust-retained weighted payloads. Get-next-votes egress uses ordered Rust vote-hash plans plus peer-filtered optimized bundle builds, while C++ still owns peer filtering, tarcap packet wrapping, split size, send, and known-vote marking. C++ remains the executor for FinalChain/key-manager fact sourcing, live `PbftVote` sidecar compatibility, slashing transaction construction/submission from Rust-normalized payloads, reward-vote sidecar mapping, and current network effects; logging at these boundaries is not an ownership blocker. PBFT finalization reward-vote reset now sources the cert `2t+1` vote set through the Rust payload-backed verified-vote API before building the Rust storage stage, and its post-commit metadata update returns a Rust-validated live-action proof for period, round, block hash, and stale extra-vote cleanup. The verified-votes feature predicate always selects the standalone overlay and excludes legacy `vote_manager.cpp`; unsupported partial feature combinations fail configuration, while pure-C++ keeps the upstream implementation. Depends on FinalChain DPoS, VRF, slashing, storage, network. |
| Pillar chain manager | `pillar_chain/pillar_chain_manager.hpp`, `pillar_chain_manager.cpp`, `pillar_chain_manager_shim/*` | 427 lines | `partial` | Standalone Rust-mode overlay shell over PBFT-service-owned pillar state | Pillar-votes builds expose the shim-owned manager facade without importing or compiling `PillarChainManagerOld`; the now-unreferenced legacy `PillarVotes` implementation is also excluded, while module-disabled and pure-C++ builds retain both untouched implementations. The App-owned `BridgePbftService` owns canonical current/latest-finalized snapshots, current validator vote-count history, pillar-vote relevance/inspection/recovered-voter uniqueness/insertion, sync-bundle apply, block creation/linkage planning, and PBFT-facing finalization persistence/cleanup behind a pillar-specific readiness gate and sibling mutex. Production injects that shared service into the manager; the deleted standalone runtime survives only as historical tracker terminology. The Rust-mode FinalChain shim supplies bridge root/epoch reads from committed `StateAPI` bridge-contract calls for the finalized request block instead of throwing, and returns zero when the configured bridge contract has no committed code. C++ still owns external FinalChain DPoS fact sourcing, temporary `PillarBlock`/`PillarVote`/`PeriodData` materialization, signing, event emission, network requests, and compatibility publication. |
| Transaction queue | `transaction/transaction_queue.hpp`, `transaction/transaction_queue.cpp`, `rustaxa-consensus::transaction_queue` | 501 legacy lines | `rust-owned` | Private native Rust state inside the DAG/transaction service | The standalone C++ overlay, `BridgeTransactionQueue`, bridge module, feature flag, and shim test are retired. Private transaction state inside `BridgeDagTransactionService` exclusively owns deterministic queue metadata, payloads, ordering, replacement/demotion, expiry, purge, limits, gas threshold, known-cache, and overflow/drop state in Rust production mode. Rust builds exclude the untouched legacy C++ queue source; direct C++ queue cases remain pure-C++ reference tests. |
| Transaction manager | `transaction/transaction_manager.hpp`, `transaction/transaction_manager.cpp`, `transaction_manager_shim/*` | 837 lines | `rust-backed` | Private service-owned live sidecars, queue, and gas oracle plus C++ materialization/orchestration shell | Rust mode now uses a standalone `TransactionManager` overlay and does not compile, inherit, or construct `TransactionManagerOld`; the original header/source are clean reference-only code. The facade preserves public/shared-pointer identity and owns only locks plus the classified FinalChain/EVM, thread-pool, event, logging, and object-materialization shell. App-owned `BridgeDagTransactionService` state restores `TrxCount` and finalized gas-price history before publication and is authoritative for queue metadata/payloads, known-cache state, non-finalized/recently-finalized sidecars, gas-estimation cache policy, transaction count, gas-price policy, and persistence. The same service also owns private DAG state behind a sibling mutex; the two C++ facades never pass internal handles between them. Production pool bids derive the inclusion floor directly from the service-owned queue and proposal gas limit; no queue scalar crosses C++ before the oracle applies its configured floor. `packTrxs` candidate snapshotting, candidate scan, Rust-inspected candidate envelope facts for EVM input, declared-gas fit checks, invalid-estimate demotion mutation, accepted output ordering, accepted gas accounting, and stop decisions route through a Rust pack session under `RUSTAXA_ENABLE`; C++ now uses a Rust step protocol that returns either the next EVM estimate request or final selected RLP outputs, and materializes only final accepted outputs. A shim-owned guard serializes pack calls around the single Rust session while C++ performs EVM execution outside the transaction lock. `estimateTransactionGas` and `estimateTransactions` ask Rust for declared-gas shortcut, cache-hit, cache-miss, and cache-store decisions while C++ keeps EVM execution and result materialization. `isTransactionKnown` routes through a hash-only Rust query that derives queue-known and sidecar membership from Rust state, and public `insertTransaction` uses one typed Rust admission command that owns known-fast-path precheck, verification decisioning, latest FinalChain account sourcing, public status/message mapping, finalized-location mapping, queue mutation, and explicit event/log shell intents. Rust legacy transaction envelope inspection provides hashes, senders, nonces, gas fields, costs, intrinsic-gas coverage, signature validity, and canonical RLP payloads for verification, admission, packing, DAG persistence, finalized-status updates, recovery, and proposal-period lookup. Rust owns transaction storage batches, sidecar mutation, count updates, queue erasure, recovery validation, finalized-status cleanup, queue expiry, and account-nonce-based purging before returning typed receipts for the remaining C++ log/event sinks. Remaining C++ is classified shell work: locks, public transaction object materialization, event/log dispatch infrastructure, EVM gas-estimation execution, public transaction construction, and lifecycle wiring. |
| Gas pricer | `transaction/gas_pricer.hpp`, `transaction/gas_pricer.cpp`, `gas_pricer_shim/*` | 171 lines | `rust-backed` | Native gas oracle composed into private transaction service state behind a stable C++ facade | `BridgeGasPricer` and its standalone CXX exports are retired. Finalized-block history restoration, live finalized-block gas-price updates, minimum-price flooring, percentile bid selection, and queue-aware pool pricing are owned by private transaction state inside `BridgeDagTransactionService`. Production `GasPricer` delegates through `TransactionManager`; a storage-free transaction-only service is retained only for standalone facade compatibility tests. The facade does not import or compile `GasPricerOld`, and the untouched original implementation remains available only when the module flag is disabled. |
| Pillar block/votes | `pillar_chain/pillar_block.hpp`, `pillar_chain/pillar_votes.hpp`, matching `.cpp` files | 627 lines | `rust-backed` for vote aggregation | Native Rust domain behind the manager runtime | `rustaxa-types::pillar` mirrors `PillarBlock`, `ValidatorVoteCountChange`, `PillarVote`, `PillarBlockData`, optimized pillar-vote bundles, and current pillar data RLP/Solidity/hash shapes. `rustaxa-consensus::pillar_votes` owns verified-vote uniqueness, weighted aggregation, deterministic threshold selection, cleanup, inspection, and sync-bundle planning. Pillar-votes builds exclude the unused legacy `pillar_votes.cpp`; C++ retains only temporary `PillarVote` sidecars and compatibility materialization through the manager facade. Vote signing and JSON/RPC materialization remain later slices. |
| Pillar manager | `pillar_chain/pillar_chain_manager.hpp`, `pillar_chain/pillar_chain_manager.cpp`, `pillar_chain_manager_shim/*` | 629 lines | `partial` | C++ compatibility/executor facade over PBFT-service-owned pillar state | The App-owned `BridgePbftService` now restores and owns the single production pillar state behind an independent readiness gate and sibling mutex. Production `PillarChainManager` receives that shared service; the standalone runtime handle/factory are deleted. The full overlay retains FinalChain validator/bridge facts, network transport, signing, C++ block/vote materialization, storage-compatible payloads, events, and finalization effects as explicit executor boundaries. A pillar-only partial service is confined to compatibility construction/tests, and module-disabled/pure-C++ builds retain the untouched legacy manager and `PillarVotes` implementations. |
| Rewards stats | `rewards/block_stats.*`, `rewards/rewards_stats.*` | 407 lines | `partial` | Rust deterministic domain plus compatibility-only shim facade | Rust-mode production FinalChain owns one long-lived rewards-stats runtime for native and external-EVM finalization. External-EVM execution reports now prepare a request/head/generation-bound Rust plan, expose canonical distribution-stat RLP only to the C++ `StateAPI` adapter, attach cache mutation internally to the atomic FinalChain publication, audit already-applied durable rows, and reload runtime state after publication/recovery. The production FinalChain overlay no longer constructs or sequences `rewards::Stats`; the standalone shim and `BridgeRewardsStatsRuntime` remain only for the stable public compatibility API and focused tests. Native FinalChain continues to stage account mutation until finalization storage commits, credit supported transaction-fee and minted rewards, persist Aspen supply/yield and DPoS reward state, execute supported claims, and route receipt logs/blooms in Rust. C++ retains temporary `BlockStats` decoding solely at the external `StateAPI::distribute_rewards` boundary. |
| Slashing manager | `slashing_manager/slashing_manager.*`, `slashing_manager_shim/*` | 102 lines | `partial` | PBFT-service-owned Rust planner behind a C++ executor facade | Double-voting proof eligibility, Magnolia vote-A-period admission, canonical proof hash/cache, first funded submitter selection, contract address/gas/value envelope, and calldata construction route through slashing state owned by `BridgePbftService` under `RUSTAXA_ENABLE_SLASHING_MANAGER`. `VoteManager::addVerifiedVote` submits Rust-normalized unweighted evidence through that canonical service, and the standalone `BridgeSlashingProofPlanner` handle/factory are deleted. The facade no longer imports or compiles `SlashingManagerOld`; module-disabled and pure-C++ configurations retain the untouched original. C++ keeps FinalChain account reads, GasPricer bid, transaction signing, TransactionManager insertion, and the live-vote compatibility overload. Rust FinalChain uses legacy-compatible inclusive Magnolia/Cacti activation, including activation zero from genesis. |
| Key manager | `key_manager/key_manager.*` | 55 lines | `cpp-owned` | C++ initially | Small wallet/secret wrapper; not on critical rewrite path. |

TransactionManager DAG payload note: proposed DAG blocks that already carry canonical transaction RLP payloads now call
`saveTransactionPayloadsFromDagBlock()`, so Rust inspects and persists DAG transaction facts without first constructing
live `Transaction` objects. Live transaction materialization remains for public reads, EVM/gas execution, and network
gossip compatibility.

Current TransactionManager packing boundary: Rust now owns declared-gas and cached-gas handling inside the active
runtime pack session, so C++ receives a pack callback only when an EVM estimate is required. The stale standalone pack
planner FFI surface and explicit pack-finalize path have been removed; C++ cleans failed sessions through a Rust abort
entrypoint and still materializes only final selected RLP payloads. The shim-only public
`TransactionQueue::demoteToNonProposable` method has been removed because pack-session demotion now mutates the Rust
runtime queue directly.

Current TransactionManager read boundary: `getTransaction`, `getTransactions`, `getBlockTransactions`,
`getNonfinalizedTrx`, and `getPoolTransactions` consume Rust-owned transaction views that preserve request order and
duplicates while resolving queue, sidecar, pending-storage, finalized-regular, and finalized-system sources. C++ keeps
only transaction object materialization, locks, EVM execution, event mechanics, broader orchestration, and temporary log
emission.

## Public API Tracker

### DAG

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `Dag` / `PivotTree` | vertex/edge counts, leaves, ghost path, deterministic order, graph clearing | native hashes in Rust; Boost only in reference mode | native `DagGraph` tests; pure-C++ `dag_test` graph cases | Native Rust graphs owned by `DagManagerState`; no standalone Rust-mode C++ facade |
| `DagManager` | block known/get/verify/add, pivot/tip availability, ordering, frontier, non-finalized blocks, anchors, expiry, VDF message | `DbStorage`, `TransactionManager`, `PbftChain`, `FinalChain`, `Network`, `KeyManager`, config | `dag_test`, `dag_block_test`, `pbft_manager_test`, `full_node_test` | Private DAG state in `BridgeDagTransactionService` owns deterministic graph/order, verification sessions, finalized-order application, non-finalized sync selection, atomic accepted-block DAG/transaction persistence, Rust-storage cleanup, and proposer frontier/proposal-attempt planning; C++ remains an executor/compatibility shell for live fact sourcing, EVM gas execution, event/network/public object and compatibility-cache materialization, counter mirroring, logging, and network egress |
| `DagBlockProposer` | proposer lifecycle, propose block, select tips, proposer eligibility | `DagManager`, `TransactionManager`, `FinalChain`, `DbStorage`, `Network`, VDF | `dag_block_test`, `pbft_manager_test`, `sortition_test`, full-node tests | Rust proposer session owns eligibility, VRF input bytes, deterministic tip selection, transaction-pack command flow, proposal timestamps, VDF input/message bytes, wait/cancel/stale-proof decisions, retry-cursor updates, block construction planning, final signed-RLP construction after temporary C++ node-secret signing, and signed-RLP manager submission; C++ remains an executor shell for lifecycle, live network throttle checks, async VDF compute, node-secret signature execution, compatibility materialization, logging, and network egress |
| `SortitionParamsManager` | params lookup, DAG efficiency, interval recalculation, cleanup | `BridgeDagTransactionService`, config, `PeriodData`, VDF params | `sortition_test`, `rust_consensus_tests`, `sortition_params_manager_shim_test`, full-node lambda tests | Private service-owned Rust deterministic state; C++ compatibility/materialization shell |

### PBFT

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PbftChain` | head/hash/size reads, block lookup, update head, block validation | `DbStorage`, `PbftBlock` | `pbft_chain_test`, `pbft_manager_test`, `full_node_test` | Early Rust-backed PBFT state slice |
| `ProposedBlocks` | push, mark valid, lookup, presence, cleanup, old-block checks | `DbStorage`, `PbftBlock` | `pbft_manager_test` proposed-block cases | Rust domain plus storage adapter |
| `PeriodDataQueue` | push/pop/clear/size/period/last block/old-data cleanup | `PeriodData`, `PbftVote`, peer `NodeID` | `rustaxa-consensus` period-data queue tests, `rustaxa-bridge` PBFT manager runtime queue test, `pbft_manager_test`, full-node sync tests | Rust metadata queue is owned by `BridgePbftManagerRuntime` and carries block links, FinalChain hash, PBFT extra-data presence, reward-vote hashes, pillar-vote RLP bytes, transaction hashes and transaction identity facts, previous-cert flags, and pillar-presence facts; C++ still owns live payload sidecars |
| `PbftManager` | lifecycle, state machine, proposal generation, period/round/step, DPoS counts, sync queue, block validation, gossip, finalization, dynamic lambda | nearly every consensus subsystem | `pbft_manager_test`, `vote_test`, `pillar_chain_test`, `full_node_test`, Python integration | Full Rust-mode overlay with Rust-owned scalar runtime, daemon-tick/action cursors, sync-period admission, proposal selection, transition persistence, finalization planning/storage apply, bounded resume classification, and compact sync queue facts through PBFT block-fact/reward-vote/transaction/pillar-presence/pillar-vote-RLP metadata; C++ remains the live executor for network, FinalChain/EVM dispatch, object materialization, timers, and compatibility side effects |

### Votes and Eligibility

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `VerifiedVotes` | vote insertion, unique voter tracking, step/round/period lookup, 2t+1 voted blocks, cleanup | `PbftVote` | `vote_test`, `pbft_manager_test` | Rust domain vote aggregation |
| `VoteManager` | vote validation, generation, reward votes, two_t_plus_one thresholds, VRF sortition, current period/round | `FinalChain`, `PbftChain`, `KeyManager`, `SlashingManager`, `DbStorage`, `Network`, VRF | `vote_test`, `pbft_manager_test` | Rust-backed validation, verified-vote aggregation, replay protection, threshold caching, local vote byte generation, reward-vote selection, vote payload construction, and Rust-storage persistence; C++ still owns temporary live vote sidecars, FinalChain/key fact sourcing, network wrapping/gossip execution, and broader PBFT orchestration |
| FinalChain DPoS ports | `dposIsEligible`, eligible vote count, total vote count, validators eligible vote counts, validators total stakes, total delegated, yield, total supply, VRF key, selected DPoS/slashing precompile reads | FinalChain/state API/EVM | `rust_consensus_tests`, `final_chain_test`, `rpc_test`, `pbft_manager_test`, `state_api_test`, proposer tests | Partial: genesis snapshot is Rust-backed and block numbers are preserved through the shim/bridge. Rust finalization now appends snapshots for native-transfer blocks, `registerValidator`, `delegate`, `undelegate`, `confirmUndelegate`, `cancelUndelegate`, `undelegateV2`, `confirmUndelegateV2`, `cancelUndelegateV2`, `reDelegate`, `claimRewards`, `claimCommissionRewards`, current-ABI `claimAllRewards`, legacy pre-fix `claimAllRewards(uint32)` batches, and slashing `commitDoubleVotingProof` jailing, records post-Magnolia transaction-fee commission rewards by finalized DAG block author through the Rust rewards-stat planner using bridged cert-vote facts, credits the supported fee-reward total to the Rust DPoS contract account, persists the DPoS/account snapshots, rewards-stat interval cache, and legacy FinalChain log-bloom index atomically with finalized-block visibility, and reloads those snapshots/cache rows on startup. Selected DPoS/slashing precompile reads use Rust snapshots so current validator commission rewards, delegator reward pages, V1 and V2 undelegation queues, jail blocks, and jailed validators are visible even when eligibility APIs use a delegation-delayed snapshot. Genesis delegations now cross the bridge into a Rust validator/delegator stake ledger so undelegation, redelegation, and slashing validator checks are Rust-owned. Rust-backed shim reads now serve total delegated, Aspen yield, Aspen total supply, `getTotalDelegation(address)`, `getDelegations(address,uint32)`, `getUndelegations(address,uint32)`, `getUndelegationsV2(address,uint32)`, `getUndelegationV2(address,address,uint64)`, slashing `getJailBlock`, slashing `getJailedValidators`, and `withBlockBloom` from Rust storage. Stake mutations with pending delegator rewards now auto-claim in Rust before changing stake, and receipts for the supported DPoS/slashing subset include Rust-generated `ValidatorRegistered`, `Delegated`, `Undelegated`, `UndelegateConfirmed`, `UndelegateCanceled`, `UndelegatedV2`, `UndelegateConfirmedV2`, `UndelegateCanceledV2`, `Redelegated`, `RewardsClaimed`, `CommissionRewardsClaimed`, and `Jailed` logs in legacy order; claim-all gas is computed from the staged Rust DPoS delegation view. Missing historical snapshots, unsupported state/EVM transitions outside this subset, and full failed-receipt parity for older supported mutation paths remain explicit gaps instead of falling back. |

### Transactions

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| Transaction queue runtime | insert/erase/order/group/contains/size/purge/known tx/min gas price | Rust FinalChain and transaction facts through the private transaction state in `BridgeDagTransactionService` | native Rust queue/runtime tests, `transaction_manager_shim_test`, pure-C++ reference queue cases | Private native Rust queue owned by the application service; no standalone C++ production facade |
| `TransactionManager` | verify/insert/pack/get/finalize status/non-finalized recovery/gas estimation | `DbStorage`, `FinalChain`, thread pool, `DagBlock`, state API | `transaction_test`, `transaction_manager_shim_test`, `dag_block_test`, `pbft_manager_test`, `full_node_test` | Rust-backed packing, DAG persistence, finalized-status execution, storage-backed transaction lookup, Rust-owned transaction views for queue/sidecar/storage reads, non-finalized recovery reads, finalized filter/verification helpers, verification/validated-insert admission planning, Rust-owned queue payloads, typed Rust mutation command reports, and Rust-mode pending-transaction event emission with C++ materialization/estimation shell; continue moving lifecycle mutation before broader orchestration |
| `GasPricer` | gas price reads/calculation | composed `TransactionManager` runtime | `gas_pricer_test`, `transaction_manager_shim_test`, transaction/full-node tests | Stable C++ facade over the transaction runtime's native gas oracle; production pool mode derives queue policy without a C++ scalar echo, while standalone storage-free construction exists only for compatibility tests |

### Pillar, Rewards, Slashing

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PillarBlock` / `PillarBlockData` | RLP, hash, JSON, Solidity encode/decode, validator vote-count deltas | hashes, state API data | `pillar_chain_test` encoding/finalization cases | Rust domain and codec parity |
| `PillarVotes` | vote uniqueness, threshold accumulation, above-threshold selection, cleanup | `PillarVote` | Rust `pillar_votes` unit tests; `pillar_votes_shim_test`; future `pillar_chain_test` parity | Rust-backed overlay under `RUSTAXA_ENABLE_PILLAR_VOTES`; C++ owns live vote sidecars and Rust owns deterministic aggregation |
| `PillarChainManager` | create block, validate/generate/finalize votes, current/finalized block state | `FinalChain`, `DbStorage`, `Network`, `KeyManager` | `pillar_chain_test`, `full_node_test` | Full Rust-mode overlay with Rust pillar-vote relevance/inspection/recovered-voter insertion and first pillar-block planning slice; C++ still owns DPoS/root facts, signing, storage writes, event/network effects, live sidecars, and finalization orchestration |
| `rewards::BlockStats` / `rewards::Stats` | per-block stats, interval recovery/processing/cleanup | `PeriodData`, PBFT votes, DPoS total vote count, storage | Rust `rewards_stats` unit tests, `rust_consensus_tests`, `rewards_stats_test`, full-node reward paths | partial: Rust deterministic runtime produces legacy-compatible `BlockStats` RLP and Rust `FinalChain` now owns planning, cache mutation, durable audit, and runtime reload for both native and external-EVM publication. The external adapter receives only distribution RLP and materializes temporary C++ `BlockStats` for `StateAPI`; it cannot supply or commit rewards-cache state. The obsolete independent feature flag and `StatsOld` scaffold remain retired. Standalone `rewards::Stats` plus `BridgeRewardsStatsRuntime` are retained only for public compatibility tests. Remaining work is eliminating the temporary C++ `BlockStats` materialization boundary itself. |
| `SlashingManager` | double-voting proof submission | `BridgePbftService`, `FinalChain`, `TransactionManager`, `GasPricer` | Rust slashing planner tests and `StateAPITest.slashing` end-to-end submission/jailing coverage | PBFT-service-owned deterministic planner plus Rust-normalized unweighted vote evidence payloads; C++ executor shell until transaction construction/signing moves to Rust |

## First Slice: Rust DAG Graph

Status: `rust-owned` for the native graphs inside `DagManagerState`, and complete for the current DAG manager/proposer
orchestration boundary. The standalone Rust-mode C++ graph facade is retired: feature-on builds exclude the original
Boost graph source and expose no `DagOld`/`PivotTreeOld` or `BridgeDagGraph` surface. Pure-C++ builds still select the
untouched original implementation for direct `Dag`/`PivotTree` reference tests. Rust-enabled builds keep private DAG
state inside the application-owned `BridgeDagTransactionService` for deterministic in-memory state and storage-backed
manager decisions. Frontier, ghost path,
ordering, counters, anchors, period, expiry level, non-finalized indexes, minimum difficulty, pivot/tip availability metadata,
storage-backed persistence, finalized-order application, non-finalized sync selection, add-block planning,
proposed-block signed-RLP fact decoding, proposed DAG transaction payload persistence, proposer frontier facts,
runtime-owned proposer observation/revalidation, proposal-attempt planning, block construction planning, and deterministic `verifyBlock` reject decisions route through
that state. The Rust-mode `DagManager` shim now owns the `verifyBlock` flow directly for prechecks,
Rust-planned transaction lookup, Rust-storage-backed missing transaction RLP lookup for hashes not present in the live
pool, DAG VDF payload/difficulty/proof verification, legacy DAG VRF/VDF message construction, VDF/DPoS authorization
decision ordering, and Rust-backed gas policy decisions. The Rust-mode facade is fully detached from the legacy
manager compile scaffold; only pure-C++ builds select the original manager header and source.
Finalized DAG order application also now advances the
Rust `DagManagerState` directly, including empty-period advancement, and Rust-mode finalization cleanup asks Rust
storage for finalized-block counter facts, expired block hashes, and transaction cleanup hashes before Rust applies the
persistent cleanup. `DagManager::setDagBlockOrder()` now uses a single composed Rust apply call that resolves the anchor
level from Rust storage, computes finalization on a candidate state, applies finalized-block counter updates, expired
DAG deletes, and expired non-finalized transaction deletes in one Rust storage batch, commits DAG state, clears matching
private transaction sidecars, and returns only the finalized count plus expired DAG hashes needed by the retained
compatibility cache. Rust-mode non-finalized DAG sync reads the
Rust-owned period/index state rather than querying the old manager. Ordered non-finalized sync block selection now
returns a Rust-storage-backed payload with period, selected DAG block RLPs, and de-duplicated transaction RLP lookups;
C++ only reconstructs legacy `DagBlock` and `Transaction` objects for the public API. Non-finalized transaction query
planning and expired-block transaction cleanup selection also route through Rust hash plans. Remaining DAG manager C++
is classified as executor/compatibility work: FinalChain/DPoS fact sourcing, EVM gas execution, event/network/public
object and compatibility-cache materialization, counter mirroring for retained public views, logging, and network egress.

Target behavior:

- `Dag::hasVertex`
- Rust `DagGraph::add_vertex_edges` parity with C++ `Dag::addVEEs`
- leaf collection
- `PivotTree::getGhostPath`
- `Dag::computeOrder`
- vertex/edge counters and clear behavior

Rust design sketch:

- `rustaxa-consensus` has a `dag` module with explicit hash-keyed graph state instead of mirrored Boost graph types.
- Ordering is deterministic and covered by native Rust `DagGraph` unit tests plus production `DagManager` runtime
  coverage; there is no standalone graph bridge.
- The manager bridge uses fixed hash bytes and explicit conversion only for manager/proposer operations that cross the
  C++ executor boundary.
- `DagManager` now uses a Rust manager runtime for deterministic graph/order/verification/add/sync/proposer planning.
  C++ still executes external effects and compatibility materialization under `RUSTAXA_ENABLE`.

Required tests:

- Rust unit tests for graph insertion, leaves, reachability, ghost path, and deterministic order. Landed.
- Rust bridge tests for the production DAG manager/proposer runtime. Landed in `rustaxa-bridge`.
- Proposer-session tests cover runtime-derived observations, stale external-fact rejection, missing periods,
  out-of-order reports, runtime-derived VDF cancellation/resume, and independent wallet sessions.
- Rust-mode C++ production coverage through all `DagManager` cases in `dag_test` and through `dag_block_test`.
- Direct `Dag`/`PivotTree` regression cases remain pure-C++ reference coverage; Rust mode intentionally has no
  standalone C++ graph facade.
- Rust `verifyBlock` coverage for tip count/uniqueness, missing proposal-period mapping, expired block, transaction
  availability, VDF/DPoS authorization decision ordering, and gas-policy decisions. Landed in `rustaxa-consensus` and
  `rustaxa-bridge`; the shim now passes an explicit status-coded Rust VDF/DPoS fact envelope instead of encoding separate
  authorization branches in C++. DPoS/VRF facts are collected through a Rust FinalChain bridge bundle. Rust now decodes
  the DAG VDF payload, verifies the embedded VRF proof, calculates sortition difficulty, and verifies the Wesolowski
  proof against the exact legacy ASCII-hex modulus bytes used by C++ `VdfSortition`. Rust also builds the legacy
  `level + proposal-period-hash` VRF input, `pivot + transaction-hashes` VDF message bytes, and the verify-side VDF
  sortition denominator from Rust FinalChain config. The path no longer requires a `DagManagerOld::verifyBlock` method
  forward, and it no longer derives VRF output, VRF input, DAG VDF messages, or per-block verify-side VDF denominator
  policy through C++ consensus helpers. Producer-side `DagBlockProposer` now uses a standalone Rust-mode overlay facade
  with no feature-on original source or Old scaffold and
  Rust proposer session for proposer eligibility status decisions, legacy VRF input construction, deterministic tip
  selection, transaction-pack command flow through the Rust-owned `TransactionManager` pack session, VDF input/message
  bytes, wait/cancel/stale-proof decisions, retry-cursor updates, proposal timestamps, block construction planning,
  final signed-RLP construction after temporary C++ node-secret signing, and manager submission through signed RLP plus transaction
  payloads. C++ keeps the live thread/network shell, live network throttle checks, async VDF compute execution,
  node-secret signature execution, compatibility materialization, logging, and network egress.

Open questions:

- Whether `computeOrder` must preserve every Boost traversal tie-breaker or only the externally visible block order.
- Whether direct C++ legacy `Dag` linking can be added to `rust_consensus_tests` without duplicate dependency symbols, or
  whether parity should stay fixture/transcript based.

## Validation Matrix

| Change area | Minimum validation |
| --- | --- |
| Rust consensus domain only | `cargo fmt --manifest-path rust/Cargo.toml`, `cargo clippy --manifest-path rust/Cargo.toml`, `cargo test --manifest-path rust/Cargo.toml` |
| DAG graph routing | Native Rust DAG tests plus production `dag_test`, `dag_block_test`, and manager/proposer bridge coverage; direct `Dag`/`PivotTree` cases remain pure-C++ reference tests |
| DAG proposer routing | Rust validation plus `rust_consensus_tests`, `dag_block_test`, and proposer-path full-node or PBFT coverage when orchestration changes |
| Sortition params routing | Rust validation plus `rust_consensus_tests`, `sortition_test`, and `sortition_params_manager_shim_test` |
| PBFT chain/proposed-block/queue routing | Rust validation plus `rust_consensus_tests`, `pbft_chain_test`, `pbft_chain_shim_test`, `proposed_blocks_shim_test`, the Rust PBFT manager runtime queue test, and relevant `pbft_manager_test` cases |
| Vote aggregation/eligibility | Rust validation plus `rust_consensus_tests`, `verified_votes_shim_test`, `vote_test`, relevant `pbft_manager_test`, and DPoS/state API coverage |
| Ingress message inspection/enrichment planning | Rust validation plus message-shape unit tests and C++ parity/golden-vector coverage for each routed ingress message kind; add scheduler or egress-event tests once the network pipeline lands |
| Transaction queue behavior | Native Rust queue/runtime validation plus `transaction_manager_shim_test`; pure-C++ queue-focused `transaction_test` and `gas_pricer_test` preserve legacy reference coverage; run affected DAG/PBFT tests when manager/proposer packing changes |
| Slashing proof planning | Rust byte-level proof-hash and calldata fixtures plus `StateAPITest.slashing`; richer C++ legacy vote/submission transcripts remain useful when available |
| Pillar vote aggregation and sync bundle validation | Rust validation plus `rust_consensus_tests` and `pillar_votes_shim_test`; broaden to `pbft_manager_test` and `pillar_chain_test` when manager behavior is touched |
| Pillar/reward behavior | Rust validation plus `pillar_chain_test`, `rewards_stats_test`, and affected full-node tests |
| PBFT manager state machine | Targeted PBFT/vote/DAG tests plus full-node smoke and Python integration coverage as needed; feature-on source/archive audits now prove the original manager and `PbftManagerOld` scaffold are absent, while `rust_consensus_tests` records CXX bridge transcripts for daemon tick action order/restart/reset behavior, finish-polling state-action effects, staged sync admission through accept, finalization runtime and duplicate/restart resume actions, crash-window resume classification, period-advance effects, storage-backed startup snapshot restore, and the completed PBFT manager closeout boundary |

## Current Open Items

| Item | Status | Owner decision needed |
| --- | --- | --- |
| Replace temporary DPoS query behavior | `partial` | Genesis DPoS vote-count, eligibility, validator total stake, validator eligible vote-count queries, total delegated, Aspen yield, Aspen total supply, `getValidator`, `getValidators`, `getValidatorsFor`, `getTotalDelegation`, `getDelegations`, V1 and V2 undelegation queue reads, `claimRewards`, `claimCommissionRewards`, current-ABI `claimAllRewards`, legacy pre-fix `claimAllRewards(uint32)`, slashing `getJailBlock`, slashing `getJailedValidators`, `withBlockBloom`, validator metadata/commission setters, Rust-executed double-vote jailing, and the `delegate`/`undelegate`/`confirmUndelegate`/`cancelUndelegate`/`undelegateV2`/`confirmUndelegateV2`/`cancelUndelegateV2`/`reDelegate` stake-mutation subset are Rust-backed for account/DPoS state. Rust-finalized native-transfer and supported DPoS/slashing mutation blocks now carry forward snapshots, post-Magnolia fee commission rewards, supported DPoS contract fee-balance credits, fixed/dynamic minted reward pools, direct delegator reward claims, commission reward claims, claim-all dynamic gas from staged DPoS state, owner-update and V2 undelegation failed receipts, auto-claim cursor updates, legacy ABI receipt logs for supported DPoS/slashing events, header blooms derived from those logs, legacy log-bloom index chunks, and restart-durable DPoS/account snapshot sidecars with a persisted validator/delegator stake ledger, validator insertion order, ordered V1 queues, V2 undelegation queues with per-delegator IDs, last commission-change blocks, F1 reward cursors, Aspen supply state, slashing jail blocks, jailed-validator order, and duplicate double-vote proof keys. All 25 current Solidity DPoS ABI methods have native Rust routing, and all 16 mutation selectors execute transiently through `FinalChain::call` with outputs, logs, errors, and rollback. Remaining gaps: full failed-contract receipt parity for older supported mutation paths and replay/migration or an explicitly designed hybrid route for historical blocks without complete Rust snapshots. |
| Create Rust DAG graph module | `complete` | Native `DagGraph` is owned by `DagManagerState`; the standalone CXX handle, overlay shim, and bridge-mechanics tests are retired. Pure-C++ reference builds retain the untouched original graph. |
| Route sortition params through Rust | `rust-backed` | Landed under master `RUSTAXA_ENABLE`; native Rust storage owns startup and finalized-period persistence. The public `Batch&` parameter remains only as a cross-mode compatibility signature. |
| Route verified votes through Rust | `rust-backed` | Landed under `RUSTAXA_ENABLE_VERIFIED_VOTES`; C++ shim preserves live `PbftVote` ownership while Rust owns deterministic index semantics and 2t+1 metadata. VoteManager Rust mode now consumes Rust-owned verified-vote state directly from the shim overlay for insertion, lookup, snapshots, cleanup, round advancement, and current-round 2t+1 bundle persistence. |
| Route DagManager verify/finalization/add/sync orchestration through Rust/shim | `partial` | Tip count/uniqueness, proposal-period availability, expiry, transaction availability, verify-side transaction query planning, verify-side missing transaction RLP lookup for hashes not present in the live pool, DAG embedded-VRF/VDF payload/difficulty/proof verification, legacy DAG VRF/VDF message construction, verify-side VDF max-vote ceiling selection, VDF/DPoS authorization ordering, gas-policy decisions, finalized DAG order application, empty-period advancement, expiry-level advancement, transitive expired-block classification, counter-update planning, storage-backed finalized counter facts, Rust-batched finalized counter writes, Rust-storage expired block removal, Rust-storage expired non-finalized transaction removal, non-finalized DAG sync period/index reads, ordered non-finalized sync block selection, Rust-storage-backed selected DAG block RLP loads, sync transaction RLP lookup, non-finalized transaction query planning, expired-block transaction cleanup selection, add-block duplicate/expiry/reference planning, signed-RLP proposed-block fact decoding, proposed DAG transaction payload persistence, proposer frontier and transaction-pressure facts, and proposal-attempt planning route through Rust. Remaining C++ is classified as executor/compatibility work: FinalChain/DPoS fact sourcing, EVM gas execution, event/network/public object and compatibility-cache materialization, counter mirroring for retained public views, and logging. |
| Route DagBlockProposer policy through Rust/shim | `partial` | The standalone overlay facade no longer imports or compiles `DagBlockProposerOld` in feature-on builds. Rust owns proposer eligibility status decisions, legacy VRF input construction, deterministic tip selection, transaction-pack command flow through the Rust-owned TransactionManager pack session, proposal timestamps, VDF input/message bytes, VDF wait/cancel/stale-proof decisions, retry-cursor updates, block construction planning, final signed-RLP construction after temporary C++ node-secret signing, and manager submission through signed RLP plus transaction payloads. DAG proposer FinalChain-height and DPoS authorization collection enters through a typed Rust `BridgeFinalChain::get_dag_proposer_final_chain_facts` port instead of a C++-constructed PBFT fact request. Remaining C++ is classified as executor/compatibility work: proposer worker lifecycle, live network throttle checks, async VDF compute execution, node-secret signature execution, compatibility materialization, add-block effect execution, logging, and network egress. |
| Retire standalone TransactionQueue facade | `complete` | The overlay shim, standalone bridge handle/module/CXX exports, feature flag, and shim-only tests are deleted. The private transaction state in `BridgeDagTransactionService` is the only Rust-production queue owner, and Rust builds exclude the untouched legacy source. Native Rust/runtime tests replace facade coverage; direct legacy queue tests remain pure-C++ only. |
| Route TransactionManager packing, storage reads, and DAG transaction persistence through Rust/shim | `rust-backed` | Rust runtime pack session landed for `packTrxs` candidate snapshotting, candidate scan, Rust-inspected candidate envelope facts for EVM input, declared-gas fit, invalid-estimate demotion mutation, accepted output ordering, accepted gas accumulation, and stop rules. C++ now drives `packTrxs` through a Rust step protocol that asks only for required EVM estimates, returns final selected RLP outputs, and clears the session without a C++ finalize step; a shim-owned guard serializes concurrent pack callers around the single Rust runtime session. `BridgeDagTransactionService` now owns private transaction state for live queue metadata/payloads, known-cache state, non-finalized and recently-finalized sidecars, authoritative transaction count, and bounded opaque gas-estimation cache state; the standalone runtime handle and factories are retired. TransactionManager Rust-mode height facts for verification, admission, queue insertion, packing, and estimate recording now read `BridgeFinalChain::get_last_block_number` directly instead of calling the C++ `FinalChain::lastBlockNumber` API. `estimateTransactionGas` and `estimateTransactions` no longer forward to `TransactionManagerOld`; Rust plans declared-gas shortcuts, cache hits, cache misses, and cache insertion while C++ still executes FinalChain/EVM and decodes cached `ExecutionResult` RLP. Shared Rust legacy transaction envelope inspection now owns canonical RLP hashing, sender recovery/signature validity, system sender identity, nonce/gas/value/cost facts, and intrinsic-gas coverage for TransactionManager Rust-mode inputs. `saveTransactionsFromDagBlock` is now only a public-object adapter into `saveTransactionPayloadsFromDagBlock`; Rust re-inspects canonical transaction payload bytes, sources latest account nonces from Rust FinalChain, then performs sidecar membership checks, duplicate filtering, nonce-gated finalized-storage lookup, accepted ordering, count planning, accepted storage batching, accepted non-finalized sidecar insertion, accepted queue erasure, and direct-hash queue-erased receipt construction for C++ logging. `getTransaction`, `getNonfinalizedTrx`, and `getNonFinalizedTransaction` materialize Rust sidecar RLP hits before generic Rust storage misses are resolved with explicit source classification. `getTransactions` and `getBlockTransactions` now use a Rust proposal lookup for storage misses that resolves transaction payloads, verifies stored RLP hashes, inspects legacy transaction sender/nonce identity in Rust, and filters old finalized hits against Rust FinalChain block-scoped account snapshots before C++ materializes accepted payloads; `getPoolTransactions` and `getAllPoolTrxs` materialize from Rust-owned queue views/payload groups at public/test edges. `recoverNonfinalizedTransactions` now loads hash/RLP recovery entries through Rust, removes stale finalized pending rows in Rust storage, validates survivor payload hash/sender through Rust envelope inspection, and inserts survivor payloads into Rust runtime sidecars without returning count mirrors or lifecycle notices. `initializeRecentlyFinalizedTransactions` now enters one Rust runtime command over finalized hash/RLP payloads, and `updateFinalizedTransactionsStatus` treats `PeriodData` as an edge adapter before Rust re-inspects canonical bytes; Rust plans count increments, stale recently-finalized eviction, current-period recently-finalized sidecar insertion, non-finalized sidecar removal, known-cache marking, queue erasure, typed finalized-status direct-hash receipt construction, and periodic finalized-account queue purge execution while persisting `TrxCount` before C++ logs side effects. Block-finalized queue expiry now calls the Rust runtime mutator directly, and periodic finalized-account purge reads sender account nonces from Rust FinalChain before mutating the Rust queue. `excludeFinalizedTransactions` and `verifyTransactionsNotFinalized` now use Rust-owned sidecar membership instead of C++ map facts; verify-not-finalized also sources latest sender account nonces from Rust FinalChain. `verifyTransaction`, `insertTransaction`, and `insertValidatedTransaction` now derive transaction envelope facts in Rust before entering typed Rust runtime admission command reports: Rust prechecks known state, performs public verification decisioning, sources latest account facts and finalized-location facts from Rust FinalChain, supports storage-completed admission, performs validated proposable/non-proposable queue mutation, returns explicit event/log shell intents, and maps public statuses/messages. The stale generic lifecycle/action report bridge has been removed. The Rust-mode facade owns the public `transaction_added_` event surface and emits it from shim-owned code only when requested by a Rust admission shell intent. Remaining C++ is classified shell work: event dispatch infrastructure, EVM gas-estimation execution, public transaction construction, final selected transaction materialization, and lifecycle wiring. |
| Route SlashingManager proof planning through Rust/shim | `partial` | Rust planner landed under `RUSTAXA_ENABLE_SLASHING_MANAGER` for report flag gating, Magnolia activation, PBFT slot equality, canonical double-vote proof hash/cache, unweighted vote evidence payload normalization, submitter selection, contract envelope, and calldata construction. The standalone facade no longer imports or compiles `SlashingManagerOld`, while the flag remains for valid partial configurations. Rust rejects pre-Magnolia evidence while accepting the activation period, and Rust FinalChain verifies legacy PBFT double-vote calldata, recovers offending validators, applies Magnolia/Cacti jail times, emits `Jailed`, stores duplicate proof keys and jail state in Rust DPoS snapshots, and filters DPoS eligibility/total vote counts from that Rust jail state. C++ still owns live vote object selection, FinalChain account reads for proof submission, gas bid, transaction signing, and transaction-manager insertion; duplicate cache marking occurs only after successful transaction insertion. |
| Route PillarVotes aggregation through Rust/shim | `rust-backed` | Private pillar state inside `BridgePbftService` owns period initialization, uniqueness, weighted aggregation, threshold subset selection, cleanup, canonical vote inspection, payload retention, and PBFT-facing finalization lookup/cleanup. Synced bundles and single-vote/finalization preparations retain their generation-bound tokens across the one-time readiness transition. The standalone runtime handle/factory and old receiver exports are deleted; chain-only services fail explicitly, and the compatibility-only partial service is not reachable from App production. C++ retains external FinalChain DPoS reads, signing, tarcap transport, legacy vote/period-data materialization, and event execution only. |
| Close PBFT manager protocol runtime rewrite | `complete` | The dedicated PBFT manager slice tracker was folded into `PLAN.md` after Slices 1-11 completed. Rust-mode PBFT manager production behavior no longer depends on `DbStorage`, `BridgeStorage`, generic storage-shim batches, C++ scalar mirrors, or C++ protocol sidecars except at classified network/tarcap, EVM/state execution, lifecycle wiring, and public API materialization boundaries. PBFT manager FinalChain-height facts now read the Rust `BridgeFinalChain` port directly; remaining `waitForFinalized`, bridge-root, and bridge-epoch calls are lifecycle or external-EVM executor boundaries. Future PBFT manager work is executor or compatibility cleanup unless it expands one of those explicit boundary migrations. |
| Route PBFT manager pillar-vote bundle validation through Rust | `partial` | The Rust-enabled sync path now lives in the full `PbftManager` overlay instead of an original-source early-return hook. The helper inspects each vote through Rust, uses Rust-returned period/block/vote hash/voter/signature status to build bundle facts, looks up DPoS weight in C++ with the Rust-recovered voter, calls the Rust bundle planner, returns explicit shim/planner statuses plus the first bad vote hash, and inserts only Rust-accepted votes with Rust-planned weights and Rust-recovered voter identities through a temporary PillarChainManager hook. This removes `validatePillarVote` / `addVerifiedPillarVote` revalidation, C++ signature recovery, C++ voter recovery, and DPoS relookup from insertion while keeping live sidecar insertion visible as temporary manager debt. Direct CXX bridge coverage for `pillar_vote_inspect` and `plan_pillar_vote_bundle` lives in `rust_consensus_tests`. Broader PBFT manager proposal, transition, and finalization paths are now partially Rust-planned as tracked in the dedicated PBFT manager rows; network gossip execution and many live compatibility side effects remain C++ executor work. |
| Route PBFT sync-period admission through Rust | `partial` | `rustaxa-consensus::pbft_sync` now owns the deterministic `processPeriodData` admission decision table for already-known and stale drops, previous-hash mismatch, missing/invalid FinalChain state, invalid reward/cert votes, invalid pillar data, and invalid required pillar votes. The CXX bridge uses stable u8 status codes plus transaction-hash wrappers, and the full `PbftManager` overlay passes facts at each existing sync checkpoint before applying Rust-returned side-effect intents. C++ still owns fact sourcing for live sidecars, `waitForFinalized`, queue clearing, peer reporting, live `PeriodData` ownership, final accept dispatch, and temporary log emission from Rust statuses. Logging is not a blocker for moving more of this path into Rust. Missing transaction and finalized-transaction facts are carried as warning signals and intentionally preserve legacy warn-only behavior. The queue-backed facts now include compact FinalChain hash, PBFT extra-data presence, reward-vote hashes, pillar-vote RLP bytes, PBFT cert-vote RLP bytes, transaction hashes, transaction identity facts, canonical period-data transaction RLP bytes, previous-cert sidecar flags, and pillar-presence metadata, reducing `PeriodData`/`PbftBlock` reads that existed only to build admission facts; invalid-state-root sync rejection logging, cert-vote block-identity checks, reward-vote hash selection, pillar-vote bundle validation, accepted cert-vote rematerialization, and accepted transaction rematerialization now use the same queue-backed PBFT block facts. The overlay also guards empty cert-vote bundles before random vote selection so the Rust-planned rejection path cannot hit the old modulo-by-zero failure. The same overlay now routes PBFT manager transition persistence through a Rust-owned storage apply: Rust commits round/step writes, next-voted status resets, cert-voted cleanup, and latest own-vote cleanup as one batch before C++ updates live mirrors; post-FinalChain executed-status reset intentionally remains a separate ordered effect. |
| Route PBFT manager startup restore and scalar runtime through Rust | `partial` | The full `PbftManager` overlay now owns a `BridgePbftManagerRuntime` handle for scalar manager state. Startup builds compact facts, Rust reads persisted round/step/lambda/status fields through `rustaxa-storage`, applies legacy-compatible scratch/default and finish-step normalization rules, persists any normalized startup step through Rust storage before the shim mirrors fields back into C++, rejects missing Cacti dynamic-lambda facts explicitly, and returns a typed runtime snapshot for temporary C++ mirror hydration. Transition persistence now commits through Rust storage and advances the Rust runtime cursor only after the storage batch succeeds. C++ still owns proposed-block/cert-voted live materialization, timers, daemon scheduling, VoteManager period/round side effects, FinalChain waits, network effects, and compatibility objects. Broad `pbft_manager_test` Rust-mode integration remains gated by existing FinalChain DPoS snapshot gaps, while direct Rust domain, bridge, storage, and consensus bridge tests cover this slice. |
| Route PBFT finalization persistence apply through Rust | `partial` | `rustaxa-consensus::pbft_finalize` now owns the storage-batch lifecycle for PBFT finalized-period persistence stages. Its native `apply_pbft_finalization_storage_writes` API creates a Rust storage batch over `rustaxa-storage`, appends ordered stage facts, commits on success, and drops the batch on rejected or failed stages without bridge-owned batch ids. The bridge `apply_pbft_finalization_storage_writes` entry point is now a CXX DTO adapter that converts stage payloads and result statuses while preserving existing staged appender compatibility APIs. The PBFT manager overlay calls this owned apply path for the primary finalized-period/reward-reset/sortition group, dynamic-lambda persistence, and post-FinalChain executed-status persistence. Rust also owns a finalization runtime session plus Cacti dynamic-lambda calculation: the bridge returns ordered mixed-executor actions, owns the action cursor, validates C++ action result reports before advancing, and returns block-period lambda, reward `blocks_per_year`, post-adjust rounds count, post-adjust dynamic lambda, and increase/decrease telemetry flags. The overlay consumes those Rust lambda outputs instead of calling the C++ `adjustDynamicLambda` routine in the Rust-mode finalization path; the shim-local duplicate helper has been removed, and the C++ lambda mirrors update only after Rust storage accepts the dynamic-lambda stage. Rust now also classifies already-persisted PBFT blocks through a storage-backed resume inspector in `rustaxa-consensus::pbft_finalize`: durable hash-to-period, period-data, finalized DAG/transaction positions, optional period-lambda, executed-status, FinalChain height, and pillar-post-processing requirement facts are read from `rustaxa-storage` or the accepted write intent to produce explicit complete/replay-needed/missing-primary/conflicting-primary/no-replay statuses, while the bridge only adapts the existing CXX DTO surface. The PBFT overlay consumes replay-needed tail classifications through a Rust-owned resume runtime session: when primary finalization and dynamic lambda are already durable and FinalChain is exactly one period behind, it replays FinalChain finalization, persists executed status through Rust, sets the live executed flag, advances the PBFT period, optionally processes the pillar block, and reports each action before Rust advances the cursor. Dynamic-lambda gaps, missing/conflicting primary facts, pillar-post-processing gaps without a durable proof, and complete duplicates remain explicit no-replay paths. DAG finalized-order mutation, transaction finalized-status sidecar cleanup, PBFT-chain live head updates, and pillar post-processing now run only as Rust-planned runtime actions; Rust validates finalized DAG counts, finalized transaction counts, finalized period, PBFT block hash, anchor hash, PBFT-chain size, PBFT-chain head/anchor state, pillar preflight identity, and non-empty accepted pillar-vote counts against the accepted plan before relevant cursors advance. VoteManager exposes a shim-owned reward-reset stage builder and only mutates live reward metadata after the Rust-owned batch commits. C++ still owns FinalChain dispatch mechanics, live field assignment from Rust plans, timers, period advancement mechanics, sortition live replay, pillar manager execution, object materialization, any restart replay of live side effects outside the bounded tail, bridge compatibility appenders, and temporary log emission. Logging is explicitly not a blocker for moving more finalization logic into Rust. |
| Route PBFT proposed-block restore and cleanup through Rust storage | `partial` | `ProposedBlocks` now exposes shim-owned storage-backed restore and cleanup operations. Rust iterates persisted proposed PBFT block RLPs, decodes each signed PBFT block link through `rustaxa-types`, validates the storage key against the decoded canonical block hash, and hydrates the Rust proposed-block index without materializing C++ `PbftBlock` objects during Rust-mode PBFT startup. Stale proposed-block cleanup now plans from the Rust index, deletes proposed-block storage keys in one Rust storage batch, and only then mutates the in-memory Rust index. The full `PbftManager` overlay calls the Rust-backed restore during startup while the original upstream-owned manager source remains clean. Proposed-block mark-valid mutations now have a compact period/hash shim API, and PBFT manager Rust-planned admission/leader-selection commands use Rust-returned identities instead of C++ `PbftBlock` objects as the mutation authority. Live PBFT block object materialization, proposal handling, finalization, gossip, public proposed-block APIs, and vote sidecars remain C++ owned. |
| Route PillarChainManager relevance through Rust | `rust-backed` | The Rust-mode path now lives in the full `PillarChainManager` overlay instead of an original-source guarded hook. `isRelevantPillarVote` calls a shim-owned helper and stateless Rust relevance planner. Rust owns the deterministic period/block/known-vote decision and stable rejection statuses; C++ still owns current-block lookup, live vote sidecars, network, storage, validation, signing, DPoS queries, finalization, and temporary log emission. Logging is not an ownership blocker. |
| Route PillarChainManager vote validation and block planning through Rust | `partial` | Pillar state is now a private sibling of PBFT manager/chain/vote/slashing state in `BridgePbftService`, with distinct readiness and exact unavailable behavior for partial services. Rust owns PBFT candidate/current-anchor decisions, strict-majority threshold arithmetic, admission/bundle contexts, validator vote-count history, block creation/linkage, and PBFT-facing prepare/acknowledge publication. PBFT production calls the four pure current-anchor decisions directly through its shared service; public manager methods remain compatibility wrappers. C++ supplies external FinalChain DPoS/bridge facts and retains signing, tarcap transport, logging, event dispatch, and temporary `PillarBlock`/`PillarVote`/`PeriodData` materialization. |
| Route rewards stats planning through Rust | `partial` | `rustaxa-consensus::rewards_stats` owns deterministic `BlockStats` calculation, interval caching, distribution boundaries, Magnolia fee gating, and Aspen DAG counting. Rust `FinalChain` owns the production runtime for native and external-EVM finalization; external plans are session/head/generation bound, storage mutation is included in the Rust publication identity, matching already-applied publications audit durable rewards rows, and successful publication/recovery reloads runtime state. CXX exposes only distribution-stat RLP to the external `StateAPI` adapter; the former FinalChain rewards-update carrier and C++ commit/clear acknowledgements are deleted. The standalone shim-owned `rewards::Stats` plus `BridgeRewardsStatsRuntime` remain compatibility-test-only. Remaining gap: remove temporary C++ `BlockStats` decoding at the external EVM boundary. |
| Restore `pbft_manager.cpp` upstream shape after complete shim | `complete` | The standalone Rust-mode `pbft_manager_shim` owns startup restore, sync validation, runtime routing, and the public PBFT compatibility API without importing or compiling `PbftManagerOld`. Feature-on builds exclude the original source; module-disabled and pure-C++ builds retain it unchanged, and the original header/source are clean versus `upstream-main`. The remaining debt is overlay drift: upstream manager changes must be ported into `pbft_manager_overlay.cpp` until Rust owns the manager runtime. |
| Define consensus storage ports | `partial` | Started historically in the retired `BridgeDagManagerRuntime`; the same storage-backed DAG sync and finalization cleanup payload assembly now lives in `BridgeDagTransactionService`'s private DAG state. Rust selects non-finalized hashes, loads selected DAG block RLPs through Rust storage, decodes transaction references, performs shared Rust transaction RLP lookup, resolves finalized transaction status, returns finalized-block counter facts, and applies finalized counter updates plus expired DAG block and expired non-finalized transaction deletes through one Rust storage batch. Broader typed storage traits for consensus services are still needed before PBFT/DAG finalization side effects move fully into Rust. |
| Decide CXX bridge shape for consensus hashes and vectors | `rust-backed` for DAG graph | DAG bridge uses fixed bytes and explicit boundary conversion; revisit if PBFT/vote bridges need richer payloads. |
| Add C++/Rust DAG parity fixture | `rust-backed` | Rust bridge fixture tests and C++ public API regression tests landed. Direct in-process legacy-vs-Rust comparison remains optional if duplicate dependency symbols are resolved. |
| Vote packet duplicate-with-block delivery gap | `deferred` | Reproduced in `PbftManagerTest.propose_block_and_vote_broadcast`: some peers can miss proposed-block insertion when vote paths short-circuit in network packet handlers. Do not patch upstream-owned network C++ in this rewrite stream; track as network-module follow-up and resolve via rewrite-side network shim when network work starts. |
| Prepare consensus ingress for arena data pipeline | `partial` | Future network ingress will call `BridgeNetwork::ingest_network_packet(packet_type, from_node, data)` for latest-tarcap payload bytes, store accepted bytes in an application-owned arena, and communicate by ingress/enrichment payload reference ids after that. The returned `bool` is ingestion success only, not payload validity or consensus acceptance. Consensus slices should add raw-byte inspection and compact-fact planning surfaces for PBFT votes, DAG blocks, transactions, pillar votes, and PBFT sync data as those modules are touched. The first PBFT vote ingress planner now provides this shape for compact already-decoded vote facts and returns admit/drop/request-sync status without owning network packet bytes. The arena data plane is expected to expose multiple logical pipelines, not one monolithic consensus path: peer status/sync control, transaction admission, DAG admission, DAG sync, PBFT vote progress, PBFT sync/finalized-period intake, and pillar vote handling. The goal is to let prefilter and consensus stages drop or route events before expensive materialization, then emit typed pipeline/egress effects for the network module. Temporary guarded hooks in latest-tarcap vote packet handlers route Rust-enabled builds through this planner; those hooks should move into a network/tarcap pipeline overlay once that module exists. |
