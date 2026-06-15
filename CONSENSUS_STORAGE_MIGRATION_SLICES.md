# Rustaxa Consensus Storage Migration Slices

This plan breaks the remaining consensus storage migration into bold slices that move Rust-mode consensus fully onto
`rustaxa-storage` without routing production behavior through C++ storage orchestration, bridge-owned batch ids, or
`DbStorage` compatibility APIs.

The target boundary is:

- Rust consensus runtimes own or receive `Arc<rustaxa_storage::Storage>` / `&rustaxa_storage::Storage`.
- `rustaxa-consensus` owns storage fact collection, write ordering, idempotency checks, restart normalization, and batch
  commit/drop behavior for consensus-owned rows.
- `rustaxa-bridge` only adapts CXX-safe DTOs and exposes temporary construction/reporting functions.
- C++ shims may still materialize legacy network, EVM, RPC, and live sidecar objects while those boundaries remain
  outside the current migration.
- Production Rust-mode consensus must not silently delegate storage behavior to legacy C++.

## Current Position

Completed before this plan file:

- PBFT manager startup restore and scalar transition persistence already read/write Rust storage from
  `rustaxa-consensus`.
- Sortition startup replay and PBFT finalization threshold-change persistence already use native Rust storage ownership.
- PBFT finalization storage apply now lives in `rustaxa-consensus::pbft_finalize` and commits ordered
  `rustaxa-storage` batches directly.
- PBFT finalization resume inspection now lives in `rustaxa-consensus::pbft_finalize` and classifies duplicate/restart
  state from direct `rustaxa-storage` reads; the bridge keeps the existing CXX DTO surface.
- The Rust-mode FinalChain shim now serves `getBridgeRoot` / `getBridgeEpoch` from committed `StateAPI` reads or returns
  zero for native/no-bridge-contract runs, so the old unimplemented shim gap no longer blocks PBFT pillar processing.

Known current blockers / unrelated gaps:

- `pbft_manager_test` now gets past the old `getBridgeRoot` abort and fails later on
  `Unsupported Rust PBFT second-finish primary intent 1`.
- `final_chain_test` still has an unrelated `FinalChainTest.remove_jailed_validator_votes_from_total` failure with
  `std::bad_alloc`.

## Slice 1: PBFT Finalization Resume Inspector

Goal: move already-persisted PBFT finalization classification out of `rustaxa-bridge` and into
`rustaxa-consensus::pbft_finalize` over direct `rustaxa-storage` reads.

Status: complete. The production bridge entry now converts the FFI write intent to the domain type, calls the
consensus-owned inspector, and maps the returned plan back to the existing FFI DTO.

Move:

- durable hash-to-period lookup
- period-data payload lookup
- finalized DAG position checks
- finalized transaction location checks
- optional period-lambda lookup
- executed-status lookup
- FinalChain height comparison facts
- complete / replay-needed / missing-primary / conflicting-primary classification

Keep temporarily:

- C++ supplies the accepted finalization write intent and live FinalChain height until the broader FinalChain/PBFT
  runtime boundary moves.
- The CXX bridge returns existing stable status codes and DTO shapes.

Done when:

