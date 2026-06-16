#include <algorithm>
#include <chrono>
#include <future>
#include <memory>
#include <optional>
#include <unordered_map>
#include <unordered_set>
#include <utility>

#include "common/util.hpp"
#include "config/config.hpp"
#include "dag/dag_block_proposer.hpp"
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

blk_hash_t from_bridge_dag_hash(const rustaxa::DagHash& hash) { return from_bridge_hash(hash.hash); }

std::vector<blk_hash_t> from_bridge_dag_hashes(const rust::Vec<rustaxa::DagHash>& hashes) {
  std::vector<blk_hash_t> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.emplace_back(from_bridge_dag_hash(hash));
  }
  return out;
}

dev::bytes to_bytes(const rust::Vec<uint8_t>& bytes) { return dev::bytes(bytes.begin(), bytes.end()); }

dev::bytes dag_vrf_input(level_t level, const blk_hash_t& proposal_period_hash) {
  auto bytes = rustaxa::dag_vrf_input(level, to_bridge_hash(proposal_period_hash));
  return to_bytes(bytes);
}

rustaxa::LegacySortitionParams to_legacy_sortition_params(const rustaxa::SortitionRuntimeParams& params) {
  rustaxa::LegacySortitionParams out;
  out.vrf_threshold_upper = params.threshold_upper;
  out.vdf_difficulty_min = params.difficulty_min;
  out.vdf_difficulty_max = params.difficulty_max;
  out.vdf_difficulty_stale = params.difficulty_stale;
  out.vdf_lambda_bound = params.lambda_bound;
  return out;
}

rustaxa::VdfSortitionVerifyConfig to_vdf_sortition_config(const rustaxa::SortitionRuntimeParams& params) {
  rustaxa::VdfSortitionVerifyConfig out;
  out.threshold_upper = params.threshold_upper;
  out.difficulty_min = params.difficulty_min;
  out.difficulty_max = params.difficulty_max;
  out.difficulty_stale = params.difficulty_stale;
  out.lambda_bound = params.lambda_bound;
  return out;
}

vdf_sortition::VdfSortition vdf_sortition_from_proof(const rustaxa::VdfSortitionProofResult& proof) {
  rustaxa::VdfSortitionPayload payload;
  payload.vrf_proof = proof.vrf_proof;
  payload.vdf_solution_proof = proof.vdf_proof;
  payload.vdf_solution_output = proof.vdf_output;
  payload.difficulty = proof.difficulty;
  return vdf_sortition::VdfSortition(to_bytes(rustaxa::vdf_sortition_payload_encode(payload)));
}

uint64_t rust_final_chain_last_block_number(const final_chain::FinalChain& final_chain) {
  rustaxa::PbftFinalChainFactRequest request;
  request.period = 0;
  request.candidate_final_chain_hash = {};
  request.collect_final_chain_hash = false;
  request.validate_candidate_final_chain_hash = false;
  request.collect_total_vote_count = false;
  request.collect_address_vote_counts = false;
  const auto facts = final_chain.rustFinalChainForRust().collect_pbft_final_chain_facts(std::move(request));
  return facts.last_block_number;
}

