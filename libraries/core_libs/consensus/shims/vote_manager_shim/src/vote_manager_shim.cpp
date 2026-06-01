#include <libdevcore/RLP.h>

#include <limits>
#include <mutex>
#include <sstream>
#include <stdexcept>

#include "common/constants.hpp"
#include "pbft/pbft_manager.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#include "slashing_manager/slashing_manager.hpp"
#include "storage/storage.hpp"
#include "vote/pbft_vote.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kPbftFinalizedPeriodApplyStatusApplied = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusAlreadyApplied = 1;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusRejected = 2;
constexpr uint8_t kPbftFinalizationStorageStageRewardVotesReset = 4;
constexpr uint8_t kPbftFinalizationRuntimeActionCommitRewardVotesReset = 3;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

rust::Vec<rustaxa::PbftFinalizationHash> toBridgeRewardVoteHashes(const std::vector<vote_hash_t>& hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{toBridgeHash(hash)});
  }
  return out;
}

rustaxa::PbftFinalizedPeriodApplyResult rewardResetResult(uint8_t status, PbftPeriod period,
                                                          const blk_hash_t& block_hash, const char* error_code) {
  rustaxa::PbftFinalizedPeriodApplyResult result{};
  result.status = status;
  result.block_period = period;
  result.pbft_block_hash = toBridgeHash(block_hash);
  result.error_code = rust::String(error_code);
  return result;
}

rustaxa::PbftFinalizationLiveMutationReport makeRewardVotesResetLiveReport(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, uint64_t extra_reward_votes_count) {
  rustaxa::PbftFinalizationLiveMutationReport report{};
  report.action = kPbftFinalizationRuntimeActionCommitRewardVotesReset;
  report.block_period = write_intent.block_period;
  report.pbft_block_hash = write_intent.pbft_block_hash;
  report.anchor_hash = write_intent.anchor_hash;
  report.reward_votes_period = write_intent.reward_vote_period;
  report.reward_votes_round = write_intent.reward_vote_round;
  report.reward_votes_block_hash = write_intent.reward_vote_block_hash;
  report.reward_votes_extra_count = extra_reward_votes_count;
  return report;
}

rustaxa::PbftFinalizationStorageWritePlan makeRewardResetWritePlan(PbftPeriod period, PbftRound round, PbftStep step,
                                                                   const blk_hash_t& block_hash) {
  rustaxa::PbftFinalizationStorageWritePlan write_plan{};
  write_plan.reset_reward_votes = true;
  write_plan.pbft_block_hash = toBridgeHash(block_hash);
  write_plan.block_period = period;
  write_plan.reward_vote_period = period;
  write_plan.reward_vote_round = round;
  write_plan.reward_vote_step = step;
  write_plan.reward_vote_block_hash = toBridgeHash(block_hash);
  return write_plan;
}

rustaxa::PbftFinalizationStorageWriteStage makeRewardResetWriteStage(
    const std::vector<std::shared_ptr<PbftVote>>& votes, const std::vector<vote_hash_t>& extra_reward_votes) {
  dev::RLPStream votes_stream(votes.size());
  for (const auto& vote : votes) {
    if (!vote) {
      throw std::runtime_error("VoteManager reward-vote reset cannot bridge a null cert vote");
    }
    votes_stream.appendRaw(vote->rlp(true, true));
  }

  rustaxa::PbftFinalizationStorageWriteStage write_stage{};
  write_stage.stage = kPbftFinalizationStorageStageRewardVotesReset;
  write_stage.has_reward_votes_reset = true;
  write_stage.reward_votes_bundle_rlp = toBridgeBytes(votes_stream.out());
  write_stage.extra_reward_vote_hashes = toBridgeRewardVoteHashes(extra_reward_votes);
  return write_stage;
}

rustaxa::VerifiedVotePayload toBridgeVerifiedVotePayload(const std::shared_ptr<PbftVote>& vote) {
  return rustaxa::VerifiedVotePayload{toBridgeHash(vote->getHash()), toBridgeHash(vote->getBlockHash()),
                                      toBridgeAddress(vote->getVoterAddr()), vote->getPeriod(),
                                      vote->getRound(), vote->getStep(), static_cast<uint8_t>(vote->getType()),
                                      *vote->getWeight()};
}

