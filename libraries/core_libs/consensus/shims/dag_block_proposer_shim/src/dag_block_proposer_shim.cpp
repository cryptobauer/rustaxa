#include "dag/dag_block_proposer.hpp"

#include <algorithm>
#include <chrono>
#include <future>
#include <memory>
#include <numeric>
#include <unordered_map>
#include <unordered_set>
#include <utility>

#include "common/util.hpp"
#include "config/config.hpp"
#include "dag/dag_manager.hpp"
#include "final_chain/final_chain.hpp"
#include "key_manager/key_manager.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kDagProposerActionContinue = 1;
constexpr uint8_t kDagProposerActionRetryLater = 3;
constexpr uint32_t kDagProposerReasonVrfKeyMismatch = 3;
constexpr uint32_t kDagProposerReasonZeroDenominator = 6;

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t& hash) { return hash.asArray(); }

blk_hash_t from_bridge_hash(const std::array<uint8_t, 32>& hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

dev::bytes to_bytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

dev::bytes dag_vrf_input(level_t level, const blk_hash_t& proposal_period_hash) {
  auto bytes = rustaxa::dag_vrf_input(level, to_bridge_hash(proposal_period_hash));
  return to_bytes(bytes);
}

}  // namespace

using namespace vdf_sortition;

DagBlockProposer::DagBlockProposer(const FullNodeConfig& config, std::shared_ptr<DagManager> dag_mgr,
                                   std::shared_ptr<TransactionManager> trx_mgr,
                                   std::shared_ptr<final_chain::FinalChain> final_chain,
                                   std::shared_ptr<KeyManager> key_manager)
    : executor_(config.wallets.size()),
      total_trx_shards_(std::max(config.genesis.dag.block_proposer.shard, uint16_t(1))),
      dag_mgr_(std::move(dag_mgr)),
      trx_mgr_(std::move(trx_mgr)),
      final_chain_(std::move(final_chain)),
      nodes_dag_proposers_data_(),
      kDagProposeGasLimit(
          std::min(config.propose_dag_gas_limit, config.genesis.getGasLimits(final_chain_->lastBlockNumber()).first)),
      kPbftGasLimit(config.genesis.getGasLimits(final_chain_->lastBlockNumber()).second),
      kDagGasLimit(config.genesis.getGasLimits(final_chain_->lastBlockNumber()).first) {
  (void)key_manager;
  const auto& node_addr = dev::toAddress(config.getFirstWallet().node_secret);
  LOG_OBJECTS_CREATE("DAG_PROPOSER");

  for (const auto& wallet : config.wallets) {
    nodes_dag_proposers_data_.emplace_back(
        std::make_shared<NodeDagProposerData>(wallet, max_num_tries_, total_trx_shards_));
  }
}

