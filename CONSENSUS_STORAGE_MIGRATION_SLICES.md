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

Status: complete. Rewards-stat startup reload and cache/clear persistence now live in
`rustaxa-consensus::rewards_stats` over direct `rustaxa-storage` reads and Rust-owned write batches. The bridge creates
the runtime from the consensus storage loader and exposes a DTO-only apply function; the old rewards-stat bridge batch-id
appender has been removed from the CXX bridge surface, the rewards stats shim, and focused C++ test. The delayed PBFT
manager executed-block reset storage write now lives in `rustaxa-consensus::pbft_manager`; the bridge updates the runtime
snapshot only after the consensus storage helper succeeds. PBFT manager transition cursor/status persistence and
latest-round own-vote cleanup now commit through `rustaxa-consensus::pbft_manager::apply_pbft_manager_transition_storage`
over direct `rustaxa-storage`; the old bridge batch-id transition appender was removed from the CXX bridge surface and
bridge tests now cover only the committed apply route. Successful next-vote manager status writes now route from the
PBFT manager shim through `rustaxa-consensus::pbft_manager::apply_next_voted_status_storage`; the shim still owns vote
generation, gossip, and live next-voted flags until the state-action executor moves to Rust.

Move:

- completed: rewards stats persistence formerly exposed through bridge append helpers
- completed: FinalChain-adjacent PBFT manager status writes for transition resets, executed-block reset, and successful
  next-vote status persistence
- completed: startup reload of rewards/stat cache rows where consensus logic depends on them
- completed: stale PBFT sync C++ tests now use the committed Rust-owned finalization apply API instead of removed bridge
  batch appenders

Keep temporarily:

- FinalChain/EVM execution, receipts, contract execution, and state commits.
- bridge-contract `StateAPI` reads at the accepted FinalChain shim boundary.
- PBFT finalization test-only Rust append helpers remain in Rust unit-test compatibility paths; production and CXX bridge
  validation use committed Rust-owned apply APIs.

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
updated rewards bridge C++ test compiles before that known PBFT sync failure stops the target. The PBFT manager
transition-storage sub-slice passes `cargo test -p rustaxa-consensus pbft_manager` and
`cargo test -p rustaxa-bridge pbft_manager`, plus `rust_storage_tests`. The broad `rust_consensus_tests` build was
rerun for this sub-slice and still stops in the same stale PBFT sync appender compile errors before executing focused
PBFT manager coverage. The next-voted status sub-slice adds focused Rust consensus and bridge coverage for accepting only
the next-voted status family. `pbft_manager_test` builds after the shim route change; running the binary still fails the
known broad Rust-mode PBFT runtime cases (`check_get_eligible_vote_count`, `pbft_manager_run_single_node`,
`pbft_manager_run_multi_nodes`, `check_committeeSize_less_or_equal_to_activePlayers`,
`check_committeeSize_greater_than_activePlayers`) while the DAG-creation suite passes. The stale PBFT sync test-fix
sub-slice rebuilds `rust_consensus_tests` and `/build/bin/rust_consensus_tests` passes all 67 tests.

## Slice 7: Pillar Chain Storage And Bridge Root/Epoch Facts

Goal: move pillar-chain consensus storage and bridge root/epoch fact handling to Rust-owned ports while preserving the
current FinalChain/EVM boundary.

Status: complete. Pillar manager current-block data, own pillar vote, and finalized pillar block persistence now route
through `rustaxa-consensus::pillar_chain` storage helpers over direct `rustaxa-storage`. The C++ pillar manager shim still
materializes `PillarBlock`/`PillarVote`/period-data vote bundle RLP bytes and owns live mirrors, vote aggregation,
gossip, and event emission. The older `BridgeStorage` pillar save/read methods remain as compatibility/query helpers but
are no longer used by the Rust-mode pillar manager production write and restart/recovery read paths. Pillar block
creation also routes state root plus bridge root/epoch through an explicit Rust creation-plan DTO before temporary C++
`PillarBlock` materialization.

Move:

- completed: pillar block persistence for finalized pillar blocks
- completed: current pillar block data/own-vote persistence
- completed: pillar-vote restart/recovery storage reads
- completed: bridge root/epoch fact DTOs consumed by Rust pillar planning

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
- `pillar_chain_test` build and targeted runtime coverage where the broader Rust-mode node path allows it
- PBFT pillar-processing subset if available

Validation note: the pillar storage-write sub-slice passes `cargo test -p rustaxa-consensus pillar_chain`,
`cargo test -p rustaxa-bridge pillar_chain`, `rust_storage_tests`, and `cmake --build /build --target pillar_chain_test
--parallel 12`. Running `/build/bin/pillar_chain_test` still exposes broader Rust-mode runtime gaps outside the moved
storage write route: `PillarChainTest.votes_count_changes` fails after repeated `VoteManager Rust PBFT vote admission
weight mismatched legacy sidecar hydration` errors, and the binary later aborts in `FinalChain::getBridgeRoot` because
block 3 lacks committed external-EVM state. Keep the pillar storage route committed, but treat those runtime failures as
follow-up PBFT/FinalChain boundary work rather than storage-write blockers.

Validation note: the pillar restart/recovery read sub-slice passes `cargo test -p rustaxa-consensus pillar_chain`,
`cargo test -p rustaxa-bridge pillar_chain`, `rust_storage_tests`, `cmake --build /build --target pillar_chain_test
--parallel 12`, and a narrow `/build/bin/pillar_chain_test` filter covering pillar DB, block/vote serialization, compact
signature, and Rust-inspected pillar vote validation. The broad unfiltered `pillar_chain_test` runtime remains covered by
the storage-write note above and is still blocked by PBFT/FinalChain boundary issues rather than pillar storage reads.

Validation note: the bridge root/epoch creation-plan sub-slice passes `cargo test -p rustaxa-consensus pillar_chain`,
`cargo test -p rustaxa-bridge pillar_chain`, `rust_storage_tests`, `cmake --build /build --target pillar_chain_test
--parallel 12`, and the same narrow `/build/bin/pillar_chain_test` filter. The C++ shim still calls FinalChain/EVM for
the root/epoch facts, but Rust now consumes those facts through typed planning before C++ materializes the temporary
pillar block object.

## Slice 8: Consensus Read Surface Isolation

Goal: separate consensus storage reads from query/API compatibility reads and prevent new `DbStorage` consensus ports.

Status: replanned / partially complete. Gas-pricer finalized-history restoration now performs its FinalChain
`LAST_NUMBER` and period-data walk inside `rustaxa-consensus::gas_pricer` over native `rustaxa-storage`; the bridge
adapter only passes the shared storage handle and oracle lock. This removes bridge-local raw storage reads and
period-data gas-price decoding from the deterministic gas-pricer initialization path. The storage-boundary guard now also
rejects new C++ `getDB()` additions by default; RPC/GraphQL compatibility reads must carry an inline
`RUSTAXA_QUERY_COMPAT_READ` marker so query debt is visible instead of silently expanding.
Existing RPC/Debug query reads now carry that marker. GraphQL does not currently add new `getDB()`/`rustStorage()`
extractor calls, but it still has pre-existing `db_` compatibility reads that are tracked in Slice 13.
PBFT manager startup replay now loads finalized period data, closest dynamic-lambda facts, and finalized DAG hash order
through a `rustaxa-consensus::pbft_manager` storage helper over native `rustaxa-storage`; the bridge only adapts DTOs,
and the C++ shim only materializes temporary `PeriodData` objects for the existing live replay calls.
PBFT finalization dynamic-lambda planning now also loads the prior saved period lambda through
`rustaxa-consensus::pbft_finalize` over native `rustaxa-storage` instead of asking the PBFT manager shim to call
`DbStorage::getPeriodLambda`.

Boundary replan: this slice is no longer a single implementation unit. The remaining read-surface work splits across
query compatibility, PBFT-chain/proposed-block live sidecars, PBFT sync/network egress, and FinalChain/EVM facts. Those
are tracked below as Slices 10-23 so implementation can continue without broadening Slice 8 into unrelated subsystem
ownership changes. Slices 16-23 are the additional closure plan added after the boundary-slice evaluation.

Post-boundary-slice evaluation: Slice 8 cannot be marked fully complete yet. Slices 10-18, 19, and 22-26 retired the
PBFT-chain, proposed-block, PBFT sync egress, RPC/GraphQL query-marker, guard, VoteManager DPoS, pillar-sync DPoS,
gas-pricer runtime, and TransactionManager account/finalized read surfaces that had clear Rust replacements. The
remaining read-surface blockers are now temporary C++ sidecar materialization, network/app compatibility constructors,
DAG proposal-validation live-runtime facts, and the explicit FinalChain/EVM publication/account boundary. Completing
those would change larger subsystem ownership rather than finish Slice 8 as originally scoped.

Move:

- completed: network sync read paths that feed deterministic consensus decisions
- in progress: RPC/GraphQL/debug reads that can use read-only Rust storage query APIs
- in progress: app status/finalized-history reads that currently force bridge/storage compatibility methods into
  Rust-mode consensus ownership

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

Validation note: the gas-pricer read-surface sub-slice passes `cargo test -p rustaxa-consensus gas_pricer`,
`cargo test -p rustaxa-bridge gas_pricer`, `rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, `gas_pricer_shim_test`, and `gas_pricer_test`.

Validation note: the guard-tightening sub-slice extends the storage-boundary guard self-test for new `getDB()` additions
and documented RPC/GraphQL compatibility reads. It passes `scripts/rewrite_storage_boundary_guard.sh --self-test` and
`scripts/rewrite_storage_boundary_guard.sh`.

Validation note: the RPC query annotation sub-slice marks current RPC/Debug `getDB()` and direct `rustStorage()` query
reads with `RUSTAXA_QUERY_COMPAT_READ`, leaving them as visible compatibility reads until read-only Rust query APIs replace
them. It passes the storage-boundary guard self-test and current-diff guard, `git diff --check`,
`cmake --build /build --target rpc_plugin --parallel 12`, `cmake --build /build --target rpc_test --parallel 12`,
`/build/bin/rpc_test`, and `rust_storage_tests`.

Validation note: the PBFT manager startup replay read sub-slice moves the finalized-history reads used by restart replay
from shim-local `DbStorage` compatibility calls to `rustaxa-consensus::pbft_manager` over direct `rustaxa-storage`. It
keeps C++ `PeriodData` materialization and live replay calls temporary. Validate with PBFT manager Rust consensus/bridge
tests, `rust_storage_tests`, the storage-boundary guard, and focused `pbft_manager_test` build/runtime coverage; broad
runtime failures remain classified under the existing non-storage PBFT runtime gaps when reproduced. The sub-slice passes
`cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo check --manifest-path rust/Cargo.toml -p
rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`, the storage-boundary guard self-test/current-diff guard,
`git diff --check`, `rust_storage_tests`, and `cmake --build /build --target pbft_manager_test --parallel 12`. Running
`/build/bin/pbft_manager_test` still fails the known broad Rust-mode PBFT runtime cases
(`check_get_eligible_vote_count`, `pbft_manager_run_single_node`, `pbft_manager_run_multi_nodes`,
`check_committeeSize_less_or_equal_to_activePlayers`, and `check_committeeSize_greater_than_activePlayers`), while all
8 `PbftManagerWithDagCreation` tests pass.

Validation note: the PBFT finalization dynamic-lambda read sub-slice moves the prior-period lambda lookup used by
finalization planning from shim-local `DbStorage::getPeriodLambda` to `rustaxa-consensus::pbft_finalize` over direct
`rustaxa-storage`; the bridge maps only the existing CXX `PeriodLambda` DTO and the PBFT manager overlay passes that
explicit found/value pair into the existing planner. It passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pbft_finalize`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_finalize`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`,
`git diff --check`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
and `cmake --build /build --target pbft_manager_test --parallel 12`. Broad `/build/bin/pbft_manager_test` runtime
coverage was attempted and terminated after reproducing the existing non-storage PBFT runtime gap: balance/final-chain
execution mismatches, `Rust FinalChain DPoS snapshot for block 220 is not implemented`, PBFT sync period drift, and
proposed-block bundle validation blocked behind the failed sync state. The focused
`/build/bin/pbft_manager_test --gtest_filter=PbftManagerWithDagCreation.*` attempt also reproduced the same execution
class via `trx_generation` (`getNumTransactionExecuted()` stayed at `111` while the test expected `1111`) and did not
reach a clean gtest summary before termination. These failures do not point at the migrated period-lambda storage read.

