#include "config/version.hpp"
#include "dag/dag_block_bundle_rlp.hpp"
#include "storage/sortition_params_change.hpp"
#include "storage/storage.hpp"
#include "transaction/system_transaction.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa {

#ifdef RUSTAXA_ENABLE
SortitionParamsChange::SortitionParamsChange(PbftPeriod period, uint16_t efficiency, const VrfParams& vrf)
    : period(period), vrf_params(vrf), interval_efficiency(efficiency) {}

bytes SortitionParamsChange::rlp() const {
  dev::RLPStream stream;
  stream.appendList(3);
  stream << vrf_params.threshold_upper;
  stream << period;
  stream << interval_efficiency;
  return stream.invalidate();
}

SortitionParamsChange SortitionParamsChange::from_rlp(const dev::RLP& rlp) {
  SortitionParamsChange change;
  change.vrf_params.threshold_upper = rlp[0].toInt<uint16_t>();
  change.period = rlp[1].toInt<PbftPeriod>();
  change.interval_efficiency = rlp[2].toInt<uint16_t>();
  return change;
}
#endif

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

rust::Vec<rustaxa::DagCounterUpdate> into_dag_counter_updates(std::vector<std::shared_ptr<DagBlock>> const& blks) {
  rust::Vec<rustaxa::DagCounterUpdate> updates;
  updates.reserve(blks.size());
  for (auto const& blk : blks) {
    updates.push_back(
        rustaxa::DagCounterUpdate{into_bytes_array(blk->getHash()), blk->getLevel(), blk->getTips().size()});
  }
  return updates;
}

template <typename T>
T save_as(const Slice& key) {
  T value{};
  std::memcpy(&value, key.data(), sizeof(T));
  return value;
}

[[noreturn]] void throw_invalid_final_chain_key_size(const char* column_name, size_t got, size_t expected) {
  throw DbException("Invalid key size for " + std::string(column_name) + " in Rust shim mode. Got " +
                    std::to_string(got) + ", expected " + std::to_string(expected));
}

[[noreturn]] void throw_admin_compat_unsupported(const char* api_name) {
  throw DbException("DbStorage::" + std::string(api_name) +
                    " is a RUSTAXA_ADMIN_COMPAT_UNSUPPORTED boundary in Rust shim mode");
}

[[noreturn]] void throw_query_compat_read(const char* api_name) {
  throw DbException("DbStorage::" + std::string(api_name) +
                    " is a RUSTAXA_QUERY_COMPAT_READ boundary without a generic iterator shim");
}
}  // namespace

DbStorage::DbStorage(SharedConsensusApplication consensus_application, fs::path const& path,
                     uint32_t db_snapshot_each_n_pbft_block, uint32_t max_open_files, uint32_t db_max_snapshots,
                     PbftPeriod db_revert_to_period, addr_t node_addr, bool rebuild)
    : DbStorageOld(path, db_snapshot_each_n_pbft_block, max_open_files, db_max_snapshots, db_revert_to_period,
                   node_addr, rebuild),
      consensus_application_(std::move(consensus_application)) {
  try {
    if (!consensus_application_) {
      throw std::invalid_argument("DbStorage requires the native consensus application root");
    }
    dag_queries_ = rustaxa::create_dag_storage_queries(consensus_application_->service());
    pbft_queries_ = rustaxa::create_pbft_storage_queries(consensus_application_->service());
    pbft_vote_queries_ = rustaxa::create_pbft_vote_storage_queries(consensus_application_->service());
    transaction_queries_ = rustaxa::create_transaction_storage_queries(consensus_application_->service());
    final_chain_queries_ = rustaxa::create_final_chain_storage_queries(consensus_application_->service());
    period_queries_ = rustaxa::create_period_storage_queries(consensus_application_->service());
    consensus_query_api_ = std::make_shared<rust::Box<rustaxa::BridgeConsensusQueryApi>>(
        rustaxa::create_consensus_query_api(consensus_application_->service()));
    kMajorVersion_ = static_cast<uint32_t>(getStatusField(StatusDbField::DbMajorVersion));
    auto const minor_version = static_cast<uint32_t>(getStatusField(StatusDbField::DbMinorVersion));
    if (kMajorVersion_ != 0 && kMajorVersion_ != TARAXA_DB_MAJOR_VERSION) {
      major_version_changed_ = true;
    } else if (minor_version != TARAXA_DB_MINOR_VERSION) {
      minor_version_changed_ = true;
    }
  } catch (std::exception const& e) {
    LOG(log_er_) << "Error: " << e.what() << std::endl;
    throw DbException(std::string("Rust storage init failed: ") + e.what());
  }
}

