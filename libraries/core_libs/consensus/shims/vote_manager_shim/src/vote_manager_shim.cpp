#include <libdevcore/RLP.h>

#include <algorithm>
#include <cstdlib>
#include <limits>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "pbft/pbft_manager.hpp"
#include "pbft/pbft_service.hpp"
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
constexpr uint8_t kPbftVoteValidationStatusValid = 1;
constexpr uint8_t kPbftVoteValidationStatusZeroStake = 2;
constexpr uint8_t kPbftVoteValidationStatusMissingVrfKey = 3;
constexpr uint8_t kPbftVoteValidationStatusInvalidSignature = 4;
constexpr uint8_t kPbftVoteValidationStatusInvalidVrfProof = 5;
constexpr uint8_t kPbftVoteValidationStatusZeroWeight = 6;
constexpr uint8_t kPbftVoteValidationStatusInvalidVoteType = 9;
constexpr uint8_t kPbftCanonicalVoteInspectionStatusValid = 0;
constexpr uint8_t kPbftCanonicalVoteInspectionStatusMalformedRlp = 1;
constexpr uint8_t kPbftCanonicalVoteInspectionStatusInvalidSignature = 2;
constexpr uint8_t kPbftVoteGenerationStatusGenerated = 0;
constexpr uint8_t kPbftVoteGenerationStatusZeroStake = 4;
constexpr uint8_t kPbftVoteGenerationStatusZeroTotalDpos = 5;
constexpr uint8_t kPbftManagerLeaderBlockAlreadyValid = 0;
constexpr uint8_t kPbftManagerLeaderBlockValidated = 1;
constexpr uint8_t kPbftManagerLeaderBlockRejected = 2;
constexpr uint8_t kPbftManagerLeaderSelectionInvalidFact = 3;
constexpr uint8_t kPbftLeaderSelectionReadyOrSelected = 0;
constexpr uint8_t kPbftLeaderSelectionNoCandidates = 1;
constexpr uint8_t kPbftLeaderSelectionNoEligibleLeader = 2;
constexpr uint8_t kPbftLeaderSelectionStaleSnapshot = 3;
constexpr uint8_t kPbftLeaderSelectionInvalidValidationReport = 4;
constexpr uint8_t kPbftLeaderSelectionServiceUnavailable = 5;
constexpr uint8_t kPbftVoteGenerationStatusZeroWeight = 6;
constexpr uint8_t kPbftProposerSortitionStatusFutureDposState = 4;
constexpr uint8_t kPbftVotePersistenceStatusApplied = 0;
constexpr uint8_t kPbftVoteAdmissionPersistenceNotRequired = 0;
constexpr uint8_t kPbftVoteAdmissionPersistenceApplied = 1;
constexpr uint8_t kPbftVoteAdmissionPersistenceRejected = 2;
constexpr uint8_t kPbftTwoTPlusOneThresholdStatusAvailable = 0;

std::array<uint8_t, 32> toBridgeHash(const uint256_hash_t& hash) { return hash.asArray(); }

uint256_hash_t fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return uint256_hash_t(hash.data(), uint256_hash_t::ConstructFromPointer);
}

std::array<uint8_t, 20> toBridgeAddress(const addr_t& address) { return address.asArray(); }

template <size_t N, typename FixedHash>
std::array<uint8_t, N> toBridgeFixedBytes(const FixedHash& value) {
  std::array<uint8_t, N> out{};
  std::copy(value.data(), value.data() + N, out.begin());
  return out;
}

addr_t fromBridgeAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

vote_hash_t fromBridgeVoteHash(const std::array<uint8_t, 32>& hash) {
  return vote_hash_t(hash.data(), vote_hash_t::ConstructFromPointer);
}

rust::Vec<uint8_t> toBridgeBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

dev::bytes fromBridgeBytes(const rust::Vec<uint8_t>& bytes) {
  dev::bytes out;
  out.reserve(bytes.size());
  for (const auto byte : bytes) {
    out.push_back(byte);
  }
  return out;
}

std::runtime_error verifiedVoteViewError(const std::string& msg) {
  return std::runtime_error("VoteManager verified-vote view: " + msg);
}

std::shared_ptr<PbftVote> materializeWeightedVote(const rustaxa::PbftVoteStorageRecord& record) {
  auto vote = std::make_shared<PbftVote>(fromBridgeBytes(record.vote_rlp));
  if (vote->getHash() != fromBridgeHash(record.hash)) {
    throw verifiedVoteViewError("native retained payload hash mismatches materialized vote");
  }
  if (!vote->getWeight().has_value() || *vote->getWeight() == 0) {
    throw verifiedVoteViewError("native retained payload decoded without non-zero weight");
  }
  return vote;
}

StepVotes materializeStepVotes(const rustaxa::VerifiedStepVotePayloadsLookup& lookup, PbftPeriod period,
                               PbftRound round, PbftStep step) {
  StepVotes result;
  if (!lookup.found) {
    return result;
  }

  for (const auto& entry : lookup.entries) {
    const auto block_hash = fromBridgeHash(entry.block_hash);
    auto& voted_value = result.votes[block_hash];
    voted_value.weight = entry.total_weight;
    for (const auto& record : entry.votes) {
      auto vote = materializeWeightedVote(record);
      if (vote->getPeriod() != period || vote->getRound() != round || vote->getStep() != step ||
          vote->getBlockHash() != block_hash) {
        throw verifiedVoteViewError("native step payload mismatches requested vote bucket");
      }

      const auto vote_hash = vote->getHash();
      voted_value.votes.insert({vote_hash, vote});
      auto& unique_votes = result.unique_voters[vote->getVoterAddr()];
      if (!unique_votes.first) {
        unique_votes.first = vote;
      } else if (unique_votes.first->getHash() != vote_hash) {
        if (!unique_votes.second) {
          const auto first_is_null = unique_votes.first->getBlockHash() == kNullBlockHash;
          const auto second_is_null = vote->getBlockHash() == kNullBlockHash;
          if (vote->getType() == PbftVoteTypes::next_vote && (vote->getStep() % 2) && first_is_null != second_is_null) {
            unique_votes.second = vote;
          }
        } else if (unique_votes.second->getHash() != vote_hash) {
          throw verifiedVoteViewError("unexpected native unique-voter step conflict");
        }
      }
    }
  }
  return result;
}

RoundVerifiedVotes materializeRoundVotes(const rustaxa::VerifiedVotesStateSnapshot& snapshot, PbftPeriod period,
                                         PbftRound round) {
  RoundVerifiedVotes result;
  for (const auto& entry : snapshot.votes) {
    if (entry.vote.period != period || entry.vote.round != round) {
      continue;
    }
    auto vote = materializeWeightedVote(entry.weighted_vote);
    if (vote->getBlockHash() != fromBridgeHash(entry.vote.block_hash) || vote->getStep() != entry.vote.step ||
        static_cast<uint8_t>(vote->getType()) != entry.vote.vote_type || *vote->getWeight() != entry.vote.weight) {
      throw verifiedVoteViewError("native snapshot metadata mismatches retained payload");
    }
    auto& step_votes = result.step_votes[static_cast<PbftStep>(entry.vote.step)];
    auto& voted_value = step_votes.votes[vote->getBlockHash()];
    voted_value.weight += entry.vote.weight;
    voted_value.votes.insert({vote->getHash(), vote});
    auto& unique_votes = step_votes.unique_voters[vote->getVoterAddr()];
    if (!unique_votes.first) {
      unique_votes.first = vote;
    } else if (unique_votes.first->getHash() != vote->getHash()) {
      if (!unique_votes.second) {
        const auto first_is_null = unique_votes.first->getBlockHash() == kNullBlockHash;
        const auto second_is_null = vote->getBlockHash() == kNullBlockHash;
        if (vote->getType() == PbftVoteTypes::next_vote && (vote->getStep() % 2) && first_is_null != second_is_null) {
          unique_votes.second = vote;
        }
      } else if (unique_votes.second->getHash() != vote->getHash()) {
        throw verifiedVoteViewError("unexpected native unique-voter snapshot conflict");
      }
    }
  }
  for (const auto& marker : snapshot.round_markers) {
    if (marker.period == period && marker.round == round) {
      result.network_t_plus_one_step = static_cast<PbftStep>(marker.network_t_plus_one_step);
    }
  }
  for (const auto& entry : snapshot.two_t_plus_one) {
    if (entry.period == period && entry.round == round) {
      result.two_t_plus_one_voted_blocks_[static_cast<TwoTPlusOneVotedBlockType>(entry.kind)] =
          VotedBlock{fromBridgeHash(entry.block_hash), static_cast<PbftStep>(entry.step)};
    }
  }
  return result;
}

rust::Vec<rustaxa::PbftFinalizationHash> toBridgeVoteHashes(const std::vector<vote_hash_t>& hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{hash.asArray()});
  }
  return out;
}

rust::Slice<const uint8_t> toBridgeByteSlice(const rust::Vec<uint8_t>& bytes) {
  return rust::Slice<const uint8_t>(bytes.data(), bytes.size());
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

RewardVotesFinalizationResetReport makeRewardVotesResetLiveReport(PbftPeriod period, PbftRound round,
                                                                  const blk_hash_t& block_hash,
                                                                  uint64_t reward_votes_reset_generation) {
  RewardVotesFinalizationResetReport report{};
  report.period = period;
  report.round = round;
  report.block_hash = block_hash;
  report.reward_votes_reset_generation = reward_votes_reset_generation;
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

rustaxa::PbftVoteStorageRecord makeVoteStorageRecord(const std::shared_ptr<PbftVote>& vote) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot persist a null PBFT vote");
  }
  const auto weight = vote->getWeight();
  if (!weight.has_value()) {
    throw std::runtime_error("VoteManager cannot persist an unweighted PBFT vote");
  }

  auto canonical_vote_rlp = toBridgeBytes(vote->rlp(true, false));
  auto record = rustaxa::pbft_vote_weighted_payload_from_canonical_vote(toBridgeByteSlice(canonical_vote_rlp), *weight);
  if (record.hash != toBridgeHash(vote->getHash())) {
    throw std::runtime_error("Rust PBFT vote storage payload hash mismatches live vote hash");
  }
  return record;
}

rust::Vec<rustaxa::PbftVoteStorageRecord> makeVoteStorageRecords(const std::vector<std::shared_ptr<PbftVote>>& votes,
                                                                 const char* operation) {
  rust::Vec<rustaxa::PbftVoteStorageRecord> records;
  records.reserve(votes.size());
  for (const auto& vote : votes) {
    if (!vote) {
      std::stringstream err;
      err << "VoteManager cannot persist " << operation << " with null votes";
      throw std::runtime_error(err.str());
    }
    records.push_back(makeVoteStorageRecord(vote));
  }
  return records;
}

rustaxa::PbftRewardVotesResetRequest makeRewardResetRequest(PbftPeriod period, PbftRound round, PbftStep step,
                                                            const blk_hash_t& block_hash, bool sync) {
  rustaxa::PbftRewardVotesResetRequest request{};
  request.period = period;
  request.round = round;
  request.step = step;
  request.block_hash = toBridgeHash(block_hash);
  request.sync = sync;
  return request;
}

rustaxa::PbftTwoTPlusOneVoteBundle makeTwoTPlusOneVoteBundle(TwoTPlusOneVotedBlockType type,
                                                             const std::vector<std::shared_ptr<PbftVote>>& votes) {
  if (votes.empty() || !votes.front()) {
    throw std::runtime_error("VoteManager cannot persist an empty 2t+1 PBFT vote bundle");
  }

  auto records = makeVoteStorageRecords(votes, "2t+1 PBFT vote bundle");

  rustaxa::PbftTwoTPlusOneVoteBundle bundle{};
  bundle.kind = static_cast<uint8_t>(type);
  bundle.period = votes.front()->getPeriod();
  bundle.round = votes.front()->getRound();
  bundle.step = votes.front()->getStep();
  bundle.block_hash = toBridgeHash(votes.front()->getBlockHash());
  bundle.votes_bundle_rlp = rustaxa::pbft_vote_bundle_payload_from_records(std::move(records));
  return bundle;
}

void requireApplied(const rustaxa::PbftVotePersistenceResult& result, const char* operation) {
  if (result.status == kPbftVotePersistenceStatusApplied) {
    return;
  }

  std::stringstream err;
  err << "Rust PBFT vote persistence rejected " << operation << ": " << static_cast<std::string>(result.error_code);
  throw std::runtime_error(err.str());
}

void persistVoteProgressToRustStorage(const SharedPbftService& pbft_service,
                                      const std::shared_ptr<PbftVote>& extra_reward_vote,
                                      std::optional<TwoTPlusOneVotedBlockType> two_t_plus_one_type,
                                      const std::vector<std::shared_ptr<PbftVote>>& two_t_plus_one_votes) {
  rustaxa::PbftVoteProgressPersistenceWrite write{};
  if (extra_reward_vote) {
    write.has_extra_reward_vote = true;
    write.extra_reward_vote = makeVoteStorageRecord(extra_reward_vote);
  }

  if (two_t_plus_one_type.has_value()) {
    write.has_two_t_plus_one_bundle = true;
    write.two_t_plus_one_bundle = makeTwoTPlusOneVoteBundle(*two_t_plus_one_type, two_t_plus_one_votes);
  }

  requireApplied(pbft_service->service().pbft_service_verified_votes_persist_pbft_vote_progress(std::move(write)),
                 "vote progress");
}

rustaxa::PbftVoteEventFactFlags makeVoteEventFactFlags() {
  rustaxa::PbftVoteEventFactFlags flags{};
  flags.vote_already_known = false;
  flags.carries_proposed_block = true;
  return flags;
}

void requireRuntimeAdmissionVoteMatches(const rustaxa::VerifiedVotePayload& fact, const std::shared_ptr<PbftVote>& vote,
                                        uint64_t rust_weight) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot compare PBFT vote admission result without a vote sidecar");
  }

  if (fact.vote_hash != toBridgeHash(vote->getHash()) || fact.block_hash != toBridgeHash(vote->getBlockHash()) ||
      fact.voter != toBridgeAddress(vote->getVoterAddr()) || fact.period != vote->getPeriod() ||
      fact.round != vote->getRound() || fact.step != vote->getStep() ||
      fact.vote_type != static_cast<uint8_t>(vote->getType()) || fact.weight != rust_weight) {
    throw std::runtime_error("VoteManager Rust PBFT vote admission result mismatched live vote sidecar");
  }
}

void requireRuntimeVoteTransitionIntentsMatch(const rustaxa::PbftVoteAdmissionRuntimeResult& result,
                                              const std::shared_ptr<PbftVote>& vote) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot validate PBFT vote transition intents without a live sidecar");
  }

  const auto vote_hash = toBridgeHash(vote->getHash());
  if (result.mark_vote_known && result.mark_vote_known_hash != vote_hash) {
    throw std::runtime_error("VoteManager Rust PBFT vote transition returned mismatched peer-known intent");
  }
  if (result.gossip_vote && result.gossip_vote_hash != vote_hash) {
    throw std::runtime_error("VoteManager Rust PBFT vote transition returned mismatched gossip intent");
  }
  if (result.request_proposed_block_sidecar) {
    throw std::runtime_error(
        "VoteManager Rust PBFT vote transition requested unsupported proposed-block sidecar fetch");
  }
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

rustaxa::PbftVoteAdmissionValidationRequest makeVoteAdmissionValidationRequest(const std::shared_ptr<PbftVote>& vote,
                                                                               const PbftConfig& config) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot build PBFT vote admission request without a vote");
  }

  rustaxa::PbftVoteAdmissionValidationRequest request{};
  request.strict_vrf = true;
  request.committee_size = config.committee_size;
  request.number_of_proposers = config.number_of_proposers;
  if (vote->getWeight().has_value()) {
    request.has_preverified_weight = true;
    request.preverified_weight = *vote->getWeight();
  }
  return request;
}

rustaxa::PbftProposerSortitionRequest makeProposerSortitionRequest(PbftPeriod pbft_period, PbftRound pbft_round,
                                                                   const WalletConfig& wallet,
                                                                   const PbftConfig& config) {
  rustaxa::PbftProposerSortitionRequest request{};
  request.pbft_period = pbft_period;
  request.pbft_round = pbft_round;
  request.number_of_proposers = config.number_of_proposers;
  request.vrf_secret = toBridgeFixedBytes<64>(wallet.vrf_secret);
  request.expected_vrf_public_key = toBridgeFixedBytes<32>(wallet.vrf_pk);
  request.voter_public_key = toBridgeFixedBytes<64>(wallet.node_pk);
  request.expected_voter = toBridgeAddress(wallet.node_addr);
  return request;
}

rustaxa::PbftVoteGenerationInput makeVoteGenerationInput(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                         PbftPeriod period, PbftRound round, PbftStep step,
                                                         const WalletConfig& wallet) {
  rustaxa::PbftVoteGenerationInput input{};
  input.block_hash = toBridgeHash(blockhash);
  input.vote_type = static_cast<uint8_t>(vote_type);
  input.period = period;
  input.round = round;
  input.step = step;
  input.node_secret = toBridgeFixedBytes<32>(wallet.node_secret);
  input.vrf_secret = toBridgeFixedBytes<64>(wallet.vrf_secret);
  input.expected_voter = toBridgeAddress(wallet.node_addr);
  input.expected_vrf_public_key = toBridgeFixedBytes<32>(wallet.vrf_pk);
  return input;
}

void requireRustVoteGenerationRejected(const rustaxa::PbftGeneratedVote& generated, uint8_t expected_status,
                                       const char* operation) {
  if (!generated.accepted && generated.status == expected_status) {
    return;
  }

  std::stringstream err;
  err << "Rust PBFT vote generation parity failed for " << operation << ": expected status "
      << static_cast<uint32_t>(expected_status) << ", got status " << static_cast<uint32_t>(generated.status)
      << " error " << static_cast<std::string>(generated.error_code);
  throw std::runtime_error(err.str());
}

void requireRustVoteGenerationMatches(const std::shared_ptr<PbftVote>& vote,
                                      const rustaxa::PbftGeneratedVote& generated, bool expect_weight) {
  if (!vote) {
    throw std::runtime_error("Rust PBFT vote generation parity cannot compare a null C++ vote");
  }
  if (!generated.accepted || generated.status != kPbftVoteGenerationStatusGenerated) {
    std::stringstream err;
    err << "Rust PBFT vote generation rejected a legacy-generated vote: status "
        << static_cast<uint32_t>(generated.status) << " error " << static_cast<std::string>(generated.error_code);
    throw std::runtime_error(err.str());
  }

  const auto canonical_vote_rlp = toBridgeBytes(vote->rlp(true, false));
  const auto inspection = rustaxa::pbft_inspect_canonical_vote(toBridgeByteSlice(canonical_vote_rlp));
  if (inspection.status != kPbftCanonicalVoteInspectionStatusValid) {
    throw std::runtime_error("Rust PBFT vote generation parity cannot inspect the legacy-generated vote");
  }

  if (toBridgeHash(vote->getHash()) != generated.vote_hash || inspection.signing_hash != generated.signing_hash ||
      toBridgeHash(vote->getBlockHash()) != generated.block_hash || vote->getPeriod() != generated.period ||
      vote->getRound() != generated.round || vote->getStep() != generated.step ||
      static_cast<uint8_t>(vote->getType()) != generated.vote_type ||
      toBridgeAddress(vote->getVoterAddr()) != generated.voter ||
      toBridgeFixedBytes<64>(vote->getVoter()) != generated.voter_public_key ||
      toBridgeFixedBytes<80>(vote->getSortitionProof()) != generated.vrf_proof) {
    throw std::runtime_error("Rust PBFT vote generation facts do not match legacy C++ vote facts");
  }

  if (expect_weight) {
    if (!generated.has_weight || !vote->getWeight().has_value() || *vote->getWeight() != generated.weight ||
        fromBridgeBytes(generated.vote_rlp) != vote->rlp(true, true)) {
      throw std::runtime_error("Rust weighted PBFT vote generation bytes do not match legacy C++ vote bytes");
    }
  } else if (generated.has_weight || fromBridgeBytes(generated.vote_rlp) != vote->rlp(true, false)) {
    throw std::runtime_error("Rust PBFT vote generation bytes do not match legacy C++ vote bytes");
  }
}

std::shared_ptr<PbftVote> materializeRustGeneratedVote(const rustaxa::PbftGeneratedVote& generated,
                                                       const WalletConfig& wallet, bool expect_weight) {
  if (!generated.accepted || generated.status != kPbftVoteGenerationStatusGenerated) {
    std::stringstream err;
    err << "Rust PBFT vote generation rejected local vote materialization: status "
        << static_cast<uint32_t>(generated.status) << " error " << static_cast<std::string>(generated.error_code);
    throw std::runtime_error(err.str());
  }
  if (generated.vote_rlp.empty() || generated.has_weight != expect_weight) {
    throw std::runtime_error("Rust PBFT vote generation returned an invalid materialization payload");
  }

  auto vote = std::make_shared<PbftVote>(fromBridgeBytes(generated.vote_rlp));
  if (!vote->verifyVrfSortition(wallet.vrf_pk, true)) {
    throw std::runtime_error("Rust-generated PBFT vote failed local VRF hydration");
  }
  requireRustVoteGenerationMatches(vote, generated, expect_weight);
  return vote;
}

std::shared_ptr<PbftVote> materializeOwnVoteRecord(const rustaxa::PbftVoteStorageRecord& record) {
  auto vote = std::make_shared<PbftVote>(fromBridgeBytes(record.vote_rlp));
  if (vote->getHash() != fromBridgeVoteHash(record.hash)) {
    throw std::runtime_error("VoteManager Rust own-vote storage key mismatches materialized vote");
  }
  if (!vote->getWeight().has_value() || *vote->getWeight() == 0) {
    throw std::runtime_error("VoteManager Rust own-vote record decoded without non-zero weight");
  }
  return vote;
}

}  // namespace

VoteManager::VoteManager(const FullNodeConfig& config, SharedPbftService pbft_service,
                         std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                         std::shared_ptr<KeyManager> key_manager, std::shared_ptr<SlashingManager> slashing_manager)
    : kPbftConfig(config.genesis.pbft),
      pbft_chain_(std::move(pbft_chain)),
      final_chain_(std::move(final_chain)),
      slashing_manager_(std::move(slashing_manager)),
      pbft_service_(std::move(pbft_service)) {
  (void)key_manager;
  const auto node_addr = dev::toAddress(config.getFirstWallet().node_secret);
  LOG_OBJECTS_CREATE("VOTE_MGR");
}

void VoteManager::setNetwork(std::weak_ptr<Network> network) { network_ = std::move(network); }

bool VoteManager::addVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  return addVerifiedVoteWithReport(vote).accepted;
}

bool VoteManager::addLocallyGeneratedVote(const std::shared_ptr<PbftVote>& vote) {
  if (!addVerifiedVoteWithReport(vote).accepted) {
    return false;
  }
  saveOwnVerifiedVote(vote);
  return true;
}

VoteManager::SyncedCertVoteValidationResult VoteManager::validateSyncedCertVoteBundle(
    PbftPeriod block_period, const blk_hash_t& block_hash, const std::vector<std::shared_ptr<PbftVote>>& cert_votes) {
  auto make_cert_vote_bundle_fact = [&](bool check_weight_threshold, bool two_t_plus_one_found,
                                        uint64_t two_t_plus_one) {
    rustaxa::PbftSyncCertVoteBundleFact fact;
    fact.block_period = block_period;
    fact.block_hash = toBridgeHash(block_hash);
    fact.check_weight_threshold = check_weight_threshold;
    fact.two_t_plus_one_found = two_t_plus_one_found;
    fact.two_t_plus_one = two_t_plus_one;
    fact.votes.reserve(cert_votes.size());
    for (const auto& vote : cert_votes) {
      if (!vote) {
        throw std::runtime_error("VoteManager cannot validate a null synced cert vote");
      }
      rustaxa::PbftSyncCertVoteFact vote_fact;
      vote_fact.vote_hash = toBridgeHash(vote->getHash());
      vote_fact.block_hash = toBridgeHash(vote->getBlockHash());
      vote_fact.period = vote->getPeriod();
      vote_fact.round = vote->getRound();
      vote_fact.step = vote->getStep();
      vote_fact.vote_type = static_cast<uint8_t>(vote->getType());
      vote_fact.live_vote_valid = true;
      vote_fact.weight_present = vote->getWeight().has_value();
      vote_fact.weight = vote->getWeight().value_or(0);
      fact.votes.push_back(vote_fact);
    }
    return fact;
  };

  SyncedCertVoteValidationResult result;
  rustaxa::PbftSyncCertVoteBundleValidation shape_validation{};
  try {
    shape_validation = rustaxa::validate_pbft_sync_cert_vote_bundle(make_cert_vote_bundle_fact(false, false, 0));
  } catch (const std::exception& e) {
    result.validation_error = e.what();
    return result;
  }
  result.status = shape_validation.status;
  result.first_bad_vote_hash = fromBridgeVoteHash(shape_validation.first_bad_vote_hash);
  result.total_weight = shape_validation.total_weight;
  result.two_t_plus_one = shape_validation.two_t_plus_one;
  if (!shape_validation.valid) {
    return result;
  }

  const uint32_t full_vote_validation_interval = 100;
  const uint32_t vote_to_validate = std::rand() % cert_votes.size();
  const bool strict_validation = (block_period % full_vote_validation_interval == 0);
  for (uint32_t vote_counter = 0; vote_counter < cert_votes.size(); vote_counter++) {
    const auto& vote = cert_votes[vote_counter];
    const bool strict = strict_validation || (vote_counter == vote_to_validate);
    const auto validation = validateVote(vote, strict);
    if (!validation.first) {
      result.first_bad_vote_hash = vote ? vote->getHash() : vote_hash_t();
      result.validation_error = validation.second;
      return result;
    }

    assert(vote->getWeight());
    addVerifiedVote(vote);
  }

  const auto two_t_plus_one = getPbftTwoTPlusOne(block_period - 1, PbftVoteTypes::cert_vote);
  const auto threshold_validation = rustaxa::validate_pbft_sync_cert_vote_bundle(
      make_cert_vote_bundle_fact(true, two_t_plus_one.has_value(), two_t_plus_one.value_or(0)));
  result.status = threshold_validation.status;
  result.first_bad_vote_hash = fromBridgeVoteHash(threshold_validation.first_bad_vote_hash);
  result.total_weight = threshold_validation.total_weight;
  result.two_t_plus_one = threshold_validation.two_t_plus_one;
  result.accepted = threshold_validation.valid;
  return result;
}

VoteManager::StartupReplayVoteValidationResult VoteManager::validateStartupReplayVotes(
    const std::vector<std::shared_ptr<PbftVote>>& replay_votes) const {
  StartupReplayVoteValidationResult result;
  for (const auto& vote : replay_votes) {
    if (!vote) {
      result.validation_error = "missing startup replay vote";
      return result;
    }
    const auto validation = validateVote(vote);
    if (!validation.first) {
      result.first_bad_vote_hash = vote->getHash();
      result.validation_error = validation.second;
      return result;
    }
  }

  result.accepted = true;
  return result;
}

VoteManager::PbftVoteAdmissionReport VoteManager::addVerifiedVoteWithReport(const std::shared_ptr<PbftVote>& vote) {
  PbftVoteAdmissionReport report{};
  if (!vote) {
    LOG(log_er_) << "Unable to add vote into the verified queue. Missing vote";
    return report;
  }

  const auto hash = vote->getHash();
  if (vote->getPeriod() == 0) {
    LOG(log_er_) << "Unable to add vote " << hash << " into the verified queue. Invalid zero vote period";
    return report;
  }

  const auto two_t_plus_one = getPbftTwoTPlusOne(vote->getPeriod() - 1, vote->getType());
  const auto progress_context = makeVoteProgressContext(current_pbft_period_, current_pbft_round_, two_t_plus_one);

  const auto canonical_vote_rlp = toBridgeBytes(vote->rlp(true, false));
  const auto inspection = rustaxa::pbft_inspect_canonical_vote(toBridgeByteSlice(canonical_vote_rlp));
  if (inspection.status != kPbftCanonicalVoteInspectionStatusValid) {
    LOG(log_er_) << "VoteManager Rust PBFT vote admission rejected vote " << hash
                 << " during canonical inspection, status: " << static_cast<uint32_t>(inspection.status)
                 << ", error: " << static_cast<std::string>(inspection.error_code);
    if (inspection.status == kPbftCanonicalVoteInspectionStatusInvalidSignature) {
      pbft_service_->service().pbft_service_verified_votes_replay_insert(inspection.vote_hash);
    }
    return report;
  }
  if (toBridgeHash(vote->getHash()) != inspection.vote_hash ||
      toBridgeHash(vote->getBlockHash()) != inspection.block_hash || vote->getPeriod() != inspection.period ||
      vote->getRound() != inspection.round || vote->getStep() != inspection.step ||
      static_cast<uint8_t>(vote->getType()) != inspection.vote_type ||
      toBridgeAddress(vote->getVoterAddr()) != inspection.recovered_voter) {
    throw std::runtime_error("VoteManager Rust PBFT vote admission inspection mismatched live vote sidecar");
  }

  auto admission_request = makeVoteAdmissionValidationRequest(vote, kPbftConfig);
  const auto preverified_weight = vote->getWeight();
  if (preverified_weight.has_value()) {
    if (*preverified_weight == 0) {
      LOG(log_er_) << "Unable to add vote " << hash << " into the verified queue. Invalid vote weight";
      return report;
    }
  }

  const auto runtime_result = pbft_service_->service().pbft_service_verified_votes_admit_and_persist_with_final_chain(
      final_chain_->rustFinalChain(), toBridgeByteSlice(canonical_vote_rlp), std::move(admission_request),
      makeVoteEventFactFlags(), progress_context);
  if (runtime_result.persistence_status == kPbftVoteAdmissionPersistenceRejected) {
    std::stringstream err;
    err << "Rust PBFT vote admission persistence rejected vote " << hash << ": "
        << static_cast<std::string>(runtime_result.error_code);
    LOG(log_er_) << err.str();
    throw std::runtime_error(err.str());
  }
  if ((runtime_result.persistence_required &&
       (runtime_result.persistence_status != kPbftVoteAdmissionPersistenceApplied ||
        runtime_result.persistence_applied_writes == 0)) ||
      (!runtime_result.persistence_required &&
       (runtime_result.persistence_status != kPbftVoteAdmissionPersistenceNotRequired ||
        runtime_result.persistence_applied_writes != 0))) {
    throw std::runtime_error("Rust PBFT vote admission returned inconsistent persistence publication state");
  }
  if (runtime_result.transition_published) {
    requireRuntimeVoteTransitionIntentsMatch(runtime_result, vote);
    report.mark_vote_known = runtime_result.mark_vote_known;
    report.mark_vote_known_hash = fromBridgeVoteHash(runtime_result.mark_vote_known_hash);
    report.gossip_vote = runtime_result.gossip_vote;
    report.gossip_vote_hash = fromBridgeVoteHash(runtime_result.gossip_vote_hash);
    report.report_slashing = runtime_result.report_slashing;
    report.drive_pbft_progress = runtime_result.drive_pbft_progress;
    report.progress_period = runtime_result.progress_period;
    report.progress_round = runtime_result.progress_round;
  } else if (runtime_result.accepted) {
    throw std::runtime_error("Rust PBFT vote admission accepted an unpublished transition");
  }

  if (!runtime_result.has_validation || !runtime_result.validation.accepted ||
      runtime_result.validation.status != kPbftVoteValidationStatusValid) {
    LOG(log_er_) << "VoteManager Rust PBFT vote admission rejected vote " << vote->getHash()
                 << " during validation, status: "
                 << static_cast<uint32_t>(runtime_result.has_validation ? runtime_result.validation.status : 0)
                 << ", error: " << static_cast<std::string>(runtime_result.error_code);
    return report;
  }
  if (!runtime_result.has_vote) {
    LOG(log_er_) << "VoteManager Rust PBFT vote admission rejected vote " << vote->getHash()
                 << ", status: " << static_cast<uint32_t>(runtime_result.status)
                 << ", error: " << static_cast<std::string>(runtime_result.error_code);
    return report;
  }
  if (!runtime_result.validation.has_sortition_threshold || !runtime_result.validation.weight_calculated) {
    throw std::runtime_error("VoteManager Rust PBFT vote admission accepted validation without weight facts");
  }
  if (!runtime_result.transition_published) {
    return report;
  }
  requireRuntimeAdmissionVoteMatches(runtime_result.vote, vote, runtime_result.validation.calculated_weight);
  if (!preverified_weight.has_value()) {
    auto weighted_record = rustaxa::pbft_vote_weighted_payload_from_canonical_vote(
        toBridgeByteSlice(canonical_vote_rlp), runtime_result.validation.calculated_weight);
    PbftVote weighted_vote(fromBridgeBytes(weighted_record.vote_rlp));
    if (weighted_record.hash != toBridgeHash(vote->getHash()) ||
        weighted_vote.rlp(true, false) != fromBridgeBytes(canonical_vote_rlp) ||
        weighted_vote.getHash() != vote->getHash() || weighted_vote.getBlockHash() != vote->getBlockHash() ||
        weighted_vote.getPeriod() != vote->getPeriod() || weighted_vote.getRound() != vote->getRound() ||
        weighted_vote.getStep() != vote->getStep() || weighted_vote.getType() != vote->getType() ||
        weighted_vote.getVoterAddr() != vote->getVoterAddr() || !weighted_vote.getWeight().has_value() ||
        *weighted_vote.getWeight() != runtime_result.validation.calculated_weight) {
      throw std::runtime_error("VoteManager Rust weighted admission payload mismatched the live vote identity");
    }
    *vote = std::move(weighted_vote);
  }

  if (runtime_result.report_slashing) {
    LOG(log_wr_) << "Non unique vote " << vote->getHash().abridged() << " (race condition)";
    SlashingDoubleVoteEvidence evidence;
    evidence.incoming_vote = runtime_result.slashing_incoming_vote;
    evidence.conflicting_vote = runtime_result.slashing_conflicting_vote;
    evidence.period = vote->getPeriod();
    evidence.round = vote->getRound();
    evidence.step = vote->getStep();
    submitRustPlannedSlashingProof(evidence);
    return report;
  }

  if (!runtime_result.accepted) {
    return report;
  }
  if (!runtime_result.has_verified_vote_add || !runtime_result.verified_vote_add.inserted) {
    LOG(log_dg_) << "VoteManager Rust PBFT vote admission accepted vote " << vote->getHash()
                 << " without a new verified-vote insertion";
    return report;
  }
  LOG(log_nf_) << "Added verified vote: " << hash;
  LOG(log_dg_) << "Added verified vote: " << *vote;

  if (!two_t_plus_one.has_value()) [[unlikely]] {
    LOG(log_er_) << "Cannot set(or not) 2t+1 voted block as 2t+1 threshold is unavailable, vote " << vote->getHash();
    report.accepted = true;
    return report;
  }

  if (runtime_result.network_t_plus_one_step_updated) {
    LOG(log_nf_) << "Set t+1 next voted block " << vote->getHash() << " for period " << vote->getPeriod() << ", round "
                 << vote->getRound() << ", step " << vote->getStep();
  }

  report.accepted = true;
  return report;
}

bool VoteManager::voteInVerifiedMap(std::shared_ptr<PbftVote> const& vote) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_step_payloads(
      vote->getPeriod(), vote->getRound(), vote->getStep());
  if (!lookup.found) {
    return false;
  }
  const auto step_votes_map = materializeStepVotes(lookup, vote->getPeriod(), vote->getRound(), vote->getStep());

  const auto found_voted_value_it = step_votes_map.votes.find(vote->getBlockHash());
  if (found_voted_value_it == step_votes_map.votes.end()) {
    return false;
  }

  return found_voted_value_it->second.votes.find(vote->getHash()) != found_voted_value_it->second.votes.end();
}

std::pair<bool, std::shared_ptr<PbftVote>> VoteManager::isUniqueVote(const std::shared_ptr<PbftVote>& vote) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_step_payloads(
      vote->getPeriod(), vote->getRound(), vote->getStep());
  if (!lookup.found) {
    return {true, nullptr};
  }
  const auto step_votes_map = materializeStepVotes(lookup, vote->getPeriod(), vote->getRound(), vote->getStep());

  const auto found_voter_it = step_votes_map.unique_voters.find(vote->getVoterAddr());
  if (found_voter_it == step_votes_map.unique_voters.end()) {
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

std::vector<std::shared_ptr<PbftVote>> VoteManager::getVerifiedVotes() const {
  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_state_snapshot();
  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(snapshot.votes.size());
  for (const auto& entry : snapshot.votes) {
    auto vote = materializeWeightedVote(entry.weighted_vote);
    if (vote->getBlockHash() != fromBridgeHash(entry.vote.block_hash) || vote->getPeriod() != entry.vote.period ||
        vote->getRound() != entry.vote.round || vote->getStep() != entry.vote.step ||
        static_cast<uint8_t>(vote->getType()) != entry.vote.vote_type || *vote->getWeight() != entry.vote.weight) {
      throw verifiedVoteViewError("native snapshot metadata mismatches retained payload");
    }
    votes.push_back(std::move(vote));
  }
  return votes;
}

uint64_t VoteManager::getVerifiedVotesSize() const {
  return pbft_service_->service().pbft_service_verified_votes_size();
}

void VoteManager::cleanupVotesByPeriod(PbftPeriod pbft_period) {
  pbft_service_->service().pbft_service_verified_votes_cleanup_votes_by_period(pbft_period);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getProposalVotes(PbftPeriod period, PbftRound round) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_step_payloads(
      period, round, PbftStates::value_proposal_state);
  if (!lookup.found) {
    return {};
  }
  const auto step_votes = materializeStepVotes(lookup, period, round, PbftStates::value_proposal_state);

  std::vector<std::shared_ptr<PbftVote>> proposal_votes;
  for (const auto& voted_value : step_votes.votes) {
    for (const auto& vote_pair : voted_value.second.votes) {
      proposal_votes.emplace_back(vote_pair.second);
    }
  }

  return proposal_votes;
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> VoteManager::identifyLeaderBlock(
    PbftPeriod period, PbftRound round,
    const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const {
  const auto snapshot = pbft_service_->service().pbft_service_prepare_leader_selection(period, round);
  if (snapshot.status != kPbftLeaderSelectionReadyOrSelected) {
    const auto error = static_cast<std::string>(snapshot.error_code);
    if (snapshot.status == kPbftLeaderSelectionNoCandidates) {
      LOG(log_dg_) << "Rust PBFT leader selection found no proposal candidates for period " << period << ", round "
                   << round << ": " << error;
    } else {
      LOG(log_er_) << "Rust PBFT leader selection snapshot failed for period " << period << ", round " << round
                   << ", status " << static_cast<uint32_t>(snapshot.status) << ": " << error;
    }
    return {};
  }

  rustaxa::PbftLeaderSelectionFinishRequest request;
  request.period = period;
  request.round = round;
  request.snapshot_fingerprint = snapshot.snapshot_fingerprint;
  request.validations.reserve(snapshot.candidates.size());
  for (const auto& candidate : snapshot.candidates) {
    if (!candidate.needs_external_validation) {
      continue;
    }

    auto block = std::make_shared<PbftBlock>(fromBridgeBytes(candidate.proposed_block_rlp));
    if (block->getBlockHash() != fromBridgeHash(candidate.block_hash)) {
      throw std::runtime_error("Rust PBFT leader snapshot block payload mismatches candidate identity");
    }

    rustaxa::PbftLeaderCandidateValidation validation;
    validation.vote_hash = candidate.vote_hash;
    validation.block_hash = candidate.block_hash;
    validation.status = validate_block(block) ? kPbftManagerLeaderBlockValidated : kPbftManagerLeaderBlockRejected;
    request.validations.push_back(std::move(validation));
  }

  const auto result = pbft_service_->service().pbft_service_finish_leader_selection(std::move(request));
  if (result.status != kPbftLeaderSelectionReadyOrSelected) {
    const auto error = static_cast<std::string>(result.error_code);
    if (result.status == kPbftLeaderSelectionNoEligibleLeader || result.status == kPbftLeaderSelectionStaleSnapshot) {
      LOG(log_dg_) << "Rust PBFT leader selection returned no leader for period " << period << ", round " << round
                   << ", status " << static_cast<uint32_t>(result.status) << ": " << error;
    } else if (result.status == kPbftLeaderSelectionInvalidValidationReport ||
               result.status == kPbftLeaderSelectionServiceUnavailable) {
      LOG(log_er_) << "Rust PBFT leader selection rejected the validation report for period " << period << ", round "
                   << round << ", status " << static_cast<uint32_t>(result.status) << ": " << error;
    } else {
      LOG(log_er_) << "Rust PBFT leader selection returned unexpected status " << static_cast<uint32_t>(result.status)
                   << " for period " << period << ", round " << round << ": " << error;
    }
    return {};
  }
  if (!result.selected) {
    throw std::runtime_error("Rust PBFT leader selection returned selected status without an owned leader payload");
  }

  auto vote = materializeOwnVoteRecord(result.selected_vote);
  auto block = std::make_shared<PbftBlock>(fromBridgeBytes(result.selected_block_rlp));
  if (vote->getPeriod() != period || block->getPeriod() != period || vote->getBlockHash() != block->getBlockHash()) {
    throw std::runtime_error("Rust PBFT leader selection returned inconsistent owned vote and block payloads");
  }
  return std::pair{std::move(block), std::move(vote)};
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> VoteManager::identifyLeaderBlock(
    std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>&& local_candidates,
    const std::function<bool(const blk_hash_t&)>& block_in_chain,
    const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const {
  return identifyLeaderBlockFromLocalCandidates(std::move(local_candidates), block_in_chain, validate_block);
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>
VoteManager::identifyLeaderBlockFromLocalCandidates(
    std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>>&& local_candidates,
    const std::function<bool(const blk_hash_t&)>& block_in_chain,
    const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const {
  std::vector<std::shared_ptr<PbftVote>> propose_votes;
  propose_votes.reserve(local_candidates.size());
  for (const auto& candidate : local_candidates) {
    propose_votes.push_back(candidate.second);
  }
  if (propose_votes.empty()) {
    return {};
  }

  rust::Vec<rustaxa::PbftManagerLeaderCandidateInputFact> candidate_facts;
  candidate_facts.reserve(propose_votes.size());
  std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> materialized_candidates;
  rust::Vec<rustaxa::ProposedBlockLookup> local_lookups;
  rust::Vec<rustaxa::ProposedBlockSnapshotEntry> candidate_entries;
  candidate_entries.reserve(local_candidates.size());
  for (const auto& candidate : local_candidates) {
    rustaxa::ProposedBlockSnapshotEntry entry;
    entry.period = candidate.first->getPeriod();
    entry.block_hash = toBridgeHash(candidate.first->getBlockHash());
    entry.pivot_hash = toBridgeHash(candidate.first->getPivotDagBlockHash());
    entry.block_rlp = toBridgeBytes(candidate.first->rlp(true));
    entry.is_valid = false;
    candidate_entries.push_back(std::move(entry));
  }
  rust::Vec<rustaxa::ProposedBlockIdentity> identities;
  identities.reserve(propose_votes.size());
  for (const auto& vote : propose_votes) {
    identities.push_back(rustaxa::ProposedBlockIdentity{vote->getPeriod(), toBridgeHash(vote->getBlockHash())});
  }
  local_lookups = rustaxa::proposed_blocks_local_candidate_lookups(std::move(candidate_entries), std::move(identities));
  if (local_lookups.size() != propose_votes.size()) {
    throw std::runtime_error("Rust local proposed-block lookup returned a misaligned result set");
  }

  for (size_t vote_index = 0; vote_index < propose_votes.size(); ++vote_index) {
    auto&& vote = propose_votes[vote_index];
    rustaxa::PbftManagerLeaderCandidateInputFact fact;
    fact.vote_hash = toBridgeHash(vote->getHash());
    fact.block_hash = toBridgeHash(vote->getBlockHash());
    fact.period = vote->getPeriod();
    fact.credential = toBridgeFixedBytes<64>(vote->getCredential());
    fact.voter_public_key = toBridgeFixedBytes<64>(vote->getVoter());
    fact.weight_found = false;
    fact.weight = 0;
    fact.block_in_chain = false;
    fact.proposed_block_found = false;
    fact.block_validation_status = kPbftManagerLeaderBlockAlreadyValid;
    fact.pivot_hash = toBridgeHash(kNullBlockHash);

    const auto weight = vote->getWeight();
    if (!weight.has_value() || *weight == 0) {
      candidate_facts.push_back(fact);
      continue;
    }
    fact.weight_found = true;
    fact.weight = *weight;

    const auto proposed_block_hash = vote->getBlockHash();
    if (proposed_block_hash == kNullBlockHash) {
      LOG(log_er_) << "Propose block hash should not be NULL. Vote " << vote;
      candidate_facts.push_back(fact);
      continue;
    }

    if (block_in_chain(proposed_block_hash)) {
      fact.block_in_chain = true;
      candidate_facts.push_back(fact);
      continue;
    }

    auto proposed_block = std::move(local_lookups[vote_index]);
    if (!proposed_block.found) {
      LOG(log_er_) << "Unable to get proposed block " << proposed_block_hash;
      candidate_facts.push_back(fact);
      continue;
    }
    fact.proposed_block_found = true;
    fact.pivot_hash = proposed_block.pivot_hash;
    auto materialized_block = std::make_shared<PbftBlock>(fromBridgeBytes(proposed_block.block_rlp));

    if (proposed_block.is_valid) {
      fact.block_validation_status = kPbftManagerLeaderBlockAlreadyValid;
    } else if (validate_block(materialized_block)) {
      fact.block_validation_status = kPbftManagerLeaderBlockValidated;
    } else {
      fact.block_validation_status = kPbftManagerLeaderBlockRejected;
    }

    if (fact.block_validation_status != kPbftManagerLeaderBlockRejected) {
      materialized_candidates.emplace_back(std::move(materialized_block), std::move(vote));
    }
    candidate_facts.push_back(fact);
  }

  const auto plan = rustaxa::plan_pbft_manager_leader_candidates(std::move(candidate_facts));
  if (!plan.selected) {
    if (plan.status == kPbftManagerLeaderSelectionInvalidFact) {
      LOG(log_er_) << "Rust PBFT leader candidate planner rejected proposal facts: "
                   << static_cast<std::string>(plan.error_code);
    }
    return {};
  }

  const auto selected_vote_hash = fromBridgeHash(plan.selected_vote_hash);
  const auto selected_block_hash = fromBridgeHash(plan.selected_block_hash);
  for (auto&& candidate : materialized_candidates) {
    if (candidate.second->getHash() == selected_vote_hash && candidate.first->getBlockHash() == selected_block_hash) {
      return std::move(candidate);
    }
  }

  LOG(log_er_) << "Rust PBFT leader candidate planner selected missing live proposal vote " << selected_vote_hash
               << " for block " << selected_block_hash;
  return {};
}

std::optional<PbftRound> VoteManager::determineNewRound(PbftPeriod current_pbft_period, PbftRound current_pbft_round) {
  const auto decision = roundAdvanceDecision(current_pbft_period, current_pbft_round);
  if (!decision.has_new_round) {
    return {};
  }
  return decision.new_round;
}

VoteManager::RoundAdvanceDecision VoteManager::roundAdvanceDecision(PbftPeriod current_pbft_period,
                                                                    PbftRound current_pbft_round) {
  RoundAdvanceDecision result;
  const auto decision =
      pbft_service_->service().pbft_service_verified_votes_determine_new_round(current_pbft_period, current_pbft_round);
  if (!decision.found) {
    return result;
  }

  LOG(log_nf_) << "New round " << decision.new_round << " determined for period " << current_pbft_period
               << ". Found 2t+1 votes for block " << fromBridgeHash(decision.block_hash) << " in round "
               << decision.source_round << ", step " << decision.step;

  result.has_new_round = true;
  result.new_round = decision.new_round;
  return result;
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
  auto result = checkRewardVotesDetailed(pbft_block, copy_votes);
  return {result.accepted, std::move(result.votes)};
}

VoteManager::RewardVoteValidationResult VoteManager::checkRewardVotesDetailed(
    const std::shared_ptr<PbftBlock>& pbft_block, bool copy_votes) {
  return checkRewardVotesDetailed(pbft_block->getPeriod(), pbft_block->getBlockHash(), pbft_block->getPrevBlockHash(),
                                  pbft_block->getRewardVotes(), copy_votes);
}

std::pair<bool, std::vector<std::shared_ptr<PbftVote>>> VoteManager::checkRewardVotes(
    PbftPeriod block_period, const blk_hash_t& block_hash, const blk_hash_t& prev_block_hash,
    const std::vector<vote_hash_t>& reward_vote_hashes, bool copy_votes) {
  auto result = checkRewardVotesDetailed(block_period, block_hash, prev_block_hash, reward_vote_hashes, copy_votes);
  return {result.accepted, std::move(result.votes)};
}

VoteManager::RewardVoteValidationResult VoteManager::checkRewardVotesDetailed(
    PbftPeriod block_period, const blk_hash_t& block_hash, const blk_hash_t& prev_block_hash,
    const std::vector<vote_hash_t>& reward_vote_hashes, bool copy_votes) {
  const auto cursor = pbft_service_->service().pbft_service_verified_votes_reward_vote_cursor();

  rustaxa::PbftRewardVotePayloadSelection selection{};
  std::vector<std::shared_ptr<PbftVote>> selected_votes;
  try {
    selection = pbft_service_->service().pbft_service_verified_votes_select_reward_vote_payloads(
        block_period, toBridgeVoteHashes(reward_vote_hashes));
    if (selection.accepted && copy_votes) {
      selected_votes.reserve(selection.selected_records.size());
      const auto expected_block_hash = fromBridgeHash(selection.selected_block_hash);
      for (const auto& record : selection.selected_records) {
        auto vote = materializeWeightedVote(record);
        if (vote->getPeriod() != selection.selected_period || vote->getRound() != selection.selected_round ||
            vote->getStep() != static_cast<PbftStep>(PbftVoteTypes::cert_vote) ||
            vote->getBlockHash() != expected_block_hash) {
          throw verifiedVoteViewError("native reward-vote selection returned mismatched weighted payload");
        }
        selected_votes.push_back(std::move(vote));
      }
    }
  } catch (const std::exception& e) {
    LOG(log_er_) << "Rust reward-vote payload selection failed for block " << block_hash << ", period: " << block_period
                 << ", reward cursor found: " << cursor.found << ", reward cursor period: " << cursor.period
                 << ", reward cursor round: " << cursor.round
                 << ", reward cursor block hash: " << fromBridgeHash(cursor.block_hash) << ", error: " << e.what();
    assert(false);
    RewardVoteValidationResult result;
    result.error_code = e.what();
    return result;
  }

  const auto& plan = selection;
  RewardVoteValidationResult result;
  result.accepted = plan.accepted;
  result.status = plan.status;
  result.error_code = static_cast<std::string>(plan.error_code);
  result.selected_period = plan.selected_period;
  result.selected_round = plan.selected_round;
  result.selected_block_hash = fromBridgeHash(plan.selected_block_hash);
  result.missing_vote_hash = fromBridgeHash(plan.missing_vote_hash);
  result.votes = std::move(selected_votes);

  if (!plan.accepted) {
    LOG(log_er_) << "No (or not enough) reward votes found for block " << block_hash << ", period: " << block_period
                 << ", prev. block hash: " << prev_block_hash << ", reward cursor found: " << cursor.found
                 << ", reward cursor period: " << cursor.period << ", reward cursor round: " << cursor.round
                 << ", selected_round: " << plan.selected_round
                 << ", reward cursor block hash: " << fromBridgeHash(cursor.block_hash)
                 << ", status: " << static_cast<uint32_t>(plan.status)
                 << ", error: " << static_cast<std::string>(plan.error_code);
    return result;
  }

  return result;
}

bool VoteManager::validateRewardVotesForBlock(const std::shared_ptr<PbftBlock>& pbft_block) {
  return checkRewardVotesDetailed(pbft_block, false).accepted;
}

std::optional<std::vector<std::shared_ptr<PbftVote>>> VoteManager::collectRewardVotesForBlock(
    const std::shared_ptr<PbftBlock>& pbft_block) {
  auto result = checkRewardVotesDetailed(pbft_block, true);
  if (!result.accepted) {
    return {};
  }
  return std::move(result.votes);
}

std::optional<std::vector<std::shared_ptr<PbftVote>>> VoteManager::collectRewardVotesForBlock(
    PbftPeriod block_period, const blk_hash_t& block_hash, const blk_hash_t& prev_block_hash,
    const std::vector<vote_hash_t>& reward_vote_hashes) {
  auto result = checkRewardVotesDetailed(block_period, block_hash, prev_block_hash, reward_vote_hashes, true);
  if (!result.accepted) {
    return {};
  }
  return std::move(result.votes);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getRewardVotes() {
  try {
    const auto snapshot = pbft_service_->service().pbft_service_verified_votes_current_reward_snapshot();
    std::vector<std::shared_ptr<PbftVote>> votes;
    votes.reserve(snapshot.records.size());
    for (const auto& record : snapshot.records) {
      auto vote = materializeWeightedVote(record);
      if (!snapshot.cursor.found || vote->getPeriod() != snapshot.cursor.period ||
          vote->getRound() != snapshot.cursor.round || vote->getStep() != snapshot.cursor.step ||
          vote->getBlockHash() != fromBridgeHash(snapshot.cursor.block_hash)) {
        throw verifiedVoteViewError("native current reward-vote payload mismatches authoritative cursor");
      }
      votes.push_back(std::move(vote));
    }
    return votes;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Rust current reward-vote payload lookup failed: " << e.what();
    assert(false);
    return {};
  }
}

VoteManager::ProposalRewardVotes VoteManager::proposalRewardVotesForPeriod(PbftPeriod propose_period) {
  ProposalRewardVotes result;
  result.reward_votes = getRewardVotes();
  result.reward_vote_hashes.reserve(result.reward_votes.size());
  for (const auto& vote : result.reward_votes) {
    if (!vote) {
      result.validation_error = "reward-vote payload contains a null vote";
      return result;
    }
    result.reward_vote_hashes.push_back(vote->getHash());
  }

  if (propose_period <= 1) {
    result.valid = true;
    return result;
  }

  if (result.reward_votes.empty()) {
    result.validation_error = "missing reward votes for non-genesis proposal";
    return result;
  }

  const auto reward_vote_period = result.reward_votes.front()->getPeriod();
  if (reward_vote_period != propose_period - 1) {
    std::stringstream err;
    err << "reward vote period(" << reward_vote_period << ") != propose_period - 1(" << propose_period - 1 << ")";
    result.validation_error = err.str();
    return result;
  }

  result.valid = true;
  return result;
}

PbftPeriod VoteManager::getRewardVotesPbftBlockPeriod() {
  return pbft_service_->service().pbft_service_verified_votes_reward_vote_period();
}

void VoteManager::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot persist a null own verified vote");
  }
  auto record = makeVoteStorageRecord(vote);
  requireApplied(pbft_service_->service().pbft_service_verified_votes_save_own_verified_vote(std::move(record)),
                 "own verified vote");
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getOwnVerifiedVotes() {
  const auto records = pbft_service_->service().pbft_service_verified_votes_own_vote_records();
  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(records.size());
  for (const auto& record : records) {
    votes.push_back(materializeOwnVoteRecord(record));
  }
  return votes;
}

void VoteManager::clearOwnVerifiedVotes(Batch& write_batch) {
  (void)write_batch;
  requireApplied(pbft_service_->service().pbft_service_verified_votes_clear_own_verified_votes(),
                 "own verified vote cleanup");
}

std::shared_ptr<PbftVote> VoteManager::generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                              PbftPeriod period, PbftRound round, PbftStep step,
                                                              const WalletConfig& wallet) {
  const auto generation_input = makeVoteGenerationInput(blockhash, vote_type, period, round, step, wallet);
  try {
    const auto generated = pbft_service_->service().pbft_service_generate_signed_vote_with_weight(
        final_chain_->rustFinalChain(), generation_input, kPbftConfig.committee_size, kPbftConfig.number_of_proposers);
    if (generated.status == kPbftVoteGenerationStatusZeroStake) {
      requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroStake, "zero-stake weighted vote");
      return nullptr;
    }
    if (generated.status == kPbftVoteGenerationStatusZeroTotalDpos) {
      requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroTotalDpos, "zero-total weighted vote");
      return nullptr;
    }
    if (generated.status == kPbftVoteGenerationStatusZeroWeight) {
      requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroWeight, "zero-weight weighted vote");
      return nullptr;
    }
    return materializeRustGeneratedVote(generated, wallet, true);
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to place vote for period: " << period << ", round: " << round << ", step: " << step
                 << ", voted block hash: " << blockhash.abridged() << ". Err msg: " << e.what();
    return nullptr;
  }
}

VoteManager::LocallyGeneratedVotePlacement VoteManager::generateAndPlaceLocalVote(const blk_hash_t& block_hash,
                                                                                  PbftVoteTypes vote_type,
                                                                                  PbftPeriod period, PbftRound round,
                                                                                  PbftStep step,
                                                                                  const WalletConfig& wallet) {
  LocallyGeneratedVotePlacement result;
  result.vote = generateVoteWithWeight(block_hash, vote_type, period, round, step, wallet);
  if (!result.vote) {
    std::stringstream err;
    err << "Failed to generate vote for " << block_hash << ", period " << period << ", round " << round << ", step "
        << step << ", validator " << wallet.node_addr;
    result.error = err.str();
    return result;
  }

  if (!addLocallyGeneratedVote(result.vote)) {
    std::stringstream err;
    err << "Unable to place vote " << result.vote->getHash() << " for block " << block_hash << ", period " << period
        << ", round " << round << ", step " << step << ", validator " << wallet.node_addr;
    result.error = err.str();
    result.vote.reset();
    return result;
  }

  result.placed = true;
  return result;
}

VoteManager::LocalProposalVoteGeneration VoteManager::generateUniqueProposalVoteForBlock(const blk_hash_t& block_hash,
                                                                                         PbftPeriod period,
                                                                                         PbftRound round, PbftStep step,
                                                                                         const WalletConfig& wallet) {
  LocalProposalVoteGeneration result;
  result.vote = generateVoteWithWeight(block_hash, PbftVoteTypes::propose_vote, period, round, step, wallet);
  if (!result.vote) {
    std::stringstream err;
    err << "Failed to generate propose vote for block " << block_hash << ", period " << period << ", round " << round
        << ", step " << step << ", validator " << wallet.node_addr;
    result.error = err.str();
    return result;
  }

  if (!isUniqueVote(result.vote).first) {
    std::stringstream err;
    err << "Non unique propose vote " << result.vote->getHash() << " for block " << block_hash << ", period " << period
        << ", round " << result.vote->getRound() << ", step " << result.vote->getStep() << ", validator "
        << wallet.node_addr;
    result.error = err.str();
    result.vote.reset();
    return result;
  }

  result.generated = true;
  return result;
}

std::shared_ptr<PbftVote> VoteManager::generateVote(const blk_hash_t& blockhash, PbftVoteTypes type, PbftPeriod period,
                                                    PbftRound round, PbftStep step, const WalletConfig& wallet) {
  const auto generated =
      rustaxa::pbft_generate_signed_vote(makeVoteGenerationInput(blockhash, type, period, round, step, wallet));
  return materializeRustGeneratedVote(generated, wallet, false);
}

std::pair<bool, std::string> VoteManager::validateVote(const std::shared_ptr<PbftVote>& vote, bool strict) const {
  if (!vote) {
    return {false, "Invalid vote: null vote"};
  }

  std::stringstream err_msg;
  const auto canonical_vote_rlp = toBridgeBytes(vote->rlp(true, false));
  const auto inspection = rustaxa::pbft_inspect_canonical_vote(toBridgeByteSlice(canonical_vote_rlp));
  if (inspection.status == kPbftCanonicalVoteInspectionStatusMalformedRlp) {
    err_msg << "Invalid vote " << vote->getHash() << ": malformed canonical PBFT vote RLP";
    return {false, err_msg.str()};
  }

  if (toBridgeHash(vote->getHash()) != inspection.vote_hash ||
      toBridgeHash(vote->getBlockHash()) != inspection.block_hash || vote->getPeriod() != inspection.period ||
      vote->getRound() != inspection.round || vote->getStep() != inspection.step ||
      static_cast<uint8_t>(vote->getType()) != inspection.vote_type) {
    err_msg << "Invalid vote " << vote->getHash()
            << ": Rust canonical PBFT vote inspection mismatched C++ vote identity";
    return {false, err_msg.str()};
  }

  if (inspection.status == kPbftCanonicalVoteInspectionStatusInvalidSignature) {
    pbft_service_->service().pbft_service_verified_votes_replay_insert(inspection.vote_hash);
    err_msg << "Invalid vote " << vote->getHash() << ": invalid signature";
    return {false, err_msg.str()};
  }

  if (inspection.status != kPbftCanonicalVoteInspectionStatusValid) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown Rust canonical PBFT vote inspection status "
            << static_cast<uint32_t>(inspection.status);
    return {false, err_msg.str()};
  }

  const auto recovered_voter = fromBridgeAddress(inspection.recovered_voter);
  rustaxa::PbftVoteRuntimeValidationResult validation_result{};
  rustaxa::PbftCanonicalVoteValidation validation{};

  try {
    validation_result = pbft_service_->service().pbft_service_verified_votes_validate_with_final_chain(
        final_chain_->rustFinalChain(), toBridgeByteSlice(canonical_vote_rlp), strict, kPbftConfig.committee_size,
        kPbftConfig.number_of_proposers);
    validation = validation_result.validation;

    if (validation.status == kPbftVoteValidationStatusZeroStake) {
      err_msg << "Invalid vote " << vote->getHash() << ": author " << recovered_voter << " has zero stake";
      return {false, err_msg.str()};
    }

    if (validation.status == kPbftVoteValidationStatusInvalidVoteType) {
      err_msg << "Invalid vote " << vote->getHash() << ": invalid PBFT vote type";
      return {false, err_msg.str()};
    }

    if (validation.status == kPbftVoteValidationStatusMissingVrfKey) {
      err_msg << "No vrf key mapped for vote author " << recovered_voter;
      return {false, err_msg.str()};
    }

    if (validation.status == kPbftVoteValidationStatusInvalidSignature) {
      err_msg << "Invalid vote " << vote->getHash() << ": invalid signature";
      return {false, err_msg.str()};
    }

    if (validation.status == kPbftVoteValidationStatusInvalidVrfProof) {
      err_msg << "Invalid vote " << vote->getHash() << ": invalid vrf proof";
      return {false, err_msg.str()};
    }

    if (validation.status == kPbftVoteValidationStatusZeroWeight) {
      err_msg << "Invalid vote " << vote->getHash() << ": zero weight";
      return {false, err_msg.str()};
    }

  } catch (const std::exception& e) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation. " << e.what();
    return {false, err_msg.str()};
  } catch (...) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation";
    return {false, err_msg.str()};
  }

  if (validation.status != kPbftVoteValidationStatusValid || !validation.accepted) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation";
    return {false, err_msg.str()};
  }
  if (!validation.has_sortition_threshold) {
    throw std::runtime_error("Rust PBFT vote validation accepted a vote without a sortition threshold");
  }
  if (!validation.weight_calculated) {
    throw std::runtime_error("Rust PBFT vote validation accepted validation facts without a calculated weight");
  }
  if (!validation_result.has_weighted_vote || validation_result.weighted_vote_rlp.empty()) {
    throw std::runtime_error("Rust PBFT vote validation accepted a vote without a weighted payload");
  }

  if (!vote->getWeight().has_value()) {
    if (validation.calculated_weight == 0) {
      err_msg << "Invalid vote " << vote->getHash() << ": zero weight";
      return {false, err_msg.str()};
    }
    PbftVote weighted_vote(fromBridgeBytes(validation_result.weighted_vote_rlp));
    if (weighted_vote.rlp(true, false) != fromBridgeBytes(canonical_vote_rlp) ||
        weighted_vote.getHash() != vote->getHash() || weighted_vote.getBlockHash() != vote->getBlockHash() ||
        weighted_vote.getPeriod() != vote->getPeriod() || weighted_vote.getRound() != vote->getRound() ||
        weighted_vote.getStep() != vote->getStep() || weighted_vote.getType() != vote->getType() ||
        weighted_vote.getVoterAddr() != vote->getVoterAddr() || !weighted_vote.getWeight().has_value() ||
        *weighted_vote.getWeight() != validation.calculated_weight) {
      throw std::runtime_error("Rust weighted PBFT vote payload mismatches the live vote identity");
    }
    *vote = std::move(weighted_vote);
  } else if (*vote->getWeight() != validation.calculated_weight) {
    err_msg << "Invalid vote " << vote->getHash() << ": Rust calculated weight " << validation.calculated_weight
            << " mismatches live vote weight " << vote->getWeight().value_or(0);
    return {false, err_msg.str()};
  }

  return {true, ""};
}

std::optional<uint64_t> VoteManager::getPbftTwoTPlusOne(PbftPeriod pbft_period, PbftVoteTypes vote_type) const {
  rustaxa::PbftTwoTPlusOneThresholdFact threshold_fact{};
  threshold_fact.pbft_period = pbft_period;
  threshold_fact.vote_type = static_cast<uint8_t>(vote_type);
  threshold_fact.committee_size = kPbftConfig.committee_size;
  threshold_fact.number_of_proposers = kPbftConfig.number_of_proposers;

  rustaxa::PbftTwoTPlusOneThresholdPlan threshold_plan{};
  try {
    threshold_plan = pbft_service_->service().pbft_service_verified_votes_two_t_plus_one_threshold_with_final_chain(
        final_chain_->rustFinalChain(), threshold_fact);
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period << ". Err msg: " << e.what()
                 << ". Rust composed threshold lookup failed";
    return {};
  } catch (...) {
    LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period
                 << ". Unknown error during Rust composed threshold lookup";
    return {};
  }

  if (threshold_plan.status == kPbftTwoTPlusOneThresholdStatusAvailable && threshold_plan.has_threshold) {
    return threshold_plan.threshold;
  }

  LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period << ". Rust threshold status "
               << static_cast<uint32_t>(threshold_plan.status) << " error "
               << static_cast<std::string>(threshold_plan.error_code);
  return {};
}

bool VoteManager::voteAlreadyValidated(const vote_hash_t& vote_hash) const {
  const auto bridge_hash = toBridgeHash(vote_hash);
  return pbft_service_->service().pbft_service_verified_votes_replay_contains(bridge_hash);
}

bool VoteManager::genAndValidateVrfSortition(PbftPeriod pbft_period, PbftRound pbft_round,
                                             const WalletConfig& wallet) const {
  try {
    auto sortition_request = makeProposerSortitionRequest(pbft_period, pbft_round, wallet, kPbftConfig);
    const auto sortition_result = pbft_service_->service().pbft_service_generate_and_validate_proposer_sortition(
        final_chain_->rustFinalChain(), std::move(sortition_request));
    if (sortition_result.accepted) {
      return true;
    }

    if (sortition_result.status == kPbftProposerSortitionStatusFutureDposState) {
      LOG(log_er_) << "Unable to generate proposer VRF sortition for period " << pbft_period << ", round " << pbft_round
                   << ". Period is too far ahead of actual finalized pbft chain size. Err msg: "
                   << static_cast<std::string>(sortition_result.error_code);
      return false;
    }

    LOG(log_dg_) << "Generated proposer VRF sortition for period " << pbft_period << ", round " << pbft_round
                 << " is invalid. Status: " << static_cast<uint32_t>(sortition_result.status)
                 << ", error: " << static_cast<std::string>(sortition_result.error_code);
    return false;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to generate proposer VRF sortition for period " << pbft_period << ", round " << pbft_round
                 << ". Err msg: " << e.what();
    return false;
  } catch (...) {
    LOG(log_er_) << "Unable to generate proposer VRF sortition for period " << pbft_period << ", round " << pbft_round;
    return false;
  }
}

VoteManager::ProposalWalletFacts VoteManager::proposalWalletFacts(
    PbftPeriod pbft_period, PbftRound pbft_round, const std::vector<std::pair<bool, WalletConfig>>& wallets) const {
  ProposalWalletFacts result;
  result.local_wallets.reserve(wallets.size());
  result.wallet_facts.reserve(wallets.size());

  uint64_t wallet_index = 0;
  for (const auto& wallet : wallets) {
    result.local_wallets.push_back(wallet.second);

    rustaxa::PbftManagerProposalWalletFact wallet_fact;
    wallet_fact.wallet_index = wallet_index;
    wallet_fact.dpos_eligible = wallet.first;
    wallet_fact.sortition_valid = false;
    if (wallet.first) {
      wallet_fact.sortition_valid = genAndValidateVrfSortition(pbft_period, pbft_round, wallet.second);
      if (!wallet_fact.sortition_valid) {
        LOG(log_dg_) << "Unable to propose block for period " << pbft_period << ", round " << pbft_round
                     << ", validator " << wallet.second.node_addr << ". Invalid vrf sortition";
      }
    }
    result.wallet_facts.push_back(wallet_fact);
    wallet_index++;
  }

  return result;
}

std::optional<blk_hash_t> VoteManager::getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                                TwoTPlusOneVotedBlockType type) const {
  const auto voted_block = pbft_service_->service().pbft_service_verified_votes_get_two_t_plus_one_voted_block(
      period, round, static_cast<uint8_t>(type));
  if (!voted_block.found) {
    return {};
  }
  return fromBridgeHash(voted_block.block_hash);
}

VoteManager::StateActionVoteFacts VoteManager::stateActionVoteFacts(PbftPeriod period, PbftRound round,
                                                                    bool needs_previous_round_next_null,
                                                                    bool needs_previous_round_next_value,
                                                                    bool needs_current_round_soft) const {
  StateActionVoteFacts facts;
  if (round >= 2 && needs_previous_round_next_null) {
    facts.has_previous_round_next_null =
        getTwoTPlusOneVotedBlock(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value();
  }

  if (round >= 2 && needs_previous_round_next_value) {
    if (const auto previous_round_next_value =
            getTwoTPlusOneVotedBlock(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedBlock);
        previous_round_next_value.has_value()) {
      facts.has_previous_round_next_value = true;
      facts.previous_round_next_value_hash = *previous_round_next_value;
    }
  }

  if (needs_current_round_soft) {
    if (const auto current_round_soft_value =
            getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock);
        current_round_soft_value.has_value()) {
      facts.has_current_round_soft_value = true;
      facts.current_round_soft_value_hash = *current_round_soft_value;
    }
  }

  return facts;
}

VoteManager::PreviousRoundNextVoteLogFacts VoteManager::previousRoundNextVoteLogFacts(PbftPeriod period,
                                                                                      PbftRound previous_round) const {
  PreviousRoundNextVoteLogFacts facts;
  facts.next_voted_block = getTwoTPlusOneVotedBlock(period, previous_round, TwoTPlusOneVotedBlockType::NextVotedBlock);
  facts.next_voted_null_block =
      getTwoTPlusOneVotedBlock(period, previous_round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value();
  return facts;
}

VoteManager::PreviousRoundNextVoteLogFacts VoteManager::applyStartupPeriodRoundAndLogFacts(PbftPeriod period,
                                                                                           PbftRound round) {
  setCurrentPbftPeriodAndRound(period, round);
  return previousRoundNextVoteLogFacts(period, round - 1);
}

void VoteManager::applyRustPlannedPeriodRound(PbftPeriod pbft_period, PbftRound pbft_round) {
  setCurrentPbftPeriodAndRound(pbft_period, pbft_round);
}

VoteManager::CertVotedBlockSelection VoteManager::certVotedBlockSelection(PbftPeriod period, PbftRound round) const {
  CertVotedBlockSelection selection;
  const auto cert_voted_block = getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (!cert_voted_block) {
    return selection;
  }

  auto votes = getTwoTPlusOneVotedBlockVotes(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (votes.empty()) {
    return selection;
  }

  selection.found = true;
  selection.block_hash = *cert_voted_block;
  selection.votes = std::move(votes);
  return selection;
}

VoteManager::StuckRoundVoteBroadcastPayloads VoteManager::stuckRoundVoteBroadcastPayloads(PbftPeriod period,
                                                                                          PbftRound round) {
  StuckRoundVoteBroadcastPayloads payloads;
  payloads.reward_votes = getRewardVotes();
  payloads.own_votes = getOwnVerifiedVotes();
  payloads.soft_votes = getTwoTPlusOneVotedBlockVotes(period, round, TwoTPlusOneVotedBlockType::SoftVotedBlock);
  if (round > 1) {
    payloads.previous_round_next_votes =
        getTwoTPlusOneVotedBlockVotes(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedBlock);
    payloads.previous_round_next_null_votes =
        getTwoTPlusOneVotedBlockVotes(period, round - 1, TwoTPlusOneVotedBlockType::NextVotedNullBlock);
  }
  return payloads;
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getTwoTPlusOneVotedBlockVotes(
    PbftPeriod period, PbftRound round, TwoTPlusOneVotedBlockType type) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
      period, round, static_cast<uint8_t>(type));
  if (!lookup.found) {
    return {};
  }

  std::vector<std::shared_ptr<PbftVote>> votes;
  votes.reserve(lookup.votes.size());
  const auto expected_block_hash = fromBridgeHash(lookup.block_hash);
  for (const auto& record : lookup.votes) {
    auto vote = materializeWeightedVote(record);
    if (vote->getPeriod() != period || vote->getRound() != round || vote->getStep() != lookup.step ||
        vote->getBlockHash() != expected_block_hash) {
      throw verifiedVoteViewError("native retained 2t+1 payload mismatches mapped voted block");
    }
    votes.push_back(std::move(vote));
  }
  return votes;
}

rustaxa::PbftNextVotesBundleEgressPlan VoteManager::planNextVotesBundleEgress(PbftPeriod period,
                                                                              PbftRound round) const {
  return pbft_service_->service().pbft_service_verified_votes_plan_next_votes_bundle_egress(period, round);
}

rustaxa::PbftOptimizedVoteBundleBuildResult VoteManager::buildOptimizedVotesBundleEgress(
    rustaxa::PbftOptimizedVoteBundleBuildRequest request) const {
  return pbft_service_->service().pbft_service_verified_votes_build_optimized_votes_bundle_egress(std::move(request));
}

std::string VoteManager::softVoteDebugMessage(PbftPeriod period, PbftRound round) const {
  uint64_t votes_weight = 0;
  std::string debug_msg;
  auto soft_votes = getStepVotes(period, round, 2 /* soft voting step */);
  for (const auto& block_soft_votes : soft_votes.votes) {
    votes_weight += block_soft_votes.second.weight;
    debug_msg += "Block " + block_soft_votes.first.abridged() + "(votes weight " +
                 std::to_string(block_soft_votes.second.weight) + ") -> [";

    for (const auto& vote : block_soft_votes.second.votes) {
      debug_msg += vote.first.abridged() + "(voter " + vote.second->getVoterAddr().abridged() + "), ";
    }

    debug_msg += "]\n";
  }
  debug_msg += "all votes weight " + std::to_string(votes_weight) + ", 2t+1 threshold " +
               std::to_string(getPbftTwoTPlusOne(period - 1, PbftVoteTypes::soft_vote).value());
  return debug_msg;
}

StepVotes VoteManager::getStepVotes(PbftPeriod period, PbftRound round, PbftStep step) const {
  const auto lookup = pbft_service_->service().pbft_service_verified_votes_step_payloads(period, round, step);
  return materializeStepVotes(lookup, period, round, step);
}

bool VoteManager::submitRustPlannedSlashingProof(const SlashingDoubleVoteEvidence& evidence) {
  return slashing_manager_->submitDoubleVotingProof(evidence);
}

void VoteManager::setCurrentPbftPeriodAndRound(PbftPeriod pbft_period, PbftRound pbft_round) {
  current_pbft_period_ = pbft_period;
  current_pbft_round_ = pbft_round;

  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_state_snapshot();
  auto round_votes = materializeRoundVotes(snapshot, pbft_period, pbft_round);
  if (round_votes.step_votes.empty() && round_votes.two_t_plus_one_voted_blocks_.empty() &&
      round_votes.network_t_plus_one_step == 0) {
    return;
  }

  for (const auto& two_t_plus_one_voted_block : round_votes.two_t_plus_one_voted_blocks_) {
    const auto two_t_plus_one_voted_block_type = two_t_plus_one_voted_block.first;
    if (two_t_plus_one_voted_block_type == TwoTPlusOneVotedBlockType::CertVotedBlock) {
      continue;
    }

    const auto& [two_t_plus_one_voted_block_hash, two_t_plus_one_voted_block_step] = two_t_plus_one_voted_block.second;
    const auto found_step_votes_it = round_votes.step_votes.find(two_t_plus_one_voted_block_step);
    if (found_step_votes_it == round_votes.step_votes.end()) {
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

    persistVoteProgressToRustStorage(pbft_service_, nullptr, two_t_plus_one_voted_block_type, votes);
  }
}

PbftStep VoteManager::getNetworkTplusOneNextVotingStep(PbftPeriod period, PbftRound round) const {
  const auto snapshot = pbft_service_->service().pbft_service_verified_votes_state_snapshot();
  return materializeRoundVotes(snapshot, period, round).network_t_plus_one_step;
}

rustaxa::PbftFinalizationStorageWriteStage VoteManager::rewardVotesResetStageForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) {
  return pbft_service_->service().pbft_service_verified_votes_prepare_reward_votes_reset_stage(write_intent);
}

rustaxa::PbftRewardVotesResetRequest VoteManager::rewardVotesResetRequestForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto round = static_cast<PbftRound>(write_intent.reward_vote_round);
  const auto step = static_cast<PbftStep>(write_intent.reward_vote_step);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);
  return makeRewardResetRequest(period, round, step, block_hash, false);
}

RewardVotesFinalizationResetReport VoteManager::commitRewardVotesResetForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, uint64_t reward_votes_reset_generation) {
  const auto result = pbft_service_->service().pbft_service_verified_votes_commit_reward_vote_cursor(
      write_intent, reward_votes_reset_generation);
  if (result.status > 1) {
    throw std::runtime_error("Rust reward-vote cursor commit rejected: " + static_cast<std::string>(result.error_code));
  }
  const auto block_hash = fromBridgeHash(result.block_hash);
  LOG(log_dg_) << "Reward votes info reset to: block_hash: " << block_hash << ", period: " << result.period
               << ", round: " << result.round;
  return makeRewardVotesResetLiveReport(result.period, result.round, block_hash, result.reset_generation);
}

rustaxa::PbftFinalizedPeriodApplyResult VoteManager::resetRewardVotesForFinalization(
    const rustaxa::PbftFinalizationStorageWritePlan& write_intent, Batch& batch) {
  const auto period = static_cast<PbftPeriod>(write_intent.reward_vote_period);
  const auto block_hash = blk_hash_t(write_intent.reward_vote_block_hash.data(), blk_hash_t::ConstructFromPointer);
  (void)batch;

  rustaxa::PbftRewardVotesResetRequest request{};
  try {
    request = rewardVotesResetRequestForFinalization(write_intent);
  } catch (const std::exception& e) {
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash, e.what());
  }

  auto result = pbft_service_->service().pbft_service_verified_votes_apply_reward_votes_reset(std::move(request));
  if (result.status != kPbftFinalizedPeriodApplyStatusApplied &&
      result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
    return result;
  }

  try {
    commitRewardVotesResetForFinalization(write_intent, result.reward_votes_reset_generation);
  } catch (const std::exception& e) {
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash, e.what());
  }
  return result;
}

}  // namespace taraxa