## Slice 9: Collapse DbStorage To Compatibility Shell

Goal: remove obsolete Rust-mode consensus storage hooks and make regressions visible.

Status: replanned / stopped at boundary. The public `DbStorage::rustBatchId` shim method has been removed now that PBFT finalization,
VoteManager, sortition, pillar, DAG/proposed-block, transaction, and PBFT manager transition production writes no longer
route through bridge-owned batch ids. The storage shim still owns an internal Rust batch map for temporary
legacy-compatible `insert/remove/commitWriteBatch` behavior, but consensus callers no longer have a public API for
extracting a Rust batch id.
PBFT finalization bridge-owned batch appender scaffolding has also been deleted from `rustaxa-bridge`; bridge tests now
exercise the production `apply_pbft_finalization_storage_writes` API, which creates and commits the Rust-owned batch in
`rustaxa-consensus`.
The remaining PBFT finalization staged append helpers in `rustaxa-consensus` are now internal/test-only; the public
storage API for finalization is the Rust-owned apply function.
Remaining `DbStorage` references found by the Slice 9 audit are not stale bridge-batch appender routes. They are either
legacy/reference implementations under original upstream paths, storage-shim internals, query/admin compatibility, or
shim-owned live boundaries that still depend on FinalChain/external-EVM state, DAG/network synchronization, or temporary
C++ sidecar materialization. Removing those in this slice would require broad original C++ edits or moving FinalChain/EVM
execution ownership, so the slice stops here under the plan stop conditions.

Post-boundary-slice evaluation: Slice 9 also cannot be marked fully complete yet. The public `rustBatchId` escape hatch
and obsolete PBFT finalization appenders are gone, operation-level Rust storage handle extraction has been collapsed into
constructor/runtime ownership, and the Slice 15 guard prevents new C++ storage/DPoS regressions. The remaining closures
are not cleanup-only work; they require dedicated FinalChain/EVM publication/account ownership and DAG proposal-validation
runtime slices.

Move/remove:

- completed: stale public `rustBatchId` production escape hatch
- completed: obsolete PBFT finalization bridge storage appender APIs
- completed: public PBFT finalization staged appender APIs
- audited: no remaining bridge-batch consensus `DbStorage` routes with already-available Rust storage replacements
- deferred: remaining `DbStorage` routes tied to legacy/reference code, FinalChain/EVM, DAG/network, or sidecar boundaries
- deferred: unguarded main-only dependency audit beyond shim-owned files; broad upstream-owned edits require re-planning

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

Validation note: the `rustBatchId` cleanup sub-slice removes the unused public storage-shim method while preserving
private shim batch handling for legacy-compatible `insert/remove/commitWriteBatch`. It passes
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`,
`git diff --check`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
and `cmake --build /build --target core_libs --parallel 12`.

Validation note: the PBFT finalization appender cleanup removes the obsolete `rustaxa-bridge` test-only appender helpers
that wrote into bridge-owned batch ids. Bridge finalization tests now use the production Rust-owned
`apply_pbft_finalization_storage_writes` path for primary, dynamic-lambda, executed-status, sortition, and reward-vote
reset stages. It passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo check --manifest-path
rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_finalize`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, `git diff
--check`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, and
`cmake --build /build --target pbft_manager_test --parallel 12`.

Validation note: the PBFT finalization consensus appender API cleanup makes the staged append dispatcher private and
keeps the individual staged append helpers test-only. This preserves module-level staged write tests while making
`apply_pbft_finalization_storage_writes` the only public finalization storage apply API. It passes
`cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo check --manifest-path rust/Cargo.toml -p
rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_finalize`, `scripts/rewrite_storage_boundary_guard.sh
--self-test`, `scripts/rewrite_storage_boundary_guard.sh`, `git diff --check`, `cmake --build /build --target
rust_storage_tests --parallel 12`, and `/build/bin/rust_storage_tests`.

## Boundary Replan: Remaining Storage/C++ Boundaries

Replan date: 2026-06-15.

The Slice 9 audit found no remaining public bridge-batch escape hatch or PBFT finalization appender route. The remaining
`DbStorage`/C++ storage references cluster into five boundaries. They should be moved as separate slices because each
boundary changes a different owner:

- PBFT-chain head/history and proposed-block live sidecars still use `DbStorage` in shim-owned consensus classes.
- PBFT sync and DAG/network packet handlers still carry `DbStorage` for network egress and sync materialization.
- RPC/GraphQL/debug paths still expose compatibility query reads.
- FinalChain/EVM execution and DPoS/account facts still run through the accepted external-EVM/state boundary.
- App startup, admin/migration, storage-shim internals, tests, and legacy/reference code remain compatibility shell
  responsibilities rather than consensus storage ownership.

Stop rule for the next work: do not collapse these into one broad cleanup. Each slice must either retire a concrete
`DbStorage` consensus route or document that the route is query/admin/legacy compatibility. Stop again if the slice
requires moving external EVM execution, peer transport, tarcap scheduling, or broad upstream-owned C++ files before its
own Rust storage owner is ready.

## Slice 10: PBFT Chain Storage Runtime

Goal: move PBFT-chain head restore, head persistence, block-existence checks, and PBFT block payload lookups out of the
`PbftChain` shim's `DbStorage` calls and into `rustaxa-consensus` over direct `rustaxa-storage`.

Status: complete. `rustaxa-consensus::pbft_chain` now owns PBFT-chain head restore/default initialization, legacy
head JSON parsing, last non-null anchor recovery, PBFT block existence checks, and PBFT block RLP lookup by hash over
direct `rustaxa-storage`. The bridge exposes DTO-only storage helpers, and the C++ PBFT chain shim no longer calls
`DbStorage::getPbftHead`, `DbStorage::savePbftHead`, `DbStorage::pbftBlockInDb`, or `DbStorage::getPbftBlock` for
Rust-mode production behavior. The shim still receives `DbStorage` as the compatibility owner of the Rust storage handle
and still materializes temporary C++ `PbftBlock` sidecars from Rust-returned canonical RLP.

Current boundary:

- `libraries/core_libs/consensus/shims/pbft_chain_shim/src/pbft_chain_shim.cpp` still calls `getPbftHead`,
  `savePbftHead`, `pbftBlockInDb`, and `getPbftBlock`.
- The shim also recovers the last non-null anchor by walking PBFT blocks through `DbStorage`.
- C++ still materializes `PbftBlock` objects for public consensus APIs and existing callers.

Move:

- persisted PBFT-chain head load/initialize
- last-non-null-anchor recovery from canonical PBFT block bytes
- PBFT block existence lookup
- PBFT block RLP lookup for temporary C++ materialization
- head persistence/update writes currently emitted through `savePbftHead`

Keep temporarily:

- `PbftBlock` C++ object materialization at the public shim boundary
- JSON string compatibility for the legacy PBFT-head payload until the storage layout is intentionally changed
- network/API callers that still expect C++ `PbftBlock` instances

Done when:

- `PbftChain` Rust-mode production paths no longer call `DbStorage` methods for PBFT-chain storage facts or writes.
- `rustaxa-consensus` owns PBFT-chain storage recovery semantics, including corrupt/missing block behavior.
- C++ only maps Rust lookup results to temporary `PbftBlock` sidecars.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_chain`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_chain`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- `cmake --build /build --target pbft_chain_test --parallel 12`
- focused PBFT manager/DAG tests that exercise chain-head recovery, noting pre-existing non-storage runtime gaps

Validation note: the PBFT-chain storage-runtime slice passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pbft_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_chain`,
`cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `cmake --build
/build --target pbft_chain_test --parallel 12`, `/build/bin/pbft_chain_test`, the storage-boundary guard
self-test/current-diff guard, and `git diff --check`.

## Slice 11: Proposed-Block Constructor And Persistence Runtime

Goal: remove the remaining `DbStorage` ownership from `ProposedBlocks` by constructing the Rust index with native storage
and routing save/restore/cleanup through one Rust-owned proposed-block runtime.

Status: complete. `rustaxa-consensus::proposed_blocks` now owns proposed-block save validation and persistence through
`save_proposed_block_storage`, in addition to the existing Rust-owned restore and stale cleanup paths. The bridge exposes
a storage-backed push API that commits the proposed-block row before mutating the Rust live index, preserving legacy
save-before-duplicate-detection ordering. The C++ shim no longer calls `DbStorage::saveProposedPbftBlock` and keeps
`DbStorage` only as the lifetime owner for the Rust storage handle while temporary C++ `PbftBlock` sidecar materialization
remains at the public API/network boundary.

Current boundary:

- `ProposedBlocks` stores `std::shared_ptr<DbStorage>` and calls `saveProposedPbftBlock`.
- Restore and cleanup already call Rust storage helpers, but they still reach storage by extracting `db_->rustStorage()`.
- The PBFT manager overlay still gets proposed-block snapshots as C++ sidecars.

Move:

- constructor wiring from `DbStorage` to an explicit Rust storage handle/runtime
- proposed-block save when `save_to_db` is true
- restore and cleanup routing so the shim no longer owns storage extraction
- stale in-memory index updates after successful Rust storage commits

Keep temporarily:

- `PbftBlock` C++ materialization for proposer, validation, and network bundle paths
- in-memory proposed-block snapshot APIs required by current PBFT sync egress

Done when:

- `ProposedBlocks` no longer stores or requires `DbStorage` in Rust mode.
- Save/restore/cleanup are one Rust storage family with clear commit-before-live-mutation ordering.
- Remaining C++ sidecar APIs are explicitly live-object compatibility, not storage ownership.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus proposed_blocks`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge proposed_blocks`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- `cmake --build /build --target pbft_manager_test --parallel 12`
- focused proposed-block/PBFT sync tests where available

Validation note: the proposed-block persistence-runtime slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml
-p rustaxa-consensus proposed_blocks`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge proposed_blocks`,
`cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `cmake --build
/build --target pbft_manager_test --parallel 12`, `/build/bin/pbft_manager_test
--gtest_filter=PbftManagerWithDagCreation.*`, the storage-boundary guard self-test/current-diff guard, and `git diff
--check`.

## Slice 12: PBFT Sync And Network Egress Storage Queries

Goal: move storage-backed PBFT sync payload and network egress materialization into Rust query/runtime helpers while
keeping peer transport, packet wrapping, and tarcap scheduling in C++.

Status: complete. PBFT sync egress now routes through `rustaxa-consensus::pbft_sync` and
`rustaxa-bridge::pbft_sync::load_pbft_sync_egress_payload`, which load canonical `PeriodData` bytes directly from the
PBFT manager runtime's Rust storage handle and decide reward-vote bundle attachment from explicit packet/reward-vote
facts. The old raw `pbft_manager_runtime_period_data_raw` bridge helper and `PbftManager::getPbftSyncPeriodDataRaw`
shim method were removed so Rust-mode network egress has a single typed sync payload route. Latest/v4 packet handlers
still own packet encoding, tarcap transport, peer state, and temporary `PbftVote` sidecars. DAG sync payload
materialization was already owned by the DAG runtime's storage helpers from Slice 5; this slice keeps that boundary and
does not move peer request scheduling or packet wrapping.

Current boundary:

- completed: latest/v4 PBFT sync egress uses a Rust-owned typed payload helper instead of a raw PBFT manager storage
  getter.
- completed: Rust-mode latest/v4 PBFT/DAG sync handlers compile out `DbStorage`; legacy `DbStorage` branches remain only
  for `RUSTAXA_ENABLE=0`.
- completed earlier: DAG sync storage payload reads use the DAG runtime storage helpers and return typed block/transaction
  payload DTOs for C++ packet materialization.
- remaining boundary: proposed-block bundle egress still enumerates temporary C++ sidecars because live proposed-block
  materialization is outside this packet-storage slice.

Move:

- PBFT sync period-data lookup and reward-vote attachment facts into Rust-owned sync-query helpers
- DAG sync payload storage reads that still happen in network handlers into existing `rustaxa-consensus::dag` storage
  query APIs or narrow new APIs
- packet payload selection and de-dup facts into Rust DTOs while C++ keeps final packet encoding/sending

Keep temporarily:

- tarcap peer transport, packet sealing, gossip fanout, scheduling, and peer-known state
- final C++ packet object construction until the network ingress/egress arena work lands
- legacy pure-C++ handler branches behind `RUSTAXA_ENABLE=0`

Done when:

- completed: Rust-mode network sync handlers do not use `DbStorage` for deterministic consensus storage reads.
- completed for PBFT/DAG sync payloads: C++ network code receives typed sync payload/effect DTOs and only performs
  transport work plus temporary sidecar packet wrapping.
- completed: remaining `DbStorage` members in these handlers are compiled only for legacy/reference mode.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag pbft_manager`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag pbft_manager`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- focused DAG/PBFT sync C++ targets
- storage-boundary guard self-test/current-diff guard

Validation note: the PBFT sync egress payload slice passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pbft_sync`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_sync`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-consensus dag`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
dag`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`, `cargo test --manifest-path
rust/Cargo.toml -p rustaxa-bridge pbft_manager`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `cmake --build /build --target pbft_manager_test --parallel 12`,
`/build/bin/pbft_manager_test --gtest_filter=PbftManagerWithDagCreation.*`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

## Slice 13: Query Compatibility Read Split

Goal: separate RPC/GraphQL/debug compatibility reads from consensus storage ownership and prevent unmarked query debt
from looking like unfinished consensus migration.

Status: complete. Existing GraphQL `DbStorage` reads are now marked with `RUSTAXA_QUERY_COMPAT_READ`, matching the
already-marked RPC/Debug query reads. The storage-boundary guard self-test now explicitly rejects unmarked GraphQL storage
reads and accepts documented GraphQL compatibility reads, so query debt remains visible without being counted as
unfinished consensus storage ownership.

Current boundary:

- completed: RPC and Debug `getDB()` reads are marked with `RUSTAXA_QUERY_COMPAT_READ`.
- completed: GraphQL query storage owner plus `getPbftBlock`, `getDagBlocksAtLevel`, and
  `getFinalizedDagBlockByPeriod` reads are marked with `RUSTAXA_QUERY_COMPAT_READ`.
- These reads serve API responses and do not block consensus storage ownership, but they remain visible query
  compatibility debt.

Move:

- add explicit query compatibility markers to existing GraphQL storage reads, or route simple read-only responses through
  Rust query APIs when the Rust API already exists
- document any read-only Rust query helper introduced for RPC/GraphQL as API/query infrastructure, not consensus runtime
  ownership
- tighten the storage-boundary guard only after current GraphQL debt is annotated or routed

Keep temporarily:

- public JSON/GraphQL response materialization in C++
- FinalChain/account/EVM query access through the accepted FinalChain boundary

Done when:

- completed: every Rust-mode RPC/GraphQL/debug `DbStorage` read is either replaced by a read-only Rust query API or marked as
  `RUSTAXA_QUERY_COMPAT_READ`.
- completed: the storage-boundary guard rejects new unmarked GraphQL storage reads.
- completed: query compatibility debt is not counted as consensus storage migration work.

Validation:

- `scripts/rewrite_storage_boundary_guard.sh --self-test`
- `scripts/rewrite_storage_boundary_guard.sh`
- `git diff --check`
- `cmake --build /build --target rpc_plugin --parallel 12`
- `cmake --build /build --target rpc_test --parallel 12 && /build/bin/rpc_test`
- GraphQL build/test target if configured

Validation note: the GraphQL query compatibility split passes `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, `git diff --check`, `cmake --build /build --target rpc_plugin --parallel
12`, `cmake --build /build --target rpc_test --parallel 12`, and `/build/bin/rpc_test`. The first parallel `rpc_test`
build attempt overlapped with `rpc_plugin` in the shared `jsonrpccpp` external configure step and failed in generated
CMake config output; rerunning `rpc_test` alone with `--parallel 12` succeeded.

## Slice 14: FinalChain/EVM Fact Port For Consensus

Goal: make FinalChain/EVM-derived facts consumed by consensus explicit Rust ports so PBFT, DAG, votes, pillar, rewards,
and transaction cleanup no longer call C++ FinalChain or `DbStorage` for deterministic consensus facts.

Status: partially complete. The VoteManager DPoS/PBFT vote fact sub-slice now consumes the existing Rust FinalChain
grouped PBFT fact port instead of issuing direct C++ FinalChain DPoS count and last-block reads from the VoteManager
shim. The remaining account, bridge, system-transaction, and arbitrary EVM facts stay open because they cross the
accepted external-EVM/state boundary.

Current boundary:

- External EVM execution, state commits, receipts, bridge root/epoch reads, DPoS vote counts, account snapshots, and
  system transaction construction remain outside the consensus storage migration.
- This is the accepted boundary that explains current PBFT manager runtime failures such as external-EVM state gaps,
  DPoS snapshot gaps, and transaction execution-count mismatches.
- Finalized-account transaction queue purge was explicitly deferred from Slice 4 until FinalChain account snapshots move
  to Rust-accessible facts.

Completed in the VoteManager DPoS fact-port sub-slice:

- `VoteManager::addVerifiedVoteWithReport`, `generateVoteWithWeight`, `validateVote`, `getPbftTwoTPlusOne`, and
  `genAndValidateVrfSortition` now request DPoS voter/total vote facts through
  `BridgeFinalChain::collect_pbft_final_chain_facts`.
- The shim converts unavailable FinalChain facts into the existing Rust vote-validation external-fact statuses instead
  of catching individual C++ `dposEligible*`/`lastBlockNumber` calls.
- VoteManager no longer directly calls `dposEligibleVoteCount`, `dposEligibleTotalVoteCount`, or `lastBlockNumber` for
  the migrated DPoS fact paths.

Completed in the pillar-sync DPoS fact-port sub-slice:

- PBFT manager pillar-vote sync validation now resolves each pillar voter weight through
  `BridgeFinalChain::collect_pbft_final_chain_facts` instead of calling C++ `FinalChain::dposEligibleVoteCount`.
- The existing missing/future/zero-weight behavior is preserved: unavailable facts still reject the vote as zero weight
  for the deterministic Rust pillar bundle planner.

Completed in the pillar-manager DPoS fact-port sub-slice:

- Pillar vote validation, insertion planning, and threshold calculation now request voter eligibility/weight and total
  vote facts through `BridgeFinalChain::collect_pbft_final_chain_facts`.
- `PillarChainManager` no longer directly calls `dposIsEligible`, `dposEligibleVoteCount`, or
  `dposEligibleTotalVoteCount` in the migrated consensus paths.
- The full FinalChain DPoS snapshot used for pillar block creation remains an explicit external boundary for Slices 16
  and 21.

Move:

- define narrow Rust consensus fact ports for DPoS vote counts/eligibility, VRF keys, bridge root/epoch, account nonce
  and balance snapshots, and FinalChain height/header facts
- feed those ports from the existing FinalChain shim first, then replace individual facts with Rust storage/state reads
  as `rustaxa-consensus::final_chain` and `rustaxa-storage` coverage grows
- move finalized-account queue purge facts out of C++ TransactionManager once account snapshots are available
- convert PBFT/vote/DAG/pillar callers from ad hoc C++ FinalChain calls to typed fact DTOs or Rust runtime views

Keep temporarily:

- arbitrary EVM execution, gas/state execution, receipts, contract execution, and `state_db` commits
- C++ `StateAPI` adapter for the external-EVM boundary
- public FinalChain API materialization for RPC/GraphQL

Done when:

- Consensus decisions consume explicit Rust fact ports instead of directly calling C++ FinalChain/DbStorage.
- Remaining C++ FinalChain work is limited to external EVM execution/state access and public query materialization.
- PBFT runtime failures caused by missing DPoS/account/bridge facts have tracked Rust fact-port owners.

Validation:

- Rust final-chain/consensus tests for each fact port
- focused PBFT manager, vote, DAG, pillar, and transaction-manager C++ tests for migrated facts
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- FinalChain smoke/subsystem validation when a migrated fact changes runtime behavior

Validation note: the VoteManager DPoS fact-port sub-slice passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-bridge final_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_vote`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_vote`, `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `cmake
--build /build --target vote_test --parallel 12`, `/build/bin/vote_test`, `cmake --build /build --target
verified_votes_shim_test --parallel 12`, `/build/bin/verified_votes_shim_test`, `cmake --build /build --target
pbft_manager_test --parallel 12`, `/build/bin/pbft_manager_test --gtest_filter=PbftManagerWithDagCreation.*`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the pillar-sync DPoS fact-port sub-slice passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-bridge final_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_chain`, `cmake --build /build --target pbft_manager_test
--parallel 12`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`. The focused `/build/bin/pbft_manager_test --gtest_filter=PbftManagerWithDagCreation.*` runtime was attempted
and reproduced the existing non-storage FinalChain/EVM execution-count gap in `trx_generation`
(`getNumTransactionExecuted()` stayed at `111` while the test expected `1111`), then stayed running until terminated.

Validation note: the pillar-manager DPoS fact-port sub-slice passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pillar_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain`, `cargo
test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`, `cmake --build /build --target pillar_chain_test
--parallel 12`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`. The broad `PillarChainTest.*` runtime still reaches the known unimplemented FinalChain/EVM boundaries
documented under Slice 16.

## Slice 15: Compatibility Shell Audit And Guard Hardening

Goal: after Slices 10-14 land, make the remaining `DbStorage` surface intentionally compatibility-only and harden guards
so regressions are caught before review.

Status: complete for the storage-boundary guard/audit cleanup. The remaining `DbStorage`/FinalChain references are
categorized as compatibility shell, legacy/reference code, app/query/network materialization, or the explicit
FinalChain/EVM/DAG/transaction boundaries tracked in Slice 14 and the Slice 8/9 replanning notes.

Move/remove:

- completed: remove obsolete `DbStorage` members and raw PBFT sync helpers where Slices 10-13 made them unused
- completed: delete bridge DTOs and helper functions made unused by the new runtimes
- completed: harden `scripts/rewrite_storage_boundary_guard.sh` so new direct C++ DPoS fact reads from consensus
  consumers fail the rewrite guard
- completed: keep RPC/GraphQL query compatibility explicit through `RUSTAXA_QUERY_COMPAT_READ`
- completed: update `PLAN.md` and this file with the compatibility-shell contract

Keep:

- storage shim internals that implement the public C++ compatibility API
- legacy/reference implementation files and pure-C++ validation routes
- app startup, migration/admin, backup/pruning, and read-only query compatibility until those areas receive their own
  rewrite tracks
- explicit FinalChain/EVM/account, DAG proposal-validation, and transaction-account boundaries tracked by Slice 14

Done when:

- completed for moved storage families: PBFT chain/proposed blocks/sync, rewards stats, pillar storage, TransactionManager
  storage, PBFT manager startup/finalization, and VoteManager DPoS reads no longer add new C++ `DbStorage` or direct DPoS
  routes.
- remaining `DbStorage` usage is categorized as storage shim, legacy/reference, app lifecycle/admin, tests, marked query
  compatibility, or explicit Slice 14 boundary work.
- new consensus/storage regressions fail fast in `make rewrite-validate-fast`.

Validation:

- `make rewrite-validate-fast`
- `scripts/rewrite_storage_boundary_guard.sh --self-test`
- `scripts/rewrite_storage_boundary_guard.sh`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- targeted C++ shim builds/tests for every touched module

Validation note: the Slice 15 guard-hardening sub-slice passes `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`. The pre-commit hook also ran
`make rewrite-validate-fast` for the preceding Slice 14 commit successfully; remaining clippy output is the known
pre-existing warning set documented in earlier slice validation notes.

## Slice 16: Pillar FinalChain Fact Completion

Goal: finish the remaining pillar-chain consensus fact reads so pillar validation and threshold decisions consume typed
Rust FinalChain fact ports instead of direct C++ FinalChain calls.

Status: complete for direct pillar DPoS eligibility/count fact reads. Pillar block creation still depends on the
external FinalChain DPoS snapshot and bridge-root boundaries tracked by Slices 14 and 21.

Current boundary:

- Pillar vote validation, insertion planning, and threshold calculation now consume
  `BridgeFinalChain::collect_pbft_final_chain_facts` for voter weight/eligibility and total vote count facts.
- Pillar block creation still temporarily materializes C++ pillar objects and requests the full FinalChain DPoS snapshot,
  but storage writes and deterministic planning already live in Rust.

Move:

- completed: reused the Rust bridge helper that collects the needed pillar voter and threshold facts through
  `BridgeFinalChain::collect_pbft_final_chain_facts`
- completed: updated `PillarChainManager` shim callers to convert unavailable/zero fact results into the existing Rust
  pillar planner statuses instead of calling C++ `dposEligible*` helpers
- keep C++ object materialization only for the public/live sidecar API
- completed by Slice 15: the storage-boundary guard rejects new consensus direct DPoS reads from unapproved files

Keep temporarily:

- public C++ pillar block/vote materialization
- bridge root/epoch and other external-EVM facts already tracked under Slice 14

Done when:

- no `dposIsEligible`, `dposEligibleVoteCount`, or `dposEligibleTotalVoteCount` calls remain in pillar-manager
  consensus paths
- pillar validation/insertion paths preserve unavailable/zero fact behavior through Rust planner statuses
- Slice 14 can mark direct pillar DPoS eligibility/count facts complete while keeping the full DPoS snapshot boundary open

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pillar_chain`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain`
- `cmake --build /build --target pillar_chain_test --parallel 12`
- focused `pillar_chain_test` filters that avoid the known broad PBFT/FinalChain runtime gap
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`

Validation note: the pillar FinalChain fact-completion sub-slice passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pillar_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pillar_chain`, `cargo
test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`, `cmake --build /build --target pillar_chain_test
--parallel 12`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`. The broad `/build/bin/pillar_chain_test --gtest_filter='PillarChainTest.*'` runtime was attempted and
reproduced the known unimplemented FinalChain/EVM runtime boundaries: `votes_count_changes` hit the Rust FinalChain
DPoS snapshot gap for block 16, and `finalize_root_in_pillar_block` aborted because bridge-root reads require committed
external-EVM state for block 3.

## Slice 17: DAG Proposal Fact Port

Goal: move DAG proposer and DAG manager proposal-validation facts behind Rust fact ports so DAG consensus no longer
mixes `DbStorage` proposal-period reads with C++ FinalChain height/DPoS/gas-limit calls.

Status: stopped at live-runtime boundary. Direct C++ FinalChain calls for DAG authorization and proposer finalized-height
checks are removed from the Rust-mode DAG proposer/manager shims. A fuller `DagProposalFacts` envelope remains open
because transaction selection, gas estimation, tip metadata lookup, and DAG block materialization still cross existing
C++ live-runtime boundaries. VDF proof generation, pre-proof difficulty facts, and DAG proposal/verification sortition
runtime params now route through Rust. Moving only one remaining fact now would add another partial DTO without removing
the remaining C++ proposal owner, so this slice needs a dedicated DAG proposal-runtime replan before more implementation.

Current boundary:

- `DagBlockProposer` now reads finalized height through `BridgeFinalChain::collect_pbft_final_chain_facts` and DAG
  DPoS/VRF authorization through the shim-owned `BridgeFinalChain` runtime, not the C++ FinalChain compatibility API.
- `DagManager` verification precheck, proposal-period lookup, and period-block-hash lookup already use the
  storage-owned Rust DAG runtime, and DPoS/VRF authorization now uses the shim-owned `BridgeFinalChain` runtime.
- `DagBlockProposer` now asks the Rust TransactionManager runtime for proposer-sharded transaction packing, so the
  deterministic sender-shard filter runs inside the Rust-owned packing session before C++ materializes or estimates gas
  for candidates. Rust now also plans DAG block construction facts: legacy transaction-gas summation, tip-pruning
  decisions, and selected-tip ordering. Producer-side VDF proof generation now calls the Rust VDF sortition bridge
  directly and materializes a legacy `VdfSortition` object only from the Rust-produced payload. The proposer and verifier
  now consume Rust-native `SortitionRuntimeParams` from the Rust-backed sortition manager instead of materializing the C++
  `SortitionParams` compatibility DTO for VDF planning/verification. It still computes genesis gas-limit constants in
  C++ and keeps tip metadata lookup and `DagBlock` object creation in the compatibility shell.
- Re-audit after Slice 18 confirms the remaining DAG proposal work is not blocked by TransactionManager account/finalized
  routing anymore; it is blocked by ownership of the DAG proposal runtime itself.

Move:

- partially complete: direct DAG FinalChain authorization calls now route through `FinalChain::rustFinalChainForRust()`
  and `BridgeFinalChain::get_dag_dpos_authorization_facts`
- partially complete: proposer finalized-height checks now read the Rust FinalChain fact port instead of calling
  `FinalChain::lastBlockNumber`
- remaining: define a `DagProposalFacts` Rust/bridge DTO containing proposal period, period block hash, last finalized period,
  sender vote count, total vote count, VRF key status, and gas-limit facts needed by proposer/verification logic
- completed earlier: load proposal-period and period-block-hash facts from `rustaxa-storage` in `rustaxa-consensus::dag`
- completed: collect DPoS/VRF facts through `BridgeFinalChain` instead of the C++ FinalChain compatibility method
- partially complete for transaction selection: proposer shard filtering moved from the DAG proposer shim into the Rust
  TransactionManager packing session, while C++ still supplies live transaction materialization and EVM gas estimates
- partially complete for DAG block construction: `rustaxa-consensus::dag` owns proposer tip selection, block gas
  accumulation, and the prune/no-prune decision through `dag_proposer_plan_block_construction`; C++ still materializes
  live tip metadata, the legacy VDF payload object, transactions, and the final `DagBlock`
- partially complete for producer VDF: `DagBlockProposer` now calls `prove_legacy_vdf_sortition` directly for the async
  proof and reconstructs the legacy `VdfSortition` payload from Rust output; the pre-proof VRF/difficulty/staleness facts
  also come from existing Rust VDF bridge helpers
- complete for proposer sortition params: `DagBlockProposer` now calls the shim-owned
  `SortitionParamsManager::rustSortitionParamsForRust` accessor and feeds Rust-native sortition runtime params directly
  into the Rust VDF helpers, leaving `getSortitionParams` as public C++ compatibility only
- complete for verifier sortition params: `DagManager::verifyBlock` now feeds the same Rust-native sortition runtime
  params directly into `dag_verify_vdf_sortition_from_block`, so block verification no longer builds the C++
  `SortitionParams` compatibility DTO for Rust VDF checks
- remaining: route `DagBlockProposer` and `DagManager` shim decisions through the DTO while keeping C++ block/transaction
  materialization temporary

Keep temporarily:

- C++ DAG block object construction and network packet materialization
- external EVM gas/state execution
- public debug/query reads of DAG blocks
- live transaction materialization, gas estimation, and tip metadata lookup until their own slices move them

Done when:

- direct C++ FinalChain proposal-fact calls are removed from DAG proposer/verification
- full completion still requires DAG proposer/verification decisions to avoid per-call `DbStorage`/C++ compatibility
  routing for proposal facts by using the planned DTO
- remaining DAG `DbStorage` references are sidecar materialization or query/admin compatibility
- Slice 8 can partially remove direct C++ FinalChain DAG proposal-validation reads but must keep the broader
  DAG proposal DTO/runtime boundary open
- stop condition: do not add a narrower `DagProposalFacts` DTO until the slice also moves one coherent proposal-runtime
  owner such as transaction selection + gas facts, sortition/VDF facts, or DAG block construction facts

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag`
- `cmake --build /build --target dag_test --parallel 12` or the narrow DAG shim target available in `/build`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`

Validation note: the direct DAG authorization/finalized-height sub-slice passes `cargo test --manifest-path
rust/Cargo.toml -p rustaxa-consensus dag`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag`,
`cmake --build /build --target dag_shim_test --parallel 12`, `/build/bin/dag_shim_test`, `cmake --build /build
--target dag_test --parallel 12`, `/build/bin/dag_test`, `cmake --build /build --target rust_storage_tests --parallel
12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`, and
`scripts/rewrite_storage_boundary_guard.sh`.

Validation note: the transaction-packing shard sub-slice adds
`transaction_manager_runtime_pack_begin_sharded`, moves the legacy sender-prefix shard filter from
`DagBlockProposer::getShardedTrxs` into the Rust TransactionManager runtime, and keeps C++ only as the live
transaction/EVM estimator boundary. It passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
transaction_manager_runtime_pack_session_filters_candidate_shards`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-bridge transaction_manager`, `cmake --build /build --target dag_shim_test --parallel 12`,
`/build/bin/dag_shim_test`, `cmake --build /build --target transaction_manager_shim_test --parallel 12`,
`/build/bin/transaction_manager_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the DAG block-construction planner sub-slice moves proposer tip selection from bridge-local Rust into
`rustaxa-consensus::dag`, adds `dag_proposer_plan_block_construction`, and routes
`DagBlockProposer::createDagBlock` through that Rust plan for gas accumulation and tip pruning. It passes `cargo fmt
--manifest-path rust/Cargo.toml --all --check`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag`,
`cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag`, `cmake --build /build --target dag_shim_test
--parallel 12`, `/build/bin/dag_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the producer-VDF sub-slice routes `DagBlockProposer` async proof generation through
`prove_legacy_vdf_sortition`, uses Rust cancellation tokens for the existing cancellation path, and reconstructs the
legacy `VdfSortition` compatibility object from Rust payload bytes. It passes `cmake --build /build --target
dag_shim_test --parallel 12`, `/build/bin/dag_shim_test`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-vdf
sortition`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag_vdf`, `cmake --build /build --target
rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh
--self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the producer-VDF probe cleanup removes the temporary `VdfSortition` difficulty/staleness probe from
`DagBlockProposer`; the proposer now gets normalized vote count, VRF proof/threshold, difficulty, and stale status
through Rust VDF bridge helpers before selecting transactions. It passes `cmake --build /build --target dag_shim_test
--parallel 12`, `/build/bin/dag_shim_test`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-vdf sortition`,
`cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag_vdf`, `cmake --build /build --target
rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh
--self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the proposer-sortition-param cleanup removes the temporary C++ `SortitionParams` materialization from
`DagBlockProposer` by exposing a shim-owned Rust-native sortition params accessor on `SortitionParamsManager`. The DAG
proposer now gets period-specific sortition facts directly from the Rust sortition runtime backed by `rustaxa-storage`
before VDF probe/proof planning. It passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-consensus dag`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-bridge dag_vdf`, `cmake --build /build --target dag_shim_test --parallel 12`, `/build/bin/dag_shim_test`,
`cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the verifier-sortition-param cleanup removes the same temporary C++ `SortitionParams` materialization
from `DagManager::verifyBlock`. DAG block VDF verification now receives `SortitionRuntimeParams` directly from the
Rust-backed sortition manager and leaves `getSortitionParams` as public C++ compatibility only. It passes the focused
validation: `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus dag`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag_vdf`, `cmake --build /build
--target dag_test --parallel 12`, `/build/bin/dag_test`, `cmake --build /build --target rust_storage_tests --parallel
12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the DAG proposer cleanup removes the now-unused shim-owned `selectDagBlockTips` helper because
tip-selection policy is owned by `rustaxa-consensus::dag` through `dag_proposer_plan_block_construction`. The upstream
legacy helper remains only in the original reference implementation. It passes the focused validation listed for this
sub-slice: `rg -n "selectDagBlockTips\\(" libraries/core_libs/consensus/shims/dag_block_proposer_shim
CONSENSUS_STORAGE_MIGRATION_SLICES.md`, `cmake --build /build --target dag_shim_test --parallel 12`,
`/build/bin/dag_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the DAG proposer bridge cleanup removes the now-unused standalone
`dag_proposer_select_tips` CXX bridge API and `DagProposerTipSelection` DTO. Tip-selection policy stays in
`rustaxa-consensus::dag` and is now exposed to C++ only through the fuller `dag_proposer_plan_block_construction`
surface that owns gas accumulation, pruning, selected tips, and missing-tip counting together. It passes `rg -n
"DagProposerTipSelection|dag_proposer_select_tips|plan_dag_proposer_tip_selection" rust/crates/rustaxa-bridge/src
rust/crates/rustaxa-consensus/src libraries/core_libs/consensus/shims/dag_block_proposer_shim
CONSENSUS_STORAGE_MIGRATION_SLICES.md`, `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-bridge dag_proposer`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus dag_proposer`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, `cmake --build /build --target dag_shim_test --parallel 12`,
`/build/bin/dag_shim_test`, and `git diff --check`.

Validation note: the DAG block-construction bridge payload cleanup removes unused `pruned_tips` and
`skipped_missing_tips` fields from the CXX-facing `DagProposerBlockConstructionPlan`. The Rust domain still computes
those facts for tests and internal planner semantics, while the bridge now exposes only the selected tips and block gas
that the C++ compatibility shell consumes. It passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo
test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag_proposer_block_construction`, `cargo test --manifest-path
rust/Cargo.toml -p rustaxa-consensus dag_proposer_block`, `cmake --build /build --target dag_shim_test --parallel 12`,
`/build/bin/dag_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

## Slice 18: Transaction Account And Finalized Fact Port

Goal: complete the TransactionManager account/finalized fact boundary so transaction queue admission, packing,
finalized-status updates, and finalized-account purge run through Rust runtime facts rather than C++ storage/final-chain
lookups.

Status: complete for TransactionManager-owned account/finalized fact routing. DAG-block transaction persistence,
`verifyTransactionsNotFinalized`, live transaction admission, proposal-period transaction filtering, and finalized-status
queue cleanup now source sender account nonce/balance and finalized-transaction location facts through Rust
FinalChain-backed bridge helpers instead of C++ `FinalChain::getAccount` / `transactionLocation` calls. The broader
account snapshot publication/execution mismatch remains under Slice 21, not in TransactionManager-owned storage routing.

Current boundary:

- TransactionManager shim now captures its Rust storage handle during construction; deterministic operation methods use
  that shim-owned handle rather than extracting `db_->rustStorage()` per call.
- DAG-save, verify-not-finalized, and live admission account/finalized facts now flow through `BridgeFinalChain`; public
  transaction admission no longer builds account nonce/balance/finalized-location facts in C++.
- Finalized-account purge facts now flow through the Rust runtime queue-cleanup helper, which collects account nonces
  from `BridgeFinalChain` before mutating the Rust-owned queue.
- The bridge has `*_with_runtime_and_final_chain` routes for account facts, proposal transaction filtering, and queue
  cleanup; remaining work should use those routes or narrower Rust runtime wrappers instead of adding C++ fact builders.

Move:

- completed for DAG-save, verify-not-finalized, and live admission: move account/finalized lookup into existing
  `*_with_runtime_and_final_chain` routes
- completed by Slice 26: remove direct `db_->rustStorage()` calls from TransactionManager operation methods; constructor
  wiring owns the Rust storage handle
- completed: move queue cleanup and finalized-account purge into a TransactionManager runtime method that owns the needed
  Rust storage handle and `BridgeFinalChain` reference
- collapse duplicated C++ command-report glue into typed Rust execution reports
- document any remaining C++ transaction object materialization as sidecar/API compatibility

Keep temporarily:

- C++ transaction object materialization for network/API surfaces
- external EVM gas estimation execution and receipt production
- `StateAPI` / `state_db` execution boundary

Done when:

- completed sub-slice: DAG-save, verify-not-finalized, and live admission no longer call C++ `FinalChain::getAccount` or
  `transactionLocation`
- completed sub-slice: TransactionManager consensus operations no longer extract `DbStorage`/`BridgeStorage` per call
- completed sub-slice: finalized-account queue purge no longer depends on C++ account snapshots
- Slice 8 can remove TransactionManager account/finalized facts from its open read-surface list
- Slice 9 can categorize TransactionManager `DbStorage` ownership as constructor compatibility only

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus transaction`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager`
- `cmake --build /build --target transaction_test --parallel 12` if configured
- focused PBFT/DAG tests that exercise transaction packing without broad FinalChain runtime assumptions
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`

Validation note: the transaction account-fact sub-slice passes `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus transaction_manager`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
transaction_manager`, `cmake --build /build --target transaction_manager_shim_test --parallel 12`,
`/build/bin/transaction_manager_shim_test`, `cmake --build /build --target transaction_test --parallel 12`,
`/build/bin/transaction_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`, and
`scripts/rewrite_storage_boundary_guard.sh`.

Validation note: the live-admission FinalChain fact sub-slice switches TransactionManager admission from a C++-built
account/finalized-location fact DTO to the existing Rust FinalChain-backed runtime route. The C++ shim now passes
`BridgeFinalChain` into Rust before queue mutation, so lookup failures cannot partially admit a transaction and no C++
`FinalChain::getAccount` / `transactionLocation` calls remain in the TransactionManager shim admission path. Validate
with `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge transaction_manager`, `cmake --build /build --target
transaction_manager_shim_test --parallel 12 && /build/bin/transaction_manager_shim_test`, `cmake --build /build
--target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the finalized-account queue cleanup sub-slice switches finalized-status updates to
`update_finalized_transactions_status_command_report_with_runtime_and_final_chain`, so Rust applies finalized status,
collects purge account facts from `BridgeFinalChain`, and mutates the Rust-owned queue before returning logging-only
command buckets to C++. Validate with `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
transaction_manager`, `cmake --build /build --target transaction_manager_shim_test --parallel 12 &&
/build/bin/transaction_manager_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12 &&
/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test &&
scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

## Slice 19: Gas-Pricer Runtime Storage Ownership

Goal: remove the remaining gas-pricer `DbStorage` constructor/init dependency by making the Rust gas-pricer runtime own
or receive native Rust storage at construction time.

Status: complete. The deterministic finalized-history restoration already lived in Rust; this slice removed the
remaining C++ init-time handle plumbing by constructing a storage-owned Rust gas-pricer runtime.

Current boundary:

- `BridgeGasPricer` can now retain a cloned Rust `Storage` handle and restore finalized gas-price history during
  construction.
- `GasPricer` no longer has an async `init(std::shared_ptr<DbStorage>)` path and no longer calls `db->rustStorage()` from
  a separate init method.
- The Rust gas-pricer loads finalized history through `rustaxa-consensus::gas_pricer` over Rust storage.

Move:

- completed: introduced `create_gas_pricer_from_storage`, which restores history and keeps the Rust storage owner alive
- completed: changed the shim constructor to select the storage-owned Rust constructor when block-history pricing is
  enabled and storage is provided
- completed: removed `GasPricer::init(const std::shared_ptr<DbStorage>&)` and the init thread/error replay path
- not needed yet: storage-boundary guard tightening; the remaining `DbStorage` mention is constructor compatibility

Keep temporarily:

- C++ gas-price oracle lock and transaction-pool gas-price calculation
- public C++ constructor shape still accepts `DbStorage`, but only to expose the Rust storage handle at construction

Done when:

- gas-pricer Rust-mode init no longer calls `db->rustStorage()` outside constructor-time bridge creation
- Slice 8 can remove gas-pricer runtime handle initialization from its open read-surface list
- Slice 9 can classify the remaining gas-pricer `DbStorage` mention as constructor compatibility only

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus gas_pricer`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge gas_pricer`
- `cmake --build /build --target gas_pricer_shim_test --parallel 12 && /build/bin/gas_pricer_shim_test`
- `cmake --build /build --target gas_pricer_test --parallel 12 && /build/bin/gas_pricer_test`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`

Validation note: Slice 19 passes `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus gas_pricer`, `cargo
test --manifest-path rust/Cargo.toml -p rustaxa-bridge gas_pricer`, `cmake --build /build --target
gas_pricer_shim_test --parallel 12`, `/build/bin/gas_pricer_shim_test`, `cmake --build /build --target
gas_pricer_test --parallel 12`, `/build/bin/gas_pricer_test`, `cmake --build /build --target rust_storage_tests
--parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`, and
`scripts/rewrite_storage_boundary_guard.sh`.

## Slice 20: PBFT Manager Runtime Storage Handle Consolidation

Goal: stop PBFT manager overlay methods from repeatedly extracting `db_->rustStorage()` and move storage ownership into
the long-lived Rust PBFT manager runtime.

Status: complete for PBFT manager operation-site storage extraction. Next-voted-status writes, startup replay reads, and
finalization storage operations now run through the long-lived PBFT manager runtime's Rust storage handle. The only
remaining `db_->rustStorage()` use in the PBFT manager overlay is runtime construction itself, which is constructor
compatibility rather than an operation-site storage route.

Current boundary:

- PBFT manager startup replay, transition storage, finalization staging, and cert-voted-block reads use long-lived Rust
  runtime calls instead of per-call `db_->rustStorage()` extraction.
- Several storage families already moved to `rustaxa-consensus`, but the shim still orchestrates the handles.

Move:

- extend the PBFT manager Rust runtime to own the storage handle needed by startup replay, status/cursor persistence,
  finalization storage stages, and proposed-block/pillar sidecar reload
- replace per-call `db_->rustStorage()` arguments in PBFT manager overlay methods with runtime methods
- keep C++ calls only where they materialize temporary `PbftBlock`, `PeriodData`, pillar vote, or network sidecar objects
- delete bridge helpers made redundant by runtime-owned storage

Completed sub-slices:

- next-voted-status persistence: `placeStateActionVote()` now calls
  `pbft_manager_runtime_apply_next_voted_status()`, which persists through the runtime-owned Rust storage handle and
  advances the Rust runtime snapshot after the storage write commits. The redundant standalone
  `apply_pbft_manager_next_voted_status(storage, ...)` bridge entry point was removed.
- finalization storage helpers: dynamic-lambda lookup, duplicate-finalization resume inspection, and all staged
  finalized-period storage writes now have PBFT-manager-runtime wrapper entry points. The PBFT manager overlay uses those
  wrappers for the finalization path, so finalization storage operations no longer take `db_->rustStorage()` at each
  operation site.
- startup replay storage: the PBFT manager overlay now creates the long-lived Rust runtime before constructor replay and
  calls `pbft_manager_runtime_load_startup_replay_period()` for both FinalChain catch-up replay and recently-finalized
  transaction restoration. `initialState()` reuses the existing runtime instead of recreating it.

Keep temporarily:

- external EVM execution and FinalChain publication reports
- C++ timers, logging, signing, and network effects
- public storage shim methods used by RPC/query/admin compatibility

Done when:

- PBFT manager overlay has no per-operation `db_->rustStorage()` calls for migrated storage families: done
- remaining `db_` use in PBFT manager is constructor compatibility, snapshot toggling, or explicit sidecar materialization:
  done for `rustStorage()` extraction; broader `DbStorage` materialization remains categorized for Slice 22/23
- Slice 9 can remove PBFT manager runtime storage extraction from its open compatibility-shell list: done

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager pbft_finalize pbft_sync`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager pbft_finalize pbft_sync`
- `cmake --build /build --target pbft_manager_test --parallel 12`
- focused `PbftManagerWithDagCreation.*` only after checking whether the known FinalChain/EVM runtime gap is still active
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`

Validation notes:

- Next-voted-status runtime-storage sub-slice passed `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`,
  `cmake --build /build --target pbft_manager_test --parallel 12`,
  `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
  `scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and
  `git diff --check`.
- `/build/bin/pbft_manager_test --gtest_filter='PbftManagerTest.*'` was attempted as a broad smoke, but the run reached
  the existing FinalChain/EVM runtime gap: balance assertions failed after transactions were not fully executed, vote
  admission reported missing Rust FinalChain DPoS snapshots for later blocks, and the process entered a repeated PBFT sync
  loop at synced period 84 before being terminated. This is the same integration boundary tracked for the later
  FinalChain/EVM slices, not a new next-voted-status storage issue.
- Finalization runtime-storage sub-slice passed `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_finalize`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_finalize`,
  `cmake --build /build --target pbft_manager_test --parallel 12`,
  `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
  `scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and
  `git diff --check`.
- Startup replay runtime-storage sub-slice passed `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus pbft_manager`,
  `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`,
  `cmake --build /build --target pbft_manager_test --parallel 12`,
  `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
  `scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and
  `git diff --check`.

## Slice 21: FinalChain Publication And Status Write Boundary

Goal: move FinalChain-adjacent consensus status writes out of C++ batch orchestration without moving arbitrary EVM
execution or `state_db` ownership.

Status: partial. External-EVM FinalChain publication already commits the main header, receipt, transaction index,
execution-counter, DPoS snapshot, rewards-stat, system-transaction hash, and pending-marker cleanup rows through
`rustaxa-consensus::FinalChain` over `rustaxa-storage`. This sub-slice also moved the proposal-period DAG-level mapping
row from the FinalChain C++ shim's post-publication `DbStorage` batch into the Rust publication plan and batch. The
FinalChain shim also now hydrates its temporary executed DAG/transaction counter sidecars from a typed Rust FinalChain
execution-status query instead of `DbStorage::getStatusField`, and no longer calls the C++ `DbStorage::createSnapshot`
checkpoint hook after a Rust-owned external-EVM publication. Remaining work is still needed for the broader
`getNumTransactionExecuted()`/DPoS/account snapshot mismatch class without violating the external-EVM boundary.

Current boundary:

- FinalChain shim now supplies the optional anchor-derived proposal-period DAG-level fact to the Rust session before
  external state commit; Rust includes that row in the publication plan id, pending-publication marker, publication
  batch, and audit.
- Rust publication owns executed block/transaction counters, system transaction indexes, and final-chain publication
  rows around the C++ `StateAPI` execution adapter.
- FinalChain shim constructor counter sidecars now read the persisted executed block/transaction counters through
  `BridgeFinalChain::get_execution_status`, leaving those atomics as public API/live compatibility mirrors rather than
  storage owners.
- External-EVM publication no longer performs a post-commit `DbStorage::createSnapshot` check from the FinalChain shim;
  snapshot/checkpoint lifecycle remains outside the current Rust storage-ownership slice and must not be used as a
  consensus publication owner.
- PBFT runtime failures currently reproduce execution-count mismatches (`111` vs `1111`) and missing DPoS snapshot
  publication; those are not storage-shim bugs but publication-boundary gaps.

Move:

- completed for proposal-period mapping: define a Rust publication report/plan field that records the anchor-derived
  DAG-level mapping and commits it with final-chain indexes, executed counters, DPoS snapshot sidecars,
  system-transaction hashes, rewards stats, and pending-publication marker cleanup after C++ `StateAPI` reports success
- completed for startup counter hydration: expose a Rust FinalChain execution-status query and use it to hydrate the C++
  compatibility counters without `DbStorage::getStatusField`
- remaining: extend publication ownership for account snapshot sidecars and the DPoS/account facts that still explain
  the broad PBFT runtime mismatches
- keep C++ as the executor that produces receipts/state roots, but make Rust validate and commit the durable publication
  facts in one storage session
- expose restart/resume validation for partial publication windows
- update PBFT manager finalization to consume typed publication status instead of checking scattered C++ counters

Keep temporarily:

- C++ `StateAPI` transaction execution, receipts, contract execution, bridge-contract calls, and `state_db` commit
- public FinalChain API materialization for RPC/GraphQL

Done when:

- the PBFT runtime execution-count mismatch has a Rust publication owner and focused regression coverage
- FinalChain publication does not write consensus/final-chain `DbStorage` rows through C++ batches
- Slice 14 can mark FinalChain status/publication facts complete while leaving arbitrary EVM execution out of scope

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus final_chain`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`
- `cmake --build /build --target final_chain_test --parallel 12 && /build/bin/final_chain_test`
- focused PBFT manager runtime test that previously reproduced the execution-count mismatch
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`

Validation note: the proposal-period publication sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml
-p rustaxa-consensus final_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`,
`cmake --build /build --target final_chain_test --parallel 12`, `cmake --build /build --target rust_storage_tests
--parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`. Broad `/build/bin/final_chain_test` runtime was
attempted and still fails before a clean summary: `FinalChainTest.coin_transfers` reports Rust-mode account balance
mismatches and `FinalChainTest.nonce_test` segfaults. Treat those as the remaining FinalChain/EVM/account execution
boundary tracked by this slice, not as a proposal-period publication storage regression.

Validation note: the execution-status startup sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus final_chain`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge final_chain`, `cmake
--build /build --target final_chain_test --parallel 12`, `cmake --build /build --target rust_storage_tests --parallel
12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the post-publication snapshot-check cleanup removes the no-op Rust-mode `DbStorage::createSnapshot`
call from the FinalChain shim after external-EVM publication. It passes `cmake --build /build --target final_chain_test
--parallel 12`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and
`git diff --check`.

## Slice 22: Network And Query Compatibility Shell Split

Goal: separate network/app/query compatibility reads from consensus runtime storage ownership so Slice 8/9 can close
without waiting for every public API to be rewritten.

Status: complete. Rust-mode PBFT sync egress stays on the typed Rust runtime payload route, while remaining network,
RPC, GraphQL, and debug storage references are classified as legacy-only tarcap compatibility or marked public query
compatibility.

Current boundary:

- tarcap constructors and sync handlers still carry `std::shared_ptr<DbStorage>` for legacy/reference and materialization
  reasons, but the latest PBFT sync handler's Rust-mode egress path uses `PbftManager::getPbftSyncEgressPayload` instead
  of `DbStorage::getPeriodDataRaw`.
- RPC/GraphQL/debug reads are now marked where they remain compatibility reads, but some can move to read-only Rust query
  APIs.
- `get_pbft_sync_packet_handler` still has a legacy-mode `db_->getPeriodDataRaw` branch while Rust mode uses the typed
  PBFT sync egress payload.

Implemented sub-slice:

- guarded tarcap `DbStorage` forward declarations and the latest PBFT sync storage include so Rust-enabled packet handler
  headers/sources no longer expose storage declarations for handlers that do not need them in Rust mode
- removed an unused v4 PBFT sync storage include from the network compatibility path
- added `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY` to the storage-boundary guard and self-tests so future tarcap storage
  additions must be explicitly marked and remain legacy/compatibility-only; unmarked network storage additions now fail
  the guard just like unmarked RPC/GraphQL query reads
- marked the remaining legacy PBFT sync `DbStorage::getPeriodDataRaw` read with
  `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY`; the Rust-mode branch continues to use the typed Rust sync payload
- marked the remaining tarcap `DbStorage` constructor/member surfaces with `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY` and the
  GraphQL HTTP processor storage owner with `RUSTAXA_QUERY_COMPAT_READ`, so Slice 23 can audit network/query debt by
  marker instead of treating it as unclassified consensus storage ownership
- audited existing Rust query helpers for the marked RPC/GraphQL reads. The easy pillar-block-data query already uses
  Rust storage in Rust mode; the remaining reads materialize C++ `PbftBlock`, `DagBlock`, transaction, vote, or public
  JSON objects and stay marked compatibility reads until the API/materialization rewrite lands.

Move:

- split Rust-mode tarcap handler constructors from legacy `DbStorage` constructors where the handler no longer needs DB
  for deterministic decisions
- move easy RPC/GraphQL read-only storage lookups to Rust query APIs when an equivalent Rust storage helper already
  exists
- leave marked `RUSTAXA_QUERY_COMPAT_READ` comments for public API reads that still require C++ materialization
- update the guard allowlist to distinguish network/query compatibility from consensus runtime paths

Keep temporarily:

- public JSON/GraphQL object materialization
- legacy/reference network handler routes
- snapshot toggling and app lifecycle/admin storage ownership

Validation note: the network compatibility guard sub-slice passes `cmake --build /build --target network_test --parallel
12`, `cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`. Running `/build/bin/network_test` still aborts in `NetworkTest.node_pbft_sync` with
`RUST_STORAGE_TX_VERIFY_NOT_FINALIZED_FAILED: TM_FINAL_CHAIN_ACCOUNT_LOOKUP_FAILED`, after earlier network sync cases pass;
that is the existing FinalChain/account lookup boundary and is not counted as a Slice 22 tarcap compatibility regression.

Validation note: the constructor/query classification sub-slice passes `cmake --build /build --target network_test
--parallel 12`, `cmake --build /build --target rpc_plugin --parallel 12`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the Slice 22 closure audit uses `rg` over `libraries/core_libs/network/{rpc,graphql,src/tarcap}` and
`libraries/core_libs/network/include/network/tarcap` to verify remaining `getDB()`, `rustStorage()`, `DbStorage`, and
`db_->` references are marked with either `RUSTAXA_QUERY_COMPAT_READ` or `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY`, then checks
the existing Rust query helpers to confirm no remaining marked read has an equivalent drop-in Rust query API that avoids
C++ object materialization.

Done when:

- Slice 8 can treat network/query reads as explicit compatibility debt, not consensus blockers
- Slice 9 can close with a compatibility-shell contract that excludes consensus runtime storage
- guard failures catch any new unmarked query/network storage use

Validation:

- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`
- `cmake --build /build --target rpc_plugin --parallel 12`
- `cmake --build /build --target rpc_test --parallel 12 && /build/bin/rpc_test`
- focused tarcap/network build targets affected by constructor splits
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`

## Slice 23: Slice 8/9 Closure Gate

Goal: after Slices 16-22 land, make Slice 8 and Slice 9 objectively closable with a guard-backed audit rather than a
manual judgment call.

Status: re-audited / partially unblocked. The original Slice 23 blocker list was reduced by Slices 24-26 and the Slice
18 follow-ups: PBFT manager residual storage routes, rewards-stats column access, operation-level `db_->rustStorage()`
extraction in PBFT chain/VoteManager/TransactionManager, and TransactionManager account/finalized fact routing are
closed. Slice 8 and Slice 9 still stay open because DAG proposal facts and FinalChain/EVM account-publication facts remain
broader runtime boundaries rather than compatibility-shell cleanup.

Move/remove:

- run a code-search audit for `DbStorage`, `db_->`, `rustStorage`, `createWriteBatch`, `commitWriteBatch`, direct
  FinalChain DPoS/account facts, and C++ batch APIs across Rust-mode consensus shims
- update `scripts/rewrite_storage_boundary_guard.sh` allowlists so remaining consensus shim additions fail unless they
  are storage shim internals, tests, or explicitly marked query/admin compatibility
- delete obsolete bridge DTOs/runtime wrappers made unused by Slices 16-22
- update Slice 8 and Slice 9 statuses from replanned/stopped to complete only if the audit proves no consensus runtime
  storage or direct C++ FinalChain fact route remains outside explicitly accepted boundaries

Audit result:

- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh` passes for the
  current diff, but that guard only prevents new additions and does not prove pre-existing routes are gone.
- The network/query audit now passes: remaining `getDB()`, `rustStorage()`, `DbStorage`, and `db_->` references under
  `libraries/core_libs/network/{rpc,graphql,src/tarcap}` and `libraries/core_libs/network/include/network/tarcap` are
  marked with `RUSTAXA_QUERY_COMPAT_READ` or `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY`.
- Direct FinalChain DPoS fact calls under shim-owned consensus code are limited to the FinalChain shim fact provider.
- Storage-shim `DbStorage` and batch APIs remain expected compatibility-shell internals.
- Closed by Slices 24-26:
  - PBFT manager overlay storage routes for DAG-period lookup, PBFT existence checks, Round/Step persistence,
    cert-voted-block persistence, own pillar vote rebroadcast, and snapshot lifecycle classification.
  - Rewards stats reload, clear, cache snapshot, and write apply routes formerly using `DbStorage::Columns` and
    operation-site `db_->rustStorage()`.
  - PBFT chain, VoteManager, and TransactionManager operation-level Rust storage handle extraction.
- Remaining closure blockers:
  - Slice 17: DAG proposer/verification still needs a full proposal-fact DTO/runtime for transaction selection,
    gas-estimation, tip metadata, and DAG block materialization boundaries after direct authorization, finalized-height,
    sortition-param, and standalone tip-selection bridge surfaces were moved or retired.
  - Slice 18 is closed for TransactionManager-owned account/finalized routing; any remaining account snapshot mismatch is
    now tracked under Slice 21's FinalChain publication boundary.
  - Slice 21: FinalChain publication still owns only part of the account/DPoS snapshot and execution-status surface; the
    broad PBFT runtime mismatch class remains there.
  - FinalChain snapshot creation remains C++ app lifecycle/storage-shell compatibility and should stay outside consensus
    closure unless snapshot ownership is explicitly rescoped.

Keep:

- storage shim public C++ compatibility API
- legacy/reference implementation files
- app lifecycle/admin/snapshot/migration/query compatibility routes that are documented outside consensus runtime

Done when:

- `rg` shows no unclassified Rust-mode consensus `DbStorage`/`rustStorage`/direct FinalChain fact route
- storage-boundary guard self-tests cover the final allowlist contract
- Slice 8 and Slice 9 are marked complete with exact residual compatibility categories

Validation:

- `make rewrite-validate-fast`
- `scripts/rewrite_storage_boundary_guard.sh --self-test && scripts/rewrite_storage_boundary_guard.sh`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- targeted C++ builds/tests for any deleted shim/bridge helpers

Validation note: the Slice 23 re-audit after Slices 24-26 ran focused `rg` searches for `DbStorage`, `db_->`, `getDB()`,
`rustStorage()`, `createWriteBatch`, `commitWriteBatch`, `rustBatchId`, `DbStorage::Columns`, direct FinalChain DPoS
facts, and account/finalized fact calls across Rust-mode consensus shims, Rust bridge/runtime crates, and the
network/query compatibility shell. The guard passes, and the stale PBFT manager/rewards/VoteManager/TransactionManager
operation-handle blockers are closed. Slices 8 and 9 remain open only for the remaining Slice 17 and Slice 21 runtime
fact boundaries listed above.

## Slice 24: PBFT Manager Residual Storage Route Closure

Goal: remove the remaining direct `DbStorage` API calls from the PBFT manager overlay or reclassify snapshot toggles as
non-consensus lifecycle compatibility.

Status: complete for Rust-mode consensus storage routes. Round/Step cursor persistence no longer calls `DbStorage::savePbftMgrField` from the PBFT manager
overlay. `rustaxa-consensus::pbft_manager` now owns a cursor-field storage API that only accepts the Round and Step
fields, `rustaxa-bridge` exposes it as a runtime method over the PBFT manager's owned Rust storage handle, and the C++
overlay updates `round_` / `step_` only after the Rust storage write succeeds. Dynamic-lambda field writes are
intentionally rejected by this cursor API because that state remains owned by the finalization/dynamic-lambda storage
paths. DAG block period lookup and PBFT block existence checks now also route through PBFT-manager runtime helpers backed
by `rustaxa-consensus::dag` and `rustaxa-consensus::pbft_chain`, so the overlay no longer calls
`db_->getDagBlockPeriod` or `db_->pbftBlockInDb`. Cert-voted-block persistence now writes through a PBFT-manager runtime
helper backed by `rustaxa-consensus::pbft_manager`; the overlay updates the live `cert_voted_block_for_round_` sidecar
only after the Rust storage write succeeds. Own pillar vote rebroadcast now reads through the PBFT-manager runtime's
Rust storage handle and materializes a temporary C++ `PillarVote` only for the existing network gossip boundary.
The remaining `db_` references are classified: runtime construction extracts the shim-owned Rust storage handle,
proposed-block restore is legacy-only in the `RUSTAXA_ENABLE_PROPOSED_BLOCKS=0` branch, and snapshot enable/disable is
documented app/storage-shell lifecycle compatibility.

Move/remove:

- completed: Round/Step persistence routes through the PBFT manager runtime's Rust storage handle instead of direct
  `db_->savePbftMgrField`
- completed: DAG block period lookup routes through the PBFT manager runtime's Rust storage handle instead of direct
  `db_->getDagBlockPeriod`
- completed: PBFT block existence checks route through the PBFT manager runtime's Rust storage handle instead of direct
  `db_->pbftBlockInDb`
- completed: cert-voted-block persistence routes through the PBFT manager runtime's Rust storage handle instead of
  direct `db_->saveCertVotedBlockInRound`
- completed: own pillar vote rebroadcast reads through the PBFT manager runtime's Rust storage handle instead of direct
  `db_->getOwnPillarBlockVote`
- completed: keep the non-Rust proposed-block startup restore branch as legacy-only while Rust mode uses
  `proposed_blocks_.restoreFromStorage()`
- completed: keep snapshot enable/disable as app lifecycle compatibility with an explicit marker and exclude it from
  consensus storage closure

Residual PBFT manager overlay `db_` categories after Slice 24:

- runtime construction extracts `db_->rustStorage()` as the Rust storage-handle owner
- non-Rust proposed-block restoration still calls `db_->getProposedPbftBlocks` in the `RUSTAXA_ENABLE_PROPOSED_BLOCKS`
  fallback branch and is marked `RUSTAXA_PBFT_LEGACY_ONLY`
- snapshot enable/disable still calls `db_->enableSnapshots` / `db_->disableSnapshots` and is marked
  `RUSTAXA_PBFT_LIFECYCLE_COMPAT`

Done when:

- `pbft_manager_overlay.cpp` has no direct `db_->` consensus storage reads/writes
- any remaining `db_` member use is constructor/runtime-handle ownership or documented lifecycle compatibility
- PBFT manager Rust/bridge tests and focused C++ PBFT manager builds cover the moved routes

Validation note: the Round/Step cursor-field sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`,
`cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-consensus pbft_manager`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`,
`cmake --build /build --target pbft_manager_test --parallel 12`, `cmake --build /build --target rust_storage_tests
--parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the DAG period / PBFT existence sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml
-p rustaxa-consensus dag_block_period_storage_lookup`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
pbft_manager`, `cmake --build /build --target pbft_manager_test --parallel 12`, `cmake --build /build --target
rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh
--self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the cert-voted-block persistence sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml
-p rustaxa-consensus pbft_manager`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge pbft_manager`,
`cmake --build /build --target pbft_manager_test --parallel 12`, `cmake --build /build --target rust_storage_tests
--parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the own pillar vote rebroadcast sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml
-p rustaxa-bridge pbft_manager`, `cmake --build /build --target pbft_manager_test --parallel 12`, `cmake --build
/build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the Slice 24 closure classification sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cmake --build /build --target
pbft_manager_test --parallel 12`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

## Slice 25: Rewards Stats Storage Runtime Closure

Goal: make rewards-stats reload, clear, and write ownership fully Rust-runtime/storage-owned instead of using C++
`DbStorage` column APIs.

Status: complete. `rustaxa-consensus::rewards_stats` now owns startup reload, restart-boundary clearing, runtime cache
snapshots, committed storage clears, and storage write apply over direct `rustaxa-storage` access. `rustaxa-bridge` wraps
the rewards-stat state together with the shared Rust storage handle and exposes runtime-owned snapshot/apply/clear
methods. The C++ rewards stats shim no longer calls `DbStorage::Columns`, `deleteColumnData`, `getBlocksRewardsStats`,
or operation-site `db_->rustStorage()`; its remaining `db_` use is constructor-time Rust storage handle ownership.

Move/remove:

- move `block_rewards_stats` reload and frequency-boundary clear into `rustaxa-consensus::rewards_stats` over
  `rustaxa-storage`
- replace `db_->deleteColumnData(DbStorage::Columns::block_rewards_stats)` and `db_->getBlocksRewardsStats` in the shim
  with Rust runtime calls
- remove operation-site `db_->rustStorage()` extraction from rewards stats once the runtime owns its storage handle

Implementation notes:

- `rewards_stats_runtime_from_storage` clears stale `block_rewards_stats` rows through `rustaxa-storage` when startup is
  already at a distribution boundary, then returns an empty runtime cache.
- `RewardsStatsRuntime::cached_stats_rlp` exposes an ordered compatibility snapshot so the shim can rebuild its temporary
  `blocks_stats_` sidecar without C++ storage reads.
- `BridgeRewardsStatsRuntime` owns the shared `Arc<Storage>` and applies cache writes / boundary clears through runtime
  methods, leaving the free bridge apply helper as compatibility/test scaffolding only.

Done when:

- `rewards_stats_shim.cpp` has no direct `DbStorage::Columns` or `db_->` storage calls
- rewards-stats Rust/bridge tests and focused C++ rewards stats builds cover restart/reload, clear, and write behavior

Validation note: Slice 25 passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo check
--manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus
rewards_stats`, `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge rewards_stats`, `cmake --build /build
--target rewards_stats_test --parallel 12`, `/build/bin/rewards_stats_test`, `cmake --build /build --target
rust_consensus_tests --parallel 12`, `/build/bin/rust_consensus_tests --gtest_filter=RustRewardsStatsBridgeTest.*`,
`cmake --build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

## Slice 26: Operation-Level Rust Storage Handle Extraction Cleanup

Goal: collapse remaining operation-level `db_->rustStorage()` extraction in consensus shims into runtime construction or
shim-owned Rust handles so C++ no longer passes storage handles through deterministic operation methods.

Status: complete. `BridgePbftChain` can now be constructed with an owned Rust storage handle, and PBFT block
existence/load operations are runtime methods over that handle. The PBFT chain shim no longer passes
`db_->rustStorage()` from `findPbftBlockInChain` or `getPbftBlockInChain`; its remaining storage handle extraction is
constructor-time runtime ownership. VoteManager captures a Rust storage handle during construction and uses it for
vote-progress persistence, own-vote save/clear, and finalization reward-vote reset instead of extracting
`db_->rustStorage()` inside operation methods. TransactionManager also captures a Rust storage handle during
construction and uses that handle for DAG transaction persistence, bounded transaction views, finalized filtering,
not-finalized verification, non-finalized recovery, and finalized-status persistence.

Move/remove:

- completed: TransactionManager operation methods use a constructor-owned Rust storage handle instead of operation-site
  `db_->rustStorage()`
- completed: VoteManager vote progress, own-vote cleanup, and finalization reward-vote reset use a
  constructor-owned Rust storage handle instead of operation-site `db_->rustStorage()`
- completed: PBFT chain block existence and PBFT block materialization now use runtime-owned storage
  methods instead of operation-site `db_->rustStorage()`
- completed: PBFT chain, gas-pricer, sortition, proposed-block, pillar-chain, DAG, and FinalChain shims were audited for
  storage handle ownership versus operation-site extraction

Done when:

- non-storage consensus shims no longer call `db_->rustStorage()` from operation methods
- Slice 23 can rerun the closure audit and either mark Slices 8/9 complete or report only explicitly documented
  app/query/lifecycle compatibility

Validation note: the PBFT chain operation-handle sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cargo test --manifest-path
rust/Cargo.toml -p rustaxa-bridge pbft_chain`, `cmake --build /build --target rust_consensus_tests --parallel 12`,
`/build/bin/rust_consensus_tests --gtest_filter=RustPbftChainTest.*`, `cmake --build /build --target
rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh
--self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

Validation note: the VoteManager operation-handle sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml --all
--check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cmake --build /build --target vote_test
--parallel 12`, `/build/bin/vote_test`, `cmake --build /build --target pbft_manager_test --parallel 12`, `cmake
--build /build --target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Validation note: the TransactionManager operation-handle sub-slice passes `cargo fmt --manifest-path rust/Cargo.toml
--all --check`, `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`, `cmake --build /build --target
transaction_manager_shim_test --parallel 12`, `/build/bin/transaction_manager_shim_test`, `cmake --build /build
--target rust_storage_tests --parallel 12`, `/build/bin/rust_storage_tests`,
`scripts/rewrite_storage_boundary_guard.sh --self-test`, `scripts/rewrite_storage_boundary_guard.sh`, and `git diff
--check`.

Residual categories after Slice 26:

- storage-shim `DbStorage::rustStorage()` accessors remain the compatibility shell internals that own the shared
  `BridgeStorage`.
- PBFT manager, PBFT chain, DAG manager, gas pricer, sortition, proposed blocks, pillar chain, rewards stats, FinalChain,
  VoteManager, and TransactionManager only extract Rust storage handles during constructor/runtime setup or documented
  startup restore; deterministic operation methods use runtime-owned or shim-owned Rust storage handles.

## Slice 27: Slashing Submitter FinalChain Account Facts

Goal: stop the slashing manager shim from collecting submitter nonce/balance facts through the C++ `FinalChain`
compatibility API when the Rust FinalChain runtime already exposes the account lookup.

Status: complete. `SlashingManager` now reads each configured wallet account through
`FinalChain::rustFinalChainForRust().get_account(...)` and maps the returned Rust FinalChain account fact into the
existing slashing planner DTO. Missing accounts still produce zero nonce and zero balance, preserving the old
`ZeroAccount` fallback semantics without routing through `FinalChain::getAccount`.

Move/remove:

- completed: slashing submitter account fact collection no longer calls `FinalChain::getAccount`
- completed: shim documentation now classifies submitter nonce/balance sourcing as Rust FinalChain runtime-owned

Keep temporarily:

- gas-price lookup
- slashing transaction construction/signing
- transaction-pool insertion
- external EVM/state execution of the submitted slashing transaction

Done when:

- `slashing_manager_shim.cpp` has no `FinalChain::getAccount` compatibility call for submitter facts
- the focused slashing shim build and Rust slashing planner tests pass
- storage-boundary guard and `rust_storage_tests` stay green

Validation note: Slice 27 passes `cargo fmt --manifest-path rust/Cargo.toml --all --check`, `cargo test
--manifest-path rust/Cargo.toml -p rustaxa-consensus slashing`, `cargo test --manifest-path rust/Cargo.toml -p
rustaxa-bridge slashing`, `cmake --build /build --target slashing_manager_shim_test --parallel 12`,
`/build/bin/slashing_manager_shim_test`, `cmake --build /build --target rust_storage_tests --parallel 12`,
`/build/bin/rust_storage_tests`, `scripts/rewrite_storage_boundary_guard.sh --self-test`,
`scripts/rewrite_storage_boundary_guard.sh`, and `git diff --check`.

## Stop Conditions

Stop and re-plan before continuing a slice if:

- The work requires changing network/tarcap transport, packet wrapping, gossip fanout, EVM execution, receipt execution,
  or contract execution ownership.
- The implementation would silently forward production Rust-mode storage behavior to legacy C++.
- A slice needs broad original upstream C++ edits instead of shim-owned overlay changes.
- The change would weaken or retarget tests to make Rust mode pass.
- Validation exposes a new non-storage PBFT runtime gap, such as the current second-finish primary-intent failure.