- Bridge resume-inspection functions are DTO adapters only.
- Resume classification behavior and error codes are owned by `rustaxa-consensus`.
- Restart replay tests cover complete duplicate, replay-needed executed-status tail, missing primary, conflicting
  primary, and dynamic-lambda gap paths.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_finalize`
- `cmake --build /build --target rust_storage_tests && /build/bin/rust_storage_tests`
- focused PBFT restart/finalization C++ test target if available

## Slice 2: Remove PBFT Finalization Bridge Appenders

Goal: remove production dependence on bridge batch append helpers for PBFT finalization side stages.

Status: complete. PBFT manager Rust-mode finalization already commits primary, reward reset, sortition, dynamic-lambda,
and executed-status stages through `apply_pbft_finalization_storage_writes`, which creates and commits a Rust-owned
`rustaxa-storage` batch. The remaining VoteManager reward-reset compatibility path now also uses the Rust-owned apply API
instead of `DbStorage::rustBatchId`, and raw PBFT finalization appenders are no longer exposed through the CXX bridge.
Rust-side appender helpers remain only as compatibility/test scaffolding for staged write coverage.

Move or delete:

- `append_pbft_finalization_storage_write` production callers
- bridge-owned batch append helpers for primary, reward reset, sortition, dynamic lambda, and executed status
- any `BridgeStorage` batch-id routing used only for PBFT finalization production behavior

Keep temporarily:

- Compatibility/test-only helpers may remain behind explicit names or comments if tests still need direct staged append
  coverage.
- C++ may still construct stage DTOs until stage construction moves with the live executor boundary.

Done when:

- All production PBFT finalization persistence goes through one or more Rust-owned apply/session APIs.
- No PBFT finalization production route accepts a bridge batch id.
- Any remaining appender is clearly test/compatibility scaffolding and not called by the PBFT manager overlay.

Validation:

- PBFT finalization Rust domain and bridge tests
- `rust_storage_tests`
- `cmake --build /build --target pbft_manager_test`
- `ctest --output-on-failure -R pbft_manager_test`, expected to reach only known non-storage PBFT runtime gaps

## Slice 3: Vote Persistence Storage Runtime

Goal: move VoteManager persistence from bridge batch helpers into a Rust-owned vote storage runtime over
`rustaxa-storage`.

Status: complete for VoteManager production persistence. `rustaxa-consensus::pbft_vote_storage` now owns Rust storage
batches for locally generated own-vote writes, accepted vote-progress persistence, latest-round `2t+1` bundle writes,
extra reward-vote writes, extra reward-vote cleanup, latest own-vote cleanup, and the finalized reward-vote reset stage
already moved with Slice 2's finalization apply path. The bridge converts CXX DTOs only; VoteManager no longer calls
`DbStorage::rustBatchId` or a bridge batch appender for vote persistence. C++ still materializes live `PbftVote` sidecars
and supplies FinalChain/key-manager facts until those runtime boundaries move.

Move:

- locally generated own-vote persistence
- accepted vote progress persistence
- latest-round `2t+1` bundle writes
- extra reward-vote writes and cleanup
- latest own-vote cleanup
- finalized reward-vote reset persistence

Keep temporarily:

- C++ packet handling, tarcap wrapping, peer-known side effects, slashing transaction submission, and live sidecar
  materialization.
- C++ FinalChain/key-manager fact sourcing until those ports move.

Done when:

- Vote persistence batches are created, ordered, committed, and dropped by `rustaxa-consensus`.
- VoteManager shim does not call `DbStorage::rustBatchId`, bridge batch appenders, or C++ batch commit for production
  vote persistence.
- Rust vote runtime owns idempotency/conflict semantics for its persisted rows.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus verified_votes pbft_vote`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge verified_votes pbft_vote`
- focused VoteManager C++ tests if configured
- `rust_storage_tests`

## Slice 4: Transaction Manager Consensus Storage Runtime

Goal: remove TransactionManager consensus paths from `BridgeStorage` / `DbStorage` and use direct Rust storage plus the
Rust transaction runtime.

Status: complete for TransactionManager-owned consensus storage. `rustaxa-consensus::transaction_storage` now owns Rust
storage batches for DAG-block non-finalized transaction persistence, TransactionManager `TrxCount` status writes used by
finalized-status updates, stored transaction lookup source classification, and restart recovery cleanup of stale
finalized rows in the non-finalized transaction column. Finalized-index membership checks used by filter/verify gates are
routed through the consensus storage runtime, including DAG-block admission duplicate/finalized checks. The bridge adapts
CXX DTOs and no longer owns those batch write groups or lookup classification rules. Direct `storage.0.transaction()`
reads have been removed from the TransactionManager bridge.

Deferred gap: finalized-account queue purge still receives account facts through the current FinalChain compatibility
boundary. Moving that read completely requires the upcoming FinalChain account snapshot migration to `rustaxa-storage`;
forcing it through this slice would change the FinalChain boundary rather than finish TransactionManager-owned storage.

Move:

- DAG-block transaction persistence
- finalized transaction status updates
- recently-finalized and non-finalized sidecar persistence/cleanup
- transaction-count updates
- proposal lookup storage misses
- restart recovery of non-finalized transactions
- finalized-account queue purge storage reads: deferred to the FinalChain account snapshot migration noted above

Keep temporarily:

- EVM gas estimation execution in C++.
- public `Transaction` materialization for API/network boundaries.
- event dispatch and temporary logging.

Done when:

- TransactionManager production storage writes use Rust-owned batches.
- Storage reads that feed deterministic TransactionManager decisions are sourced in `rustaxa-consensus` or a narrow Rust
  transaction storage runtime.
- C++ only receives typed execution/materialization effects.

Validation:

- transaction-manager Rust tests
- affected C++ transaction manager tests
- `rust_storage_tests`
- focused DAG transaction persistence tests

## Slice 5: DAG And Proposed-Block Storage Ownership

Goal: move DAG/proposed-block consensus storage reads/writes into Rust runtimes with direct `rustaxa-storage` access.

Status: complete. `rustaxa-consensus::proposed_blocks` now owns proposed PBFT block storage restore from
`Column::ProposedPbftBlocks` and stale proposed-block cleanup deletes through one Rust-owned batch. The bridge updates
the in-memory proposed-block index only after the consensus storage cleanup commits. `rustaxa-consensus::dag` now owns
storage-backed expired DAG transaction cleanup fact collection by loading expired/remaining DAG block RLPs and finalized
transaction membership directly from `rustaxa-storage`. It also owns finalized DAG cleanup storage apply: counter/index
facts are loaded from canonical DAG storage, expired DAG rows and expired non-finalized transaction rows are committed in
one Rust-owned storage batch, and the bridge only maps the returned side-effect facts for temporary live sidecar cleanup.
Non-finalized DAG sync payload materialization now also loads selected DAG block RLPs and de-duplicated transaction RLP
lookups in `rustaxa-consensus::dag` over direct `rustaxa-storage` reads; the bridge keeps only period/hash selection DTO
mapping and network packet materialization. The remaining DAG runtime scalar storage helpers for block existence/load/save,
proposal-period lookup/write, period PBFT-block hash lookup, persistence counters, and verify-precheck proposal-period
facts now route through `rustaxa-consensus::dag`; the bridge no longer owns direct DAG storage reads/writes in
`rustaxa-bridge/src/dag.rs`.

Move:

- proposed PBFT block restore and cleanup completion
- DAG finalized-order persistence
- finalized counter persistence
- expired block removal
- expired non-finalized transaction cleanup
- non-finalized sync period/index reads
- selected DAG block RLP storage reads

Keep temporarily:

- live DAG block object materialization
- network sync packet construction
- transaction-pool live reads at API boundaries

Done when:

- DAG and proposed-block production paths no longer call bridge batch ids or `DbStorage` compatibility methods for
  consensus storage decisions.
- Rust returns typed live-effect reports for the C++ shim to execute.

Validation:

- DAG Rust tests
- affected DAG/PBFT C++ tests
- `rust_storage_tests`

Validation note: the focused DAG/proposed-block/storage checks pass for the storage-owned proposed-block cleanup and
expired-DAG transaction fact collection steps. The broad `rust_consensus_tests` target currently fails before DAG tests
compile because stale PBFT sync tests still call the removed Slice 2 PBFT finalization bridge appender API; that is
tracked as test compatibility debt outside the DAG storage sub-slice.

## Slice 6: Rewards And FinalChain-Adjacent Status Writes

Goal: finish consensus-owned storage around rewards stats and PBFT/FinalChain status rows without moving EVM execution
inside the PBFT manager.

Status: in progress. Rewards-stat startup reload and cache/clear persistence now live in
`rustaxa-consensus::rewards_stats` over direct `rustaxa-storage` reads and Rust-owned write batches. The bridge creates
the runtime from the consensus storage loader and exposes a DTO-only apply function; the old rewards-stat bridge batch-id
appender has been removed from the CXX bridge surface, the rewards stats shim, and focused C++ test. The delayed PBFT
manager executed-block reset storage write now lives in `rustaxa-consensus::pbft_manager`; the bridge updates the runtime
snapshot only after the consensus storage helper succeeds.

Move:

- rewards stats persistence still exposed through bridge append helpers
- FinalChain-adjacent PBFT manager status writes
- executed-status reset/persist helpers that are not already owned by `rustaxa-consensus`
- startup reload of rewards/stat cache rows where consensus logic depends on them

Keep temporarily:

- FinalChain/EVM execution, receipts, contract execution, and state commits.
- bridge-contract `StateAPI` reads at the accepted FinalChain shim boundary.

Done when:

- Rewards/status persistence that influences PBFT consensus is committed by Rust-owned storage sessions.
- C++ only reports external execution facts and receives typed status/rewards effects.

Validation:

- rewards stats Rust tests
- final-chain execution Rust tests
- focused FinalChain/PBFT C++ targets, noting existing unrelated failures separately
- `rust_storage_tests`

Validation note: the rewards-stat sub-slice passes targeted Rust rewards tests, Rust bridge rewards tests,
`rust_storage_tests`, and the focused C++ `rewards_stats_test` target. The broader `rust_consensus_tests` target still
fails at compile time in stale PBFT sync tests that reference the removed Slice 2 PBFT finalization appender API; the
updated rewards bridge C++ test compiles before that known PBFT sync failure stops the target.

## Slice 7: Pillar Chain Storage And Bridge Root/Epoch Facts

Goal: move pillar-chain consensus storage and bridge root/epoch fact handling to Rust-owned ports while preserving the
current FinalChain/EVM boundary.

Move:

- pillar block persistence
- current pillar block data/own-vote persistence
- pillar-vote restart/recovery storage reads
- bridge root/epoch fact DTOs consumed by Rust pillar planning

Keep temporarily:

- `PillarBlock` / `PillarVote` C++ object materialization
- signing
- event emission
- network request/response handling

Done when:

- Pillar manager production persistence no longer depends on `DbStorage` or bridge batch ids.
- Rust pillar planning receives bridge root/epoch facts through explicit typed facts, not ad hoc C++ calls embedded in
  consensus decisions.

Validation:

- pillar Rust tests
- `pillar_chain_test`
- PBFT pillar-processing subset if available

## Slice 8: Consensus Read Surface Isolation

Goal: separate consensus storage reads from query/API compatibility reads and prevent new `DbStorage` consensus ports.

Move:

- network sync read paths that feed deterministic consensus decisions
- RPC/GraphQL/debug reads that can use read-only Rust storage query APIs
- app status reads that currently force `DbStorage` compatibility methods into Rust-mode consensus ownership

Keep:

- `DbStorage` as the external C++ compatibility shell where no Rust-mode consensus decision depends on it.

Done when:

- Consensus shims do not call `getDB()` / `DbStorage` for deterministic Rust-mode decisions.
- Query/API paths use read-only Rust query APIs or are clearly documented compatibility reads.
- Storage-boundary guard allowlists shrink.

Validation:

- storage-boundary guard
- focused RPC/network sync tests
- `rust_storage_tests`

## Slice 9: Collapse DbStorage To Compatibility Shell

Goal: remove obsolete Rust-mode consensus storage hooks and make regressions visible.

Move/remove:

- stale `rustBatchId` production use
- obsolete bridge storage appender APIs
- allowlisted consensus `DbStorage` routes that now have Rust runtime replacements
- unguarded main-only dependencies in upstream-owned C++ files

Done when:

- Rust-mode consensus production paths no longer use `DbStorage` as a storage API.
- Remaining `DbStorage` usage is legacy/reference, app lifecycle, migration/admin, or query compatibility.
- Boundary tests fail on new bridge-batch or `DbStorage` consensus routes unless explicitly allowlisted.

Validation:

- `make rewrite-validate-fast`
- storage-boundary guard
- `rust_storage_tests`
- targeted C++ shim builds/tests for touched modules
- broader CTest/Python integration gates only when the task owner accepts the cost

## Stop Conditions

Stop and re-plan before continuing a slice if:

- The work requires changing network/tarcap transport, packet wrapping, gossip fanout, EVM execution, receipt execution,
  or contract execution ownership.
- The implementation would silently forward production Rust-mode storage behavior to legacy C++.
- A slice needs broad original upstream C++ edits instead of shim-owned overlay changes.
- The change would weaken or retarget tests to make Rust mode pass.
- Validation exposes a new non-storage PBFT runtime gap, such as the current second-finish primary-intent failure.