bool DagBlockProposer::proposeDagBlock(const std::shared_ptr<NodeDagProposerData>& node_dag_proposer_data) {
  if (trx_mgr_->getTransactionPoolSize() == 0) {
    return false;
  }

  if (trx_mgr_->getNonfinalizedTrxSize() > kMaxNonFinalizedTransactions) {
    return false;
  }

  auto frontier = dag_mgr_->getDagFrontier();
  LOG(log_dg_) << "Get frontier with pivot: " << frontier.pivot << " tips: " << frontier.tips;
  assert(!frontier.pivot.isZero());
  const auto propose_level = getProposeLevel(frontier.pivot, frontier.tips) + 1;

  const auto proposal_period = dag_mgr_->getProposalPeriodForDagLevel(propose_level);
  if (!proposal_period.has_value()) {
    LOG(log_wr_) << "No proposal period for propose_level " << propose_level << " found";
    return false;
  }

  if (*proposal_period + kDagExpiryLevelLimit < final_chain_->lastBlockNumber()) {
    LOG(log_wr_) << "Trying to propose old block " << propose_level;
  }

  if (!hasDposSnapshotForProposal(*proposal_period)) {
    return false;
  }

  const auto authorization_facts =
      final_chain_->dagDposAuthorizationFacts(*proposal_period, node_dag_proposer_data->wallet.node_addr);
  rustaxa::DagProposerEligibilityInput eligibility_input;
  eligibility_input.proposal_period_found = true;
  eligibility_input.wallet_vrf_public_key = node_dag_proposer_data->wallet.vrf_pk.asArray();
  eligibility_input.authorization_facts = authorization_facts;
  const auto eligibility = rustaxa::dag_proposer_check_eligibility(std::move(eligibility_input));
  if (eligibility.action != kDagProposerActionContinue) {
    if (eligibility.reason_code == kDagProposerReasonVrfKeyMismatch) {
      LOG(log_er_) << "VRF public key mismatch for DAG proposer " << node_dag_proposer_data->wallet.node_addr;
    } else if (eligibility.reason_code == kDagProposerReasonZeroDenominator) {
      LOG(log_er_) << node_dag_proposer_data->wallet.node_addr
                   << " total vote count 0 at proposal period: " << *proposal_period;
    }
    if (eligibility.action == kDagProposerActionRetryLater) {
      LOG(log_wr_) << "DAG proposer eligibility facts unavailable at proposal period " << *proposal_period;
    }
    return false;
  }

  const auto vote_count = eligibility.vote_count;
  const auto max_vote_count = eligibility.max_vote_count;
  if (max_vote_count == 0) {
    LOG(log_er_) << node_dag_proposer_data->wallet.node_addr
                 << " total vote count 0 at proposal period: " << *proposal_period;
    return false;
  }

  const auto period_block_hash = dag_mgr_->getPeriodBlockHashForDagProposal(*proposal_period);
  const auto sortition_params = dag_mgr_->sortitionParamsManager().getSortitionParams(*proposal_period);
  vdf_sortition::VdfSortition vdf(sortition_params, node_dag_proposer_data->wallet.vrf_secret,
                                  dag_vrf_input(propose_level, period_block_hash), vote_count, max_vote_count);

  auto anchor = dag_mgr_->getAnchors().second;
  if (frontier.pivot != anchor) {
    if (dag_mgr_->getNonFinalizedBlocksSize().second > kMaxNonFinalizedDagBlocks) {
      return false;
    }
    if (dag_mgr_->getNonFinalizedBlocksMinDifficulty() < vdf.getDifficulty() &&
        dag_mgr_->getNonFinalizedBlocksSize().second > kMaxNonFinalizedDagBlocksLowDifficulty) {
      return false;
    }
  }

  if (vdf.isStale(sortition_params)) {
    if (node_dag_proposer_data->last_propose_level == propose_level) {
      if (node_dag_proposer_data->num_tries < node_dag_proposer_data->max_num_tries) {
        LOG(log_dg_) << node_dag_proposer_data->wallet.node_addr
                     << " will not propose DAG block. Get difficulty at stale, tried "
                     << node_dag_proposer_data->num_tries << " times.";
        node_dag_proposer_data->num_tries++;
        return false;
      }
    } else {
      LOG(log_dg_)
          << node_dag_proposer_data->wallet.node_addr
          << " will not propose DAG block, will reset number of tries. Get difficulty at stale, current propose level "
          << propose_level;
      node_dag_proposer_data->last_propose_level = propose_level;
      node_dag_proposer_data->num_tries = 0;
      return false;
    }
  }

  auto [transactions, estimations] =
      getShardedTrxs(*proposal_period, kDagProposeGasLimit, node_dag_proposer_data->trx_shard);
  if (transactions.empty()) {
    node_dag_proposer_data->last_propose_level = propose_level;
    node_dag_proposer_data->num_tries = 0;
    return false;
  }

  dev::bytes vdf_msg = DagManager::getVdfMessage(frontier.pivot, transactions);

  std::atomic_bool cancellation_token = false;
  std::promise<void> sync;
  executor_.post([&vdf, &sortition_params, &vdf_msg, cancel = std::ref(cancellation_token), &sync]() mutable {
    vdf.computeVdfSolution(sortition_params, vdf_msg, cancel);
    sync.set_value();
  });

  std::future<void> result = sync.get_future();
  while (result.wait_for(std::chrono::milliseconds(100)) != std::future_status::ready) {
    auto latest_frontier = dag_mgr_->getDagFrontier();
    const auto latest_level = getProposeLevel(latest_frontier.pivot, latest_frontier.tips) + 1;
    if (latest_level > propose_level + 1 && vdf.getDifficulty() > sortition_params.vdf.difficulty_min) {
      cancellation_token = true;
      break;
    }
  }

  if (cancellation_token) {
    node_dag_proposer_data->last_propose_level = propose_level;
    node_dag_proposer_data->num_tries = 0;
    result.wait();
    return true;
  }

  if (vdf.isStale(sortition_params)) {
    thisThreadSleepForSeconds(1);
    auto latest_frontier = dag_mgr_->getDagFrontier();
    const auto latest_level = getProposeLevel(latest_frontier.pivot, latest_frontier.tips) + 1;
    if (latest_level > propose_level) {
      node_dag_proposer_data->last_propose_level = propose_level;
      node_dag_proposer_data->num_tries = 0;
      return false;
    }
  }

  LOG(log_dg_) << node_dag_proposer_data->wallet.node_addr << " VDF computation time " << vdf.getComputationTime()
               << " difficulty " << vdf.getDifficulty();

  auto dag_block = createDagBlock(std::move(frontier), propose_level, transactions, std::move(estimations),
                                  std::move(vdf), node_dag_proposer_data->wallet.node_secret);

  if (dag_mgr_->addDagBlock(dag_block, std::move(transactions), true).first) {
    LOG(log_nf_) << node_dag_proposer_data->wallet.node_addr << " proposed new DAG block " << dag_block->getHash()
                 << ", pivot " << dag_block->getPivot() << ", txs num " << dag_block->getTrxs().size();
    proposed_blocks_count_ += 1;
  } else {
    LOG(log_er_) << "Failed to add newly proposed dag block " << dag_block->getHash() << ", proposed by "
                 << node_dag_proposer_data->wallet.node_addr << " into dag";
  }

  node_dag_proposer_data->last_propose_level = propose_level;
  node_dag_proposer_data->num_tries = 0;

  return true;
}