rustaxa::PbftVoteProgressFact makeVoteProgressFact(const std::shared_ptr<PbftVote>& vote,
                                                   bool valid_stale_reward_vote) {
  rustaxa::PbftVoteProgressFact fact{};
  fact.vote = toBridgeVerifiedVotePayload(vote);
  fact.vote_already_known = false;
  fact.carries_proposed_block = true;
  fact.valid_stale_reward_vote = valid_stale_reward_vote;
  return fact;
}

rustaxa::PbftVoteProgressContext makeVoteProgressContext(PbftPeriod current_period, PbftRound current_round,
                                                         std::optional<uint64_t> two_t_plus_one) {
  rustaxa::PbftVoteProgressContext context{};
  context.current_period = current_period;
  context.current_round = current_round;
  context.max_future_period_delta = std::numeric_limits<uint64_t>::max();
  context.has_two_t_plus_one_threshold = two_t_plus_one.has_value();
  context.two_t_plus_one_threshold = two_t_plus_one.value_or(0);
  context.require_proposed_block_sidecar = false;
  context.slashing_enabled = true;
  return context;
}

}  // namespace

void VoteManager::setNetwork(std::weak_ptr<Network> network) {
  // TODO(rustaxa): move VoteManager network wiring to Rust/shim-owned state.
  VoteManagerOld::setNetwork(std::move(network));
}

bool VoteManager::addVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  if (!vote || !vote->getWeight().has_value()) {
    LOG(log_er_) << "Unable to add vote into the verified queue. Missing vote or vote weight";
    return false;
  }

  const auto hash = vote->getHash();
  const auto weight = *vote->getWeight();
  if (!weight) {
    LOG(log_er_) << "Unable to add vote " << hash << " into the verified queue. Invalid vote weight";
    return false;
  }

  bool is_valid_potential_reward_vote = false;
  if (vote->getPeriod() < current_pbft_period_) {
    is_valid_potential_reward_vote = isValidRewardVoteForRust(vote);
    if (!is_valid_potential_reward_vote) {
      LOG(log_tr_) << "Old vote " << vote->getHash().abridged() << " vote period" << vote->getPeriod()
                   << " current period " << current_pbft_period_;
      return false;
    }
  }

  // TODO(rustaxa): move PBFT threshold calculation and cache ownership to Rust.
  const auto two_t_plus_one = VoteManagerOld::getPbftTwoTPlusOne(vote->getPeriod() - 1, vote->getType());
  const auto progress_fact = makeVoteProgressFact(vote, is_valid_potential_reward_vote);
  const auto progress_context =
      makeVoteProgressContext(current_pbft_period_, current_pbft_round_, two_t_plus_one);

  const auto precheck_plan = rustaxa::pbft_vote_progress_plan_precheck(progress_fact, progress_context);
  if (!precheck_plan.should_insert_verified_vote) {
    return false;
  }

  const auto add_outcome = verified_votes_.addVerifiedVoteWithThreshold(vote, two_t_plus_one);
  const auto execution_plan =
      rustaxa::pbft_vote_progress_plan_after_add(progress_fact, progress_context, add_outcome.report);

  if (execution_plan.report_slashing) {
    LOG(log_wr_) << "Non unique vote " << vote->getHash().abridged() << " (race condition)";
    if (!add_outcome.conflicting_vote) {
      throw std::runtime_error("VoteManager Rust vote-progress planner requested slashing without conflict sidecar");
    }
    slashing_manager_->submitDoubleVotingProof(vote, *add_outcome.conflicting_vote);
    return false;
  }

  if (!execution_plan.accepted) {
    return false;
  }

  const auto votes_with_weight = add_outcome.votes_with_weight;
  if (!votes_with_weight) {
    throw std::runtime_error("VoteManager Rust vote-progress planner accepted vote without inserted vote sidecars");
  }

  LOG(log_nf_) << "Added verified vote: " << hash;
  LOG(log_dg_) << "Added verified vote: " << *vote;

  if (execution_plan.persist_extra_reward_vote) {
    extra_reward_votes_.emplace_back(vote->getHash());
    db_->saveExtraRewardVote(vote);
  }

  if (!two_t_plus_one.has_value()) [[unlikely]] {
    LOG(log_er_) << "Cannot set(or not) 2t+1 voted block as 2t+1 threshold is unavailable, vote " << vote->getHash();
    return true;
  }

  if (execution_plan.network_t_plus_one_step_updated) {
    LOG(log_nf_) << "Set t+1 next voted block " << vote->getHash() << " for period " << vote->getPeriod() << ", round "
                 << vote->getRound() << ", step " << vote->getStep();
  }

  if (!execution_plan.persist_two_t_plus_one_votes) {
    return true;
  }

  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(votes_with_weight->votes.size());
  for (const auto& tmp_vote : votes_with_weight->votes) {
    votes.push_back(tmp_vote.second);
  }

  db_->replaceTwoTPlusOneVotes(static_cast<TwoTPlusOneVotedBlockType>(execution_plan.two_t_plus_one_kind), votes);
  return true;
}

bool VoteManager::voteInVerifiedMap(std::shared_ptr<PbftVote> const& vote) const {
  const auto step_votes_map = verified_votes_.getStepVotes(vote->getPeriod(), vote->getRound(), vote->getStep());
  if (!step_votes_map) {
    return false;
  }

  const auto found_voted_value_it = step_votes_map->votes.find(vote->getBlockHash());
  if (found_voted_value_it == step_votes_map->votes.end()) {
    return false;
  }

  return found_voted_value_it->second.votes.find(vote->getHash()) != found_voted_value_it->second.votes.end();
}

std::pair<bool, std::shared_ptr<PbftVote>> VoteManager::isUniqueVote(const std::shared_ptr<PbftVote>& vote) const {
  const auto step_votes_map = verified_votes_.getStepVotes(vote->getPeriod(), vote->getRound(), vote->getStep());
  if (!step_votes_map) {
    return {true, nullptr};
  }

  const auto found_voter_it = step_votes_map->unique_voters.find(vote->getVoterAddr());
  if (found_voter_it == step_votes_map->unique_voters.end()) {
    return {true, nullptr};
  }

  if (found_voter_it->second.first->getHash() == vote->getHash()) {
    return {true, nullptr};
  }

  if (vote->getType() == PbftVoteTypes::next_vote && vote->getStep() % 2) {
    if (found_voter_it->second.second == nullptr) {
      if (found_voter_it->second.first->getBlockHash() == kNullBlockHash && vote->getBlockHash() != kNullBlockHash) {
        return {true, nullptr};
      }
      if (found_voter_it->second.first->getBlockHash() != kNullBlockHash && vote->getBlockHash() == kNullBlockHash) {
        return {true, nullptr};
      }
    } else if (found_voter_it->second.second->getHash() == vote->getHash()) {
      return {true, nullptr};
    }
  }

  std::stringstream err;
  err << "Non unique vote: "
      << ", new vote hash (voted value): " << vote->getHash().abridged() << " (" << vote->getBlockHash().abridged()
      << ")"
      << ", orig. vote hash (voted value): " << found_voter_it->second.first->getHash().abridged() << " ("
      << found_voter_it->second.first->getBlockHash().abridged() << ")";
  if (found_voter_it->second.second != nullptr) {
    err << ", orig. vote 2 hash (voted value): " << found_voter_it->second.second->getHash().abridged() << " ("
        << found_voter_it->second.second->getBlockHash().abridged() << ")";
  }
  err << ", round: " << vote->getRound() << ", step: " << vote->getStep() << ", voter: " << vote->getVoterAddr();
  LOG(log_er_) << err.str();

  if (found_voter_it->second.second && vote->getHash() != found_voter_it->second.second->getHash()) {
    return {false, found_voter_it->second.second};
  }
  return {false, found_voter_it->second.first};
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getVerifiedVotes() const { return verified_votes_.votes(); }

uint64_t VoteManager::getVerifiedVotesSize() const { return verified_votes_.size(); }

void VoteManager::cleanupVotesByPeriod(PbftPeriod pbft_period) { verified_votes_.cleanupVotesByPeriod(pbft_period); }

std::vector<std::shared_ptr<PbftVote>> VoteManager::getProposalVotes(PbftPeriod period, PbftRound round) const {
  const auto& step_votes = verified_votes_.getStepVotes(period, round, PbftStates::value_proposal_state);
  if (!step_votes) {
    return {};
  }

  std::vector<std::shared_ptr<PbftVote>> proposal_votes;
  for (const auto& voted_value : step_votes->votes) {
    for (const auto& vote_pair : voted_value.second.votes) {
      proposal_votes.emplace_back(vote_pair.second);
    }
  }

  return proposal_votes;
}

std::optional<PbftRound> VoteManager::determineNewRound(PbftPeriod current_pbft_period, PbftRound current_pbft_round) {
  const auto decision = verified_votes_.determineRoundAdvance(current_pbft_period, current_pbft_round);
  if (!decision) {
    return {};
  }

  LOG(log_nf_) << "New round " << decision->new_round << " determined for period " << current_pbft_period
               << ". Found 2t+1 votes for block " << decision->voted_block.hash << " in round "
               << decision->supporting_round << ", step " << decision->voted_block.step;

  return decision->new_round;
}

void VoteManager::resetRewardVotes(PbftPeriod period, PbftRound round, PbftStep step, const blk_hash_t& block_hash,
                                   Batch& batch) {
  const auto result = resetRewardVotesForFinalization(makeRewardResetWritePlan(period, round, step, block_hash), batch);
  if (result.status != kPbftFinalizedPeriodApplyStatusApplied &&
      result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
    LOG(log_er_) << "Rust reward-vote reset storage appender rejected block " << block_hash << ", period " << period
                 << ", status " << static_cast<uint32_t>(result.status) << ", error "
                 << static_cast<std::string>(result.error_code);
    assert(false);
  }
}

std::pair<bool, std::vector<std::shared_ptr<PbftVote>>> VoteManager::checkRewardVotes(
    const std::shared_ptr<PbftBlock>& pbft_block, bool copy_votes) {
  // TODO(rustaxa): move reward-vote validation to Rust.
  return VoteManagerOld::checkRewardVotes(pbft_block, copy_votes);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getRewardVotes() {
  // TODO(rustaxa): move reward-vote retrieval to Rust.
  return VoteManagerOld::getRewardVotes();
}

PbftPeriod VoteManager::getRewardVotesPbftBlockPeriod() {
  // TODO(rustaxa): move reward-vote metadata ownership to Rust.
  return VoteManagerOld::getRewardVotesPbftBlockPeriod();
}

void VoteManager::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  // TODO(rustaxa): move own-vote persistence to Rust.
  VoteManagerOld::saveOwnVerifiedVote(vote);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getOwnVerifiedVotes() {
  // TODO(rustaxa): move own-vote snapshots to Rust.
  return VoteManagerOld::getOwnVerifiedVotes();
}

void VoteManager::clearOwnVerifiedVotes(Batch& write_batch) {
  // TODO(rustaxa): move own-vote cleanup to Rust.
  VoteManagerOld::clearOwnVerifiedVotes(write_batch);
}

std::shared_ptr<PbftVote> VoteManager::generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                              PbftPeriod period, PbftRound round, PbftStep step,
                                                              const WalletConfig& wallet) {
  // TODO(rustaxa): move weighted PBFT vote generation to Rust.
  return VoteManagerOld::generateVoteWithWeight(blockhash, vote_type, period, round, step, wallet);
}

std::shared_ptr<PbftVote> VoteManager::generateVote(const blk_hash_t& blockhash, PbftVoteTypes type, PbftPeriod period,
                                                    PbftRound round, PbftStep step, const WalletConfig& wallet) {
  // TODO(rustaxa): move PBFT vote generation to Rust.
  return VoteManagerOld::generateVote(blockhash, type, period, round, step, wallet);
}

std::pair<bool, std::string> VoteManager::validateVote(const std::shared_ptr<PbftVote>& vote, bool strict) const {
  // TODO(rustaxa): move PBFT vote validation to Rust.
  return VoteManagerOld::validateVote(vote, strict);
}

std::optional<uint64_t> VoteManager::getPbftTwoTPlusOne(PbftPeriod pbft_period, PbftVoteTypes vote_type) const {
  // TODO(rustaxa): move PBFT threshold calculation and cache ownership to Rust.
  return VoteManagerOld::getPbftTwoTPlusOne(pbft_period, vote_type);
}

bool VoteManager::voteAlreadyValidated(const vote_hash_t& vote_hash) const {
  // TODO(rustaxa): move validated-vote replay protection to Rust.
  return VoteManagerOld::voteAlreadyValidated(vote_hash);
}

bool VoteManager::genAndValidateVrfSortition(PbftPeriod pbft_period, PbftRound pbft_round,
                                             const WalletConfig& wallet) const {
  // TODO(rustaxa): move VRF sortition validation to Rust.
  return VoteManagerOld::genAndValidateVrfSortition(pbft_period, pbft_round, wallet);
}

std::optional<blk_hash_t> VoteManager::getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                                TwoTPlusOneVotedBlockType type) const {
  const auto voted_block = verified_votes_.getTwoTPlusOneVotedBlock(period, round, type);
  if (!voted_block) {
    return {};
  }
  return voted_block->hash;
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  return verified_votes_.getTwoTPlusOneVotedBlockVotes(period, round, type);
}

StepVotes VoteManager::getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const {
  return verified_votes_.getStepVotes(period, round, step).value_or(StepVotes{});
}

void VoteManager::setCurrentPbftPeriodAndRound(PbftPeriod pbft_period, PbftRound pbft_round) {
  current_pbft_period_ = pbft_period;
  current_pbft_round_ = pbft_round;

  auto round_votes = verified_votes_.getRoundVotes(pbft_period, pbft_round);
  if (!round_votes) {
    return;
  }

  for (const auto& two_t_plus_one_voted_block : round_votes->two_t_plus_one_voted_blocks_) {
    const auto two_t_plus_one_voted_block_type = two_t_plus_one_voted_block.first;
    if (two_t_plus_one_voted_block_type == TwoTPlusOneVotedBlockType::CertVotedBlock) {
      continue;
    }

    const auto& [two_t_plus_one_voted_block_hash, two_t_plus_one_voted_block_step] = two_t_plus_one_voted_block.second;
    const auto found_step_votes_it = round_votes->step_votes.find(two_t_plus_one_voted_block_step);
    if (found_step_votes_it == round_votes->step_votes.end()) {
      LOG(log_er_) << "Unable to find 2t+1 votes in verified_votes for period " << pbft_period << ", round "
                   << pbft_round << ", step " << two_t_plus_one_voted_block_step;
      assert(false);
      return;
    }

    const auto found_verified_votes_it = found_step_votes_it->second.votes.find(two_t_plus_one_voted_block_hash);
    if (found_verified_votes_it == found_step_votes_it->second.votes.end()) {
      LOG(log_er_) << "Unable to find 2t+1 votes in verified_votes for period " << pbft_period << ", round "
                   << pbft_round << ", step " << two_t_plus_one_voted_block_step << ", block hash "
                   << two_t_plus_one_voted_block_hash;
      assert(false);
      return;
    }

    std::vector<std::shared_ptr<PbftVote>> votes;
    votes.reserve(found_verified_votes_it->second.votes.size());
    for (const auto& vote : found_verified_votes_it->second.votes) {
      votes.push_back(vote.second);
    }

    db_->replaceTwoTPlusOneVotes(two_t_plus_one_voted_block_type, votes);
  }
}

PbftStep VoteManager::getNetworkTplusOneNextVotingStep(PbftPeriod period, PbftRound round) const {
  auto round_votes = verified_votes_.getRoundVotes(period, round);
  if (!round_votes) {
    return 0;
  }

  return round_votes->network_t_plus_one_step;
}

bool VoteManager::isValidRewardVoteForRust(const std::shared_ptr<PbftVote>& vote) const {
  std::shared_lock lock(reward_votes_info_mutex_);
  if (vote->getType() != PbftVoteTypes::cert_vote) {
    LOG(log_tr_) << "Invalid reward vote: type " << static_cast<uint64_t>(vote->getType())
                 << " is different from cert type";
    return false;
  }

  if (vote->getBlockHash() != reward_votes_block_hash_) {
    LOG(log_tr_) << "Invalid reward vote: block hash " << vote->getBlockHash()
                 << " is different from reward_votes_block_hash " << reward_votes_block_hash_;
    return false;
  }

  if (vote->getPeriod() != reward_votes_period_) {
    LOG(log_tr_) << "Invalid reward vote: period " << vote->getPeriod()
                 << " is different from reward_votes_block_period " << reward_votes_period_;
    return false;
  }

  if (vote->getRound() > reward_votes_round_ + 100) {
    LOG(log_wr_) << "Invalid reward vote: round " << vote->getRound() << " exceeded max round "
                 << reward_votes_round_ + 100;
    return false;
  }

  return true;
}