Batch DbStorage::createWriteBatch() { return Batch(); }
DbStorage::~DbStorage() = default;
std::shared_ptr<rust::Box<rustaxa::BridgeConsensusQueryApi>> DbStorage::consensusQueryApi() const {
  return consensus_query_api_;
}

rustaxa::BridgeStorageBatch& DbStorage::getOrCreateRustBatch(Batch& batch) {
  std::lock_guard<std::mutex> lock(rust_batches_mutex_);
  auto it = rust_batches_.find(&batch);
  if (it != rust_batches_.end()) {
    return *it->second;
  }

  auto rust_batch = rustaxa::create_storage_shim_batch(consensus_application_->service());
  auto [inserted_it, _] = rust_batches_.emplace(&batch, std::move(rust_batch));
  return *inserted_it->second;
}

void DbStorage::commitWriteBatch(Batch& write_batch, const rocksdb::WriteOptions& opts) {
  std::optional<::rust::Box<rustaxa::BridgeStorageBatch>> rust_batch;
  {
    std::lock_guard<std::mutex> lock(rust_batches_mutex_);
    auto it = rust_batches_.find(&write_batch);
    if (it != rust_batches_.end()) {
      rust_batch = std::move(it->second);
      rust_batches_.erase(it);
    }
  }

  if (rust_batch.has_value()) {
    rustaxa::storage_shim_commit_batch(std::move(*rust_batch), opts.sync);
  } else if (write_batch.Count() != 0) {
    throw DbException("commitWriteBatch called with unsupported non-rust batch content");
  }

  write_batch.Clear();
}

void DbStorage::commitWriteBatch(Batch& write_batch) { commitWriteBatch(write_batch, async_write_); }
void DbStorage::DeleteRange(const Column&, uint64_t, uint64_t) { throw_admin_compat_unsupported("DeleteRange"); }
void DbStorage::CompactRange(const Column&, uint64_t, uint64_t) { throw_admin_compat_unsupported("CompactRange"); }
bool DbStorage::createSnapshot(PbftPeriod) { throw_admin_compat_unsupported("createSnapshot"); }

void DbStorage::disableSnapshots() { snapshots_enabled_ = false; }
void DbStorage::enableSnapshots() { snapshots_enabled_ = true; }
void DbStorage::deleteColumnData(const Column& c) {
  if (c.ordinal_ == Columns::block_rewards_stats.ordinal_) {
    rustaxa::storage_shim_clear_block_rewards_stats(consensus_application_->service());
    return;
  }

  throw DbException("DbStorage::deleteColumnData(" + c.name() +
                    ") is a RUSTAXA_ADMIN_COMPAT_UNSUPPORTED boundary in Rust shim mode");
}
uint32_t DbStorage::getMajorVersion() const { return kMajorVersion_; }
std::unique_ptr<rocksdb::Iterator> DbStorage::getColumnIterator(const Column&) {
  throw_query_compat_read("getColumnIterator(Column)");
}

std::unique_ptr<rocksdb::Iterator> DbStorage::getColumnIterator(rocksdb::ColumnFamilyHandle*) {
  throw_query_compat_read("getColumnIterator(ColumnFamilyHandle*)");
}

