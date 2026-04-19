# DbStorage Public API Rewrite Tracker

Source of truth: `libraries/core_libs/storage/include/storage/storage.hpp`

Goal: keep the `DbStorage` public interface stable while replacing all internal `db_` access with the Rust storage module behind small C++ shims.

Current shim placement:
- Rust-mode shim implementations now live in `libraries/core_libs/storage_shim/src/storage_shim.cpp`.
- Legacy `libraries/core_libs/storage/src/storage.cpp` no longer contains Rust `#ifdef` branches for shimmed APIs.
- Shimmed APIs execute Rust logic directly (no fallback calls into `DbStorageOld` inside shim methods).

Current migration direction:
- First focus on read paths.
- After the read surface is stable, move write APIs and batch handling.
- Keep RocksDB administration and file-management helpers out of the first wave unless they block functional parity.
- Snapshotting and storage migrations stay in C++ and are generally out of Rust rewrite scope.
- `light_plugin` storage maintenance/pruning flows stay in C++ and are out of Rust rewrite scope.

## Rewrite Scope Boundary

Out of scope for the Rust storage rewrite (unless explicitly re-scoped in a future batch):
- storage migration flows (`libraries/core_libs/storage/src/migration/*`)
- light-node / `plugin/light` maintenance and pruning flows
- snapshot lifecycle and recovery APIs (`createSnapshot`, `loadSnapshots`, `recoverToPeriod`, `deleteSnapshot`,
  `enableSnapshots`, `disableSnapshots`)

These paths stay C++-owned for now and are not rewrite blockers.

## Legend

- `[x]` Rust-backed shim exists in the Rust storage shim layer (`storage_shim.cpp`/`storage_shim.hpp`)
- `[ ]` Public API still reads/writes via `db_`, `lookup`, `exist`, iterators, or `Batch`
- `[~]` Public helper or cached accessor; no dedicated Rust FFI entry point is required if underlying primitive reads are already shimmed
- `[u]` Public API with no external callers found in the workspace; defer for now
- `[!]` Infrastructure/admin API; not a first-wave repository shim and likely needs separate design work

## External Usage Audit

I checked each public method against workspace call sites outside `storage.hpp`, `storage.cpp`, and `storage_shim.cpp`.

Audit rule used:
- search for member/static call patterns `->method(`, `.method(`, and `DbStorage::method(` across the workspace
- exclude the class declaration and implementation files
- count tests and tooling as external consumers

This is good enough for migration planning, but it will not catch exotic cases such as macro-generated calls or pointer-to-member usage.

## Public but Currently Unused Outside DbStorage

These methods are public today but I found no external call sites in the workspace. They should be ignored for now and only revisited if a later rewrite step needs them.

### Path and Admin Helpers

- `[u] path()`
- `[u] dbStoragePath()`
- `[u] rebuildColumns(const rocksdb::Options& options)`
- `[u] deleteSnapshot(PbftPeriod period)`
- `[u] recoverToPeriod(PbftPeriod period)`
- `[u] loadSnapshots()`
- `[u] replaceColumn(const Column& to_be_replaced_col, std::unique_ptr<rocksdb::ColumnFamilyHandle>&& replacing_col)`
- `[u] copyColumn(rocksdb::ColumnFamilyHandle* orig_column, const std::string& new_col_name, bool move_data = false)`
- `[u] removeTempFiles() const`
- `[u] removeFilesWithPattern(const std::string& directory, const std::regex& pattern) const`
- `[u] deleteTmpDirectories(const std::string& path) const`

### Deferred Read/Write APIs

- `[u] removeDagBlockBatch(Batch& write_batch, blk_hash_t const& hash)`
- `[u] transactionsInDb(std::vector<trx_hash_t> const& trx_hashes)`
- `[u] getTransactionReceipt(EthBlockNumber blk_n, uint64_t position) const`
- `[u] getPeriodSystemTransactions(PbftPeriod period) const`
- `[u] getLastPbftBlockHashAndFinalizedDagBlockByPeriod(PbftPeriod period)`

## Current Shim Coverage

These public read methods already branch to `rust_storage_` today:

- `[x] getDagBlock(blk_hash_t const& hash)`
- `[x] dagBlockInDb(blk_hash_t const& hash)`
- `[x] getBlocksByLevel(level_t level)`
- `[x] getLastBlocksLevel() const`
- `[x] getDagBlocksAtLevel(level_t level, int number_of_levels)`
- `[x] getNonfinalizedDagBlocks()`
- `[x] getDagBlockPeriod(blk_hash_t const& hash)`
- `[x] getProposalPeriodForDagLevel(uint64_t level)`
- `[x] getPeriodDataRaw(PbftPeriod period) const`
- `[x] getPeriodFromPbftHash(taraxa::blk_hash_t const& pbft_block_hash)`
- `[x] pbftBlockInDb(blk_hash_t const& hash)`
- `[x] getBlockReceipts(PbftPeriod period) const`
- `[x] getGenesisHash()`
- `[x] getLastSortitionParams(size_t count)`
- `[x] getParamsChangeForPeriod(PbftPeriod period)`
- `[x] getStatusField(StatusDbField const& field)`
- `[x] getPeriodLambda(PbftPeriod period, bool find_closest)`
- `[x] getRoundsCountDynamicLambda()`
- `[x] getBlocksRewardsStats() const`
- `[x] getPbftMgrField(PbftMgrField field)`
- `[x] getPbftMgrStatus(PbftMgrStatus field)`
- `[x] getCertVotedBlockInRound() const`
- `[x] getProposedPbftBlocks()`
- `[x] getPbftHead(blk_hash_t const& hash)`
- `[x] getOwnVerifiedVotes()`
- `[x] getAllTwoTPlusOneVotes()`
- `[x] getRewardVotes()`
- `[x] getPillarBlock(PbftPeriod period) const`
- `[x] getLatestPillarBlock() const`
- `[x] getOwnPillarBlockVote() const`
- `[x] getCurrentPillarBlockData() const`
- `[x] transactionInDb(trx_hash_t const& hash)`
- `[x] transactionFinalized(trx_hash_t const& hash)`
- `[x] transactionsFinalized(std::vector<trx_hash_t> const& trx_hashes)`
- `[x] getTransactionLocation(trx_hash_t const& hash) const`
- `[x] getTransaction(trx_hash_t const& hash) const`
- `[x] getTransaction(PbftPeriod period, uint32_t position) const`
- `[x] getTransactionCount(PbftPeriod period) const`
- `[x] getSystemTransaction(const trx_hash_t& hash) const`
- `[x] getAllNonfinalizedTransactions()`
- `[x] getAllTransactionPeriod()`
- `[x] getPeriodSystemTransactionsHashes(PbftPeriod period) const`

Current bridge coverage now includes the DAG read slice, proposal-period lookup, period-data primitives (`period_data` and `pbft_block_period`), finalized-chain receipts by period (`final_chain_receipt_by_period`), metadata/config reads (`genesis`, `status`, `sortition_params_change`, `period_lambda`, `rounds_count_dynamic_lambda`, `block_rewards_stats`), PBFT hash presence checks, PBFT manager/vote reads (`pbft_mgr_round_step`, `pbft_mgr_status`, `cert_voted_block_in_round`, `proposed_pbft_blocks`, `pbft_head`, `latest_round_own_votes`, `latest_round_two_t_plus_one_votes`, `extra_reward_votes`), pillar reads (`pillar_block`, `current_pillar_block_own_vote`, `current_pillar_block_data`), and a broader transaction read slice over `transactions`, `trx_period`, `system_transaction`, `period_data`, and `period_system_transactions`.

Final-chain internal read coverage is also routed through Rust in shim mode via additive `DbStorage::lookup*` interception for:
- `final_chain_meta`
- `final_chain_blk_by_number`
- `final_chain_blk_hash_by_number`
- `final_chain_blk_number_by_hash`
- `final_chain_log_blooms_index`
- `final_chain_receipt_by_trx_hash`

This keeps existing `FinalChain` and `GasPricer` call sites unchanged while replacing direct RocksDB reads under the shim boundary.

## Latest Gap Audit (2026-04-19)

I re-ran a workspace scan of:
- all `DbStorage::lookup` / `lookup_int` call sites, and
- all public `DbStorage` APIs that are still inherited from `DbStorageOld` (not redeclared in `storage_shim.hpp`).

### 1. Unshimmed `lookup*` usage still present

- `lookup_int<bool>(..., Columns::migrations)` in `libraries/core_libs/storage/include/storage/migration/migration_base.hpp`
  - This column is not intercepted in the shim lookup dispatcher.
  - Since shim lookup now throws for unsupported columns, this path is currently a migration-scope gap.

### 2. Public APIs still not shim-covered but externally used

These APIs are inherited from `DbStorageOld` (not Rust shim overrides) and have external callers:

- `DeleteRange(...)` and `CompactRange(...)`
  - used by `libraries/plugin/light/src/light.cpp`
- `createSnapshot(...)`
  - used by `libraries/core_libs/consensus/src/final_chain/final_chain.cpp`
- `deleteColumnData(...)`
  - used by `libraries/core_libs/consensus/src/rewards/rewards_stats.cpp`
  - used by `libraries/core_libs/storage/src/migration/block_stats.cpp`
- `disableSnapshots()` / `enableSnapshots()`
  - used by `libraries/core_libs/network/src/tarcap/packets_handlers/interface/sync_packet_handler.cpp`
- `getColumnIterator(...)`
  - used by `libraries/core_libs/storage/src/migration/{block_stats,transaction_receipts_by_period}.cpp`
  - used by `libraries/plugin/light/src/light.cpp`
- `getMajorVersion() const`
  - used by `libraries/core_libs/storage/src/migration/migration_manager.cpp`
  - note: cached/helper-style accessor, not a direct DB read path
- `transactionsFromPeriodDataRlp(...)`
  - used by `libraries/core_libs/storage/src/migration/transaction_receipts_by_period.cpp`

All of the above align with existing scope notes: migration/admin/snapshot and `plugin/light` maintenance paths are intentionally non-blocking for first-wave Rust shims.

### 3. Public APIs still not shim-covered and with no external callers found

No external call sites were found for:
- `getPeriodSystemTransactions(PbftPeriod)`
- `getTransactionReceipt(EthBlockNumber, uint64_t)`
- `removeDagBlockBatch(Batch&, blk_hash_t const&)`
- `transactionsInDb(std::vector<trx_hash_t> const&)`
- `rebuildColumns(...)`
- `deleteSnapshot(...)`
- `recoverToPeriod(...)`
- `loadSnapshots()`
- `replaceColumn(...)`
- `removeTempFiles()`
- `removeFilesWithPattern(...)`
- `deleteTmpDirectories(...)`

## Suggested Storage Buckets

| Bucket | Main column families | Notes |
| --- | --- | --- |
| Genesis and chain metadata | `genesis`, `status`, `migrations` | Small surface, useful for bootstrap and counters |
| Period and finalized chain data | `period_data`, `pbft_block_period`, `final_chain_receipt_by_period` | High leverage bucket because many helper reads compose on top of `period_data` |
| DAG state | `dag_blocks`, `dag_blocks_level`, `dag_block_period`, `proposal_period_levels_map` | First batch already started here |
| Transaction state | `transactions`, `trx_period`, `system_transaction`, `period_system_transactions` | Natural second read bucket after `period_data` |
| PBFT manager and votes | `pbft_mgr_round_step`, `pbft_mgr_status`, `cert_voted_block_in_round`, `proposed_pbft_blocks`, `pbft_head`, `latest_round_own_votes`, `latest_round_two_t_plus_one_votes`, `extra_reward_votes` | Mostly key-value or small iteration APIs |
| Pillar chain | `pillar_block`, `current_pillar_block_data`, `current_pillar_block_own_vote` | Mostly isolated from the DAG path |
| Sortition and dynamic config | `sortition_params_change`, `period_lambda`, `rounds_count_dynamic_lambda`, `block_rewards_stats` | Iterator-heavy reads |

## Read-Phase Tracker

### 1. DAG Read APIs

- `[x] getDagBlock(blk_hash_t const& hash)`
- `[x] dagBlockInDb(blk_hash_t const& hash)`
- `[x] getBlocksByLevel(level_t level)`
- `[x] getLastBlocksLevel() const`
- `[x] getDagBlocksAtLevel(level_t level, int number_of_levels)`
- `[x] getNonfinalizedDagBlocks()`
- `[x] getDagBlockPeriod(blk_hash_t const& hash)`
- `[x] getProposalPeriodForDagLevel(uint64_t level)`
- `[~] getFinalizedDagBlockHashesByPeriod(PbftPeriod period)`
  Note: pure composition over `getPeriodDataRaw`; no separate FFI needed once finalized period-data reads are shimmed.
- `[~] getFinalizedDagBlockByPeriod(PbftPeriod period)`
- `[u] getLastPbftBlockHashAndFinalizedDagBlockByPeriod(PbftPeriod period)`

### 2. Period Data and Finalized Chain Read APIs

- `[x] getPeriodDataRaw(PbftPeriod period) const`
- `[~] getPeriodData(PbftPeriod period) const`
- `[~] getPbftBlock(PbftPeriod period) const`
- `[~] getPeriodCertVotes(PbftPeriod period) const`
- `[~] getPeriodBlockHash(PbftPeriod period) const`
- `[~] getPeriodTransactions(PbftPeriod period) const`
- `[~] getPeriodPillarVotes(PbftPeriod period) const`
- `[~] transactionsFromPeriodDataRlp(PbftPeriod period, const dev::RLP& period_data_rlp) const`
  Note: decode helper only. This should stay in C++ unless the decode layer also moves.
- `[x] getPeriodFromPbftHash(taraxa::blk_hash_t const& pbft_block_hash)`
- `[~] getPbftBlock(blk_hash_t const& hash)`
  Note: composes over `getPeriodFromPbftHash` and `getPbftBlock(period)`.
- `[x] pbftBlockInDb(blk_hash_t const& hash)`
  Note: now bridged through Rust (`PbftRepository`) and backed by `pbft_block_period` existence checks.
- `[x] getBlockReceipts(PbftPeriod period) const`
  Note: now bridged through Rust (`PeriodRepository::block_receipt`) and backed by `final_chain_receipt_by_period`.
- `[u] getTransactionReceipt(EthBlockNumber blk_n, uint64_t position) const`

### 3. Transaction Read APIs

- `[x] getTransaction(trx_hash_t const& hash) const`
- `[x] getTransaction(PbftPeriod period, uint32_t position) const`
- `[x] getAllNonfinalizedTransactions()`
- `[x] transactionInDb(trx_hash_t const& hash)`
- `[x] transactionFinalized(trx_hash_t const& hash)`
- `[u] transactionsInDb(std::vector<trx_hash_t> const& trx_hashes)`
- `[x] transactionsFinalized(std::vector<trx_hash_t> const& trx_hashes)`
- `[x] getTransactionLocation(trx_hash_t const& hash) const`
- `[x] getAllTransactionPeriod()`
- `[x] getTransactionCount(PbftPeriod period) const`
- `[~] getFinalizedTransactions(std::vector<trx_hash_t> const& trx_hashes) const`
  Note: composition over already-shimmed primitives (`getTransactionLocation`, `getPeriodDataRaw`).
- `[x] getSystemTransaction(const trx_hash_t& hash) const`
- `[x] getPeriodSystemTransactionsHashes(PbftPeriod period) const`
- `[u] getPeriodSystemTransactions(PbftPeriod period) const`
  Note: can remain a composition helper if the primitive reads above are bridged.

### 4. PBFT Manager and Vote Read APIs

- `[x] getPbftMgrField(PbftMgrField field)`
- `[x] getPbftMgrStatus(PbftMgrStatus field)`
- `[x] getCertVotedBlockInRound() const`
- `[x] getProposedPbftBlocks()`
- `[x] getPbftHead(blk_hash_t const& hash)`
- `[x] getOwnVerifiedVotes()`
- `[x] getAllTwoTPlusOneVotes()`
- `[x] getRewardVotes()`

### 5. Pillar Read APIs

- `[x] getPillarBlock(PbftPeriod period) const`
- `[x] getLatestPillarBlock() const`
- `[x] getOwnPillarBlockVote() const`
- `[x] getCurrentPillarBlockData() const`

### 6. Metadata, Config, and Statistics Read APIs

- `[x] getGenesisHash()`
- `[x] getLastSortitionParams(size_t count)`
- `[x] getParamsChangeForPeriod(PbftPeriod period)`
- `[x] getStatusField(StatusDbField const& field)`
- `[x] getPeriodLambda(PbftPeriod period, bool find_closest)`
- `[x] getRoundsCountDynamicLambda()`
- `[x] getBlocksRewardsStats() const`
- `[~] getMajorVersion() const`
  Note: returns cached constructor state, not a direct `db_` call.
- `[~] getEarliestBlockNumber() const`
  Note: cached field today.
- `[~] getDagBlocksCount() const`
  Note: cached atomic; correctness depends on write-path parity.
- `[~] getDagEdgeCount() const`
- `[~] getNumTransactionExecuted()`
  Note: wrapper over `getStatusField`.
- `[~] getNumTransactionInDag()`
- `[~] getNumBlockExecuted()`

## Write-Phase Tracker

These methods mutate RocksDB state today and will need either direct Rust shims or a batch translation layer.

Batch migration note:
- Rust-mode `createWriteBatch` / `commitWriteBatch` now route through a Rust bridge batch registry.
- `Batch&` write APIs that use `insert(batch, ...)` / `remove(batch, ...)` now enqueue operations into Rust-side batches and
  apply on commit, preserving deferred batch-commit behavior.
- The explicit `saveDagBlock(..., Batch* write_batch_p)` pointer-based accumulation path is still intentionally unsupported in Rust mode
  (no external callers in the workspace).

### 1. DAG Write APIs

- `[~] saveDagBlock(const std::shared_ptr<DagBlock>& blk, Batch* write_batch_p = nullptr)`
  Note: Rust-backed for non-batch path (`write_batch_p == nullptr`).
  The explicit C++ `Batch*` accumulation path (`write_batch_p != nullptr`) has no external callers in the workspace and is intentionally not planned for Rust parity.
- `[x] updateDagBlockCounters(std::vector<std::shared_ptr<DagBlock>> blks)`
  TODO: Rust path currently updates one block per FFI call and derives counters from `status` reads each call.
  C++ path updates in-memory atomics and commits one batch for the whole vector.
  Revisit for parity/performance (bulk Rust API + single commit, and coherence of C++ cached DAG counters in Rust mode).
- `[u] removeDagBlockBatch(Batch& write_batch, blk_hash_t const& hash)`
- `[x] removeDagBlock(blk_hash_t const& hash)`
- `[x] addDagBlockPeriodToBatch(blk_hash_t const& hash, PbftPeriod period, uint32_t position, Batch& write_batch)`
- `[x] saveProposalPeriodDagLevelsMap(uint64_t level, PbftPeriod period)`
- `[x] addProposalPeriodDagLevelsMapToBatch(uint64_t level, PbftPeriod period, Batch& write_batch)`

### 2. Period Data and Finalized Chain Write APIs

- `[x] savePeriodData(const PeriodData& period_data, Batch& write_batch)`
- `[x] addPbftBlockPeriodToBatch(PbftPeriod period, taraxa::blk_hash_t const& pbft_block_hash, Batch& write_batch)`

### 3. Transaction Write APIs

- `[x] addTransactionToBatch(Transaction const& trx, Batch& write_batch)`
- `[x] removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch)`
- `[x] addTransactionLocationToBatch(Batch& write_batch, trx_hash_t const& trx, PbftPeriod period, uint32_t position, bool is_system = false)`
- `[x] addSystemTransactionToBatch(Batch& write_batch, SharedTransaction trx)`
- `[x] addPeriodSystemTransactions(Batch& write_batch, SharedTransactions trxs, PbftPeriod period)`

### 4. PBFT Manager and Vote Write APIs

- `[x] savePbftMgrField(PbftMgrField field, uint32_t value)`
- `[x] addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch)`
- `[x] savePbftMgrStatus(PbftMgrStatus field, bool const& value)`
- `[x] addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch)`
- `[x] saveCertVotedBlockInRound(PbftRound round, const std::shared_ptr<PbftBlock>& block)`
- `[x] removeCertVotedBlockInRound(Batch& write_batch)`
- `[x] saveProposedPbftBlock(const std::shared_ptr<PbftBlock>& block)`
- `[x] removeProposedPbftBlock(const blk_hash_t& block_hash, Batch& write_batch)`
- `[x] savePbftHead(blk_hash_t const& hash, std::string const& pbft_chain_head_str)`
- `[x] addPbftHeadToBatch(taraxa::blk_hash_t const& head_hash, std::string const& head_str, Batch& write_batch)`
- `[x] saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote)`
- `[x] clearOwnVerifiedVotes(Batch& write_batch, const std::vector<std::shared_ptr<PbftVote>>& own_verified_votes)`
- `[x] replaceTwoTPlusOneVotes(TwoTPlusOneVotedBlockType type, const std::vector<std::shared_ptr<PbftVote>>& votes)`
- `[x] replaceTwoTPlusOneVotesToBatch(TwoTPlusOneVotedBlockType type, const std::vector<std::shared_ptr<PbftVote>>& votes, Batch& write_batch)`
- `[x] removeExtraRewardVotes(const std::vector<vote_hash_t>& votes, Batch& write_batch)`
- `[x] saveExtraRewardVote(const std::shared_ptr<PbftVote>& vote)`

### 5. Pillar Write APIs

- `[x] savePillarBlock(const std::shared_ptr<pillar_chain::PillarBlock>& pillar_block)`
- `[x] saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>& vote)`
- `[x] saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb& current_pillar_block_data)`

### 6. Metadata, Config, and Statistics Write APIs

- `[x] setGenesisHash(const h256& genesis_hash)`
- `[x] saveSortitionParamsChange(PbftPeriod period, const SortitionParamsChange& params, Batch& batch)`
- `[x] saveStatusField(StatusDbField const& field, uint64_t value)`
- `[x] addStatusFieldToBatch(StatusDbField const& field, uint64_t value, Batch& write_batch)`
- `[x] savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch)`
- `[x] saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch)`
- `[x] saveBlockRewardsStats(uint64_t period, const rewards::BlockStats& stats, Batch& write_batch)`

## Infrastructure and Admin APIs

These are part of the public surface but they are not good first-wave Rust repository shims.
Scope note: snapshotting and DB migration flow are intentionally out of Rust rewrite scope. Some low-level admin APIs are also out of scope for now.
Scope note: APIs primarily exercised by `libraries/plugin/light` (iterator/range/compaction/history maintenance) are intentionally not migration blockers for Rust storage.

- `[x] createWriteBatch()`
- `[x] commitWriteBatch(Batch& write_batch, const rocksdb::WriteOptions& opts)`
- `[x] commitWriteBatch(Batch& write_batch)`
- `[!] getColumnIterator(const Column& c)`
- `[!] getColumnIterator(rocksdb::ColumnFamilyHandle* c)`
- `[!] DeleteRange(const Column& col, uint64_t begin, uint64_t end)`
- `[!] CompactRange(const Column& col, uint64_t begin, uint64_t end)`
- `[!] compactColumn(Column const& column)`
- `[!] clearColumnHistory(std::unordered_set<T>& to_keep, Column c)`
- `[u] rebuildColumns(const rocksdb::Options& options)`
- `[!] createSnapshot(PbftPeriod period)`
- `[u] deleteSnapshot(PbftPeriod period)`
- `[u] recoverToPeriod(PbftPeriod period)`
- `[u] loadSnapshots()`
- `[!] disableSnapshots()`
- `[!] enableSnapshots()`
- `[x] updateDbVersions()`
- `[!] deleteColumnData(const Column& c)`
- `[u] replaceColumn(const Column& to_be_replaced_col, std::unique_ptr<rocksdb::ColumnFamilyHandle>&& replacing_col)`
- `[u] copyColumn(rocksdb::ColumnFamilyHandle* orig_column, const std::string& new_col_name, bool move_data = false)`
- `[u] removeTempFiles() const`
- `[u] removeFilesWithPattern(const std::string& directory, const std::regex& pattern) const`
- `[u] deleteTmpDirectories(const std::string& path) const`
- `[!] forEach(Column const& col, OnEntry const& f)`

Notes:
- `path()` and `dbStoragePath()` currently have no external callers.
- `stateDbStoragePath()` is externally used.
- The constructor and destructor are also out of scope for repository-level tracking, except that constructor bootstrapping already creates `rust_storage_`.
- `Batch` still aliases `rocksdb::WriteBatch`; Rust-mode compatibility is implemented via a shim-side batch-handle map.

## Sequencing Recommendation

1. Maintain parity on shimmed APIs while expanding direct Rust coverage for remaining externally-used public APIs.
2. Benchmark and harden Rust-mode batch-heavy paths for parity/performance (especially hot final-chain and consensus write flows).
3. Keep snapshot/migration/admin APIs in C++ (out of scope), and avoid regression in dual-mode build behavior.

## Design Notes for the Next Batch

- Prefer shimming primitive reads over helper/composite reads.
  Example already implemented: `getPeriodDataRaw` is bridged, while `getPeriodData`, `getPeriodTransactions`, and finalized DAG helpers remain in C++ decode/composition code.
- Keep the shim boundary small.
  The current DAG work follows the right pattern: the public API remains in C++, and only the storage lookup logic crosses into Rust.
- Be careful with APIs that currently expose RocksDB types directly.
  `Batch`, iterators, compaction, and snapshot APIs are not simple repository calls and should not drive the first write migration design.