void DagBlockProposer::start() {
  if (bool b = true; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }
  const uint16_t min_proposal_delay = 100;

  LOG(log_nf_) << "DagBlockProposer started ...";

  proposed_blocks_count_ = 0;

  for (auto node_dag_proposer_data : nodes_dag_proposers_data_) {
    proposer_workers_.emplace_back(([this, node_dag_proposer_data]() {
      while (!stopped_) {
        auto syncing = false;
        auto packets_over_the_limit = false;
        if (auto net = network_.lock()) {
          syncing = net->pbft_syncing();
          packets_over_the_limit = net->packetQueueOverLimit();
        }
        if (syncing || packets_over_the_limit || !proposeDagBlock(node_dag_proposer_data)) {
          thisThreadSleepForMilliSeconds(min_proposal_delay);
        }
      }
    }));
  }
}

void DagBlockProposer::stop() {
  if (bool b = false; !stopped_.compare_exchange_strong(b, !b)) {
    return;
  }
  for (auto& proposer_worker : proposer_workers_) {
    if (proposer_worker.joinable()) {
      proposer_worker.join();
    }
  }

  LOG(log_nf_) << "DagBlockProposer stopped ...";
}

std::pair<SharedTransactions, std::vector<uint64_t>> DagBlockProposer::getShardedTrxs(
    PbftPeriod proposal_period, uint64_t weight_limit, const uint16_t node_trx_shard) const {
  auto syncing = false;
  if (auto net = network_.lock()) {
    syncing = net->pbft_syncing();
  }
  if (syncing) {
    return {};
  }

  if (total_trx_shards_ == 1) return trx_mgr_->packTrxs(proposal_period, weight_limit);

  auto [transactions, estimations] = trx_mgr_->packTrxs(proposal_period, weight_limit);

  if (transactions.empty()) {
    LOG(log_tr_) << "Skip block proposer, zero unpacked transactions ..." << std::endl;
    return {};
  }
  SharedTransactions sharded_trxs;
  std::vector<uint64_t> sharded_estimations;
  for (uint32_t i = 0; i < transactions.size(); i++) {
    auto shard = std::stoull(transactions[i]->getSender().toString().substr(0, 10), NULL, 16) +
                 proposal_period / kShardProposePeriodInterval;
    if (shard % total_trx_shards_ == node_trx_shard) {
      sharded_trxs.emplace_back(transactions[i]);
      sharded_estimations.emplace_back(estimations[i]);
    }
  }
  if (sharded_trxs.empty()) {
    LOG(log_tr_) << "Skip block proposer, zero sharded transactions ..." << std::endl;
    return {};
  }
  return {sharded_trxs, sharded_estimations};
}

level_t DagBlockProposer::getProposeLevel(blk_hash_t const& pivot, vec_blk_t const& tips) const {
  level_t max_level = 0;
  auto pivot_blk = dag_mgr_->getDagBlock(pivot);
  if (!pivot_blk) {
    LOG(log_er_) << "Cannot find pivot dag block " << pivot;
    return 0;
  }
  max_level = std::max(pivot_blk->getLevel(), max_level);

  for (auto const& t : tips) {
    auto tip_blk = dag_mgr_->getDagBlock(t);
    if (!tip_blk) {
      LOG(log_er_) << "Cannot find tip dag block " << t;
      return 0;
    }
    max_level = std::max(tip_blk->getLevel(), max_level);
  }
  return max_level;
}

vec_blk_t DagBlockProposer::selectDagBlockTips(const vec_blk_t& frontier_tips, uint64_t gas_limit) const {
  rust::Vec<rustaxa::DagProposerTipCandidate> candidates;
  candidates.reserve(frontier_tips.size());
  for (const auto& t : frontier_tips) {
    rustaxa::DagProposerTipCandidate candidate;
    candidate.hash = to_bridge_hash(t);
    candidate.sender = {};
    candidate.level = 0;
    candidate.gas_estimation = 0;
    auto tip_block = dag_mgr_->getDagBlock(t);
    if (tip_block == nullptr) {
      LOG(log_nf_) << "selectDagBlockTips, Cannot find tip dag block " << t;
      candidate.found = false;
    } else {
      candidate.found = true;
      candidate.sender = tip_block->getSender().asArray();
      candidate.level = tip_block->getLevel();
      candidate.gas_estimation = tip_block->getGasEstimation();
    }
    candidates.push_back(std::move(candidate));
  }

  const auto selection = rustaxa::dag_proposer_select_tips(std::move(candidates), gas_limit, kDagBlockMaxTips);
  vec_blk_t tips;
  tips.reserve(selection.selected.size());
  for (const auto& hash : selection.selected) {
    tips.emplace_back(from_bridge_hash(hash.hash));
  }
  return tips;
}

std::shared_ptr<DagBlock> DagBlockProposer::createDagBlock(DagFrontier&& frontier, level_t level,
                                                           const SharedTransactions& trxs,
                                                           std::vector<uint64_t>&& estimations, VdfSortition&& vdf,
                                                           const dev::Secret& node_secret) const {
  vec_trx_t trx_hashes;
  for (const auto& trx : trxs) {
    trx_hashes.push_back(trx->getHash());
  }

  const uint64_t block_estimation = std::accumulate(estimations.begin(), estimations.end(), uint64_t{0});

  if (frontier.tips.size() > kDagBlockMaxTips || (frontier.tips.size() + 1) > kPbftGasLimit / kDagGasLimit) {
    frontier.tips = selectDagBlockTips(frontier.tips, kPbftGasLimit - block_estimation);
  }

  return std::make_shared<DagBlock>(frontier.pivot, std::move(level), std::move(frontier.tips), std::move(trx_hashes),
                                    block_estimation, std::move(vdf), node_secret);
}

bool DagBlockProposer::hasDposSnapshotForProposal(PbftPeriod propose_period) const {
  if (final_chain_->lastBlockNumber() < propose_period) {
    LOG(log_wr_) << "Last finalized block period " << final_chain_->lastBlockNumber() << " < propose_period "
                 << propose_period;
    return false;
  }
  return true;
}

void DagBlockProposer::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa
