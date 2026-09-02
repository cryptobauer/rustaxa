# Rust Rewrite Plan

This is the consolidated plan for progressively replacing C++ internals with Rust while keeping the node buildable, testable, and syncable with upstream.

## Objectives

- Keep upstream C++ sync practical through a pure-C++ validation gate.
- Preserve protocol, wire, storage, RPC, and explicitly named external API behavior while allowing Rust-enabled `main`
  to retire internal C++ consensus manager APIs aggressively.
- Move high-value storage and FinalChain paths first.
- Use existing Rust rewrite implementations as aggressively as correctness allows, so each slice moves production routing
  toward the long-term goal of complete Rust ownership rather than rebuilding orchestration in C++.
- Keep Rust APIs type-safe, throughput-conscious, and unit-testable.
- Maintain C++ vs Rust behavioral parity with focused tests and conformance checks.

## Branch and Gate Model

| Branch | Role | Rule |
| --- | --- | --- |
| `upstream-main` | Upstream mirror | Clean 1:1 upstream C++ mirror; no local commits. |
| `cpp-reference` | Validation gate | C++ code plus integration hooks; verify with `RUSTAXA_ENABLE=0`. |
| `main` | Rewrite branch | Primary Rust rewrite branch and future source of truth. |

The normal upstream flow is:

1. Update `upstream-main` from upstream.
2. Merge `upstream-main` into `cpp-reference`.
3. Preserve shim/`#ifdef` structure while updating legacy C++ logic.
4. Build and test `cpp-reference` in pure C++ mode.
5. Merge `cpp-reference` into a temporary sync branch from `main`.
6. Port changed logic to Rust.
7. Validate Rust-enabled mode and merge to `main`.

When Rust feature work on `main` touches C++ intersection files, carry the relevant C++ side back to `cpp-reference`:

```bash
make cpp-intersection-list
make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>
```

## Architecture Direction

Use thin adapters at genuine C++ boundaries and idiomatic Rust composition internally.

Core rules:

- Rust-enabled `main` is a cutover target, not a compatibility replica of the upstream internal class graph. Internal
  C++ consensus manager APIs, constructors, callbacks, events, locks, object-returning methods, and partial feature
  combinations may be changed or deleted after their production callers migrate.
- Compatibility is guaranteed for protocol-visible behavior, canonical encodings, durable data, RPC/public contracts,
  and the explicitly named external boundaries below. A historical C++ type or method is not a compatibility contract
  merely because tests or another internal manager still use it.
- Pure-C++ reference behavior remains available through the untouched upstream implementations and the
  all-Rust-disabled validation route. It does not require Rust-enabled production to expose matching internal classes.
- New shims are exceptional. Before adding one, prove that a named external C++ client cannot use an existing query,
  transport, execution, bootstrap, admin, or signing adapter. A temporary shim must have a normal roadmap issue,
  deletion condition, and owner.
- When a named external client still requires an upstream-owned C++ class, use the overlay shim pattern: header overlay
  plus a standalone facade, with the untouched implementation selected only for pure-C++ mode. Prefer this over
  scattered inline `#ifdef` edits.
- Do not create a full class overlay solely to preserve internal C++ architecture. Migrate internal callers to a native
  application API and delete the class from Rust mode. When an approved overlay is required, before adding behavior to an
  upstream-owned C++ class, create or extend the class overlay (`shims/<class>_shim/include/.../<class>.hpp`), compile
  the legacy implementation as `<Class>Old`, and keep Rust routing, shim-only helper methods, temporary stubs, and
  parity scaffolding in shim-owned files. Do not add Rust-only methods, `ForRust` hooks, bridge includes, or scattered
  `#ifdef` branches to original upstream headers/sources unless the task owner explicitly approves a temporary guarded
  hook. Closeout for touched upstream-owned files should include `git diff upstream-main -- <original C++ paths>`; the
  expected result is empty or an explicitly documented temporary exception.
- Hard rule: when a dependency or subsystem already has a Rust rewrite path, new rewrite work should leverage that Rust
  implementation directly instead of re-centering behavior in C++. Prefer extending Rust crates, bridges, and shim-owned
  Rust handles over adding C++ orchestration or C++ data materialization, unless a concrete blocker is documented and
  accepted by the task owner.
- Tests do not justify production compatibility surface. Move behavioral tests with their native Rust owner; bridge
  tests cover CXX conversion, lifetime, error mapping, and externally observable parity. Test-only CXX exports are
  forbidden unless explicitly allowlisted as conformance boundaries.
- Rust production uses one supported application composition. Granular rewrite flags and partial-service factories are
  migration scaffolding to remove, not configurations that the final Rust architecture must preserve.
- Rust production converges on one native `ConsensusApplication` composition root. The root owns construction,
  restoration, storage-backed consensus services, internal service lifetimes, and cross-service lock ordering. Its
  PBFT, DAG/transaction, FinalChain, vote, pillar, slashing, sortition, rewards, and network-pipeline capabilities are
  private implementation details and must not be exposed as separately constructible or passable CXX handles.
- `ConsensusApplication` is a composition and lifetime boundary, not a service locator. C++ may hold one opaque
  application bootstrap handle and operation-specific external adapters, but it may not fetch internal services,
  borrow mutable consensus state, or pass one internal owner into another manager. Rust task APIs consume canonical
  bytes or explicit domain inputs and return typed leaf effects whose exact results are reported back before native
  state advances.
- Prefer vertical subsystem cutovers that migrate every production caller, move behavioral tests, and delete the old
  Rust-mode manager facade, constructors, handles, carriers, sidecars, and materialization together. A validated slice
  is bounded by one coherent ownership transition, not by a small diff. Do not preserve an obsolete intermediate
  Rust-mode manager topology merely to keep standalone C++ fixtures usable.
- Logging and observability are not architectural blockers for Rust ownership. Do not keep deterministic consensus
  behavior in C++ merely because the legacy implementation logs at that point. Rust planners may return typed statuses,
  telemetry facts, or executor reports that C++ logs temporarily, and logging can be moved, changed, or dropped in a
  later observability cleanup without affecting the ownership decision.
- Before selecting or implementing a rewrite slice, proactively inspect adjacent Rust crates, bridge APIs, shim-owned
  handles, and existing storage/FinalChain/DAG/transaction/vote coverage for reuse opportunities. Prefer connecting the
  new path to those Rust implementations, even if that makes the slice slightly larger, when it reduces future C++
  ownership and keeps behavior on the path to full Rust replacement.
- Network ingress uses operation-specific application pipelines, not a generic shadow byte arena. Each routed packet
  family should cross once with its canonical payload and compact transport facts, then return typed decisions/effects.
  Preserve canonical bytes, decode late, avoid eager C++ object materialization, and retain payloads only when a concrete
  pipeline cursor or queued effect owns them. Do not add an ingestion-success-only CXX call, long-lived payload-id arena,
  or capacity configuration that has no authoritative downstream consumer.
- The arena direction is not a single consensus pipeline. Current tarcap scheduling has three priority lanes, but the
  rewrite should model seven logical data pipelines over those lanes: peer status/sync control, transaction gossip and
  admission, DAG block gossip and admission, DAG sync, PBFT vote and round progress, PBFT chain sync/finalized-period
  intake, and pillar vote/bundle handling. Cross-pipeline impact must stay explicit in typed effects: deep PBFT sync
  filters most traffic, transaction ingress can peer-order block later DAG blocks from the same peer, DAG gaps trigger
  DAG sync, status can trigger PBFT or DAG sync, votes drive PBFT round/finalization progress and slashing, PBFT sync
  feeds the PBFT manager period-data queue, and pillar votes/data affect PBFT period validation.
  The intended stage shape is packet-family ingress -> prefilter -> dispatcher -> pipeline-specific queues -> effect
  executors. `NetworkEvent`, prefilter decisions, dispatcher classification, and ring-buffer allocation belong in the
  network crate or a dedicated pipeline crate. The consensus crate should define only consensus event/effect types such
  as `ConsensusEvent`, `PbftVoteEvent`, `ConsensusEffect`, and opaque ingress payload references used to decode arena
  bytes late. These consensus pipeline types are provisional scaffolding until the first routed pipeline lands; names,
  variants, and payload fields are expected to change as the design is validated.
- Consensus business logic should be expressed as deterministic protocol planners, not as async tasks, actors, or
  workflows that own data. A planner receives a consensus event or command plus explicit borrowed state views/facts and
  returns a protocol plan: validation outcome data, ordered state/write intents, follow-up consensus events, and external
  effects. The plan describes the protocol state transition, but the planner must not perform I/O, spawn Tokio work,
  write storage, send network messages, or mutate another pipeline directly. Runtime workers, actors, Tokio tasks, ring
  buffers, and effect executors may schedule and apply plans at the boundary; they should not hide consensus rules inside
  mailbox-local state.
- Hard rule for Rust-enabled paths: never forward, delegate, or rely on inherited behavior from legacy C++ implementations. Any not-yet-ported API must stay explicit in the shim as a documented stub/no-op/throw until Rust parity lands. If fallback is proposed, require explicit task-owner approval first.
- Hard rule: preserve existing test intent while it still represents target behavior. Do not loosen or rewrite tests to
  accommodate Rust rewrite regressions; fix implementation parity first. C++ tests may be disabled, removed, or retargeted
  when they block retiring legacy C++ behavior, old object materialization, or shim scaffolding, but only after equivalent
  or stronger Rust module coverage exists for the moved behavior. If parity depends on the CXX bridge, add bridge-level
  Rust coverage or a focused Rust-enabled shim test before dropping the C++ test signal, and document why the old C++ test
  no longer represents target Rust-mode behavior.
- Documentation rule: whenever adding or changing rewrite code, document modules, types, and functions as complete units (purpose, inputs, outputs, invariants, and error or edge behavior), not just isolated lines.
- Rust domain modules define narrow ports for required capabilities.
- Infrastructure modules implement those ports over RocksDB, CXX bridges, or test fakes.
- Runtime/bootstrap code wires concrete implementations together.
- Avoid "everything bag" context structs and broad service locators.
- Keep dependency direction one-way: domain -> ports, infra -> port implementations, runtime -> wiring.
- Rust rewrite code should use clearer names than C++ when legacy names are ambiguous, unclear, overly abbreviated, or
  easy to misread. Preserve C++ compatibility at the shim boundary, but prefer descriptive Rust APIs internally.
- Rust modules should use `anyhow` for fallible APIs unless a narrower error type is explicitly needed at a domain
  boundary. Convert lower-level errors into `anyhow::Result` with useful context instead of leaking incidental backend
  details through domain APIs.

Dispatch guidance:

- Use generics/static dispatch for hot loops.
- Use `Arc<dyn Trait + Send + Sync>` only where runtime flexibility is needed.
- Collapse abstractions only when profiling or code clarity justifies it.

## Rust Type and Codec Policy

Introduce Rust domain types by semantic role, not by mirroring C++ class names.

Recommended module grouping:

- `rustaxa_types::pbft`: PBFT block metadata and future PBFT domain objects.
- `rustaxa_types::final_chain`: final-chain block, receipt, log, bloom, and execution-facing models.
- `rustaxa_types::dag`: DAG block and DAG-related models.
- `rustaxa_types::codec`: RLP and other wire/storage encoders and decoders.

Rules:

- Use `#[repr(transparent)]` newtypes for distinct scalar values such as `PbftPeriod`, `BlockNumber`, `TrxPosition`, `Gas`, and hashes.
- Keep domain fields private when invariants matter; expose validated constructors or `TryFrom`.
- Keep domain models separate from wire/storage representations.
- Avoid `TryFrom<&[u8]> for DomainType` when bytes could mean multiple formats; use codec-specific wrapper inputs.
- Decode bytes only when logic needs typed fields.
- Preserve canonical encoded bytes when repeated hashing or persistence would otherwise require re-encoding.
- Keep CXX bridge transport wrappers plain and separate from core domain models.

Each new domain type should document:

- invariants
- canonical encoding shape
- C++ compatibility expectations
- allocation and throughput impact
- DB, codec, and CXX bridge conversion points
- byte-compatibility and malformed-input tests

## Storage Rewrite Plan

### Scope

In scope:

- `DbStorage` public API parity through Rust-backed shims.
- DAG, transaction, period-data, PBFT/vote, pillar, metadata/config/statistics read and write primitives.
- Batch semantics that are needed by active consensus and final-chain paths.

Out of scope for the current wave unless explicitly re-scoped:

- storage migration flows
- light-node / `plugin/light` maintenance and pruning flows
- snapshot lifecycle and recovery APIs
- low-level RocksDB admin APIs unless they block functional parity

### Current Storage Shape

`DbStorage` owns the main RocksDB database under `db/`. `state_db/` is a sibling database used by FinalChain state execution, not a column family inside `DbStorage`.

The main database uses column families grouped by domain:

- schema and metadata: `default`, `migrations`, `genesis`, `status`
- finalized period data: `period_data`, `pbft_block_period`, `dag_block_period`
- pending DAG and transactions: `dag_blocks`, `dag_blocks_level`, `transactions`, `trx_period`
- PBFT runtime and votes
- proposal/sortition/lambda/reward configuration
- pillar chain state
- system transactions
- final-chain indexes and receipts

The central storage lifecycle is pending-to-finalized:

1. DAG blocks and transactions live in pending columns.
2. `savePeriodData` writes the finalized bundle to `period_data`.
3. Reverse indexes map PBFT block hashes, DAG hashes, and transaction hashes back to period/position.
4. Historical reads use indexes plus `period_data`.

This makes `period_data` one of the highest-value Rust rewrite targets.

### Rust-Mode Storage Boundary

Target state: Rust-mode consensus and FinalChain code must not route storage reads or writes through C++.
`rustaxa-storage` is the durable storage owner below the Rust consensus and FinalChain runtimes. The C++ shell may hold
opaque application/query/execution API handles and translate legacy public API views, but it must not collect consensus facts from
`DbStorage`, assemble storage batches, choose storage write ordering, or commit consensus/final-chain rows on behalf of
Rust. Any remaining C++ storage access in Rust mode is temporary migration debt that should be removed by the relevant
subsystem slice.

The only accepted near-term exception is the external EVM boundary: `StateAPI` / `state_db` may remain behind the C++
EVM executor adapter while arbitrary EVM execution is outside Rust. That exception must not be used to read or write
`DbStorage` consensus/final-chain column families.

New Rust-mode C++ storage routes are guarded by `scripts/rewrite_storage_boundary_guard.sh`, which is part of
`make rewrite-validate-fast`. The guard checks newly added C++ lines for direct `DbStorage`, `db_->`, C++ batch, Rust
storage-handle, and column-family usage outside the legacy storage implementation and tests. It also
rejects new direct C++ FinalChain DPoS fact reads from consensus consumers now that those paths have a typed Rust
FinalChain fact port.

Current state: Rust-mode consensus storage ownership is closed for the migrated production routes audited during the
storage migration. `rustaxa-consensus` and the relevant Rust runtimes own storage fact collection, write ordering,
idempotency checks, restart normalization, and batch commit/drop for PBFT finalization, VoteManager persistence,
TransactionManager consensus storage, DAG/proposed-block storage, rewards stats, pillar storage, PBFT-manager scalar and
residual storage, gas-pricer storage, FinalChain fact ports used by consensus, and the FinalChain publication/account
status rows that were moved with the consensus storage work. The post-migration audit found no remaining unclassified
production consensus route that depends on `DbStorage`, direct `getDB()`, public `rustBatchId`, or a bridge-batch
appender as the storage authority.

`ConsensusApplication` now opens the sole Rust-mode storage owner and constructs native FinalChain before restoring the
other consensus siblings. Rust-enabled production does not compile or construct `DbStorage`; the standalone storage
handle, query families, batches, bridge module, C++ overlay, legacy materializers, and compatibility mutexes are deleted.
Public reads use `ConsensusQueryApi`, light-history cleanup is one atomic application task, and storage conformance uses
one versioned production-root transcript without exposing storage authority. The C++ FinalChain executor receives only
its exact `state_db` path leaf.

FinalChain external-EVM publication atomically persists system-transaction payloads with their native indexes and
receipts. Bridge-contract reads and slashing account facts remain explicit concrete-EVM query boundaries.

Remaining accepted compatibility categories are:

- the versioned production-root storage-conformance transcript
- client-oriented RPC, GraphQL, debug, stats, and light-plugin reads through `ConsensusQueryApi`
- app lifecycle and admin operations, including snapshot create/delete/recover/load paths that now throw explicit
  unsupported errors in Rust mode where the migration does not own them
- FinalChain external-EVM, `StateAPI`, bridge-contract, account/code/storage, and publication-query boundaries that
  still sit outside PBFT manager ownership
- temporary materialization of legacy C++ sidecars such as `PbftBlock`, `DagBlock`, `Transaction`, votes, pillar objects,
  API return objects, and network payloads

The storage-boundary guard prevents newly added unclassified C++ storage routes. Existing compatibility references should
be removed only when the caller is replaced by a Rust-owned runtime, query API, fixture, or executor boundary.

### Current Rust Storage Coverage

Rust-mode storage lives in `rustaxa-storage` and is composed only by native `ConsensusApplication`. Legacy
`libraries/core_libs/storage/src/storage.cpp` remains pure-C++-only logic.

Current Rust-backed coverage includes:

- DAG read/index APIs.
- `period_data` primitives and finalized receipt-by-period reads.
- metadata/config/statistics reads.
- PBFT block-hash presence checks and PBFT manager/vote reads.
- pillar reads.
- transaction presence, location, count, retrieval, and finalized-state reads.
- write primitives for DAG, period data, transactions, PBFT manager/votes, pillar, and metadata/config/statistics APIs
  moved during the storage migration.
- Atomic, idempotent light-history pruning through an application-root task.
- Versioned production-root conformance execution through a test-only adapter that returns observations, not a handle.

Current Rust repositories include:

- `DagRepository`
- `PeriodRepository`
- `MetadataRepository`
- `PillarRepository`
- `PbftRepository`
- `TransactionRepository`

This section and the current repository implementations are the storage coverage source of truth. Do not maintain a
separate unchecked repository checklist: record a demonstrated storage gap in **Storage Gaps and Risks** and the normal
issue/roadmap process only when it is actionable, then remove it when implementation and required validation land.

### Storage Boundary and Remaining Risks

- Rust-mode `DbStorage`, bridge batches, and the storage overlay are deleted. Production paths derive operation-shaped
  services from `ConsensusApplication` or bounded reads from `ConsensusQueryApi`; no general storage accessor may be
  added.
- Rust consensus runtimes own storage fact collection, atomic write ordering, idempotency, restart normalization, and
  durable commits through `rustaxa-storage` repositories. Legacy `DbStorage` remains only in the pure-C++ reference
  composition and classified external/test paths.
- Snapshot, migration, and broad iterator/compaction administration remain unsupported in Rust mode until a named
  operation-shaped task is justified. They must not be recreated as a general storage facade.
- External-EVM system-transaction publication requires non-empty canonical RLP whose Keccak hash matches the indexed
  hash. Recovery rejects legacy pending markers without that payload; operators must rebuild rather than publish
  incomplete historical data.
- Live classifications and deletion conditions are recorded in `doc/consensus_bridge_shim_audit.md`. Re-run the storage
  boundary guard whenever a storage-adjacent path changes and treat any unclassified production fallback to legacy C++
  as a blocker.

Validation:

- Always run `rust_storage_tests` for storage changes.
- Run impacted C++ gtests or `ctest` subsets when C++ storage behavior changes.
- Add or update conformance tests for changed serialization, update, or read/write semantics.
- Run `scripts/storage_conformance_diff.sh` before closing larger storage refactors, after confirming with the task owner.

## FinalChain Rewrite Plan

### Scope

Goal: preserve protocol, RPC, GraphQL, and pure-C++ FinalChain behavior while
keeping Rust-mode FinalChain ownership native. Public reads use
`ConsensusQueryApi`; one application-bootstrap-owned concrete-state adapter
retains exact concrete-EVM/`state_db`, tracing, public-state, recovery, and
state-lifecycle operations for named clients. The historical Rust-mode C++
class surface is not a compatibility contract and has been deleted.

The untouched pure-C++ FinalChain implementation is selected by the single
Rust production composition switch:

- `RUSTAXA_ENABLE`

When enabled, the standalone overlay supplies `final_chain::FinalChain` and the untouched legacy implementation is
excluded from Rust production builds. Native Rust FinalChain owns rewards-stat planning, cache persistence, restart,
and distribution behavior directly; Rust mode has no standalone `rewards::Stats` overlay or bridge runtime. The former
standalone rewards-stats flag, `StatsOld` scaffold, and compatibility facade are retired. Pure-C++ reference builds
retain the untouched legacy RewardsStats header, source, and focused test.

### Current Implementation Status

- Rust-mode chain-index, block, transaction, receipt, bloom, and public DPoS
  reads are client-oriented `ConsensusQueryApi` operations. RPC, GraphQL,
  debug/Test RPC, log replay, light clients, observers, and Rust-mode fixtures
  do not retrieve those values through the FinalChain overlay or the opaque
  application root.
- DPoS query boundary is Rust-backed:
  - genesis vote-count snapshot is derived in Rust from genesis validator stake.
  - public vote-count, stake, delegated-amount, yield, and supply operations
    preserve the requested `EthBlockNumber` through `ConsensusQueryApi`.
  - validator eligible-vote-count sets and pillar header/state-root facts stay
    native for PBFT pillar construction. The concrete EVM host receives only
    the requested period and returns bridge root/epoch; it no longer
    materializes native headers, validators, signer weights, or total votes in
    C++.
  - DagManager verification now receives DPoS authorization facts whose VDF sortition denominator is selected in Rust
    from genesis DPoS config plus the configured Magnolia boundary, instead of passing per-block hardfork or
    validator-max policy through the C++ shim.
  - DagBlockProposer now has a standalone Rust-mode overlay facade with no feature-on legacy proposer source or
    `DagBlockProposerOld` scaffold. Rust owns proposer eligibility status decisions, legacy VRF
    input bytes, deterministic tip-selection policy, transaction-pack command flow, atomic DAG observation and
    revalidation, VDF input/message bytes, asynchronous VDF proof jobs and cancellation, runtime-derived
    wait/cancel/stale-proof decisions, retry-cursor updates, proposal timestamps, session-owned block construction and
    unsigned intent state, and final signed-RLP construction after temporary C++ node-secret signing. C++ receives only
    exact signing/VRF requests and returns only signature/proof bytes; it no longer echoes frontier, transaction, gas,
    timestamp, unsigned-intent, or VDF job fields through standalone bridge planners. C++ still owns process-thread and
    network mechanics, node-secret signature/VRF execution, concrete gas execution, logging, and network egress.
  - Rust finalization appends DPoS snapshots for finalized native-transfer blocks and the Rust-supported
    `registerValidator(address,bytes,bytes,uint16,string,string)`, `delegate(address)`,
    `undelegate(address,uint256)`, `confirmUndelegate(address)`, `cancelUndelegate(address)`,
    `undelegateV2(address,uint256)`, `confirmUndelegateV2(address,uint64)`,
    `cancelUndelegateV2(address,uint64)`, `reDelegate(address,address,uint256)`, `setValidatorInfo(address,string,string)`,
    `setCommission(address,uint16)`, and the exact four-byte Phalaenopsis DPoS escrow-transfer action `0x44df8e70`
    contract subset. The Rust snapshot
    persists validator stake/vote aggregates plus a validator/delegator stake ledger seeded from genesis delegations so
    undelegation and redelegation ownership checks stay in Rust. It also persists validator insertion order and
    commission-change block numbers, ordered V1 undelegation queues, and V2 undelegation queues with per-delegator IDs
    so paged validator/undelegation reads, combined pending counts, and owner commission rules remain restart-durable.
    Before Magnolia, validator queries preserve the legacy zero pending-count field. At Magnolia and later, Rust derives
    the count and deletion guard from the actual combined queues, intentionally correcting the legacy `ValidatorV1`
    blind spot where a pre-Magnolia request was omitted from the persisted counter and could permit premature deletion.
    A full pre-Magnolia V2 undelegation removes a zero-stake, zero-commission validator after stake mutation but retains
    the V2 request and last-ID cursor as custody/history state, so the request remains queryable and confirmable after
    registration deletion. V2 unlock blocks select active Cacti, Cornus, then base locking configuration. The staged
    same-block claim-gas view removes confirmed V2 requests and restores delegation membership and principal after a
    successful V2 cancellation before pricing a later `claimAllRewards()` call. Claim-all gas starts from live finalized
    delegation membership even when eligibility APIs use a nonzero delayed snapshot.
    The Phalaenopsis escrow-transfer action is gated by its configured activation period, remains payable after Cornus,
    charges 1,000 action gas, and mutates only sender/DPoS account balances through the common successful contract-payment
    path; exact-input failures and pre-activation calls remain normal status-zero unknown-method receipts.
    Native redelegation now preserves the legacy ordered business checks, reward-claim/log ordering, unchanged escrow
    and aggregate delegated amount, below-minimum destination-pair creation, Aspen zero-amount boundary, disabled
    maximum-stake semantics when the configured maximum is zero, and validator deletion guard. Before and at
    `fix_redelegate_block_num`, Rust reproduces the historical same-validator stale stake, ordered vote-delta, and
    restored reward-pool writes required to replay old blocks; at the exact fix block it applies the configured ordered
    corrections after reward distribution and transaction effects but before snapshot publication, and later
    same-validator calls fail normally.
    Every successful pre-fix same-validator call records restart-durable complete-history state and a corruption marker,
    including zero-amount calls that do not leave an inferable stake gap. A reward-bearing call from an ambiguous older
    snapshot, or a repeated call for a marked or stake/principal-mismatched validator, remains an explicit hard unsupported
    history until Rust owns the legacy reward-state reference graph; Rust must not publish an approximate cumulative-reward
    result for that topology. Repeated zero-pool calls remain representable on complete-history snapshots.
    Databases finalized by the immediately preceding unreleased redelegation build require rebuild/replay if they contain
    a markerless zero-amount same-validator call, because scalar stake state cannot reveal that reference corruption.
    The extended 23-item snapshot codec still decodes the prior 21-item same-validator form as history-incomplete and
    adds an independent delegation-ledger completeness bit. Direct schema-five/six snapshots remain ledger-incomplete;
    schema-seven through schema-22 snapshots reconstruct complete principal membership. Rollback to a
    pre-slice binary is unsafe after any post-slice DPoS snapshot is finalized because that binary cannot decode the
    appended items. Snapshots
    are persisted atomically with finalized block indexes, executed DAG/transaction status counters, and `lastBlockNumber`. Startup reloads persisted historical DPoS
    snapshots so PBFT, DAG, and pillar reads can reuse block-scoped Rust FinalChain facts after restart.
  - Rust finalization persists account snapshots atomically with finalized block indexes plus `lastBlockNumber`.
    Startup reloads persisted account snapshots and only serves latest account reads when the Rust account snapshot has
    caught up to the finalized head, so transaction purge and proposal filtering do not silently use genesis state after
    restart.
  - Because Rust finalization is scoped to post-Magnolia execution, native transaction fees are assigned to validator
    commission rewards by finalized DAG block author and transaction hash through the shared Rust rewards-stat planner,
    using bridged Magnolia/Aspen rewards configuration, DAG difficulty, `blocks_per_year`, and previous-block cert-vote
    facts. The native FinalChain path owns a long-lived Rust rewards-stats runtime, persists non-boundary interval cache
    rows in the finalized-block batch before `lastBlockNumber`, reloads cached stats on startup, and clears the cache at
    distribution boundaries after applying interval rewards. Native execution and reward account mutation now use a
    staged account map that is published only after finalization storage commits; post-Magnolia transaction fees credit
    the Rust DPoS contract account while the DPoS snapshot records per-validator commission ownership. Rust native
    finalization also distributes fixed-yield and Aspen part-two dynamic-yield minted block/DAG/vote rewards from decoded
    Rust reward stats into staged validator commission and delegator reward pools, credits the DPoS contract account with
    the minted total, lazily migrates part-one minted tokens exactly once into an explicit durable Aspen supply state,
    enforces monotonic restart provenance and the configured maximum supply, and writes typed header `total_reward`
    from the Rust plan. Bounded native FinalChain queries expose DPoS total delegated, yield, total supply, and read-only
    delegator reward pages backed by the persisted Rust reward-reference graph. The page
    read preserves legacy insertion/removal ordering, wrapping offsets, widened gas calculation, and strict corruption
    handling without falling back to scalar reward state. Rust now executes delegator
    `claimRewards(address)`, validator-owner `claimCommissionRewards(address)`, validator-owner metadata/commission
    updates with ordered business failures, snapshot-consistency checks, and same-block commission reward effects,
    current-ABI `claimAllRewards()`, and
    stake-mutation auto-claims by moving reward balances through staged Rust account/DPoS snapshots. Receipts for the
    supported native DPoS subset now carry Rust-generated legacy ABI logs for validator registration, delegation,
    undelegation, V1 and V2 undelegation confirmation/cancelation, redelegation, direct claims, commission claims,
    validator info/commission updates, claim-all, and
    stake-mutation auto-claims, with the block header bloom derived from those logs. Supported DPoS owner validation
    failures now persist failed receipts without mutating DPoS state. Rust native finalization now accepts
    both the current `claimAllRewards()` ABI and the legacy pre-fix `claimAllRewards(uint32)` batch ABI, gates the batch
    selector on `fix_claim_all_block_num`, and charges claim-all gas from a staged live Rust DPoS delegation view that is
    independent of the configured eligibility delay. The active
    Rust finalization path also persists the legacy two-level `final_chain_log_blooms_index` chunks with author-augmented
    blooms and routes `FinalChain::withBlockBloom` through Rust. All current Solidity DPoS ABI methods have native Rust
    routing, and `FinalChain::call` simulates all 16 mutation selectors through an atomic transient envelope.
    Finalized DPoS mutation dispatch is consolidated behind a caller-owned staged-snapshot kernel so transient calls
    reuse deterministic contract transitions without inheriting finalized value injection, fees, receipts, reward
    planning, cleanup, or publication.
    The shared kernel result now preserves the successful `undelegateV2` ABI request ID and legacy pre-fix
    `claimAllRewards(uint32)` end flag while finalized receipt publication continues to discard return bytes. Existing
    widened finalized page selection remains unchanged; aligning wrapping-batch selection has separate historical replay
    consequences. The kernel now also carries exact legacy mutation business errors separately from ABI output, including
    pinned no-CGO btcec registration-proof diagnostics and claim-all validator context, while ABI lookup failures remain
    untyped and corrupt state stays a hard error. Finalized receipts discard the typed reason. `FinalChain::call` now
    routes all 16 mutation selectors through one atomic transient envelope over exact requested-block account and DPoS
    snapshots. The envelope reproduces gas-cap and value affordability, intrinsic-plus-action gas, Cornus payability,
    staged payable value, typed business errors, ABI outputs, and logs, then drops all staged account/DPoS changes
    without reward advancement, receipts, end-block cleanup, or publication. Historical blocks without complete Rust
    snapshots fail closed and still require replay, migration, or an explicitly retained hybrid route. A narrow call-log
    result carrier is the only bridge delta; no new handle or request export was added.
    Rust native finalization also executes the slashing `commitDoubleVotingProof(bytes,bytes)` precompile path for
    legacy PBFT vote RLPs: Rust decodes the calldata, recovers both vote signers, validates the double-vote facts,
    persists restart-durable jail blocks, jailed-validator order, and duplicate-proof keys in the DPoS snapshot, emits
    the legacy `Jailed(address,uint64,uint64,uint8)` log, and derives effective DPoS eligibility/total vote counts from
    the Rust jail state. Value-bearing proof calls preserve the legacy precompile behavior despite the ABI's nonpayable
    metadata: value moves into the slashing account only after a successful proof, while failed or duplicate proofs roll
    it back, and the first successful write initializes the slashing account nonce to one. Slashing reads for
    `getJailBlock(address)` and `getJailedValidators()` are Rust-backed both through `FinalChain::call` and as finalized
    native transactions. Recognized read transactions charge the legacy fixed action gas, retain successful value at
    the slashing account, emit no logs, and leave its nonce unchanged; malformed and out-of-gas reads roll value back.
  - The application-root execution cut supersedes the former C++-driven session boundary: `ConsensusApplication` now
    owns system-transaction/rewards planning, concrete-EVM sequencing, result/receipt/root validation, durable pending
    recovery ordering, state-commit approval, and FinalChain publication. C++ retains exact typed StateAPI leaves only;
    the broad execution API/session handles, factories, C++ action loop, and consensus materializers are deleted.
    The concrete-root rollout uses one eager concrete-state policy: StateAPI executes every Rust-mode finalized period
    and its committed post-rewards root is the canonical header root, including genesis. Rust binds prior,
    post-transaction, post-rewards, and committed descriptors through the version-one concrete execution/provenance
    contract and the version-three pending-publication marker. It independently replays Rust-supported transfers, DPoS,
    and slashing
    against ordered per-transaction concrete effects, including native actions after arbitrary EVM work. Exact rewards
    retries are idempotent; Rust checks the reported minted total and final DPoS contract balance after minted/fee
    credits. Reward-only DPoS rows are projected, zero configured yield disables rewards, and period zero
    is an explicit genesis activation while `u64::MAX` disables Aspen part two or Cacti in local fixtures. Concrete-root
    policy version one is paired across Rust storage and `state_db`. Markerless/synthetic history fails with
    `FINAL_CHAIN_CONCRETE_ROOT_REBUILD_REQUIRED`; `--rebuild-db` preserves the old pair in a timestamped backup and
    starts a clean full-resync database instead of rewriting finalized hashes or falling back. Deployment evidence is
    complete: native differential tests cover transfer, DPoS, slashing, system/reward, arbitrary-EVM,
    marker/commit/publication crash windows, and staged-state rejection, while the reproducible five-node Python gate
    proves timestamped rebuild, clean full resync, graceful/crash restart, mixed-lane finalization, and exact
    state/transaction/receipt/header/finalized-
    hash agreement. The Rust-mode `FinalChain` facade and `final_chain_shim` are deleted: one private
    `ExternalEvmStateOwner` is constructed at application bootstrap and serves exact finalization, account/code/storage/
    call, trace, prune, descriptor, commit, and discard operations. Native DPoS/slashing calls remain authoritative on
    the bounded query client; arbitrary EVM calls remain physical StateAPI operations. No CXX StateAPI handle, session,
    action loop, range executor, service locator, or C++ publication authority exists.
  - PBFT manager fact collection now connects directly to the Rust FinalChain runtime for PBFT final-chain hash lookup
    and validation, total eligible vote counts, per-wallet eligible vote counts, and wallet eligibility refresh. Missing
    delayed headers or DPoS snapshots are returned to PBFT as typed Rust facts instead of re-centering those consensus
    decisions in C++ FinalChain orchestration.
  - non-genesis DPoS queries still return typed errors or throw when the queried block has not been finalized through
    Rust snapshot maintenance. All current-ABI DPoS mutations execute transiently through `FinalChain::call`; historical
    databases without complete Rust account/DPoS/reward-graph snapshots remain fail-closed pending replay/rebuild or an
    explicitly designed hybrid migration.
  - `rustaxa-types::FinalChainNonce` owns arbitrary-width FinalChain account and transaction nonce semantics. Account
    snapshots retain their existing schema and canonical RLP for historical values, while native execution may persist
    state above U256 after a maximum transaction nonce. Finalization, account lookup, system-account facts, and
    Rust-planned external-EVM transcript carriers use canonical minimal big-endian bytes. The legacy public C++ account
    API remains U256 and fails explicitly when Rust state cannot be represented; it never truncates.
  - selected DPoS precompile reads through `FinalChain::call` are Rust-backed for `isValidatorEligible(address)`,
    `getTotalEligibleVotesCount()`, `getValidatorEligibleVotesCount(address)`, `getValidator(address)`,
    `getValidators(uint32)`, `getValidatorsFor(address,uint32)`,
    `getTotalDelegation(address)`, `getDelegations(address,uint32)`, `getUndelegations(address,uint32)`,
    `getUndelegationsV2(address,uint32)`,
    and `getUndelegationV2(address,address,uint64)`. The three eligibility reads use the configured delayed snapshot and
    delayed hardfork/jail evaluation block, matching the legacy delayed reader; the remaining selected reads use the
    exact finalized-block snapshot. The fixed-gas eligibility family is also executable as native finalized
    transactions with its Cornus nonpayability behavior. Native finalization additionally executes the two fixed-5,000
    singleton reads, `getValidator(address)` and Cornus-gated `getUndelegationV2(address,address,uint64)`, against live
    block-local DPoS state so they observe successful earlier same-block mutations. The dynamic validator-page family,
    `getValidators(uint32)` and `getValidatorsFor(address,uint32)`, is likewise native-executed against live state with
    legacy page gas, Cornus nonpayability, wrapping `uint32` page offsets, and swap-remove validator ordering. The V1 and
    V2 undelegation-page reads are also native-executed against live state with legacy queue ordering, wrapping page
    offsets, fork/payability behavior, and their distinct storage-read gas formulas. `getTotalDelegation(address)` is
    native-executed against the same live principal ledger with zero gas for empty membership, 5,000 gas per validator
    membership, Cornus nonpayability, and explicit rejection of incomplete legacy delegation ledgers.
  - `rustaxa-consensus::dpos_reward_graph` is persisted as DPoS snapshot item 24 and is the reward-reference authority
    for genesis, registration, checkpoints, claims, stake mutations, terminal deletion, and pre-fix same-validator
    correction. It preserves arbitrary-width reward-per-stake, explicit validator heads and delegation cursors, exact
    counts including legacy inflation/orphans, incomplete-history provenance, stale live-or-missing heads, and exact
    reward arithmetic. Schemas through 23 fail closed for graph-dependent behavior pending replay/rebuild. Native and
    direct `getDelegations(address,uint32)` routing, including paging, graph rewards, corruption handling, and restart
    parity, is complete. Current-ABI FinalChain/DPoS parity is closed; historical snapshot replay/rebuild remains
    an explicit deployment boundary rather than a legacy execution fallback.