std::string DbStorage::lookupFinalChainMeta(const Slice& key) const {
  if (key.size() != sizeof(uint32_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_meta.name().c_str(), key.size(), sizeof(uint32_t));
  }

  auto const meta_key = save_as<uint32_t>(key);
  auto rust_value = final_chain_queries_.value()->get_final_chain_meta_value(meta_key);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockByNumber(const Slice& key) const {
  if (key.size() != sizeof(uint64_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_by_number.name().c_str(), key.size(), sizeof(uint64_t));
  }

  auto const block_number = save_as<uint64_t>(key);
  auto rust_value = final_chain_queries_.value()->get_final_chain_block_header(block_number);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockHashByNumber(const Slice& key) const {
  if (key.size() != sizeof(uint64_t)) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_hash_by_number.name().c_str(), key.size(),
                                       sizeof(uint64_t));
  }

  auto const block_number = save_as<uint64_t>(key);
  auto rust_value = final_chain_queries_.value()->get_final_chain_block_hash_by_number(block_number);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainBlockNumberByHash(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_blk_number_by_hash.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> hash{};
  std::memcpy(hash.data(), key.data(), hash.size());
  auto rust_value = final_chain_queries_.value()->get_final_chain_block_number_by_hash(hash);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainLogBloomsChunk(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_log_blooms_index.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> chunk_id{};
  std::memcpy(chunk_id.data(), key.data(), chunk_id.size());
  auto rust_value = final_chain_queries_.value()->get_final_chain_log_blooms_chunk(chunk_id);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

std::string DbStorage::lookupFinalChainReceiptByTrxHash(const Slice& key) const {
  if (key.size() != 32) {
    throw_invalid_final_chain_key_size(Columns::final_chain_receipt_by_trx_hash.name().c_str(), key.size(), 32);
  }

  std::array<uint8_t, 32> trx_hash{};
  std::memcpy(trx_hash.data(), key.data(), trx_hash.size());
  auto rust_value = final_chain_queries_.value()->get_final_chain_receipt_by_trx_hash(trx_hash);
  return std::string(reinterpret_cast<const char*>(rust_value.data()), rust_value.size());
}

void DbStorage::updateDbVersions() {
  saveStatusField(StatusDbField::DbMajorVersion, TARAXA_DB_MAJOR_VERSION);
  saveStatusField(StatusDbField::DbMinorVersion, TARAXA_DB_MINOR_VERSION);
  kMajorVersion_ = TARAXA_DB_MAJOR_VERSION;
}

void DbStorage::setGenesisHash(const h256& genesis_hash) {
  auto bytes = into_bytes_array(genesis_hash);
  rustaxa::storage_shim_set_genesis_hash(consensus_application_->service(), bytes);
}

std::optional<h256> DbStorage::getGenesisHash() {
  auto rust_hash = consensus_application_->service().get_genesis_hash();
  if (!rust_hash.empty()) {
    return h256(dev::bytes(rust_hash.begin(), rust_hash.end()));
  }
  return {};
}

std::shared_ptr<DagBlock> DbStorage::getDagBlock(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto rlp_bytes = dag_queries_.value()->get_dag_block(h_arr);
  if (rlp_bytes.empty()) {
    return nullptr;
  }
  dev::RLP rlp(dev::bytesConstRef(rlp_bytes.data(), rlp_bytes.size()));
  return std::make_shared<DagBlock>(rlp);
}

bool DbStorage::dagBlockInDb(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  return dag_queries_.value()->dag_block_in_db(h_arr);
}

std::set<blk_hash_t> DbStorage::getBlocksByLevel(level_t level) {
  auto bytes = dag_queries_.value()->get_blocks_by_level(level);
  std::set<blk_hash_t> res;
  for (size_t i = 0; i < bytes.size(); i += 32) {
    blk_hash_t h;
    std::memcpy(h.data(), bytes.data() + i, 32);
    res.insert(h);
  }
  return res;
}

level_t DbStorage::getLastBlocksLevel() const { return dag_queries_.value()->get_last_blocks_level(); }
std::vector<std::shared_ptr<DagBlock>> DbStorage::getDagBlocksAtLevel(level_t level, int number_of_levels) {
  std::vector<std::shared_ptr<DagBlock>> res;
  auto blocks_rlp = dag_queries_.value()->get_dag_blocks_at_level(level, (uint32_t)number_of_levels);
  for (auto const& item : blocks_rlp) {
    dev::RLP rlp(dev::bytesConstRef(item.data.data(), item.data.size()));
    res.push_back(std::make_shared<DagBlock>(rlp));
  }
  return res;
}

std::map<level_t, std::vector<std::shared_ptr<DagBlock>>> DbStorage::getNonfinalizedDagBlocks() {
  std::map<level_t, std::vector<std::shared_ptr<DagBlock>>> res;
  auto levels = dag_queries_.value()->get_nonfinalized_dag_blocks();
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
  auto trxs = transaction_queries_.value()->get_all_nonfinalized_transactions();
  res.reserve(trxs.size());
  for (auto const& trx_rlp : trxs) {
    res.emplace_back(std::make_shared<Transaction>(dev::bytes(trx_rlp.data.begin(), trx_rlp.data.end())));
  }
  return res;
}

void DbStorage::removeDagBlock(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  commitImmediateRustBatch(
      [&](rustaxa::BridgeStorageBatch& batch) { rustaxa::storage_shim_remove_dag_block(batch, h_arr); });
}

void DbStorage::updateDagBlockCounters(std::vector<std::shared_ptr<DagBlock>> blks) {
  std::lock_guard<std::mutex> u_lock(dag_blocks_mutex_);
  auto write_batch = createWriteBatch();
  for (auto const& blk : blks) {
    dag_blocks_count_.fetch_add(1);
    dag_edge_count_.fetch_add(blk->getTips().size() + 1);
  }
  rustaxa::storage_shim_update_dag_block_counters(getOrCreateRustBatch(write_batch), into_dag_counter_updates(blks),
                                                  dag_blocks_count_.load(), dag_edge_count_.load());
  commitWriteBatch(write_batch);
}

void DbStorage::mirrorDagBlockCounters(uint64_t dag_blocks_count, uint64_t dag_edge_count) {
  std::lock_guard<std::mutex> u_lock(dag_blocks_mutex_);
  dag_blocks_count_.store(dag_blocks_count);
  dag_edge_count_.store(dag_edge_count);
}

uint64_t DbStorage::getDagBlocksCount() const { return dag_blocks_count_.load(); }
uint64_t DbStorage::getDagEdgeCount() const { return dag_edge_count_.load(); }

void DbStorage::saveDagBlock(const std::shared_ptr<DagBlock>& blk, Batch* write_batch_p) {
  // Keep parity with legacy semantics: when called with caller-provided batch,
  // stage all writes there and update counters; otherwise delegate to Rust
  // atomic write path.
  if (!write_batch_p) {
    std::lock_guard<std::mutex> u_lock(dag_blocks_mutex_);
    auto block_hash = blk->getHash();
    auto h_arr = into_bytes_array(block_hash);
    auto block_bytes = blk->rlp(true);
    auto block_rlp = into_rust_vec(block_bytes);
    auto const dag_blocks_count = dag_blocks_count_.load() + 1;
    auto const dag_edge_count = dag_edge_count_.load() + blk->getTips().size() + 1;
    commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
      rustaxa::storage_shim_save_dag_block(batch, h_arr, blk->getLevel(), std::move(block_rlp), dag_blocks_count,
                                           dag_edge_count);
    });
    dag_blocks_count_.store(dag_blocks_count);
    dag_edge_count_.store(dag_edge_count);
    return;
  }

  std::lock_guard<std::mutex> u_lock(dag_blocks_mutex_);
  auto& write_batch = *write_batch_p;
  auto block_bytes = blk->rlp(true);
  auto block_hash = blk->getHash();

  dag_blocks_count_.fetch_add(1);
  dag_edge_count_.fetch_add(blk->getTips().size() + 1);
  auto h_arr = into_bytes_array(block_hash);
  rustaxa::storage_shim_save_dag_block(getOrCreateRustBatch(write_batch), h_arr, blk->getLevel(),
                                       into_rust_vec(block_bytes), dag_blocks_count_.load(), dag_edge_count_.load());
}

void DbStorage::saveSortitionParamsChange(PbftPeriod period, const SortitionParamsChange& params, Batch& batch) {
  rustaxa::storage_shim_save_sortition_params_change(getOrCreateRustBatch(batch), period, into_rust_vec(params.rlp()));
}

std::deque<SortitionParamsChange> DbStorage::getLastSortitionParams(size_t count) {
  std::deque<SortitionParamsChange> changes;

  auto rust_changes = consensus_application_->service().get_last_sortition_params(static_cast<uint64_t>(count));
  for (auto const& change_rlp : rust_changes) {
    auto bytes = dev::bytes(change_rlp.data.begin(), change_rlp.data.end());
    changes.emplace_back(SortitionParamsChange::from_rlp(dev::RLP(bytes)));
  }
  return changes;
}

std::optional<SortitionParamsChange> DbStorage::getParamsChangeForPeriod(PbftPeriod period) {
  auto rust_change = consensus_application_->service().get_params_change_for_period(period);
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
    auto h_arr = into_bytes_array(block->getHash());
    rustaxa::storage_shim_remove_dag_block(getOrCreateRustBatch(write_batch), h_arr);
    addDagBlockPeriodToBatch(block->getHash(), period, block_pos, write_batch);
    block_pos++;
  }

  uint32_t trx_pos = 0;
  for (auto const& trx : period_data.transactions) {
    removeTransactionToBatch(trx->getHash(), write_batch);
    addTransactionLocationToBatch(write_batch, trx->getHash(), period, trx_pos);
    trx_pos++;
  }

  auto period_data_rlp = into_rust_vec(period_data.rlp());
  rustaxa::storage_shim_save_period_data(getOrCreateRustBatch(write_batch), period, std::move(period_data_rlp));
}

dev::bytes DbStorage::getPeriodDataRaw(PbftPeriod period) const {
  auto period_data = period_queries_.value()->get_period_data_raw(period);
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
  SharedTransactions ret;
  ret.reserve(period_data_rlp[TRANSACTIONS_POS_IN_PERIOD_DATA].size());
  for (auto&& transaction_data : period_data_rlp[TRANSACTIONS_POS_IN_PERIOD_DATA]) {
    ret.emplace_back(std::make_shared<Transaction>(std::move(transaction_data)));
  }
  auto period_system_transaction_hashes = getPeriodSystemTransactionsHashes(period);
  ret.reserve(ret.size() + period_system_transaction_hashes.size());
  for (const auto& trx_hash : period_system_transaction_hashes) {
    ret.emplace_back(getSystemTransaction(trx_hash));
  }
  return ret;
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
  consensus_application_->service().pillar_chain_storage_apply_finalized_block(pillar_block->getPeriod(),
                                                                                std::move(pillar_rlp));
}

std::shared_ptr<pillar_chain::PillarBlock> DbStorage::getPillarBlock(PbftPeriod period) const {
  auto data = consensus_application_->service().pillar_chain_storage_load_block(period);
  if (data.empty()) {
    return {};
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<pillar_chain::PillarBlock>(dev::RLP(rust_bytes));
}

std::shared_ptr<pillar_chain::PillarBlock> DbStorage::getLatestPillarBlock() const {
  auto data = consensus_application_->service().pillar_chain_storage_load_latest_block();
  if (data.empty()) {
    return {};
  }

  auto bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<pillar_chain::PillarBlock>(dev::RLP(bytes));
}

void DbStorage::saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>& vote) {
  auto vote_bytes = util::rlp_enc(vote);
  auto vote_rlp = into_rust_vec(vote_bytes);
  consensus_application_->service().pillar_chain_storage_apply_own_vote(std::move(vote_rlp));
}

