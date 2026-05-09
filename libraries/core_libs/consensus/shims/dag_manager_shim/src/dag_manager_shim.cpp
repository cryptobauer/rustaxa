#include "dag/dag_manager.hpp"

#include <array>
#include <fstream>
#include <iostream>
#include <map>
#include <mutex>
#include <optional>
#include <shared_mutex>
#include <stdexcept>
#include <utility>
#include <vector>

#include "dag/dag_block.hpp"
#include "network/network.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t &hash) { return hash.asArray(); }

rustaxa::DagHash to_bridge_dag_hash(const blk_hash_t &hash) { return rustaxa::DagHash{to_bridge_hash(hash)}; }

rust::Vec<rustaxa::DagHash> to_bridge_dag_hashes(const std::vector<blk_hash_t> &hashes) {
  rust::Vec<rustaxa::DagHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(to_bridge_dag_hash(hash));
  }
  return out;
}

blk_hash_t from_bridge_hash(const std::array<uint8_t, 32> &hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

blk_hash_t from_bridge_dag_hash(const rustaxa::DagHash &hash) { return from_bridge_hash(hash.hash); }

std::vector<blk_hash_t> from_bridge_dag_hashes(const rust::Vec<rustaxa::DagHash> &hashes) {
  std::vector<blk_hash_t> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.emplace_back(from_bridge_dag_hash(hash));
  }
  return out;
}

std::map<uint64_t, std::unordered_set<blk_hash_t>> from_bridge_level_hashes(
    const rust::Vec<rustaxa::DagLevelHashes> &levels) {
  std::map<uint64_t, std::unordered_set<blk_hash_t>> out;
  for (const auto &level_hashes : levels) {
    auto &hashes = out[level_hashes.level];
    for (const auto &hash : level_hashes.hashes) {
      hashes.insert(from_bridge_dag_hash(hash));
    }
  }
  return out;
}

rustaxa::DagManagerBlock to_bridge_manager_block(const std::shared_ptr<DagBlock> &block) {
  rustaxa::DagManagerBlock out;
  out.hash = to_bridge_hash(block->getHash());
  out.pivot = to_bridge_hash(block->getPivot());
  out.tips = to_bridge_dag_hashes(block->getTips());
  out.level = block->getLevel();
  out.difficulty = block->getDifficulty();
  return out;
}

rust::Vec<uint8_t> to_rust_vec(const dev::bytes &bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

rustaxa::DagVerifyPrecheckBlock to_bridge_verify_precheck_block(const std::shared_ptr<DagBlock> &block) {
  rustaxa::DagVerifyPrecheckBlock out;
  out.level = block->getLevel();
  out.pivot = to_bridge_hash(block->getPivot());
  out.tips = to_bridge_dag_hashes(block->getTips());
  return out;
}

std::optional<DagManager::VerifyBlockReturnType> to_verify_block_reject(uint32_t reject_code) {
  switch (reject_code) {
    case 0:
      return std::nullopt;
    case 2:
      return DagManager::VerifyBlockReturnType::AheadBlock;
    case 6:
      return DagManager::VerifyBlockReturnType::ExpiredBlock;
    case 9:
      return DagManager::VerifyBlockReturnType::FailedTipsVerification;
    default:
      // Reject-code skew is an integration error, not an invalid-block outcome.
      // Do not fall back to DagManagerOld here because that would hide Rust
      // production routing drift.
      throw std::runtime_error("DagManager: unknown Rust verify precheck reject code");
  }
}

}  // namespace

struct DagManager::RustDagManagerGraphs {
  RustDagManagerGraphs(const blk_hash_t &genesis, uint32_t dag_expiry_limit, rustaxa::BridgeStorage &storage)
      : runtime(rustaxa::create_dag_manager_runtime_from_storage(to_bridge_hash(genesis), dag_expiry_limit, storage)) {}

  rust::Box<rustaxa::BridgeDagManagerRuntime> runtime;
};

DagManager::DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
                       std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                       std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager)
    : DagManagerOld(config, node_addr, trx_mgr, std::move(pbft_chain), std::move(final_chain), db,
                    std::move(key_manager)),
      trx_mgr_(std::move(trx_mgr)),
      genesis_block_(std::make_shared<DagBlock>(config.genesis.dag_genesis_block)),
      max_levels_per_period_(config.max_levels_per_period),
      seen_blocks_(cache_max_size_, cache_delete_step_),
      rust_graphs_(std::make_unique<RustDagManagerGraphs>(config.genesis.dag_genesis_block.getHash(),
                                                          config.dag_expiry_limit, db->rustStorage())) {
  rust_graphs_->runtime->dag_manager_runtime_ensure_proposal_period_mapping(max_levels_per_period_, 0);
  rebuildRustGraphsFromOld();
}

DagManager::~DagManager() = default;

void DagManager::rebuildRustGraphsFromOld() {
  const auto anchors = DagManagerOld::getAnchors();
  const auto [period, non_finalized_blks] = DagManagerOld::getNonFinalizedBlocks();
  const auto next_anchor = anchors.second;
  const auto anchor_block = getDagBlock(next_anchor);

  rustaxa::DagManagerSnapshot snapshot;
  snapshot.old_anchor = to_bridge_hash(anchors.first);
  snapshot.anchor = to_bridge_hash(next_anchor);
  snapshot.anchor_level = anchor_block ? anchor_block->getLevel() : 0;
  snapshot.period = period;
  snapshot.max_level = DagManagerOld::getMaxLevel();
  snapshot.dag_expiry_level = DagManagerOld::getDagExpiryLevel();
  snapshot.non_finalized_min_difficulty = DagManagerOld::getNonFinalizedBlocksMinDifficulty();
  snapshot.non_finalized_blocks.reserve(non_finalized_blks.size());

  for (const auto &[level, hashes] : non_finalized_blks) {
    (void)level;
    for (const auto &hash : hashes) {
      if (auto blk = getDagBlock(hash); blk) {
        snapshot.non_finalized_blocks.push_back(to_bridge_manager_block(blk));
      }
    }
  }

  try {
    std::unique_lock lock(rust_graphs_mutex_);
    rust_graphs_->runtime->dag_manager_runtime_rebuild(std::move(snapshot));
  } catch (const std::exception &e) {
    std::cerr << "DagManager: failed to rebuild Rust state mirror: " << e.what() << std::endl;
  }
}

bool DagManager::addBlockToRustGraphs(const std::shared_ptr<DagBlock> &blk) {
  try {
    rust_graphs_->runtime->dag_manager_runtime_add_block(to_bridge_manager_block(blk));
    return true;
  } catch (const std::exception &e) {
    std::cerr << "DagManager: failed to add block to Rust state mirror: " << e.what() << std::endl;
    return false;
  }
}

std::pair<blk_hash_t, std::vector<blk_hash_t>> DagManager::getRustFrontier() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto frontier = rust_graphs_->runtime->dag_manager_runtime_frontier();
  return {from_bridge_hash(frontier.pivot), from_bridge_dag_hashes(frontier.tips)};
}

std::shared_ptr<DagManager> DagManager::getShared() {
  try {
    return std::static_pointer_cast<DagManager>(DagManagerOld::shared_from_this());
  } catch (std::bad_weak_ptr &e) {
    std::cerr << "DagManager: " << e.what() << std::endl;
    return nullptr;
  }
}

void DagManager::setNetwork(std::weak_ptr<Network> network) {
  // TODO(rust-rewrite): migrate DagManager networking ownership to Rust instead of DagManagerOld.
  network_ = network;
  DagManagerOld::setNetwork(std::move(network));
}

bool DagManager::isDagBlockKnown(const blk_hash_t &hash) const {
  if (seen_blocks_.count(hash) != 0 || hash == genesis_block_->getHash()) {
    return true;
  }
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_block_exists(to_bridge_hash(hash));
}

std::shared_ptr<DagBlock> DagManager::getDagBlock(const blk_hash_t &hash) const {
  auto blk = seen_blocks_.get(hash);
  if (blk.second) {
    return blk.first;
  }
  if (hash == genesis_block_->getHash()) {
    return genesis_block_;
  }
  std::shared_lock lock(rust_graphs_mutex_);
  auto block = rust_graphs_->runtime->dag_manager_runtime_load_block(to_bridge_hash(hash));
  if (!block.found) {
    return nullptr;
  }
  dev::RLP rlp(dev::bytesConstRef(block.block_rlp.data(), block.block_rlp.size()));
  return std::make_shared<DagBlock>(rlp);
}

std::pair<DagManager::VerifyBlockReturnType, SharedTransactions> DagManager::verifyBlock(
    const std::shared_ptr<DagBlock> &blk,
    const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs) {
  {
    std::shared_lock lock(rust_graphs_mutex_);
    // Rust bridge/storage failures intentionally propagate as exceptions: they
    // are infrastructure errors, while consensus-invalid blocks are returned as
    // explicit reject codes.
    const auto precheck =
        rust_graphs_->runtime->dag_manager_runtime_verify_precheck(to_bridge_verify_precheck_block(blk));
    if (const auto reject = to_verify_block_reject(precheck.reject_code); reject.has_value()) {
      if (*reject == VerifyBlockReturnType::AheadBlock) {
        seen_blocks_.erase(blk->getHash());
      }
      return {*reject, {}};
    }
  }

  // TODO(rust-rewrite): migrate remaining transaction/VDF/DPOS/gas DAG block verification to Rust instead of
  // DagManagerOld.
  return DagManagerOld::verifyBlock(blk, trxs);
}

std::pair<bool, std::vector<blk_hash_t>> DagManager::pivotAndTipsAvailable(const std::shared_ptr<DagBlock> &blk) {
  const auto pivot_hash = blk->getPivot();
  const auto tips = blk->getTips();
  std::vector<blk_hash_t> missing_tips_or_pivot;

  level_t expected_level = 0;
  if (auto pivot_block = getDagBlock(pivot_hash); pivot_block) {
    expected_level = pivot_block->getLevel() + 1;
  } else {
    missing_tips_or_pivot.push_back(pivot_hash);
  }

  for (auto const &tip : tips) {
    if (auto tip_block = getDagBlock(tip); tip_block) {
      expected_level = std::max(expected_level, tip_block->getLevel() + 1);
    } else {
      missing_tips_or_pivot.push_back(tip);
    }
  }

  if (!missing_tips_or_pivot.empty()) {
    return {false, missing_tips_or_pivot};
  }

  if (expected_level != blk->getLevel()) {
    return {false, missing_tips_or_pivot};
  }

  return {true, {}};
}

std::pair<bool, std::vector<blk_hash_t>> DagManager::addDagBlock(const std::shared_ptr<DagBlock> &blk,
                                                                 SharedTransactions &&trxs, bool proposed, bool save) {
  const auto blk_hash = blk->getHash();
  std::scoped_lock order_lock(rust_order_dag_blocks_mutex_);

  if (save) {
    {
      std::shared_lock lock(rust_graphs_mutex_);
      if (rust_graphs_->runtime->dag_manager_runtime_block_exists(to_bridge_hash(blk_hash))) {
        return {true, {}};
      }
    }

    if (blk->getLevel() < getDagExpiryLevel()) {
      std::cerr << "DagManager: dropping old block " << blk_hash << ". Expiry level: " << getDagExpiryLevel()
                << ". Block level: " << blk->getLevel() << std::endl;
      return {false, {}};
    }

    auto res = pivotAndTipsAvailable(blk);
    if (!res.first) {
      return res;
    }

    trx_mgr_->saveTransactionsFromDagBlock(trxs);
    auto block_rlp = to_rust_vec(blk->rlp(true));
    std::shared_lock lock(rust_graphs_mutex_);
    rust_graphs_->runtime->dag_manager_runtime_save_block(to_bridge_hash(blk_hash), blk->getLevel(),
                                                          blk->getTips().size(), std::move(block_rlp));
  }

  bool added_to_rust_graph = false;
  {
    std::unique_lock lock(rust_graphs_mutex_);
    added_to_rust_graph = addBlockToRustGraphs(blk);
  }
  if (!added_to_rust_graph) {
    throw std::runtime_error("DagManager: failed to add persisted DAG block to Rust graph");
  }

  seen_blocks_.insert(blk_hash, blk);

  // TODO(rust-rewrite): remove this compatibility mirror once out-of-scope
  // verify/sync accessors no longer depend on DagManagerOld in-memory DAG state.
  // This call does not persist, validate, emit, or gossip.
  DagManagerOld::addDagBlock(blk, {}, false, false);

  if (save) {
    block_verified_.emit(blk);
    if (std::shared_ptr<Network> net = network_.lock()) {
      net->gossipDagBlock(blk, proposed, trxs);
    }
  }

  return {true, {}};
}

vec_blk_t DagManager::getDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period) {
  std::shared_lock lock(rust_graphs_mutex_);
  if (period != rust_graphs_->runtime->dag_manager_runtime_latest_period() + 1) {
    return {};
  }
  if (from_bridge_hash(rust_graphs_->runtime->dag_manager_runtime_anchors().anchor) == anchor) {
    return {};
  }
  const auto order = rust_graphs_->runtime->dag_manager_runtime_compute_order(to_bridge_hash(anchor));
  if (!order.found) {
    return {};
  }
  return from_bridge_dag_hashes(order.hashes);
}

uint DagManager::setDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order) {
  std::scoped_lock order_lock(rust_order_dag_blocks_mutex_);
  // TODO(rust-rewrite): migrate finalized DAG order application to Rust instead of DagManagerOld.
  const auto finalized_count = DagManagerOld::setDagBlockOrder(anchor, period, dag_order);
  rebuildRustGraphsFromOld();
  return finalized_count;
}

std::optional<std::pair<blk_hash_t, std::vector<blk_hash_t>>> DagManager::getLatestPivotAndTips() const {
  return {getRustFrontier()};
}

std::vector<blk_hash_t> DagManager::getGhostPath(const blk_hash_t &source) const {
  std::shared_lock lock(rust_graphs_mutex_);
  return from_bridge_dag_hashes(rust_graphs_->runtime->dag_manager_runtime_ghost_path(to_bridge_hash(source)));
}

std::vector<blk_hash_t> DagManager::getGhostPath() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return from_bridge_dag_hashes(rust_graphs_->runtime->dag_manager_runtime_anchor_ghost_path());
}

void DagManager::drawTotalGraph(std::string const &str) const {
  std::shared_lock lock(rust_graphs_mutex_);
  std::ofstream outfile(str.c_str());
  outfile << std::string(rust_graphs_->runtime->dag_manager_runtime_graphviz_dot(false));
  std::cout << "Dot file " << str << " generated!" << std::endl;
  std::cout << "Use \"dot -Tpdf <dot file> -o <pdf file>\" to generate pdf file" << std::endl;
}

void DagManager::drawPivotGraph(std::string const &str) const {
  std::shared_lock lock(rust_graphs_mutex_);
  std::ofstream outfile(str.c_str());
  outfile << std::string(rust_graphs_->runtime->dag_manager_runtime_graphviz_dot(true));
  std::cout << "Dot file " << str << " generated!" << std::endl;
  std::cout << "Use \"dot -Tpdf <dot file> -o <pdf file>\" to generate pdf file" << std::endl;
}

void DagManager::drawGraph(std::string const &dotfile) const {
  drawPivotGraph("pivot." + dotfile);
  drawTotalGraph("total." + dotfile);
}