### FinalChain Persisted and External-State Boundaries

Rust storage owns FinalChain headers, hash/number indexes, receipts, transaction locations, bloom indexes, executed
counters, rewards statistics, concrete-root provenance, and publication markers. Publication applies the complete
visibility batch atomically and advances the finalized head last.

One application-owned `ExternalEvmStateOwner` retains the concrete `StateAPI` lifetime behind exact finalization,
account, code, storage, call, trace, prune, descriptor, commit, and discard operations. Rust owns request identity,
transaction/native-action ordering, rewards planning, result validation, commit approval, recovery, and publication.
There is no Rust-mode FinalChain facade, shim, CXX StateAPI handle, C++-driven execution session, range executor,
service locator, or C++ publication authority. Native Rust may use internal typed sessions as implementation details.

Historical databases without the paired concrete-root provenance marker fail closed and require the timestamped
backup/full-resync path. Changes to FinalChain persistence, concrete execution, DPoS/slashing, receipts, roots, tracing,
or pruning require the applicable native tests, current-source pure-C++ differential, restart/recovery coverage, and
full-node gate in `doc/rewrite_validation_strategy.md`.

## Consensus Rewrite Plan

### Scope

Goal: cut consensus over to native Rust ownership while keeping protocol behavior, external product contracts, and the
pure-C++ reference route stable. Internal C++ manager APIs and wiring are explicitly in scope for removal.

Native Rust consensus gap closeout:

- The native Rust consensus campaign is complete for the authorized consensus, network-planning, execution-
  orchestration, and concrete-root scope. Stable ownership rules live here, retained boundaries live in
  `doc/consensus_bridge_shim_audit.md`, and actionable new work belongs in the normal issue/roadmap process; do not
  recreate a gap-plan document for completed scope.
- Rust owns consensus rules, durable consensus state, restart normalization, storage/query selection, canonical payload
  retention, validation decisions, lifecycle command selection where it affects consensus behavior, scheduler/timer
  policy, ordered side-effect planning, and typed executor-result validation.
- C++ may remain only as a leaf adapter for named public APIs, minimal app hosting, OS primitives, key signing,
  tarcap peer transport mechanics, and concrete EVM/StateAPI operations.
- Rust is now authorized to own network packet inspection, admission, routing, consensus queueing, effect ordering,
  gossip/send selection, and executor-result validation. Tarcap retains socket/peer mechanics, packet wrapping, actual
  transmission, disconnect execution, and lane scheduling.
- Rust is now authorized to own FinalChain/EVM execution orchestration, request construction, canonical rewards
  payloads, receipt/result validation, commit ordering, recovery, and publication. C++ `StateAPI` retains concrete EVM
  calls and `state_db/` mutation as a leaf executor until separately rewritten.
- Legacy C++ object materialization is accepted only for a named RPC/plugin/public client or at an unavoidable leaf
  executor. Tests, internal managers, logging, and convenience are not valid retention reasons.

Completed closeout slices:

1. FinalChain and DPoS fact ports: consensus-facing FinalChain/DPoS facts route through typed Rust ports, while arbitrary
   EVM/state execution remains an explicit boundary.
2. Transaction application cutover: native `ConsensusApplication` owns verification, admission, queueing, packing,
   persistence, recovery, finalized cleanup, gas-oracle state, and public-event intent selection. Public submission is an
   operation-shaped application task, reads/stats use `ConsensusQueryApi`, packet ingress/gossip use
   `ConsensusNetworkApi`, and C++ retains only public formatting/event dispatch and concrete EVM gas execution.
3. DAG application cutover: native services own graph/runtime state, verification, admission, atomic DAG/transaction
   persistence, finalized cleanup, non-finalized sync, restart, and canonical network decisions. Rust-mode App,
   network, RPC, GraphQL, stats, and light clients cannot obtain a `DagManager` or materialize mutable internal DAG
   graphs; the facade, bridge task/materialization family, and shim directory are deleted.
4. DAG proposer application cutover: native runtime owns scheduling, eligibility, packing, retry/throttle decisions,
   asynchronous VDF progression and proof execution, tip selection, block construction, signing progression, local
   admission, and gossip planning. App supplies exact timer/process, signing/VRF, concrete gas, tarcap, and public-event
   reports. The Rust-mode proposer facade, worker-command bridge module, App ownership, and shim directory are deleted.
5. Vote, slashing, and pillar application cutover: native services own pillar startup/restoration, vote admission,
   threshold state, block construction, finalization persistence/cleanup, lifecycle, network decisions, public query
   views, and post-ack observation. C++ retains exact signing/VRF, tarcap, FinalChain pillar-anchor facts, and best-effort
   public event delivery; Rust mode exposes no vote or pillar manager facade.
6. Rewards stats carrier ownership: Rust owns rewards-stat decisions, compatibility encoding, interval cache
   persistence/reload/clear, and native finalization integration; C++ decoded carriers are public/test/EVM adapters.
7. Typed consensus storage port generalization: migrated production consensus routes use task-specific Rust storage
   ports/runtimes and Rust-owned atomic write groups instead of broad storage shim batches or generic appenders.
8. Public object surface and compatibility adapter deletion: obsolete sidecar maps, stale helper APIs, and public/debug
   surfaces that only needed DTOs were replaced or deleted; remaining materialization is classified as public, network,
   EVM/executor, or active subsystem edge work.
9. Lifecycle, scheduler, signing, and event executor shell collapse: PBFT and DAG scheduler decisions found in the
   closeout audit are Rust-planned, and subsystem sessions carry typed executor reports. Remaining C++ shell work is the
   accepted host/executor boundary listed above.

Closeout definition now in force: consensus production behavior, including network consensus pipelines and external-EVM
orchestration, must not require manager-shaped C++ shims or broad bridge compatibility code. Only the leaf mechanics
listed above may remain in C++. New work extends native Rust runtimes, application services, typed ports, or planners.
When a shim or bridge path is touched, delete the complete compatibility family or reduce it to a named leaf adapter in
the same slice.

### Native Application Composition Boundary

Rust-enabled production has one target composition: a native `ConsensusApplication` root constructed once by `App`.
The root opens native storage, verifies schema and genesis identity, constructs FinalChain, and restores the private
PBFT and DAG/transaction/sortition graphs, including vote, pillar, slashing, and consensus-network services, before C++
can publish it. Rust mode fails closed for legacy rebuild, revert, and migration-only startup modes until native admin
operations replace those workflows; pure-C++ mode retains the upstream maintenance path.
The deleted `BridgePbftService` and `BridgeDagTransactionService` handles must not be recreated behind replacement C++
managers. The bootstrap CXX surface is one opaque `BridgeConsensusApplication` created by one
`create_consensus_application` operation. It exposes no internal-service accessors; operation-specific
application and network APIs are invoked on or bound from the root without publishing its private owners.

Construction and restoration are atomic publication boundaries: configuration and every required native sibling must
restore successfully before C++ receives the application handle or any external adapter. Failure publishes no partial
root. One root does not imply one global mutex: subservices retain private lock domains, application tasks own the
declared cross-service lock order, release every native guard across external leaf execution, and generation/cursor-
revalidate the exact result before mutation or publication. The root owns native FinalChain state and orchestration;
concrete `StateAPI`, EVM, and `state_db/` execution remain the named C++ leaf boundary.

Cross-boundary execution follows a resumable typed-effect protocol:

1. C++ submits canonical bytes, opaque identities, configuration, or a client-specific request to one application task.
2. Rust validates the request, reads private consensus state, may update only provisional session/cursor state, and
   returns the next typed external leaf effect when physical work is required. Authoritative durable or published state
   does not mutate until the matching result is accepted.
3. C++ executes only the named tarcap, concrete EVM/`state_db`, signing, timer/process, or public-formatting leaf.
4. C++ reports the exact effect identity and typed result; Rust validates it before advancing, persisting, publishing,
   or producing the next effect.

Every effect carries a native session and effect identity plus an operation-specific payload. Unknown, stale,
cross-session, wrong-operation, and contradictory reports are rejected. Duplicate reports for a completed effect are
idempotent, and retryable mutation effects retain a Rust-issued idempotency identity. Success requires the matching
result variant; failure carries no success value and has a stable machine-readable code. Free-form diagnostics are
observability only and never consensus input.

Internal consensus objects and service handles do not cross CXX. Canonical RLP/bytes, hashes, scalar identities, and
operation-specific DTOs may cross when owned by a named leaf or public client. Temporary C++ materialization is allowed
only at that leaf and must not become authoritative state or a manager-to-manager protocol.

Migration proceeds by vertical subsystem checkpoint rather than a global preparation waterfall. Once the native owner
for a PBFT, DAG/transaction, network, or execution family exists, the same checkpoint should combine its owner move,
facade deletion, materialization removal, shim contraction, and boundary narrowing. It need not wait for unrelated
subsystems to finish. Every checkpoint must still leave Rust mode buildable, retain the pure-C++ reference route, and
satisfy the validation strategy.

### External Consensus Facade Boundaries

The native application root, two client-oriented Rust APIs, and exact host ports define the long-lived external
consensus contracts. They are narrow operation boundaries, not service locators: they must not expose consensus manager
handles, mutable sidecars, storage iterators, `DbStorage`, `StateAPI`, or internal runtime state.

| Boundary | Rust facade | Rust ownership | External executor or adapter ownership |
| --- | --- | --- | --- |
| Application tasks and public submission | `ConsensusApplication` | Lifecycle, DAG/transaction/proposer and pillar mutation tasks, canonical admission/finalization persistence, public-event selection, and exact host-effect/result validation | App process hosting, RPC/GraphQL mutation formatting, concrete gas execution, and best-effort public event delivery |
| Network and tarcap | `ConsensusNetworkApi` | Canonical packet ingestion, inspection, admission, routing, consensus queues, peer/gossip/send decisions, effect ordering, identity, and result validation | Socket and peer mechanics, packet wrapping, actual send/gossip/disconnect execution, and physical lane scheduling |
| External EVM and StateAPI | Application-owned `ExternalEvmPort` | Execution orchestration, canonical requests and rewards payloads, result/receipt validation, lifecycle, commit ordering, recovery, publication planning, and storage-publication authorization | Exact concrete EVM calls, staged `state_db/` mutation, contract execution, tracing, pruning, and descriptor operations |
| Public reads | `ConsensusQueryApi` | Stable read-only consensus DTOs backed by Rust storage and query logic | RPC/GraphQL/plugin formatting, live network/admin views, and public C++ object materialization where still required |

C++ adapters may execute or format these contracts, but they must not recreate consensus decisions from returned facts.
Residual adapter classifications and deletion conditions belong in `doc/consensus_bridge_shim_audit.md`; open a normal
roadmap item only when a named client can migrate or a correctness gap is demonstrated. Exact DTOs and methods are owned
by the Rust facade modules and their bridge tests rather than by a separate touchpoint inventory.

Signing, concrete gas estimation, and best-effort public observation remain operation-specific leaf calls rather
than shared service facades. Host thread, timer, sleep, and process mechanics may remain as Rust-commanded executors,
but they do not justify an internal manager or service handle and are deleted or narrowed when native infrastructure
owns the physical operation.

Rules:

- Do not delegate Rust shim behavior back to legacy FinalChain or other old implementation methods.
- Temporary Rust-mode gaps must be explicit shim-local defaults, no-ops, or tracked unimplemented paths.
- Temporary guarded touches to upstream-owned C++ files should be removed once a complete native route can own
  Rust-mode behavior. The PBFT application runtime now owns the former manager pillar-vote sync hook; original
  `pbft_manager.cpp` stays clean versus `upstream-main`; manager/materialization removal is complete; the private
  FinalChain facade is deleted, leaving only the classified concrete StateAPI leaf operations.
- Treat `dposIsEligible` and related vote-count methods as real consensus work, not permanent dummy behavior.
- Keep only physical network and OS-thread mechanics in C++; consensus callbacks, queues, routing, and orchestration move
  to the Rust application service.
- `ConsensusNetworkApi` owns canonical ingress inspection and bounded one-shot egress operations, including peer
  eligibility, known/sync filtering, exclusions, chunking, packet construction, exact target selection, dependent known
  marks, acknowledgement, retry, and cancellation. Tarcap retains only immutable peer snapshots, lane scheduling,
  packet sealing, exact-peer sends/disconnects, and known-cache writes. No Rust-mode handler-local consensus planning or
  object-shaped fanout helper remains.
- Ignore logging when deciding whether behavior can move to Rust. Logs are boundary observability, not consensus
  ownership. Keep temporary C++ logging only as an executor/reporting detail while moving the underlying decision,
  state transition, or persistence logic into Rust.
- Shape new ingress-adjacent consensus APIs as operation-specific application pipelines. Prefer functions that consume
  canonical payload bytes or compact facts only when their pipeline can make an authoritative decision, produce small
  protocol plans/effects, and defer C++ `PbftVote`, `PbftBlock`, `DagBlock`, `PeriodData`, and `Transaction`
  materialization until a compatibility executor truly needs those objects.
- Do not add a generic `ingest_network_packet` shadow call whose result is ignored. Network-to-Rust entry points must be
  packet-family-specific and return protocol status or typed effects consumed by the live handler.
- Treat ingress processing as multiple logical pipelines, not one monolithic consensus loop. Ingress-adjacent Rust APIs
  should keep pipeline-specific facts and effects separate for peer status/sync control, transaction admission, DAG
  admission, DAG sync, PBFT vote progress, PBFT sync/finalized-period intake, and pillar votes. When one pipeline must
  affect another, return an explicit effect such as request-sync, block-peer-order, mark-known, admit, gossip, report
  malicious, enqueue-period-data, or drive-PBFT-progress instead of mutating another pipeline implicitly.
- Shape consensus pipeline units around ingress-payload-backed events. The dispatcher owns ingress-message routing outside
  the consensus crate, and passes typed consensus units such as `PbftVoteEvent` into consensus pipelines. Those units may
  carry compact facts or enrichment ids while preserving canonical bytes under the active pipeline owner. Queues should transfer
  ownership of these event units between stages instead of exposing shared mutable message objects. The current event
  names and payload shapes are deliberately open to change while the first production pipeline integration proves the
  right boundaries.
- Express consensus business logic as deterministic protocol planners over explicit state views. A planner receives a
  consensus event or command, compact facts, config/time inputs, and borrowed state views, then returns a protocol plan.
  The plan may include validation outcome data, ordered effects, storage/write intents, and follow-up consensus events.
  This is the concrete meaning of a protocol state transition: the observed consensus state plus an input maps to the
  next intended state and effects, while execution remains outside the planner. Tokio, actors, and ring-buffer workers
  belong around the planner as scheduling/execution machinery, not inside the consensus rule implementation.

### PBFT Manager Rust Ownership Boundary

The PBFT manager protocol brain, consensus queue ownership, and effect ordering live behind native
`ConsensusApplication` tasks. The Rust-mode `PbftManager` facade and its compatibility surface are deleted. Transport,
execution, signing, timer/process, and public adapters are operation-shaped rather than manager-shaped.

Boundaries that should not move as part of the PBFT manager breakthrough:

- Network/tarcap transport: peer connections, packet wrapping, physical fanout, physical known-peer mutation,
  disconnect/report execution, and lane scheduling stay outside the consensus manager migration. Rust owns consensus
  queueing, send/gossip selection, dependency ordering, and typed egress, mark-known, sync-request, and peer-report
  effects for the transport executor to perform.
  The current Rust-enabled route uses one `Network`-owned consensus-network API shared by the latest and v5 capability
  handler families. Its effect queue is partitioned by transport lane and PBFT gossip effects own canonical vote/block
  payloads. Verified-vote admission is an application-root task: the network service composes its private PBFT and
  FinalChain siblings, admits canonical vote bytes directly, and returns a typed outcome before releasing proposed-block
  publication, peer-known, and gossip effects. Only a typed slashing-transaction signing/insertion result crosses back
  from the retained C++ executor leaf. Exact duplicate votes may still carry a
  previously unseen block without being regossiped. Bundle shape preflight completes before any member admission. The
  no-consumer generic shadow-ingress arena and its capacity configuration are deleted. Get-PBFT-sync response authority
  is also native for versions five and six: Rust validates canonical requests and history bounds, reads native period
  data, attaches reward votes, builds complete sync packets, and emits version-six proposed-block bundles. Tarcap still
  owns lane serialization, packet sealing, peer syncing/report/disconnect execution, and acknowledgements. Accepted-only
  bundle aggregation is native. Latest-version proposed-block bundle intake is native too: Rust owns raw signed-block
  decoding, relevance and unique-author checks, FinalChain-backed DPoS admission, and storage-first publication, while
  tarcap retains only syncing-peer gating and malicious-peer execution. Sync intake and handler routing are native
  application/network operations.
- EVM/StateAPI execution: Rust owns transaction ordering, native-action semantics, rewards planning, result validation,
  commit approval, recovery, and publication. Exact concrete EVM, staged `state_db`, trace, prune, and descriptor
  operations remain outside PBFT as the application-owned physical executor leaf.
- Named leaf/public compatibility: temporary `PbftBlock`, `PbftVote`, `PeriodData`, `DagBlock`, `Transaction`, and
  pillar materialization may remain only for a named public client or unavoidable executor leaf. An internal manager,
  test, log, or convenient remaining caller is not a retention reason.
- Node process mechanics: the App-owned shell may retain one worker thread, interruptible waits, clock reads,
  startup/shutdown wiring, event emission mechanics, and key custody. Native `ConsensusApplicationRuntime` owns daemon
  scheduling, lifecycle state, retries, and ordered protocol effects; the shell retains no protocol cursor or manager.

All former `PbftManager` authority is native: period/round/step state, daemon-tick control flow,
proposal/certify/finish-polling transitions, sync-period admission, proposed-block selection and cleanup planning, vote
and reward-vote selection, finalization planning and bounded resume, dynamic-lambda decisions, DAG/transaction cleanup
planning, PBFT-chain head advancement plans, storage/write intents, cross-pipeline effects, and ordered side-effect
contracts. Logging is explicitly not a boundary; it can stay temporarily in the C++ executor as reporting derived from
Rust statuses and telemetry.

Current status: PBFT manager ownership is complete for this protocol-runtime boundary. Rust owns the manager scalar
runtime, its native mutex and complete session container, daemon-tick and state-action cursors, transition persistence,
broadcast planning, sync-period admission,
queue-backed compact facts and canonical transaction/cert-vote payload sources, proposal ranking, finalization planning,
dynamic-lambda decisions, and bounded restart/duplicate classification. Remaining `PbftBlock`, `PbftVote`,
`PeriodData`, `DagBlock`, `Transaction`, network, timer, concrete FinalChain/EVM, and public-formatting crossings are
named external executor or client boundaries above, not authoritative PBFT manager decision state. Their live
classifications and deletion conditions are tracked in `doc/consensus_bridge_shim_audit.md`.

Rust-enabled composition no longer includes a `PbftManager` object or shim. `App` constructs one
`ConsensusApplication` and drives lifecycle through it; network, query, execution, signing, timer, and lifecycle callers
use operation-shaped root adapters. Local generated-vote admission and own-vote persistence commit atomically, and
canonical proposal publication derives its identity natively. The untouched original manager remains selected only in
all-Rust-disabled reference builds. The private C++ runtime and manager-shaped CXX task/effect family are deleted.
Native application code owns scheduling, startup recovery, lifecycle, state actions, sync continuation, proposal,
vote/pillar work, and finalization orchestration. An App-owned process shell supplies only exact monotonic/Unix-time,
signing and VRF custody, tarcap transport, FinalChain account facts, pillar-anchor-state facts, concrete EVM execution,
and best-effort post-ack public observation leaves with canonical requests and typed reports; it retains no protocol
state. Rust-mode pillar network handlers and public readers use `ConsensusNetworkApi` and `ConsensusQueryApi` directly;
there is no C++ pillar manager facade or PBFT runtime mirror.
Terminal `App` teardown stops and joins that process before host configuration is destroyed; restartable stop/start
remains a separate lifecycle operation.
Status-packet and sync-start planning each reuse one coherent application-root status snapshot.

Sync and ordinary PBFT block FinalChain-hash admission now compose the native PBFT and FinalChain roots. Rust captures
the exact sync request identity, performs delayed-hash lookup outside the manager lock, exact-reports the result, and
continues reward admission without a C++ hash-decision branch or standalone validation bridge API.

## Consensus Rewrite Closeout

The authorized consensus rewrite campaign is complete. Native Rust owns consensus state, protocol
planning, scheduling, persistence, finalization/publication authority, network inspection/routing/selection, concrete-
root validation/recovery, and bounded public query/mutation semantics. C++ remains only at the named external leaves in
`doc/consensus_bridge_shim_audit.md`: process mechanics, signing, physical tarcap transport, concrete EVM/
`state_db`, public-client formatting, administration, conformance, and the pure-C++ reference composition.

The live bridge inventory and its guard are the deletion authority for retained CXX modules, functions, carriers,
handles, factories, and consumers. No repository-local consensus campaign queue remains after closeout. New work
requires a demonstrated correctness/parity gap, a named client migration, or explicit authorization to move an external
boundary native through the normal issue/roadmap process; line-count reduction alone is not a roadmap item. Completed
slice sequencing and implementation history remain available in git and must not be recreated as planning documents.