std::shared_ptr<PillarVote> DbStorage::getOwnPillarBlockVote() const {
  auto data = consensus_application_->service().pillar_chain_storage_load_own_vote();
  if (data.empty()) {
    return nullptr;
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return std::make_shared<PillarVote>(dev::RLP(rust_bytes));
}

void DbStorage::saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb& current_pillar_block_data) {
  auto data_bytes = util::rlp_enc(current_pillar_block_data);
  auto data_rlp = into_rust_vec(data_bytes);
  consensus_application_->service().pillar_chain_storage_apply_current_block_data(std::move(data_rlp));
}

std::optional<pillar_chain::CurrentPillarBlockDataDb> DbStorage::getCurrentPillarBlockData() const {
  auto data = consensus_application_->service().pillar_chain_storage_load_current_block_data();
  if (data.empty()) {
    return {};
  }

  auto rust_bytes = dev::bytes(data.begin(), data.end());
  return util::rlp_dec<pillar_chain::CurrentPillarBlockDataDb>(dev::RLP(rust_bytes));
}

void DbStorage::addTransactionLocationToBatch(Batch& write_batch, trx_hash_t const& trx_hash, PbftPeriod period,
                                              uint32_t position, bool is_system) {
  auto h_arr = into_bytes_array(trx_hash);
  rustaxa::storage_shim_save_transaction_location(getOrCreateRustBatch(write_batch), h_arr, period, position,
                                                  is_system);
}

std::optional<TransactionLocation> DbStorage::getTransactionLocation(trx_hash_t const& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto location_bytes = transaction_queries_.value()->get_transaction_location(h_arr);
  if (!location_bytes.empty()) {
    auto location_data = dev::bytes(location_bytes.begin(), location_bytes.end());
    // Don't use std::move - RLP stores a reference and needs data to stay alive
    return TransactionLocation::fromRlp(dev::RLP(location_data));
  }
  return std::nullopt;
}

std::vector<bool> DbStorage::transactionsFinalized(std::vector<trx_hash_t> const& trx_hashes) {
  std::vector<bool> result(trx_hashes.size(), false);
  for (size_t i = 0; i < trx_hashes.size(); ++i) {
    auto h_arr = into_bytes_array(trx_hashes[i]);
    result[i] = transaction_queries_.value()->transaction_finalized(h_arr);
  }
  return result;
}

std::unordered_map<trx_hash_t, PbftPeriod> DbStorage::getAllTransactionPeriod() {
  std::unordered_map<trx_hash_t, PbftPeriod> res;
  auto data = transaction_queries_.value()->get_all_transaction_period();
  res.reserve(data.size());
  for (auto const& item : data) {
    auto hash_bytes = dev::bytes(item.hash.begin(), item.hash.end());
    res[trx_hash_t(hash_bytes)] = item.period;
  }
  return res;
}

void DbStorage::saveProposedPbftBlock(const std::shared_ptr<PbftBlock>& block) {
  auto block_hash = block->getBlockHash();
  auto block_bytes = block->rlp(true);
  auto block_rlp = into_rust_vec(block_bytes);
  (void)pbft_queries_.value()->save_proposed_pbft_block(block->getPeriod(), into_bytes_array(block_hash),
                                                        into_bytes_array(block->getPivotDagBlockHash()),
                                                        std::move(block_rlp));
}

void DbStorage::removeProposedPbftBlock(const blk_hash_t& block_hash, Batch& write_batch) {
  auto h_arr = into_bytes_array(block_hash);
  rustaxa::storage_shim_remove_proposed_pbft_block(getOrCreateRustBatch(write_batch), h_arr);
}

std::vector<std::shared_ptr<PbftBlock>> DbStorage::getProposedPbftBlocks() {
  std::vector<std::shared_ptr<PbftBlock>> res;
  auto blocks = pbft_queries_.value()->get_proposed_pbft_blocks();
  res.reserve(blocks.size());
  for (auto const& block : blocks) {
    res.emplace_back(std::make_shared<PbftBlock>(dev::bytes(block.data.begin(), block.data.end())));
  }
  return res;
}

std::shared_ptr<Transaction> DbStorage::getTransaction(trx_hash_t const& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto rust_data = transaction_queries_.value()->get_transaction(h_arr);
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
  auto data = transaction_queries_.value()->get_transaction_by_period_position(period, position);
  if (!data.empty()) {
    return std::make_shared<Transaction>(dev::bytes(data.begin(), data.end()));
  }
  return nullptr;
}

uint64_t DbStorage::getTransactionCount(PbftPeriod period) const {
  return transaction_queries_.value()->get_transaction_count(period);
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
  auto h_arr = into_bytes_array(trx->getHash());
  auto trx_rlp = into_rust_vec(trx->rlp());
  rustaxa::storage_shim_save_system_transaction(getOrCreateRustBatch(write_batch), h_arr, std::move(trx_rlp));
}

std::shared_ptr<Transaction> DbStorage::getSystemTransaction(const trx_hash_t& hash) const {
  auto h_arr = into_bytes_array(hash);
  auto rust_data = transaction_queries_.value()->get_system_transaction(h_arr);
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
  auto hashes_rlp_vec = into_rust_vec(hashes_rlp);
  rustaxa::storage_shim_save_period_system_transactions_hashes(getOrCreateRustBatch(write_batch), period,
                                                               std::move(hashes_rlp_vec));
}

std::vector<trx_hash_t> DbStorage::getPeriodSystemTransactionsHashes(PbftPeriod period) const {
  auto rust_data = transaction_queries_.value()->get_period_system_transactions_hashes(period);
  if (rust_data.empty()) {
    return {};
  }
  auto hashes_data = dev::bytes(rust_data.begin(), rust_data.end());
  return util::rlp_dec<std::vector<trx_hash_t>>(dev::RLP(hashes_data));
}

SharedTransactionReceipts DbStorage::getBlockReceipts(PbftPeriod period) const {
  auto rust_value = period_queries_.value()->get_block_receipt(period);
  if (rust_value.empty()) {
    return {};
  }
  auto data_bytes = dev::bytes(rust_value.begin(), rust_value.end());
  return std::make_shared<std::vector<TransactionReceipt>>(
      util::rlp_dec<std::vector<TransactionReceipt>>(dev::RLP(data_bytes)));
}

void DbStorage::addTransactionToBatch(Transaction const& trx, Batch& write_batch) {
  auto h_arr = into_bytes_array(trx.getHash());
  auto trx_rlp = into_rust_vec(trx.rlp());
  rustaxa::storage_shim_save_transaction(getOrCreateRustBatch(write_batch), h_arr, std::move(trx_rlp));
}

void DbStorage::removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch) {
  auto h_arr = into_bytes_array(trx);
  rustaxa::storage_shim_remove_transaction(getOrCreateRustBatch(write_batch), h_arr);
}

bool DbStorage::transactionInDb(trx_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  return transaction_queries_.value()->transaction_in_db(h_arr);
}

bool DbStorage::transactionFinalized(trx_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  return transaction_queries_.value()->transaction_finalized(h_arr);
}

uint64_t DbStorage::getStatusField(StatusDbField const& field) {
  return consensus_application_->service().get_status_field(static_cast<uint8_t>(field));
}

uint64_t DbStorage::getNumTransactionExecuted() { return getStatusField(StatusDbField::ExecutedTrxCount); }

uint64_t DbStorage::getNumTransactionInDag() { return getStatusField(StatusDbField::TrxCount); }

uint64_t DbStorage::getNumBlockExecuted() { return getStatusField(StatusDbField::ExecutedBlkCount); }

void DbStorage::saveStatusField(StatusDbField const& field, uint64_t value) {
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) {
    rustaxa::storage_shim_save_status_field(batch, static_cast<uint8_t>(field), value);
  });
}

void DbStorage::addStatusFieldToBatch(StatusDbField const& field, uint64_t value, Batch& write_batch) {
  rustaxa::storage_shim_save_status_field(getOrCreateRustBatch(write_batch), static_cast<uint8_t>(field), value);
}

uint32_t DbStorage::getPbftMgrField(PbftMgrField field) {
  return pbft_queries_.value()->get_pbft_mgr_field(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrField(PbftMgrField field, uint32_t value) {
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) {
    rustaxa::storage_shim_save_pbft_mgr_field(batch, static_cast<uint8_t>(field), value);
  });
}

void DbStorage::addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch) {
  rustaxa::storage_shim_save_pbft_mgr_field(getOrCreateRustBatch(write_batch), static_cast<uint8_t>(field), value);
}

bool DbStorage::getPbftMgrStatus(PbftMgrStatus field) {
  return pbft_queries_.value()->get_pbft_mgr_status(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrStatus(PbftMgrStatus field, bool const& value) {
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) {
    rustaxa::storage_shim_save_pbft_mgr_status(batch, static_cast<uint8_t>(field), value);
  });
}

void DbStorage::addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch) {
  rustaxa::storage_shim_save_pbft_mgr_status(getOrCreateRustBatch(write_batch), static_cast<uint8_t>(field), value);
}

void DbStorage::saveCertVotedBlockInRound(PbftRound round, const std::shared_ptr<PbftBlock>& block) {
  assert(block);
  auto block_bytes = block->rlp(true);
  auto block_rlp = into_rust_vec(block_bytes);
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
    rustaxa::storage_shim_save_cert_voted_block_in_round(batch, round, std::move(block_rlp));
  });
}

std::optional<std::pair<PbftRound, std::shared_ptr<PbftBlock>>> DbStorage::getCertVotedBlockInRound() const {
  auto rust_value = pbft_queries_.value()->get_cert_voted_block_in_round();
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
  rustaxa::storage_shim_remove_cert_voted_block_in_round(getOrCreateRustBatch(write_batch));
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
  auto res = pbft_queries_.value()->pbft_block_in_db(h_arr);
  return res;
}

std::string DbStorage::getPbftHead(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto data = pbft_queries_.value()->get_pbft_head(h_arr);
  return std::string(data.begin(), data.end());
}

void DbStorage::savePbftHead(blk_hash_t const& hash, std::string const& pbft_chain_head_str) {
  auto h_arr = into_bytes_array(hash);
  auto head_bytes = into_rust_vec(pbft_chain_head_str);
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
    rustaxa::storage_shim_save_pbft_head(batch, h_arr, std::move(head_bytes));
  });
}

void DbStorage::addPbftHeadToBatch(taraxa::blk_hash_t const& head_hash, std::string const& head_str,
                                   Batch& write_batch) {
  auto h_arr = into_bytes_array(head_hash);
  auto head_bytes = into_rust_vec(head_str);
  rustaxa::storage_shim_save_pbft_head(getOrCreateRustBatch(write_batch), h_arr, std::move(head_bytes));
}

void DbStorage::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  auto vote_hash = vote->getHash();
  auto h_arr = into_bytes_array(vote_hash);

  auto vote_bytes = vote->rlp(true, true);
  auto vote_rlp = into_rust_vec(vote_bytes);
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
    rustaxa::storage_shim_save_own_verified_vote(batch, h_arr, std::move(vote_rlp));
  });
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getOwnVerifiedVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = pbft_vote_queries_.value()->get_own_verified_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::clearOwnVerifiedVotes(Batch& write_batch,
                                      const std::vector<std::shared_ptr<PbftVote>>& own_verified_votes) {
  for (const auto& own_vote : own_verified_votes) {
    auto h_arr = into_bytes_array(own_vote->getHash());
    rustaxa::storage_shim_remove_own_verified_vote(getOrCreateRustBatch(write_batch), h_arr);
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
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
    rustaxa::storage_shim_replace_two_t_plus_one_votes(batch, static_cast<uint8_t>(type), std::move(votes_bundle_rlp));
  });
}

void DbStorage::replaceTwoTPlusOneVotesToBatch(TwoTPlusOneVotedBlockType type,
                                               const std::vector<std::shared_ptr<PbftVote>>& votes,
                                               Batch& write_batch) {
  dev::RLPStream rust_votes_stream(votes.size());
  for (const auto& vote : votes) {
    rust_votes_stream.appendRaw(vote->rlp(true, true));
  }
  auto votes_bundle = rust_votes_stream.out();
  auto votes_bundle_rlp = into_rust_vec(votes_bundle);
  rustaxa::storage_shim_replace_two_t_plus_one_votes(getOrCreateRustBatch(write_batch), static_cast<uint8_t>(type),
                                                     std::move(votes_bundle_rlp));
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getAllTwoTPlusOneVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = pbft_vote_queries_.value()->get_all_two_t_plus_one_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::removeExtraRewardVotes(const std::vector<vote_hash_t>& votes, Batch& write_batch) {
  for (const auto& v : votes) {
    auto h_arr = into_bytes_array(v);
    rustaxa::storage_shim_remove_extra_reward_vote(getOrCreateRustBatch(write_batch), h_arr);
  }
}

void DbStorage::saveExtraRewardVote(const std::shared_ptr<PbftVote>& vote) {
  auto vote_hash = vote->getHash();
  auto h_arr = into_bytes_array(vote_hash);
  auto vote_bytes = vote->rlp(true, true);
  auto vote_rlp = into_rust_vec(vote_bytes);
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) mutable {
    rustaxa::storage_shim_save_extra_reward_vote(batch, h_arr, std::move(vote_rlp));
  });
}

std::vector<std::shared_ptr<PbftVote>> DbStorage::getRewardVotes() {
  std::vector<std::shared_ptr<PbftVote>> votes;
  auto rust_votes = pbft_vote_queries_.value()->get_reward_votes();
  votes.reserve(rust_votes.size());
  for (auto const& vote_rlp : rust_votes) {
    votes.emplace_back(std::make_shared<PbftVote>(dev::bytes(vote_rlp.data.begin(), vote_rlp.data.end())));
  }

  return votes;
}

void DbStorage::addPbftBlockPeriodToBatch(PbftPeriod period, taraxa::blk_hash_t const& pbft_block_hash,
                                          Batch& write_batch) {
  auto h_arr = into_bytes_array(pbft_block_hash);
  rustaxa::storage_shim_save_pbft_block_period(getOrCreateRustBatch(write_batch), h_arr, period);
}

std::pair<bool, PbftPeriod> DbStorage::getPeriodFromPbftHash(taraxa::blk_hash_t const& pbft_block_hash) {
  auto h_arr = into_bytes_array(pbft_block_hash);
  auto res = period_queries_.value()->get_period_from_pbft_hash(h_arr);
  return {res.found, static_cast<PbftPeriod>(res.period)};
}

std::shared_ptr<std::pair<PbftPeriod, uint32_t>> DbStorage::getDagBlockPeriod(blk_hash_t const& hash) {
  auto h_arr = into_bytes_array(hash);
  auto res = dag_queries_.value()->get_dag_block_period_lookup(h_arr);
  if (!res.found) {
    return nullptr;
  }
  return std::make_shared<std::pair<PbftPeriod, uint32_t>>(res.period, res.position);
}

void DbStorage::addDagBlockPeriodToBatch(blk_hash_t const& hash, PbftPeriod period, uint32_t position,
                                         Batch& write_batch) {
  auto h_arr = into_bytes_array(hash);
  rustaxa::storage_shim_save_dag_block_period(getOrCreateRustBatch(write_batch), h_arr, period, position);
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

std::optional<PbftPeriod> DbStorage::getProposalPeriodForDagLevel(uint64_t level) {
  auto res = dag_queries_.value()->get_proposal_period_for_dag_level(level);
  if (res.found) {
    return std::optional<PbftPeriod>(res.period);
  }
  return std::nullopt;
}

void DbStorage::saveProposalPeriodDagLevelsMap(uint64_t level, PbftPeriod period) {
  commitImmediateRustBatch([&](rustaxa::BridgeStorageBatch& batch) {
    rustaxa::storage_shim_save_proposal_period_dag_level(batch, level, period);
  });
}

void DbStorage::addProposalPeriodDagLevelsMapToBatch(uint64_t level, PbftPeriod period, Batch& write_batch) {
  rustaxa::storage_shim_save_proposal_period_dag_level(getOrCreateRustBatch(write_batch), level, period);
}

void DbStorage::savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch) {
  rustaxa::storage_shim_save_period_lambda(getOrCreateRustBatch(write_batch), period, period_lambda);
}

std::optional<uint32_t> DbStorage::getPeriodLambda(PbftPeriod period, bool find_closest) {
  auto rust_value = consensus_application_->service().get_period_lambda(period, find_closest);
  if (rust_value.found) {
    return rust_value.value;
  }
  return {};
}

void DbStorage::saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch) {
  rustaxa::storage_shim_save_rounds_count_dynamic_lambda(getOrCreateRustBatch(write_batch), rounds_count);
}

uint32_t DbStorage::getRoundsCountDynamicLambda() {
  return consensus_application_->service().get_rounds_count_dynamic_lambda();
}

std::unordered_map<PbftPeriod, rewards::BlockStats> DbStorage::getBlocksRewardsStats() const {
  std::unordered_map<PbftPeriod, rewards::BlockStats> rewards_stats;

  auto rust_stats = consensus_application_->service().get_blocks_rewards_stats();
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
  rustaxa::storage_shim_save_block_rewards_stats(getOrCreateRustBatch(write_batch), period,
                                                 into_rust_vec(encoding.out()));
}

bool DbStorage::hasMajorVersionChanged() { return major_version_changed_; }

void DbStorage::compactColumn(Column const& column) {
  (void)column;
  throw_admin_compat_unsupported("compactColumn");
}

}  // namespace taraxa
