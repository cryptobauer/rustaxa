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
constexpr uint32_t kDagProposerReasonMissingProposalPeriod = 1;
constexpr uint32_t kDagProposerReasonVrfKeyMismatch = 3;
constexpr uint32_t kDagProposerReasonZeroDenominator = 6;
constexpr uint32_t kDagProposerReasonFinalizedPeriodNotReady = 9;
constexpr uint32_t kDagProposerReasonPackedTransactionsEmpty = 14;

std::array<uint8_t, 32> to_bridge_hash(const blk_hash_t& hash) { return hash.asArray(); }

rustaxa::DagHash to_bridge_dag_hash(const blk_hash_t& hash) { return rustaxa::DagHash{to_bridge_hash(hash)}; }

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

SharedTransactions materialize_transactions(const vec_trx_t& transaction_hashes,
                                            const std::vector<dev::bytes>& transaction_rlps) {
  if (transaction_hashes.size() != transaction_rlps.size()) {
    throw std::runtime_error("Rust DAG proposer transaction payload lengths do not match");
  }

  SharedTransactions transactions;
  transactions.reserve(transaction_rlps.size());
  for (size_t idx = 0; idx < transaction_rlps.size(); ++idx) {
    auto transaction = std::make_shared<Transaction>(transaction_rlps[idx]);
    if (transaction->getHash() != transaction_hashes[idx]) {
      throw std::runtime_error("Rust DAG proposer transaction payload hash mismatch");
    }
    transactions.push_back(std::move(transaction));
  }
  return transactions;
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

rustaxa::DagProposerRetryResetPlan plan_retry_reset(uint64_t propose_level) {
  rustaxa::DagProposerRetryResetInput input;
  input.proposal_level = propose_level;
  return rustaxa::dag_proposer_plan_retry_reset(std::move(input));
}

rustaxa::DagProposerVdfWaitPlan plan_vdf_wait(uint64_t propose_level, uint64_t latest_level, uint16_t vdf_difficulty,
                                              uint16_t minimum_vdf_difficulty) {
  rustaxa::DagProposerVdfWaitInput input;
  input.proposal_level = propose_level;
  input.latest_proposal_level = latest_level;
  input.vdf_difficulty = vdf_difficulty;
  input.minimum_vdf_difficulty = minimum_vdf_difficulty;
  return rustaxa::dag_proposer_plan_vdf_wait(std::move(input));
}

rustaxa::DagProposerStaleProofPlan plan_stale_proof(uint64_t propose_level, uint64_t latest_level) {
  rustaxa::DagProposerStaleProofInput input;
  input.proposal_level = propose_level;
  input.latest_proposal_level = latest_level;
  return rustaxa::dag_proposer_plan_stale_proof(std::move(input));
}

void apply_retry_reset(const rustaxa::DagProposerRetryResetPlan& plan,
                       const std::shared_ptr<DagBlockProposer::NodeDagProposerData>& node_dag_proposer_data) {
  if (!plan.update_retry_state) {
    return;
  }
  node_dag_proposer_data->last_propose_level = plan.next_last_propose_level;
  node_dag_proposer_data->num_tries = static_cast<uint16_t>(plan.next_retry_count);
}

void apply_retry_reset(const rustaxa::DagProposerStaleProofPlan& plan,
                       const std::shared_ptr<DagBlockProposer::NodeDagProposerData>& node_dag_proposer_data) {
  if (!plan.update_retry_state) {
    return;
  }
  node_dag_proposer_data->last_propose_level = plan.next_last_propose_level;
  node_dag_proposer_data->num_tries = static_cast<uint16_t>(plan.next_retry_count);
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
  const auto frontier_facts = dag_mgr_->getProposerFrontierFacts();
  const auto proposal_period = dag_mgr_->getProposalPeriodForDagLevel(frontier_facts.propose_level);
  const auto last_finalized_period = rust_final_chain_last_block_number(*final_chain_);
  rustaxa::DagDposAuthorizationFacts authorization_facts{};
  rustaxa::SortitionRuntimeParams sortition_params{};
  if (proposal_period.has_value() && last_finalized_period >= *proposal_period) {
    authorization_facts =
        rust_dag_authorization_facts(*final_chain_, *proposal_period, node_dag_proposer_data->wallet.node_addr);
    sortition_params = dag_mgr_->sortitionParamsManager().rustSortitionParamsForRust(*proposal_period);
  }

  rustaxa::DagProposerAttemptInput attempt_input;
  attempt_input.transaction_pool_size = trx_mgr_->getTransactionPoolSize();
  attempt_input.non_finalized_transaction_count = trx_mgr_->getNonfinalizedTrxSize();
  attempt_input.max_non_finalized_transactions = kMaxNonFinalizedTransactions;
  attempt_input.frontier_facts = frontier_facts;
  attempt_input.proposal_period_found = proposal_period.has_value();
  attempt_input.proposal_period = proposal_period.value_or(0);
  attempt_input.last_finalized_period = last_finalized_period;
  attempt_input.dag_expiry_level_limit = kDagExpiryLevelLimit;
  attempt_input.wallet_vrf_public_key = node_dag_proposer_data->wallet.vrf_pk.asArray();
  attempt_input.wallet_vrf_secret = node_dag_proposer_data->wallet.vrf_secret.asArray();
  attempt_input.authorization_facts = authorization_facts;
  attempt_input.sortition_params = sortition_params;
  attempt_input.max_non_finalized_dag_blocks = kMaxNonFinalizedDagBlocks;
  attempt_input.max_non_finalized_dag_blocks_low_difficulty = kMaxNonFinalizedDagBlocksLowDifficulty;
  attempt_input.last_propose_level = node_dag_proposer_data->last_propose_level;
  attempt_input.retry_count = node_dag_proposer_data->num_tries;
  attempt_input.max_retry_count = node_dag_proposer_data->max_num_tries;
  attempt_input.proposal_weight_limit = kDagProposeGasLimit;
  attempt_input.total_transaction_shards = total_trx_shards_;
  attempt_input.node_transaction_shard = node_dag_proposer_data->trx_shard;
  attempt_input.shard_period_interval = kShardProposePeriodInterval;

  const auto attempt = dag_mgr_->planProposerAttempt(std::move(attempt_input));
  DagFrontier frontier(from_bridge_hash(attempt.frontier_pivot), from_bridge_dag_hashes(attempt.frontier_tips));
  LOG(log_dg_) << "Get frontier with pivot: " << frontier.pivot << " tips: " << frontier.tips;
  assert(!frontier.pivot.isZero());
  const auto propose_level = attempt.proposal_level;
  if (attempt.old_proposal) {
    LOG(log_wr_) << "Trying to propose old block " << propose_level;
  }

  if (attempt.action != kDagProposerActionContinue) {
    if (attempt.update_retry_state) {
      node_dag_proposer_data->last_propose_level = attempt.next_last_propose_level;
      node_dag_proposer_data->num_tries = static_cast<uint16_t>(attempt.next_retry_count);
    }
    if (attempt.reason_code == kDagProposerReasonMissingProposalPeriod) {
      LOG(log_wr_) << "No proposal period for propose_level " << propose_level << " found";
    } else if (attempt.reason_code == kDagProposerReasonFinalizedPeriodNotReady) {
      LOG(log_wr_) << "Last finalized block period " << attempt.last_finalized_period << " < propose_period "
                   << attempt.proposal_period;
    } else if (attempt.reason_code == kDagProposerReasonVrfKeyMismatch) {
      LOG(log_er_) << "VRF public key mismatch for DAG proposer " << node_dag_proposer_data->wallet.node_addr;
    } else if (attempt.reason_code == kDagProposerReasonZeroDenominator) {
      LOG(log_er_) << node_dag_proposer_data->wallet.node_addr
                   << " total vote count 0 at proposal period: " << attempt.proposal_period;
    }
    if (attempt.action == kDagProposerActionRetryLater) {
      LOG(log_wr_) << "DAG proposer eligibility facts unavailable at proposal period " << attempt.proposal_period;
    }
    return false;
  }

  const auto vote_count = attempt.vote_count;
  const auto max_vote_count = attempt.max_vote_count;
  if (max_vote_count == 0) {
    LOG(log_er_) << node_dag_proposer_data->wallet.node_addr
                 << " total vote count 0 at proposal period: " << attempt.proposal_period;
    return false;
  }

  const auto vrf_input = to_bytes(attempt.vrf_input);
  const auto vdf_difficulty = attempt.vdf_difficulty;
  const bool vdf_stale = attempt.vdf_stale;

  auto transaction_payloads = getShardedTrxs(
      attempt.transaction_request.proposal_period, attempt.transaction_request.weight_limit,
      attempt.transaction_request.total_transaction_shards, attempt.transaction_request.node_transaction_shard,
      attempt.transaction_request.shard_period_interval);
  rustaxa::DagProposerPostPackInput post_pack_input;
  post_pack_input.proposal_level = propose_level;
  post_pack_input.packed_transaction_count = transaction_payloads.transaction_hashes.size();
  const auto post_pack = rustaxa::dag_proposer_plan_post_pack(std::move(post_pack_input));
  if (post_pack.action != kDagProposerActionContinue) {
    if (post_pack.update_retry_state) {
      node_dag_proposer_data->last_propose_level = post_pack.next_last_propose_level;
      node_dag_proposer_data->num_tries = static_cast<uint16_t>(post_pack.next_retry_count);
    }
    if (post_pack.reason_code == kDagProposerReasonPackedTransactionsEmpty) {
      LOG(log_tr_) << "Skip block proposer, zero sharded transactions ..." << std::endl;
    }
    return false;
  }

  dev::bytes vdf_msg = DagManager::getVdfMessage(frontier.pivot, transaction_payloads.transaction_hashes);

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
    const auto wait_plan = plan_vdf_wait(propose_level, latest_level, vdf_difficulty, sortition_params.difficulty_min);
    if (wait_plan.cancel_in_flight_proof) {
      cancellation_token = true;
      break;
    }
  }

  if (cancellation_token) {
    apply_retry_reset(plan_retry_reset(propose_level), node_dag_proposer_data);
    result.wait();
    return true;
  }

  if (vdf_stale) {
    thisThreadSleepForSeconds(1);
    const auto latest_level = dag_mgr_->getProposerFrontierFacts().propose_level;
    const auto stale_plan = plan_stale_proof(propose_level, latest_level);
    if (stale_plan.action != kDagProposerActionContinue) {
      apply_retry_reset(stale_plan, node_dag_proposer_data);
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

  auto dag_block = createDagBlock(std::move(frontier), propose_level, transaction_payloads.transaction_hashes,
                                  std::move(transaction_payloads.gas_estimations), std::move(vdf),
                                  node_dag_proposer_data->wallet.node_secret);

  auto transactions =
      materialize_transactions(transaction_payloads.transaction_hashes, transaction_payloads.transaction_rlps);
  if (dag_mgr_->addDagBlock(dag_block, std::move(transactions), true).first) {
    LOG(log_nf_) << node_dag_proposer_data->wallet.node_addr << " proposed new DAG block " << dag_block->getHash()
                 << ", pivot " << dag_block->getPivot() << ", txs num " << dag_block->getTrxs().size();
    proposed_blocks_count_ += 1;
  } else {
    LOG(log_er_) << "Failed to add newly proposed dag block " << dag_block->getHash() << ", proposed by "
                 << node_dag_proposer_data->wallet.node_addr << " into dag";
  }

  apply_retry_reset(plan_retry_reset(propose_level), node_dag_proposer_data);

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

DagBlockProposer::ShardedProposalTransactions DagBlockProposer::getShardedTrxs(PbftPeriod proposal_period,
                                                                               uint64_t weight_limit,
                                                                               const uint16_t total_trx_shards,
                                                                               const uint16_t node_trx_shard,
                                                                               uint64_t shard_period_interval) const {
  auto syncing = false;
  if (auto net = network_.lock()) {
    syncing = net->pbft_syncing();
  }
  if (syncing) {
    return {};
  }

  auto payloads = trx_mgr_->packShardedTransactionPayloads(proposal_period, weight_limit, total_trx_shards,
                                                           node_trx_shard, shard_period_interval);
  return {std::move(payloads.transaction_hashes), std::move(payloads.transaction_rlps),
          std::move(payloads.gas_estimations)};
}

std::shared_ptr<DagBlock> DagBlockProposer::createDagBlock(DagFrontier&& frontier, level_t level,
                                                           const vec_trx_t& trx_hashes,
                                                           std::vector<uint64_t>&& estimations, VdfSortition&& vdf,
                                                           const dev::Secret& node_secret) const {
  rustaxa::DagProposerStorageBlockConstructionInput plan_input;
  plan_input.pbft_gas_limit = kPbftGasLimit;
  plan_input.dag_gas_limit = kDagGasLimit;
  plan_input.max_tips = kDagBlockMaxTips;
  plan_input.frontier_tips.reserve(frontier.tips.size());
  for (const auto& t : frontier.tips) {
    plan_input.frontier_tips.push_back(to_bridge_dag_hash(t));
  }

  plan_input.transaction_gas_estimations.reserve(estimations.size());
  for (const auto estimation : estimations) {
    plan_input.transaction_gas_estimations.push_back(estimation);
  }

  const auto plan = dag_mgr_->planProposerBlockConstruction(std::move(plan_input));
  frontier.tips.clear();
  frontier.tips.reserve(plan.selected_tips.size());
  for (const auto& hash : plan.selected_tips) {
    frontier.tips.emplace_back(from_bridge_hash(hash.hash));
  }

  return std::make_shared<DagBlock>(frontier.pivot, std::move(level), std::move(frontier.tips), trx_hashes,
                                    plan.block_gas_estimation, std::move(vdf), node_secret);
}

void DagBlockProposer::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

}  // namespace taraxa
