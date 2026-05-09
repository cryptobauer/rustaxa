#include "dag/dag_manager.hpp"

#include <array>
#include <iostream>
#include <utility>

#include "dag/dag_block.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {
namespace {

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t &hash) { return hash.asArray(); }

rustaxa::DagReferenceMetadata to_bridge_reference(const blk_hash_t &hash, const std::shared_ptr<DagBlock> &block) {
  rustaxa::DagReferenceMetadata out;
  out.hash = to_bridge_hash(hash);
  out.found = static_cast<bool>(block);
  out.level = block ? block->getLevel() : 0;
  return out;
}

}  // namespace

DagManager::DagManager(const FullNodeConfig &config, addr_t node_addr, std::shared_ptr<TransactionManager> trx_mgr,
                       std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                       std::shared_ptr<DbStorage> db, std::shared_ptr<KeyManager> key_manager)
    : DagManagerOld(config, node_addr, std::move(trx_mgr), std::move(pbft_chain), std::move(final_chain), std::move(db),
                    std::move(key_manager)) {}

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
  return DagManagerOld::addDagBlock(blk, std::move(trxs), proposed, save);
}

vec_blk_t DagManager::getDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period) {
  // TODO(rust-rewrite): migrate DAG ordering to Rust instead of DagManagerOld.
  return DagManagerOld::getDagBlockOrder(anchor, period);
}

uint DagManager::setDagBlockOrder(blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order) {
  // TODO(rust-rewrite): migrate finalized DAG order application to Rust instead of DagManagerOld.
  return DagManagerOld::setDagBlockOrder(anchor, period, dag_order);
}

std::optional<std::pair<blk_hash_t, std::vector<blk_hash_t>>> DagManager::getLatestPivotAndTips() const {
  // TODO(rust-rewrite): migrate frontier retrieval to Rust instead of DagManagerOld.
  return DagManagerOld::getLatestPivotAndTips();
}

std::vector<blk_hash_t> DagManager::getGhostPath(const blk_hash_t &source) const {
  // TODO(rust-rewrite): migrate ghost-path traversal to Rust instead of DagManagerOld.
  return DagManagerOld::getGhostPath(source);
}

std::vector<blk_hash_t> DagManager::getGhostPath() const {
  // TODO(rust-rewrite): migrate anchor ghost-path traversal to Rust instead of DagManagerOld.
  return DagManagerOld::getGhostPath();
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
  // TODO(rust-rewrite): migrate DAG vertex counting to Rust instead of DagManagerOld.
  return DagManagerOld::getNumVerticesInDag();
}

std::pair<uint64_t, uint64_t> DagManager::getNumEdgesInDag() const {
  // TODO(rust-rewrite): migrate DAG edge counting to Rust instead of DagManagerOld.
  return DagManagerOld::getNumEdgesInDag();
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
  // TODO(rust-rewrite): migrate DAG frontier cache to Rust instead of DagManagerOld.
  return DagManagerOld::getDagFrontier();
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
