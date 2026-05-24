#include <libdevcore/RLP.h>

#include <mutex>
#include <stdexcept>

#include "rustaxa-bridge/ffi.rs.h"
#include "storage/storage.hpp"
#include "vote/pbft_vote.hpp"
#include "vote_manager/vote_manager.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kPbftFinalizedPeriodApplyStatusApplied = 0;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusAlreadyApplied = 1;
constexpr uint8_t kPbftFinalizedPeriodApplyStatusRejected = 2;
constexpr uint8_t kPbftFinalizationStorageStageRewardVotesReset = 4;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

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

}  // namespace

void VoteManager::setNetwork(std::weak_ptr<Network> network) {
  // TODO(rustaxa): move VoteManager network wiring to Rust/shim-owned state.
  VoteManagerOld::setNetwork(std::move(network));
}

bool VoteManager::addVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  // TODO(rustaxa): move verified-vote insertion to Rust.
  return VoteManagerOld::addVerifiedVote(vote);
}

bool VoteManager::voteInVerifiedMap(std::shared_ptr<PbftVote> const& vote) const {
  // TODO(rustaxa): move verified-vote lookup to Rust.
  return VoteManagerOld::voteInVerifiedMap(vote);
}

std::pair<bool, std::shared_ptr<PbftVote>> VoteManager::isUniqueVote(const std::shared_ptr<PbftVote>& vote) const {
  // TODO(rustaxa): move unique-vote conflict detection to Rust.
  return VoteManagerOld::isUniqueVote(vote);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getVerifiedVotes() const {
  // TODO(rustaxa): move verified-vote snapshots to Rust.
  return VoteManagerOld::getVerifiedVotes();
}

uint64_t VoteManager::getVerifiedVotesSize() const {
  // TODO(rustaxa): move verified-vote accounting to Rust.
  return VoteManagerOld::getVerifiedVotesSize();
}

void VoteManager::cleanupVotesByPeriod(PbftPeriod pbft_period) {
  // TODO(rustaxa): move period vote cleanup to Rust.
  VoteManagerOld::cleanupVotesByPeriod(pbft_period);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getProposalVotes(PbftPeriod period, PbftRound round) const {
  // TODO(rustaxa): move proposal-vote selection to Rust.
  return VoteManagerOld::getProposalVotes(period, round);
}

std::optional<PbftRound> VoteManager::determineNewRound(PbftPeriod current_pbft_period, PbftRound current_pbft_round) {
  // TODO(rustaxa): move PBFT round advancement planning to Rust.
  return VoteManagerOld::determineNewRound(current_pbft_period, current_pbft_round);
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
  // TODO(rustaxa): move two-t-plus-one voted-block selection to Rust.
  return VoteManagerOld::getTwoTPlusOneVotedBlock(period, round, type);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  // TODO(rustaxa): move two-t-plus-one vote bundle selection to Rust.
  return VoteManagerOld::getTwoTPlusOneVotedBlockVotes(period, round, type);
}

StepVotes VoteManager::getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const {
  // TODO(rustaxa): move step-vote snapshots to Rust.
  return VoteManagerOld::getStepVotes(period, round, step);
}

void VoteManager::setCurrentPbftPeriodAndRound(PbftPeriod pbft_period, PbftRound pbft_round) {
  // TODO(rustaxa): move current PBFT period/round state tracking to Rust.
  VoteManagerOld::setCurrentPbftPeriodAndRound(pbft_period, pbft_round);
}

PbftStep VoteManager::getNetworkTplusOneNextVotingStep(PbftPeriod period, PbftRound round) const {
  // TODO(rustaxa): move network next-vote step analysis to Rust.
  return VoteManagerOld::getNetworkTplusOneNextVotingStep(period, round);
}

rustaxa::PbftFinalizedPeriodApplyResult VoteManager::resetRewardVotesForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, Batch& batch) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto round = static_cast<PbftRound>(write_intent.reward_vote_round);
  const auto step = static_cast<PbftStep>(write_intent.reward_vote_step);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);

  const auto& round_votes = verified_votes_.getRoundVotes(period, round);
  if (!round_votes) {
    LOG(log_er_) << "resetRewardVotes missing round " << round << " or period " << period;
    assert(false);
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash,
                             "PBFT_FINALIZE_MISSING_REWARD_VOTES_ROUND");
  }

  auto found_step_it = round_votes->step_votes.find(step);
  if (found_step_it == round_votes->step_votes.end()) {
    LOG(log_er_) << "resetRewardVotes missing step" << step;
    assert(false);
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash,
                             "PBFT_FINALIZE_MISSING_REWARD_VOTES_STEP");
  }

  auto found_two_t_plus_one_voted_block =
      round_votes->two_t_plus_one_voted_blocks_.find(TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (found_two_t_plus_one_voted_block == round_votes->two_t_plus_one_voted_blocks_.end()) {
    LOG(log_er_) << "resetRewardVotes missing cert voted block";
    assert(false);
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash,
                             "PBFT_FINALIZE_MISSING_REWARD_VOTES_CERT_BLOCK");
  }
  if (found_two_t_plus_one_voted_block->second.hash != block_hash) {
    LOG(log_er_) << "resetRewardVotes incorrect block " << found_two_t_plus_one_voted_block->second.step << " expected "
                 << block_hash;
    assert(false);
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash,
                             "PBFT_FINALIZE_REWARD_VOTES_CERT_BLOCK_MISMATCH");
  }
  auto found_voted_value_it = found_step_it->second.votes.find(block_hash);
  if (found_voted_value_it == found_step_it->second.votes.end()) {
    LOG(log_er_) << "resetRewardVotes missing vote block " << block_hash;
    assert(false);
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash,
                             "PBFT_FINALIZE_MISSING_REWARD_VOTES_BLOCK");
  }

  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(found_voted_value_it->second.votes.size());
  for (const auto& tmp_vote : found_voted_value_it->second.votes) {
    votes.push_back(tmp_vote.second);
  }

  auto result = rustaxa::append_pbft_finalization_storage_write(
      db_->rustStorage(), db_->rustBatchId(batch), write_intent, makeRewardResetWriteStage(votes, extra_reward_votes_));
  if (result.status != kPbftFinalizedPeriodApplyStatusApplied &&
      result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
    return result;
  }

  {
    std::scoped_lock lock(reward_votes_info_mutex_);
    reward_votes_block_hash_ = block_hash;
    reward_votes_period_ = period;
    reward_votes_round_ = round;
  }
  extra_reward_votes_.clear();

  LOG(log_dg_) << "Reward votes info reset to: block_hash: " << block_hash << ", period: " << period
               << ", round: " << round;
  return result;
}

}  // namespace taraxa