rustaxa::DagDposAuthorizationFacts rust_dag_authorization_facts(const final_chain::FinalChain& final_chain,
                                                                PbftPeriod proposal_period, const addr_t& proposer) {
  return final_chain.rustFinalChainForRust().get_dag_dpos_authorization_facts(static_cast<uint64_t>(proposal_period),
                                                                              proposer.asArray());
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
          std::min(config.propose_dag_gas_limit,
                   config.genesis.getGasLimits(rust_final_chain_last_block_number(*final_chain_)).first)),
      kPbftGasLimit(config.genesis.getGasLimits(rust_final_chain_last_block_number(*final_chain_)).second),
      kDagGasLimit(config.genesis.getGasLimits(rust_final_chain_last_block_number(*final_chain_)).first) {
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

  const auto frontier_facts = dag_mgr_->getProposerFrontierFacts();
  DagFrontier frontier(from_bridge_hash(frontier_facts.pivot), from_bridge_dag_hashes(frontier_facts.tips));
  LOG(log_dg_) << "Get frontier with pivot: " << frontier.pivot << " tips: " << frontier.tips;
  assert(!frontier.pivot.isZero());
  const auto propose_level = frontier_facts.propose_level;

  const auto proposal_period = dag_mgr_->getProposalPeriodForDagLevel(propose_level);
  if (!proposal_period.has_value()) {
    LOG(log_wr_) << "No proposal period for propose_level " << propose_level << " found";
    return false;
  }

  if (*proposal_period + kDagExpiryLevelLimit < rust_final_chain_last_block_number(*final_chain_)) {
    LOG(log_wr_) << "Trying to propose old block " << propose_level;
  }

  if (!hasDposSnapshotForProposal(*proposal_period)) {
    return false;
  }

  const auto authorization_facts =
      rust_dag_authorization_facts(*final_chain_, *proposal_period, node_dag_proposer_data->wallet.node_addr);
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
  const auto sortition_params = dag_mgr_->sortitionParamsManager().rustSortitionParamsForRust(*proposal_period);
  const auto vrf_input = dag_vrf_input(propose_level, period_block_hash);
  rust::Slice<const uint8_t> vrf_input_slice{vrf_input.data(), vrf_input.size()};
  const auto normalized_vote_count = rustaxa::vdf_sortition_normalize_vote_count(vote_count, max_vote_count);
  const auto vrf_probe =
      rustaxa::prove_legacy_vrf_sortition(node_dag_proposer_data->wallet.vrf_secret.asArray(), vrf_input_slice,
                                          normalized_vote_count);
  if (!vrf_probe.ok) {
    throw vdf_sortition::VdfSortition::InvalidVdfSortition(
        "Rust DAG proposer VRF probe failed. status " + std::to_string(vrf_probe.status) + ": " +
        std::string(vrf_probe.error));
  }
  const auto vdf_difficulty = rustaxa::vdf_sortition_difficulty(to_vdf_sortition_config(sortition_params),
                                                               vrf_probe.threshold);
  const bool vdf_stale = vdf_difficulty == sortition_params.difficulty_stale;

  const auto anchor = from_bridge_hash(frontier_facts.anchor);
  if (frontier.pivot != anchor) {
    if (frontier_facts.non_finalized_block_count > kMaxNonFinalizedDagBlocks) {
      return false;
    }
    if (frontier_facts.non_finalized_min_difficulty < vdf_difficulty &&
        frontier_facts.non_finalized_block_count > kMaxNonFinalizedDagBlocksLowDifficulty) {
      return false;
    }
  }

  if (vdf_stale) {
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
  auto rust_cancellation_token =
      rustaxa::make_cancellation_token_with_atomic(reinterpret_cast<const bool*>(&cancellation_token));
  std::optional<rustaxa::VdfSortitionProofResult> proof_result;
  executor_.post([params = to_legacy_sortition_params(sortition_params),
                  secret = node_dag_proposer_data->wallet.vrf_secret.asArray(), &vrf_input, &vdf_msg,
                  &rust_cancellation_token, &proof_result, &sync, vote_count, max_vote_count]() mutable {
    rust::Slice<const uint8_t> vrf_input_slice{vrf_input.data(), vrf_input.size()};
    rust::Slice<const uint8_t> vdf_input_slice{vdf_msg.data(), vdf_msg.size()};
    proof_result.emplace(rustaxa::prove_legacy_vdf_sortition(params, secret, vrf_input_slice, vdf_input_slice,
                                                             vote_count, max_vote_count, *rust_cancellation_token));
    sync.set_value();
  });

  std::future<void> result = sync.get_future();
  while (result.wait_for(std::chrono::milliseconds(100)) != std::future_status::ready) {
    const auto latest_level = dag_mgr_->getProposerFrontierFacts().propose_level;
    if (latest_level > propose_level + 1 && vdf_difficulty > sortition_params.difficulty_min) {
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

  if (vdf_stale) {
    thisThreadSleepForSeconds(1);
    const auto latest_level = dag_mgr_->getProposerFrontierFacts().propose_level;
    if (latest_level > propose_level) {
      node_dag_proposer_data->last_propose_level = propose_level;
      node_dag_proposer_data->num_tries = 0;
      return false;
    }
  }

  if (!proof_result.has_value() || !proof_result->ok) {
    throw vdf_sortition::VdfSortition::InvalidVdfSortition(
        "Rust DAG proposer VDF proof failed. status " +
        std::to_string(proof_result.has_value() ? proof_result->status : 0) + ": " +
        (proof_result.has_value() ? std::string(proof_result->error) : std::string("missing proof result")));
  }

  auto vdf = vdf_sortition_from_proof(*proof_result);
  LOG(log_dg_) << node_dag_proposer_data->wallet.node_addr << " VDF difficulty " << vdf.getDifficulty();

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

  auto [transactions, estimations] =
      trx_mgr_->packShardedTrxs(proposal_period, weight_limit, total_trx_shards_, node_trx_shard,
                                kShardProposePeriodInterval);
  if (transactions.empty()) {
    LOG(log_tr_) << "Skip block proposer, zero sharded transactions ..." << std::endl;
    return {};
  }
  return {transactions, estimations};
}

std::shared_ptr<DagBlock> DagBlockProposer::createDagBlock(DagFrontier&& frontier, level_t level,
                                                           const SharedTransactions& trxs,
                                                           std::vector<uint64_t>&& estimations, VdfSortition&& vdf,
                                                           const dev::Secret& node_secret) const {
  vec_trx_t trx_hashes;
  for (const auto& trx : trxs) {
    trx_hashes.push_back(trx->getHash());
  }

  rustaxa::DagProposerBlockConstructionInput plan_input;
  plan_input.pbft_gas_limit = kPbftGasLimit;
  plan_input.dag_gas_limit = kDagGasLimit;
  plan_input.max_tips = kDagBlockMaxTips;
  plan_input.frontier_tips.reserve(frontier.tips.size());
  for (const auto& t : frontier.tips) {
    rustaxa::DagProposerTipCandidate candidate;
    candidate.hash = to_bridge_hash(t);
    candidate.sender = {};
    candidate.level = 0;
    candidate.gas_estimation = 0;
    auto tip_block = dag_mgr_->getDagBlock(t);
    if (tip_block == nullptr) {
      candidate.found = false;
    } else {
      candidate.found = true;
      candidate.sender = tip_block->getSender().asArray();
      candidate.level = tip_block->getLevel();
      candidate.gas_estimation = tip_block->getGasEstimation();
    }
    plan_input.frontier_tips.push_back(std::move(candidate));
  }

  plan_input.transaction_gas_estimations.reserve(estimations.size());
  for (const auto estimation : estimations) {
    plan_input.transaction_gas_estimations.push_back(estimation);
  }

  const auto plan = rustaxa::dag_proposer_plan_block_construction(std::move(plan_input));
  frontier.tips.clear();
  frontier.tips.reserve(plan.selected_tips.size());
  for (const auto& hash : plan.selected_tips) {
    frontier.tips.emplace_back(from_bridge_hash(hash.hash));
  }

  return std::make_shared<DagBlock>(frontier.pivot, std::move(level), std::move(frontier.tips), std::move(trx_hashes),
                                    plan.block_gas_estimation, std::move(vdf), node_secret);
}

bool DagBlockProposer::hasDposSnapshotForProposal(PbftPeriod propose_period) const {
  const auto last_block_number = rust_final_chain_last_block_number(*final_chain_);
  if (last_block_number < propose_period) {
    LOG(log_wr_) << "Last finalized block period " << last_block_number << " < propose_period " << propose_period;
    return false;
  }
  return true;
}

void DagBlockProposer::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa
