# DbStorage Public API Rewrite Tracker

Source of truth: `libraries/core_libs/storage/include/storage/storage.hpp`

Goal: keep the `DbStorage` public interface stable while replacing all internal `db_` access with the Rust storage module behind small C++ shims.

Current migration direction:
- First focus on read paths.
- After the read surface is stable, move write APIs and batch handling.
- Keep RocksDB administration and file-management helpers out of the first wave unless they block functional parity.

## Legend

- `[x]` Rust-backed shim already exists in `storage.cpp`
- `[ ]` Public API still reads/writes via `db_`, `lookup`, `exist`, iterators, or `Batch`
- `[~]` Public helper or cached accessor; no dedicated Rust FFI entry point is required if underlying primitive reads are already shimmed
- `[u]` Public API with no external callers found in the workspace; defer for now
- `[!]` Infrastructure/admin API; not a first-wave repository shim and likely needs separate design work

## External Usage Audit

I checked each public method against workspace call sites outside `storage.hpp` and `storage.cpp`.

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

Current bridge coverage now includes the DAG read slice, proposal-period lookup, period-data primitives (`period_data` and `pbft_block_period`), finalized-chain receipts by period (`final_chain_receipt_by_period`), metadata/config reads (`genesis`, `status`, `sortition_params_change`, `period_lambda`, `rounds_count_dynamic_lambda`, `block_rewards_stats`), PBFT hash presence checks, PBFT manager/vote reads (`pbft_mgr_round_step`, `pbft_mgr_status`, `cert_voted_block_in_round`, `proposed_pbft_blocks`, `pbft_head`, `latest_round_own_votes`, `latest_round_two_t_plus_one_votes`, `extra_reward_votes`), pillar reads (`pillar_block`, `current_pillar_block_own_vote`, `current_pillar_block_data`), and a broader transaction read slice over `transactions`, `trx_period`, `system_transaction`, `period_data`, and `period_system_transactions`. Everything else below is still backed by C++ RocksDB access.

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
- `[ ] addDagBlockPeriodToBatch(blk_hash_t const& hash, PbftPeriod period, uint32_t position, Batch& write_batch)`
- `[x] saveProposalPeriodDagLevelsMap(uint64_t level, PbftPeriod period)`
- `[ ] addProposalPeriodDagLevelsMapToBatch(uint64_t level, PbftPeriod period, Batch& write_batch)`

### 2. Period Data and Finalized Chain Write APIs

- `[ ] savePeriodData(const PeriodData& period_data, Batch& write_batch)`
- `[ ] addPbftBlockPeriodToBatch(PbftPeriod period, taraxa::blk_hash_t const& pbft_block_hash, Batch& write_batch)`

### 3. Transaction Write APIs

- `[ ] addTransactionToBatch(Transaction const& trx, Batch& write_batch)`
- `[ ] removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch)`
- `[ ] addTransactionLocationToBatch(Batch& write_batch, trx_hash_t const& trx, PbftPeriod period, uint32_t position, bool is_system = false)`
- `[ ] addSystemTransactionToBatch(Batch& write_batch, SharedTransaction trx)`
- `[ ] addPeriodSystemTransactions(Batch& write_batch, SharedTransactions trxs, PbftPeriod period)`

### 4. PBFT Manager and Vote Write APIs

- `[ ] savePbftMgrField(PbftMgrField field, uint32_t value)`
- `[ ] addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch)`
- `[ ] savePbftMgrStatus(PbftMgrStatus field, bool const& value)`
- `[ ] addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch)`
- `[ ] saveCertVotedBlockInRound(PbftRound round, const std::shared_ptr<PbftBlock>& block)`
- `[ ] removeCertVotedBlockInRound(Batch& write_batch)`
- `[ ] saveProposedPbftBlock(const std::shared_ptr<PbftBlock>& block)`
- `[ ] removeProposedPbftBlock(const blk_hash_t& block_hash, Batch& write_batch)`
- `[ ] savePbftHead(blk_hash_t const& hash, std::string const& pbft_chain_head_str)`
- `[ ] addPbftHeadToBatch(taraxa::blk_hash_t const& head_hash, std::string const& head_str, Batch& write_batch)`
- `[ ] saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote)`
- `[ ] clearOwnVerifiedVotes(Batch& write_batch, const std::vector<std::shared_ptr<PbftVote>>& own_verified_votes)`
- `[ ] replaceTwoTPlusOneVotes(TwoTPlusOneVotedBlockType type, const std::vector<std::shared_ptr<PbftVote>>& votes)`
- `[ ] replaceTwoTPlusOneVotesToBatch(TwoTPlusOneVotedBlockType type, const std::vector<std::shared_ptr<PbftVote>>& votes, Batch& write_batch)`
- `[ ] removeExtraRewardVotes(const std::vector<vote_hash_t>& votes, Batch& write_batch)`
- `[ ] saveExtraRewardVote(const std::shared_ptr<PbftVote>& vote)`

### 5. Pillar Write APIs

- `[ ] savePillarBlock(const std::shared_ptr<pillar_chain::PillarBlock>& pillar_block)`
- `[ ] saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>& vote)`
- `[ ] saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb& current_pillar_block_data)`

### 6. Metadata, Config, and Statistics Write APIs

- `[ ] setGenesisHash(const h256& genesis_hash)`
- `[ ] saveSortitionParamsChange(PbftPeriod period, const SortitionParamsChange& params, Batch& batch)`
- `[ ] saveStatusField(StatusDbField const& field, uint64_t value)`
- `[ ] addStatusFieldToBatch(StatusDbField const& field, uint64_t value, Batch& write_batch)`
- `[ ] savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch)`
- `[ ] saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch)`
- `[ ] saveBlockRewardsStats(uint64_t period, const rewards::BlockStats& stats, Batch& write_batch)`

## Infrastructure and Admin APIs

These are part of the public surface but they are not good first-wave Rust repository shims.

- `[!] createWriteBatch()`
- `[!] commitWriteBatch(Batch& write_batch, const rocksdb::WriteOptions& opts)`
- `[!] commitWriteBatch(Batch& write_batch)`
- `[!] getColumnIterator(const Column& c)`
- `[!] getColumnIterator(rocksdb::ColumnFamilyHandle* c)`
- `[!] DeleteRange(const Column& col, uint64_t begin, uint64_t end)`
- `[!] CompactRange(const Column& col, uint64_t begin, uint64_t end)`
- `[!] compactColumn(Column const& column)`
- `[u] rebuildColumns(const rocksdb::Options& options)`
- `[!] createSnapshot(PbftPeriod period)`
- `[u] deleteSnapshot(PbftPeriod period)`
- `[u] recoverToPeriod(PbftPeriod period)`
- `[u] loadSnapshots()`
- `[!] disableSnapshots()`
- `[!] enableSnapshots()`
- `[!] updateDbVersions()`
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
- `Batch` currently aliases `rocksdb::WriteBatch`, so write-phase porting needs an explicit compatibility plan.

## Sequencing Recommendation

1. `period_data` primitive read shims are complete (`getPeriodDataRaw`, `getPeriodFromPbftHash`).
   Keep composite decode helpers in C++ for now.
2. Continue transaction read shims second.
   The main transaction retrieval primitives are now bridged (`getTransaction`, `getTransaction(period, position)`,
   `getSystemTransaction`, `getTransactionCount`, `transactionInDb`, `transactionFinalized`,
   `transactionsFinalized`, `getTransactionLocation`, `getAllNonfinalizedTransactions`,
   `getAllTransactionPeriod`, `getPeriodSystemTransactionsHashes`).
   Next step is optional batching/perf work (`transactionsInDb` or multi-hash lookups) and
   the remaining finalized-chain receipt read (`getTransactionReceipt`).
3. PBFT manager/vote read shims are now bridged (`getPbftMgrField`, `getPbftMgrStatus`,
   `getCertVotedBlockInRound`, `getProposedPbftBlocks`, `getPbftHead`,
   `getOwnVerifiedVotes`, `getAllTwoTPlusOneVotes`, `getRewardVotes`).
4. Metadata/config/statistics read shims are now bridged (`getGenesisHash`, `getLastSortitionParams`,
   `getParamsChangeForPeriod`, `getStatusField`, `getPeriodLambda`, `getRoundsCountDynamicLambda`,
   `getBlocksRewardsStats`).
5. Next target is finalized-chain per-transaction receipt read (`getTransactionReceipt`).
6. Only then decide how to represent write batches across the C++ and Rust boundary.

## Design Notes for the Next Batch

- Prefer shimming primitive reads over helper/composite reads.
  Example already implemented: `getPeriodDataRaw` is bridged, while `getPeriodData`, `getPeriodTransactions`, and finalized DAG helpers remain in C++ decode/composition code.
- Keep the shim boundary small.
  The current DAG work follows the right pattern: the public API remains in C++, and only the storage lookup logic crosses into Rust.
- Be careful with APIs that currently expose RocksDB types directly.
  `Batch`, iterators, compaction, and snapshot APIs are not simple repository calls and should not drive the first write migration design.
