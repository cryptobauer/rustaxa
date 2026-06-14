#include <algorithm>
#include <array>
#include <fstream>
#include <iostream>
#include <map>
#include <mutex>
#include <optional>
#include <shared_mutex>
#include <stdexcept>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>

#include "dag/dag_block.hpp"
#include "dag/dag_manager.hpp"
#include "key_manager/key_manager.hpp"
#include "network/network.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kDagVerifyVdfStatusNotChecked = 0;
constexpr uint8_t kDagVerifyVdfStatusValid = 1;
constexpr uint8_t kDagVerifyVdfStatusInvalid = 2;
constexpr uint8_t kDagVerifyDposStatusNotChecked = 0;
constexpr uint8_t kDagVerifyDposStatusSnapshotUnavailable = 1;
constexpr uint8_t kPbftFinalizationRuntimeActionSetDagBlockOrder = 4;

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t &hash) { return hash.asArray(); }

rustaxa::DagHash to_bridge_dag_hash(const blk_hash_t &hash) { return rustaxa::DagHash{to_bridge_hash(hash)}; }

rustaxa::DagTransactionHash to_bridge_dag_transaction_hash(const trx_hash_t &hash) {
  return rustaxa::DagTransactionHash{hash.asArray()};
}

rust::Vec<rustaxa::DagHash> to_bridge_dag_hashes(const std::vector<blk_hash_t> &hashes) {
  rust::Vec<rustaxa::DagHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(to_bridge_dag_hash(hash));
  }
  return out;
}

rust::Vec<rustaxa::DagHash> to_bridge_dag_hashes(const std::unordered_set<blk_hash_t> &hashes) {
  rust::Vec<rustaxa::DagHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(to_bridge_dag_hash(hash));
  }
  return out;
}

rust::Vec<rustaxa::DagTransactionHash> to_bridge_dag_transaction_hashes(const std::vector<trx_hash_t> &hashes) {
  rust::Vec<rustaxa::DagTransactionHash> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.push_back(to_bridge_dag_transaction_hash(hash));
  }
  return out;
}

blk_hash_t from_bridge_hash(const std::array<uint8_t, 32> &hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

blk_hash_t from_bridge_dag_hash(const rustaxa::DagHash &hash) { return from_bridge_hash(hash.hash); }
trx_hash_t from_bridge_dag_transaction_hash(const rustaxa::DagTransactionHash &hash) {
  return trx_hash_t(hash.hash.data(), trx_hash_t::ConstructFromPointer);
}

std::vector<blk_hash_t> from_bridge_dag_hashes(const rust::Vec<rustaxa::DagHash> &hashes) {
  std::vector<blk_hash_t> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.emplace_back(from_bridge_dag_hash(hash));
  }
  return out;
}

std::vector<trx_hash_t> from_bridge_dag_transaction_hashes(const rust::Vec<rustaxa::DagTransactionHash> &hashes) {
  std::vector<trx_hash_t> out;
  out.reserve(hashes.size());
  for (const auto &hash : hashes) {
    out.emplace_back(from_bridge_dag_transaction_hash(hash));
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

dev::bytes from_rust_bytes(const rust::Vec<uint8_t> &bytes) {
  dev::bytes out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.emplace_back(byte);
  }
  return out;
}

std::vector<std::shared_ptr<Transaction>> from_bridge_dag_transaction_rlps(
    const rust::Vec<rustaxa::DagTransactionRlpLookup> &rlps) {
  std::vector<std::shared_ptr<Transaction>> out;
  out.reserve(rlps.size());
  for (const auto &entry : rlps) {
    if (!entry.found) {
      throw std::runtime_error("DagManager: selected non-finalized transaction missing from Rust storage");
    }
    auto transaction = std::make_shared<Transaction>(from_rust_bytes(entry.tx_rlp));
    if (transaction->getHash() != trx_hash_t(entry.hash.data(), trx_hash_t::ConstructFromPointer)) {
      throw std::runtime_error("DagManager: Rust storage transaction RLP hash does not match requested hash");
    }
    out.emplace_back(std::move(transaction));
  }
  return out;
}

std::vector<std::shared_ptr<DagBlock>> from_bridge_dag_sync_blocks(const rust::Vec<rustaxa::DagSyncBlockRlp> &blocks) {
  std::vector<std::shared_ptr<DagBlock>> out;
  out.reserve(blocks.size());
  for (const auto &entry : blocks) {
    const auto bytes = from_rust_bytes(entry.block_rlp);
    dev::RLP rlp(dev::bytesConstRef(bytes.data(), bytes.size()));
    auto block = std::make_shared<DagBlock>(rlp);
    if (block->getHash() != from_bridge_hash(entry.hash)) {
      throw std::runtime_error("DagManager: Rust sync DAG block RLP hash does not match selected hash");
    }
    out.emplace_back(std::move(block));
  }
  return out;
}

std::array<uint8_t, 32> to_bridge_vrf_public_key(const rust::Vec<uint8_t> &vrf_public_key) {
  if (vrf_public_key.size() != 32) {
    throw std::runtime_error("DagManager: VRF public key must be 32 bytes");
  }

  std::array<uint8_t, 32> out{};
  std::copy(vrf_public_key.begin(), vrf_public_key.end(), out.begin());
  return out;
}

rustaxa::SortitionRuntimeParams to_bridge_sortition_params(const SortitionParams &params) {
  rustaxa::SortitionRuntimeParams out;
  out.threshold_upper = params.vrf.threshold_upper;
  out.difficulty_min = params.vdf.difficulty_min;
  out.difficulty_max = params.vdf.difficulty_max;
  out.difficulty_stale = params.vdf.difficulty_stale;
  out.lambda_bound = params.vdf.lambda_bound;
  return out;
}

rustaxa::DagVerifyVdfSortitionFromBlockInput to_bridge_vdf_sortition_input(
    const dev::bytes &block_rlp, uint64_t block_level, const blk_hash_t &proposal_period_hash,
    SortitionParams const &sortition_params, const rust::Vec<uint8_t> &vrf_public_key,
    uint64_t sender_eligible_vote_count, uint64_t vdf_sortition_max_vote_count) {
  rustaxa::DagVerifyVdfSortitionFromBlockInput out;
  out.block_rlp = to_rust_vec(block_rlp);
  out.block_level = block_level;
  out.proposal_period_hash = to_bridge_hash(proposal_period_hash);
  out.sortition_params = to_bridge_sortition_params(sortition_params);
  out.vrf_public_key = to_bridge_vrf_public_key(vrf_public_key);
  out.sender_eligible_vote_count = sender_eligible_vote_count;
  out.vdf_sortition_max_vote_count = vdf_sortition_max_vote_count;
  return out;
}

dev::bytes rust_vdf_message(const blk_hash_t &pivot, const std::vector<trx_hash_t> &trx_hashes) {
  const auto bridge_pivot = to_bridge_hash(pivot);
  return from_rust_bytes(rustaxa::dag_vdf_message(bridge_pivot, to_bridge_dag_hashes(trx_hashes)));
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
    case 1:
      return DagManager::VerifyBlockReturnType::MissingTransaction;
    case 2:
      return DagManager::VerifyBlockReturnType::AheadBlock;
    case 3:
      return DagManager::VerifyBlockReturnType::FailedVdfVerification;
    case 4:
      return DagManager::VerifyBlockReturnType::FutureBlock;
    case 5:
      return DagManager::VerifyBlockReturnType::NotEligible;
    case 6:
      return DagManager::VerifyBlockReturnType::ExpiredBlock;
    case 7:
      return DagManager::VerifyBlockReturnType::IncorrectTransactionsEstimation;
    case 8:
      return DagManager::VerifyBlockReturnType::BlockTooBig;
    case 9:
      return DagManager::VerifyBlockReturnType::FailedTipsVerification;
    case 10:
      return DagManager::VerifyBlockReturnType::MissingTip;
    default:
      // Reject-code skew is an integration error, not an invalid-block outcome.
      // Do not fall back to DagManagerOld here because that would hide Rust
      // production routing drift.
      throw std::runtime_error("DagManager: unknown Rust verify precheck reject code");
  }
}

rustaxa::DagVerifyVdfDposFacts to_bridge_vdf_dpos_facts(bool vrf_key_found, uint64_t sender_eligible_vote_count,
                                                        uint64_t vdf_sortition_max_vote_count, uint8_t vdf_status,
                                                        uint8_t dpos_status) {
  rustaxa::DagVerifyVdfDposFacts out;
  out.vrf_key_found = vrf_key_found;
  out.sender_eligible_vote_count = sender_eligible_vote_count;
  out.vdf_sortition_max_vote_count = vdf_sortition_max_vote_count;
  out.vdf_status = vdf_status;
  out.dpos_status = dpos_status;
  return out;
}

std::optional<DagManager::VerifyBlockReturnType> decide_vdf_dpos_authorization(
    const rustaxa::DagVerifyVdfDposFacts &facts) {
  const auto decision = rustaxa::dag_decide_vdf_dpos_authorization(facts);
  return to_verify_block_reject(decision.reject_code);
}

rustaxa::DagVerifyTransactionAvailabilityInput to_bridge_transaction_availability_input(
    uint64_t expected_transactions, uint64_t resolved_transactions) {
  rustaxa::DagVerifyTransactionAvailabilityInput out;
  out.expected_transactions = expected_transactions;
  out.resolved_transactions = resolved_transactions;
  return out;
}

rustaxa::DagVerifyGasInput to_bridge_gas_input(uint64_t block_gas_estimation, uint64_t estimated_transactions_weight,
                                               uint64_t dag_gas_limit, uint64_t pbft_gas_limit,
                                               rust::Vec<rustaxa::DagTipGas> tip_gas_estimations) {
  rustaxa::DagVerifyGasInput out;
  out.block_gas_estimation = block_gas_estimation;
  out.estimated_transactions_weight = estimated_transactions_weight;
  out.dag_gas_limit = dag_gas_limit;
  out.pbft_gas_limit = pbft_gas_limit;
  out.tip_gas_estimations = std::move(tip_gas_estimations);
  return out;
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
    : DagManagerOld(config, node_addr, trx_mgr, pbft_chain, final_chain, db, key_manager),
      trx_mgr_(std::move(trx_mgr)),
      pbft_chain_(std::move(pbft_chain)),
      final_chain_(std::move(final_chain)),
      db_(std::move(db)),
      key_manager_(std::move(key_manager)),
      sortition_params_manager_(node_addr, config, db_),
      genesis_config_(config.genesis),
      genesis_block_(std::make_shared<DagBlock>(config.genesis.dag_genesis_block)),
      max_levels_per_period_(config.max_levels_per_period),
      seen_blocks_(cache_max_size_, cache_delete_step_),
      rust_graphs_(std::make_unique<RustDagManagerGraphs>(config.genesis.dag_genesis_block.getHash(),
                                                          config.dag_expiry_limit, db_->rustStorage())) {
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
    const std::shared_ptr<DagBlock> &blk, const std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> &trxs) {
  const auto &block_hash = blk->getHash();
  uint64_t proposal_period = 0;
  {
    std::shared_lock lock(rust_graphs_mutex_);
    // Rust bridge/storage failures intentionally propagate as exceptions: they
    // are infrastructure errors, while consensus-invalid blocks are returned as
    // explicit reject codes.
    const auto precheck =
        rust_graphs_->runtime->dag_manager_runtime_verify_precheck(to_bridge_verify_precheck_block(blk));
    proposal_period = precheck.proposal_period;
    if (const auto reject = to_verify_block_reject(precheck.reject_code); reject.has_value()) {
      if (*reject == VerifyBlockReturnType::AheadBlock) {
        seen_blocks_.erase(block_hash);
      }
      return {*reject, {}};
    }
  }

  const auto &all_block_trx_hashes = blk->getTrxs();
  vec_trx_t supplied_trx_hashes;
  supplied_trx_hashes.reserve(trxs.size());
  for (const auto &entry : trxs) {
    supplied_trx_hashes.push_back(entry.first);
  }
  auto query_plan = rustaxa::dag_plan_verify_transaction_query(to_bridge_dag_transaction_hashes(all_block_trx_hashes),
                                                               to_bridge_dag_transaction_hashes(supplied_trx_hashes));
  std::vector<trx_hash_t> planned_query_hashes;
  planned_query_hashes.reserve(query_plan.query_hashes.size());
  for (const auto &hash : query_plan.query_hashes) {
    planned_query_hashes.emplace_back(from_bridge_dag_transaction_hash(hash));
  }

  SharedTransactions all_block_trxs;
  all_block_trxs.reserve(all_block_trx_hashes.size());

  std::unordered_map<trx_hash_t, std::shared_ptr<Transaction>> queried_transactions;
  queried_transactions.reserve(planned_query_hashes.size());

  for (const auto &transaction : trx_mgr_->getTransactions(planned_query_hashes, proposal_period)) {
    queried_transactions.emplace(transaction->getHash(), transaction);
  }

  for (const auto &tx_hash : all_block_trx_hashes) {
    if (const auto it = trxs.find(tx_hash); it != trxs.end()) {
      all_block_trxs.emplace_back(it->second);
      continue;
    }

    const auto it = queried_transactions.find(tx_hash);
    if (it == queried_transactions.end()) {
      seen_blocks_.erase(block_hash);
      return {VerifyBlockReturnType::MissingTransaction, {}};
    }
    all_block_trxs.emplace_back(it->second);
  }

  const auto transaction_availability = rustaxa::dag_verify_transaction_availability(
      to_bridge_transaction_availability_input(all_block_trx_hashes.size(), all_block_trxs.size()));
  if (const auto reject = to_verify_block_reject(transaction_availability.reject_code); reject.has_value()) {
    if (*reject == VerifyBlockReturnType::MissingTransaction) {
      seen_blocks_.erase(block_hash);
    }
    return {*reject, {}};
  }

  const auto authorization_facts = final_chain_->dagDposAuthorizationFacts(proposal_period, blk->getSender());
  const bool vrf_key_found = authorization_facts.vrf_key_found;
  const uint64_t sender_eligible_vote_count = authorization_facts.sender_eligible_vote_count;
  const uint64_t vdf_sortition_max_vote_count = authorization_facts.vdf_sortition_max_vote_count;
  const uint8_t eligibility_status = authorization_facts.eligibility_status;

  if (const auto reject = decide_vdf_dpos_authorization(to_bridge_vdf_dpos_facts(
          vrf_key_found, sender_eligible_vote_count, vdf_sortition_max_vote_count, kDagVerifyVdfStatusNotChecked,
          eligibility_status == kDagVerifyDposStatusSnapshotUnavailable ? kDagVerifyDposStatusSnapshotUnavailable
                                                                        : kDagVerifyDposStatusNotChecked));
      reject.has_value()) {
    return {*reject, {}};
  }

  uint8_t vdf_status = kDagVerifyVdfStatusValid;
  if (vrf_key_found) {
    try {
      const auto proposal_period_hash = getPeriodBlockHashForDagProposal(proposal_period);
      const auto block_rlp = blk->rlp(true);
      const auto sortition_params = sortition_params_manager_.getSortitionParams(proposal_period);
      const auto vdf_result = rustaxa::dag_verify_vdf_sortition_from_block(to_bridge_vdf_sortition_input(
          block_rlp, blk->getLevel(), proposal_period_hash, sortition_params, authorization_facts.vrf_key,
          sender_eligible_vote_count, vdf_sortition_max_vote_count));
      vdf_status = vdf_result.vdf_status;
    } catch (std::exception const &) {
      vdf_status = kDagVerifyVdfStatusInvalid;
    }
  }
  if (const auto reject = decide_vdf_dpos_authorization(to_bridge_vdf_dpos_facts(
          true, sender_eligible_vote_count, vdf_sortition_max_vote_count, vdf_status,
          eligibility_status == kDagVerifyDposStatusSnapshotUnavailable ? kDagVerifyDposStatusSnapshotUnavailable
                                                                        : kDagVerifyDposStatusNotChecked));
      reject.has_value()) {
    return {*reject, {}};
  }

  if (const auto reject = decide_vdf_dpos_authorization(
          to_bridge_vdf_dpos_facts(true, sender_eligible_vote_count, vdf_sortition_max_vote_count,
                                   kDagVerifyVdfStatusValid, eligibility_status));
      reject.has_value()) {
    return {*reject, {}};
  }

  const auto [dag_gas_limit, pbft_gas_limit] = genesis_config_.getGasLimits(proposal_period);
  const auto estimated_transactions_weight = trx_mgr_->estimateTransactions(all_block_trxs, proposal_period);

  rust::Vec<rustaxa::DagTipGas> tip_gas_estimations;
  const auto needs_tip_gas =
      dag_gas_limit == 0 || static_cast<uint64_t>(blk->getTips().size() + 1) > pbft_gas_limit / dag_gas_limit;
  if (needs_tip_gas) {
    tip_gas_estimations.reserve(blk->getTips().size());
    for (const auto &tip_hash : blk->getTips()) {
      rustaxa::DagTipGas tip_gas;
      if (const auto tip_block = getDagBlock(tip_hash); tip_block) {
        tip_gas.found = true;
        tip_gas.gas_estimation = tip_block->getGasEstimation();
      } else {
        tip_gas.found = false;
        tip_gas.gas_estimation = 0;
      }
      tip_gas_estimations.push_back(tip_gas);
    }
  }

  const auto gas =
      rustaxa::dag_verify_gas(to_bridge_gas_input(blk->getGasEstimation(), estimated_transactions_weight, dag_gas_limit,
                                                  pbft_gas_limit, std::move(tip_gas_estimations)));
  if (const auto reject = to_verify_block_reject(gas.reject_code); reject.has_value()) {
    return {*reject, {}};
  }

  return {VerifyBlockReturnType::Verified, std::move(all_block_trxs)};
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

  try {
    {
      std::shared_lock graph_lock(rust_graphs_mutex_);
      if (period != rust_graphs_->runtime->dag_manager_runtime_latest_period() + 1) {
        return 0;
      }
    }

    rustaxa::DagManagerFinalizationApplyPayload finalized;
    {
      std::unique_lock graph_lock(rust_graphs_mutex_);
      if (period != rust_graphs_->runtime->dag_manager_runtime_latest_period() + 1) {
        return 0;
      }
      finalized = rust_graphs_->runtime->dag_manager_runtime_apply_finalized_order(to_bridge_hash(anchor), period,
                                                                                   to_bridge_dag_hashes(dag_order));
    }

    const auto finalized_count = finalized.finalized_count;
    for (const auto &bridge_hash : finalized.expired_hashes) {
      seen_blocks_.erase(from_bridge_dag_hash(bridge_hash));
    }

    const auto transactions_to_remove = from_bridge_dag_transaction_hashes(finalized.remove_transaction_hashes);
    if (!transactions_to_remove.empty()) {
      std::unordered_set<trx_hash_t> transactions_to_remove_set;
      transactions_to_remove_set.reserve(transactions_to_remove.size());
      for (const auto &hash : transactions_to_remove) {
        transactions_to_remove_set.emplace(hash);
      }
      trx_mgr_->removeNonFinalizedTransactions(std::move(transactions_to_remove_set));
    }

    return static_cast<uint>(finalized_count);
  } catch (const std::exception &e) {
    throw std::runtime_error(std::string("DagManager: failed to apply finalized DAG order in Rust runtime: ") +
                             e.what());
  }
}

rustaxa::PbftFinalizationLiveMutationReport DagManager::setDagBlockOrderForPbftFinalization(
    blk_hash_t const &anchor, PbftPeriod period, vec_blk_t const &dag_order,
    const rustaxa::PbftFinalizationStorageWritePlan &write_intent) {
  const auto finalized_count = setDagBlockOrder(anchor, period, dag_order);

  rustaxa::PbftFinalizationLiveMutationReport report{};
  report.action = kPbftFinalizationRuntimeActionSetDagBlockOrder;
  report.block_period = write_intent.block_period;
  report.pbft_block_hash = write_intent.pbft_block_hash;
  report.anchor_hash = write_intent.anchor_hash;
  report.dag_finalized_count = finalized_count;
  return report;
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
  rustaxa::DagManagerNonFinalizedSyncPayload payload;
  {
    std::shared_lock lock(rust_graphs_mutex_);
    payload = rust_graphs_->runtime->dag_manager_runtime_non_finalized_sync_payload(to_bridge_dag_hashes(known_hashes));
  }

  auto dag_blocks = from_bridge_dag_sync_blocks(payload.blocks);
  auto trxs = from_bridge_dag_transaction_rlps(payload.transactions);
  return {payload.period, std::move(dag_blocks), std::move(trxs)};
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

SortitionParamsManager &DagManager::sortitionParamsManager() { return sortition_params_manager_; }

const DagConfig &DagManager::getDagConfig() const { return genesis_config_.dag; }

uint64_t DagManager::getDagExpiryLevel() const {
  std::shared_lock lock(rust_graphs_mutex_);
  return rust_graphs_->runtime->dag_manager_runtime_dag_expiry_level();
}

uint64_t DagManager::getMaxLevelsPerPeriod() const { return max_levels_per_period_; }

std::optional<PbftPeriod> DagManager::getProposalPeriodForDagLevel(level_t level) const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto lookup = rust_graphs_->runtime->dag_manager_runtime_proposal_period_for_level(level);
  if (!lookup.found) {
    return std::nullopt;
  }
  return lookup.period;
}

blk_hash_t DagManager::getPeriodBlockHashForDagProposal(PbftPeriod period) const {
  std::shared_lock lock(rust_graphs_mutex_);
  const auto lookup = rust_graphs_->runtime->dag_manager_runtime_period_block_hash(period);
  if (!lookup.found) {
    return {};
  }
  return from_bridge_hash(lookup.hash);
}

dev::bytes DagManager::getVdfMessage(blk_hash_t const &hash, SharedTransactions const &trxs) {
  std::vector<trx_hash_t> trx_hashes;
  trx_hashes.reserve(trxs.size());
  for (const auto &trx : trxs) {
    trx_hashes.emplace_back(trx->getHash());
  }
  return rust_vdf_message(hash, trx_hashes);
}

dev::bytes DagManager::getVdfMessage(blk_hash_t const &hash, std::vector<trx_hash_t> const &trx_hashes) {
  return rust_vdf_message(hash, trx_hashes);
}

}  // namespace taraxa
