# Consensus Rewrite Tracker

This tracker expands the consensus section in `PLAN.md`. Keep it current as consensus code moves from C++ into Rust.
Consensus validation policy is defined in `doc/rewrite_validation_strategy.md`; unresolved work is tracked only in the
dependency-ordered **Remaining Consensus Work Queue**, while subsystem-specific minimum coverage stays in the
**Validation Matrix** below.

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
| PBFT vote ingress/progress planners | `rust/crates/rustaxa-consensus/src/pbft_vote_ingress.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_admission.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_event.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_progress.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_pipeline.rs`, `rust/crates/rustaxa-consensus/src/pbft_vote_runtime.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_progress.rs`, `rust/crates/rustaxa-bridge/src/verified_votes.rs`, `rust/crates/rustaxa-bridge/src/network.rs` | `partial` | Side-effect-free protocol planners for PBFT vote ingress and vote pipeline decisions plus a Rust-owned admission runtime for Rust-mode `VoteManager::addVerifiedVote`. Rust now owns the deterministic single-vote and bundle ingress gates for relevance, period/round/step windows, proposed-vote bundle rejection, bundle identity consistency, and PBFT/next-vote sync hints. Production tarcap enters only through the composed network ingress methods, which invoke those planners and own typed effect queueing; the direct side-effect-free CXX planning API and standalone bridge module are deleted. The production shim opens admission from canonical vote RLP plus explicit validation facts, so Rust carries canonical validation output, explicit replay mutation facts, and the validation-calculated weight into compact progress facts, mutates the single Rust-owned `VerifiedVotes` runtime, retains weighted storage payloads and unweighted slashing evidence payloads, owns PBFT `2t+1` threshold cache lookup/update, and commits any required progress rows before publishing, and returns one terminal executor report with typed peer-known, proposed-block sidecar, gossip, slashing, threshold, and PBFT-progress intents. Retained weighted payload views now back legacy snapshot, reward-vote materialization, and 2t+1 materialization APIs, and missing retained payloads for Rust-owned selected votes are invariant errors instead of partial results. The Rust runtime also builds PBFT reward-vote candidate facts from its own verified-vote metadata and resolves selected reward records in PBFT-block requested order. The Rust-mode `VoteManager` shim now exposes the peer-known, gossip, slashing, and PBFT-progress portions as a shim-owned admission report; latest-tarcap single-vote and bundle handlers execute those effects only after Rust admission accepts the vote, and Rust-mode bundle rebroadcast includes only accepted votes. The older admission session, pipeline session, standalone progress planners, weight-supplied constructor, and low-level verified-vote mutators remain compatibility/test scaffolding rather than production state-transition authority. Required extra-reward and current-round 2t+1 rows commit under the vote lock before the live mutation is published; persistence failure restores the bounded replay/round/payload checkpoint and exposes no executor effects. C++ still decodes network packets, supplies peer/live-sidecar facts, executes peer/network effects and proposed-block sidecar handling, and temporarily hosts guarded packet-handler ingress hooks until the network/tarcap pipeline overlay owns those routes. The latest-tarcap method signature changes are temporary hook debt, not the target pipeline API. |
| PBFT vote validation planner | `rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs`, `rust/crates/rustaxa-consensus/src/pbft_thresholds.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_validation.rs`, `rust/crates/rustaxa-bridge/src/verified_votes.rs` | `partial` | Rust owns received-vote validation decisions, stable rejection statuses, replay-marker timing, canonical PBFT vote RLP inspection, signed/unsigned vote hash derivation, recovered voter identity, signature and VRF proof checks, Rust-computed received-vote weight, the PBFT sortition-threshold formula, runtime-owned replay protection, runtime-owned `2t+1` threshold lookup/current-period caching, and local proposer-sortition proof/weight composition. The Rust-mode `VoteManager` shim routes `validateVote`, `addVerifiedVote`, `voteAlreadyValidated`, `getPbftTwoTPlusOne`, and `genAndValidateVrfSortition` through service-owned verified-vote methods instead of `VoteManagerOld`; composed validation borrows Rust FinalChain to resolve voter/total DPoS and an address-cached exact/prior/next VRF key, then returns authoritative weighted RLP for checked temporary sidecar hydration. `addVerifiedVote` still collects typed Rust `BridgeFinalChain` DPoS/key facts and lets Rust validation-backed admission produce the authoritative weight, verified-vote mutation, replay mutation, threshold decisions, and post-mutation executor effects. The older standalone `BridgePbftVoteValidationRuntime` and `BridgeVerifiedVotes` CXX handles have been deleted; production and bridge coverage route validation replay/threshold state through `BridgePbftService`. C++ retains the local-sortition bool/log adapter and temporary live vote-sidecar mutation; proposer proof, weight, and validation DPoS/VRF facts no longer cross CXX. |
| PBFT vote generation planner | `rust/crates/rustaxa-consensus/src/pbft_vote_generation.rs`, `rust/crates/rustaxa-bridge/src/pbft_vote_generation.rs` | `partial` | Rust owns side-effect-free local PBFT vote byte generation: it validates vote type/step compatibility, derives the expected voter/VRF public key from ephemeral wallet secrets, creates the legacy PBFT VRF proof/output, signs `PbftVote::sha3(false)`, returns canonical signed or weighted `PbftVote` RLP, and reports zero-stake, zero-total-DPoS, and zero-weight outcomes as stable statuses. Weighted generation is now a PBFT-service operation that borrows Rust FinalChain, reads voter stake before total stake, and keeps DPoS counts/readiness and weight facts private. The Rust-mode `VoteManager` shim supplies signing and committee configuration only, then materializes `generateVote` and `generateVoteWithWeight` sidecars directly from Rust-generated RLP, hydrates the temporary C++ VRF output cache through local VRF verification, and checks Rust hashes, recovered identity, VRF proof, weight, and exact RLP bytes before returning the sidecar. Locally generated own-vote persistence therefore stores Rust-generated weighted vote bytes through Rust storage. C++ still owns the temporary `PbftVote` sidecar type and PBFT manager/network orchestration. Logging around these calls is temporary observability, not an ownership blocker. |
| PBFT vote payload builders | `rust/crates/rustaxa-consensus/src/pbft_vote_payload.rs` | `rust-owned` | Rust owns legacy-compatible PBFT vote payload construction for post-admission side effects. It derives weighted storage RLP records from canonical signed vote bytes plus the authoritative calculated weight, builds raw weighted vote-bundle RLP for latest-round 2t+1 persistence and finalized reward-vote reset stages, builds optimized PBFT vote-bundle RLP for get-next network egress from retained weighted records, and normalizes unweighted signed vote RLP for slashing evidence so storage weights do not leak into slashing calldata. The admission runtime retains these payloads for accepted votes. Own-vote persistence encodes canonical bytes plus weight inside the native service, and direct vote-progress persistence resolves extra-reward and exact 2t+1 mapping identities from retained runtime payloads before building the storage bundle under the runtime lock. The standalone bridge payload module and its two free CXX codec exports are deleted; C++ no longer supplies weighted records or bundle RLP for these writes. The bridge still exposes operation-shaped plan/build results where tarcap peer filtering or retained external effects require them. |
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
- Slice 7 — CXX carrier minimization is complete at the accepted boundary: the transaction finalized-check identity
  input moved from generated CXX into a shim-owned C++ carrier, and four transaction/DAG staging DTOs moved from
  `ffi.rs` into private Rust module types. `CRW-02` then replaced `BridgePbftManagerRuntime` and `BridgePbftChain` with
  one application-owned
  `BridgePbftService`, deleted the finalization chain report carrier/API, and moved the obsolete
  `PbftManagerStartupFact` out of CXX into a Rust-private test fixture. The remaining production exports have callers,
  while the one test-only FinalChain seed helper is an enforced storage-conformance boundary. Future reduction follows
  a demonstrated last-caller migration or explicitly re-scoped network/EVM work rather than the completed no-caller
  export sweep. The first `CRW-03` sub-slice then deleted `BridgeProposedBlocks`, its factory and explicit restore
  exports, the storage-shim-owned live handle, and the C++ facade mutex after all production callers moved to the PBFT
  service or stateless compatibility functions.

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
| `CRW-07` | `complete` | Continue CXX carrier/export, module-flag, shim, and compatibility-test minimization after every consumer migration. | Runs alongside every consolidation item | The bridge exposes only `BridgeConsensusQueryApi`, `BridgeConsensusNetworkApi`, `BridgeConsensusExecutionApi`, application/bootstrap handles, and demonstrably necessary public compatibility handles. The inventory guard has no undocumented or stale entries, and tests protect behavior rather than retired scaffolding. |
| `CRW-08` | `complete` | Close remaining FinalChain/DPoS behavior parity: required contract methods outside the previously supported mutation subset and full failed-contract receipt parity for older supported paths. | Completed bounded method/receipt families and canonical legacy evidence | All 25 current-ABI DPoS methods, both slashing reads, supported slashing execution, and all 16 mutation selectors execute through Rust account/DPoS state with byte-compatible outputs, receipts, logs, blooms, persistence, restart behavior, and targeted legacy-vs-Rust parity coverage. Historical databases without complete Rust snapshots remain an explicit replay/rebuild deployment boundary rather than a current-ABI execution fallback. |
| `CRW-09` | `complete` | Introduce missing P0 FinalChain domain types/codecs and reduce temporary C++ `StateAPI` fact collection while preserving external EVM/state execution as an explicit adapter. | `CRW-09A` through `CRW-09I` | All ready P0 FinalChain domain/codec families are complete; every retained raw scalar/byte is a demonstrated codec, FFI, or external-executor representation; C++ `StateAPI` supplies only classified execution/committed-state operations; and the tracker, audit, and plan agree. |
| `CRW-09A` | `complete` | Establish the FinalChain scalar/codec foundation: nonce, transaction position, bloom, gas price, transaction value, account balance, and complete gas lifecycle. | None | Rust FinalChain uses the typed domains end to end while CXX carriers, persisted bytes, request identities, headers, receipts, and error ordering remain compatible. |
| `CRW-09B` | `complete` | Establish the shared DPoS monetary prerequisite and persisted-encoding evidence. | `CRW-09A` | `DposTokenAmount(U256)` owns policy amounts; characterization proves valid mixed encodings, schema 5/6 self-delegation synthesis, arbitrary-width reward indexes, and explicitly tracked malformed-decoder debt. |
| `CRW-09C` | `complete` | Type the complete FinalChain block-number lifecycle and PBFT-period conversion boundary. | `CRW-09A` | `FinalChainBlockNumber` owns head progression, hardfork heights, headers, historical reads, snapshots, native/external sessions, publication/recovery/audit, bloom endpoints, rewards heads, request identities, and bridge conversion. PBFT converts once at FinalChain admission; storage/RLP/CXX representations are unchanged; focused Rust coverage, Tier 1, FinalChain Tier 2, Tier 3 pure-C++ parity, and independent mapping review passed. |
| `CRW-09D` | `complete` | Migrate the complete DPoS principal/custody ledger using shared `DposTokenAmount` plus private `StoredDposTokenAmount` provenance. | `CRW-09B`; `CRW-09C` | Aggregate stake, delegation principal, V1/V2 custody and transitions, ABI reads, snapshots, malformed-input rejection, restart, and aggregate/ledger invariants are typed and parity-covered. |
| `CRW-09E` | `complete` | Type redelegation correction amounts and their ordered hardfork application. | `CRW-09C`; `CRW-09D` | Corrections preserve configured order and activation semantics, validate the principal anchor and repair aggregate/reward-head state atomically, and retain same-validator corruption policy and restart parity. |
| `CRW-09F` | `complete` | Type arbitrary-width reward-per-stake accumulators and delegation cursors. | `CRW-09D` | A distinct BigUint-backed reward-index domain owns validator accumulators and delegation cursors while regression policy, rounding, claims, persistence, and restart remain compatible. |
| `CRW-09G` | `complete` | Migrate reward pools and claim settlement using shared token amounts. | `CRW-09D`; `CRW-09F` | Commission/delegator pools, fee rewards, claims, account credits, cursor advancement, rollback, receipts, and restart use `DposTokenAmount` without changing reward-index semantics. |
| `CRW-09H` | `complete` | Type supply, minted rewards, and Aspen migration state. | `CRW-09C`; `CRW-09G` | Pre-/post-Aspen supply state is explicit; migration runs once; inconsistent persisted combinations fail closed; checked cap/reward arithmetic and accepted old schemas retain activation, publication, and restart parity. |
| `CRW-09I` | `complete` | Finish non-EVM FinalChain adapter contraction and reconcile retained CXX carriers. | `CRW-09C` through `CRW-09H` | C++ supplies only accepted external executor/state-lifecycle operations; obsolete non-EVM fact DTOs/conversions are removed; retained carriers are classified; and `CRW-07`, the audit, and `PLAN.md` agree. |
| `CRW-10` | `complete` | Perform final consensus consolidation closeout: delete newly obsolete code/docs, reconcile the audit, run required Rust/C++ validation, and synchronize applicable upstream-owned C++ intersections to `cpp-reference`. | `CRW-02` through `CRW-08`; `CRW-09I`, excluding work explicitly scope-gated below | No actionable unclassified consensus ownership or compatibility-deletion item remains; retained C++ surfaces match the declared network, EVM, lifecycle, signing/VDF, and public-materialization boundaries, and the tracker/audit/plan agree. |
| `CRW-11` | `complete` | Establish the aggressive bridge/shim deletion contract: name supported Rust-mode C++ clients, collapse compatibility policy, record measured surface budgets, and make the inventory distinguish production from test-only callers. | None | Task-owner decisions are recorded; every retained facade has a named client and deletion condition; checked metrics cover lines, exports, carriers, handles, shims, flags, partial factories, and callers; test-only exports fail the guard unless explicitly allowlisted. |
| `CRW-12` | `active` | Move PBFT and DAG/transaction application-service ownership, construction, restoration, lock domains, and behavioral tests out of `rustaxa-bridge`; leave thin CXX wrappers. | `CRW-11` | Native Rust application/runtime owners compile and test without CXX support; `rustaxa-bridge` owns no protocol runtime state or behavioral test suite. |
| `CRW-13` | `complete` | Collapse granular Rust production feature topology and delete partial-service factories and compatibility-only constructors. | `CRW-11`; coordinate with `CRW-12` | One supported Rust production composition path remains; partial capability services and redundant module flags are deleted; all-Rust-disabled builds still select untouched C++. |
| `CRW-14` | `blocked` | Retire state-free or non-production facades, starting with rewards stats, proposed blocks, verified votes, sortition params, gas pricer, then PBFT chain after client migration. | `CRW-11`; `CRW-12` where the facade calls an application service | Rewards stats, proposed blocks, verified votes, sortition params, and gas pricer are retired. PBFT chain remains blocked on its named network/RPC client migration; each selected family deletes its shim, bridge declarations, carriers, constructors, flag, and compatibility-only tests together. |
| `CRW-15` | `blocked` | Replace internal legacy consensus object materialization with canonical bytes, opaque identities, borrowed native views, or client-specific DTOs. | `CRW-11`; native owners from `CRW-12` | Internal C++ code cannot obtain mutable Rust-owned consensus object graphs; associated sidecars, mirrors, compatibility mutexes, and revalidation protocols are deleted. |
| `CRW-16` | `blocked` | Contract PBFT/vote/pillar and DAG/transaction/proposer manager shims into explicit leaf adapters for transport, EVM, signing, VDF, timers, and public clients. | `CRW-12` through `CRW-15` | Manager-shaped internal APIs and manager-to-manager calls are gone; remaining leaf adapters have named external clients and typed effects. |
| `CRW-17` | `blocked` | Reduce storage and FinalChain bridge surfaces to native bootstrap ownership, public query/admin clients, storage conformance, and a narrow external-EVM executor contract. | `CRW-12` through `CRW-16`; coordinate EVM expansion with `CRW-E01` | Broad storage/query-family handles and consensus use of `BridgeFinalChain` are deleted; retained query, admin, conformance, and EVM surfaces are client-specific and minimal. |
| `CRW-18` | `blocked` | Final bridge/shim reduction closeout and documentation deletion. | `CRW-11` through `CRW-17`; `CRW-N01`/`CRW-E01` only where explicitly activated | Required validation passes; budgets demonstrate the intended contraction; no unclassified or unnamed compatibility surface remains; this reduction plan is deleted and only stable strategy, live inventory, and remaining tracker items survive. |

`CRW-11` is complete. The task-owner contract now retains only named tarcap transport, concrete EVM/StateAPI,
operation-specific signing, VDF execution, public-read, and pure-C++ reference clients. The checked starting budgets are
51,651 bridge lines, 19,545 shim lines, 428 CXX functions, 366 carriers, 21 opaque handles, 16 shim directories, 9
granular flags, 5 partial-service factories/call sites, and 44 non-test C++ bridge consumers. The guard parses the CXX
module, classifies bridge-shaped C++ calls with opaque receiver context, rejects callerless or unallowlisted test-only
exports, inventories every owned-handle factory, checks each partial-factory path and occurrence, and ratchets budgets
against every historical audit from the target merge base. Its multi-commit, duplicate-call, inactive-code,
comment/string, uncalled-export, stale-row, and malformed-inventory self-tests pass. This contract-only slice migrated
zero production callers and removed zero bridge lines, shim lines, CXX functions, carriers, handles, shims, flags,
partial factories, compatibility constructors, or tests. `make rewrite-validate-fast`, the focused live/self guards,
`git diff --check`, architecture review, and independent correctness review pass; all touched paths are rewrite-only
and have no upstream-owned C++ intersection.

The first bounded `CRW-12` PBFT sibling ownership moves are implemented for proposed blocks and the PBFT chain. Native
`rustaxa-consensus::proposed_blocks::ProposedBlocksService` now owns the shared storage handle, restoration, sibling
`RwLock`, storage-before-memory publication, validation-cache mutation, reads, snapshots, and stale-period cleanup.
`BridgePbftService` embeds that CXX-free owner, and `rustaxa-bridge/src/proposed_blocks.rs` retains only FFI DTO
conversion plus the separately classified stateless storage adapters. Cross-domain leader
selection and combined vote/proposal cleanup still borrow native proposal guards from bridge orchestration and remain
explicit PBFT-owner migration debt. Five duplicated protocol
behavior tests moved out of the bridge suite into native owner coverage; the remaining bridge test covers the focused
storage compatibility projection. Native `rustaxa-consensus::pbft_chain::PbftChainService` likewise owns its storage lifetime,
restoration/default initialization, sibling lock, head projection/update, next-block validation, and finalized-block
lookup. The production bridge constructor publishes that native owner; the old bridge chain-state struct,
bridge-owned `Arc<RwLock<_>>`, and chain-only constructor are deleted. Cross-domain finalization and leader-selection code temporarily borrow
native chain guards, while C++ bridge/shim tests cover constructor/DTO/lifetime behavior and native tests own
the runtime contracts. Native `rustaxa-consensus::pbft_vote_runtime::PbftVerifiedVotesService` now owns the
verified-vote storage lifetime, atomic restoration, shared admission runtime, and mutex as the third PBFT sibling
extraction. Production PBFT construction publishes that native owner; `BridgePbftService` no longer owns the runtime
or vote lock, and storage-free verified-vote construction plus its bridge-only tests are deleted. Cross-domain
FinalChain validation, leader selection, combined cleanup, finalization, and effect/DTO composition still borrow its
temporary native guard. Native `rustaxa-consensus::slashing::SlashingProofService` is the fourth PBFT sibling
extraction: it owns immutable planner configuration, the bounded submitted-proof cache, and the shared mutex, exposes
task-oriented plan/report methods, and keeps every guard inside native Rust. `BridgePbftService` now holds only the
native capability, while the bridge converts CXX evidence and executor reports. Two duplicated bridge cache/lifetime
tests plus redundant bridge configuration coverage moved to native service tests. This is partial progress rather than
`CRW-12` completion. Native `rustaxa-consensus::pbft_readiness::PbftServiceReadiness` additionally owns independent
monotonic PBFT and pillar-bootstrap readiness atomics, acquire/release publication, and clone-visible lifecycle tests;
the bridge root no longer owns either control state or its `accepts_live_commands` facade method. The PBFT manager/pillar owner
and the DAG/transaction/sortition owner, their construction, their remaining lock domains, and their behavioral suites
still need to leave `rustaxa-bridge`.

The first `CRW-14` facade-contraction slice deletes the non-production Rust-mode rewards-stats family as one unit:
`rewards_stats_shim`, `BridgeRewardsStatsRuntime`, its six CXX operations, seven facade-only carriers, partial factory
and constructor, storage-batch relay, bridge module, Rust CXX integration test, and bridge-local behavioral tests.
Native `rustaxa-consensus` FinalChain already owns production rewards-stat planning, persistence, restart, and
distribution behavior. The slice also deletes the now-callerless native apply/append/clear report API and its
facade-mechanics-only tests; restart coverage seeds native storage directly. The untouched legacy `rewards::Stats`
implementation and `rewards_stats_test` remain available
only in the all-Rust-disabled pure-C++ reference configuration. The ratcheted inventory falls by 596 bridge lines, 322
shim lines, six CXX functions, seven carriers, one handle, one shim directory, one partial factory/call site, and one
non-test C++ bridge consumer. `make rewrite-validate-fast` passes 376 tests; focused native rewards tests pass 13/13;
Rust CXX tests pass 52/52; Rust-enabled FinalChain tests pass 56/56; the all-Rust-disabled legacy
`rewards_stats_test` passes 7/7; Rust-enabled `core_libs`, inventory self/live/base, storage-boundary, whitespace, and
independent review gates pass. The original rewards header/source/test have an empty diff against `upstream-main`.
The guarded `tests/CMakeLists.txt` build-selection hook is the documented upstream-owned exception: it retains that
test only when `RUSTAXA_ENABLE=OFF`; the existing Rust-mode core-libs CMake overlay hook merely stops
registering the deleted shim. Remaining `CRW-14` families are blocked on `CRW-12` because their C++ facades call the
bridge-owned PBFT or DAG/transaction application service.

The next `CRW-13` topology contraction deletes
`create_pillar_capable_pbft_service_for_compatibility`, its CXX export, and the Rust-mode
`PillarChainManager` compatibility constructor. Production already injects the full App-owned PBFT service; C++ pillar
tests now use that production constructor path, and Rust-only pillar behavior retains a private `#[cfg(test)]` fixture.
The checked topology falls to three partial factories and three compatibility-constructor calls without changing the
untouched pure-C++ manager constructor. The guarded `tests/pillar_chain_test.cpp` constructor rewiring is the sole
upstream-owned test exception; it preserves assertions and changes only Rust-enabled fixture construction.

The following `CRW-13` contraction deletes `create_dag_transaction_service_for_gas_pricer`, its CXX export, the
storage-free transaction runtime builder, and the shim-owned compatibility service/lock. Production already delegates
GasPricer reads and updates through the App-owned `TransactionManager`; native `GasPriceOracle` tests replace the
standalone Rust compatibility behavior, while `gas_pricer_test` remains pure-C++-reference-only. The checked topology
falls to two partial factories and two compatibility-constructor calls. The guarded `tests/CMakeLists.txt` source
selection is the sole upstream-owned exception for this slice.

The next `CRW-13` contraction deletes `create_dag_transaction_service_for_transaction_manager`, its CXX export, and the
transaction-only Rust service shape. Production already injects the fully restored DAG/transaction/sortition service;
the retained four-argument Rust-mode facade constructor now restores that same composition for standalone C++ callers.
Bridge tests keep full-service restoration and cursor-safety coverage while dropping unavailable-domain assertions for
the retired topology. The checked topology falls to one partial factory and one compatibility-constructor call.

The final `CRW-13` partial-service contraction deletes `create_pbft_chain_service_from_storage`, its CXX export, and the
Rust-mode `PbftChain(DbStorage)` constructor. Production and boundary tests now inject the same full PBFT service,
while native PBFT-chain tests continue to own restoration, projection, validation, and update behavior. Rust bridge
tests no longer preserve unavailable pillar, verified-vote, or slashing behavior for a topology the application does
not support. Guarded helpers in the upstream-owned `tests/dag_test.cpp` and `tests/full_node_test.cpp` are the documented
temporary test exceptions: Rust mode injects the full service, while pure-C++ mode retains the original storage
constructor. The checked topology reaches zero partial factories and zero compatibility-constructor calls.

The next bounded `CRW-12` owner contraction moves the PBFT manager serialization domain and its complete mutable
runtime/session container into native `rustaxa-consensus::pbft_manager::PbftManagerService`. `BridgePbftService` now
holds that native owner directly; the bridge-owned mutex, optional manager capability, runtime alias, guard, and
chain-only bootstrap rejection are deleted. Bridge tests that previously assembled manager-less PBFT roots now use the
full production composition. The bridge retains construction plus DTO/effect orchestration temporarily, but manager
state and poison handling no longer belong to `rustaxa-bridge`. The checked bridge budget falls to 50,111 lines.

The following `CRW-13` topology contraction makes every remaining PBFT sibling structurally required.
`BridgePbftService` now contains verified-vote, slashing, and pillar owners directly; the optional fields, unavailable
branches, and CXX `has_slashing`/`has_pillar` probes are deleted. C++ facades still reject null shared services, and
pillar consumers still require completed pillar bootstrap, so lifecycle failures remain explicit without preserving
an application topology that can no longer be constructed. The checked bridge budget falls to 50,058 lines and 416
CXX functions.

The final `CRW-13` topology contraction deletes all eight subsystem-specific
Rustaxa CMake options and compile definitions. `RUSTAXA_ENABLE=ON` now selects
the complete VDF, storage, FinalChain, PBFT, vote, pillar, slashing, DAG, and
transaction rewrite composition; `RUSTAXA_ENABLE=OFF` selects the untouched
pure-C++ reference sources. Core source selection, app and tarcap branches,
tests, and parity/conformance scripts use that same binary choice, so unsupported
mixed module topologies can no longer compile accidentally. The checked
granular-feature budget falls from eight to zero; partial factories and
compatibility constructor calls remain zero. No Rust/CXX API, carrier, handle,
shim, or production caller changes.

Validation used fresh master-ON and master-OFF CMake trees. The ON composition built `taraxad`, the Rust consensus and
CXX bridge suites, storage conformance, and the macro-sensitive PBFT, pillar, FinalChain, and StateAPI targets; fast,
inventory, storage-boundary, and smoke gates passed. The OFF composition built VDF, storage conformance, FinalChain,
StateAPI, PBFT, pillar, rewards, and gas-pricer targets; all 32 native DPoS FinalChain cases passed, and ON/OFF storage
conformance transcripts matched exactly. The original C++ intersections in this slice are guarded topology changes
only: they replace retired subsystem predicates with the master predicate and do not alter legacy behavior. Independent
review found no code defect; its stale-current-state documentation finding was corrected before closeout.

The next `CRW-12` owner extraction moves the complete pillar lifetime and synchronization topology into CXX-free
`rustaxa-consensus::pillar_chain_service::PillarChainService`. The native service owns storage and restoration,
`PillarVotes`, the canonical current/latest snapshot, single-vote and finalization preparation registries, finalization
token sequencing, the outer serialization mutex, and monotonic pillar readiness. `BridgePbftService` embeds that native
owner directly and retains only a documented temporary guard adapter for FFI-shaped task logic. FinalChain-composed
paths preserve prepare, guard release, external query, and generation-bound relock/apply ordering. Native tests cover
restoration, readiness publication, clone-visible state, token reuse/generation scoping, and durable generation
recovery. The checked bridge budget falls to 49,794 lines.

The following pillar contraction moves non-external storage and anchor tasks behind `PillarChainService` methods.
Native DTOs now carry startup bootstrap, anchor decisions, block creation/linkage plans, and validator vote-count
changes. The service owns readiness enforcement, generation sampling and revalidation, current-data and own-vote
persistence, threshold planning, canonical latest-finalized lookup, and all related state access; bridge methods only
convert CXX carriers. The FinalChain block-creation adapter still performs its external validator snapshot between two
short native calls and retains the generation check. Native tests cover pending/ready behavior, bootstrap and own-vote
storage, anchor/linkage decisions, block planning, and latest-finalized lookup. The checked bridge budget falls to
49,669 lines.

The final pillar-owner contraction moves single-vote preparation/admission/relevance, weighted bundle planning and
apply, verified payload and network-bundle lookup, and pillar finalization prepare/ack behavior into CXX-free native
Rust. `PillarChainBridgeGuard`, the raw `pillar_state` accessor, and every production bridge state borrow are deleted.
FinalChain adapters retain the required prepare, unlocked external query, and generation-bound native apply sequence.
Forty-two protocol/state tests and their fixtures move native; `pillar_votes.rs` retains seven boundary-only tests for
FFI inspection/conversion and FinalChain unwrapping/error mapping. Independent review confirmed readiness/status
precedence, the 4,096-entry preparation bound, storage fallback validation, chunking, and finalization token retry
semantics. The checked bridge budget falls to 46,193 lines.

The pillar closeout contraction removes the remaining public native escape hatch. `PillarChainState`,
`PillarChainGuard`, `PillarChainStateSnapshot`, the service lock, and the snapshot decoder become crate-private and
their re-exports are deleted. Bridge tests no longer inspect state pointers, generations, or token counters; they assert
readiness/bootstrap/task behavior, while current/latest snapshot relationship tests move to the native module.
Independent review confirms this is visibility and test-ownership contraction only, with no runtime or FFI behavior
change. The checked bridge budget falls to 46,074 lines.

The first bounded DAG/transaction owner contraction introduces
`rustaxa-consensus::sortition::SortitionService`. It restores the sortition manager before publication and owns its
mutex and stable poison policy. `BridgeDagTransactionService` now contains that native owner directly; its optional
sortition field, immutable capability mirror, bridge-defined guard, unavailable-domain branches, and CXX
`dag_transaction_service_has_sortition` probe are deleted. DAG-composed callers retain DAG-then-sortition lock order
through a temporary native guard that must not cross an external executor boundary. Native and bridge tests cover
restoration, parameter access, persistence, preview/commit behavior, and cursor-safe DAG composition. The checked
budgets fall to 45,991 bridge lines, 19,099 shim lines, and 415 CXX functions.

The enabled `CRW-14` follow-up retires the Rust-mode `SortitionParamsManager` facade as a complete family. The DAG
manager overlay drops its embedded facade and accessor; the overlay proposer and PBFT manager already use the native
DAG/transaction service. Three Rust-mode network fixtures use the equivalent genesis sortition parameters while their
PBFT finalization loop is stopped, and the original accessor remains unchanged in the pure-C++ branch. The shim
directory, six direct CXX operations, two facade carriers, facade-only bridge methods/tests, C++ shim test, and
Rust-mode `sortition_test` target are deleted. Native Rust retains restoration, replay, lookup, efficiency,
preview/commit, persistence, and restart coverage; the untouched legacy implementation and `sortition_test` remain
pure-C++ reference coverage. The storage overlay retains only the canonical `SortitionParamsChange` codec required by
the stable storage API. The checked budgets fall to 45,564 bridge lines, 18,848 shim lines, 409 CXX functions, 357
CXX carriers, 14 shim directories, and 42 non-test C++ bridge consumers.

The next `CRW-14` contraction retires the Rust-mode `ProposedBlocks` facade as a complete family. The PBFT manager
overlay drops its embedded class and routes durable publication, candidate lookup/validation, validation marking, and
network snapshot materialization through the shared native `ProposedBlocksService`. Direct publication preserves the
legacy storage-before-duplicate ordering, while network ingress retains its pre-write duplicate fast path. The shim
directory, three facade-only CXX operations, two carriers, and facade-only C++/bridge tests are deleted. Combined
period cleanup already uses the native PBFT service. The storage overlay and vote-manager candidate selection retain
three separately classified stateless/native helpers, not a proposed-block runtime. The untouched original class
remains pure-C++-reference-only. The checked budgets fall to 45,472 bridge lines, 18,572 shim lines, 406 CXX functions,
355 carriers, 13 shim directories, and 41 non-test C++ bridge consumers.

The next `CRW-14` contraction retires the Rust-mode `VerifiedVotes` facade as a complete family. `VoteManager` now
calls the shared native `PbftVerifiedVotesService` directly for admission, validation, snapshots, reward selection,
threshold state, cleanup, persistence, and network egress planning. Stable carrier-only vote view declarations move
under the VoteManager overlay; no C++ verified-vote runtime, lock, or facade remains in Rust mode. The shim directory,
eight facade-only CXX operations, five carriers, the facade-only C++ test, and its CMake wiring are deleted. Native
verified-vote tests retain behavioral coverage, while `vote_test` covers the stable carrier contract in both Rust and
pure-C++ modes. The untouched original class remains pure-C++-reference-only. The checked budgets fall to 45,383
bridge lines, 17,629 shim lines, 398 CXX functions, 350 carriers, 12 shim directories, and 40 non-test C++ bridge
consumers.

The next `CRW-14` contraction retires the Rust-mode `GasPricer` facade as a complete family. App finalization and
metrics, Eth RPC, GraphQL, and the slashing overlay now call the App-owned `TransactionManager` gas-price query/update
operations directly; GraphQL retains only its operation-shaped `QueryGasPriceReader` callback. The shim directory and
granular gas-pricer flag are deleted, and the slashing overlay no longer stores or validates a redundant facade.
All-Rust-disabled builds continue compiling the untouched legacy implementation and `gas_pricer_test`. Before the
`CRW-13` topology collapse, the now-retired mixed slashing configuration also retained that implementation solely for
the untouched legacy `SlashingManager` constructor. Focused transaction-manager shim tests cover block-history updates, empty updates,
percentile selection, and the pool-mode floor through the production API. The checked budgets fall to 17,516 shim
lines, 11 shim directories, eight granular flags, and 39 generated-bridge consumers; bridge lines, CXX functions,
carriers, handles, partial factories, and compatibility constructors are unchanged.

The next bounded `CRW-12` owner contraction introduces CXX-free
`rustaxa-consensus::pbft_service::PbftService` as the PBFT application root. It validates storage-independent slashing
configuration first, restores the chain before deriving manager period and Cacti facts from its head, restores every
storage-backed manager/proposed-block/verified-vote/pillar sibling from the same storage handle, and starts bootstrap
readiness pending for publication through the native lifecycle. The
exported `BridgePbftService` becomes a one-field CXX adapter with no sibling state, storage sidecar, mutex, or readiness
flag. Root derivation, shared-chain ownership, readiness, and failure-before-publication tests move native;
one bridge test retains factory/bootstrap-gate boundary wiring. CXX signatures and sibling lock domains are unchanged. The
checked bridge budget falls to 45,169 lines; functions, carriers, handles, shims, flags, partial factories,
compatibility constructors, and non-test C++ consumers are unchanged.

The next bounded `CRW-12` DAG/transaction contraction moves the complete transient transaction proposal-packing
protocol into CXX-free `rustaxa-consensus::transaction_packing_service::TransactionPackingService`. The native service
owns its private mutex and poison policy, the single compatibility-or-DAG owner binding, canonical candidate/RLP
snapshot, shard cursor, planner accounting, pending-estimate ordering, selected output, stop state, ordered typed
effects, actual-demotion acknowledgement, and selective abort. Compatibility and DAG proposer flows share that owner;
all locks are released for C++ EVM execution, and DAG finalization preserves DAG-then-transaction locking before native
owner/count/hash validation and selected-payload transfer. Count/hash mismatch retains the matching session, wrong-owner
abort is a no-op, and successful or explicit matching abort clears it. Native tests replace bridge-owned planner,
sharding, ownership, ordering, and poison-policy coverage; bridge tests retain only queue/cache effect application and
CXX boundary behavior. Queue, sidecar/cache, storage, and shared DAG/transaction batch publication remain bridge-owned
`CRW-12` debt. The checked bridge budget falls by 146 lines to 45,023; functions, carriers, handles, shims, flags,
partial factories, compatibility constructors, and non-test C++ consumers are unchanged.

The following bounded `CRW-12` owner contraction introduces CXX-free
`rustaxa-consensus::transaction_service::TransactionService` as the complete transaction runtime owner. It restores
the durable transaction count, validates and restores gas-price history, and publishes one mutex-protected state
containing the queue, sidecar/count/gas cache, gas oracle, proposal gas limit, mandatory shared storage, drop
observation, and native packing subowner. `BridgeDagTransactionService` embeds this service directly and wraps only
short-lived native guards for FFI-shaped methods, preserving DAG-then-sortition-then-transaction ordering and releasing
all guards for EVM callbacks. Native tests now cover count/default restoration, restart and light/full gas history,
configuration failure-before-publication, owner coherence, gas/queue behavior, and the stable poison identifier;
duplicated bridge gas/restoration behavior tests are deleted. Shared DAG/transaction batch composition and the
temporarily public state/guard adapter remain explicit `CRW-12` debt. The checked bridge budget falls by 166 lines to
44,857; functions, carriers, handles, shims, flags, partial factories, compatibility constructors, production callers,
and non-test C++ consumers are unchanged. Focused native, bridge, DAG, C++ transaction-shim, transaction, and consensus
tests pass, as do the fast rewrite and inventory gates. The aggregate consensus/Tier 3 gates remain non-green on the
existing multi-test process-lifetime failure: one node-owning test leaves its RocksDB directory locked and later cases
cascade; the first affected `PillarChainTest.votes_count_changes` case passes when run alone in a clean runtime.
The Python integration runner is additionally unavailable in the current container because its externally managed
Python installation has neither `virtualenv` nor `pytest`.

The next bounded `CRW-12` owner contraction introduces CXX-free
`rustaxa-consensus::dag_service::DagService` as the complete DAG runtime owner. It owns the graph and storage handle,
proposer/verifier/add-block cursors and id sequences, proposer retry state, restoration, initial proposal-period
mapping, serialization mutex, and the stable DAG poison policy. Stored proposer inputs and selected transactions are
native domain values rather than CXX carriers. `BridgeDagTransactionService` now embeds the mandatory native service;
the optional bridge state, bridge mutex, `DAG_SERVICE_UNAVAILABLE` branch, bridge-local session types, construction,
and restoration logic are deleted. Five native owner tests cover fresh and idempotent restoration, shared storage and
empty-session publication, persisted PBFT-anchor/non-finalized graph restoration, missing-anchor and malformed-row
failure-before-publication, and the stable poison identifier. The former bridge restoration behavior test is deleted;
bridge coverage remains for conversion, external executor revalidation, lock order, and successful shared
DAG/transaction publication. Injected batch-commit failure coverage remains follow-up validation debt. Shared
three-service construction, DAG/transaction batch composition, and temporary
FFI-shaped methods over the public native state/guard remain explicit `CRW-12` debt. The checked bridge budget falls by
298 lines to 44,559; shim lines, CXX functions, carriers, handles, shim directories, granular flags, partial factories,
compatibility constructors, production callers, and non-test C++ consumers are unchanged.

The next bounded `CRW-12` root-ownership contraction introduces CXX-free
`rustaxa-consensus::dag_transaction_service::DagTransactionService` as the sole DAG-cluster application root. It owns
the native transaction, DAG, and sortition siblings, restores them in the existing transaction-then-DAG-then-sortition
error order from one shared storage owner, publishes only after all three succeed, and owns access to the canonical
DAG-then-sortition-then-transaction lock domains plus composed DAG-then-transaction acquisition.
`BridgeDagTransactionService` now contains only that native root
and FFI-shaped task adapters; it no longer owns sibling state, restore sequencing, lock policy, the add-block cursor,
add-block planning, transaction inspection, the shared DAG/transaction batch, finalized-order storage application, or
post-finalization transaction-sidecar cleanup. Nine native tests cover shared ownership, restart and failure ordering,
atomic add-block success/restart, injected commit failure with cursor retention and no premature publication, cursor
overlap/abort behavior, terminal/save-false behavior, concurrent preparation, and finalized cleanup publication ordering. They replace five bridge-owned restoration,
add-block-planner, and finalization semantic tests; retained bridge add-block/finalization tests exercise carrier
conversion and the full CXX boundary. Temporary FFI-shaped proposer, verifier, packing, query, and compatibility methods
over hidden native guard accessors remain explicit `CRW-12` debt. The checked bridge budget falls by 994 lines to
43,565; shim lines, CXX functions, carriers, handles, shim directories, granular flags, partial factories,
compatibility constructors, production callers, and non-test C++ consumers are unchanged.

The following bounded `CRW-12` contraction moves the complete DAG-proposer
transaction-pack task behind native `DagTransactionService`. Native code now
owns DAG-stage validation, queue/cache snapshot and ordered effect publication,
owner-bound transaction-packing cursors, immediate terminal advancement,
estimate finalization, selected payload transfer, and exact DAG/transaction
cursor cleanup. EVM estimation remains an unlocked leaf boundary over
executor-ready native candidates; the three bridge functions contain only
carrier conversion and one native-root call. Four native application tests
cover the unlocked estimate interval and cache/payload publication, throttled,
empty, retry-state publication, mismatch, idempotent and lock-poison cleanup
paths, compatibility-owner isolation, and malformed queue payload rejection.
Four superseded bridge semantic tests are deleted while one full
carrier-conversion path remains. Temporary
FFI-shaped proposer phases, verifier, query, and compatibility methods over
hidden native guard accessors remain explicit `CRW-12` debt. The checked bridge
budget falls by 154 lines to 43,411; shim lines, CXX functions, carriers,
handles, shim directories, granular flags, partial factories, compatibility
constructors, production callers, and non-test C++ consumers are unchanged.

The next bounded `CRW-12` owner contraction moves the complete DAG
`verifyBlock` application task behind native `DagTransactionService`. Native
Rust now owns the verifier cursor and status transitions, ordered
queue/sidecar/storage transaction resolution, proposal-period finalized nonce
filtering, exact authorization snapshots and stale-cursor cleanup, historical
sortition lookup, VDF proof verification, and terminal tip-gas validation.
Every native guard is released before the retained FinalChain authorization
query or C++ transaction/EVM materialization boundary. The production bridge
functions now convert CXX carriers, perform the unlocked FinalChain leaf query,
and call one native-root method; they no longer construct private transaction
queries, coordinate DAG/transaction/sortition locks, validate cursor identity,
run VDF proof logic, or decide verifier transitions. Native tests cover ordered
transaction sources and finalized filtering, stale cursor preservation,
authorization replacement, VDF fingerprint revalidation, and wrong-stage gas
reports; bridge tests retain carrier and external-leaf wiring coverage.
Superseded bridge-local verifier test helpers and the remaining proposer/query
guard escape hatch remain explicit `CRW-12` cleanup debt. The checked bridge
budget falls by 198 lines to 43,213; all other checked surface budgets are
unchanged.

The following bounded `CRW-12` owner contraction moves the complete DAG
proposer-session application task behind native `DagTransactionService`.
Native Rust now owns frontier and proposal-period observation, cursor identity,
historical-sortition revalidation, eligibility/retry planning, VDF wait and
stale-proof transitions, timestamped unsigned block construction, signature
validation, signed block assembly, add-block report classification, and
terminal cleanup. The native application root captures transaction pressure
under DAG-then-transaction locking, releases every guard before FinalChain,
VDF, sleep, signing, and add-block execution, and revalidates exact cursor
snapshots before applying external facts. Production bridge functions now only
convert CXX carriers, execute the unlocked FinalChain read, and translate the
native step. General proposer abort also removes an exact owner-bound pending
transaction-pack cursor. The existing bridge behavioral suite now exercises
the native production route; relocating and deleting its superseded direct
runtime adapters remains explicit `CRW-12` cleanup debt. The checked bridge
budget falls by 59 lines to 43,154; all other checked surface budgets are
unchanged.

The next bounded `CRW-12` deletion removes the complete superseded direct DAG
verifier adapter family from `rustaxa-bridge/src/dag.rs`. Production already
routes verification through native `DagTransactionService`; the deleted bridge
surface duplicated cursor construction, transaction and authorization
advancement, VDF snapshot/application, conditional tip-gas loading, step
encoding, and terminal/error selection solely for bridge-local behavioral
tests. Five focused tests now exercise those contracts directly on native
`DagServiceState`, including ordered live reports and missing transactions,
stale VDF snapshot rejection with replacement preservation, conditional
retained-tip loading, missing-tip rejection, and wrong-stage short-circuiting.
The native verifier application methods are narrowed from
hidden public escape hatches to crate-private APIs. The production CXX ABI and
the retained FinalChain, VDF, transaction materialization, and EVM leaves are
unchanged. Direct proposer behavioral adapters remain the corresponding
`CRW-12` cleanup debt. The checked bridge budget falls by 812 lines to 42,342;
shim lines, CXX functions, carriers, handles, shim directories, granular flags,
partial factories, compatibility constructors, production callers, and
non-test C++ consumers are unchanged.

The following bounded `CRW-12` deletion removes the complete superseded direct
DAG proposer adapter and behavioral-test family from
`rustaxa-bridge/src/dag.rs`. Proposer cursor ownership, observation
fingerprinting, FinalChain-fact application, retry/VDF transitions, block
construction, signature validation, add-block classification, and cleanup are
now tested directly on native `DagServiceState`; bridge fixtures that still
exercise the production ABI enter through native `DagTransactionService`
instead of a parallel runtime facade. The native proposer entry and observation
methods are crate-private, while the production CXX service functions and the
retained FinalChain, VDF generation, signing, add-block, and transport leaves
are unchanged. The checked bridge budget falls by 2,068 lines to 40,274; shim
lines, CXX functions, carriers, handles, shim directories, granular flags,
partial factories, compatibility constructors, production callers, and
non-test C++ consumers are unchanged.

The next bounded `CRW-12` deletion moves the complete DAG manager query,
storage-lookup, graph-status, and non-finalized-sync task family behind native
`DagTransactionService`. The public native root now returns owned domain
snapshots for reference validation, sync payloads, order/frontier/GHOST reads,
diagnostic graph rendering, lock-consistent status and non-finalized indexes,
block membership/loading, proposer-tip selection, period hashes, and persisted
counters. The bridge retains the existing CXX methods only as carrier
conversion; it no longer acquires a raw DAG guard or extends native state with
bridge-owned behavior. `DagService`, its mutable state and guard, and the
root's DAG lock escape hatches are crate-private. Eleven superseded bridge DAG
runtime/storage/finalization tests and one duplicate composed-finalization test
are deleted; native storage/state tests plus a root-level query snapshot test
cover the moved semantics, while two leaf conversion tests remain for the C++
worker command and legacy VDF bytes. The CXX ABI and retained FinalChain, EVM,
signing, VDF, transport, and public-query boundaries are unchanged. The checked
bridge budget falls by 1,238 lines to 39,036; all other checked surface budgets
are unchanged.

The following bounded `CRW-12` owner contraction moves the complete
transaction read-task family behind native `TransactionService` and
`DagTransactionService`. Fifteen production operations now acquire and release
the transaction mutex inside the native owner: gas bid and estimation
decisions, transaction count and knownness, queue groups/size/accounts/drop and
limit facts, minimum inclusion price, and queue-only, sidecar-only, combined,
and proposal-period transaction views. Native requests and results use owned
hashes, addresses, scalars, canonical RLP, and an explicit
declared/cached/external-EVM estimation enum; the bridge only converts those
values to the unchanged CXX carriers. Proposal lookup preserves permissive
missing-nonce-fact behavior, storage lookup remains in the same lock epoch as
queue/sidecar precedence, and every native guard is released before the
retained EVM or FinalChain leaf executes. The production infallible transaction
accessor, both read-forwarding macros, and fifteen read methods on
`TransactionRuntimeAccess` are deleted. Six superseded or stale bridge
behavioral tests and their test-only storage/proposal lookup duplicates are
replaced by native service coverage for source precedence, finalized nonce
filtering, queue/gas facts, and declared/cache/EVM estimation decisions.
Mutation, admission, cache-store, queue-finalization, sidecar, and packing
adapters remain the explicit transaction guard escape-hatch debt. The checked
bridge budget falls by 856 lines to 38,180; CXX functions, carriers, handles,
shim lines and directories, granular flags, partial factories, compatibility
constructors, production C++ consumers, and public signatures are unchanged.

The next bounded `CRW-12` transaction contraction moves compatibility packing
and the remaining direct mutation family behind lock-owning native
`TransactionService` and `DagTransactionService` APIs. Rust now owns
compatibility pack preparation/finalization/abort, canonical candidate
inspection, queue/cache effect publication, gas-oracle and estimation-cache
updates, recently-finalized sidecar initialization, durable non-finalized
removal, and finalized-block queue expiry. Packing alone retains an
owner-scoped cursor across the unlocked EVM interval; every other operation
completes in one native transaction lock epoch. The bridge preserves all CXX
signatures and only converts owned hashes, scalars, canonical RLP, estimates,
selections, and demotion facts. Eight production guard methods, the infallible
mutation-forwarding macro, bridge-local candidate inspection, and nine
superseded bridge behavioral tests are deleted; native service coverage now
exercises packing, cache/sidecar publication, durable removal, gas updates, and
queue expiry. The C++ compatibility mutex remains explicit debt because
admission, DAG-save/finalized-status, recovery, and finalized
filter/verification still use multi-step compatibility fact collection. The
checked bridge budget falls by 807 lines to 37,373; CXX functions, carriers,
handles, shim lines and directories, granular flags, partial factories,
compatibility constructors, production C++ consumers, and public signatures
are unchanged.

The following bounded `CRW-12` transaction contraction moves finalized
filtering, first-finalized verification, and startup non-finalized recovery
behind lock-owning native `TransactionService` and `DagTransactionService`
tasks. Rust now owns sidecar/storage precedence, sender-nonce-gated finalized
lookups, source classification, stale finalized-row cleanup, survivor envelope
validation, and atomic live-sidecar publication. The bridge preserves the CXX
signatures and converts only owned indices, hashes, nonces, and result tags.
Three bridge behavioral implementations, their recovery insertion helper, and
four superseded bridge tests are deleted; native transaction-service coverage
exercises recent-sidecar and durable filtering, both verification sources, and
validated recovery publication. The remaining transaction guard debt is
fact-backed admission and DAG-save/finalized-status. The checked bridge budget
falls by 420 lines to 36,953; CXX functions, carriers, handles, shim lines and
directories, granular flags, partial factories, compatibility constructors,
production C++ consumers, and public signatures are unchanged.

The next bounded `CRW-12` transaction contraction moves validated and public
admission behind lock-owning native `TransactionService` and
`DagTransactionService` tasks. Rust now owns known-fast-path precedence,
verification, latest-account eligibility, proposable/non-proposable queue
mutation, overflow/drop observation, insertion status and legacy public-result
selection, and the transaction-added shell fact. The retained C++ FinalChain
boundary supplies owned account and finalized-location facts; the bridge only
converts those facts, canonical queue entries, and native reports into unchanged
CXX carriers. The bridge admission state machine, private planning carriers,
guard methods, mutation helpers, and one duplicated planner test are deleted;
native service tests cover accepted publication, known-before-verification
precedence, and rejection without mutation. The remaining transaction guard
escape hatch is DAG-save/finalized-status. The checked bridge budget falls by
422 lines to 36,531; CXX functions, carriers, handles, shim lines and
directories, granular flags, partial factories, compatibility constructors,
production C++ consumers, and public signatures are unchanged.

The following bounded `CRW-12` transaction contraction moves finalized-status
persistence, recently-finalized sidecar publication, queue-known/erasure
mutation, and periodic account-nonce purge behind one lock-owning native task.
Count persistence retains storage-before-memory ordering; the bridge now only
converts finalized payload/account facts and returned logging hashes into the
unchanged CXX report. The bridge status state machine, purge composition, and
two duplicated behavioral tests are deleted; native coverage proves durable
count, sidecar, queue, and report publication. DAG-save is now the sole
transaction guard escape hatch. The checked bridge budget falls by 269 lines to
36,262; CXX functions, carriers, handles, shim lines and directories, granular
flags, partial factories, compatibility constructors, production C++ consumers,
and public signatures are unchanged.

The next bounded `CRW-12` transaction contraction moves standalone DAG
transaction persistence behind lock-owning native `TransactionService` and
`DagTransactionService` tasks. Rust now owns finalized filtering, count
planning, cloned queue/sidecar prepublication, the durable batch,
commit-before-publish ordering, and the typed accepted outcome. The bridge
converts only owned CXX facts and the unchanged queue-erasure report. Its
DAG-save state machine, prepared persistence/publication carriers, storage
helpers, production guard/access wrappers, and duplicated behavioral test are
deleted; native success and injected commit-failure tests now prove durable
commit-before-publication behavior beside the owner. No production transaction
guard escape hatch remains. The checked bridge budget falls by 275 lines to
35,987; CXX functions, carriers, handles, shim lines and directories, granular
flags, partial factories, compatibility constructors, production C++ consumers,
and public signatures are unchanged.

The following bounded `CRW-12` transaction cleanup deletes the remaining
RocksDB-backed bridge runtime fixture and six bridge-local behavioral tests.
Native transaction-service and queue tests already own count restoration,
replacement payload retention, overflow eviction, and account cleanup; a new
native service test adds the missing admission-overflow drop-window assertion.
`rustaxa-bridge` retains one focused verification status-mapping test and owns
no transaction runtime fixture, cleanup planner, queue mutator, storage
lifetime, or drop-observation state even under tests. The checked bridge budget
falls by 507 lines to 35,480; CXX functions, carriers, handles, shim lines and
directories, granular flags, partial factories, compatibility constructors,
production C++ consumers, and public signatures are unchanged.

The following bounded `CRW-12` DAG cleanup deletes seven RocksDB-backed
add-block behavioral tests and their bridge-only block/request fixtures.
Existing native root tests already cover atomic commit and restart, injected
commit failure, terminal and save-false outcomes, stale-safe abort, and
concurrent cursor publication. Native coverage now also owns compatibility
object identity with supplied-only transaction persistence, finalized-nonce
filtering, active-cursor preservation across terminal and malformed retries,
and duplicate/count-mismatched nonce facts. `rustaxa-bridge` retains the
unchanged CXX conversion and focused external-leaf/ABI tests but no add-block
protocol suite. The checked bridge budget falls by 589 lines to 34,891; CXX
functions, carriers, handles, shim lines and directories, granular flags,
partial factories, compatibility constructors, production C++ consumers, and
public signatures are unchanged.

The next bounded `CRW-12` DAG cleanup deletes the last bridge-local proposer
packing behavioral test. Native `DagTransactionService` tests already cover
the unlocked estimate interval, native finalize and cache reuse, terminal and
failure cleanup, DAG poison isolation, compatibility ownership, and malformed
queue payloads. The bridge retains only conversion and external-executor
wiring coverage for this family. The checked bridge budget falls by 116 lines
to 34,775; CXX functions, carriers, handles, shim lines and directories,
granular flags, partial factories, compatibility constructors, production C++
consumers, and public signatures are unchanged.

The next bounded `CRW-12` DAG cleanup moves the complete verifier-transaction
behavioral suite beside native `DagTransactionService`. Native tests now cover
all-supplied completion without advancement, canonical queue/sidecar
resolution and deduplication, missing-transaction rejection, strict finalized
account-nonce facts and old-finalized rejection, plus stale cursor, proposal
period, and stage handling. Five RocksDB-backed bridge tests and three
bridge-only PBFT/period-data fixtures are deleted; the bridge retains only
conversion and external-leaf wiring coverage for this family. The checked
bridge budget falls by 376 lines to 34,399; CXX functions, carriers, handles,
shim lines and directories, granular flags, partial factories, compatibility
constructors, production C++ consumers, and public signatures are unchanged.

The next bounded `CRW-12` PBFT cleanup deletes seven bridge-local transcripts
that duplicate both native owner coverage and exact CXX boundary tests:
daemon-session action ordering, certify progression, cert-progress restart,
round-reset effects, cursor mismatch, startup restoration/normalization, and
structured finalization failure propagation. Native `pbft_manager` and
finalization tests remain the protocol authority, while
`rust_consensus_tests` exercises the corresponding CXX calls. Focused bridge
tests remain for unique bootstrap gating, period-data queue conversion,
invalid-stage non-publication, ineligible-sleep mapping, queue-drain mapping,
and unknown-enum conversion. The checked bridge budget falls by 226 lines to
34,173; CXX functions, carriers, handles, shim lines and directories,
granular flags, partial factories, compatibility constructors, production C++
consumers, and public signatures are unchanged.

The following bounded `CRW-12` PBFT test-ownership cleanup deletes five more
bridge-local happy-path planner transcripts for finalization readiness,
eligible-wallet period readiness, block validation, candidate admission, and
leader-candidate selection. Native tests
`finalization_wait_planner_waits_until_delegation_delay_is_covered`,
`eligible_wallet_period_wait_planner_waits_until_period_matches_chain_size`,
`block_validation_planner_drives_live_checks_in_legacy_order`,
`block_validation_planner_handles_final_chain_wait_and_rejections`,
`candidate_admission_plans_lookup_validation_and_mark_valid`, and
`leader_candidate_planner_derives_statuses_and_mark_valid_commands` own the
stronger domain behavior. The bridge retains the unknown validation-status
mapping test, one compact normal-path carrier sentinel, plus bootstrap,
persistence, external-leaf, and CXX conversion coverage. The checked bridge
budget falls by 50 lines to 34,123; CXX
functions, carriers, handles, shim lines and directories, granular flags,
partial factories, compatibility constructors, production C++ consumers, and
public signatures are unchanged.

The next bounded `CRW-12` PBFT ownership contraction moves the complete
period-state cleanup task from `rustaxa-bridge` onto native `PbftService`.
Native consensus now owns the verified-votes-before-proposed-blocks lock order,
successor validation, stale vote/payload and proposal planning, the single Rust
proposal-deletion batch, commit-before-memory publication, typed no-op and
rejection results, exact mutation counts, and retry after injected commit
failure. The two bridge behavioral tests move with the task; one exhaustive
carrier test remains around the unchanged CXX result and entrypoint used by the
temporary C++ advance-period executor. The checked bridge budget falls by 294
lines to 33,829; CXX functions, carriers, handles, shim lines and directories,
granular flags, partial factories, compatibility constructors, production C++
consumers, and public signatures are unchanged. This was the intermediate
boundary at that slice; the later period-commit contraction documented below
deletes that remaining cleanup carrier and entrypoint.

The following bounded `CRW-12` PBFT ownership contraction moves the complete
leader-selection prepare/revalidate/finish workflow from `rustaxa-bridge` onto
native `PbftService`. Native consensus now owns the verified-votes,
proposed-blocks, and chain snapshot under the manager-before-siblings lock
order, deterministic candidate snapshot and V1 fingerprint, unlocked
external-validation contract, exact report-set and stale snapshot rejection,
planner invocation, whole-command-set prevalidation, and validity publication.
Five bridge protocol tests move beside the native owner and native regression
coverage adds multi-command publication plus manager-serialized finalized
membership. The bridge retains one focused end-to-end prepare/finish and
exhaustive status/payload conversion test around the unchanged two CXX
entrypoints and five carriers used by the temporary C++ block validator
executor. The checked bridge budget falls by 330 lines to 33,499;
CXX functions, carriers, handles, shim lines and directories, granular flags,
partial factories, compatibility constructors, production C++ consumers, and
public signatures are unchanged.

The next bounded `CRW-12` PBFT test-ownership cleanup deletes three
bridge-local reward-vote finalization transcripts and their test-only generic
finalization storage executor, replacing them with one compact boundary-only
sentinel. Native `pbft_vote_runtime` coverage already owns
ordered payload selection, durable generation-bound cursor publication,
idempotence, conflicts, and restart behavior. Native `pbft_finalize` coverage
owns reward-reset bundle persistence, authoritative stale-row deletion,
serialization against in-flight admission writes, and rejected write-set
statuses. The unchanged production CXX methods remain DTO adapters for the
temporary VoteManager/finalization executor, while `rustaxa-bridge` no longer
executes a parallel generic finalization write path under tests. The retained
sentinel covers reset identity/bundle conversion, ordered selected records,
cursor Applied/AlreadyCurrent/Rejected codes, and rejected-result mapping. The
checked bridge budget falls by 148 lines to 33,351; CXX functions, carriers, handles,
shim lines and directories, granular flags, partial factories, compatibility
constructors, production C++ consumers, and public signatures are unchanged.

The following bounded `CRW-12` ownership slice moves the live reward-vote
finalization workflow behind native `PbftVerifiedVotesService`. Native
consensus now owns coherent cursor/payload snapshots, ordered reward
selection, exact cert-identity reset-stage preparation, the runtime-before-
storage lock order, standalone reset persistence plus live cursor publication,
combined-finalization generation acknowledgement, and restart restoration.
The standalone task holds the vote-runtime lock through canonical bundle
derivation and the storage-owned reset batch, then publishes the
generation-bound cursor only for a successful durable result. Native tests
cover identity rejection, authoritative stale-row deletion, publication,
idempotent replay, pre-commit stale/conflicting-cursor rejection,
combined-batch acknowledgement, and restart. The unchanged
CXX functions are conversion-only adapters for temporary C++ consumers, and
their compact sentinel preserves payload ordering and legacy status codes. The
checked bridge budget falls by 15 lines to 33,336; CXX functions, carriers,
handles, shim lines and directories, granular flags, partial factories,
compatibility constructors, production C++ consumers, and public signatures
are unchanged.

The next bounded `CRW-12` DAG sibling slice moves finalized-period sortition
preview and commit composition from `rustaxa-bridge` into native
`DagTransactionService`. Native consensus now owns canonical period-efficiency
count conversion, no-pivot handling, preview, exact expected-change
validation, clone-before-publication mutation, and the current threshold plus
history-count snapshot under one sortition mutex epoch. Calculation, poison,
and mismatch failures publish no state. Native tests replace the deleted
bridge helper test and cover changed/unchanged commit, mismatch non-publication,
and malformed-count rejection; the retained bridge tests exercise PBFT
executor integration and fatal error projection. The two bridge-only
sortition-change carriers are deleted, and production bridge guard access is
gone. PBFT chain-head/period-data preparation and the temporary
expected-change/at-most-once retry contract remain tracked CRW-12 debt. The
checked bridge budget falls by 161 lines to 33,175 and CXX carriers fall by two
to 348; CXX functions, handles, shim lines and directories, granular flags,
partial factories, compatibility constructors, production C++ consumers, and
public signatures are unchanged.

The following bounded `CRW-12` cross-root contraction moves finalized-period
sortition preparation from `rustaxa-bridge` into the lock-held native PBFT
manager task. Native consensus now rejects caller-owned sortition stages,
validates the manager-owned chain successor and non-empty size, decodes
canonical period-data counts, checks pivot/null-anchor consistency, previews
through the native DAG/transaction owner, appends the exact storage stage, and
retains the existing native commit request for direct post-storage publication.
The duplicate preparation type and bridge-side request reconstruction are
deleted; the DAG bridge exposes only an operation-specific delegate around its
private native root. Native tests cover canonical publication and malformed
RLP, head mismatch/overflow, and pivot mismatch without state or stage
publication, while the retained bridge suite covers CXX start/commit,
caller-stage rejection, storage failure cleanup, fatal drift projection, and
stale-cursor mapping. The compatible separately restored DAG-service C++ test
remains because the portable expected-change contract is still supported. The
checked bridge budget falls by 170 lines to 33,005; CXX functions, carriers,
handles, shim lines and directories, granular flags, partial factories,
compatibility constructors, production C++ consumers, and public signatures
are unchanged. The full-state preview fingerprint and at-most-once retry
constraint remain explicit CRW-12 debt.

The next bounded `CRW-12` sortition contraction moves the complete
post-storage commit workflow from `rustaxa-bridge` onto the lock-held native
PBFT manager owner. Native consensus now derives the current action, validates
cursor/action and retained plan/request consistency, commits through the
native DAG/transaction owner in manager-before-sortition order, preserves the
stable post-storage invariant prefix, consumes the retained request exactly
once after successful publication, constructs and validates all live facts,
and reports the result to the native finalization cursor. The bridge retains
only an operation-specific delegate, domain-step conversion, and the generic
owned-action drain used by remaining finalization families. Native tests cover
successful advancement/request consumption, stale-cursor non-publication, and
preview-drift fatal rejection; the superseded unchanged, drift, and stale
bridge transcripts are deleted while one changed-commit CXX sentinel and the
preparation/storage/caller-stage boundary tests remain. The checked bridge
budget falls by 272 lines to 32,733; CXX functions, carriers, handles, shim
lines and directories, granular flags, partial factories, compatibility
constructors, production C++ consumers, and public signatures are unchanged.
The portable full-state fingerprint and native generic drain/outcome shaping
remain explicit CRW-12 debt.

The following bounded `CRW-12` PBFT finalization contraction moves reward-vote
reset advancement from bridge-owned cursor/report behavior onto
`PbftManagerGuard`. Native consensus now owns missing-session/current-action
classification, stale-cursor handling, stored-plan lookup, exact nonzero
manager/storage reset-generation provenance, the complete reward live report,
validation, and runtime reporting. The bridge keeps the existing CXX report
conversion plus domain-step mapping, generic drain, and terminal cleanup.
Native tests cover successful advancement, invalid provenance, and metadata
mismatch; duplicate bridge rejection transcripts are deleted while one success
CXX sentinel and next-cycle admission/resume integration remain. The checked
bridge budget falls by 91 lines to 32,642; CXX functions, carriers, handles,
shim lines and directories, granular flags, partial factories, compatibility
constructors, production C++ consumers, and public signatures are unchanged.
A future direct `PbftService`-to-verified-votes task should delete the
remaining C++ `VoteManager` reset-report/cursor relay rather than preserve it
as compatibility.

That direct native composition is now complete. `PbftManagerGuard` derives the
accepted reward cursor from its retained finalization plan and commits it
through the native verified-vote owner under manager-before-vote lock order.
The C++ `VoteManager` commit method and report carrier, the manager report
carrier, and the CXX verified-vote cursor-commit export/carrier are deleted;
the manager CXX entrypoint is now reportless. Native tests retain success,
generation-provenance, and metadata-mismatch coverage. The checked bridge
budget falls by 229 lines to 32,413, shim lines fall by 78 to 17,438, CXX
functions fall by one to 397, and carriers fall by two to 346. Handles, shim
directories, flags, partial factories, compatibility constructors, and
production consumers are unchanged.

The obsolete reset-generation field is also removed from the manager executor
CXX state after its last C++ reader disappeared. The bridge-only fresh,
duplicate, and resume propagation transcript is deleted; equivalent generation
and resume semantics remain covered by native manager, finalization, and
verified-vote tests. This removes another 100 bridge lines, lowering the exact
budget to 32,313 without changing functions, carriers, handles, shims, flags,
factories, constructors, or consumers.

Five drain telemetry fields with no production C++ reader are removed from the
finalization executor carrier together with their bridge-only accumulation
state. Native drain tests remain authoritative for owned-action execution; the
CXX boundary retains only cursor/control state, the cache-clear effect, and the
manager snapshot consumed by the overlay. This deletes 56 bridge lines and
lowers the exact budget to 32,257. Functions, carrier count, handles, shims,
flags, factories, constructors, and consumers are unchanged.

The drain adapter now also stops materializing a duplicate status and manager
snapshot inside its intermediate result. CXX conversion reads the native next
step and manager snapshot directly, while native and focused bridge tests cover
the unchanged mutation and cursor behavior. This removes 18 more bridge lines,
setting the exact budget to 32,239 with every other inventory metric unchanged.

The next bounded `CRW-12` finalization contraction moves finalized-transaction
advancement onto `PbftManagerGuard` and the native `DagTransactionService`.
Rust owns current-action/cursor validation, canonical transaction decoding and
hashing from the accepted `PeriodData`, storage-first count persistence,
sidecar/queue/account-nonce purge mutation, accepted-count validation, stable
fatal error projection, and runtime reporting. The C++ PBFT overlay supplies
only the retention window and narrow FinalChain account nonce facts. The
per-transaction payload carrier, mutation-report carrier, C++ payload
inspection/report logging loop, and manager report relay are deleted. Native
tests cover canonical decoding, malformed pre-mutation rejection, advancement,
count mismatch, and fatal failure retention; focused transaction-shim,
PBFT-manager, and CXX PBFT transcript tests preserve production parity. The
non-PBFT stable `TransactionManager` method remains a compatibility client and
passes one opaque transaction-list RLP for partially populated legacy objects.
The checked bridge budget falls by 40 lines to 32,199, shim lines fall by 35 to
17,403, and carriers fall by two to 344. Functions, handles, shim directories,
flags, partial factories, compatibility constructors, and production consumers
are unchanged.

The next bounded `CRW-12` finalization contraction moves the complete
manager-owned action drain from `rustaxa-bridge` onto `PbftManagerGuard`.
Native consensus now derives the retained plan, storage intent, current action,
chain, and scalar state under the manager serialization domain; drains
PBFT-chain publication, anchor-cache clearing, dynamic-lambda storage/live
publication, and executed-status storage/live publication; validates and
reports each action; and stops unchanged at every external or sibling-service
boundary. Chain state is projected and validated before publication, while
dynamic-lambda and executed-status storage commits precede live publication.
The bridge keeps only one argument-free native call, executor-state projection,
and terminal cleanup. Its duplicate drain state/result, stage constructors,
behavioral loop, and six superseded behavioral transcripts are deleted; one
conversion sentinel remains. Seven native tests cover pre-/post-FinalChain
ordering, chain/cache publication, dynamic-lambda persistence and idempotent
replay, executed-status completion, missing-session projection, and rejection
without invalid chain publication. The checked bridge budget falls by 567 lines to 31,632. CXX
functions, carriers, handles, shim lines and directories, granular flags,
partial factories, compatibility constructors, and production consumers are
unchanged.

The following `CRW-12` finalization contraction moves the complete executor
start/resume workflow from `rustaxa-bridge` into native `PbftService` and
`PbftManagerGuard`. Under the manager serialization domain Rust now clears
stale state, preserves only a storage-authenticated resume generation, inspects
durable resume state, installs the fresh cursor, composes sortition preparation
through native `DagTransactionService`, applies primary storage, reports the
result, drains owned actions, captures the manager snapshot, and clears every
terminal/error session. The bridge retains request/result conversion and one
CXX integration sentinel; three superseded bridge behavior tests are replaced
by native fresh, pre-/post-preparation rejection, unknown-mode,
resume-cleanup, and reset-generation authentication coverage. This deletes 215
bridge lines and lowers the exact budget to 31,417. CXX functions,
carriers, handles, shim lines and directories, granular flags, partial
factories, compatibility constructors, and production consumers are unchanged.

The next `CRW-12` contraction moves the complete remaining finalization
advancement envelope into native `PbftService` and `PbftManagerGuard`. All
eight typed CXX entrypoints now delegate cursor/action validation, native
sortition/reward/transaction composition, DAG/FinalChain/pillar/period fact
construction and validation, owned-action continuation, terminal/error
cleanup, and lock-coherent snapshot capture to the native application root.
The bridge-owned generic mutation report, plan lookup, cursor-report protocol,
drain continuation, cleanup policy, DAG-service delegates, and ten behavioral
tests are deleted; one boundary-projection test and the CXX executor sentinels
remain. Native manager tests replace DAG advancement, external failure and
stale-cursor cleanup, and FinalChain success/mismatch behavior, while existing
native suites retain sortition, reward-reset, transaction, resume, and owned
drain coverage. Direct application-root tests prove advancement-error cleanup
and advance-period-to-pillar completion/snapshot behavior. This deletes 1,164
bridge lines and lowers the exact budget to 30,253. CXX functions, carriers,
handles, shim lines and directories, granular
flags, partial factories, compatibility constructors, and production consumers
are unchanged.

The following `CRW-12` boundary contraction deletes the three redundant
FinalChain, pillar, and advance-period CXX report carriers. The retained C++
FinalChain/EVM and pillar leaves now return only facts that Rust cannot observe:
the post-dispatch FinalChain height and the pillar request period. Native
`PbftService` derives blocks-per-year and the processed PBFT period from its
retained finalization plan, and samples the post-advance manager period under
the manager serialization lock. A native application-root test proves
FinalChain advancement validates a nonzero retained blocks-per-year value;
existing native period/pillar coverage proves the other two derivations. This
deletes 45 bridge lines and 61 shim lines, lowering the exact budgets to 30,208
and 17,342, and lowers CXX carriers from 344 to 341. CXX functions, handles,
shim directories, granular flags, partial factories, compatibility
constructors, and production consumers are unchanged.

The next `CRW-12` contraction removes the PBFT-specific DAG-manager mutation
facade and finalized-count relay. `PbftService` now validates
`SetDagBlockOrder` cursor/action identity before mutation, derives the anchor,
period, and ordered hashes from the retained finalization plan, invokes native
`DagTransactionService`, validates its native count, drains owned actions, and
returns only expired-hash and counter-refresh compatibility effects. C++ no
longer calls `DagManager::setDagBlockOrderForPbftFinalization` or feeds a
synthetic count back through CXX; the replacement DAG shim method only updates
the public counter mirror and temporary seen-block cache. Native tests prove
ordered non-empty anchor application reaches the next leaf, stale/wrong
cursors cannot mutate DAG state, and operational failure clears the application
root without compatibility effects. This deletes one bridge line and seven shim lines,
lowering the exact budgets to 30,207 and 17,335. CXX functions, carriers,
handles, shim directories, granular flags, partial factories, compatibility
constructors, and production consumers are unchanged.

The following `CRW-12` contraction removes reward-vote reset preparation from
the C++ `VoteManager` facade. PBFT finalization now calls the
application-owned service's verified-vote preparation leaf directly, and the
redundant C++ prepared-state protocol flag is deleted; existing native
certificate validation, stage persistence, and error behavior are unchanged.
The narrow CXX leaf and storage-stage relay remain explicit startup-migration
debt. This deletes 29 shim lines, lowering the exact shim budget to 17,306.
Bridge lines, CXX functions, carriers, handles, shim directories, granular
flags, partial factories, compatibility constructors, and production
consumers are unchanged.

The next `CRW-12`/`CRW-15` contraction removes broad finalization-plan
materialization from the remaining direct `VoteManager::resetRewardVotes`
compatibility method. C++ now builds only the existing period/round/step/hash
identity request and delegates reset preparation, storage, and live publication
to the native verified-vote owner. The callerless plan builder, rejected-result
fabricator, and two public PBFT-finalization helper methods are deleted. This
deletes another 82 shim lines and lowers the exact shim budget to 17,224;
all other inventory metrics are unchanged.

The following `CRW-12` contraction moves reward-vote reset stage preparation
into native executor startup. `PbftService` composes its existing
verified-vote sibling into the manager task; fresh start rejects
caller-supplied reward stages, derives the exact certificate identity from the
accepted write intent, prepares the canonical bundle under the established
manager-before-verified-votes lock order, and retains that vote guard across
sortition preparation and the atomic primary storage commit so concurrent
admission cannot invalidate the durable bundle. Preparation errors clear stale sessions, plans, sortition
requests, and reset generations through the normal executor finish policy.
The PBFT shim preparation block, CXX preparation export, bridge conversion
helpers, reward-specific CXX stage fields, and bridge-only preparation
assertions are deleted. Native application-root tests use genuine signed,
weighted cert votes to prove atomic persistence and exact cursor publication;
identity mismatch, caller-stage, and blocked-storage concurrency tests prove
fail-closed cleanup and continuous serialization. The CXX boundary suite uses a
genuine signed cert admission to cover start through reward advancement and
stale-cursor rejection, retains missing-native-vote rejection, and isolates
unrelated cursor transcripts from reward fixture construction. The checked budgets fall
to 30,106 bridge lines, 17,209 shim lines, and 396 CXX functions. Carriers,
handles, shim directories, granular flags, partial factories, compatibility
constructors, and production consumers remain at 341, 20, 11, eight, zero,
zero, and 39.

The next `CRW-12` recovery slice closes the same-process failure window between
the atomic primary finalization commit and live reward-cursor publication.
Resume now preserves only a nonzero reset generation that still matches the
shared storage owner, requires both accepted reset intents, prepends the native
reward publication action ahead of the storage-derived replay tail, and relies
on the verified-vote owner to revalidate the exact durable cert cursor and
bundle before publication. Stale generations continue directly to the durable
FinalChain tail, while missing or conflicting durable reward facts fail closed.
The C++ dispatcher permits this one native lock-owning resume action without
DAG/transaction locks; every other protected action remains rejected outside
the fresh-finalization critical section. Native and CXX tests prove matching
generation replay, stale-generation exclusion, genuine signed-cert recovery,
and continuation to FinalChain. This behavioral ownership slice changes no
checked bridge/shim inventory budget.

The following `CRW-12` slice pairs bounded sortition recovery with complete
finalization-advancement export contraction. Same-process resume preserves a
sortition preview only when it emitted a concrete change and the exact change
RLP exists at the durable period key; full primary resume classification still
gates the transcript, and no-change previews remain explicitly non-replayable.
The native manager then prepends sortition before reward publication and the
storage-derived tail, preserving manager-to-sortition lock order without the
C++ DAG/transaction lock pair. The seven retained external actions now advance
through one action-gated CXX entrypoint that dispatches to typed native leaves
and consumes only matching leaf payloads. Six per-action exports plus duplicated
bridge and shim wrappers are deleted. Native tests cover durable match,
mismatch, no-change exclusion, and full-restart non-replay; CXX coverage proves
unlocked resume through sortition to FinalChain. The checked budgets fall to 29,924 bridge lines,
17,132 shim lines, and 390 CXX functions. Carriers, handles, shim directories,
granular flags, partial factories, compatibility constructors, and production
consumers remain at 341, 20, 11, eight, zero, zero, and 39.

The next bounded `CRW-12` sync-owner contraction moves the complete
synced-period admission cursor lifecycle and storage-backed sync egress task
out of `rustaxa-bridge`. Native `PbftService` gates session creation on
bootstrap readiness, and `PbftManagerService` owns session replacement,
cursor/check report validation, transaction-result application, terminal
cleanup, abort, and the storage handle used to load canonical period bytes and
select reward-vote attachment. The bridge keeps unchanged CXX functions as
plain request/result conversion and retains focused cert-vote and end-to-end
sync projection sentinels. Two RocksDB-backed admission tests and one egress test
move to a native application-root test covering readiness, ordered acceptance,
warnings, mismatch cleanup, and durable payload loading, while one compact
bridge test preserves end-to-end carrier/status projection coverage. The
checked bridge budget falls by 136 lines to 29,788; shim lines, CXX functions, carriers,
handles, shim directories, granular flags, partial factories, compatibility
constructors, and production consumers remain at 17,132, 390, 341, 20, 11,
eight, zero, zero, and 39. Tier 1 validation is `rewrite-validate-fast`; Tier 2
is the native application-root sync test, the bridge projection test, and all
56 `rust_consensus_tests`; Tier 3 is the Rust-enabled `taraxad` smoke build. No
upstream-owned C++ file changes.

The following bounded `CRW-12` test-ownership contraction deletes four PBFT
manager bridge behavior transcripts for daemon ineligible sleep, proposal
ordering/build, broadcast counter application, and deadline waiting. Native
manager tests already cover each rule directly; the bridge retains focused
carrier, persistence, bootstrap, and external-executor boundary sentinels. This
deletes a net 30 bridge lines and lowers the checked budget to 29,758. CXX
functions, carriers, handles, shim lines/directories, granular flags, partial
factories, compatibility constructors, and production consumers are unchanged.

The next `CRW-12` period-commit contraction removes cleanup from the C++
advance-period action script. After C++ executes the retained timer,
wallet-eligibility, vote-manager, and counter leaves, one fallible native
`PbftService` commit validates reset provenance and holds manager,
verified-vote, and proposed-block guards through durable proposal deletion,
live sibling cleanup, and infallible manager-period publication. Cleanup
failure preserves all live owners plus retryable reset provenance; duplicate
and mismatched reports retain the existing rejected-snapshot contract. The
standalone production cleanup API/result exports, action code 7, CXX carrier
and function, bridge module/conversion test, and shim executor/count-validation
branch are deleted. Native cleanup/retry/no-op tests and the 56-test CXX bridge
suite cover success and duplicate-rejection across the replacement route.
Checked budgets fall by 108 bridge lines, 31 shim lines, one CXX function, and
one carrier to 29,650, 17,101, 389, and 340. Handles, shim directories,
granular flags, partial factories,
compatibility constructors, and production consumers remain at 20, 11, eight,
zero, zero, and 39. No upstream-owned C++ files change.

The following bounded `CRW-12` test-ownership contraction removes three more
PBFT manager persistence transcripts from `rustaxa-bridge`: executed-block
reset, next-voted status, and round/step cursor fields. Native manager tests
now own each storage-before-runtime-publication contract and its invalid-field
or invalid-status rejection. The bridge retains the combined lifecycle
transition storage sentinel, one compact adapter status/error-mapping test,
and focused CXX projection/external-executor coverage. This deletes bridge
behavior lines while preserving ABI evidence: the bridge budget falls by 83
lines to 29,567. Shim lines, CXX functions, carriers, handles, shim directories,
granular flags, partial factories, compatibility constructors, and production
consumers remain at 17,101, 389, 340, 20, 11, eight, zero, zero, and 39. No
production caller or upstream-owned C++ file changes.
Tier 1 is `rewrite-validate-fast`; Tier 2 is the three focused native
persistence tests, the compact bridge adapter test, and all 56
`rust_consensus_tests`. Tier 3 is not required because production routing,
C++ code, startup, and externally observable behavior are unchanged.

The next bounded `CRW-12` test-ownership contraction deletes the bridge-local
broadcast-counter and cached-anchor DAG-order metadata behavior fixtures.
Native manager tests already cover counter validation/non-mutation/reset and
complete anchor record/query/remove/clear membership semantics. The live CXX
wrappers remain direct scalar/hash projections exercised by production C++
compilation; they own no conversion, lifetime, or error policy requiring a
parallel RocksDB fixture. This removes 139 bridge lines and lowers the checked
budget to 29,428. All other inventory metrics and 39 production consumers are
unchanged; no production caller or upstream-owned C++ file changes. Tier 1 is
the fast gate; Tier 2 is the two focused native tests, the remaining 22 PBFT
bridge tests, and all 56 `rust_consensus_tests`. Tier 3 is not required because
production routing and C++ code are unchanged.

The following `CRW-12` lifecycle-rejection contraction moves unneeded
network-step presence validation into a native runtime test and replaces the
bridge's separate RocksDB rejection fixtures with one compact unknown-kind
and network-step projection/non-mutation sentinel. This removes 24 bridge
lines and lowers the checked budget to 29,404; every other inventory metric
remains unchanged.
Focused native and bridge rejection tests, the fast gate, and the unchanged
56-test CXX suite cover the slice. Tier 3 is not required because production
routing and C++ code are unchanged.

The final bounded `CRW-12` test-ownership contraction in this work window moves
missing-Cacti-lambda startup rejection and storage non-mutation coverage from
the bridge into the native manager restore tests. No CXX conversion is involved
in this test-only helper path. This removes 31 bridge lines and lowers the
checked budget to 29,373; all other inventory metrics remain unchanged.
Focused native restore and remaining 20 bridge-manager tests pass. Tier 3 is
not required because production routing and C++ code are unchanged.

The last time-boxed `CRW-12` test contraction deletes the standalone bridge
own-vote reset fixture. Native transition-storage coverage owns exact deletion
semantics, and the retained combined bridge lifecycle/storage sentinel already
proves the production wrapper clears durable own votes. This removes 22 bridge
lines and lowers the checked budget to 29,351; all other inventory metrics
remain unchanged. The focused native transition test and remaining 19 bridge
manager tests cover the slice; no production or C++ code changes.

The final `CRW-12` work-window contraction deletes the bridge-local
period-advance provenance behavior transcript. Native manager/service tests
own missing, mismatched, consumed, and retryable reset provenance, while the
retained CXX advance-period transcript covers production reset, planning,
successful commit, and duplicate rejection. This removes 33 bridge lines and
lowers the checked budget to 29,318; other inventory metrics remain unchanged.
Focused native period-advance tests and the remaining 18 bridge-manager tests
cover the deletion; no production or C++ code changes.

The next bounded `CRW-12` vote-progress contraction deletes five bridge-local
planner transcripts and their test-only reconstruction of a CXX
`VerifiedVoteAddOutcome`. Native `pbft_vote_progress` tests own insert
prechecks, stale reward persistence, threshold advancement, conflicting-vote
slashing, and missing proposed-block sidecar behavior. Production projection
now consumes only the native plan rather than redundantly accepting the
already-consumed progress fact and context. The bridge retains one focused
effect/status projection sentinel plus the existing admission-result
publication-gating sentinel. This removes a net 219 bridge lines and lowers
the checked budget to 29,099; CXX functions, carriers, handles, shim lines and
directories, granular flags, partial factories, compatibility constructors,
and production consumers remain unchanged. No production CXX signature or C++
source changes.
Tier 1 `rewrite-validate-fast`, all 15 native vote-progress tests, the focused
bridge projection/publication tests, inventory live/self checks, the storage
boundary guard, and whitespace validation pass. Tier 2 is the Rust-enabled
56-test `rust_consensus_tests` suite with the complete feature bundle enabled.
Tier 3 is not required because production behavior, the CXX ABI, C++ code,
startup, and externally observable routing are unchanged. All touched paths
are Rust or rewrite documentation, so there is no upstream-owned C++
intersection.

The following bounded `CRW-12` manager-session test-ownership contraction
deletes the bridge-local PBFT sync queue-drain and state-action effect behavior
transcripts plus seven duplicate CXX manager transcripts for tick ordering,
restart/reset decisions, invalid round/cursor reports, certify transition, and
finish-polling state actions. Native `pbft_sync` and `pbft_manager` tests own
those state machines, including executor failure, invalid-report, sidecar, and
completion branches. The bridge retains one combined queue/state projection
sentinel and bootstrap-readiness coverage; the CXX suite retains
storage-backed startup, period advance, finalization/resume, and external-leaf
boundaries. This removes a net 66 bridge lines and 278 CXX test lines, lowering
the checked bridge budget to 29,033 and the Rust-enabled CXX suite from 56 to
49 tests. CXX functions, carriers, handles, shim lines/directories, granular
flags, partial factories, compatibility constructors, production consumers,
production signatures, and routing are unchanged. Focused bridge coverage,
all three native queue-drain tests, all five native state-action session tests,
all 125 native manager tests, and the 49-test CXX suite pass. Tier 3 is not
required because no production behavior, startup route, or ABI changed.

The next bounded `CRW-12` storage-read ownership contraction moves
dynamic-lambda policy-plus-prior-lambda composition out of `rustaxa-bridge`
and onto native `PbftService`. The application root now plans the update and,
only for accepted active nonzero periods, loads the closest persisted lambda
through its manager-owned storage handle; period zero explicitly has no
predecessor, and the bridge performs CXX conversion only. A
native application-root test covers found and missing prior-lambda decisions,
while the retained CXX dynamic-lambda case preserves end-to-end projection.
Four additional bridge RocksDB fixtures for DAG/PBFT existence, cert-voted
recovery, own pillar vote, and startup replay are deleted because their exact
found/missing, payload, validation, and persistence contracts already live in
native DAG, PBFT-chain/manager, pillar, and storage tests. One compact bridge
sentinel retains DAG lookup, startup replay, and dynamic-lambda carrier
projection, including hash lists and optional/prior lambda encoding. This
removes a net 288 bridge lines and lowers the checked budget to 28,745; CXX
functions, carriers, handles,
shim lines/directories, granular flags, partial factories, compatibility
constructors, and production consumers remain unchanged. Production CXX
signatures and external behavior are unchanged.
Tier 1 `rewrite-validate-fast`, the 14 native PBFT-service tests, the focused
bridge storage-projection sentinel, and the inventory/storage guards pass.
Tier 2 is the complete 49-test Rust-enabled `rust_consensus_tests` suite; Tier
3 is the Rust-enabled `taraxad` build. No upstream-owned C++ file changes.

The next bounded `CRW-12` period-data queue ownership contraction moves all
queue locking and lifecycle access behind native `PbftService`. Snapshot,
push, pop, stale cleanup, and clear now accept or return native domain types;
`rustaxa-bridge` only converts stable CXX payloads and no longer reaches
`manager_state().period_data_queue`. Native queue tests retain sequencing,
backfill, chain-advance clearing, tail visibility, certificate selection,
cleanup, and empty-pop coverage, while a new application-root test proves the
owned service lifecycle. The duplicate bridge RocksDB behavior transcript and
its fixtures are replaced by one pure all-field input/output conversion
sentinel. A duplicate happy-path lifecycle-transition transcript is also
deleted; native transition tests and the retained bridge commit-before-cursor
fixture own that behavior. This removes a net 155 production bridge lines and
lowers the checked budget to 28,590; the pure carrier sentinel lives in test
support outside the production bridge source inventory. CXX functions,
carriers, handles, shim lines and
directories, flags, factories, constructors, production consumers, C++
signatures, live `PeriodData` sidecar ownership, and external behavior remain
unchanged. Tier 1 `rewrite-validate-fast`, all seven native queue tests, the
native application-root lifecycle test, the exhaustive bridge carrier
sentinel, inventory/storage guards, and whitespace validation pass. Tier 2 is
the complete 49-test Rust-enabled `rust_consensus_tests` suite. Tier 3 is the
Rust-enabled `taraxad` build. Independent review approves the final ownership,
parity, visibility, and coverage shape. No upstream-owned C++ file changes.

The next bounded `CRW-12` lifecycle-transition ownership contraction moves
transition planning, own-vote family locking, durable manager/status/vote
commit, runtime cursor publication, and reset-provenance recording from
`rustaxa-bridge` into native `PbftService`. The bridge now only converts the
stable request and projects typed C++ executor effects for timers, live
sidecars, logging, and VoteManager period/round updates. Native application
root tests cover commit-before-publication, own-vote cleanup, unknown-kind and
network-step rejection, snapshot preservation, and cleared rejected effects.
The duplicate bridge RocksDB transcript is deleted; its compact unknown-kind
projection sentinel and the CXX lifecycle/period-advance cases remain. This
removes a net 133 production bridge lines and lowers the checked budget to
28,457. CXX functions, carriers, handles, shim lines/directories, flags,
factories, constructors, production consumers, public signatures, and the
named C++ external-effect boundary remain unchanged. Tier 1
`rewrite-validate-fast`, the focused native service tests, retained bridge
rejection sentinel, inventory/storage guards, and whitespace validation pass.
Tier 2 is the complete 49-test Rust-enabled `rust_consensus_tests` suite. Tier
3 is the Rust-enabled `taraxad` build. Independent review approves lock order,
commit-before-publication, effect parity, and boundary conversion. No
upstream-owned C++ file changes.

The following bounded `CRW-12` manager-persistence contraction moves delayed
executed-block reset, next-voted status, and round/step cursor write sequencing
behind native `PbftService`. Each operation now holds the manager domain,
performs its supported durable write, and publishes the runtime snapshot only
after success. Unsupported next-voted ids and cursor fields preserve the live
snapshot. The bridge is reduced to direct status/snapshot projection, and its
duplicate RocksDB transcript is replaced by a native application-root test
covering successful persistence plus rejected non-publication. This removes a
net 104 production bridge lines and lowers the checked budget to 28,353. CXX
functions, carriers, handles, shim lines/directories, flags, factories,
constructors, production consumers, public signatures, and C++ executor
ordering remain unchanged. No upstream-owned C++ file changes.

Validation is complete at Tier 1 with `make rewrite-validate-fast`, the focused
native service persistence test, the retained bridge rejection edge test, the
inventory guard and self-test, the storage-boundary guard, and whitespace
checks. Tier 2 is the complete 49-test Rust-enabled `rust_consensus_tests`
suite. Tier 3 is the `taraxad` build. Independent review approves durable
write-before-publication ordering, rejection parity, direct bridge projection,
test replacement, documentation, and the exact 28,353-line inventory budget;
the residual non-blocking risk is the absence of injected RocksDB operational
failure at the service layer, whose ordering remains structurally enforced.

The next bounded `CRW-12` session-ownership contraction closes bridge access to
the queue-drain, daemon-tick, state-action-effect, and proposal executor cursors. Native
`PbftService` now owns readiness gating, cursor replacement, step advancement,
report validation, abort publication, and every manager lock epoch for those
four families. Their runtime fields are crate-private; the bridge only converts
the unchanged CXX facts, reports, owned steps, and stable missing-session
sentinels. A native application-root test covers fail-closed startup, state
action availability, ready daemon/proposal publication, and observable abort.
The now-callerless bridge and native readiness-object accessors are deleted;
clients retain only task-shaped `is_ready` and `complete_bootstrap` operations.
This removes a net 51 production bridge lines and lowers the checked budget to
28,302. CXX functions, carriers, handles, shim lines/directories, flags,
factories, constructors, production consumers, CXX public signatures, and retained
FinalChain, DAG-order, signing, timer, and transport executor boundaries remain
unchanged. No upstream-owned C++ file changes.

Tier 1 `rewrite-validate-fast`, the focused native owner test, both retained
bridge session/readiness projection tests, the exact inventory and storage
boundary guards, and whitespace validation pass. Tier 2 is the complete
49-test Rust-enabled `rust_consensus_tests` suite. Tier 3 is the `taraxad`
build. Independent review caught and verified the constructor-under-lock
ordering correction, then approved readiness parity, cursor/report/abort
semantics, missing-session sentinels, visibility contraction, obsolete-accessor
deletion, tests, documentation, and inventory. Residual non-blocking risk is the
absence of a dedicated concurrent replacement stress test; existing native
domain transcripts cover normal and rejected reports.

The following bounded `CRW-12` scalar-task contraction moves manager snapshot
capture, post-reset advance planning, broadcast-counter publication, cert-voted
recovery load/save/metadata, DAG-order sidecar membership, and deadline sleep
planning behind lock-owning native `PbftService` methods. Cert-voted recovery
keeps its durable-write-before-live-publication order in one manager lock epoch;
all other tasks capture or mutate a single lock-consistent runtime view. The
bridge now only converts unchanged CXX inputs and owned results. A native
application-root test covers broadcast publication, snapshot projection,
idempotent cache membership, durable cert-vote recovery, and elapsed-deadline
sleep behavior. This removes a net 21 production bridge lines and lowers the
checked budget to 28,281. CXX functions, carriers, handles, shim lines and
directories, flags, factories, constructors, production consumers, CXX public
signatures, and retained executor boundaries remain unchanged. No
upstream-owned C++ file changes.

Tier 1 `rewrite-validate-fast`, the focused native scalar/cache owner test,
inventory and storage-boundary guards, and whitespace validation pass. Tier 2
is the complete 49-test Rust-enabled `rust_consensus_tests` suite. Tier 3 is the
`taraxad` build. Independent review approves lock consistency, cert-vote
write-before-publication and failure semantics, thin projections, retained
domain edge coverage, documentation, and the exact 28,281-line budget. The
residual non-blocking risk is no service-level storage fault injection or
concurrency stress test; structural ordering and lower-level rejection tests
cover those contracts.

The final bounded `CRW-12` manager-read contraction moves startup replay,
own-pillar-vote, finalized DAG-position, and PBFT-membership reads behind native
`PbftService`. Each task acquires the manager lock and reads its owned storage;
the bridge only converts owned results. Deleting the now-callerless
`BridgePbftService::manager_state` accessor leaves no manager-guard or direct
manager-storage access in production `rustaxa-bridge`; one private `#[cfg(test)]`
runtime replacement helper remains for boundary fixture construction. The native
application-root owner test covers missing-row behavior for all four tasks,
while existing native storage tests retain found/payload/lambda and error
propagation coverage and the bridge sentinel retains carrier projection. This removes a net
7 production bridge lines and lowers the checked budget to 28,274. CXX
functions, carriers, handles, shim lines/directories, flags, factories,
constructors, production consumers, and CXX public signatures remain unchanged.
No upstream-owned C++ file changes.

Tier 1 `rewrite-validate-fast`, the focused native missing-read owner test, the
retained bridge storage-projection sentinel, exact inventory and storage
boundary guards, and whitespace validation pass. Tier 2 is the complete
49-test Rust-enabled `rust_consensus_tests` suite. Tier 3 is the `taraxad`
build. Independent review approves storage-helper parity, lock ownership,
missing/error propagation, projection, accessor deletion, test coverage, and
inventory. The retained test-only runtime replacement helper is not compiled
into production. Residual non-blocking risk is that `PbftService::manager_state`
remains a public native API, so guards are absent from current bridge code but
not type-system-inaccessible to a future adapter.

The next bounded `CRW-12` slashing-owner contraction moves double-vote proof
planning and submission reporting behind task-shaped native `PbftService`
methods. The bridge and native slashing sibling accessors are deleted, so the
bridge can no longer reach the planner mutex or duplicate cache. Three duplicate
bridge behavioral fixtures for normal planning, Magnolia activation, and
accepted/rejected submission lifecycle move to one native application-root
test plus the existing exhaustive native slashing suite. The bridge retains
only stable status-code and canonical proof-hash/ABI-byte conversion fixtures.
C++ still owns FinalChain account fact collection, transaction construction,
signing, gas-price lookup, and transaction-pool insertion as named leaf effects.
This removes a net 100 production bridge lines and lowers the checked budget to
28,174. CXX functions, carriers, handles, shim lines/directories, flags,
factories, constructors, production consumers, and public signatures remain
unchanged. No upstream-owned C++ file changes.

Tier 1 `rewrite-validate-fast`, the focused native slashing-owner lifecycle
test, retained bridge status/ABI fixtures, exact inventory and storage-boundary
guards, and whitespace validation pass. Tier 2 is the complete 49-test
Rust-enabled `rust_consensus_tests` suite. Tier 3 is the `taraxad` build.
Independent review approves native delegation and mutex ownership, Magnolia
configuration propagation, retry/accept/duplicate behavior, accessor deletion,
ABI coverage, and the exact 28,174-line inventory. Residual non-blocking risk is
that the application-root test does not stress concurrent submissions; the
unchanged native mutex owner and lower-level clone/cache tests cover the shared
lock contract.

The next bounded `CRW-12` pillar-root contraction moves every live pillar chain
and pillar-vote operation behind task-shaped native `PbftService` methods. The
bridge pillar sibling accessor is deleted and the native accessor becomes
crate-private, so `rustaxa-bridge` cannot borrow pillar state, its mutex,
preparation registries, or generation counters. FinalChain-composed block
creation now owns generation sampling, the unlocked validator query, and
generation-bound planning in the native root. Current-anchor tag conversion
retains its readiness/lock-error precedence through an explicit root preflight.
Three FinalChain-composed bridge behavioral fixtures for zero-weight admission,
accepted weighted admission, and missing-total versus zero-weight bundles move
to native application-root coverage; the bridge retains stateless vote
inspection, CXX carrier/error mapping, readiness precedence, FinalChain handle
unwrapping, and the separately classified pillar storage compatibility facade.
Test-only unplanned current-data, pure-threshold, and fact-injected block
creation root/bridge APIs are deleted. This removes 341 net bridge
lines and lowers the checked budget to 27,833. CXX functions, carriers, handles,
shim lines/directories, flags, factories, constructors, production consumers,
and public CXX signatures remain unchanged. No upstream-owned C++ file changes.

Validation uses the Tier 2 subsystem path plus the Rust-enabled node smoke gate:
`make rewrite-validate-fast`, 114 focused native pillar tests, 20 bridge pillar
tests, all 49 `rust_consensus_tests`, the DAG and PBFT-chain C++ suites, both
structural guards and their self-tests, whitespace validation, and the
`taraxad` smoke build pass. The aggregate legacy manager/pillar runner remains
non-isolated: its first manager case and the affected pillar cases pass in fresh
processes, then live node/database resources under `/tmp/taraxa0` survive into
later cases and cause same-process port or RocksDB-lock failures. No behavioral
assertion failed before that resource leakage. Independent review required an
exact-preparation cleanup assertion, corrected finalization-token error docs,
accurate inventory wording, and removal of a dead conversion; all were applied.

The next bounded `CRW-12` DAG/FinalChain composition slice moves the complete
proposer head/authorization lookup and verifier authorization lookup behind
task-shaped native `DagTransactionService` methods. Each task snapshots its
exact cursor, releases every DAG, sortition, and transaction guard before
borrowing native `FinalChain`, and reacquires the canonical lock domains only
to validate and advance the matching cursor. Query, decode, or recovery errors
clean only the still-matching cursor; stale replacements and sortition-parameter
drift survive without advancement. The bridge now only unwraps the retained
`BridgeFinalChain` handle and converts the owned native step. Its split-protocol
request/fact adapters, test guard escape hatches, FinalChain helper, and fifteen
RocksDB-backed behavioral fixtures are deleted. Five native tests cover
successful and rejected authorization, missing/future/malformed blocks, stale
replacement, live transaction pressure, sortition drift, and matching-only
failure cleanup plus historical-parameter VDF success, malformed-proof
rejection, and wrong-stage preservation; existing native packing coverage and
the 49-test CXX suite retain the other production and ABI contracts. This removes a net 1,361 bridge
lines and lowers the exact budget to 26,472. CXX functions, carriers, handles,
shim lines/directories, flags, factories, constructors, production consumers,
and public CXX signatures are unchanged. No upstream-owned C++ file changes.

Validation passes `make rewrite-validate-fast`, the 33 native DAG application
tests (including all four new FinalChain-composed cases), all 132 retained bridge
tests, the 49-test CXX consensus suite, `dag_test`, `dag_block_test`, isolated
`pbft_chain_test`, `pbft_chain_shim_test`, both structural guards and self-tests,
whitespace checks, and the Rust-enabled `taraxad` smoke gate. The repo-wide CTest
attempt remains non-isolated and partially unbuilt: registered missing binaries,
same-process `/tmp/taraxa0` RocksDB-lock leakage, and unavailable static Go zlib/
snappy libraries prevent a clean aggregate result. Python integration cannot
start because this image lacks `virtualenv` and `pytest` under its externally
managed Python installation.
Independent review found and verified the restored sidecar-pressure, malformed
sortition, and VDF application-root coverage plus crate-private verifier snapshot
visibility, then approved the final lock order, cursor revalidation, cleanup,
bridge contraction, and documentation shape.

The next bounded `CRW-12` PBFT/FinalChain composition slice moves canonical
vote validation, transactional admission, and cache-first `2t+1` threshold
resolution behind task-shaped native `PbftService` methods. Native consensus
now owns the ordered voter-stake, cached/fallback VRF-key, and total-stake
lookups; releases the verified-vote mutex around every FinalChain read;
publishes replay state only after the terminal validation decision; commits
required vote-progress storage before publishing admission; and refreshes the
PBFT-chain head before threshold cache publication. The three CXX operations
and their carriers are unchanged, but `rustaxa-bridge` now performs only
request/result conversion and unwraps the retained FinalChain leaf handle. The
seven FinalChain-composed bridge behavior tests move beside the native
application root, covering preverified and zero-stake admission, replay
idempotence, early zero-stake rejection, ready/future threshold state, and a
cache hit that needs no FinalChain state. This removes 455 bridge lines and
lowers the exact budget to 26,017. CXX functions, carriers, handles, shim
lines/directories, flags, factories, constructors, production consumers, and
public CXX signatures are unchanged. No upstream-owned C++ file changes.

The following bounded `CRW-12` PBFT/FinalChain composition slice moves the
complete weighted-vote generation, proposer-sortition, and four DPoS fact
collection tasks behind native `PbftService` methods. Rust now owns identity
and VRF validation precedence, ordered voter/total stake reads, zero-stake
short-circuiting, typed availability outcomes with stable future/infrastructure
error codes, diagnostic typed FinalChain heads, duplicate-preserving wrapping wallet aggregation, and
ordered batch status promotion. Native facts no longer reproduce CXX
status/presence/zero-sentinel combinations; the bridge derives those legacy
fields from typed ready/unavailable outcomes. Twenty composed behavior tests
move to the native owner, while the bridge retains standalone signed-vote and
unknown-vote-type ABI sentinels plus two focused typed-result conversion tests. All six CXX
operations, carriers, callers, and signatures remain unchanged, but the bridge
module loses 506 lines and the exact `bridge_lines` budget falls to 25,511.
CXX functions, carriers, handles, shim lines/directories, flags, factories,
constructors, and production consumers are unchanged. No upstream-owned C++
file changes.

The next bounded `CRW-12` PBFT ownership slice moves the remaining fifteen
production verified-vote access, storage, and egress operations behind
task-shaped native `PbftService` methods. Native consensus now owns replay
queries/mutation, next-round and `2t+1` lookup, coherent state/step snapshots,
single-epoch next/next-null bundle planning, optimized bundle validation and
encoding, cleanup, validated own-vote persistence/lifecycle, and serialized
vote-progress persistence. Domain APIs use typed bundle families and statuses
plus `Option` for missing mappings; only the bridge projects raw CXX kinds,
numeric statuses, diagnostic strings, and zero sentinels. The bridge-local
`VerifiedVotesAccess`, storage/lock reach-through, forwarding macros, private
sibling accessor, low-level mutation helpers, and behavioral fixtures are
deleted. Three comprehensive native service tests cover query/bundle/cleanup,
own-vote validation/lifecycle, and progress-persistence mutex serialization;
six compact bridge tests retain ordered carrier, exhaustive status, invalid
build-kind, invalid persistence-kind, leader/reward status, and executor-intent
projections. This removes a net 1,588 bridge lines and lowers the exact
`bridge_lines` budget to 23,923. CXX
functions, carriers, handles, shim lines/directories, flags, factories,
constructors, production consumers, public CXX signatures, and C++ callers are
unchanged. No upstream-owned C++ file changes.

Validation for this verified-vote ownership slice passed the full
`rustaxa-consensus` (1,071 tests) and `rustaxa-bridge` (101 tests) package
suites, including all three new native service tests and all six focused
bridge projections. `rewrite-validate-fast`, `rewrite-validate-consensus`,
`rewrite-validate-storage`, `rewrite-validate-smoke`, the bridge inventory and
storage boundary guards, the 49-case `rust_consensus_tests` binary, and the
9-case `rust_storage_tests` binary passed. The affected `vote_test` cases pass
when each runs in a fresh process; the aggregate binary and full CTest run
retain the existing same-process `/tmp/taraxa0/db/db/LOCK` leak. The full CTest
gate reported 9 of 21 tests passing, with its remaining failures also including
unbuilt test executables and the Go test's unavailable static zlib/snappy
linker inputs. Python integration could not collect because this image rejects
system package installation under PEP 668 and has neither `virtualenv` nor
`pytest`. Independent review approved the ownership boundary after the native
missing-plan result was corrected to preserve the requested period and round
through the stable CXX projection.

The next bounded `CRW-12` PBFT root-closure slice moves all seven PBFT-chain
and four production proposed-block operations behind task-shaped native
`PbftService` methods. The bridge can no longer obtain chain or proposal
siblings, and the native sibling accessors are crate-private. The remaining
test-only manager replacement hook is deleted as well, making the native
manager accessor crate-private and leaving `BridgePbftService` with no state,
lock, storage, sibling, or mutation escape hatch. CXX declarations, carriers,
signatures, and C++ callers are unchanged; chain status/sentinel projection and
proposal DTO projection remain bridge-only. Two focused native root tests cover
the eleven routes, including typed validation mismatches, missing storage
lookups, durable proposal publication, duplicate behavior, validity mutation,
and deterministic snapshots. Existing native sibling suites continue to own
failure and restart behavior, while the bridge retains only boundary and the
separately classified stateless storage/temporary-candidate fixtures. This
removes a net 48 bridge lines and lowers the exact `bridge_lines` budget to
23,875. CXX functions, carriers, handles, shim lines/directories, flags,
factories, constructors, and production consumers are unchanged. No
upstream-owned C++ file changes.

Validation for this root-closure slice passed the full native consensus (1,073
tests) and bridge (101 tests) suites, `rewrite-validate-fast`, both structural
guards, the 49-case `rust_consensus_tests` binary, the two-case
`pbft_chain_shim_test`, the storage-backed `PbftChainTest.pbft_db_test` in a
fresh process, the focused single-node PBFT manager case, and
`rewrite-validate-smoke` with `RUSTAXA_ENABLE=ON`. The aggregate consensus gate
continues to expose the existing same-process `/tmp/taraxa0/db/db/LOCK` leak in
multi-case binaries; an overlapping PBFT-manager run also encountered its
existing fixed-port conflict. Focused affected cases pass when isolated.
Independent review approved the native ownership, ABI stability, error and
sentinel projection, documentation, and exact inventory contraction.

The next bounded `CRW-12` DAG/PBFT composition slice closes the last raw
`DagTransactionService` escape from `rustaxa-bridge`. The two PBFT
finalization routes now enter through operation-specific, crate-private
delegates on `BridgeDagTransactionService`, which compose the retained CXX
two-handle contract without returning the native root, a lock, or a guard to
the PBFT bridge. Finalization cursor validation, action dispatch, lock order,
terminal cleanup, and error propagation remain unchanged inside native
`PbftService`; CXX declarations, carriers, signatures, and callers are
unchanged. The bridge-local DAG worker-planner behavioral fixture moves to the
native DAG owner, while a focused bridge fixture retains FFI hash conversion
and byte-order coverage for the exported VDF adapter. This removes a net 8
bridge lines and lowers the exact `bridge_lines` budget to 23,867. CXX functions,
carriers, handles, shim lines/directories, flags, factories, constructors, and
production consumers are unchanged. No upstream-owned C++ file changes.

Validation for this DAG/PBFT root-closure slice passed the full native
consensus (1,074 tests) and bridge (100 tests) suites,
`rewrite-validate-fast`, both structural guards, nine focused CXX PBFT
finalization start/advance/resume cases, and `rewrite-validate-smoke` with
`RUSTAXA_ENABLE=ON`. A strict ad hoc `clippy -D warnings` remains blocked by
pre-existing workspace warnings outside this slice; the repository clippy gate
passes and the new multi-argument boundary delegate carries a focused lint
allowance matching the retained CXX operation shape. Independent review
approved the ownership closure after focused FFI VDF conversion coverage was
retained at the bridge boundary.

The next bounded `CRW-12` PBFT sibling test-ownership slice removes the
remaining pillar-chain protocol/runtime suite from `rustaxa-bridge`. Native
`PillarChainService` and root `PbftService` tests now exclusively own pillar
restoration, readiness, bootstrap/restart, anchor generation and atomic
mutation, linkage, block creation, and composed FinalChain behavior. Three
missing negative cases move beside the native owner: malformed persisted
current data, malformed latest-finalized data, and malformed current-data
apply with unchanged durable bytes and in-memory generation/snapshot. The
bridge deletes its ready-service fixture and twelve RocksDB-backed behavioral
tests, retaining only three focused sentinels for current-anchor tag/status and
readiness-error projection, typed pillar-storage byte/missing/error behavior,
and FinalChain-handle plus block-plan carrier conversion. This removes 434
bridge lines and lowers the exact `bridge_lines` budget to 23,433. CXX
functions, carriers, handles, shim lines/directories, flags, factories,
constructors, production consumers, declarations, and C++ callers are
unchanged. No upstream-owned C++ file changes.

Validation for this pillar-chain test-ownership slice passed the full native
consensus (1,077 tests) and bridge (88 tests) suites,
`rewrite-validate-fast`, the bridge inventory guard and its self-test, the
storage-boundary guard, `git diff --check`, and `rewrite-validate-smoke` with
`RUSTAXA_ENABLE=ON`. The focused `pillar_chain_test` binary passed 10 of 13
cases in one process; two cases that encountered the suite's shared
`/tmp/taraxa0/db/db/LOCK` leak passed when rerun independently. The remaining
`finalize_root_in_pillar_block` case still requires committed external-EVM
state for block 3 and fails independently before reaching this Rust-test-only
change. No `Old::`, retired consensus-network-queue, or native
`BridgeStorage` references were found in the closeout searches. Independent
review approved the contraction after the retained storage sentinel proved
cloned-handle lifetime and exact error identities and the FinalChain sentinel
covered every material block-plan carrier field.

The following bounded `CRW-12` PBFT-manager test-ownership slice removes the
last storage-backed manager protocol fixtures from `rustaxa-bridge`. Native
`PbftService` and `PbftManagerRuntime` tests provide the authoritative coverage
for bootstrap gating, runtime/proposal/sync cursor activation, lifecycle
rejection without mutation,
startup replay ranges, reset-authenticated period advance, and owned
finalization draining. The bridge deletes its temporary-directory, storage,
startup-service, seeded-finalization fixtures, and the now-callerless
test-only manager runtime factory/configuration; its retained tests now
construct only plain domain/FFI values and cover bootstrap fallback DTOs,
startup/advance status codes, lifecycle request/result conversion, manager
session carriers, storage-read carriers, and external-finalization effect
projection. Lifecycle request/result conversion is centralized in private
helpers used by the unchanged production entrypoint. This removes 130 bridge
lines and lowers the exact `bridge_lines` budget to 23,303. Production callers,
CXX functions, carriers, handles, shim lines/directories, flags, factories,
constructors, declarations, signatures, and C++ callers are unchanged. No
upstream-owned C++ file changes.

Validation for this PBFT-manager bridge test-ownership slice passed the nine
focused manager adapter tests, the period-data-queue adapter sentinel, the full
native consensus (1,077 tests) and bridge (88 tests) suites, three focused CXX
startup/period-advance/finalization-tail transcripts,
`rewrite-validate-fast`, the bridge inventory guard and self-test, the
storage-boundary guard, `git diff --check`, and `rewrite-validate-smoke` with
`RUSTAXA_ENABLE=ON`. The aggregate consensus target again reached the known
`pillar_chain_test` same-process database-lock leak: 10 of 13 cases passed,
while `votes_count_changes`, `pillar_chain_syncing`, and
`finalize_root_in_pillar_block` could not reacquire `/tmp/taraxa0/db/db/LOCK`.
The focused PBFT tests required by this slice passed independently. Closeout
searches found no `Old::`, retired consensus-network-queue, native
`BridgeStorage`, or deleted PBFT test-factory references. Independent review
approved after the lifecycle sentinel covered the rejected path and reachable
reset, certify, and finish-polling effect combinations without relying on a
protocol runtime.

The next bounded `CRW-12` PBFT-sync test-ownership slice removes the complete
RocksDB-backed admission/egress protocol fixture from `rustaxa-bridge`. Native
`PbftService` and PBFT-sync tests remain authoritative for bootstrap gating,
cursor/check ordering, mismatched reports, terminal cleanup, abort, period-data
loads, and reward-vote attachment. The bridge retains only pure all-field
sentinels for admission input, transaction reports, not-started status/error
fallback, egress projection, and cert-vote validation.
Production signatures, routing, CXX functions, carriers, handles, callers,
shims, flags, factories, and constructors are unchanged. The contraction
removes ten net bridge lines and lowers the exact `bridge_lines` budget to
23,293; no upstream-owned C++ file changes.

Validation passed the five focused PBFT-sync bridge sentinels, the authoritative
native ownership case, the full native consensus (1,077 tests) and bridge (91
tests) suites, `rewrite-validate-fast`, the bridge inventory guard and its
self-test, the storage-boundary guard and its self-test, `git diff --check`, and
`rewrite-validate-smoke` with `RUSTAXA_ENABLE=ON`. Closeout searches found no
deleted fixture helpers, `Old::`, retired consensus-network-queue, or native
`BridgeStorage` references in the PBFT-sync bridge module. Because production
routing and CXX declarations are unchanged, the existing native behavior and
CXX external-finalization suites remain the correctly scoped boundary coverage.

The next bounded `CRW-12` compatibility contraction deletes the complete
temporary proposed-block candidate bridge used by local PBFT leader selection.
`VoteManager` now consumes its already paired caller-owned block/vote objects
directly, rejects period/hash mismatches before validation, and passes only
typed candidate facts to the existing native Rust status/ranking planner. The
path no longer serializes live blocks into a temporary Rust candidate map,
performs an identity lookup, or rematerializes the same block RLP into C++.
`proposed_blocks_local_candidate_lookups`, its `ProposedBlockIdentity` carrier,
the bridge-only helpers, and the duplicated storage/service isolation fixture
are deleted. Native proposed-block and leader-planner suites retain behavioral
ownership; the bridge keeps only its two named storage compatibility functions
and focused projection sentinel. This removes 136 bridge lines, 21 shim lines,
one CXX function, and one CXX carrier, lowering the exact budgets to 23,157,
17,080, 388, and 339. Handles, shim directories, flags, partial factories,
compatibility constructors, and production bridge consumers are unchanged.

Validation passed the full native consensus (1,077 tests) and bridge (90
tests) suites, `rewrite-validate-fast`, bridge inventory and storage-boundary
guards with self-tests, `rewrite-validate-smoke` with `RUSTAXA_ENABLE=ON`, the
two CXX proposed-block cases, both focused Rust-authoritative leader-selection
cases when run in fresh processes, PBFT proposal/broadcast integration, and
`FullNodeTest.multiple_wallets_support`. The aggregate consensus gate again
reached the known `pillar_chain_test` same-process `/tmp/taraxa0/db/db/LOCK`
leak: 10 of 13 cases passed and the same three node-constructing cases could
not reacquire the shared database. The changed local-candidate path passed its
focused and full-node coverage. Closeout searches found no deleted export or
carrier references, retired consensus-network-queue, native `BridgeStorage`,
or legacy `Old::` calls in the touched shim. Independent review approved the
identity-preserving direct-pair route and confirmed the touched C++ files are
main-only overlay files with no upstream-owned intersection.

The next `CRW-12` contraction removes the standalone PBFT vote-payload codec
bridge. Own-vote persistence now passes canonical signed vote bytes plus the
authoritative weight to the existing PBFT service, which builds and validates
the weighted storage record natively. Direct vote-progress persistence passes
only an extra-reward vote hash and exact 2t+1 mapping coordinates; under the
vote-runtime lock Rust resolves retained weighted payloads, fail-closes missing
or mismatched state, builds the canonical ordered bundle, and commits the
existing atomic storage write. Admission returns its native weighted bytes for
the temporary live sidecar, so the shim no longer calls a free codec helper.
The complete `pbft_vote_payload` bridge module, two CXX functions, one nested
bundle carrier, and the C++ weighted-record/bundle materializers are deleted.
This removes 137 bridge lines and 36 shim lines, lowering the exact budgets to
23,020 bridge lines, 17,044 shim lines, 386 CXX functions, and 338 carriers.
Handles, shim directories, flags, partial factories, compatibility
constructors, and production bridge consumers are unchanged.

Validation passed the full native consensus (1,078 tests) and bridge (86
tests) suites, all nine Rust storage bridge tests, `rewrite-validate-fast`,
bridge inventory and storage-boundary guards with self-tests,
`rewrite-validate-smoke` with `RUSTAXA_ENABLE=ON`, focused native success and
identity-mismatch/no-write persistence cases, own-vote persistence/reload,
missing-weight admission, and 2t+1 vote progression. Node-constructing vote
tests passed in fresh processes. The aggregate consensus gate again reached
the known `pillar_chain_test` same-process `/tmp/taraxa0/db/db/LOCK` leak: 10
of 13 cases passed and the same three later node-constructing cases could not
reacquire the shared database. The changed paths pass their focused coverage;
no deleted export, carrier, or module references remain, and independent
review approved the native identity-resolution and atomic storage route.

The following `CRW-12` vote-validation contraction deletes the standalone
canonical PBFT vote-inspection bridge. `VoteManager` validation and admission
now consume the complete canonical identity, signature status, replay outcome,
and weighted payload returned by the composed native verified-vote service;
local generation parity retains exact canonical byte and decoded-field checks
without reinspecting Rust-generated bytes through a second CXX call. The
complete `pbft_vote_validation` bridge module, its CXX export, its inspection
carrier, the now-callerless manual replay-insert export, and two now-unread signing-hash carrier fields are deleted, while the remaining validation and threshold
conversion lives with the named verified-vote boundary. Native application-root
tests replace the bridge inspection fixture and prove malformed RLP never marks
replay state while an invalid signature marks it exactly once. This removes 103
bridge lines and 25 shim lines, lowering the exact budgets to 22,917 bridge
lines, 17,019 shim lines, 384 CXX functions, and 337 carriers. Handles, shim
directories, flags, partial factories, compatibility constructors, and
production bridge consumers are unchanged. The retained boundary is the
composed verified-vote CXX adapter used by the Rust-mode `VoteManager`; C++
continues only live vote materialization, FinalChain fact access, logging, and
executor effects.

Validation passed both focused native malformed/invalid-signature replay tests,
the complete native consensus (1,080 tests) and bridge (85 tests) suites,
`rewrite-validate-fast`, `rewrite-validate-smoke`, the four focused Rust-mode
`vote_test` validation/admission/generation/persistence cases, all 49
`rust_consensus_tests`, targeted C++ formatting, inventory and storage-boundary
guards with self-tests, and whitespace validation. The aggregate consensus gate
again reached the known `pillar_chain_test` same-process
`/tmp/taraxa0/db/db/LOCK` leak: 10 of 13 cases passed and the same three later
node-constructing cases could not reacquire the shared database. No original
upstream-owned C++ file changed; the only C++ edit is in the main-only
VoteManager overlay. Independent review approved replay parity, admission
identity checks, exact-RLP generation parity, FFI completeness, metrics, and
coverage with no blocking or medium findings.

The next bounded `CRW-N01`/`CRW-12` network contraction deletes the parallel
side-effect-free PBFT vote-ingress planning API after its production callers
had already moved to the composed Rust ingress/effect pipeline. Tarcap now has
only `consensus_network_ingest_pbft_vote` and
`consensus_network_ingest_pbft_vote_bundle_member`; those methods invoke the
same native planners and own typed sync/report/disconnect effect queueing. The
two direct CXX exports, `PbftVoteIngressPlan` carrier, C++ wrapper methods,
standalone bridge module, bridge projection/error mapping, and duplicated
direct planner tests are deleted. Native planner tests retain protocol behavior,
while composed CXX tests cover accepted current votes with no effects,
future-period sync identity, and ordered bundle report/disconnect effects. The
retained named boundary is `BridgeConsensusNetworkApi` for latest-tarcap
transport execution; C++ still supplies decoded vote/context facts and executes
physical peer/network effects as explicit `CRW-N01` debt.

As ancillary storage narrowing in the same larger slice, proposed-block
compatibility save/read calls stop accepting the broad `BridgeStorage` handle
through free functions. The unchanged `DbStorage` client uses two methods on
its existing typed `BridgePbftStorageQueries` handle, which still validates
canonical block identity through the native proposed-block codec/storage
helpers. This relocates temporary compatibility plumbing and is not claimed as
standalone ownership completion; the methods retire with the storage facade.
Together the changes remove 274 bridge lines, two CXX functions, and one CXX
carrier, lowering the exact budgets to 22,643 bridge lines, 17,019 shim lines,
382 functions, and 336 carriers. Handles, shim directories, flags, partial
factories, compatibility constructors, and 39 production bridge consumers are
unchanged.

Validation passed the focused native proposed-block and composed vote-ingress
tests, the complete native consensus (1,080 tests) and bridge (81 tests)
suites, `rewrite-validate-fast`, `rewrite-validate-smoke`, all 9
`rust_storage_tests`, all 48 `rust_consensus_tests`, and the focused
`PbftManagerTest.propose_block_and_vote_broadcast` three-node case. Targeted
C++ formatting, inventory and whitespace guards also pass. The Rust-enabled
build cache is confirmed with `RUSTAXA_ENABLE=ON`. The only original
upstream-owned C++ changes in this slice delete the two guarded, Rust-only
wrapper pairs; they add no feature-on dependency and do not alter the pure-C++
route. The branch intersection helper cannot compare this feature branch to a
local `main` ref because that ref is absent, so the original-file audit was
performed directly against the pre-slice tree and `upstream-main`. Independent
review approved status/error parity, effect identity and ordering,
proposed-block canonical validation/restoration, inventory accuracy, and test
coverage with no blocking or medium findings.

The aggregate consensus gate passed `rust_consensus_tests`, `dag_test`,
`dag_block_test`, `pbft_chain_test`, and `pbft_chain_shim_test`, then reproduced
the existing same-process `/tmp/taraxa0/db/db/LOCK` leak across later
node-constructing cases: `pbft_manager_test` passed 1 of 14, `vote_test` passed
4 of 20, and `pillar_chain_test` passed 10 of 13. Each failure reports the same
Rust storage lock already held by the current process. The focused three-node
PBFT case passed in a fresh process before this aggregate run, so the failure is
recorded as harness cleanup debt rather than a slice regression.

The next bounded `CRW-12`/`CRW-07` lifetime contraction deletes the complete
`BridgePillarChainStorage` handle and owned-handle factory family. Its sole
production client, the Rust-mode `DbStorage` overlay, now invokes the same seven
pillar compatibility operations through its already-owned `BridgeStorage`
root. Native pillar services remain PBFT-owned protocol authority; this slice
does not broaden or complete blocked `CRW-17`. Stable C++ `DbStorage` block,
own-vote, and current-block-data APIs retain identical canonical RLP, missing
read, and error behavior while one cloned storage lifetime disappears. Native
pillar storage tests retain persistence, missing-read, empty-input, and
restart coverage; the focused bridge storage sentinel retains byte/error
projection coverage. The change removes 29 bridge lines, 2 shim lines, one CXX
function, one opaque handle, and one owned-handle factory, lowering exact
budgets to 22,614 bridge lines, 17,017 shim lines, 381 CXX functions, and 19
handles. Carriers remain 336; shim directories, flags, partial factories,
compatibility constructors, and 39 production bridge consumers are unchanged.

Validation passed the three focused native pillar-storage tests, the focused
bridge storage and public-query sentinels, all 9 `rust_storage_tests`,
`rewrite-validate-fast` (1,080 native consensus and 81 bridge tests),
`rewrite-validate-smoke`, targeted C++ formatting, the inventory/storage
guards, and whitespace validation with `RUSTAXA_ENABLE=ON`. The complete
`pillar_chain_test` again passed 10 of 13 cases and reproduced the known
same-process `/tmp/taraxa0/db/db/LOCK` leak in the three later node-constructing
cases; its direct `pillar_chain_db` compatibility round trip passed before the
leak. Both changed C++ files are main-only storage overlay paths, so no original
upstream-owned C++ file or pure-C++ source selection changed. Independent
review approved the seven-operation semantic and lifetime parity, CXX
completeness, metrics, tests, and upstream audit with no blocking or medium
findings.

The adjacent `CRW-12`/`CRW-07` storage-lifetime contraction deletes the
complete `BridgeMetadataStorageQueries` handle and owned-handle factory family.
Its sole production client, the storage overlay, and its storage-conformance
client now use the already-owned `BridgeStorage` root for the same seven
immutable genesis, sortition, status, lambda, and rewards-stat projections.
Rust bridge fixtures likewise retain the root storage owner instead of testing
a detached cloned lifetime. Missing values, closest-period selection,
u64-to-usize saturation, ordering, canonical bytes, and storage error identity
remain unchanged. This is another isolated lifetime deletion alongside active
`CRW-12`/`CRW-07`, not completion of blocked `CRW-17`. The change removes 39
bridge lines, 4 shim lines, one CXX function, one opaque handle, and one
owned-handle factory, lowering exact budgets to 22,575 bridge lines, 17,013
shim lines, 380 CXX functions, and 18 handles. Carriers remain 336; shim
directories, flags, partial factories, compatibility constructors, and 39
production bridge consumers are unchanged.

Validation passed the focused genesis/rewards metadata bridge tests, the
FinalChain rewards-publication fixture that reads metadata through the root,
all 9 `rust_storage_tests`, the full C++-reference-versus-Rust storage
conformance differential, `rewrite-validate-fast` (1,080 native consensus and
81 bridge tests), `rewrite-validate-smoke`, the inventory/storage guards,
targeted C++ formatting, and whitespace validation with `RUSTAXA_ENABLE=ON`.
This read-only family owns no
write batch or restart sidecar; the differential proves the stable metadata
transcript is byte-identical after the receiver-lifetime deletion. The storage
overlay and conformance runner are main-only paths absent from `upstream-main`,
so no original upstream-owned C++ file or pure-C++ source selection changed.
Independent review approved semantic and lifetime parity, CXX completeness,
inventory accuracy, conformance coverage, and the upstream ownership audit with
no blocking or medium findings.

The first `CRW-10` closeout slice makes the bridge inventory mechanically complete before further deletion. The guard
now compares all declared Rust bridge modules and all live consensus shim directories against their dedicated audit
tables in addition to exported `Bridge*` handles, rejects missing and stale rows, and self-tests every inventory family.
The grouped PBFT-vote module row is expanded into exact module classifications, previously omitted internal modules are
documented, and the already-retired `pillar_votes_shim` row is removed from the live inventory. Runtime behavior and
accepted network, EVM, lifecycle, signing/VDF, storage-compatibility, and public-materialization boundaries are unchanged.

The second `CRW-10` closeout slice deletes the obsolete trailing **Current Open Items** table. That table duplicated the
authoritative queue, described completed CRW-02 through CRW-09 work as partial, named retired handles and generic fact
carriers, and mixed the scope-gated network/EVM follow-ups with active consensus work. The queue above is now the only
status/dependency authority. The later aggressive-cutover policy activates `CRW-N01` and `CRW-E01`; reusable validation
guidance remains in the Validation Matrix.

The third `CRW-10` closeout slice removes the no-caller CXX
`pbft_service_verified_votes_weighted_payload` export, its export-specific `PbftVotePayloadLookup` carrier, and the
bridge helper used only by Rust tests. The lower-level Rust runtime still owns weighted-payload retention and lookup for
production aggregate selection and snapshot behavior. This is a `CRW-07` export/carrier contraction only: no handle,
shim, module flag, runtime behavior, or accepted compatibility boundary changes.

The fourth `CRW-10` closeout slice removes the no-caller CXX rewards preview/commit pair
`preview_finalized_period_rewards_stats` and `rewards_stats_runtime_commit_process_result`, their public bridge
wrappers, the now-callerless native `RewardsStatsRuntime::apply_process_plan` helper, and the bridge-only test that
exercised that retired protocol. Production FinalChain instead generation/head-checks and installs the fully processed
cloned runtime, while the C++ shim continues to use direct processing, staged storage writes, cache views, and committed
clears. This is a `CRW-07` export contraction only: no carrier, handle, shim, module flag, runtime routing, or accepted
compatibility boundary changes.

The fifth `CRW-10` closeout slice removes the test-only CXX
`pbft_service_pillar_apply_current_block_data` export. Its two C++ fixtures now install their initial anchor through the
same generation-checked `pbft_service_pillar_apply_planned_current_block_data` contract used by production, with the
fresh runtime's expected generation `0`. The unchecked Rust state operation is deleted; a crate-test-only setup helper
samples the current generation and delegates through the checked operation, while production Rust exposes only the
generation-checked apply contract. This is a `CRW-07` export/test contraction only: no carrier, handle, shim, module
flag, production route, or accepted compatibility boundary changes.

The final `CRW-10` audit finds no actionable unclassified consensus ownership or compatibility-deletion item. A fresh
complete CXX caller census leaves only `storage_shim_seed_final_chain_conformance_lookup_rows` without a production
caller; its storage-conformance-only use remains explicitly classified and guard-confined. Closeout searches find no
legacy `*Old::` consensus-shim calls, queue-named network exports, or `BridgeStorage` use in `rustaxa-consensus`; the
remaining query and bridge-batch hits are the documented network/public-query, storage-shim, rewards-staging, and test
boundaries. Inventory and storage-boundary guards, focused Rust/C++ tests, Tier 1, consensus Tier 2, pre-commit, and
independent review pass. Every path changed by the CRW-10 commit range is absent from `upstream-main`, so there is no
applicable upstream-owned C++ intersection to synchronize to `cpp-reference`. `CRW-N01` and `CRW-E01` remain explicitly
scope-gated follow-ups and do not block this closeout.

The VoteManager threshold sub-slice of `CRW-09I` is routed: `getPbftTwoTPlusOne` no longer collects or interprets generic
FinalChain DPoS facts in C++. The PBFT service owns cache-first threshold composition, reads its sibling Rust PBFT-chain
size without a C++ relay, borrows Rust FinalChain only for an exact-period total on cache miss, and returns the existing
operation-specific threshold plan. At that intermediate point, CRW-09I remained active because local sortition and
PBFT-manager validation/eligibility still consumed generic `PbftFinalChainFact*` carriers. The validation
sub-slice is now composed as well: `validateVote` supplies canonical vote bytes and immutable committee configuration
to `BridgePbftService`, which performs voter DPoS lookup, address-keyed VRF-key cache/fallback lookup, proof validation,
total DPoS lookup, weight calculation, replay publication, and weighted-payload construction through borrowed Rust
FinalChain state. C++ only verifies and hydrates the temporary live `PbftVote` sidecar from the returned canonical
weighted payload. The dead
non-composed threshold CXX export and C++ facade are removed alongside this routing as a `CRW-07` contraction.
The remaining operation-specific request/result carriers are narrowed to configuration input and the four
VoteManager-consumed result fields; PBFT-chain, DPoS, sortition, cache, and two-pass control state stays Rust-private.
Weighted local vote generation is composed as well: `generateVoteWithWeight` supplies only wallet/signing input and
committee configuration, while the PBFT service borrows Rust FinalChain, reads voter stake before total stake, and
returns the existing typed generation status plus canonical weighted RLP. The bridge-only `PbftVoteWeightFacts` DTO and
free weighted-generation export are deleted; the consensus-owned weight facts remain private implementation detail.
Local proposer sortition is composed as well: `genAndValidateVrfSortition` supplies only the PBFT period/round,
proposer count, and wallet identity material to one PBFT-service call. Rust validates both supplied identities before
borrowing FinalChain, reads voter stake before total stake, generates and verifies the canonical proposer VRF proof for
`[period, round, 1]`, and calculates the legacy-compatible weight without exposing proof, output, threshold, or DPoS
facts to C++. The obsolete `PbftProposerSortitionFact`/`PbftProposerSortitionPlan` CXX carriers and free planner export
are deleted. Vote admission is composed as well: C++ supplies canonical bytes plus a narrow admission-policy request,
and Rust inspects the vote, resolves voter stake and VRF key before total stake, validates, and transactionally publishes
the admission without holding the verified-vote mutex across FinalChain reads. Preverified weighted votes skip the
admission-validation voter-stake, VRF-key, and total-stake reads; the separately composed threshold lookup may still
read FinalChain on a cache miss. The old CXX external-facts carrier, low-level validation/admission exports, C++ KeyManager member,
and VoteManager generic-fact helpers are deleted. At that intermediate point, generic `PbftFinalChainFact*` carriers
remained live only for PBFT-manager consumers, so `CRW-09I` was not yet complete.

The PBFT-manager DPoS query/eligibility sub-slice is composed as well. Current total votes, the eligible local-wallet
vote sum, single-node participation, and batch wallet eligibility now enter operation-specific `BridgePbftService`
methods that borrow Rust FinalChain synchronously. C++ supplies only the period and wallet addresses and consumes the
operation result; ordered lookup, aggregation, zero-stake eligibility, future-state status, and error classification
stay behind the Rust boundary. The generic `PbftFinalChainFact*` request/result was then hash-only, and its DPoS flags,
total fields, address facts, address conversion helper, and mixed Rust collection logic were deleted. Three PBFT-manager
hash consumers remained (proposal hash lookup plus live and sync validation), so `CRW-09I` was not complete until that
family was composed and the generic carrier/export removed entirely.

The final PBFT-manager hash sub-slice completes `CRW-09I`. Proposal-session initialization now borrows Rust FinalChain
through `BridgePbftService` and keeps the required hash private to the Rust proposal fact. Live and sync admission use
one operation-specific tri-state hash-validation call that preserves valid, missing, and invalid behavior plus the
expected-hash diagnostic. The generic `PbftFinalChainFact*` request/result family, collector export, FinalChain shim
relay, and C++ conversion helper are deleted. The remaining CXX FinalChain surfaces are classified executor,
state-lifecycle, configuration, or public-materialization boundaries; that completed `CRW-09` and made `CRW-10` ready.

The current `CRW-09` slice replaces raw Rust account-balance bytes with a `U256` domain value plus one private encoding-
provenance bit. Untouched genesis accounts retain their fixed 32-byte lookup and snapshot representation, while new or
successfully mutated accounts retain canonical minimal bytes. Snapshot decode rejects oversized and short leading-zero
encodings before state installation. Existing CXX carriers remain byte vectors, and no handle, export, shim, module flag,
snapshot shape, migration, or `CRW-07` inventory change is introduced.

The next `CRW-09` slice introduces `FinalChainGas(u64)` across FinalChain transaction/call limits, native and external
execution results, cumulative-gas validation, rewards inputs, receipts, headers, publication, audit, and restart. Checked
domain arithmetic replaces raw cumulative addition, and gas-price fee calculation accepts only typed gas. CXX and public
query carriers remain `u64`; codecs and request identities explicitly unwrap the same value, preserving all existing
bytes and error ordering. No carrier, handle, export, shim, module flag, schema, or `CRW-07` inventory change is required.

The next `CRW-09` dependency slice introduces the shared `DposTokenAmount(U256)` semantic domain for the four
non-persisted DPoS policy values: eligibility threshold, vote step, validator maximum stake, and minimum deposit. The
bridge validates the retained byte-vector inputs once, and eligibility/max/minimum arithmetic consumes typed amounts.
Persisted principal, undelegation custody, corrections, rewards, supply, and arbitrary-width reward-per-stake remain
outside this slice pending their complete vertical migrations. No carrier, snapshot, handle, export, shim, module flag,
or `CRW-07` inventory delta is introduced.

Standalone prerequisite coverage now records the DPoS snapshot codec's byte-level contract before persisted principal
typing: valid mixed fixed/minimal/empty and arbitrary-width reward-index values round-trip exactly, and legacy schema 5/6
self-delegation synthesis preserves aggregate-stake bytes. A separately labeled characterization-only test records that
the legacy decoder still accepts short leading-zero and greater-than-U256 principal bytes. That acceptance is migration
debt, not a compatibility guarantee; the persisted DPoS amount slice must replace it with immediate stable rejection
before state installation. This test-only slice has no `CRW-07` delta.

`CRW-09D` now types aggregate stake, per-delegator principal, and V1/V2 undelegation custody with the shared
`DposTokenAmount` semantic value and a private consensus-owned `StoredDposTokenAmount` encoding-provenance wrapper. Snapshot
decode rejects oversized and short leading-zero values before state installation, untouched fixed/minimal encodings
round-trip byte-for-byte, arithmetic mutations canonicalize their persisted representation, and registration retains
its legacy fixed-width ABI-funded stake bytes. Current complete
ledgers reject orphan validators, principal-sum overflow, and aggregate mismatches while schema 5/6 rebuild state and
explicit historical same-validator corruption markers retain their established exceptions. ABI and snapshot shapes,
CXX carriers, bridge exports, shims, and module flags are unchanged, so `CRW-07` has no inventory delta. `CRW-09E` and
`CRW-09F` are now independently ready.

`CRW-09E` types ordered hardfork correction amounts as the shared `DposTokenAmount` while retaining byte-vector CXX
configuration carriers and converting once at bridge ingress. The exact-height transition now applies each configured
entry in legacy order against a candidate DPoS snapshot: it validates the principal pair, repairs the stale reward head,
subtracts only the historically inflated aggregate, and recomputes the validator vote count without changing principal,
global vote total, history state, or corruption markers. Duplicate and later failures discard the entire candidate, so
no partial aggregate, vote, or reward-graph mutation reaches publication. Oversized configuration amounts fail with an
indexed stable error before FinalChain construction; ordering and duplicates remain intact. No snapshot/CXX shape,
handle, export, shim, or module flag changed, so `CRW-07` has no inventory delta. `CRW-09F` remains ready.

`CRW-09F` keeps the reward-index semantic domain inside `rustaxa-consensus`: authoritative graph nodes and arithmetic
use arbitrary-width `DposRewardIndex`, while scalar snapshot accumulator/cursor mirrors use a separate private length-
provenance wrapper. Untouched padded and over-32-byte mirrors round-trip exactly, successful mutations canonicalize only
the affected rows, graph RLP remains canonical, and the historical activation-only regression policy remains explicit.
No CXX carrier, handle, export, shim, module flag, or snapshot-shape change occurs, so `CRW-07` has no inventory delta.
All required Rust, FinalChain Tier 2, Tier 3 parity, bridge-inventory, and pre-commit gates passed; independent review
approved the slice without blockers, and `CRW-09G` is now ready.

`CRW-09G` types transaction-fee ownership, commission/delegator reward deltas, persisted reward pools, account credits,
and successful claim receipts with `DposTokenAmount`. The consensus-private `StoredDposTokenAmount` wrapper preserves
untouched empty, canonical-minimal, and fixed-32 snapshot provenance for slots 1 and 7, rejects malformed or duplicate
rows before restart-state installation, and canonicalizes successful mutations. Reward indexes remain arbitrary-width
and consensus-private; exact delegator settlement checks contract funds before converting to the payable U256 domain.
Candidate-state publication keeps pool, cursor, account, receipt, and reward-stat rollback atomic. Minted totals, supply,
Aspen migration, yield/cap arithmetic, and header reward typing remain in `CRW-09H`. No CXX, bridge, shim, module-flag,
snapshot-shape, compatibility-test, or `CRW-07` inventory delta is introduced.
All 58 `rustaxa-types`, 811 `rustaxa-consensus`, and 347 `rustaxa-bridge` tests, Tier 1, FinalChain Tier 2, Tier 3
Rust-enabled/pure-C++ parity, bridge-inventory, and pre-commit gates passed. Independent configured review approved the
slice without actionable findings; `CRW-09H` is now ready.

`CRW-09H` keeps the broadly shared fungible U256 vocabulary as `DposTokenAmount` in `rustaxa-types` while defining
Aspen migration phase, yield, and persisted byte provenance privately in `rustaxa-consensus`. Reward configuration,
minted plans, supply-cap arithmetic, and finalized header rewards are typed after bridge ingress without changing the
external EVM or CXX byte carriers. Snapshot slots 8-10 retain their accepted schema, but conflicting fields, zero or
oversized supplies, activation-order violations, configured supply above the cap, and restart histories that regress
minted totals, migration phase, or total supply now fail closed. Migration remains lazy, records the complete
post-reward state once, and candidate staging prevents cap failures from publishing accounts, headers, snapshots, or
heads. No CXX, shim, module-flag, compatibility-test, or `CRW-07` inventory delta is introduced. All 58
`rustaxa-types`, 816 `rustaxa-consensus`, and 348 `rustaxa-bridge` tests and the required Tier 1, FinalChain Tier 2,
Tier 3 parity, bridge-inventory, and pre-commit gates passed. Independent configured review approved the final diff;
`CRW-09I` is now ready.

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
migration, or `CRW-07` inventory.

The `CRW-08` completion audit found no remaining demonstrated current-ABI method or failed-receipt family. All 25 DPoS
ABI methods are native-routed, all 16 mutation selectors share the Rust mutation kernel and transient-call path, both
slashing reads plus supported slashing execution are Rust-backed, and the dual-mode fixture inventory covers every
mutation family. `getDelegations(address,uint32)` already landed with native transaction, direct-call, graph-authoritative
reward, paging, corruption, restart, and pure-C++ parity coverage. Historical databases whose snapshots predate the
complete account/DPoS/reward graph still fail closed and require replay/rebuild or an explicitly designed hybrid route;
that deployment compatibility boundary is retained explicitly and was not redefined as completed behavior. With no
current-ABI parity gap remaining, the dependency-ordered queue advances to ready `CRW-09`.

The first bounded `CRW-09` slice replaces the Rust FinalChain `u64` nonce boundary with
`rustaxa_types::FinalChainNonce`, an arbitrary-width unsigned domain type encoded as canonical minimal big-endian bytes.
Account snapshots preserve the existing six-item schema and byte-identical RLP for old values while accepting state
above `u256`; native Cornus execution advances a `u256::MAX` transaction nonce to a 257-bit account nonce and persists it
across restart. Finalization requests, Rust-planned external-EVM transcripts, account lookup results, and system-account
facts use canonical byte carriers instead of narrowing conversions. Execution request identities are domain-versioned
and hash length-prefixed nonce bytes. System-transaction construction explicitly rejects a nonce above the legacy U256
wire limit, and the unchanged public C++ account API throws `FINAL_CHAIN_NONCE_EXCEEDS_CPP_U256` rather than truncating
unrepresentable Rust state.

This is a cross-cutting `CRW-07` carrier-field change: four existing CXX nonce fields change from `u64` to canonical
`Vec<u8>`, but no handle, free export, constructor, shim, module flag, or compatibility-only test is added or removed.
The standalone dual-mode fixture proves a nonce above `u64` through transaction execution, receipt lookup, account
materialization, and restart; Rust tests cover canonical encoding, leading-zero rejection, old snapshot byte identity,
state above U256, maximum-nonce advancement, system-transaction limits, and request-ID sensitivity. `CRW-09` remains
active for the remaining P0 FinalChain domain families and external-executor adapter contraction.

The next bounded `CRW-09` slice introduces `FinalChainTransactionPosition(u32)` for Rust-owned execution, publication,
receipt, location, audit, and transaction-index semantics. Regular transaction counts are checked when the execution
session is constructed, and the combined regular/system count is checked before the session can emit
`EXECUTE_EXTERNAL_EVM`; unrepresentable positions therefore reject before `StateAPI` side effects rather than during
later publication. Existing CXX execution/report and receipt-query positions remain `u64`, while persisted index and
location fields remain `u32`, with checked ingress and infallible widening at bridge edges. Request identities retain
their established eight-byte widened position
preimage, and persisted location/publication RLP retains its existing `u32` schema. This changes no CXX carrier, handle,
export, shim, module flag, compatibility test, or `CRW-07` inventory entry. `CRW-09` remains active for the other P0
FinalChain domain families and the temporary external-executor adapter contraction.

The next bounded `CRW-09` slice promotes the existing 256-byte storage bloom invariant into the shared
`FinalChainLogBloom` domain type. Stored and full headers, native and external-EVM bloom construction, commit/publication
plans, pending markers, storage chunks/index updates, and Rust query paths now carry the fixed-size value. Persisted
header and pending-marker decoders reject malformed widths immediately with contextual error codes instead of allowing
arbitrary `Vec<u8>` values to reach late commit/audit checks. Valid historical header RLP/hashes, publication identities,
marker and chunk bytes remain unchanged. CXX and RPC carriers remain vectors or arrays with explicit edge conversion.
This changes no CXX carrier, handle, export, shim, module flag, compatibility-only test, or `CRW-07` inventory entry.
`CRW-09` remains active for role-specific monetary and scalar domains plus any bounded external-executor contraction.

The next bounded `CRW-09` slice introduces `FinalChainGasPrice(U256)` for finalization transactions, transient calls,
external-EVM inputs, native affordability/charging, and external fee/reward derivation. Bridge ingress accepts the
existing zero-to-32-byte big-endian shape, including fixed-width leading zeros, and rejects wider values with
`FINAL_CHAIN_GAS_PRICE_EXCEEDS_U256`. Rust-to-CXX execution requests always emit the production-compatible fixed 32-byte
shape. Request IDs retain fixed 32-byte regular-transaction prices and the legacy minimal system-transaction price
preimage, so no domain-version change is required; short regular Rust fixtures and equivalent fixed-width input normalize
to one identity. Transaction value, account balance, stake, reward, supply, and
reward-per-stake remain separate roles. This changes no CXX carrier, handle, export, shim, module flag,
compatibility-only test, or `CRW-07` inventory entry. `CRW-09` remains active for the remaining role-specific monetary
and scalar domains plus any bounded external-executor contraction.

The next bounded `CRW-09` slice introduces `FinalChainTransactionValue(U256)` for finalization transactions, transient
calls, external-EVM inputs, native transfers, and payable DPoS execution. Bridge ingress accepts zero through 32
big-endian bytes and rejects wider values with `FINAL_CHAIN_TRANSACTION_VALUE_EXCEEDS_U256`. Regular C++ execution
requests retain fixed 32-byte values, while decoded system transactions retain the legacy minimal representation with
zero encoded as `[0]`; request identities preserve the same source-sensitive split without a version change. Canonical
transaction RLP/hash/root bytes remain authoritative and are never reconstructed from the numeric type. Account
balances, gas prices, stakes, fees, rewards, and supply remain distinct roles. This changes no CXX carrier, handle,
export, shim, module flag, compatibility-only test, or `CRW-07` inventory entry. `CRW-09` remains active for account
balance encoding analysis, remaining scalar domains, and any bounded external-executor contraction.

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
materialization and the existing typed PBFT preview/stage/commit boundary. The later `CRW-09I` slices remove the DAG
verification and proposer FinalChain/sortition fact relays from C++.

The latest `CRW-05` slice absorbs external-EVM rewards-stat planning and publication state into Rust `FinalChain`.
Validated execution reports now build a session-owned rewards plan bound to the exact request, period, prior FinalChain
head, and rewards-runtime generation. Rust supplies only canonical distribution-stat RLP to the external C++
`StateAPI::distribute_rewards` adapter, attaches the storage mutation to its own publication plan, verifies matching
already-applied publications against durable rewards rows, and monotonically binds the live runtime to a head-stable
storage snapshot after durable publication or recovery. Planning fails closed while durable head and runtime head differ.
The production FinalChain overlay no longer constructs `rewards::Stats`, relays a
`FinalChainExternalEvmRewardsStatsUpdate`, or acknowledges/clears a second runtime. The standalone `rewards::Stats` and
`BridgeRewardsStatsRuntime` surfaces were compatibility-test-only at this point and were later deleted by `CRW-14`;
the untouched legacy class remains pure-C++-reference-only. The following `CRW-05` slice completes the
PBFT-owned pillar lifetime target.

The latest `CRW-05` lifetime slice moves the complete pillar runtime behind the application-owned `BridgePbftService`.
Full-service construction restores exactly one private pillar state; `PillarChainManager` replays startup data on that
same state and completes a pillar-specific readiness transition before live calls. Chain-only services fail with
`PBFT_SERVICE_PILLAR_UNAVAILABLE`, while a narrowly named partial service exists only for compatibility constructors and
tests. Production App injects its existing service, `BridgePillarChainRuntime` and its factory are deleted, and all pillar
receivers use sibling service locking without crossing C++ FinalChain, signing, network, event, or materialization
effects. PBFT's four pure current-anchor decisions now call the shared service directly; the public manager wrappers
remain compatibility adapters. The later storage-lifetime contraction deletes the separate
`BridgePillarChainStorage` handle while preserving stable `DbStorage` methods on the existing root storage adapter.

The next bounded `CRW-05`/`CRW-07` slice internalizes the PBFT-finalization sortition commit across the two
application-owned services. C++ still derives the finalized DAG/transaction counts needed by the Rust operation and
keeps the previewed optional change in the primary Rust storage batch, but it no longer commits sortition through the
compatibility facade or returns a six-field live-state report to PBFT. `BridgePbftService` validates the active cursor
and retained storage-stage change before `BridgeDagTransactionService` atomically publishes a cloned next sortition
state; stale cursors and preview mismatches leave sortition unchanged. The CXX report carrier and the rewrite-only C++
commit helper are deleted. A post-primary preview/stage divergence is fatal rather than retryable because duplicate
resume does not replay protected sortition mutation. The later `CRW-09I` slices remove the DAG verification and proposer
FinalChain/sortition fact relays from C++.

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
`BridgeStorage`, `BridgeStorageBatch`, storage-query-family, and `DbStorage` references are
limited to typed application/bootstrap construction, stable public compatibility, network/query, external FinalChain/EVM,
admin/migration, conformance, and test boundaries. `BridgeStorageBatch` is an opaque carrier inside the stable
`DbStorage::Batch` lifecycle; C++ compatibility callers still sequence typed append operations, while Rust owns
validation, key/column selection, batch storage, and atomic commit.
At CRW-06 closeout, standalone `rewards::Stats::processStats(..., Batch&)` remained test compatibility; `CRW-14`
subsequently deleted that Rust-mode facade and its batch relay after native FinalChain ownership was verified. The
storage-boundary and bridge-inventory guards, targeted symbol searches, skill/prompt
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

The public `PbftChain::updatePbftChain(...)` signature remains stable. Rust-mode construction now requires the
application-owned full PBFT service; the former `PbftChain(addr_t, std::shared_ptr<DbStorage>)` compatibility constructor
and chain-only bridge factory are deleted. The C++ `PbftChain` class itself is not yet a `CRW-02` deletion target because
its external compatibility consumers are still live. Normal finalization, duplicate resume, storage failure/crash recovery,
concurrent chain reads, shared-handle teardown, and bootstrap rejection are required targeted coverage for the composed
service.

#### CRW-02 PBFT application service implementation

`CRW-02` implemented the selected boundary as one exported `BridgePbftService`. That exported type is now a thin CXX
adapter over native `rustaxa-consensus::PbftService`, which owns coherent sibling restoration and bootstrap lifecycle.
The sibling services own the manager runtime, period-data queue, sync/proposal/finalization sessions, PBFT chain state,
block lookups, and shared native storage. Separate manager and chain locks preserve concurrent public reads; operations
requiring both follow the manager-before-chain order and do not retain a Rust guard across a C++ executor call.

Production construction is one `create_pbft_service_from_storage` adapter call into native
`PbftService::restore`. Native Rust validates slashing configuration, restores every storage-backed sibling from one
storage handle, derives the manager period and Cacti activation from the restored chain head plus immutable
`PbftServiceConfig`, and returns only a complete root. The
production service starts behind an explicit bootstrap gate. `PbftManager` completes the gate only after
startup replay, `initialState`, wallet eligibility, and pillar restart processing; daemon, proposal, and PBFT sync
session entry points fail closed before that transition.

`App` owns one shim-local `PbftService` RAII holder and shares it with the retained C++ `PbftChain`, `PbftManager`, and
`VoteManager` facades. The app bootstrap edit is an intentional guarded `RUSTAXA_ENABLE` exception in upstream-owned `app.hpp` and
`app.cpp`; pure-C++ construction remains unchanged and no main-only header is required when Rust mode is disabled. The
Rust-mode `PbftChain` construction accepts only the application-owned full service, while public
`updatePbftChain(...)` mutates service-owned state as compatibility behavior rather than independent production
authority.

The exported `BridgePbftManagerRuntime` and `BridgePbftChain` handles, their production constructors, app-side startup
derivation, facade-owned boxes, `PbftChainFinalizationUpdateReport`,
`PbftChain::updatePbftChainForPbftFinalization`, and
`pbft_manager_runtime_advance_finalization_pbft_chain` are deleted. Finalization now drains the chain update inside the
service, validates the resulting head against the accepted finalization plan, and advances the existing manager cursor
before returning the next true external effect. The obsolete CXX `PbftManagerStartupFact` test carrier is also deleted;
Rust-only tests use a private fixture, while C++ bridge/storage tests seed a durable chain head and exercise the production
service constructor.

Focused coverage proves chain-derived period/Cacti facts, invalid configuration failure, bootstrap rejection and
one-way completion, shared full-service chain visibility/lifetime, internal finalization drain, Rust bridge
manager/sync behavior, the C++ public chain facade, single-node/null-anchor finalization, consensus bridge fixtures, and
storage lifecycle behavior. Required Tier 1, consensus Tier 2, bridge inventory, storage bridge, smoke, and upstream-file
diff evidence is recorded in the consolidation plan closeout for this slice.

#### CRW-03 PBFT-private state absorption

`CRW-03` is complete after dependency-ordered implementation sub-slices. The first moved proposed-block state into the
application-owned `BridgePbftService`, migrates production callers in the PBFT and vote-manager facades, replaces the
storage shim's independent proposed-block handle with storage-only compatibility operations, and deletes
`BridgeProposedBlocks`. Tentative wallet candidates remain caller-owned block/vote pairs until leader selection; they
enter the Rust planner only as identity-checked facts and never enter the authoritative proposed-block index.

The second ownership sub-slice is implemented: verified-vote/admission state is restored into the same service before
publication, `BridgeVerifiedVotes` and its factory are deleted, and the retained C++ vote/network facades are service
clients. Coherent state, step, and current-reward materializers use owned snapshots from one vote-lock epoch. Proposed-
block absorption landed first so this change did not recreate an independently owned cross-shim handle.

The current service uses sibling Rust lock domains for manager, verified votes, proposed blocks, and chain state.
Production construction restores all four state families before bootstrap publication; no exported partial-service
topology remains. No service guard crosses C++ validation,
network, FinalChain/EVM, logging, or gossip callbacks.

The first native verified-vote owner extraction is implemented. CXX-free
`PbftVerifiedVotesService` retains the Rust storage `Arc`, restores `PbftVoteAdmissionRuntime` before publication, and
owns its shared mutex. Production `BridgePbftService` publishes that native service capability as part of the single
full composition; it no longer owns a `Mutex<Option<PbftVoteAdmissionRuntime>>`. Native service tests
cover restoration, malformed durable-row rejection before publication, cloned-handle state visibility, and retained
storage lifetime. The former storage-free bridge test
configuration, duplicate empty-restore factory test, and duplicated replay/threshold-cache behavior test are deleted.
The production `BridgePbftService` root also drops its redundant optional storage `Arc`; manager and native sibling
owners now supply the exact durable handle for each operation, with a `#[cfg(test)]` mirror retained only by legacy
bridge fixtures. Temporary raw guard access remains explicit
`CRW-12` debt for cross-domain validation, admission/effect conversion, leader selection, combined cleanup, and
finalization.

The proposed-block sub-slice is implemented. The CXX-free native `ProposedBlocksService` owns restoration, storage,
and the sibling lock; PBFT service construction embeds that owner before publication. Push and cleanup serialize
durable-first mutation plus live publication under its Rust proposed-block write lock.
`ProposedBlocks` is now a lock-free C++ client view over the shared service, `VoteManager` no longer accepts a
`ProposedBlocks&`, and the former temporary `ProposedBlocks(DbStorage)` candidate index is replaced by one ordered
non-persisted Rust batch lookup. `DbStorage` save/snapshot compatibility uses the existing typed PBFT storage-query
handle rather than free functions over `BridgeStorage`, so no
second process-local index exists. `VerifiedVotes` likewise owns no runtime, storage handle, or mutex; Network still
reaches vote state through the stable `VoteManager` facade.

The final combined-operation debt is closed. Period advance now emits only the
remaining external timer, wallet, vote-manager, and counter effects. Its native
commit validates reset provenance, then holds manager, verified-vote, and
proposed-block guards while it commits all proposal deletes, publishes both
sibling cleanups, and finally publishes the manager period through an
infallible callback before releasing any guard. Rejected validation or storage
commit leaves all three owners unchanged and retryable; empty cleanup still
publishes the manager period in the same lock epoch. The standalone production
cleanup API, CXX cleanup action/result/export, bridge module, shim executor
branch, and former manager-only VoteManager cleanup wrapper are deleted.

#### Aggressive-cutover boundary work

These items were originally scope-gated after the non-network/non-EVM closeout. The task owner has now authorized Rust
ownership of network consensus pipelines and external-EVM orchestration. Physical tarcap mechanics and concrete
EVM/`state_db` execution remain leaf C++ boundaries.

| ID | Status | Work | Unblock condition | Complete when |
| --- | --- | --- | --- | --- |
| `CRW-N01` | `active` | Implement application-owned network ingress/egress pipelines, finish PBFT gossip effect-drain integration, fix deferred vote duplicate-with-block delivery, and migrate consensus routing/queueing decisions out of tarcap handlers. | Aggressive network consensus cutover is authorized in `PLAN.md`; coordinate with `CRW-12` and `CRW-16`. | Rust owns packet inspection, admission/routing, consensus queues, peer/gossip/send decisions, typed effects, and result validation; C++ tarcap owns only socket/peer mechanics, wrapping, physical transport/disconnect execution, and lane scheduling. |
| `CRW-E01` | `ready` | Contract the external EVM/StateAPI boundary: move execution orchestration, canonical rewards payloads, result/receipt validation, commit ordering, recovery, and publication into Rust while retaining concrete EVM and `state_db/` operations as leaf C++ calls. | Aggressive execution-orchestration cutover is authorized in `PLAN.md`; coordinate with `CRW-17`. Moving concrete EVM execution itself remains out of scope. | `ConsensusExecutionApi` presents typed leaf operations; StateAPI consumes Rust-native/canonical requests and rewards data without C++ consensus materialization; no C++ manager owns execution sequencing or publication decisions. |

The latest `CRW-N01` contraction replaces five independently constructed network bridge owners with one
`Network`-owned `BridgeConsensusNetworkApi`, shared by both tarcap capabilities and their vote, sync, DAG-status, and
pillar-vote handler families. Queued effects are partitioned by a capability transport lane so latest and v5 handlers
cannot execute one another's work, while preserving queue order among dependency-ready effects in each lane. PBFT gossip
effects now own canonical
vote RLP and optional PBFT-block RLP, so execution no longer depends on caller-local C++ object lifetimes. Tarcap remains
the leaf executor for peer state, packet wrapping, physical sends, disconnects, and lane scheduling; the shared wrapper
serializes each lane's complete drain/execution/acknowledgement cycle across concurrent packet workers. Vote admission still
enters the C++ VoteManager and other handler-local consensus routing remains active `CRW-N01` debt. The exact bridge
budgets are 22,430 bridge lines, 16,752 shim lines, 379 CXX functions, 334 carriers, 18 handles, 10 shim directories,
and 38 non-test generated-header consumers.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, all 40 focused native network tests, all 17 CXX
network bridge tests, the concurrent lane-execution test, Rust-enabled and pure-C++ `network_test` builds, and the focused
peer-cache/PBFT-sync packet cases
in both modes. Inventory and storage-boundary guards, inventory self-test, targeted C++ formatting, and whitespace checks
also pass. The aggregate consensus gate reaches the existing `pillar_chain_test` same-process RocksDB lock leak: 10 of
13 cases pass, while the same three later node-constructing cases cannot reacquire `/tmp/taraxa0/db/db/LOCK`; the
network-root tests do not fail. Original upstream-owned network files change only to inject the guarded Rust-mode shared
root and keep the separately built pure-C++ route operational; the new wrapper source itself is feature guarded.

The next `CRW-N01` contraction moves post-admission PBFT vote routing into that shared root. Rust queues proposed-block
publication before dependent block-known and gossip effects and advances those effects only after the application leaf
reports success. Exact verified-vote duplicates still publish an attached previously unseen proposed block but do not
regossip the vote; failed publication cancels dependent work. The vote handler no longer directly processes or marks
the proposed-block sidecar. The same slice deletes the ignored generic packet shadow-ingress call, its retained byte
arena and payload-id allocator, two partial capacity settings, one CXX function, and two CXX carriers. Exact budgets
fall to 22,387 bridge lines, 16,752 shim lines, 378 CXX functions, and 332 carriers; handles and shim directories remain
18 and 10. `CRW-N01` remains active for composing verified-vote admission and the other handler-local routes behind the
application-owned network pipeline.

Validation for this contraction passes `rewrite-validate-fast`, `rewrite-validate-smoke`, all 37 focused native network
tests, all 15 focused CXX network bridge tests, the serialized transport-lane executor case, and focused Rust-enabled and
pure-C++ peer-cache/PBFT-sync packet cases. The pure-C++ `network_test` target builds from a fresh
`RUSTAXA_ENABLE=OFF` tree. Inventory and whitespace guards pass. The aggregate consensus gate again reaches the known
`pillar_chain_test` same-process RocksDB lock leak: 10 of 13 pillar cases pass, while the same three later
node-constructing cases cannot reacquire `/tmp/taraxa0/db/db/LOCK`; no network-root test fails.
Independent review also drove an atomic duplicate-outcome assertion through `VoteManager`, closing the race between the
fast verified-map check and admission. A handler-level injected publication-failure/duplicate-sidecar integration case
remains `CRW-N01` validation debt; native dependency cancellation and the atomic duplicate report are covered separately.

The following `CRW-N01` slice moves verified-vote admission sequencing into the shared network effect pipeline. Accepted
ingress queues a canonical PBFT-vote application effect, and Rust retains the admission context until C++ reports the
typed `VoteManager` leaf result. The packet worker holds the lane lock across ingress, drain, execution, and
acknowledgement and correlates synchronous completion by the exact effect ID, preventing another worker from draining
the result. Rust validates impossible admission reports and releases block publication, peer-known, and gossip effects
or returns the typed slashing outcome only from a successful matching result. Bundle handlers preflight the complete bundle under the lane lock before
admitting any member, preserving atomic shape rejection. The standalone admission-route function and carrier are
deleted; exact budgets fall to 22,360 bridge lines, 377 CXX functions, and 331 carriers, while shim lines, handles, shim
directories, and non-test consumers remain 16,752, 18, 10, and 38. `CRW-N01` remains active for native bundle
aggregation/rebroadcast and the other handler-local routes.

The upstream-owned latest-tarcap header and handler sources retain a guarded Rust-only integration exception for this
slice. Pure-C++ compilation selects the unchanged legacy admission/routing body; the guarded methods are temporary
network-overlay debt until a complete tarcap pipeline overlay can own the Rust route without original-file changes.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, all 39 focused native network tests, all 50 CXX
consensus bridge tests (including 18 network cases), the focused verified-vote test, the serialized lane-executor case,
and isolated behind-round/same-round PBFT-sync plus peer-cache cases in both Rust-enabled and fresh pure-C++ builds.
Inventory, formatting, and whitespace guards pass. The isolated node-backed runs also confirm that no packet worker
loses its correlated admission effect to another worker.

The next `CRW-N01` contraction replaces member-at-a-time bundle ingress with one operation-specific complete-bundle
call. Rust preflights every member before queuing admission, allocates a unique aggregation session, correlates each
application result by exact effect id, preserves accepted-member input order, and emits one optimized bundle gossip
effect only after clean completion. Ordinary rejection and exact duplicates are omitted; executor failure or slashing
cancels remaining queued admissions and suppresses aggregate gossip without rolling back earlier admissions. The
Rust-mode packet handler no longer builds an accepted `PbftVote` vector or invokes its handler-local rebroadcast route;
tarcap retains peer-known filtering, packet wrapping, physical send, and successful-send marking. The member ingress
export is replaced rather than supplemented, CXX function/carrier counts remain 377/331, and shared bridge lock-error
mapping lowers bridge lines to 22,342. Shim lines, handles, shim directories, and non-test consumers remain 16,752, 18,
10, and 38. `CRW-N01` remains active for the other handler-local routing families.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, all 45 focused native network tests, all 18 focused
CXX network bridge tests, the Rust-enabled network target, and isolated behind-round/same-round PBFT-sync plus
serialized transport-lane cases. The fresh pure-C++ network target and isolated behind-round/same-round cases also
pass. Inventory, storage-boundary, formatting, and whitespace guards pass. The aggregate consensus gate again reaches
the known `pillar_chain_test` same-process RocksDB lock leak after 10 of 13 cases pass. The broader CTest run is not a
slice gate in this partial build tree: it additionally reports unbuilt test binaries, the same shared-database lock
failures, and unavailable static zlib/snappy libraries for Go tests. Python integration is unavailable because the
environment lacks `virtualenv` and `pytest`.
Independent review found that a failed correlated admission was acknowledged to Rust but not surfaced back to the C++
bundle loop. The executor now reports the failure first so Rust cancels dependent and remaining bundle work, then throws
the original diagnostic before the handler can request another admission ID; this also makes final-member failure
observable instead of silently returning an empty result.

The following `CRW-N01` slice moves get-next-votes response routing behind the shared network root. Rust now owns the
request period/round gate, exact previous-round application query, atomic result validation, 1,000-vote packet chunking,
and next-before-next-null send ordering. One exact-ID application effect calls an atomic native verified-vote leaf that
builds both optimized bundle families under one lock epoch; missing families are successful empty slots, while any
invariant or encoding failure returns no partial pair. Tarcap only decodes the request, supplies the current PBFT
snapshot, wraps and physically sends Rust-approved chunks, and marks decoded votes known after successful transport.
The Rust-mode handler-local retry, vote materialization, chunking, and direct send decisions are unreachable. Two unused
plan/build CXX exports and four DTOs are replaced by the one atomic leaf while the network ingress export is added, so
CXX functions remain 377 and carriers fall to 328. Exact budgets are 22,100 bridge lines and 16,731 shim lines; handles,
shim directories, and non-test consumers remain 18, 10, and 38. `CRW-N01` remains active for the other handler-local
routing families.

Focused native consensus and bridge tests, the Rust-enabled and fresh pure-C++ `network_test` targets, both isolated
next-vote synchronization cases, `rewrite-validate-fast`, `rewrite-validate-smoke`, and the bridge inventory guard pass.
The aggregate consensus gate reaches the established `pillar_chain_test` database-lock baseline: 10 of 13 cases pass,
while `votes_count_changes`, `pillar_chain_syncing`, and `finalize_root_in_pillar_block` fail to reopen the same temporary
RocksDB path within one process.

The next `CRW-N01` slice moves the single and bundle pillar-vote ingress family behind one operation-specific network
call. Rust atomically preflights complete packets for canonical decoding, signatures, activation periods, and duplicate
hashes, then correlates one native PBFT-service admission effect per member by exact effect ID. Newly accepted votes
alone release sender-known effects and, for the single-vote packet, gossip; bundle sync ingress and exact duplicates do
not rebroadcast, and gossip execution honors the Rust-provided source-peer exclusion directly. The C++ leaf keeps only
pillar-vote materialization, FinalChain-composed native admission, peer-state
mutation, packet wrapping, and physical fanout. The former split `validatePillarVote`/`addVerifiedPillarVote` receipt set
is deleted, and compatibility insertion re-enters the same fully checked atomic admission task. A private constructor-only
route retains trusted admission for storage-authenticated startup votes whose historical anchor is no longer live. Two production packet
handlers migrate. The obsolete network-only relevance export and fact carrier are replaced by the bundle-ingress export
and its context, leaving CXX functions and carriers at 377 and 328. Bridge lines fall to 22,085; shim lines, handles,
shim directories, and non-test consumers are 16,728, 18, 10, and 38. `CRW-N01` remains active for the other
handler-local routing families.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, 54 pillar-focused native consensus tests, all 18
CXX network bridge cases, the Rust-enabled `network_test` and `pillar_chain_test` builds, three focused checked-admission
pillar cases, the startup restore regression, the isolated multi-node pillar synchronization case, two focused
lane/peer-cache network cases, and the bridge inventory guard. The aggregate consensus gate
again records the established three-case `pillar_chain_test` same-process RocksDB lock baseline. The formerly stale
`test_pbft_sync.cpp` admission call is now repaired, and the CXX aggregate builds and passes all 52 tests. A fresh
all-Rust-disabled tree compiles every changed network source through `core_libs`, then stops at the pre-existing
`App`/`Network` constructor mismatch before linking `network_test`. The upstream-owned tarcap headers, handlers,
registration source, and guarded test additions remain explicit Rust-only integration exceptions; their pure-C++ bodies
select the legacy route until a complete packet-family overlay can own the cutover.

The following `CRW-N01`/`CRW-12` slice moves network-runtime ownership into native `PbftService` and internalizes both
remaining vote-response queries. `ConsensusNetworkService` is constructed once from the root's pillar and verified-vote
siblings; bridge adapters clone that service and therefore share one queue, effect sequence, dependency state, and
schedule. `rustaxa-bridge` no longer constructs or locks a standalone network API, accepts a partial queue-capacity
configuration, or carries application-result payload pairs. Get-next-votes queries the native verified-vote sibling
directly. Get-pillar-votes validates the immutable Ficus schedule, performs live-first/stored-period fallback lookup,
revalidates canonical vote identity/signatures/hashes/order, chunks at 250, and queues a send followed by exact dependent
known-vote effects. Invalid peer requests queue report then disconnect; local lookup, empty, and invariant outcomes are
typed zero-effect decisions. C++ retains only lane serialization, packet wrapping, physical transport, peer bookkeeping,
and acknowledgement. The obsolete VoteManager/PillarChainManager egress helpers, five CXX carriers, one export, and 61
shim lines are deleted. Exact budgets are 22,006 bridge lines, 16,667 shim lines, 376 CXX functions, 323 carriers, 18
handles, 10 shim directories, and 38 non-test consumers. `CRW-N01` remains active for the other handler-local routes.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, all 1,101 native consensus tests (including the
disabled-Ficus zero-interval request regression), all 77 bridge tests, the bridge inventory and storage-boundary guards,
Rust-enabled `network_test`, `pillar_chain_test`, and `pbft_chain_shim_test` builds, the isolated peer-cache and
multi-node pillar-sync cases, and both PBFT-chain shim cases. Independent review found and closed two production
blockers before approval: executor acknowledgements are no longer retained in an unbounded peer-amplifiable journal,
and disabled Ficus schedules short-circuit before interval validation or modulo arithmetic. The Tier 3 CTest attempt
records the established partial-tree baseline: five binaries are not built, aggregate node suites retain same-process
`/tmp/taraxa0` RocksDB locks, and Go contract tests lack static zlib/snappy; 9 of 21 registered tests pass. Python
integration remains unavailable because the environment lacks `virtualenv` and `pytest`. A fresh all-Rust-disabled tree
compiles every changed network source through `core_libs`, then reaches the previously tracked `App`/`Network`
constructor mismatch before linking `network_test`. The aggregate CXX bridge target now builds and passes all 52 tests.

The next `CRW-N01`/`CRW-12` slice moves the complete Get-PBFT-sync response family behind the application-owned
`ConsensusNetworkService` for tarcap versions five and six. Rust now decodes the canonical request, snapshots the native
PBFT chain/reward votes/proposed blocks through fallible sibling APIs, enforces checked chain and light-history bounds,
reads finalized period bytes directly from Rust storage, and emits complete packet-ready PBFT-sync payloads. Version six
also emits deterministically ordered proposed-block bundles of at most ten; version five preserves its historical
omission. Missing storage data preserves the legacy already-built prefix and version-six proposal behavior, while
malformed/range/history requests produce typed report-then-disconnect effects. Tarcap retains only per-lane execution,
packet sealing, syncing-state clearing, peer mechanics, and exact acknowledgement. The old sync-egress payload helper,
proposal snapshot projection, two CXX carriers/functions, and two PBFT-manager shim methods are deleted; one narrow
network request carrier/function replaces them. Exact budgets are 21,960 bridge lines, 16,626 shim lines, 375 CXX
functions, 322 carriers, 18 handles, 10 shim directories, and 38 non-test consumers. `CRW-N01` remains active for
PBFT-sync intake and the remaining status, DAG, transaction, and admission routes. Upstream-owned changes in this slice
are limited to guarded application configuration, guarded handler source selection/registration, and focused integration
coverage; the original latest/v4 Get-PBFT-sync handler implementations have no worktree diff and remain the pure-C++
reference route.

Validation passes `rewrite-validate-fast`, `rewrite-validate-consensus`, `rewrite-validate-smoke`, all 1,104 native
consensus tests, all 76 bridge tests, the Rust-enabled `core_libs`/`network_test` builds, the focused lane-serialized
executor case, the bridge inventory self/live guards, the storage-boundary guard, and a fresh all-Rust-disabled
`core_libs` build that compiles both untouched legacy Get-PBFT-sync handlers. Independent review approved the cutover
with no blocker. The aggregate CXX bridge build now passes all 52 tests. Tier 3 CTest reproduces the established
9-of-21 partial-tree baseline:
six binaries are unbuilt, aggregate node suites hit same-process `/tmp/taraxa0` RocksDB locks, and Go contract tests
lack static zlib/snappy. Python integration remains unavailable because this environment lacks `virtualenv`/`pytest`
and enforces PEP 668. Focused reward-bundle golden parity and an explicit send-failure executor fixture remain
non-blocking follow-up coverage.

The following `CRW-N01` contraction replaces the remaining Rust-mode get-next-votes handler shell with a standalone
transport adapter. `ConsensusNetworkService` now shares the native `PbftManagerService` lock domain and snapshots the
live period/round cursor before releasing that guard and querying verified votes. The Rust-mode packet family no longer
inherits `IVotePacketHandler` or receives `PbftManager`, `PbftChain`, `VoteManager`, or `SlashingManager`; the untouched
original handler is compiled only for pure-C++ reference mode. Tarcap retains two-scalar request decoding, canonical
outer packet wrapping, physical send, and successful-send peer-known marking. Operation-scoped drains use the retained
source payload id, preventing this synchronous executor—and the adjacent pillar/PBFT-sync executors—from consuming
unrelated work in the same lane. The existing drain export is narrowed rather than adding a second bridge operation,
lowering the exact bridge budget to 21,959 lines while CXX functions, carriers, handles, shim directories, and non-test
consumers remain 375, 322, 18, 10, and 38. `CRW-N01` remains active for PBFT-sync intake and the status, DAG,
transaction, and proposed-block admission families.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, the exact bridge inventory guard, 63 focused native
network tests, the focused bridge network adapter test, Rust-enabled `core_libs`/`network_test` builds, both isolated
next-vote synchronization cases, and a fresh pure-C++ `core_libs` build. The new CXX source-scoped-drain regression test
compiles and covers source id zero as a valid scoped id; the aggregate `rust_consensus_tests` target now builds and
passes all 52 tests.
Independent review found no consensus or security blocker and identified a reconnect bookkeeping race; the standalone
handler now refreshes the connected peer immediately before transport and applies successful-send known marks to that
same refreshed peer object. That prior slice left the original latest handler's pre-existing upstream delta unchanged.

The next `CRW-N01`/`CRW-12` contraction moves latest-version proposed-block bundle intake behind the native network
service. Rust owns raw packet and signed-block decoding, the ten-block bound, the current-through-five-period relevance
window, exact legacy 8/9-field block shape, signature/extra-data/reward-vote invariants, per-period recovered-author
uniqueness, FinalChain-head-gated DPoS eligibility, and storage-first proposal publication. Rust predecodes every member
before publication, while protocol admission deliberately remains sequential to preserve the reference handler's
partial-progress behavior when a later member is malicious. The standalone Rust-mode packet adapter retains only
syncing-peer gating and typed
malicious-peer execution; it no longer receives or materializes through `PbftManager`. The former single-wallet PBFT
eligibility export, two carriers, and shim method are deleted while one operation-specific network function replaces the
export. Exact budgets fall to 21,918 bridge lines, 16,599 shim lines, and 320 carriers; CXX functions, handles, shim
directories, and non-test consumers remain 375, 18, 10, and 38. `CRW-N01` remains active for PBFT-sync intake and the
status, DAG, and transaction families. The original latest handler remains the pure-C++ reference route.

Validation passes `rewrite-validate-fast`, `rewrite-validate-smoke`, five focused native proposed-block-bundle cases,
the Rust-enabled `core_libs`/`network_test` builds, the isolated multi-node `sync_large_pbft_block` path,
the exact bridge inventory guard, formatting, clippy with the unchanged warning baseline, and whitespace checks. A
fresh `RUSTAXA_ENABLE=OFF` tree compiles the complete untouched legacy network route through `core_libs`, then reaches
the previously tracked `App`/`Network` constructor mismatch while linking `network_test`. The aggregate consensus gate
again records the established same-process `/tmp/taraxa0` RocksDB lock failures. The aggregate CXX bridge binary now
builds and passes all 52 tests. Independent review required direct strict-decoder regressions; duplicate reward hashes,
recovery id four, and
oversized extra data are now covered and the review found no remaining correctness, ABI, or source-selection blocker.
An explicit CXX assertion for unfinalized proposal persistence remains non-blocking follow-up coverage; the multi-node
sync case exercises the live route but asserts finalized PBFT state rather than that intermediate storage row directly.

The next bounded `CRW-12` prerequisite makes the native PBFT period-data queue the sole owner of queued payload bytes
and source peer identities. The PBFT overlay no longer retains a parallel `PeriodData`/`NodeID` deque, generates
cross-runtime entry ids, checks queue alignment, returns removed entries for sidecar cleanup, or serializes queue calls
with a compatibility mutex. Rust pop plans now return the encoded period payload and fixed 64-byte peer id; C++
materializes `PeriodData` only at the remaining sync executor edge and restores normalized previous-certificate votes
before validation/finalization. Entry-id fields, the cleanup entry carrier, and unused CXX push/pop diagnostics are
deleted. Exact budgets fall to 21,850 bridge lines, 16,571 shim lines, and 319 carriers; CXX functions, handles, shim
directories, and non-test consumers remain 375, 18, 10, and 38.

Focused native queue tests pass 9/9, including the all-zero database-replay peer identity, the bridge adapter test
passes, Rust-enabled `core_libs` and `network_test` build,
and `NetworkTest.sync_large_pbft_block` passes through live queue push, drain, reconstruction, and finalization. The
retained boundary is explicit: the latest PBFT-sync handler still decodes/materializes `PeriodData`, derives compact
facts, and re-encodes the payload before native retention. `CRW-N01` remains active to pass original wire bytes, move
handler prechecks and previous-certificate normalization native, and delete that ingress materialization.

The following `CRW-12` contraction moves period-data queue fact production into native Rust. The CXX push shrinks from
twenty-two positional values to encoded `PeriodData`, fixed peer identity, normalized previous/current certificate
votes, and the temporary C++ PBFT-chain size. Before acquiring the queue lock, Rust enforces the four/five-field period
shape and exact eight/nine-field signed-block shape, recovers the block signer, validates unique reward references,
derives PBFT linkage/final-chain hashes, expands finalized DAG transaction references and optimized pillar votes, and
decodes transaction RLPs, hashes, nonces, and recovered senders. It also consumes each RLP value exactly, preserves
binary node-implementation extra data, validates full certificate-vote signatures, and proves the optimized previous
certificate bundle matches the supplied normalized full votes. Malformed payloads return stable
`PBFT_PERIOD_DATA_QUEUE_*` errors before mutation. The overlay deletes eight fact-extraction helpers and the
large-transaction sender-prewarm fanout; only certificate normalization remains because optimized legacy period data
does not retain vote weights.

Exact budgets fall to 21,706 bridge lines and 16,413 shim lines. CXX functions fall to 374; carriers, handles, shim
directories, and non-test consumers remain 319, 18, 10, and 38. Native queue/decoder tests pass 22/22, the bridge adapter passes,
Rust-enabled `core_libs` and `network_test` build, and `NetworkTest.sync_large_pbft_block` passes through the native
decoder. `CRW-N01` still owns original-wire handoff and the packet handler's pre-queue peer/order/certificate checks;
the queue-specific `CRW-12` sibling composition is closed: snapshot and encoded push sample the PBFT chain under the
manager serialization domain, so C++ no longer injects chain size, current period, or last hash. Sync-admission queue
clears also execute under that native manager lock; the CXX clear operation and shim helper are deleted.

The next bounded `CRW-N01` ingress contraction selects a standalone PBFT-sync handler for Rust-enabled latest and v5
tarcap while preserving the untouched original handler for pure-C++ mode. The handler passes the exact outer packet
bytes to `ConsensusNetworkService`, where Rust owns exact outer/nested decoding, optimized certificate reconstruction,
strict `PeriodData` validation, native chain/queue period and previous-link sampling, duplicate/sync-complete/drop
classification, certificate target checks, Ficus pillar/extra-data scheduling, and DAG-order hashing. Typed statuses
leave peer mutation, timers, transport, sync lifecycle publication, and the temporary FinalChain-weighted reward-vote
normalization/push executor in C++. Three ingress-only manager facade methods are deleted, and queue-drain stale cleanup
is now applied and acknowledged internally before Rust returns the next external executor step.

Raw precheck runs before any legacy `PbftSyncPacket`, `PeriodData`, vote, transaction, or DAG object construction, so
malformed nested input reaches typed native classification instead of legacy positional decoders. Proposed-block bundle
DPoS lookup failures also propagate as operational errors rather than being collapsed into malicious-peer decisions.

The previous guarded constructor hook in the upstream-owned latest handler is also retired: its header and source now
match `upstream-main` exactly, while guarded source selection keeps the standalone Rust adapter out of pure-C++ builds.

Exact budgets are 21,703 bridge lines and 16,329 shim lines; CXX functions, carriers, handles, shim directories, and
non-test consumers remain 374, 319, 18, 10, and 38. Native raw-packet tests pass 4/4, classifier tests pass 5/5, the
native queue suite passes 26/26, and the complete consensus library passes 1,135/1,135 including the service-level
cleanup regression. Rust-enabled `core_libs` and `network_test` build, and `NetworkTest.sync_large_pbft_block` passes
through the selected live handler. A fresh `RUSTAXA_ENABLE=OFF` tree builds `core_libs` with the exact upstream legacy
PBFT-sync handler and without the Rust adapter. The remaining ingress debt is
native FinalChain-weighted reward-certificate normalization and direct admission of the already-decoded original
`PeriodData` child without the retained C++ materialization/re-encoding executor.

The following bounded `CRW-N01` contraction removes the last Rust-mode period-data queue push facade from
`PbftManager`. The standalone PBFT-sync handler retains the exact validated `PeriodData` child bytes, performs only the
temporary C++ reward-certificate normalization, and submits the original child plus canonical normalized vote bytes
through the application-owned PBFT service. `App::rebuildDb` likewise retains the exact bytes returned by the legacy
database and calls the same native queue operation with the zero peer identity. The manager overlay no longer declares
or defines `periodDataQueuePush`, and its overlay-only vote-RLP helper is deleted; the untouched pure-C++ manager keeps
its reference implementation. The retained CXX queue-push export is now the narrow database-replay/tarcap lifecycle
adapter until rebuild and weighted ingress move fully native.

Exact budgets remain 21,703 bridge lines, 374 CXX functions, 319 carriers, 18 handles, 10 shim directories, and 38
non-test consumers, while shim lines fall from 16,329 to 16,284. All 1,135 native consensus tests and all 76 bridge
tests pass. Rust-enabled `app`, `core_libs`, and `network_test` build; the isolated live
`NetworkTest.sync_large_pbft_block` and raw-ingress regression pass. A fresh `RUSTAXA_ENABLE=OFF` tree still builds
`core_libs` through the untouched legacy route. Full native weighted sync ingress remains gated on preserving the
legacy slashing-transaction effect through a typed executor/report boundary; it must not be silently discarded.

The next `CRW-N01` slice closes that gate. `PbftService` owns a resumable weighted PBFT-sync ingress session configured
once with committee/proposer limits and ordered slashing submitters. After native raw precheck, an empty queue admits
each previous-cert vote strictly and durably against borrowed native FinalChain state, preserving duplicate-proof
ordering by pausing at an executable slashing transaction. C++ receives only nonce/value/gas/calldata/wallet facts,
constructs and inserts the signed transaction, and reports the result before Rust continues. Rust then selects retained
weighted reward payloads and queues the exact decoded `PeriodData` child with the complete 64-byte peer id; a nonempty
queue keeps the legacy bypass. Duplicate, complete, drop, malicious, benign reward-cursor stop, and queue rejection are
terminal typed actions. A new begin replaces stale sessions even when precheck fails, while rejected transaction
insertion leaves the proof planner-retryable and continues legacy packet admission order.

Tarcap no longer decodes or materializes `PbftSyncPacket`, `PeriodData`, DAG blocks, or PBFT votes and no longer depends
on `VoteManager`; it retains syncing-peer gating, peer facts, pacing, and the narrow transaction leaf. The old precheck
and direct handler queue wrappers, `VoteManager::getRewardVotesPbftBlockPeriod`, two dead PBFT-manager overlay methods,
and a redundant manager-named PBFT storage lookup are deleted. DAG/PBFT block-RLP lookups share one generic CXX carrier,
and the slashing DTO is projected inside `ConsensusNetworkApi`, keeping the production bridge-consumer count flat.
Exact budgets fall to 21,662 bridge lines, 16,259 shim lines, 373 CXX functions, 319 carriers, 18 handles, 10 shim
directories, and 38 non-test C++ consumers.

Validation passes all 1,215 native consensus/bridge tests, `make rewrite-validate-fast`, all 52 CXX consensus tests,
the Rust-enabled `app`, `network_test`, and consensus-test builds, isolated live PBFT sync, exact malformed-ingress/lane
execution, both inventory checks, and a fresh pure-C++ `core_libs` build through the untouched legacy handler.
Independent review found and closed one availability defect: slashing submitter resolution now reads configured wallets
in order only until the first funded account, so an irrelevant later FinalChain lookup cannot abort an already-published
conflict transition or disconnect the syncing peer. Native regressions cover that read bound and both accepted/rejected
slashing executor reports through the weighted ingress pause/resume state machine.

The same `CRW-12` closeout deletes five callerless `PillarChainManager` compatibility methods: direct finalization,
candidate-anchor validation, proposal-anchor selection, local-vote anchor selection, and restart post-processing
selection. It also removes their four shim-local result DTOs and two private decision enums. Native PBFT already calls
the pillar current-anchor planner directly for all four decisions and uses only the typed finalization preflight/ack
boundary, so no C++ replacement path is introduced. This removes 245 additional shim lines and lowers the exact
`shim_lines` budget to 16,014; all other inventory budgets remain unchanged. The 31 native pillar-chain tests pass,
both affected C++ targets build, and the isolated pillar synchronization case passes. The aggregate pillar binary still
records its established same-process RocksDB lock baseline plus two unrelated live-node timing/proposer-session
failures; the deleted methods had no callers in any of those paths.

The follow-on DAG contraction removes the callerless Rust-mode `DagManager` expiry-limit and non-finalized-minimum-
difficulty compatibility queries together with their last two scalar CXX exports. The now-unconsumed values are also
removed from the broad native runtime-status and non-finalized-summary projections; native `DagManagerState` continues
to own both values for expiry pruning and proposer/VDF policy. This deletes 14 bridge lines and 12 shim lines, lowering
the exact budgets to 21,648 bridge lines, 16,002 shim lines, and 371 CXX functions without adding a carrier or replacement
boundary. The untouched original manager and proposer retain their pure-C++ reference APIs.
All 1,215 native consensus/bridge tests pass after this contraction, as do all six Rust-mode `dag_test` cases and all
13 `dag_block_test` cases.

The same facade audit removes two more callerless Rust-mode methods: `DagManager::getDagConfig` and
`TransactionManager::packShardedTrxs`. Native DAG configuration remains constructor-owned, while the live proposer
uses the owner-bound transaction-pack session instead of this public materialization helper. Their untouched original
methods remain available only in the pure-C++ reference sources. The deletion removes another 22 shim lines and lowers
the exact `shim_lines` budget to 15,980 without changing bridge exports, carriers, handles, or consumers.

The final contraction retires the Rust-mode `KeyManager` facade completely. App and the DAG manager/proposer, vote,
and pillar overlays no longer construct, retain, or accept that internal manager; native PBFT and DAG services already
borrow FinalChain directly for VRF facts. The last standalone `FinalChain::dposGetVrfKey` shim method and its scalar CXX
export are deleted with the entire shim directory. Wallet/node-secret custody and actual signature execution remain at
the classified external signing boundary, while the untouched original `KeyManager` and its legacy callers remain
source-selected in pure-C++ mode. Exact budgets fall to 21,625 bridge lines, 15,838 shim lines, 370 CXX functions, and
nine shim directories; carriers, handles, flags, factories, and consumer counts do not increase. Fresh Rust-enabled and
`RUSTAXA_ENABLE=OFF` `app` builds pass after the constructor cutover, as do the six DAG cases, 13 DAG-block cases, 37
transaction-manager shim cases, and five focused pillar vote/restore cases. The 1,215-test native workspace, inventory
and storage-boundary guards, inventory self-test, and whitespace validation also pass. The aggregate vote binary retains
its existing fixed `/tmp/taraxa0` same-process RocksDB lock collision; it reaches no KeyManager-specific assertion failure.

The closeout also removes the one-use `PbftManagerRuntimeStorageApplyResult` CXX carrier. Executed-block reset now reuses
the lifecycle transition result with every unrelated command flag explicitly false, preserving only status, authoritative
snapshot, and error information. This keeps the bridge-line budget at 21,625 while lowering the carrier count to 318;
the 75 bridge tests cover the reused projection.

The next FinalChain contraction deletes the Rust-mode `dposValidatorsEligibleVoteCounts` compatibility method, its CXX
export, and the exclusive `DposValidatorVoteCount` carrier. Native PBFT pillar construction already queries the validator
vote-count set directly from native FinalChain; the only bridge callers were two assertions that duplicated native
coverage. The untouched original method remains source-selected for the pure-C++ pillar manager. Exact budgets fall to
21,601 bridge lines, 15,824 shim lines, 369 CXX functions, and 317 carriers.

Finally, pillar relevance and single-vote admission now share one scheduling-fact CXX carrier instead of two identical
two-field structs. Rust still converts that shared boundary input into distinct native admission and relevance domain
contexts, while C++ constructs the immutable Ficus schedule once. Removing `PillarVoteRuntimeRelevanceContext` lowers
the exact budgets again to 21,592 bridge lines, 15,822 shim lines, and 316 carriers without changing an export or fact.

The period-data pop projection now reuses that same canonical `PillarVoteRlpPayload` carrier instead of wrapping identical
`vote_rlp` bytes in a queue-specific struct. Rust queue ownership and the temporary C++ pop materialization retain the
same bytes and direction. Removing `PeriodDataQueuePillarVotePayload` lowers the exact bridge budget to 21,584 lines and
the carrier count to 315.

Canonical PBFT certificate-vote RLP now follows the same rule: period-data queue input/pop and debug-query projection
share `PbftCertVoteRlp` instead of maintaining an identical queue-only wrapper. Signing/weight bytes remain opaque and
unchanged across the boundary. Removing `PeriodDataQueuePbftVotePayload` lowers the exact bridge budget to 21,577 lines
and the carrier count to 314.

Pillar creation and public pillar-block queries now share `PillarValidatorVoteCountChange` for the same signed address
delta instead of projecting an identical query-only carrier. Native query and planning domain types stay distinct, while
the CXX boundary preserves address, signedness, and value exactly. Removing `PillarBlockViewVoteCountChange` lowers the
exact bridge budget to 21,571 lines and the carrier count to 313.

Transaction queue decisions and DAG-save effect reports now share `TransactionQueueHash` for their identical transaction
identity payload instead of maintaining `TransactionManagerHashCommand`. Native outcomes, ordering, and C++ log
materialization are unchanged. This lowers the exact bridge budget to 21,563 lines and the carrier count to 312.

The Rust-mode VoteManager facade also drops the callerless `getProposalVotes` materializer. Native prepare/finish leader
selection already owns proposal-vote snapshotting, ranking, validation commands, and stale-snapshot rejection; the only
production caller of the legacy API is the untouched pure-C++ PBFT manager. This removes another 19 shim lines and lowers
the exact shim budget to 15,803 without changing the native verified-vote payload export used by other live queries.

The Rust-mode PBFT manager facade drops the callerless `getRoundLambda` compatibility method as well. Active transition,
sleep, and dynamic-lambda persistence flows already consume the native runtime snapshot directly; the legacy method and
its three callers remain together only in the untouched pure-C++ manager. This removes 24 more shim lines and lowers the
exact shim budget to 15,779 without deleting the broadly shared runtime-snapshot export.

The callerless whole-`PeriodData` `validatePbftBlockPillarVotes` facade is removed too. Native sync admission already
retains canonical pillar-vote RLP, requests the live pillar validator only at its typed executor boundary, and owns report
ordering and terminal rejection. The narrower `validatePbftBlockPillarVotesWithRust` executor remains live for that task;
the legacy whole-object method and caller remain together only in pure-C++ mode. This removes 47 shim lines and lowers
the exact shim budget to 15,732.

The Rust-mode DAG proposer drops its unread `getProposedBlocksCount` facade and compatibility counter. The native proposer
session already owns the accepted add-report outcome, while C++ retains success/failure logging without mirroring an
unobserved cumulative count. Pure-C++ retains the original getter and counter. This lowers the exact shim budget to 15,723.

Static closeout removed two pillar helpers orphaned by the whole-`PeriodData` validation contraction and their now-unused
byte-slice adapter. The live single-vote and sync-bundle executors retain their owned native validation paths and error
mapping; no public or executor boundary changes. PBFT vote logging now records the hash and weight before transferring
the vote into its compatibility vector, and the runtime snapshot step has an unambiguous local name. These shim-only
cleanups, including removal of the helpers' stale public declarations, lower the exact shim budget to 15,654 and leave
the untouched pure-C++ reference implementation unchanged.

That helper deletion also made the standalone `pillar_vote_inspect` CXX function and `PillarVoteInspection` carrier
test-only, so both are retired instead of entering the allowlist. Their bridge-wrapper and C++ adapter tests are deleted
with the boundary; the four native Rust inspection tests continue to cover signer recovery, invalid signatures,
out-of-range recovery identifiers, and malformed RLP. Production single-vote and bundle admission still invoke the same
native inspection logic internally. This lowers the exact bridge budget to 21,424 lines, CXX functions to 368, and CXX
carriers to 311.

Closeout validation after these final contractions passes the fast rewrite gate, all 71 Rust bridge tests, all 1,140
native consensus tests, all 50 `rust_consensus_tests`, all nine Rust storage bridge tests, Rust-enabled and pure-C++ `app`
builds, six DAG tests, 13 DAG-block tests, 37 transaction-manager shim tests, PBFT-chain and PBFT-chain-shim suites,
focused PBFT-manager, pillar, vote-leader, RPC query, PBFT sync wire-encoding, node-sync, insufficient-vote, and next-vote
cases, large-block and transaction-bearing sync, a Rust-enabled `taraxad` build/version smoke, both inventory modes, the
storage-boundary guard, and whitespace validation. Aggregate vote/pillar/network binaries retain the documented
same-process RocksDB-lock/timing baseline; their affected cases pass in isolated processes.

The repo-wide `check-static` gate was also exercised. It remains non-green on the existing broad
`useStlAlgorithm`/legacy-shadow baseline and an `accessMoved` finding in the untouched pure-C++ PBFT manager. All
slice-owned actionable findings are resolved: Rust-mode PBFT logging reads the vote before moving it, the runtime step no
longer shadows the snapshot value, and orphaned pillar definitions, declarations, and their byte adapter are absent.
Independent follow-up review reports no remaining findings.

Five-node full-sync validation exposed a single-member bundle rejection ambiguity. Native preflight correctly returned
one rejection decision with application effect id zero; because the input also contained one vote, C++ mistook equal
vector lengths for successful preflight and requested nonexistent effect zero. Success now requires one accepted,
nonzero application id for every member. Each vote bundle also carries its unique packet id into native effects and
drains only that source within the locked transport lane.

The same executor now represents real native bundle cancellation exactly. Rust records consumable tombstones only for
admission ids deliberately removed after a bundle member terminates aggregation; C++ accepts `cancelled` only with that
proof and preserves hard failures for arbitrary, consumed, and single-vote missing ids. Tombstones are cleared per lane
at the next bundle ingress, bounding abandoned-session memory. Focused native tests cover ordered admission, exact
cancellation, one-shot tombstone consumption, and unknown-id rejection. This narrow tarcap executor proof adds one CXX
function and 12 bridge lines, setting the exact budgets to 369 functions and 21,436 lines. The full-sync test reaches its
passing assertions after the fix, emits no missing-effect diagnostic, and exits cleanly in the final isolated rerun. The
intermittent Boost logger TLS teardown abort remains an aggregate-suite environment baseline rather than a failure of
this route.

The next `CRW-12` ownership cut moves current-certificate validation and admission out of both C++ managers. Native
`PbftService` now preflights the complete canonical vote bundle and all fallible FinalChain facts before mutation, owns
the legacy strict-VRF sampling policy, and drives durable admission through an exact-id resumable session. Rust pauses
before each ordered signing/transaction-pool slashing effect, validates and acknowledges the executor report before
admitting the next vote, retains an unadvanced acknowledgement across failures, and exposes weighted bytes only after
the terminal native `2t+1` decision. The
period-data pop path keeps current certificates as `PbftCertVoteRlp` carriers until that native decision, so C++
materializes `PbftVote` objects only at the accepted finalization boundary. The standalone compact-fact validator, its
three carriers, `VoteManager::validateSyncedCertVoteBundle`, and `PbftManager::validatePbftBlockCertVotes` are deleted;
one command-shaped session endpoint replaces the old validator export, holding the CXX function count at 369 while
lowering the exact starting budgets to 21,435 bridge lines, 15,616 shim lines, and 310 carriers. Exact-session abort on
C++ unwind prevents stale admissions from wedging later sync. Signing and transaction insertion remain explicit executor
leaves, and the untouched pure-C++ implementation remains the reference path.
Validation covers all 1,142 native consensus tests, all 71 bridge tests, 50 Rust consensus C++ tests, the focused PBFT
manager executor case, the isolated large-PBFT-block network sync, the fast rewrite gate, and the exact inventory guard.
The broader five-node smoke still stops at the previously tracked `FinalChain::prune` Rust shim stub; Python integration
setup remains unavailable in the image because PEP 668 blocks system installation and neither `virtualenv` nor `pytest`
is installed.

The following `CRW-12` validation cut moves immutable PBFT extra-data policy into the existing native block-validation
planner. C++ supplies only hardfork-required, extra-data-present, embedded-pillar-hash-present, and pillar-period facts;
Rust rejects Ficus presence and pillar-hash shape mismatches before requesting pillar or DAG executor work. The retired
extra-data executor ordinal remains reserved for ABI stability and fails closed if reported by a stale session. The
`validatePbftBlockExtraData` and `validateFinalChainHash` manager facades are deleted; the retained executor loop calls
the existing native FinalChain state-root leaf directly. No CXX function or carrier is added, bridge lines fall to
21,431, and PBFT-manager shim lines fall to 15,533. Native planner tests cover both mismatch dimensions, retired-code
stability, and stale-report rejection; the bridge conversion test covers the new immutable fields. The untouched
pure-C++ manager remains the reference path.
Validation passes all 1,217 native consensus/bridge tests, all 50 Rust consensus C++ tests, the rebuilt null-anchor
PBFT manager admission case, the fast rewrite gate, and the exact inventory guard. The focused state-root manager case
still stops before block validation at the existing missing latest-block FinalChain account snapshot boundary.

The next `CRW-12` ownership cut moves candidate DAG preparation and cache authority into native Rust. The composed PBFT
and DAG/transaction roots enforce legacy next-period/current-anchor order availability, canonical order hashing, the
previous-PBFT-pivot GHOST divergence rule, and `U256` gas-limit comparison. Rust retains ordered canonical block RLPs
by anchor, then resolves de-duplicated transaction RLPs from the live non-finalized sidecar at finalization time;
queue-only, pending, finalized, and missing transactions remain omitted. Certified-block C++ materialization occurs at the retained finalization/EVM
edge, while the existing native finalization action clears and proves the cache count. Three cache metadata exports are
replaced by two task operations using existing carriers, lowering CXX functions to 368, bridge lines to 21,428, and shim
lines to 15,508 with carriers unchanged at 310. The separate DAG-weight executor check is retired at reserved code 6;
the stable `checkBlockWeight` C++ helper remains solely because an unchanged manager parity test calls it directly.
Native and bridge validation passes 1,223 tests, Rust consensus C++ validation passes all 50 cases, and the focused
null-anchor manager case passes. The isolated overweight manager case still stops earlier at the tracked missing
latest-block FinalChain account snapshot boundary; its direct helper assertion passes before that unrelated stop.

The following `CRW-12` cut moves proposal-time DAG-order execution into the composed native PBFT and DAG roots. Rust
captures canonical order under the DAG lock, loads ordered block hash/gas facts after releasing it, preserves checked
gas clipping and closest-anchor recomputation, and revalidates the pending cursor generation/period/anchor after
releasing the manager lock for DAG work. C++ now receives only a terminal
build/skip command before the retained signing/materialization boundary. The request/report loop, two proposal DAG fact
carriers, and one net CXX function are deleted, lowering bridge lines to 21,381, shim lines to 15,485, CXX functions to
367, and carriers to 308. Native and bridge validation passes 1,228 tests; the rebuilt proposal/broadcast manager case
passes through the new native order path.

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
| DAG proposer | `dag/dag_block_proposer.hpp`, `dag/dag_block_proposer.cpp`, `dag_block_proposer_shim/*` | 576 lines | `partial` | Standalone C++ executor facade with Rust proposer session | The Rust-mode overlay is a self-contained facade: feature-on builds exclude the untouched original source and contain no `DagBlockProposerOld` scaffold, while pure-C++ builds retain the original implementation. C++ still owns thread/network lifecycle, temporary signing, live add-block execution, and live network throttle checks. Transaction packing enters the native owner-bound `TransactionPackingService`, and Rust owns proposer eligibility status decisions, atomic DAG observation/revalidation, legacy VRF input bytes, historical sortition selection, deterministic tip-selection policy, production proposal timestamps, canonical signed block RLP finalization, the legacy `selectDagBlockTips` compatibility surface through storage-backed Rust runtime planning, and the ordered `proposeDagBlock` session for skip reasons, transaction-pack command selection, transaction-pack throttle reporting, runtime-derived VDF wait/cancel and stale-proof decisions, add-block completion outcome, missing VDF input status, and retry-cursor updates. C++ no longer independently reads DAG or sortition storage or echoes those facts back to Rust; it collects only requested external FinalChain facts, then consumes Rust-selected parameters solely as an asynchronous VDF executor instruction. The proposer hands signed block RLP plus transaction payloads to `DagManager` instead of materializing `DagBlock`/`Transaction` objects locally. Remaining proposer gaps are explicit executor boundaries: temporary C++ signing, live add-block side effect execution, and worker/network lifecycle ownership. |
| Sortition params | `dag/sortition_params_manager.hpp`, `dag/sortition_params_manager.cpp` | 331 lines | `rust-owned` | Native `SortitionService` inside the DAG/transaction root; no Rust-mode C++ manager facade | Deterministic efficiency/threshold runtime state and persistence route to `rustaxa-consensus::sortition` and native `rustaxa-storage` in master Rust mode. `SortitionService` restores the manager before publication and owns its mutex and poison policy; the full application root requires this capability structurally. PBFT finalization uses a two-phase contract: preview the Rust threshold transition without publishing live state, persist any emitted `SortitionParamsChange` inside the primary Rust-owned finalization batch, then commit the live Rust runtime only after storage succeeds and validate the emitted change matches the preview. The standalone Rust handle/factories, facade-owned box, optional bridge field, unavailable branches, capability probe, C++ facade/shim, direct CXX operations, and facade-only tests are deleted. The storage overlay retains only the canonical `SortitionParamsChange` RLP codec required by the stable storage API. The untouched legacy class and `sortition_test` are pure-C++-reference-only. |
| PBFT chain | `pbft/pbft_chain.hpp`, `pbft/pbft_chain.cpp`, `pbft_chain_shim/*` | 259 lines | `rust-owned` application state; C++ facade retained | Native `rustaxa-consensus::pbft_chain::PbftChainService` behind a C++ compatibility view | The CXX-free native owner holds startup restore/default initialization, storage lifetime, the sibling `RwLock`, head projection/update, block lookup, and next-block validation. Native `PbftService` embeds that owner; production `App` and Rust-mode tests share the full service through the thin CXX adapter between manager and chain facades. The chain-only constructor/factory are deleted. Cross-domain finalization and leader selection temporarily borrow native guards until the complete PBFT owner moves. C++ retains JsonCpp formatting and temporary `PbftBlock` materialization. Feature-on builds import or compile no `PbftChainOld`; pure-C++ builds retain the untouched original implementation. |
| Proposed blocks | `pbft/proposed_blocks.hpp`, `pbft/proposed_blocks.cpp` | 178 legacy lines | `rust-owned` application state; no Rust-mode C++ facade | Native `rustaxa-consensus::proposed_blocks::ProposedBlocksService` with PBFT-manager boundary materialization | The CXX-free native owner holds restore, membership, compact pivot metadata, validation flags, canonical RLP payloads, storage-first publication, atomic stale-period cleanup, storage lifetime, and the sibling `RwLock`; native behavioral tests cover those contracts. Native `PbftService` embeds that owner, and the PBFT manager bridge adapter calls publication, lookup, mark-valid, and snapshot operations directly before materializing `PbftBlock` only at validation/network boundaries. Period advance performs combined vote/proposal cleanup and manager-period publication inside the native service, so no cleanup result crosses CXX. The standalone bridge handle/factory, C++ facade/mutex/shim, facade-only operations/carriers/tests, and combined-cleanup behavioral relay are deleted. Temporary `DbStorage` compatibility uses the existing typed PBFT storage-query handle rather than free functions over `BridgeStorage`; tentative wallet candidates use an isolated Rust-local batch lookup. The untouched original class remains pure-C++-reference-only. |
| Period data queue | `pbft/period_data_queue.hpp`, `pbft/period_data_queue.cpp`, PBFT service queue API | 168 lines | `rust-backed` | Rust payload and metadata owned by the PBFT application service | Admission rules, encoded period payloads and peer identities, block-link/reward/pillar/cert-vote metadata, transaction metadata and payloads, previous-cert metadata, processable-size/period tracking, pop decisions, cleanup, and chain-head composition route through the native service. The standalone queue CXX handle, shim overlay, module flag, C++ payload deque, peer sidecar, chain-fact inputs, CXX clear operation, and Rust-mode `PbftManager::periodDataQueuePush` facade are retired. Tarcap and database replay retain exact encoded `PeriodData` bytes and submit them through the PBFT-root queue adapter; C++ materializes compatibility vote/transaction/`PeriodData` objects only after pop at remaining executor boundaries. |
| PBFT manager | `pbft/pbft_manager.hpp`, `pbft/pbft_manager.cpp`, `pbft_manager_shim/*` | 3267 lines | `partial` | Native PBFT application service plus C++ lifecycle/executor facade | Rust-enabled builds exclude the untouched upstream manager source and expose only the shim-owned `PbftManager` facade. Native `PbftService` and its sibling services own manager scalar state, daemon/action/session cursors, period-data metadata, sync admission, proposal planning, block validation, transition persistence, PBFT chain lifetime/state, proposed-block state, and verified-vote state; `BridgePbftService` is a one-field CXX adapter. Production construction validates slashing configuration, restores every storage-backed sibling before publication, and remains bootstrap-gated until C++ replay/restart work completes. Authoritative leader selection uses a service-owned vote/proposed/chain snapshot plus fingerprint revalidation around the external C++ block validator; no separate proposal-vote snapshot, per-vote proposed lookup, or C++ chain callback remains. Vote admission commits any required progress batch under the vote lock before publishing the transition and restores a bounded checkpoint on failure. Period advance cleans vote and proposed-block state through one storage-first service action. Finalization drains and validates the chain mutation internally; C++ executes only the remaining typed external effects for FinalChain/EVM, DAG, network, timers, signing, events, and compatibility materialization. |

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
runtime now records accepted dynamic-lambda stage outputs in its snapshot, and transition/sleep consumers read that
native lambda directly. Finalization dynamic-lambda planner inputs no longer read `rounds_count_dynamic_lambda_` /
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

| Verified votes | `vote_manager/verified_votes.hpp`, `vote_manager/verified_votes.cpp`, `rustaxa-consensus::pbft_vote_runtime` | 384 legacy lines | `rust-owned` | Native PBFT verified-vote service called directly by the VoteManager overlay | Unique-voter checks, voted-value weight aggregation, 2t+1 block mappings, period cleanup, round t+1 markers, admission, validation, snapshots, reward selection, persistence, and egress planning route directly to the shared native `PbftVerifiedVotesService`. It owns storage lifetime, atomic restoration, `PbftVoteAdmissionRuntime`, and its mutex. The Rust-mode C++ facade, shim directory, facade-only CXX operations/carriers, and facade test are deleted. C++ materializes transient `PbftVote` objects only at retained VoteManager network, signing, slashing, and FinalChain executor boundaries through stable carrier-only view declarations. Rust-enabled builds do not import or compile `VerifiedVotesOld`; the untouched original remains pure-C++-reference-only. |
| Vote manager | `vote_manager/vote_manager.hpp`, `vote_manager.cpp`, `vote_manager_shim/*` | 1145 lines | `partial` | Rust domain for validation/aggregation/storage/generation; C++ network/live sidecar shell | The Rust-mode `VoteManager` overlay routes admission, validation, replay protection, vote presence and snapshots, proposal/reward selection, period cleanup, next-round detection, 2t+1 lookup, persistence, generation, sortition screening, and optimized egress planning directly through the shared native `PbftVerifiedVotesService`. Rust owns canonical vote inspection, calculated weight, replay mutation, verified-vote state, retained weighted payloads, threshold decisions, required progress persistence, reward selection, and ordered egress plans. C++ materializes temporary `PbftVote` objects only at retained FinalChain/key-manager, signing, slashing-transaction, tarcap/network, event/logging, and public compatibility boundaries. The standalone verified-votes facade and its CXX operations are deleted; stable carrier-only views live with this overlay. The feature predicate selects the overlay and excludes legacy `vote_manager.cpp`, while pure-C++ keeps the untouched upstream implementation. Depends on FinalChain DPoS, VRF, slashing, storage, and network. |
| Pillar chain manager | `pillar_chain/pillar_chain_manager.hpp`, `pillar_chain_manager.cpp`, `pillar_chain_manager_shim/*` | 427 lines | `partial` | Standalone Rust-mode overlay shell over PBFT-service-owned pillar state | Rust production exposes the shim-owned manager facade without importing or compiling `PillarChainManagerOld`; the now-unreferenced legacy `PillarVotes` implementation is also excluded, while pure-C++ builds retain both untouched implementations. Native `PbftService` and its `PillarChainService` sibling own canonical current/latest-finalized snapshots, current validator vote-count history, pillar-vote relevance/inspection/recovered-voter uniqueness/insertion, sync-bundle apply, block creation/linkage planning, PBFT-facing finalization persistence/cleanup, and composed Rust FinalChain DPoS reads behind a pillar-specific readiness gate and sibling mutex. Production injects the thin CXX service adapter into the manager; the deleted standalone runtime survives only as historical tracker terminology. The Rust-mode FinalChain shim supplies bridge root/epoch reads from committed `StateAPI` bridge-contract calls for the finalized request block instead of throwing, and returns zero when the configured bridge contract has no committed code. C++ retains temporary `PillarBlock`/`PillarVote`/`PeriodData` materialization, signing, event emission, network requests, bridge root/epoch execution, and compatibility publication. |
| Transaction queue | `transaction/transaction_queue.hpp`, `transaction/transaction_queue.cpp`, `rustaxa-consensus::transaction_queue` | 501 legacy lines | `rust-owned` | Native Rust state inside `TransactionService` | The standalone C++ overlay, `BridgeTransactionQueue`, bridge module, feature flag, and shim test are retired. Native `TransactionService` exclusively owns deterministic queue metadata, payloads, ordering, replacement/demotion, expiry, purge, limits, gas threshold, known-cache, and overflow/drop state in Rust production mode. Rust builds exclude the untouched legacy C++ queue source; direct C++ queue cases remain pure-C++ reference tests. |
| Transaction manager | `transaction/transaction_manager.hpp`, `transaction/transaction_manager.cpp`, `transaction_manager_shim/*` | 837 lines | `rust-backed` | Native `TransactionService` plus C++ materialization/orchestration shell | Rust mode now uses a standalone `TransactionManager` overlay and does not compile, inherit, or construct `TransactionManagerOld`; the original header/source are clean reference-only code. The facade preserves public/shared-pointer identity and owns only locks plus the classified FinalChain/EVM, thread-pool, event, logging, and object-materialization shell. Native `TransactionService` restores `TrxCount` and finalized gas-price history and remains authoritative for queue metadata/payloads, known-cache state, non-finalized/recently-finalized sidecars, gas-estimation cache policy, transaction count, gas-price policy, persistence, and transaction locking/poison policy. Its embedded native `TransactionPackingService` owns the private packing mutex and poison policy, compatibility-or-DAG owner identity, canonical candidate/RLP snapshot, sharding, planner accounting, pending-estimate protocol, selected order, stop state, typed effects, actual-demotion acknowledgement, and selective abort. `BridgeDagTransactionService` composes that native owner with private DAG state behind a sibling mutex; the two C++ facades never pass internal handles between them. Production pool bids derive the inclusion floor directly from the service-owned queue and proposal gas limit; no queue scalar crosses C++ before the oracle applies its configured floor. `packTrxs` snapshots queue/cache facts under the transaction lock, calls the native packing owner, releases every lock for C++ EVM work, then applies typed demotion/cache effects and materializes only final accepted outputs. The complete transaction read-task family locks inside native services and returns owned views, status/gas facts, and declared/cache/external-EVM estimation decisions; the bridge performs only carrier projection. `estimateTransactionGas` and `estimateTransactions` execute EVM work and report cache-store results through the retained leaf adapter. `isTransactionKnown` routes through a hash-only Rust query that derives queue-known and sidecar membership from Rust state, and public `insertTransaction` uses one typed Rust admission command that owns known-fast-path precheck, verification decisioning, latest FinalChain account sourcing, public status/message mapping, finalized-location mapping, queue mutation, and explicit event/log shell intents. Rust legacy transaction envelope inspection provides hashes, senders, nonces, gas fields, costs, intrinsic-gas coverage, signature validity, and canonical RLP payloads for verification, admission, packing, DAG persistence, finalized-status updates, recovery, and proposal-period lookup. Rust owns transaction storage batches, sidecar mutation, count updates, queue erasure, recovery validation, finalized-status cleanup, queue expiry, and account-nonce-based purging before returning typed receipts for the remaining C++ log/event sinks. Remaining C++ is classified shell work: locks, public transaction object materialization, event/log dispatch infrastructure, EVM gas-estimation execution, public transaction construction, and lifecycle wiring. Remaining bridge debt is the temporary FFI-shaped mutation, admission, cache-store, queue-finalization, sidecar, and compatibility-packing adapter family over a short native guard. |
| Gas pricer | `transaction/gas_pricer.hpp`, `transaction/gas_pricer.cpp`, `rustaxa-consensus::gas_pricer` | 171 legacy lines | `rust-owned` | Native gas oracle composed into private transaction service state | Finalized-block history restoration, live finalized-block gas-price updates, minimum-price flooring, percentile bid selection, and queue-aware pool pricing are owned by private transaction state inside the App-owned `BridgeDagTransactionService`. App finalization/metrics, Eth RPC, GraphQL, and the slashing overlay use `TransactionManager::updateGasPrice` / `gasPriceBid` directly. `CRW-14` deletes the Rust-mode `GasPricer` facade and shim directory. The untouched original implementation and test remain pure-C++-reference-only. |
| Pillar block/votes | `pillar_chain/pillar_block.hpp`, `pillar_chain/pillar_votes.hpp`, matching `.cpp` files | 627 lines | `rust-backed` for vote aggregation | Native Rust domain behind the manager runtime | `rustaxa-types::pillar` mirrors `PillarBlock`, `ValidatorVoteCountChange`, `PillarVote`, `PillarBlockData`, optimized pillar-vote bundles, and current pillar data RLP/Solidity/hash shapes. `rustaxa-consensus::pillar_votes` owns verified-vote uniqueness, weighted aggregation, deterministic threshold selection, cleanup, inspection, and sync-bundle planning. Rust production excludes the unused legacy `pillar_votes.cpp`; C++ retains only temporary `PillarVote` sidecars and compatibility materialization through the manager facade. Vote signing and JSON/RPC materialization remain later slices. |
| Pillar manager | `pillar_chain/pillar_chain_manager.hpp`, `pillar_chain/pillar_chain_manager.cpp`, `pillar_chain_manager_shim/*` | 629 lines | `partial` | C++ compatibility/executor facade over PBFT-service-owned pillar state | Native `PbftService` restores and owns the single production `PillarChainService` behind its independent readiness gate and sibling mutex; bridge adapters compose pillar validator facts with borrowed Rust FinalChain state. Production and Rust-mode tests use the full shared service through the one-field CXX adapter; all pillar-only construction is deleted. The full overlay retains bridge root/epoch execution, network transport, signing, C++ block/vote materialization, storage-compatible payloads, events, and finalization effects as explicit executor boundaries. Pure-C++ builds retain the untouched legacy manager and `PillarVotes` implementations. |
| Rewards stats | `rewards/block_stats.*`, `rewards/rewards_stats.*` | 407 legacy lines | `rust-owned` | Native Rust FinalChain rewards domain; untouched C++ is reference-only | Rust-mode production FinalChain owns one long-lived rewards-stats runtime for native and external-EVM finalization. External-EVM execution reports prepare a request/head/generation-bound Rust plan, expose canonical distribution-stat RLP only to the C++ `StateAPI` adapter, attach cache mutation internally to atomic FinalChain publication, audit durable rows, and reload runtime state after publication/recovery. `CRW-14` deletes the standalone Rust-mode `rewards::Stats` shim, `BridgeRewardsStatsRuntime`, bridge batch relay, carriers, and compatibility tests. Native FinalChain stages account mutation until finalization storage commits, credits transaction-fee and minted rewards, persists Aspen supply/yield and DPoS reward state, executes supported claims, and routes receipt logs/blooms in Rust. C++ retains temporary `BlockStats` decoding solely at the external `StateAPI::distribute_rewards` boundary; the original `rewards::Stats` implementation remains only in pure-C++ reference builds. |
| Slashing manager | `slashing_manager/slashing_manager.*`, `slashing_manager_shim/*` | 102 lines | `partial` | Native Rust slashing service behind a C++ executor facade | Double-voting proof eligibility, Magnolia vote-A-period admission, canonical proof hash/cache, first funded submitter selection, contract address/gas/value envelope, and calldata construction route through native `SlashingProofService` under the master Rust production composition. The service owns planner configuration, duplicate-cache state, and its mutex; native `PbftService` holds the capability and `BridgePbftService` only converts evidence/status DTOs. `VoteManager::addVerifiedVote` submits Rust-normalized unweighted evidence through that canonical service, and the standalone `BridgeSlashingProofPlanner` handle/factory are deleted. The facade no longer imports or compiles `SlashingManagerOld`; pure-C++ configurations retain the untouched original. C++ keeps FinalChain account reads, the `TransactionManager` gas-price query, transaction signing/insertion, and the live-vote compatibility overload. Rust FinalChain uses legacy-compatible inclusive Magnolia/Cacti activation, including activation zero from genesis. |
| Key manager | `key_manager/key_manager.*` | 55 lines | `cpp-owned` | C++ initially | Small wallet/secret wrapper; not on critical rewrite path. |

TransactionManager DAG payload note: proposed DAG blocks that already carry canonical transaction RLP payloads now call
`saveTransactionPayloadsFromDagBlock()`, so Rust inspects and persists DAG transaction facts without first constructing
live `Transaction` objects. Live transaction materialization remains for public reads, EVM/gas execution, and network
gossip compatibility.

Current TransactionManager packing boundary: native `TransactionPackingService` owns the active session, mutex, owner,
candidate/RLP snapshot, sharding, planner, estimate ordering, selected output, and stop/abort policy, so C++ receives a
pack callback only when an EVM estimate is required. The bridge-owned transaction runtime snapshots queue/cache facts,
applies typed queue demotion and cache-insert effects, and acknowledges only demotions that changed the live queue. C++
cleans failed sessions through a Rust abort entrypoint and materializes only final selected RLP payloads. The shim-only
public `TransactionQueue::demoteToNonProposable` method remains removed; direct queue, sidecar/cache, storage, and
shared-batch ownership are the next native-root debt.

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
| `SortitionParamsManager` | params lookup, DAG efficiency, interval recalculation, cleanup | Native `SortitionService`, config, period-data facts, VDF params | native Rust sortition tests, `rust_consensus_tests`; pure-C++ `sortition_test` | Deleted in Rust mode; private native service state owns deterministic behavior, while the untouched class remains reference-only |

### PBFT

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PbftChain` | head/hash/size reads, block lookup, update head, block validation | `DbStorage`, `PbftBlock` | `pbft_chain_test`, `pbft_manager_test`, `full_node_test` | Early Rust-backed PBFT state slice |
| `ProposedBlocks` | push, mark valid, lookup, presence, cleanup, old-block checks | Native PBFT service, storage compatibility, `PbftBlock` boundary materialization | native proposed-block tests, `rust_consensus_tests`, `pbft_manager_test`; pure-C++ PBFT coverage | Deleted in Rust mode; native service owns behavior and PBFT manager materializes only boundary objects |
| `PeriodDataQueue` | push/pop/clear/size/period/last block/old-data cleanup | `PeriodData`, `PbftVote`, peer `NodeID` | `rustaxa-consensus` period-data queue tests, `rustaxa-bridge` PBFT service queue test, `pbft_manager_test`, full-node sync tests | Private manager state within native `PbftManagerService` owns compact block links, FinalChain hashes, extra-data presence, reward/pillar/cert vote payloads, transaction identities and payloads, previous-cert flags, and pillar-presence facts; C++ retains live queue payload materialization and peer/lifecycle execution |
| `PbftManager` | lifecycle, state machine, proposal generation, period/round/step, DPoS counts, sync queue, block validation, gossip, finalization, dynamic lambda | nearly every consensus subsystem | `pbft_manager_test`, `vote_test`, `pillar_chain_test`, `full_node_test`, Python integration | Full Rust-mode overlay with Rust-owned scalar runtime, daemon-tick/action cursors, sync-period admission, proposal selection, transition persistence, finalization planning/storage apply, bounded resume classification, and compact sync queue facts through PBFT block-fact/reward-vote/transaction/pillar-presence/pillar-vote-RLP metadata; C++ remains the live executor for network, FinalChain/EVM dispatch, object materialization, timers, and compatibility side effects |

### Votes and Eligibility

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `VerifiedVotes` | Pure-C++ reference API only; Rust mode uses native vote insertion, unique voter tracking, step/round/period lookup, 2t+1 voted blocks, and cleanup directly | `PbftVote` at retained executor boundaries | Native verified-vote tests, `vote_test`, `pbft_manager_test` | Deleted Rust-mode facade; continue contracting VoteManager materialization around the native service |
| `VoteManager` | vote validation, generation, reward votes, two_t_plus_one thresholds, VRF sortition, current period/round | `FinalChain`, `PbftChain`, `KeyManager`, `SlashingManager`, `DbStorage`, `Network`, VRF | `vote_test`, `pbft_manager_test` | Rust owns validation, verified-vote aggregation, replay protection, threshold caching, local generation/sortition planning, composed FinalChain/key sourcing, reward selection, canonical payload construction, and persistence; C++ retains temporary live vote/public materialization, signing, network wrapping/gossip execution, and lifecycle wiring |
| FinalChain DPoS ports | eligibility, vote/stake totals, supply/yield, validator/delegator/reward/undelegation and slashing reads plus all current-ABI mutations | Rust FinalChain snapshots and the external StateAPI/EVM leaf executor | `rust_consensus_tests`, `final_chain_test`, `rpc_test`, `pbft_manager_test`, `state_api_test`, proposer tests | Complete for current-ABI DPoS/slashing behavior, typed state, receipts, logs, rewards, supply, persistence, restart, and composed consensus fact reads. Historical databases without complete Rust snapshots remain an explicit fail-closed replay/rebuild boundary. `CRW-E01` is now ready to contract orchestration around the retained concrete executor; that later authorization does not alter the completed CRW-10 evidence. |

### Transactions

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| Transaction queue runtime | insert/erase/order/group/contains/size/purge/known tx/min gas price | Rust FinalChain and transaction facts through the private transaction state in `BridgeDagTransactionService` | native Rust queue/runtime tests, `transaction_manager_shim_test`, pure-C++ reference queue cases | Private native Rust queue owned by the application service; no standalone C++ production facade |
| `TransactionManager` | verify/insert/pack/get/finalize status/non-finalized recovery/gas estimation | `DbStorage`, `FinalChain`, thread pool, `DagBlock`, state API | `transaction_test`, `transaction_manager_shim_test`, `dag_block_test`, `pbft_manager_test`, `full_node_test` | Rust owns packing, DAG persistence, finalized status, storage/sidecar lookup, recovery, finalized filtering, admission, queue payloads, mutation reports, and event intent; C++ retains the classified EVM gas-estimation executor, public transaction materialization, event/log dispatch, thread/lifecycle wiring, and compatibility orchestration |
| `GasPricer` | Pure-C++ reference gas price reads/calculation only; Rust mode uses `TransactionManager::gasPriceBid` / `updateGasPrice` | native transaction runtime; legacy storage/transaction manager only in pure-C++ reference mode | native `gas_pricer` tests, `transaction_manager_shim_test`, transaction/full-node tests; pure-C++-only `gas_pricer_test` | Deleted Rust-mode facade and flag; native transaction service owns the oracle and operation-shaped callers use its existing manager boundary |

### Pillar, Rewards, Slashing

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PillarBlock` / `PillarBlockData` | RLP, hash, JSON, Solidity encode/decode, validator vote-count deltas | hashes, state API data | `pillar_chain_test` encoding/finalization cases | Rust domain and codec parity |
| `PillarVotes` | vote uniqueness, threshold accumulation, above-threshold selection, cleanup | `PillarVote` | service-owned pillar-vote tests in `rustaxa-consensus`/`rustaxa-bridge`; focused `pillar_chain_test` and `pbft_manager_test` cases | Private pillar state in native `PillarChainService` owns uniqueness, weighted aggregation, threshold selection, payload retention, and cleanup; C++ retains signing, tarcap/event execution, live vote sidecars, and public materialization |
| `PillarChainManager` | create block, validate/generate/finalize votes, current/finalized block state | `FinalChain`, `DbStorage`, `Network`, `KeyManager` | `pillar_chain_test`, `full_node_test` | Full Rust-mode overlay with Rust pillar-vote relevance/inspection/recovered-voter insertion, block planning, and composed FinalChain DPoS reads; C++ retains bridge root/epoch facts, signing, compatibility payload materialization, event/network effects, live sidecars, and finalization orchestration |
| Rewards statistics / legacy `rewards::BlockStats` | per-block stats, interval recovery/processing/cleanup | Native FinalChain runtime, PBFT/vote/DAG facts, Rust storage; external `StateAPI` executor | Rust rewards-stat and FinalChain unit tests, `rust_consensus_tests`, full-node reward paths; pure-C++-only `rewards_stats_test` | Rust produces legacy-compatible `BlockStats` RLP and FinalChain owns planning, cache mutation, durable audit, and reload for native and external-EVM publication. The external adapter temporarily materializes C++ `BlockStats` for `StateAPI`; removing that accepted boundary is tracked under `CRW-E01`. The standalone Rust-mode `rewards::Stats` and `BridgeRewardsStatsRuntime` compatibility surfaces are deleted. |
| `SlashingManager` | double-voting proof submission | native `SlashingProofService`, `FinalChain`, `TransactionManager` | Rust slashing service/planner tests and `StateAPITest.slashing` end-to-end submission/jailing coverage | Native Rust owns deterministic planning, evidence normalization, duplicate protection, and locking; C++ retains classified account/gas lookup, transaction construction/signing/insertion, and logging executor boundaries |

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
- Native Rust tests for DAG verifier behavior plus bridge boundary tests for
  production DAG manager/proposer conversion and external-leaf wiring. Landed
  in `rustaxa-consensus` and `rustaxa-bridge`.
- Proposer-session tests cover runtime-derived observations, stale external-fact rejection, missing periods,
  out-of-order reports, runtime-derived VDF cancellation/resume, and independent wallet sessions.
- Rust-mode C++ production coverage through all `DagManager` cases in `dag_test` and through `dag_block_test`.
- Direct `Dag`/`PivotTree` regression cases remain pure-C++ reference coverage; Rust mode intentionally has no
  standalone C++ graph facade.
- Rust `verifyBlock` coverage for tip count/uniqueness, missing proposal-period mapping, expired block, transaction
  availability, VDF/DPoS authorization decision ordering, and gas-policy decisions. Behavioral coverage lives in
  `rustaxa-consensus`; `rustaxa-bridge` retains boundary conversion and external-leaf wiring coverage. The shim now
  passes an explicit status-coded Rust VDF/DPoS fact envelope instead of encoding separate
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
| Sortition params routing | Native Rust validation plus `rust_consensus_tests`, affected Rust-mode DAG/network coverage, and pure-C++ `sortition_test`; no Rust-mode manager facade test remains |
| PBFT chain/proposed-block/queue routing | Rust validation plus `rust_consensus_tests`, native proposed-block/PBFT service tests, `pbft_chain_test`, `pbft_chain_shim_test`, and relevant `pbft_manager_test` cases; proposed blocks have no Rust-mode facade test |
| Vote aggregation/eligibility | Native verified-vote Rust tests plus `rust_consensus_tests`, `vote_test` (including carrier-contract coverage), relevant `pbft_manager_test`, and DPoS/state API coverage; no Rust-mode verified-votes facade test remains |
| Ingress message inspection/enrichment planning | Rust validation plus message-shape unit tests and C++ parity/golden-vector coverage for each routed ingress message kind; add scheduler or egress-event tests once the network pipeline lands |
| Transaction queue behavior | Native Rust queue/runtime validation plus `transaction_manager_shim_test`; pure-C++ queue-focused `transaction_test` and `gas_pricer_test` preserve legacy reference coverage; run affected DAG/PBFT tests when manager/proposer packing changes |
| Slashing proof planning | Rust byte-level proof-hash and calldata fixtures plus `StateAPITest.slashing`; richer C++ legacy vote/submission transcripts remain useful when available |
| Pillar vote aggregation and sync bundle validation | Rust validation plus service-owned pillar-vote tests in `rustaxa-consensus`/`rustaxa-bridge` and focused `pillar_chain_test`/`pbft_manager_test` cases when manager behavior is touched |
| Pillar/reward behavior | Rust validation plus `pillar_chain_test`, `rewards_stats_test`, and affected full-node tests |
| PBFT manager state machine | Targeted PBFT/vote/DAG tests plus full-node smoke and Python integration coverage as needed; feature-on source/archive audits now prove the original manager and `PbftManagerOld` scaffold are absent. Native manager tests own daemon tick, queue-drain, and state-action session behavior; `rust_consensus_tests` retains CXX bridge coverage for staged sync admission, finalization/external actions and resume, period-advance effects, storage-backed startup restore, and the completed PBFT manager closeout boundary. |
