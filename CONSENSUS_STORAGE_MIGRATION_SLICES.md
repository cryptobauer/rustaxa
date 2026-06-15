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

Status: in progress. Gas-pricer finalized-history restoration now performs its FinalChain `LAST_NUMBER` and period-data
walk inside `rustaxa-consensus::gas_pricer` over native `rustaxa-storage`; the bridge adapter only passes the shared
storage handle and oracle lock. This removes bridge-local raw storage reads and period-data gas-price decoding from the
deterministic gas-pricer initialization path. The storage-boundary guard now also rejects new C++ `getDB()` additions by
default; RPC/GraphQL compatibility reads must carry an inline `RUSTAXA_QUERY_COMPAT_READ` marker so query debt is visible
instead of silently expanding.
Existing RPC/Debug query reads now carry that marker; the scan currently finds no GraphQL `getDB()`/`rustStorage()` query
reads to annotate.
PBFT manager startup replay now loads finalized period data, closest dynamic-lambda facts, and finalized DAG hash order
through a `rustaxa-consensus::pbft_manager` storage helper over native `rustaxa-storage`; the bridge only adapts DTOs,
and the C++ shim only materializes temporary `PeriodData` objects for the existing live replay calls.
PBFT finalization dynamic-lambda planning now also loads the prior saved period lambda through
`rustaxa-consensus::pbft_finalize` over native `rustaxa-storage` instead of asking the PBFT manager shim to call
`DbStorage::getPeriodLambda`.

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

Status: in progress. The public `DbStorage::rustBatchId` shim method has been removed now that PBFT finalization,
VoteManager, sortition, pillar, DAG/proposed-block, transaction, and PBFT manager transition production writes no longer
route through bridge-owned batch ids. The storage shim still owns an internal Rust batch map for temporary
legacy-compatible `insert/remove/commitWriteBatch` behavior, but consensus callers no longer have a public API for
extracting a Rust batch id.
PBFT finalization bridge-owned batch appender scaffolding has also been deleted from `rustaxa-bridge`; bridge tests now
exercise the production `apply_pbft_finalization_storage_writes` API, which creates and commits the Rust-owned batch in
`rustaxa-consensus`.

Move/remove:

- completed: stale public `rustBatchId` production escape hatch
- completed: obsolete PBFT finalization bridge storage appender APIs
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

## Stop Conditions

Stop and re-plan before continuing a slice if:

- The work requires changing network/tarcap transport, packet wrapping, gossip fanout, EVM execution, receipt execution,
  or contract execution ownership.
- The implementation would silently forward production Rust-mode storage behavior to legacy C++.
- A slice needs broad original upstream C++ edits instead of shim-owned overlay changes.
- The change would weaken or retarget tests to make Rust mode pass.
- Validation exposes a new non-storage PBFT runtime gap, such as the current second-finish primary-intent failure.
