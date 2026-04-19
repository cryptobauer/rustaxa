#include "config/version.hpp"
#include "dag/dag_block_bundle_rlp.hpp"
#include "dag/sortition_params_manager.hpp"
#include "storage/storage.hpp"
#include "transaction/system_transaction.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa {
namespace {
static constexpr uint16_t PBFT_BLOCK_POS_IN_PERIOD_DATA = 0;
static constexpr uint16_t CERT_VOTES_POS_IN_PERIOD_DATA = 1;
static constexpr uint16_t DAG_BLOCKS_POS_IN_PERIOD_DATA = 2;
static constexpr uint16_t TRANSACTIONS_POS_IN_PERIOD_DATA = 3;
static constexpr uint16_t PILLAR_VOTES_POS_IN_PERIOD_DATA = 4;

template <typename T>
std::array<uint8_t, 32> into_bytes_array(T const& val) {
  std::array<uint8_t, 32> bytes;
  std::memcpy(bytes.data(), val.data(), 32);
  return bytes;
}

template <typename T>
rust::Vec<uint8_t> into_rust_vec(T const& val) {
  rust::Vec<uint8_t> vec;
  vec.reserve(val.size());
  for (auto const& b : val) {
    vec.push_back(static_cast<uint8_t>(b));
  }
  return vec;
}

template <typename T>
T decode_little_endian(const Slice& key) {
  T value{};
  std::memcpy(&value, key.data(), sizeof(T));
  return value;
}

[[noreturn]] void throw_invalid_final_chain_key_size(const char* column_name, size_t got, size_t expected) {
  throw DbException("Invalid key size for " + std::string(column_name) + " in Rust shim mode. Got " +
                    std::to_string(got) + ", expected " + std::to_string(expected));
}

[[noreturn]] void throw_unimplemented_shim_api(const char* api_name) {
  throw DbException("DbStorage::" + std::string(api_name) + " is not implemented in Rust shim mode");
}
}  // namespace

DbStorage::DbStorage(fs::path const& path, uint32_t db_snapshot_each_n_pbft_block, uint32_t max_open_files,
                     uint32_t db_max_snapshots, PbftPeriod db_revert_to_period, addr_t node_addr, bool rebuild)
    : DbStorageOld(path, db_snapshot_each_n_pbft_block, max_open_files, db_max_snapshots, db_revert_to_period,
                   node_addr, rebuild) {
  try {
    rust_storage_ = rustaxa::storage::create_storage(path.string());
  } catch (std::exception const& e) {
    LOG(log_er_) << "Error: " << e.what() << std::endl;
    throw DbException(std::string("Rust storage init failed: ") + e.what());
  }
}

Batch DbStorage::createWriteBatch() { return Batch(); }

DbStorage::~DbStorage() {
  std::unordered_map<Batch*, uint64_t> batches_to_drop;
  {
    std::lock_guard<std::mutex> lock(rust_batches_mutex_);
    batches_to_drop.swap(rust_batches_);
  }

  for (const auto& [_, batch_id] : batches_to_drop) {
    rust_storage_.value()->drop_write_batch(batch_id);
  }
}

rust::Vec<uint8_t> DbStorage::sliceToRustVec(const Slice& slice) {
  rust::Vec<uint8_t> vec;
  vec.reserve(slice.size());
  auto data = reinterpret_cast<const uint8_t*>(slice.data());
  for (size_t i = 0; i < slice.size(); ++i) {
    vec.push_back(data[i]);
  }
  return vec;
}

uint64_t DbStorage::getOrCreateRustBatch(Batch& batch) {
  std::lock_guard<std::mutex> lock(rust_batches_mutex_);
  auto it = rust_batches_.find(&batch);
  if (it != rust_batches_.end()) {
    return it->second;
  }

  auto batch_id = rust_storage_.value()->create_write_batch();
  rust_batches_[&batch] = batch_id;
  return batch_id;
}

void DbStorage::commitWriteBatch(Batch& write_batch, const rocksdb::WriteOptions& opts) {
  std::optional<uint64_t> batch_id;
  {
    std::lock_guard<std::mutex> lock(rust_batches_mutex_);
    auto it = rust_batches_.find(&write_batch);
    if (it != rust_batches_.end()) {
      batch_id = it->second;
      rust_batches_.erase(it);
    }
  }

  if (batch_id.has_value()) {
    rust_storage_.value()->commit_write_batch(*batch_id, opts.sync);
  } else if (write_batch.Count() != 0) {
    throw DbException("commitWriteBatch called with unsupported non-rust batch content");
  }

  write_batch.Clear();
}

void DbStorage::commitWriteBatch(Batch& write_batch) { commitWriteBatch(write_batch, async_write_); }

void DbStorage::DeleteRange(const Column& col, uint64_t begin, uint64_t end) {
  (void)col;
  (void)begin;
  (void)end;
  throw_unimplemented_shim_api("DeleteRange");
}

void DbStorage::CompactRange(const Column& col, uint64_t begin, uint64_t end) {
  (void)col;
  (void)begin;
  (void)end;
  throw_unimplemented_shim_api("CompactRange");
}

void DbStorage::rebuildColumns(const rocksdb::Options& options) {
  (void)options;
  throw_unimplemented_shim_api("rebuildColumns");
}

bool DbStorage::createSnapshot(PbftPeriod period) {
  (void)period;
  throw_unimplemented_shim_api("createSnapshot");
}

void DbStorage::deleteSnapshot(PbftPeriod period) {
  (void)period;
  throw_unimplemented_shim_api("deleteSnapshot");
}

void DbStorage::recoverToPeriod(PbftPeriod period) {
  (void)period;
  throw_unimplemented_shim_api("recoverToPeriod");
}

void DbStorage::loadSnapshots() { throw_unimplemented_shim_api("loadSnapshots"); }

void DbStorage::disableSnapshots() { throw_unimplemented_shim_api("disableSnapshots"); }

void DbStorage::enableSnapshots() { throw_unimplemented_shim_api("enableSnapshots"); }

void DbStorage::deleteColumnData(const Column& c) {
  (void)c;
  throw_unimplemented_shim_api("deleteColumnData");
}

void DbStorage::replaceColumn(const Column& to_be_replaced_col,
                              std::unique_ptr<rocksdb::ColumnFamilyHandle>&& replacing_col) {
  (void)to_be_replaced_col;
  (void)replacing_col;
  throw_unimplemented_shim_api("replaceColumn");
}

std::unique_ptr<rocksdb::ColumnFamilyHandle> DbStorage::copyColumn(rocksdb::ColumnFamilyHandle* orig_column,
                                                                   const std::string& new_col_name, bool move_data) {
  (void)orig_column;
  (void)new_col_name;
  (void)move_data;
  throw_unimplemented_shim_api("copyColumn");
}

void DbStorage::removeTempFiles() const { throw_unimplemented_shim_api("removeTempFiles"); }

void DbStorage::removeFilesWithPattern(const std::string& directory, const std::regex& pattern) const {
  (void)directory;
  (void)pattern;
  throw_unimplemented_shim_api("removeFilesWithPattern");
}

void DbStorage::deleteTmpDirectories(const std::string& path) const {
  (void)path;
  throw_unimplemented_shim_api("deleteTmpDirectories");
}

uint32_t DbStorage::getMajorVersion() const { throw_unimplemented_shim_api("getMajorVersion"); }

std::unique_ptr<rocksdb::Iterator> DbStorage::getColumnIterator(const Column& c) {
  (void)c;
  throw_unimplemented_shim_api("getColumnIterator(Column)");
}

std::unique_ptr<rocksdb::Iterator> DbStorage::getColumnIterator(rocksdb::ColumnFamilyHandle* c) {
  (void)c;
  throw_unimplemented_shim_api("getColumnIterator(ColumnFamilyHandle*)");
}

std::string DbStorage::lookupFinalChainMeta(const Slice& key) const {
  if (key.size() != sizeof(uint32_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_meta.name().c_str(), key.size(), sizeof(uint32_t));
  }

  auto const meta_key = decode_little_endian<uint32_t>(key);
  auto rust_value = rust_storage_.value()->get_final_chain_meta_value(meta_key);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockByNumber(const Slice& key) const {
  if (key.size() != sizeof(uint64_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_by_number.name().c_str(), key.size(), sizeof(uint64_t));
  }

  auto const block_number = decode_little_endian<uint64_t>(key);
  auto rust_value = rust_storage_.value()->get_final_chain_block_header(block_number);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockHashByNumber(const Slice& key) const {
  if (key.size() != sizeof(uint64_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_hash_by_number.name().c_str(), key.size(),
                                       sizeof(uint64_t));
  }

  auto const block_number = decode_little_endian<uint64_t>(key);
  auto rust_value = rust_storage_.value()->get_final_chain_block_hash_by_number(block_number);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockNumberByHash(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_number_by_hash.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> hash{};
  std::memcpy(hash.data(), key.data(), hash.size());
  auto rust_value = rust_storage_.value()->get_final_chain_block_number_by_hash(hash);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainLogBloomsChunk(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_log_blooms_index.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> chunk_id{};
  std::memcpy(chunk_id.data(), key.data(), chunk_id.size());
  auto rust_value = rust_storage_.value()->get_final_chain_log_blooms_chunk(chunk_id);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainReceiptByTrxHash(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_receipt_by_trx_hash.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> trx_hash{};
  std::memcpy(trx_hash.data(), key.data(), trx_hash.size());
  auto rust_value = rust_storage_.value()->get_final_chain_receipt_by_trx_hash(trx_hash);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

void DbStorage::updateDbVersions() {
  saveStatusField(StatusDbField::DbMajorVersion, TARAXA_DB_MAJOR_VERSION);
  saveStatusField(StatusDbField::DbMinorVersion, TARAXA_DB_MINOR_VERSION);
  kMajorVersion_ = TARAXA_DB_MAJOR_VERSION;
}

void DbStorage::setGenesisHash(const h256& genesis_hash) {
  auto bytes = into_bytes_array(genesis_hash);
  rust_storage_.value()->set_genesis_hash(bytes);
}

std::optional<h256> DbStorage::getGenesisHash() {
  auto rust_hash = rust_storage_.value()->get_genesis_hash();
  if (!rust_hash.empty()) {
    return h256(dev::bytes(rust_hash.begin(), rust_hash.end()));
  }
  return {};
}

std::shared_ptr<DagBlock> DbStorage::getDagBlock(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto rlp_bytes = rust_storage_.value()->get_dag_block(h_arr);
  dev::RLP rlp(dev::bytesConstRef(rlp_bytes.data(), rlp_bytes.size()));
  return std::make_shared<DagBlock>(rlp);
}

bool DbStorage::dagBlockInDb(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  if (rust_storage_.value()->dag_block_in_db(h_arr)) return true;
  return false;
}

std::set<blk_hash_t> DbStorage::getBlocksByLevel(level_t level) {
  auto bytes = rust_storage_.value()->get_blocks_by_level(level);
  std::set<blk_hash_t> res;
  for (size_t i = 0; i < bytes.size(); i += 32) {
    blk_hash_t h;
    std::memcpy(h.data(), bytes.data() + i, 32);
    res.insert(h);
  }
  return res;
}

level_t DbStorage::getLastBlocksLevel() const { return rust_storage_.value()->get_last_blocks_level(); }

std::vector<std::shared_ptr<DagBlock>> DbStorage::getDagBlocksAtLevel(level_t level, int number_of_levels) {
  std::vector<std::shared_ptr<DagBlock>> res;
  auto blocks_rlp = rust_storage_.value()->get_dag_blocks_at_level(level, (uint32_t)number_of_levels);
  for (auto const& item : blocks_rlp) {
    dev::RLP rlp(dev::bytesConstRef(item.data.data(), item.data.size()));
    res.push_back(std::make_shared<DagBlock>(rlp));
  }
  return res;
}

std::map<level_t, std::vector<std::shared_ptr<DagBlock>>> DbStorage::getNonfinalizedDagBlocks() {
  std::map<level_t, std::vector<std::shared_ptr<DagBlock>>> res;
  auto levels = rust_storage_.value()->get_nonfinalized_dag_blocks();
  for (auto const& item : levels) {
    std::vector<std::shared_ptr<DagBlock>> blocks;
    for (auto const& block_rlp : item.blocks) {
      dev::RLP rlp(dev::bytesConstRef(block_rlp.data.data(), block_rlp.data.size()));
      blocks.push_back(std::make_shared<DagBlock>(rlp));
    }
    res[item.level] = blocks;
  }
  return res;
}

SharedTransactions DbStorage::getAllNonfinalizedTransactions() {
  SharedTransactions res;
  auto trxs = rust_storage_.value()->get_all_nonfinalized_transactions();
  res.reserve(trxs.size());
  for (auto const& trx_rlp : trxs) {
    res.emplace_back(std::make_shared<Transaction>(dev::bytes(trx_rlp.data.begin(), trx_rlp.data.end())));
  }
  return res;
}

void DbStorage::removeDagBlock(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  rust_storage_.value()->remove_dag_block(h_arr);
}

void DbStorage::removeDagBlockBatch(Batch& write_batch, blk_hash_t const& hash) {
  (void)write_batch;
  (void)hash;
  throw_unimplemented_shim_api("removeDagBlockBatch");
}

void DbStorage::updateDagBlockCounters(std::vector<std::shared_ptr<DagBlock>> blks) {
  for (auto const& blk : blks) {
    auto hash = blk->getHash();
    auto h_arr = into_bytes_array(hash);
    rust_storage_.value()->update_dag_block_counter(h_arr, blk->getLevel(), blk->getTips().size());
  }
}

void DbStorage::saveDagBlock(const std::shared_ptr<DagBlock>& blk, Batch* write_batch_p) {
  // There are no callers of this method that pass in a write batch. So no need to ever
  // do more than we do here.
  if (!write_batch_p) {
    auto block_hash = blk->getHash();
    auto h_arr = into_bytes_array(block_hash);

    auto block_bytes = blk->rlp(true);
    auto block_rlp = into_rust_vec(block_bytes);

    rust_storage_.value()->save_dag_block(h_arr, blk->getLevel(), blk->getTips().size(), std::move(block_rlp));
  } else {
    throw DbException("saveDagBlock was called with write batch but is not implemented.");
  }
}

void DbStorage::saveSortitionParamsChange(PbftPeriod period, const SortitionParamsChange& params, Batch& batch) {
  insert(batch, Columns::sortition_params_change, toSlice(period), toSlice(params.rlp()));
}

std::deque<SortitionParamsChange> DbStorage::getLastSortitionParams(size_t count) {
  std::deque<SortitionParamsChange> changes;

  auto rust_changes = rust_storage_.value()->get_last_sortition_params(static_cast<uint64_t>(count));
  for (auto const& change_rlp : rust_changes) {
    auto bytes = dev::bytes(change_rlp.data.begin(), change_rlp.data.end());
    changes.emplace_back(SortitionParamsChange::from_rlp(dev::RLP(bytes)));
  }
  return changes;
}

std::optional<SortitionParamsChange> DbStorage::getParamsChangeForPeriod(PbftPeriod period) {
  auto rust_change = rust_storage_.value()->get_params_change_for_period(period);
  if (rust_change.empty()) {
    return {};
  }
  auto bytes = dev::bytes(rust_change.begin(), rust_change.end());
  return SortitionParamsChange::from_rlp(dev::RLP(bytes));
}

void DbStorage::savePeriodData(const PeriodData& period_data, Batch& write_batch) {
  const auto period = period_data.pbft_blk->getPeriod();
  addPbftBlockPeriodToBatch(period, period_data.pbft_blk->getBlockHash(), write_batch);

  uint32_t block_pos = 0;
  for (auto const& block : period_data.dag_blocks) {
    remove(write_batch, Columns::dag_blocks, toSlice(block->getHash().asBytes()));
    addDagBlockPeriodToBatch(block->getHash(), period, block_pos, write_batch);
    block_pos++;
  }

  uint32_t trx_pos = 0;
  for (auto const& trx : period_data.transactions) {
    removeTransactionToBatch(trx->getHash(), write_batch);
    addTransactionLocationToBatch(write_batch, trx->getHash(), period, trx_pos);
    trx_pos++;
  }

  insert(write_batch, Columns::period_data, toSlice(period), toSlice(period_data.rlp()));
}

dev::bytes DbStorage::getPeriodDataRaw(PbftPeriod period) const {
  auto period_data = rust_storage_.value()->get_period_data_raw(period);
  return dev::bytes(period_data.begin(), period_data.end());
}

uint64_t DbStorage::getEarliestBlockNumber() const {
  // Seems like a light node feature that never got implement.
  return 0;
}

std::optional<PeriodData> DbStorage::getPeriodData(PbftPeriod period) const {
  auto period_data_bytes = getPeriodDataRaw(period);
  if (period_data_bytes.empty()) {
    return {};
  }

  return PeriodData{std::move(period_data_bytes)};
}

std::optional<PbftBlock> DbStorage::getPbftBlock(PbftPeriod period) const {
  auto period_data = getPeriodDataRaw(period);
  if (period_data.size() > 0) {
    auto period_data_rlp = dev::RLP(period_data);
    return std::optional<PbftBlock>(period_data_rlp[PBFT_BLOCK_POS_IN_PERIOD_DATA]);
  }
  return {};
}

blk_hash_t DbStorage::getPeriodBlockHash(PbftPeriod period) const {
  const auto& blk = getPbftBlock(period);
  if (blk.has_value()) {
    return blk->getBlockHash();
  }
  return {};
}

SharedTransactions DbStorage::transactionsFromPeriodDataRlp(PbftPeriod period, const dev::RLP& period_data_rlp) const {
  (void)period;
  (void)period_data_rlp;
  throw_unimplemented_shim_api("transactionsFromPeriodDataRlp");
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getPeriodCertVotes(PbftPeriod period) const {
  auto period_data = getPeriodDataRaw(period);
  if (period_data.empty()) {
    return {};
  }

  auto period_data_rlp = dev::RLP(period_data);
  auto votes_rlp = period_data_rlp[CERT_VOTES_POS_IN_PERIOD_DATA];
  if (votes_rlp.itemCount() == 0) {
    return {};
  }
  return decodePbftVotesBundleRlp(votes_rlp);
}

std::optional<SharedTransactions> DbStorage::getPeriodTransactions(PbftPeriod period) const {
  const auto period_data = getPeriodDataRaw(period);
  if (!period_data.size()) {
    return std::nullopt;
  }

  auto period_data_rlp = dev::RLP(period_data);
  SharedTransactions ret;
  ret.reserve(period_data_rlp[TRANSACTIONS_POS_IN_PERIOD_DATA].size());
  for (auto&& transaction_data : period_data_rlp[TRANSACTIONS_POS_IN_PERIOD_DATA]) {
    ret.emplace_back(std::make_shared<Transaction>(std::move(transaction_data)));
  }

  auto system_transaction_hashes = getPeriodSystemTransactionsHashes(period);
  ret.reserve(ret.size() + system_transaction_hashes.size());
  for (const auto& trx_hash : system_transaction_hashes) {
    ret.emplace_back(getSystemTransaction(trx_hash));
  }

  return ret;
}

std::vector<std::shared_ptr<PillarVote>> DbStorage::getPeriodPillarVotes(PbftPeriod period) const {
  const auto period_data = getPeriodDataRaw(period);
  if (!period_data.size()) {
    return {};
  }

  auto period_data_rlp = dev::RLP(period_data);
  // This could potentially happen if getPeriodPillarVotes is called for period that does not contain pillar votes
  if (period_data_rlp.itemCount() < PILLAR_VOTES_POS_IN_PERIOD_DATA) {
    return {};
  }

  return decodePillarVotesBundleRlp(period_data_rlp[PILLAR_VOTES_POS_IN_PERIOD_DATA]);
}

void DbStorage::savePillarBlock(const std::shared_ptr<pillar_chain::PillarBlock>& pillar_block) {
  auto pillar_rlp_bytes = pillar_block->getRlp();
  auto pillar_rlp = into_rust_vec(pillar_rlp_bytes);
  rust_storage_.value()->save_pillar_block(pillar_block->getPeriod(), std::move(pillar_rlp));
}

std::shared_ptr<pillar_chain::PillarBlock> DbStorage::getPillarBlock(PbftPeriod period) const {
  auto data = rust_storage_.value()->get_pillar_block(period);
  if (data.empty()) {
    return {};
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<pillar_chain::PillarBlock>(dev::RLP(rust_bytes));
}

std::shared_ptr<pillar_chain::PillarBlock> DbStorage::getLatestPillarBlock() const {
  auto data = rust_storage_.value()->get_latest_pillar_block();
  if (data.empty()) {
    return {};
  }

  auto bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<pillar_chain::PillarBlock>(dev::RLP(bytes));
}

void DbStorage::saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>& vote) {
  auto vote_bytes = util::rlp_enc(vote);
  auto vote_rlp = into_rust_vec(vote_bytes);
  rust_storage_.value()->save_own_pillar_block_vote(std::move(vote_rlp));
}

std::shared_ptr<PillarVote> DbStorage::getOwnPillarBlockVote() const {
  auto data = rust_storage_.value()->get_own_pillar_block_vote();
  if (data.empty()) {
    return nullptr;
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<PillarVote>(dev::RLP(rust_bytes));
}

void DbStorage::saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb& current_pillar_block_data) {
  auto data_bytes = util::rlp_enc(current_pillar_block_data);
  auto data_rlp = into_rust_vec(data_bytes);
  rust_storage_.value()->save_current_pillar_block_data(std::move(data_rlp));
}

std::optional<pillar_chain::CurrentPillarBlockDataDb> DbStorage::getCurrentPillarBlockData() const {
  auto data = rust_storage_.value()->get_current_pillar_block_data();
  if (data.empty()) {
    return {};
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return util::rlp_dec<pillar_chain::CurrentPillarBlockDataDb>(dev::RLP(rust_bytes));
}

void DbStorage::addTransactionLocationToBatch(Batch& write_batch, trx_hash_t const& trx_hash, PbftPeriod period,
                                              uint32_t position, bool is_system) {
  dev::RLPStream s;
  s.appendList(2 + is_system);
  s << period;
  s << position;
  if (is_system) {
    s << is_system;
  }
  insert(write_batch, Columns::trx_period, toSlice(trx_hash.asBytes()), toSlice(s.invalidate()));
}

std::optional<TransactionLocation> DbStorage::getTransactionLocation(trx_hash_t const& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto location_bytes = rust_storage_.value()->get_transaction_location(h_arr);
  if (!location_bytes.empty()) {
    auto location_data = dev::bytes(location_bytes.begin(), location_bytes.end());
    // Don't use std::move - RLP stores a reference and needs data to stay alive
    return TransactionLocation::fromRlp(dev::RLP(location_data));
  }
  return std::nullopt;
}

std::vector<bool> DbStorage::transactionsInDb(std::vector<trx_hash_t> const& trx_hashes) {
  (void)trx_hashes;
  throw_unimplemented_shim_api("transactionsInDb");
}

std::vector<bool> DbStorage::transactionsFinalized(std::vector<trx_hash_t> const& trx_hashes) {
  std::vector<bool> result(trx_hashes.size(), false);
  for (size_t i = 0; i < trx_hashes.size(); ++i) {
    auto h_arr = into_bytes_array(trx_hashes[i]);
    result[i] = rust_storage_.value()->transaction_finalized(h_arr);
  }
  return result;
}

std::unordered_map<trx_hash_t, PbftPeriod> DbStorage::getAllTransactionPeriod() {
  std::unordered_map<trx_hash_t, PbftPeriod> res;
  auto data = rust_storage_.value()->get_all_transaction_period();
  res.reserve(data.size());
  for (auto const& item : data) {
    auto hash_bytes = dev::bytes(item.hash.begin(), item.hash.end());
    res[trx_hash_t(hash_bytes)] = item.period;
  }
  return res;
}

void DbStorage::saveProposedPbftBlock(const std::shared_ptr<PbftBlock>& block) {
  auto block_hash = block->getBlockHash();
  auto h_arr = into_bytes_array(block_hash);
  auto block_bytes = block->rlp(true);
  auto block_rlp = into_rust_vec(block_bytes);
  rust_storage_.value()->save_proposed_pbft_block(h_arr, std::move(block_rlp));
}

void DbStorage::removeProposedPbftBlock(const blk_hash_t& block_hash, Batch& write_batch) {
  remove(write_batch, Columns::proposed_pbft_blocks, toSlice(block_hash.asBytes()));
}

std::vector<std::shared_ptr<PbftBlock>> DbStorage::getProposedPbftBlocks() {
  std::vector<std::shared_ptr<PbftBlock>> res;
  auto blocks = rust_storage_.value()->get_proposed_pbft_blocks();
  res.reserve(blocks.size());
  for (auto const& block_rlp : blocks) {
    res.emplace_back(std::make_shared<PbftBlock>(dev::bytes(block_rlp.data.begin(), block_rlp.data.end())));
  }
  return res;
}

std::shared_ptr<Transaction> DbStorage::getTransaction(trx_hash_t const& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto rust_data = rust_storage_.value()->get_transaction(h_arr);
  if (!rust_data.empty()) {
    return std::make_shared<Transaction>(dev::bytes(rust_data.begin(), rust_data.end()));
  }
  auto rust_location = getTransactionLocation(hash);
  if (rust_location && !rust_location->is_system) {
    return getTransaction(rust_location->period, rust_location->position);
  } else {
    return getSystemTransaction(hash);
  }
}

std::shared_ptr<Transaction> DbStorage::getTransaction(PbftPeriod period, uint32_t position) const {
  auto data = rust_storage_.value()->get_transaction_by_period_position(period, position);
  if (!data.empty()) {
    return std::make_shared<Transaction>(dev::bytes(data.begin(), data.end()));
  }
  return nullptr;
}

uint64_t DbStorage::getTransactionCount(PbftPeriod period) const {
  return rust_storage_.value()->get_transaction_count(period);
}

std::optional<TransactionReceipt> DbStorage::getTransactionReceipt(EthBlockNumber blk_n, uint64_t position) const {
  (void)blk_n;
  (void)position;
  throw_unimplemented_shim_api("getTransactionReceipt");
}

SharedTransactions DbStorage::getFinalizedTransactions(std::vector<trx_hash_t> const& trx_hashes) const {
  SharedTransactions trxs;
  std::map<PbftPeriod, std::set<uint32_t>> period_map;
  trxs.reserve(trx_hashes.size());
  for (auto const& tx_hash : trx_hashes) {
    auto trx_period = getTransactionLocation(tx_hash);
    if (trx_period.has_value()) {
      period_map[trx_period->period].insert(trx_period->position);
    }
  }
  for (auto it : period_map) {
    const auto period_data = getPeriodDataRaw(it.first);
    if (!period_data.size()) {
      assert(false);
    }

    auto const transactions_rlp = dev::RLP(period_data)[TRANSACTIONS_POS_IN_PERIOD_DATA];
    for (auto pos : it.second) {
      trxs.emplace_back(std::make_shared<Transaction>(transactions_rlp[pos]));
    }
  }

  return trxs;
}

void DbStorage::addSystemTransactionToBatch(Batch& write_batch, SharedTransaction trx) {
  insert(write_batch, Columns::system_transaction, toSlice(trx->getHash().asBytes()), toSlice(trx->rlp()));
}

std::shared_ptr<Transaction> DbStorage::getSystemTransaction(const trx_hash_t& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto rust_data = rust_storage_.value()->get_system_transaction(h_arr);
  if (!rust_data.empty()) {
    // construct as system transaction to have proper sender
    return std::make_shared<SystemTransaction>(dev::bytes(rust_data.begin(), rust_data.end()));
  }
  return nullptr;
}

void DbStorage::addPeriodSystemTransactions(Batch& write_batch, SharedTransactions trxs, PbftPeriod period) {
  std::vector<trx_hash_t> trx_hashes;
  trx_hashes.reserve(trxs.size());
  std::transform(trxs.begin(), trxs.end(), std::back_inserter(trx_hashes),
                 [](const auto& trx) { return trx->getHash(); });
  auto hashes_rlp = util::rlp_enc(trx_hashes);
  insert(write_batch, Columns::period_system_transactions, toSlice(period), toSlice(hashes_rlp));
}

std::vector<trx_hash_t> DbStorage::getPeriodSystemTransactionsHashes(PbftPeriod period) const {
  auto rust_data = rust_storage_.value()->get_period_system_transactions_hashes(period);
  if (rust_data.empty()) {
    return {};
  }
  auto hashes_data = dev::bytes(rust_data.begin(), rust_data.end());
  return util::rlp_dec<std::vector<trx_hash_t>>(dev::RLP(hashes_data));
}

SharedTransactions DbStorage::getPeriodSystemTransactions(PbftPeriod period) const {
  (void)period;
  throw_unimplemented_shim_api("getPeriodSystemTransactions");
}

SharedTransactionReceipts DbStorage::getBlockReceipts(PbftPeriod period) const {
  auto rust_value = rust_storage_.value()->get_block_receipt(period);
  if (rust_value.empty()) {
    return {};
  }
  auto data_bytes = dev::bytes(rust_value.begin(), rust_value.end());
  return std::make_shared<std::vector<TransactionReceipt>>(
      util::rlp_dec<std::vector<TransactionReceipt>>(dev::RLP(data_bytes)));
}

void DbStorage::addTransactionToBatch(Transaction const& trx, Batch& write_batch) {
  insert(write_batch, Columns::transactions, toSlice(trx.getHash().asBytes()), toSlice(trx.rlp()));
}

void DbStorage::removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch) {
  remove(write_batch, Columns::transactions, toSlice(trx));
}

bool DbStorage::transactionInDb(trx_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  return rust_storage_.value()->transaction_in_db(h_arr);
}

bool DbStorage::transactionFinalized(trx_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  return rust_storage_.value()->transaction_finalized(h_arr);
}

uint64_t DbStorage::getStatusField(StatusDbField const& field) {
  return rust_storage_.value()->get_status_field(static_cast<uint8_t>(field));
}

void DbStorage::saveStatusField(StatusDbField const& field, uint64_t value) {
  rust_storage_.value()->save_status_field(static_cast<uint8_t>(field), value);
}

void DbStorage::addStatusFieldToBatch(StatusDbField const& field, uint64_t value, Batch& write_batch) {
  insert(write_batch, Columns::status, toSlice(static_cast<uint8_t>(field)), toSlice(value));
}

uint32_t DbStorage::getPbftMgrField(PbftMgrField field) {
  return rust_storage_.value()->get_pbft_mgr_field(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrField(PbftMgrField field, uint32_t value) {
  rust_storage_.value()->save_pbft_mgr_field(static_cast<uint8_t>(field), value);
}

void DbStorage::addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch) {
  insert(write_batch, Columns::pbft_mgr_round_step, toSlice(static_cast<uint8_t>(field)), toSlice(value));
}

bool DbStorage::getPbftMgrStatus(PbftMgrStatus field) {
  return rust_storage_.value()->get_pbft_mgr_status(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrStatus(PbftMgrStatus field, bool const& value) {
  rust_storage_.value()->save_pbft_mgr_status(static_cast<uint8_t>(field), value);
}

void DbStorage::addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch) {
  insert(write_batch, Columns::pbft_mgr_status, toSlice(field), toSlice(value));
}

void DbStorage::saveCertVotedBlockInRound(PbftRound round, const std::shared_ptr<PbftBlock>& block) {
  assert(block);
  auto block_bytes = block->rlp(true);
  auto block_rlp = into_rust_vec(block_bytes);
  rust_storage_.value()->save_cert_voted_block_in_round(round, std::move(block_rlp));
}

std::optional<std::pair<PbftRound, std::shared_ptr<PbftBlock>>> DbStorage::getCertVotedBlockInRound() const {
  auto rust_value = rust_storage_.value()->get_cert_voted_block_in_round();
  if (rust_value.empty()) {
    return {};
  }

  auto value_bytes = dev::bytes(rust_value.begin(), rust_value.end());
  auto rust_value_rlp = dev::RLP(value_bytes);
  assert(rust_value_rlp.itemCount() == 2);

  std::pair<PbftRound, std::shared_ptr<PbftBlock>> rust_ret;
  rust_ret.first = rust_value_rlp[0].toInt<PbftRound>();
  rust_ret.second = std::make_shared<PbftBlock>(rust_value_rlp[1]);

  return rust_ret;
}

void DbStorage::removeCertVotedBlockInRound(Batch& write_batch) {
  remove(write_batch, Columns::cert_voted_block_in_round, 0);
}

std::optional<PbftBlock> DbStorage::getPbftBlock(blk_hash_t const& hash) {
  auto res = getPeriodFromPbftHash(hash);
  if (res.first) {
    return getPbftBlock(res.second);
  }
  return {};
}

bool DbStorage::pbftBlockInDb(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto res = rust_storage_.value()->pbft_block_in_db(h_arr);
  return res;
}

std::string DbStorage::getPbftHead(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto data = rust_storage_.value()->get_pbft_head(h_arr);
  return std::string(data.begin(), data.end());
}

void DbStorage::savePbftHead(blk_hash_t const& hash, std::string const& pbft_chain_head_str) {
  auto h_arr = into_bytes_array(hash);
  auto head_bytes = into_rust_vec(pbft_chain_head_str);
  rust_storage_.value()->save_pbft_head(h_arr, std::move(head_bytes));
}

void DbStorage::addPbftHeadToBatch(taraxa::blk_hash_t const& head_hash, std::string const& head_str,
                                   Batch& write_batch) {
  insert(write_batch, Columns::pbft_head, toSlice(head_hash.asBytes()), head_str);
}

void DbStorage::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  auto vote_hash = vote->getHash();
  auto h_arr = into_bytes_array(vote_hash);

  auto vote_bytes = vote->rlp(true, true);
  auto vote_rlp = into_rust_vec(vote_bytes);
  rust_storage_.value()->save_own_verified_vote(h_arr, std::move(vote_rlp));
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getOwnVerifiedVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = rust_storage_.value()->get_own_verified_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::clearOwnVerifiedVotes(Batch& write_batch,
                                      const std::vector<std::shared_ptr<PbftVote>>& own_verified_votes) {
  for (const auto& own_vote : own_verified_votes) {
    remove(write_batch, Columns::latest_round_own_votes, own_vote->getHash().asBytes());
  }
}

void DbStorage::replaceTwoTPlusOneVotes(TwoTPlusOneVotedBlockType type,
                                        const std::vector<std::shared_ptr<PbftVote>>& votes) {
  dev::RLPStream rust_votes_stream(votes.size());
  for (const auto& vote : votes) {
    rust_votes_stream.appendRaw(vote->rlp(true, true));
  }
  auto votes_bundle = rust_votes_stream.out();
  auto votes_bundle_rlp = into_rust_vec(votes_bundle);
  rust_storage_.value()->replace_two_t_plus_one_votes(static_cast<uint8_t>(type), std::move(votes_bundle_rlp));
}

void DbStorage::replaceTwoTPlusOneVotesToBatch(TwoTPlusOneVotedBlockType type,
                                               const std::vector<std::shared_ptr<PbftVote>>& votes,
                                               Batch& write_batch) {
  remove(write_batch, Columns::latest_round_two_t_plus_one_votes, static_cast<uint8_t>(type));

  dev::RLPStream rust_votes_stream(votes.size());
  for (const auto& vote : votes) {
    rust_votes_stream.appendRaw(vote->rlp(true, true));
  }
  insert(write_batch, Columns::latest_round_two_t_plus_one_votes, static_cast<uint8_t>(type), rust_votes_stream.out());
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getAllTwoTPlusOneVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = rust_storage_.value()->get_all_two_t_plus_one_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::removeExtraRewardVotes(const std::vector<vote_hash_t>& votes, Batch& write_batch) {
  for (const auto& v : votes) {
    remove(write_batch, Columns::extra_reward_votes, v.asBytes());
  }
}

void DbStorage::saveExtraRewardVote(const std::shared_ptr<PbftVote>& vote) {
  auto vote_hash = vote->getHash();
  auto h_arr = into_bytes_array(vote_hash);
  auto vote_bytes = vote->rlp(true, true);
  auto vote_rlp = into_rust_vec(vote_bytes);
  rust_storage_.value()->save_extra_reward_vote(h_arr, std::move(vote_rlp));
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getRewardVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = rust_storage_.value()->get_reward_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::addPbftBlockPeriodToBatch(PbftPeriod period, taraxa::blk_hash_t const& pbft_block_hash,
                                          Batch& write_batch) {
  insert(write_batch, Columns::pbft_block_period, toSlice(pbft_block_hash.asBytes()), toSlice(period));
}

std::pair<bool, PbftPeriod> DbStorage::getPeriodFromPbftHash(taraxa::blk_hash_t const& pbft_block_hash) {
  auto h_arr = into_bytes_array(pbft_block_hash);
  auto res = rust_storage_.value()->get_period_from_pbft_hash(h_arr);
  return {res.found, static_cast<PbftPeriod>(res.period)};
}

std::shared_ptr<std::pair<PbftPeriod, uint32_t>> DbStorage::getDagBlockPeriod(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto res = rust_storage_.value()->get_dag_block_period(h_arr);
  return std::make_shared<std::pair<PbftPeriod, uint32_t>>(res.period, res.position);
}

void DbStorage::addDagBlockPeriodToBatch(blk_hash_t const& hash, PbftPeriod period, uint32_t position,
                                         Batch& write_batch) {
  dev::RLPStream s;
  s.appendList(2);
  s << period;
  s << position;
  insert(write_batch, Columns::dag_block_period, toSlice(hash.asBytes()), toSlice(s.invalidate()));
}

std::vector<blk_hash_t> DbStorage::getFinalizedDagBlockHashesByPeriod(PbftPeriod period) {
  std::vector<blk_hash_t> ret;
  if (auto period_data = getPeriodDataRaw(period); period_data.size() > 0) {
    auto dag_blocks_data = dev::RLP(period_data)[DAG_BLOCKS_POS_IN_PERIOD_DATA];
    const auto dag_blocks = decodeDAGBlocksBundleRlp(dag_blocks_data);
    ret.reserve(dag_blocks.size());
    std::transform(dag_blocks.begin(), dag_blocks.end(), std::back_inserter(ret),
                   [](const auto& dag_block) { return dag_block->getHash(); });
  }

  return ret;
}

std::vector<std::shared_ptr<DagBlock>> DbStorage::getFinalizedDagBlockByPeriod(PbftPeriod period) {
  auto period_data = getPeriodDataRaw(period);
  if (period_data.empty()) {
    return {};
  }

  auto dag_blocks_data = dev::RLP(period_data)[DAG_BLOCKS_POS_IN_PERIOD_DATA];
  return decodeDAGBlocksBundleRlp(dag_blocks_data);
}

std::pair<blk_hash_t, std::vector<std::shared_ptr<DagBlock>>> DbStorage::getLastPbftBlockHashAndFinalizedDagBlockByPeriod(
    PbftPeriod period) {
  (void)period;
  throw_unimplemented_shim_api("getLastPbftBlockHashAndFinalizedDagBlockByPeriod");
}

std::optional<PbftPeriod> DbStorage::getProposalPeriodForDagLevel(uint64_t level) {
  auto res = rust_storage_.value()->get_proposal_period_for_dag_level(level);
  if (res.found) {
    return std::optional<PbftPeriod>(res.period);
  }
  return std::nullopt;
}

void DbStorage::saveProposalPeriodDagLevelsMap(uint64_t level, PbftPeriod period) {
  rust_storage_.value()->save_proposal_period_dag_levels_map(level, period);
}

void DbStorage::addProposalPeriodDagLevelsMapToBatch(uint64_t level, PbftPeriod period, Batch& write_batch) {
  insert(write_batch, Columns::proposal_period_levels_map, toSlice(level), toSlice(period));
}

void DbStorage::savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch) {
  insert(write_batch, Columns::period_lambda, period, period_lambda);
}

std::optional<uint32_t> DbStorage::getPeriodLambda(PbftPeriod period, bool find_closest) {
  auto rust_value = rust_storage_.value()->get_period_lambda(period, find_closest);
  if (rust_value.found) {
    return rust_value.value;
  }
  return {};
}

void DbStorage::saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch) {
  insert(write_batch, Columns::rounds_count_dynamic_lambda, 0, toSlice(rounds_count));
}

uint32_t DbStorage::getRoundsCountDynamicLambda() { return rust_storage_.value()->get_rounds_count_dynamic_lambda(); }

std::unordered_map<PbftPeriod, rewards::BlockStats> DbStorage::getBlocksRewardsStats() const {
  std::unordered_map<PbftPeriod, rewards::BlockStats> rewards_stats;

  auto rust_stats = rust_storage_.value()->get_blocks_rewards_stats();
  rewards_stats.reserve(rust_stats.size());
  for (auto const& stat : rust_stats) {
    auto bytes = dev::bytes(stat.data.begin(), stat.data.end());
    rewards_stats[stat.period] = util::rlp_dec<rewards::BlockStats>(dev::RLP(bytes));
  }
  return rewards_stats;
}

void DbStorage::saveBlockRewardsStats(uint64_t period, const rewards::BlockStats& stats, Batch& write_batch) {
  dev::RLPStream encoding;
  stats.rlp(encoding);
  insert(write_batch, Columns::block_rewards_stats, period, encoding.out());
}

bool DbStorage::hasMinorVersionChanged() { throw_unimplemented_shim_api("hasMinorVersionChanged"); }

bool DbStorage::hasMajorVersionChanged() { throw_unimplemented_shim_api("hasMajorVersionChanged"); }

void DbStorage::compactColumn(Column const& column) {
  (void)column;
  throw_unimplemented_shim_api("compactColumn");
}

void DbStorage::forEach(Column const& col, OnEntry const& f) {
  (void)col;
  (void)f;
  throw_unimplemented_shim_api("forEach");
}

}  // namespace taraxa