rustaxa::PbftFinalizationStorageWriteStage VoteManager::rewardVotesResetStageForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto round = static_cast<PbftRound>(write_intent.reward_vote_round);
  const auto step = static_cast<PbftStep>(write_intent.reward_vote_step);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);

  const auto& round_votes = verified_votes_.getRoundVotes(period, round);
  if (!round_votes) {
    LOG(log_er_) << "resetRewardVotes missing round " << round << " or period " << period;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_ROUND");
  }

  auto found_step_it = round_votes->step_votes.find(step);
  if (found_step_it == round_votes->step_votes.end()) {
    LOG(log_er_) << "resetRewardVotes missing step" << step;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_STEP");
  }

  auto found_two_t_plus_one_voted_block =
      round_votes->two_t_plus_one_voted_blocks_.find(TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (found_two_t_plus_one_voted_block == round_votes->two_t_plus_one_voted_blocks_.end()) {
    LOG(log_er_) << "resetRewardVotes missing cert voted block";
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_CERT_BLOCK");
  }
  if (found_two_t_plus_one_voted_block->second.hash != block_hash) {
    LOG(log_er_) << "resetRewardVotes incorrect block " << found_two_t_plus_one_voted_block->second.step << " expected "
                 << block_hash;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_REWARD_VOTES_CERT_BLOCK_MISMATCH");
  }
  auto found_voted_value_it = found_step_it->second.votes.find(block_hash);
  if (found_voted_value_it == found_step_it->second.votes.end()) {
    LOG(log_er_) << "resetRewardVotes missing vote block " << block_hash;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_BLOCK");
  }

  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(found_voted_value_it->second.votes.size());
  for (const auto& tmp_vote : found_voted_value_it->second.votes) {
    votes.push_back(tmp_vote.second);
  }
  return makeRewardResetWriteStage(votes, extra_reward_votes_);
}

rustaxa::PbftFinalizationLiveMutationReport VoteManager::commitRewardVotesResetForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto round = static_cast<PbftRound>(write_intent.reward_vote_round);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);

  {
    std::scoped_lock lock(reward_votes_info_mutex_);
    reward_votes_block_hash_ = block_hash;
    reward_votes_period_ = period;
    reward_votes_round_ = round;
  }
  extra_reward_votes_.clear();

  LOG(log_dg_) << "Reward votes info reset to: block_hash: " << block_hash << ", period: " << period
               << ", round: " << round;
  return makeRewardVotesResetLiveReport(write_intent, extra_reward_votes_.size());
}

rustaxa::PbftFinalizedPeriodApplyResult VoteManager::resetRewardVotesForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, Batch& batch) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);

  rustaxa::PbftFinalizationStorageWriteStage stage{};
  try {
    stage = rewardVotesResetStageForFinalization(write_intent);
  } catch (const std::exception& e) {
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash, e.what());
  }

  auto result =
      rustaxa::append_pbft_finalization_storage_write(db_->rustStorage(), db_->rustBatchId(batch), write_intent, stage);
  if (result.status != kPbftFinalizedPeriodApplyStatusApplied &&
      result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
    return result;
  }

  commitRewardVotesResetForFinalization(write_intent);
  return result;
}

}  // namespace taraxa