std::pair<uint64_t, uint64_t> DagManager::getNumVerticesInDag() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto persisted_counts = rust_graphs_->runtime->dag_manager_runtime_persistence_counters();
  return {persisted_counts.dag_blocks, rust_graphs_->runtime->dag_manager_runtime_vertex_count()};
}

std::pair<uint64_t, uint64_t> DagManager::getNumEdgesInDag() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto persisted_counts = rust_graphs_->runtime->dag_manager_runtime_persistence_counters();
  return {persisted_counts.dag_edges, rust_graphs_->runtime->dag_manager_runtime_edge_count()};
}

level_t DagManager::getMaxLevel() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_max_level();
}

PbftPeriod DagManager::getLatestPeriod() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_latest_period();
}

std::pair<blk_hash_t, blk_hash_t> DagManager::getAnchors() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto anchors = rust_graphs_->runtime->dag_manager_runtime_anchors();
  return std::make_pair(from_bridge_hash(anchors.old_anchor), from_bridge_hash(anchors.anchor));
}

uint32_t DagManager::getDagExpiryLimit() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_dag_expiry_limit();
}

const std::pair<PbftPeriod, std::map<uint64_t, std::unordered_set<blk_hash_t>>> DagManager::getNonFinalizedBlocks()
    const {
  std::shared_lock lock(rust_graphs_mutex_);
  return {rust_graphs_->runtime->dag_manager_runtime_latest_period(),
          from_bridge_level_hashes(rust_graphs_->runtime->dag_manager_runtime_non_finalized_blocks())};
}

const std::tuple<PbftPeriod, std::vector<std::shared_ptr<DagBlock>>, SharedTransactions>
DagManager::getNonFinalizedBlocksWithTransactions(const std::unordered_set<blk_hash_t> &known_hashes) const {
  // TODO(rust-rewrite): migrate non-finalized DAG block/transaction collection to Rust instead of DagManagerOld.
  return DagManagerOld::getNonFinalizedBlocksWithTransactions(known_hashes);
}

DagFrontier DagManager::getDagFrontier() {
  const auto [pivot, tips] = getRustFrontier();
  return DagFrontier(pivot, tips);
}

std::pair<size_t, size_t> DagManager::getNonFinalizedBlocksSize() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto size = rust_graphs_->runtime->dag_manager_runtime_non_finalized_blocks_size();
  return {size.levels, size.blocks};
}

uint32_t DagManager::getNonFinalizedBlocksMinDifficulty() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_non_finalized_min_difficulty();
}

std::shared_mutex &DagManager::getDagMutex() {
  // TODO(rust-rewrite): migrate DagManager synchronization ownership to Rust instead of DagManagerOld.
  return DagManagerOld::getDagMutex();
}

SortitionParamsManager &DagManager::sortitionParamsManager() {
  // TODO(rust-rewrite): migrate sortition access out of DagManagerOld.
  return DagManagerOld::sortitionParamsManager();
}

const DagConfig &DagManager::getDagConfig() const {
  // TODO(rust-rewrite): migrate DAG config access out of DagManagerOld.
  return DagManagerOld::getDagConfig();
}

uint64_t DagManager::getDagExpiryLevel() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_dag_expiry_level();
}

uint64_t DagManager::getMaxLevelsPerPeriod() const {
  return max_levels_per_period_;
}

dev::bytes DagManager::getVdfMessage(blk_hash_t const &hash, SharedTransactions const &trxs) {
  // TODO(rust-rewrite): migrate DAG VDF message encoding to Rust instead of DagManagerOld.
  return DagManagerOld::getVdfMessage(hash, trxs);
}

dev::bytes DagManager::getVdfMessage(blk_hash_t const &hash, std::vector<trx_hash_t> const &trx_hashes) {
  // TODO(rust-rewrite): migrate DAG VDF message encoding to Rust instead of DagManagerOld.
  return DagManagerOld::getVdfMessage(hash, trx_hashes);
}

}  // namespace taraxa
