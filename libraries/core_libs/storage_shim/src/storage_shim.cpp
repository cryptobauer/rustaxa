#include "dag/sortition_params_manager.hpp"
#include "storage/storage.hpp"
#include "transaction/system_transaction.hpp"

namespace taraxa {
namespace {
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
}  // namespace

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
  (void)batch;
  auto params_rlp_bytes = params.rlp();
  auto params_rlp = into_rust_vec(params_rlp_bytes);
  rust_storage_.value()->save_sortition_params_change(period, std::move(params_rlp));
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
  (void)write_batch;
  const auto rust_period = period_data.pbft_blk->getPeriod();

  auto pbft_block_hash = period_data.pbft_blk->getBlockHash();
  auto pbft_hash_arr = into_bytes_array(pbft_block_hash);
  rust_storage_.value()->save_pbft_block_period(pbft_hash_arr, rust_period);

  uint32_t rust_block_pos = 0;
  for (auto const& block : period_data.dag_blocks) {
    auto block_hash = block->getHash();
    auto block_hash_arr = into_bytes_array(block_hash);
    rust_storage_.value()->remove_dag_block(block_hash_arr);
    rust_storage_.value()->save_dag_block_period(block_hash_arr, rust_period, rust_block_pos);
    rust_block_pos++;
  }

  uint32_t rust_trx_pos = 0;
  for (auto const& trx : period_data.transactions) {
    auto trx_hash = trx->getHash();
    auto trx_hash_arr = into_bytes_array(trx_hash);
    rust_storage_.value()->remove_transaction(trx_hash_arr);
    rust_storage_.value()->save_transaction_location(trx_hash_arr, rust_period, rust_trx_pos, false);
    rust_trx_pos++;
  }

  auto period_data_bytes = period_data.rlp();
  auto period_data_rlp = into_rust_vec(period_data_bytes);
  rust_storage_.value()->save_period_data(rust_period, std::move(period_data_rlp));
}

dev::bytes DbStorage::getPeriodDataRaw(PbftPeriod period) const {
  auto period_data = rust_storage_.value()->get_period_data_raw(period);
  return dev::bytes(period_data.begin(), period_data.end());
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
  (void)write_batch;
  auto h_arr = into_bytes_array(trx_hash);
  rust_storage_.value()->save_transaction_location(h_arr, period, position, is_system);
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
  (void)write_batch;
  auto h_arr = into_bytes_array(block_hash);
  rust_storage_.value()->remove_proposed_pbft_block(h_arr);
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

void DbStorage::addSystemTransactionToBatch(Batch& write_batch, SharedTransaction trx) {
  (void)write_batch;
  auto trx_hash = trx->getHash();
  auto h_arr = into_bytes_array(trx_hash);
  auto trx_bytes = trx->rlp();
  auto trx_rlp = into_rust_vec(trx_bytes);
  rust_storage_.value()->save_system_transaction(h_arr, std::move(trx_rlp));
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
  (void)write_batch;
  std::vector<trx_hash_t> trx_hashes;
  trx_hashes.reserve(trxs.size());
  std::transform(trxs.begin(), trxs.end(), std::back_inserter(trx_hashes),
                 [](const auto& trx) { return trx->getHash(); });
  auto hashes_rlp = util::rlp_enc(trx_hashes);
  auto hashes = into_rust_vec(hashes_rlp);
  rust_storage_.value()->save_period_system_transactions_hashes(period, std::move(hashes));
}

std::vector<trx_hash_t> DbStorage::getPeriodSystemTransactionsHashes(PbftPeriod period) const {
  auto rust_data = rust_storage_.value()->get_period_system_transactions_hashes(period);
  if (rust_data.empty()) {
    return {};
  }
  auto hashes_data = dev::bytes(rust_data.begin(), rust_data.end());
  return util::rlp_dec<std::vector<trx_hash_t>>(dev::RLP(hashes_data));
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
  (void)write_batch;
  auto trx_hash = trx.getHash();
  auto h_arr = into_bytes_array(trx_hash);
  auto trx_bytes = trx.rlp();
  auto trx_rlp = into_rust_vec(trx_bytes);
  rust_storage_.value()->save_transaction(h_arr, std::move(trx_rlp));
}

void DbStorage::removeTransactionToBatch(trx_hash_t const& trx, Batch& write_batch) {
  (void)write_batch;
  auto h_arr = into_bytes_array(trx);
  rust_storage_.value()->remove_transaction(h_arr);
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
  (void)write_batch;
  rust_storage_.value()->save_status_field(static_cast<uint8_t>(field), value);
}

uint32_t DbStorage::getPbftMgrField(PbftMgrField field) {
  return rust_storage_.value()->get_pbft_mgr_field(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrField(PbftMgrField field, uint32_t value) {
  rust_storage_.value()->save_pbft_mgr_field(static_cast<uint8_t>(field), value);
}

void DbStorage::addPbftMgrFieldToBatch(PbftMgrField field, uint32_t value, Batch& write_batch) {
  (void)write_batch;
  rust_storage_.value()->save_pbft_mgr_field(static_cast<uint8_t>(field), value);
}

bool DbStorage::getPbftMgrStatus(PbftMgrStatus field) {
  return rust_storage_.value()->get_pbft_mgr_status(static_cast<uint8_t>(field));
}

void DbStorage::savePbftMgrStatus(PbftMgrStatus field, bool const& value) {
  rust_storage_.value()->save_pbft_mgr_status(static_cast<uint8_t>(field), value);
}

void DbStorage::addPbftMgrStatusToBatch(PbftMgrStatus field, bool const& value, Batch& write_batch) {
  (void)write_batch;
  rust_storage_.value()->save_pbft_mgr_status(static_cast<uint8_t>(field), value);
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
  (void)write_batch;
  rust_storage_.value()->remove_cert_voted_block_in_round();
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
  (void)write_batch;
  auto h_arr = into_bytes_array(head_hash);
  auto head_bytes = into_rust_vec(head_str);
  rust_storage_.value()->save_pbft_head(h_arr, std::move(head_bytes));
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
  (void)write_batch;
  for (const auto& own_vote : own_verified_votes) {
    auto vote_hash = own_vote->getHash();
    auto h_arr = into_bytes_array(vote_hash);
    rust_storage_.value()->remove_own_verified_vote(h_arr);
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
  (void)write_batch;
  dev::RLPStream rust_votes_stream(votes.size());
  for (const auto& vote : votes) {
    rust_votes_stream.appendRaw(vote->rlp(true, true));
  }
  auto votes_bundle = rust_votes_stream.out();
  auto votes_bundle_rlp = into_rust_vec(votes_bundle);
  rust_storage_.value()->replace_two_t_plus_one_votes(static_cast<uint8_t>(type), std::move(votes_bundle_rlp));
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
  (void)write_batch;
  for (const auto& v : votes) {
    auto h_arr = into_bytes_array(v);
    rust_storage_.value()->remove_extra_reward_vote(h_arr);
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
  (void)write_batch;
  auto h_arr = into_bytes_array(pbft_block_hash);
  rust_storage_.value()->save_pbft_block_period(h_arr, period);
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
  (void)write_batch;
  auto h_arr = into_bytes_array(hash);
  rust_storage_.value()->save_dag_block_period(h_arr, period, position);
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
  (void)write_batch;
  rust_storage_.value()->save_proposal_period_dag_levels_map(level, period);
}

void DbStorage::savePeriodLambda(PbftPeriod period, uint32_t period_lambda, Batch& write_batch) {
  (void)write_batch;
  rust_storage_.value()->save_period_lambda(period, period_lambda);
}

std::optional<uint32_t> DbStorage::getPeriodLambda(PbftPeriod period, bool find_closest) {
  auto rust_value = rust_storage_.value()->get_period_lambda(period, find_closest);
  if (rust_value.found) {
    return rust_value.value;
  }
  return {};
}

void DbStorage::saveRoundsCountDynamicLambda(uint32_t rounds_count, Batch& write_batch) {
  (void)write_batch;
  rust_storage_.value()->save_rounds_count_dynamic_lambda(rounds_count);
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
  (void)write_batch;
  dev::RLPStream rust_encoding;
  stats.rlp(rust_encoding);
  auto stats_bytes = rust_encoding.out();
  auto stats_rlp = into_rust_vec(stats_bytes);
  rust_storage_.value()->save_block_rewards_stats(period, std::move(stats_rlp));
}

}  // namespace taraxa
