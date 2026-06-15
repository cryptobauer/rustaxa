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
are tracked below as Slices 10-14 so implementation can continue without broadening Slice 8 into unrelated subsystem
ownership changes.

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

Status: planned.

Current boundary:

- Latest PBFT sync egress already calls `pbft_mgr_->getPbftSyncPeriodDataRaw` in Rust mode, but network handlers still
  carry `DbStorage` and legacy branches call `getPeriodDataRaw`.
- DAG sync and status handlers still receive `DbStorage` because network packet handlers predate the Rust storage
  boundary.
- Proposed-block bundle egress still enumerates temporary C++ sidecars.

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

- Rust-mode network sync handlers do not use `DbStorage` for deterministic consensus storage reads.
- C++ network code receives typed sync payload/effect DTOs and only performs transport work.
- Any remaining `DbStorage` member in network handlers is legacy/query compatibility or removed from Rust-mode builds.

Validation:

- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus dag pbft_manager`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge dag pbft_manager`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- focused DAG/PBFT sync C++ targets
- storage-boundary guard self-test/current-diff guard

## Slice 13: Query Compatibility Read Split

Goal: separate RPC/GraphQL/debug compatibility reads from consensus storage ownership and prevent unmarked query debt
from looking like unfinished consensus migration.

Status: planned.

Current boundary:

- RPC and Debug `getDB()` reads are marked with `RUSTAXA_QUERY_COMPAT_READ`.
- GraphQL query code still has `DbStorage` constructor/state and calls `getPbftBlock`, `getDagBlocksAtLevel`, and
  `getFinalizedDagBlockByPeriod` without compatibility markers.
- These reads serve API responses and should not block consensus storage ownership, but they should be visible debt.

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

- Every Rust-mode RPC/GraphQL/debug `DbStorage` read is either replaced by a read-only Rust query API or marked as
  `RUSTAXA_QUERY_COMPAT_READ`.
- The storage-boundary guard rejects new unmarked GraphQL storage reads.
- Query compatibility debt is not counted as consensus storage migration work.

Validation:

- `scripts/rewrite_storage_boundary_guard.sh --self-test`
- `scripts/rewrite_storage_boundary_guard.sh`
- `git diff --check`
- `cmake --build /build --target rpc_plugin --parallel 12`
- `cmake --build /build --target rpc_test --parallel 12 && /build/bin/rpc_test`
- GraphQL build/test target if configured

## Slice 14: FinalChain/EVM Fact Port For Consensus

Goal: make FinalChain/EVM-derived facts consumed by consensus explicit Rust ports so PBFT, DAG, votes, pillar, rewards,
and transaction cleanup no longer call C++ FinalChain or `DbStorage` for deterministic consensus facts.

Status: planned and intentionally larger than a storage cleanup slice.

Current boundary:

- External EVM execution, state commits, receipts, bridge root/epoch reads, DPoS vote counts, account snapshots, and
  system transaction construction remain outside the consensus storage migration.
- This is the accepted boundary that explains current PBFT manager runtime failures such as external-EVM state gaps,
  DPoS snapshot gaps, and transaction execution-count mismatches.
- Finalized-account transaction queue purge was explicitly deferred from Slice 4 until FinalChain account snapshots move
  to Rust-accessible facts.

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

## Slice 15: Compatibility Shell Audit And Guard Hardening

Goal: after Slices 10-14 land, make the remaining `DbStorage` surface intentionally compatibility-only and harden guards
so regressions are caught before review.

Status: planned final cleanup.

Move/remove:

- remove obsolete `DbStorage` members from Rust-mode consensus/network shim classes
- delete bridge DTOs and helper functions made unused by the new runtimes
- shrink allowlists in `scripts/rewrite_storage_boundary_guard.sh`
- add checks for known shim paths once their storage route has moved
- update `PLAN.md` and this file with the final compatibility-shell contract

Keep:

- storage shim internals that implement the public C++ compatibility API
- legacy/reference implementation files and pure-C++ validation routes
- app startup, migration/admin, backup/pruning, and read-only query compatibility until those areas receive their own
  rewrite tracks

Done when:

- A code search shows no Rust-mode consensus production route using `DbStorage` as a storage API.
- Remaining `DbStorage` usage is categorized as storage shim, legacy/reference, app lifecycle/admin, tests, or marked
  query compatibility.
- New consensus/storage regressions fail fast in `make rewrite-validate-fast`.

Validation:

- `make rewrite-validate-fast`
- `scripts/rewrite_storage_boundary_guard.sh --self-test`
- `scripts/rewrite_storage_boundary_guard.sh`
- `cmake --build /build --target rust_storage_tests --parallel 12 && /build/bin/rust_storage_tests`
- targeted C++ shim builds/tests for every touched module

## Stop Conditions

Stop and re-plan before continuing a slice if:

- The work requires changing network/tarcap transport, packet wrapping, gossip fanout, EVM execution, receipt execution,
  or contract execution ownership.
- The implementation would silently forward production Rust-mode storage behavior to legacy C++.
- A slice needs broad original upstream C++ edits instead of shim-owned overlay changes.
- The change would weaken or retarget tests to make Rust mode pass.
- Validation exposes a new non-storage PBFT runtime gap, such as the current second-finish primary-intent failure.
