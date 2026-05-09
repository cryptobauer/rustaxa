#include "dag/dag_manager.hpp"

#include <array>
#include <iostream>
#include <map>
#include <mutex>
#include <shared_mutex>
#include <utility>
#include <vector>

#include "dag/dag_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"

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

rust::Vec<rustaxa::DagLevelHashes> to_bridge_level_hashes(
    const std::map<uint64_t, std::unordered_set<blk_hash_t>> &non_finalized_blks) {
  rust::Vec<rustaxa::DagLevelHashes> out;
  out.reserve(non_finalized_blks.size());
  for (const auto &[level, hashes] : non_finalized_blks) {
    rustaxa::DagLevelHashes level_hashes;
    level_hashes.level = level;
    level_hashes.hashes.reserve(hashes.size());
    for (const auto &hash : hashes) {
      level_hashes.hashes.push_back(to_bridge_dag_hash(hash));
    }
    out.push_back(std::move(level_hashes));
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

rustaxa::DagReferenceMetadata to_bridge_reference(const blk_hash_t &hash, const std::shared_ptr<DagBlock> &block) {
  rustaxa::DagReferenceMetadata out;
  out.hash = to_bridge_hash(hash);
  out.found = static_cast<bool>(block);
  out.level = block ? block->getLevel() : 0;
  return out;
}

}  // namespace

struct DagManager::RustDagManagerGraphs {
  explicit RustDagManagerGraphs(const blk_hash_t &genesis)
      : total_dag(rustaxa::create_dag_graph(to_bridge_hash(genesis))),
        pivot_tree(rustaxa::create_dag_graph(to_bridge_hash(genesis))),
        anchor(genesis) {}

  rust::Box<rustaxa::BridgeDagGraph> total_dag;
  rust::Box<rustaxa::BridgeDagGraph> pivot_tree;
  blk_hash_t anchor;
};

DagManager::DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
                       std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                       std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager)
    : DagManagerOld(config, node_addr, std::move(trx_mgr), std::move(pbft_chain), std::move(final_chain), std::move(db),
                    std::move(key_manager)),
      rust_graphs_(std::make_unique<RustDagManagerGraphs>(config.genesis.dag_genesis_block.getHash())) {
  rebuildRustGraphsFromOld();
}

DagManager::~DagManager() = default;

void DagManager::rebuildRustGraphsFromOld() {
  const auto anchors = DagManagerOld::getAnchors();
  const auto [_, non_finalized_blks] = DagManagerOld::getNonFinalizedBlocks();
  std::vector<std::shared_ptr<DagBlock>> non_finalized_blocks;
  for (const auto &[level, hashes] : non_finalized_blks) {
    (void)level;
    for (const auto &hash : hashes) {
      if (auto blk = DagManagerOld::getDagBlock(hash); blk) {
        non_finalized_blocks.push_back(std::move(blk));
      }
    }
  }

  std::unique_lock lock(rust_graphs_mutex_);
  rust_graphs_->anchor = anchors.second.isZero() ? rust_graphs_->anchor : anchors.second;
  rust_graphs_->total_dag->dag_clear();
  rust_graphs_->pivot_tree->dag_clear();
  if (!rust_graphs_->total_dag->dag_add_vertex_edges(to_bridge_hash(rust_graphs_->anchor),
                                                     to_bridge_hash(kNullBlockHash), {})) {
    std::cerr << "DagManager: failed to add Rust total DAG anchor " << rust_graphs_->anchor << std::endl;
  }
  if (!rust_graphs_->pivot_tree->dag_add_vertex_edges(to_bridge_hash(rust_graphs_->anchor),
                                                      to_bridge_hash(kNullBlockHash), {})) {
    std::cerr << "DagManager: failed to add Rust pivot DAG anchor " << rust_graphs_->anchor << std::endl;
  }

  for (const auto &blk : non_finalized_blocks) {
    if (!addBlockToRustGraphs(blk)) {
      std::cerr << "DagManager: failed to mirror DAG block into Rust graph " << blk->getHash() << std::endl;
    }
  }
}

bool DagManager::addBlockToRustGraphs(const std::shared_ptr<DagBlock> &blk) {
  const auto block_hash = blk->getHash();
  if (rust_graphs_->total_dag->dag_has_vertex(to_bridge_hash(block_hash)) &&
      rust_graphs_->pivot_tree->dag_has_vertex(to_bridge_hash(block_hash))) {
    return true;
  }

  const auto pivot_hash = blk->getPivot();
  const auto tips = blk->getTips();
  const auto total_added = rust_graphs_->total_dag->dag_add_vertex_edges(
      to_bridge_hash(block_hash), to_bridge_hash(pivot_hash), to_bridge_dag_hashes(tips));
  const auto pivot_added =
      rust_graphs_->pivot_tree->dag_add_vertex_edges(to_bridge_hash(block_hash), to_bridge_hash(pivot_hash), {});
  return total_added && pivot_added;
}

std::pair<blk_hash_t, std::vector<blk_hash_t>> DagManager::getRustFrontier() const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto pivot_chain = rust_graphs_->pivot_tree->dag_ghost_path(to_bridge_hash(rust_graphs_->anchor));
  const auto leaves = rust_graphs_->total_dag->dag_leaves();
  const auto frontier = rustaxa::dag_derive_frontier(pivot_chain, leaves);
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
  DagManagerOld::setNetwork(std::move(network));
}

bool DagManager::isDagBlockKnown(const blk_hash_t &hash) const {
  // TODO(rust-rewrite): migrate DAG block-known checks to Rust instead of DagManagerOld.
  return DagManagerOld::isDagBlockKnown(hash);
}

std::shared_ptr<DagBlock> DagManager::getDagBlock(const blk_hash_t &hash) const {
  // TODO(rust-rewrite): migrate DAG block lookup to Rust-backed state/storage instead of DagManagerOld.
  return DagManagerOld::getDagBlock(hash);
}

std::pair<DagManager::VerifyBlockReturnType, SharedTransactions> DagManager::verifyBlock(
    const std::shared_ptr<DagBlock> &blk,
    const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs) {
  // TODO(rust-rewrite): migrate DAG block verification to Rust instead of DagManagerOld.
  return DagManagerOld::verifyBlock(blk, trxs);
}

std::pair<bool, std::vector<blk_hash_t>> DagManager::pivotAndTipsAvailable(const std::shared_ptr<DagBlock> &blk) {
  std::vector<blk_hash_t> missing_tips_or_pivot;
  const auto pivot_hash = blk->getPivot();

  // TODO(rust-rewrite): source reference block metadata from Rust DAG state instead of DagManagerOld.
  const auto dag_blk_pivot = DagManagerOld::getDagBlock(pivot_hash);
  if (!dag_blk_pivot) {
    missing_tips_or_pivot.push_back(pivot_hash);
  }

  rust::Vec<rustaxa::DagReferenceMetadata> tip_refs;
  tip_refs.reserve(blk->getTips().size());
  for (const auto &tip : blk->getTips()) {
    // TODO(rust-rewrite): source tip metadata from Rust DAG state instead of DagManagerOld.
    const auto tip_block = DagManagerOld::getDagBlock(tip);
    tip_refs.push_back(to_bridge_reference(tip, tip_block));
    if (!tip_block) {
      missing_tips_or_pivot.push_back(tip);
    }
  }

  if (!missing_tips_or_pivot.empty()) {
    return {false, missing_tips_or_pivot};
  }

  const auto validation = rustaxa::dag_validate_pivot_tips_metadata(
      blk->getLevel(), to_bridge_reference(pivot_hash, dag_blk_pivot), std::move(tip_refs));
  if (!validation.level_matches) {
    return {false, {}};
  }

  return {true, {}};
}

std::pair<bool, std::vector<blk_hash_t>> DagManager::addDagBlock(const std::shared_ptr<DagBlock> &blk,
                                                                 SharedTransactions &&trxs, bool proposed, bool save) {
  // TODO(rust-rewrite): migrate DAG block insertion/frontier updates to Rust instead of DagManagerOld.
  const auto was_known_save = save && DagManagerOld::isDagBlockKnown(blk->getHash());
  auto result = DagManagerOld::addDagBlock(blk, std::move(trxs), proposed, save);
  if (result.first && !was_known_save) {
    bool mirrored = false;
    {
      std::unique_lock lock(rust_graphs_mutex_);
      mirrored = addBlockToRustGraphs(blk);
    }
    if (!mirrored) {
      std::cerr << "DagManager: failed to mirror added DAG block into Rust graph " << blk->getHash()
                << "; rebuilding Rust graph mirror" << std::endl;
      rebuildRustGraphsFromOld();
    }
  }
  return result;
}

vec_blk_t DagManager::getDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period) {
  if (period != DagManagerOld::getLatestPeriod() + 1) {
    return {};
  }
  if (DagManagerOld::getAnchors().second == anchor) {
    return {};
  }

  const auto [_, non_finalized_blks] = DagManagerOld::getNonFinalizedBlocks();
  std::shared_lock lock(rust_graphs_mutex_);
  const auto order =
      rust_graphs_->total_dag->dag_compute_order(to_bridge_hash(anchor), to_bridge_level_hashes(non_finalized_blks));
  if (!order.found) {
    return {};
  }
  return from_bridge_dag_hashes(order.hashes);
}

uint DagManager::setDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order) {
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
  return from_bridge_dag_hashes(rust_graphs_->pivot_tree->dag_ghost_path(to_bridge_hash(source)));
}

std::vector<blk_hash_t> DagManager::getGhostPath() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return from_bridge_dag_hashes(rust_graphs_->pivot_tree->dag_ghost_path(to_bridge_hash(rust_graphs_->anchor)));
}

void DagManager::drawTotalGraph(std::string const &str) const {
  // TODO(rust-rewrite): migrate total DAG graph output to Rust instead of DagManagerOld.
  DagManagerOld::drawTotalGraph(str);
}

void DagManager::drawPivotGraph(std::string const &str) const {
  // TODO(rust-rewrite): migrate pivot DAG graph output to Rust instead of DagManagerOld.
  DagManagerOld::drawPivotGraph(str);
}

void DagManager::drawGraph(std::string const &dotfile) const {
  // TODO(rust-rewrite): migrate DAG graph output to Rust instead of DagManagerOld.
  DagManagerOld::drawGraph(dotfile);
}

std::pair<uint64_t, uint64_t> DagManager::getNumVerticesInDag() const {
  const auto persisted_count = DagManagerOld::getNumVerticesInDag().first;
  std::shared_lock lock(rust_graphs_mutex_);
  return {persisted_count, rust_graphs_->total_dag->dag_vertex_count()};
}

std::pair<uint64_t, uint64_t> DagManager::getNumEdgesInDag() const {
  const auto persisted_count = DagManagerOld::getNumEdgesInDag().first;
  std::shared_lock lock(rust_graphs_mutex_);
  return {persisted_count, rust_graphs_->total_dag->dag_edge_count()};
}

level_t DagManager::getMaxLevel() const {
  // TODO(rust-rewrite): migrate max-level tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getMaxLevel();
}

PbftPeriod DagManager::getLatestPeriod() const {
  // TODO(rust-rewrite): migrate latest-period tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getLatestPeriod();
}

std::pair<blk_hash_t, blk_hash_t> DagManager::getAnchors() const {
  // TODO(rust-rewrite): migrate anchor tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getAnchors();
}

uint32_t DagManager::getDagExpiryLimit() const {
  // TODO(rust-rewrite): migrate DAG expiry configuration access to Rust instead of DagManagerOld.
  return DagManagerOld::getDagExpiryLimit();
}

const std::pair<PbftPeriod, std::map<uint64_t, std::unordered_set<blk_hash_t>>> DagManager::getNonFinalizedBlocks()
    const {
  // TODO(rust-rewrite): migrate non-finalized DAG block tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getNonFinalizedBlocks();
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
  // TODO(rust-rewrite): migrate non-finalized DAG size tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getNonFinalizedBlocksSize();
}

uint32_t DagManager::getNonFinalizedBlocksMinDifficulty() const {
  // TODO(rust-rewrite): migrate non-finalized DAG difficulty tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getNonFinalizedBlocksMinDifficulty();
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
  // TODO(rust-rewrite): migrate DAG expiry level tracking to Rust instead of DagManagerOld.
  return DagManagerOld::getDagExpiryLevel();
}

uint64_t DagManager::getMaxLevelsPerPeriod() const {
  // TODO(rust-rewrite): migrate max-levels-per-period access out of DagManagerOld.
  return DagManagerOld::getMaxLevelsPerPeriod();
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
