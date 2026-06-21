#include <libdevcore/RLP.h>

#include <algorithm>
#include <cstdlib>
#include <limits>
#include <mutex>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <utility>

#include "common/constants.hpp"
#include "pbft/pbft_manager.hpp"
#include "pbft/proposed_blocks.hpp"
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
constexpr uint8_t kPbftVoteGenerationStatusZeroWeight = 6;
constexpr uint8_t kPbftVotePersistenceStatusApplied = 0;
constexpr uint8_t kPbftTwoTPlusOneThresholdStatusAvailable = 0;
constexpr uint8_t kPbftTwoTPlusOneThresholdStatusNeedsDposTotal = 1;
constexpr uint8_t kPbftFinalChainFactStatusReady = 0;

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

rust::Slice<const uint8_t> toBridgeByteSlice(const rust::Vec<uint8_t>& bytes) {
  return rust::Slice<const uint8_t>(bytes.data(), bytes.size());
}

rust::Vec<rustaxa::PbftFinalizationHash> toBridgeRewardVoteHashes(const std::vector<vote_hash_t>& hashes) {
  rust::Vec<rustaxa::PbftFinalizationHash> out;
  out.reserve(hashes.size());
  for (const auto& hash : hashes) {
    out.push_back(rustaxa::PbftFinalizationHash{toBridgeHash(hash)});
  }
  return out;
}

rustaxa::PbftFinalChainFacts collectPbftDposFacts(const std::shared_ptr<final_chain::FinalChain>& final_chain,
                                                  PbftPeriod dpos_period, bool collect_total_vote_count,
                                                  const std::vector<addr_t>& addresses) {
  rustaxa::PbftFinalChainFactRequest request{};
  request.period = dpos_period;
  request.collect_total_vote_count = collect_total_vote_count;
  request.collect_address_vote_counts = !addresses.empty();
  request.addresses.reserve(addresses.size());
  for (const auto& address : addresses) {
    request.addresses.push_back(rustaxa::PbftFinalChainFactAddress{toBridgeAddress(address)});
  }
  return final_chain->rustFinalChainForRust().collect_pbft_final_chain_facts(std::move(request));
}

bool finalChainFactReady(uint8_t status) { return status == kPbftFinalChainFactStatusReady; }

std::string finalChainFactError(const rustaxa::PbftFinalChainFacts& facts) {
  if (!facts.error_code.empty()) {
    return static_cast<std::string>(facts.error_code);
  }
  return "PBFT_FINAL_CHAIN_FACTS_UNAVAILABLE";
}

std::string finalChainAddressFactError(const rustaxa::PbftFinalChainAddressFact& fact,
                                       const rustaxa::PbftFinalChainFacts& facts) {
  if (!fact.error_code.empty()) {
    return static_cast<std::string>(fact.error_code);
  }
  return finalChainFactError(facts);
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

rustaxa::PbftFinalizationStorageWriteStage makeRewardResetWriteStage(
    const std::vector<std::shared_ptr<PbftVote>>& votes, const std::vector<vote_hash_t>& extra_reward_votes) {
  auto records = makeVoteStorageRecords(votes, "reward-vote reset");

  rustaxa::PbftFinalizationStorageWriteStage write_stage{};
  write_stage.stage = kPbftFinalizationStorageStageRewardVotesReset;
  write_stage.has_reward_votes_reset = true;
  write_stage.reward_votes_bundle_rlp = rustaxa::pbft_vote_bundle_payload_from_records(std::move(records));
  write_stage.extra_reward_vote_hashes = toBridgeRewardVoteHashes(extra_reward_votes);
  return write_stage;
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

rustaxa::PbftVoteStorageRecord cloneVoteStorageRecord(const rustaxa::PbftVoteStorageRecord& record) {
  rustaxa::PbftVoteStorageRecord out;
  out.hash = record.hash;
  out.vote_rlp.reserve(record.vote_rlp.size());
  for (const auto byte : record.vote_rlp) {
    out.vote_rlp.push_back(byte);
  }
  return out;
}

rustaxa::PbftTwoTPlusOneVoteBundle cloneTwoTPlusOneVoteBundle(const rustaxa::PbftTwoTPlusOneVoteBundle& bundle) {
  rustaxa::PbftTwoTPlusOneVoteBundle out;
  out.kind = bundle.kind;
  out.period = bundle.period;
  out.round = bundle.round;
  out.step = bundle.step;
  out.block_hash = bundle.block_hash;
  out.votes_bundle_rlp.reserve(bundle.votes_bundle_rlp.size());
  for (const auto byte : bundle.votes_bundle_rlp) {
    out.votes_bundle_rlp.push_back(byte);
  }
  return out;
}

void requireApplied(const rustaxa::PbftVotePersistenceResult& result, const char* operation) {
  if (result.status == kPbftVotePersistenceStatusApplied) {
    return;
  }

  std::stringstream err;
  err << "Rust PBFT vote persistence rejected " << operation << ": " << static_cast<std::string>(result.error_code);
  throw std::runtime_error(err.str());
}

void persistVoteProgressToRustStorage(const VerifiedVotes& verified_votes,
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

  requireApplied(verified_votes.persistPbftVoteProgress(std::move(write)), "vote progress");
}

void persistVoteProgressPayloadsToRustStorage(const VerifiedVotes& verified_votes, bool has_extra_reward_vote,
                                              const rustaxa::PbftVoteStorageRecord& extra_reward_vote,
                                              bool has_two_t_plus_one_bundle,
                                              const rustaxa::PbftTwoTPlusOneVoteBundle& two_t_plus_one_bundle) {
  rustaxa::PbftVoteProgressPersistenceWrite write{};
  if (has_extra_reward_vote) {
    write.has_extra_reward_vote = true;
    write.extra_reward_vote = cloneVoteStorageRecord(extra_reward_vote);
  }

  if (has_two_t_plus_one_bundle) {
    write.has_two_t_plus_one_bundle = true;
    write.two_t_plus_one_bundle = cloneTwoTPlusOneVoteBundle(two_t_plus_one_bundle);
  }

  requireApplied(verified_votes.persistPbftVoteProgress(std::move(write)), "vote progress");
}

rustaxa::PbftVoteEventFactFlags makeVoteEventFactFlags(bool valid_stale_reward_vote) {
  rustaxa::PbftVoteEventFactFlags flags{};
  flags.vote_already_known = false;
  flags.carries_proposed_block = true;
  flags.valid_stale_reward_vote = valid_stale_reward_vote;
  return flags;
}

void requireRuntimeAdmissionVoteMatches(const rustaxa::VerifiedVotePayload& fact,
                                        const std::shared_ptr<PbftVote>& vote) {
  if (!vote || !vote->getWeight().has_value()) {
    throw std::runtime_error("VoteManager cannot attach PBFT vote admission result without a weighted vote sidecar");
  }

  if (fact.vote_hash != toBridgeHash(vote->getHash()) || fact.block_hash != toBridgeHash(vote->getBlockHash()) ||
      fact.voter != toBridgeAddress(vote->getVoterAddr()) || fact.period != vote->getPeriod() ||
      fact.round != vote->getRound() || fact.step != vote->getStep() ||
      fact.vote_type != static_cast<uint8_t>(vote->getType()) || fact.weight != *vote->getWeight()) {
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

rustaxa::PbftVoteValidationExternalFacts makeVoteValidationExternalFacts(bool strict, const PbftConfig& config) {
  rustaxa::PbftVoteValidationExternalFacts facts{};
  facts.strict_vrf = strict;
  facts.committee_size = config.committee_size;
  facts.number_of_proposers = config.number_of_proposers;
  return facts;
}

rustaxa::PbftProposerSortitionFact makeProposerSortitionFact(const PbftConfig& config) {
  rustaxa::PbftProposerSortitionFact fact{};
  fact.number_of_proposers = config.number_of_proposers;
  return fact;
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

rustaxa::PbftVoteWeightFacts makeVoteWeightFacts(uint64_t voter_dpos_votes_count, uint64_t total_dpos_votes_count,
                                                 uint64_t committee_size, uint64_t number_of_proposers) {
  rustaxa::PbftVoteWeightFacts facts{};
  facts.voter_dpos_vote_count = voter_dpos_votes_count;
  facts.total_dpos_vote_count = total_dpos_votes_count;
  facts.committee_size = committee_size;
  facts.number_of_proposers = number_of_proposers;
  return facts;
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

}  // namespace

VoteManager::VoteManager(const FullNodeConfig& config, std::shared_ptr<DbStorage> db,
                         std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<final_chain::FinalChain> final_chain,
                         std::shared_ptr<KeyManager> key_manager, std::shared_ptr<SlashingManager> slashing_manager)
    : VoteManagerOld(config, std::move(db), std::move(pbft_chain), std::move(final_chain), std::move(key_manager),
                     std::move(slashing_manager)) {
  verified_votes_.attachRustStorage(db_->rustStorage());
}

void VoteManager::setNetwork(std::weak_ptr<Network> network) {
  // TODO(rustaxa): move VoteManager network wiring to Rust/shim-owned state.
  VoteManagerOld::setNetwork(std::move(network));
}

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
  bool is_valid_potential_reward_vote = false;
  if (vote->getPeriod() < current_pbft_period_) {
    is_valid_potential_reward_vote = isValidRewardVoteForRust(vote);
    if (!is_valid_potential_reward_vote) {
      LOG(log_tr_) << "Old vote " << vote->getHash().abridged() << " vote period" << vote->getPeriod()
                   << " current period " << current_pbft_period_;
      return report;
    }
  }

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
      verified_votes_.replayInsert(fromBridgeVoteHash(inspection.vote_hash));
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

  auto external_facts = makeVoteValidationExternalFacts(true, kPbftConfig);
  const auto recovered_voter = fromBridgeAddress(inspection.recovered_voter);
  try {
    const auto dpos_facts = collectPbftDposFacts(final_chain_, vote->getPeriod() - 1, true, {recovered_voter});
    if (dpos_facts.address_facts.empty() || !finalChainFactReady(dpos_facts.address_facts[0].status) ||
        !finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      external_facts.future_dpos_state = true;
      const auto error = dpos_facts.address_facts.empty()
                             ? finalChainFactError(dpos_facts)
                             : finalChainAddressFactError(dpos_facts.address_facts[0], dpos_facts);
      LOG(log_er_) << "Unable to admit vote " << hash << " against dpos contract. Its period (" << vote->getPeriod()
                   << ") is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                   << "). Err msg: " << error;
      return report;
    }
    external_facts.voter_dpos_vote_count = dpos_facts.address_facts[0].vote_count;
    external_facts.voter_dpos_ready = true;

    const auto pk = key_manager_->getVrfKey(vote->getPeriod() - 1, recovered_voter);
    external_facts.vrf_key_ready = true;
    external_facts.has_vrf_key = pk != nullptr;
    if (pk != nullptr) {
      external_facts.vrf_public_key = pk->asArray();
    }

    external_facts.total_dpos_vote_count = dpos_facts.total_vote_count;
    external_facts.total_dpos_ready = true;
  } catch (const std::exception& e) {
    external_facts.unknown_error = true;
    LOG(log_er_) << "Unable to admit vote " << hash << ". Err msg: " << e.what();
    return report;
  } catch (...) {
    external_facts.unknown_error = true;
    LOG(log_er_) << "Unable to admit vote " << hash << ". Unknown error";
    return report;
  }

  const auto runtime_result =
      verified_votes_.admitValidatedVote(toBridgeByteSlice(canonical_vote_rlp), external_facts,
                                         makeVoteEventFactFlags(is_valid_potential_reward_vote), progress_context);
  requireRuntimeVoteTransitionIntentsMatch(runtime_result, vote);
  report.mark_vote_known = runtime_result.mark_vote_known;
  report.mark_vote_known_hash = fromBridgeVoteHash(runtime_result.mark_vote_known_hash);
  report.gossip_vote = runtime_result.gossip_vote;
  report.gossip_vote_hash = fromBridgeVoteHash(runtime_result.gossip_vote_hash);
  report.report_slashing = runtime_result.report_slashing;
  report.drive_pbft_progress = runtime_result.drive_pbft_progress;
  report.progress_period = runtime_result.progress_period;
  report.progress_round = runtime_result.progress_round;

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
  // TODO(rustaxa): remove this legacy sidecar hydration once VoteManager no longer keeps live C++ PbftVote sidecars.
  const auto cpp_weight =
      vote->calculateWeight(external_facts.voter_dpos_vote_count, external_facts.total_dpos_vote_count,
                            runtime_result.validation.sortition_threshold);
  if (cpp_weight != runtime_result.validation.calculated_weight) {
    throw std::runtime_error("VoteManager Rust PBFT vote admission weight mismatched legacy sidecar hydration");
  }
  requireRuntimeAdmissionVoteMatches(runtime_result.vote, vote);

  if (runtime_result.report_slashing) {
    LOG(log_wr_) << "Non unique vote " << vote->getHash().abridged() << " (race condition)";
    submitRustPlannedSlashingProof(runtime_result.slashing_incoming_vote, runtime_result.slashing_conflicting_vote,
                                   vote->getPeriod(), vote->getRound(), vote->getStep());
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
  if (!runtime_result.has_storage_vote) {
    throw std::runtime_error("VoteManager Rust PBFT vote admission accepted without a storage payload");
  }

  const auto votes_with_weight = verified_votes_.attachRuntimeAcceptedVote(vote, runtime_result);
  if (!votes_with_weight) {
    throw std::runtime_error("VoteManager Rust vote-progress planner accepted vote without inserted vote sidecars");
  }

  LOG(log_nf_) << "Added verified vote: " << hash;
  LOG(log_dg_) << "Added verified vote: " << *vote;

  if (!two_t_plus_one.has_value()) [[unlikely]] {
    if (runtime_result.persist_extra_reward_vote) {
      persistVoteProgressPayloadsToRustStorage(verified_votes_, true, runtime_result.extra_reward_vote, false,
                                               runtime_result.two_t_plus_one_bundle);
      extra_reward_votes_.emplace_back(vote->getHash());
    }
    LOG(log_er_) << "Cannot set(or not) 2t+1 voted block as 2t+1 threshold is unavailable, vote " << vote->getHash();
    report.accepted = true;
    return report;
  }

  if (runtime_result.network_t_plus_one_step_updated) {
    LOG(log_nf_) << "Set t+1 next voted block " << vote->getHash() << " for period " << vote->getPeriod() << ", round "
                 << vote->getRound() << ", step " << vote->getStep();
  }

  if (!runtime_result.persist_two_t_plus_one_votes) {
    if (runtime_result.persist_extra_reward_vote) {
      persistVoteProgressPayloadsToRustStorage(verified_votes_, true, runtime_result.extra_reward_vote, false,
                                               runtime_result.two_t_plus_one_bundle);
      extra_reward_votes_.emplace_back(vote->getHash());
    }
    report.accepted = true;
    return report;
  }

  persistVoteProgressPayloadsToRustStorage(verified_votes_, runtime_result.persist_extra_reward_vote,
                                           runtime_result.extra_reward_vote, true,
                                           runtime_result.two_t_plus_one_bundle);
  if (runtime_result.persist_extra_reward_vote) {
    extra_reward_votes_.emplace_back(vote->getHash());
  }
  report.accepted = true;
  return report;
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

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> VoteManager::identifyLeaderBlock(
    ProposedBlocks& propose_blocks, PbftPeriod period, PbftRound round,
    const std::function<bool(const blk_hash_t&)>& block_in_chain,
    const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const {
  return identifyLeaderBlock(propose_blocks, getProposalVotes(period, round), block_in_chain, validate_block);
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> VoteManager::identifyLeaderBlock(
    ProposedBlocks& propose_blocks, std::vector<std::shared_ptr<PbftVote>>&& propose_votes,
    const std::function<bool(const blk_hash_t&)>& block_in_chain,
    const std::function<bool(const std::shared_ptr<PbftBlock>&)>& validate_block) const {
  if (propose_votes.empty()) {
    return {};
  }

  rust::Vec<rustaxa::PbftManagerLeaderCandidateInputFact> candidate_facts;
  candidate_facts.reserve(propose_votes.size());
  std::vector<std::pair<std::shared_ptr<PbftBlock>, std::shared_ptr<PbftVote>>> materialized_candidates;

  for (auto&& vote : propose_votes) {
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

    const auto block_metadata = propose_blocks.getPbftProposedBlockMetadata(vote->getPeriod(), proposed_block_hash);
    if (!block_metadata.has_value()) {
      LOG(log_er_) << "Unable to get proposed block " << proposed_block_hash;
      candidate_facts.push_back(fact);
      continue;
    }
    fact.proposed_block_found = true;
    fact.pivot_hash = toBridgeHash(block_metadata->pivot_hash);

    const auto proposed_block = propose_blocks.getPbftProposedBlock(vote->getPeriod(), proposed_block_hash);
    if (!proposed_block.has_value()) {
      LOG(log_er_) << "Unable to materialize proposed block " << proposed_block_hash;
      fact.proposed_block_found = false;
      candidate_facts.push_back(fact);
      continue;
    }

    if (block_metadata->is_valid || proposed_block->second) {
      fact.block_validation_status = kPbftManagerLeaderBlockAlreadyValid;
    } else if (validate_block(proposed_block->first)) {
      fact.block_validation_status = kPbftManagerLeaderBlockValidated;
    } else {
      fact.block_validation_status = kPbftManagerLeaderBlockRejected;
    }

    if (fact.block_validation_status != kPbftManagerLeaderBlockRejected) {
      materialized_candidates.emplace_back(proposed_block->first, std::move(vote));
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

  for (const auto& valid_block : plan.valid_blocks) {
    propose_blocks.markBlockAsValid(valid_block.period, fromBridgeHash(valid_block.block_hash));
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
  const auto decision = verified_votes_.determineRoundAdvance(current_pbft_period, current_pbft_round);
  if (!decision) {
    return result;
  }

  LOG(log_nf_) << "New round " << decision->new_round << " determined for period " << current_pbft_period
               << ". Found 2t+1 votes for block " << decision->voted_block.hash << " in round "
               << decision->supporting_round << ", step " << decision->voted_block.step;

  result.has_new_round = true;
  result.new_round = decision->new_round;
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
  blk_hash_t reward_votes_block_hash;
  PbftPeriod reward_votes_period;
  PbftRound reward_votes_round;
  {
    std::shared_lock reward_votes_info_lock(reward_votes_info_mutex_);
    reward_votes_block_hash = reward_votes_block_hash_;
    reward_votes_period = reward_votes_period_;
    reward_votes_round = reward_votes_round_;
  }

  VerifiedVotes::RewardVotePayloadSelection selection{};
  try {
    selection = verified_votes_.selectRewardVotePayloads(block_period, reward_votes_period, reward_votes_round,
                                                         reward_votes_block_hash, reward_vote_hashes, copy_votes);
  } catch (const std::exception& e) {
    LOG(log_er_) << "Rust reward-vote payload selection failed for block " << block_hash
                 << ", period: " << block_period << ", reward_votes_period: " << reward_votes_period
                 << ", reward_votes_round_: " << reward_votes_round
                 << ", reward_votes_block_hash: " << reward_votes_block_hash << ", error: " << e.what();
    assert(false);
    RewardVoteValidationResult result;
    result.error_code = e.what();
    return result;
  }

  const auto& plan = selection.report;
  RewardVoteValidationResult result;
  result.accepted = plan.accepted;
  result.status = plan.status;
  result.error_code = static_cast<std::string>(plan.error_code);
  result.selected_period = plan.selected_period;
  result.selected_round = plan.selected_round;
  result.selected_block_hash = fromBridgeHash(plan.selected_block_hash);
  result.missing_vote_hash = fromBridgeHash(plan.missing_vote_hash);
  result.votes = std::move(selection.votes);

  if (!plan.accepted) {
    LOG(log_er_) << "No (or not enough) reward votes found for block " << block_hash << ", period: " << block_period
                 << ", prev. block hash: " << prev_block_hash
                 << ", reward_votes_period: " << reward_votes_period << ", reward_votes_round_: " << reward_votes_round
                 << ", selected_round: " << plan.selected_round
                 << ", reward_votes_block_hash: " << reward_votes_block_hash
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
  blk_hash_t reward_votes_block_hash;
  PbftRound reward_votes_period;
  PbftRound reward_votes_round;
  {
    std::shared_lock reward_votes_info_lock(reward_votes_info_mutex_);
    reward_votes_block_hash = reward_votes_block_hash_;
    reward_votes_period = reward_votes_period_;
    reward_votes_round = reward_votes_round_;
  }

  auto reward_votes =
      getTwoTPlusOneVotedBlockVotes(reward_votes_period, reward_votes_round, TwoTPlusOneVotedBlockType::CertVotedBlock);

  if (!reward_votes.empty() && reward_votes[0]->getBlockHash() != reward_votes_block_hash) {
    LOG(log_er_) << "Proposal reward votes block hash mismatch. reward_votes_block_hash " << reward_votes_block_hash
                 << ", reward_votes[0]->getBlockHash() " << reward_votes[0]->getBlockHash();
    assert(false);
    return {};
  }

  return reward_votes;
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
  std::shared_lock lock(reward_votes_info_mutex_);
  return reward_votes_period_;
}

void VoteManager::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>& vote) {
  if (!vote) {
    throw std::runtime_error("VoteManager cannot persist a null own verified vote");
  }
  auto record = makeVoteStorageRecord(vote);
  requireApplied(verified_votes_.saveOwnVerifiedVote(std::move(record)), "own verified vote");
  own_verified_votes_.push_back(vote);
}

std::vector<std::shared_ptr<PbftVote>> VoteManager::getOwnVerifiedVotes() { return own_verified_votes_; }

void VoteManager::clearOwnVerifiedVotes(Batch& write_batch) {
  (void)write_batch;
  std::vector<vote_hash_t> own_vote_hashes;
  own_vote_hashes.reserve(own_verified_votes_.size());
  for (const auto& vote : own_verified_votes_) {
    if (!vote) {
      throw std::runtime_error("VoteManager cannot clear a null own verified vote");
    }
    own_vote_hashes.emplace_back(vote->getHash());
  }

  requireApplied(verified_votes_.clearOwnVerifiedVotes(toBridgeRewardVoteHashes(own_vote_hashes)),
                 "own verified vote cleanup");
  own_verified_votes_.clear();
}

void VoteManager::clearOwnVerifiedVotesAfterRustPersistence() { own_verified_votes_.clear(); }

std::shared_ptr<PbftVote> VoteManager::generateVoteWithWeight(const blk_hash_t& blockhash, PbftVoteTypes vote_type,
                                                              PbftPeriod period, PbftRound round, PbftStep step,
                                                              const WalletConfig& wallet) {
  const auto generation_input = makeVoteGenerationInput(blockhash, vote_type, period, round, step, wallet);
  uint64_t voter_dpos_votes_count = 0;
  uint64_t total_dpos_votes_count = 0;

  try {
    const auto dpos_facts = collectPbftDposFacts(final_chain_, period - 1, true, {wallet.node_addr});
    if (dpos_facts.address_facts.empty() || !finalChainFactReady(dpos_facts.address_facts[0].status) ||
        !finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      LOG(log_er_) << "Unable to place vote for period: " << period << ", round: " << round << ", step: " << step
                   << ", voted block hash: " << blockhash.abridged() << ". "
                   << "Period is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                   << "). Err msg: " << finalChainFactError(dpos_facts);
      return nullptr;
    }

    voter_dpos_votes_count = dpos_facts.address_facts[0].vote_count;
    if (!voter_dpos_votes_count) {
      const auto generated = rustaxa::pbft_generate_signed_vote_with_weight(
          generation_input,
          makeVoteWeightFacts(voter_dpos_votes_count, 0, kPbftConfig.committee_size, kPbftConfig.number_of_proposers));
      requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroStake, "zero-stake weighted vote");
      return nullptr;
    }

    total_dpos_votes_count = dpos_facts.total_vote_count;
  } catch (const std::exception& e) {
    LOG(log_er_) << "Unable to place vote for period: " << period << ", round: " << round << ", step: " << step
                 << ", voted block hash: " << blockhash.abridged() << ". Err msg: " << e.what();
    return nullptr;
  }

  if (!total_dpos_votes_count) {
    const auto generated = rustaxa::pbft_generate_signed_vote_with_weight(
        generation_input, makeVoteWeightFacts(voter_dpos_votes_count, total_dpos_votes_count,
                                              kPbftConfig.committee_size, kPbftConfig.number_of_proposers));
    requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroTotalDpos, "zero-total weighted vote");
    return nullptr;
  }

  const auto generated = rustaxa::pbft_generate_signed_vote_with_weight(
      generation_input, makeVoteWeightFacts(voter_dpos_votes_count, total_dpos_votes_count, kPbftConfig.committee_size,
                                            kPbftConfig.number_of_proposers));

  if (generated.status == kPbftVoteGenerationStatusZeroWeight) {
    requireRustVoteGenerationRejected(generated, kPbftVoteGenerationStatusZeroWeight, "zero-weight weighted vote");
    return nullptr;
  }

  return materializeRustGeneratedVote(generated, wallet, true);
}

VoteManager::LocallyGeneratedVotePlacement VoteManager::generateAndPlaceLocalVote(
    const blk_hash_t& block_hash, PbftVoteTypes vote_type, PbftPeriod period, PbftRound round, PbftStep step,
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

VoteManager::LocalProposalVoteGeneration VoteManager::generateUniqueProposalVoteForBlock(
    const blk_hash_t& block_hash, PbftPeriod period, PbftRound round, PbftStep step, const WalletConfig& wallet) {
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
    err << "Non unique propose vote " << result.vote->getHash() << " for block " << block_hash << ", period "
        << period << ", round " << result.vote->getRound() << ", step " << result.vote->getStep() << ", validator "
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
    verified_votes_.replayInsert(fromBridgeVoteHash(inspection.vote_hash));
    err_msg << "Invalid vote " << vote->getHash() << ": invalid signature";
    return {false, err_msg.str()};
  }

  if (inspection.status != kPbftCanonicalVoteInspectionStatusValid) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown Rust canonical PBFT vote inspection status "
            << static_cast<uint32_t>(inspection.status);
    return {false, err_msg.str()};
  }

  const uint64_t vote_period = inspection.period;
  const auto recovered_voter = fromBridgeAddress(inspection.recovered_voter);
  auto external_facts = makeVoteValidationExternalFacts(strict, kPbftConfig);
  rustaxa::PbftCanonicalVoteValidation validation{};

  uint64_t voter_dpos_votes_count = 0;
  uint64_t total_dpos_votes_count = 0;
  rustaxa::PbftFinalChainFacts dpos_facts{};
  try {
    dpos_facts = collectPbftDposFacts(final_chain_, vote_period - 1, true, {recovered_voter});
    if (dpos_facts.address_facts.empty() || !finalChainFactReady(dpos_facts.address_facts[0].status)) {
      external_facts.future_dpos_state = true;
      (void)verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts);
      err_msg << "Unable to validate vote " << vote->getHash() << " against dpos contract. It's period (" << vote_period
              << ") is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
              << "). Err msg: "
              << (dpos_facts.address_facts.empty()
                      ? finalChainFactError(dpos_facts)
                      : finalChainAddressFactError(dpos_facts.address_facts[0], dpos_facts));
      return {false, err_msg.str()};
    }
    voter_dpos_votes_count = dpos_facts.address_facts[0].vote_count;
    external_facts.voter_dpos_ready = true;
    external_facts.voter_dpos_vote_count = voter_dpos_votes_count;
  } catch (...) {
    external_facts.unknown_error = true;
    (void)verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts);
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation";
    return {false, err_msg.str()};
  }

  validation = verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts).validation;
  if (validation.status == kPbftVoteValidationStatusZeroStake) {
    err_msg << "Invalid vote " << vote->getHash() << ": author " << recovered_voter << " has zero stake";
    return {false, err_msg.str()};
  }
  if (validation.status == kPbftVoteValidationStatusInvalidVoteType) {
    err_msg << "Invalid vote " << vote->getHash() << ": invalid PBFT vote type";
    return {false, err_msg.str()};
  }

  try {
    const auto pk = key_manager_->getVrfKey(vote_period - 1, recovered_voter);
    external_facts.vrf_key_ready = true;
    external_facts.has_vrf_key = pk != nullptr;
    if (pk != nullptr) {
      external_facts.vrf_public_key = pk->asArray();
    }

    validation =
        verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts).validation;
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

    if (!finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      external_facts.future_dpos_state = true;
      (void)verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts);
      err_msg << "Unable to validate vote " << vote->getHash() << " against dpos contract. It's period (" << vote_period
              << ") is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
              << "). Err msg: " << finalChainFactError(dpos_facts);
      return {false, err_msg.str()};
    }
    total_dpos_votes_count = dpos_facts.total_vote_count;
    external_facts.total_dpos_ready = true;
    external_facts.total_dpos_vote_count = total_dpos_votes_count;
    validation =
        verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts).validation;
    if (!validation.has_sortition_threshold) {
      throw std::runtime_error("Rust PBFT vote validation did not return a sortition threshold");
    }
    if (validation.status == kPbftVoteValidationStatusZeroWeight) {
      err_msg << "Invalid vote " << vote->getHash() << ": zero weight";
      return {false, err_msg.str()};
    }
    if (!validation.weight_calculated) {
      throw std::runtime_error("Rust PBFT vote validation accepted validation facts without a calculated weight");
    }

    // TODO(rustaxa): remove this legacy sidecar mutation once Rust owns the live PBFT vote object or the shim has a
    // Rust-owned verified-vote payload path that no longer requires `PbftVote::weight_`.
    const auto cpp_weight =
        vote->calculateWeight(voter_dpos_votes_count, total_dpos_votes_count, validation.sortition_threshold);
    if (cpp_weight != validation.calculated_weight) {
      throw std::runtime_error("Rust PBFT vote weight does not match legacy C++ sidecar weight");
    }
  } catch (state_api::ErrFutureBlock& e) {
    external_facts.future_dpos_state = true;
    (void)verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts);
    err_msg << "Unable to validate vote " << vote->getHash() << " against dpos contract. It's period (" << vote_period
            << ") is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
            << "). Err msg: " << e.what();
    return {false, err_msg.str()};
  } catch (...) {
    external_facts.unknown_error = true;
    (void)verified_votes_.validateCanonicalVote(toBridgeByteSlice(canonical_vote_rlp), external_facts);
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation";
    return {false, err_msg.str()};
  }

  if (validation.status != kPbftVoteValidationStatusValid || !validation.accepted) {
    err_msg << "Invalid vote " << vote->getHash() << ": unknown error during validation";
    return {false, err_msg.str()};
  }

  return {true, ""};
}

std::optional<uint64_t> VoteManager::getPbftTwoTPlusOne(PbftPeriod pbft_period, PbftVoteTypes vote_type) const {
  rustaxa::PbftTwoTPlusOneThresholdFact threshold_fact{};
  threshold_fact.pbft_period = pbft_period;
  threshold_fact.vote_type = static_cast<uint8_t>(vote_type);
  threshold_fact.current_pbft_chain_size = pbft_chain_->getPbftChainSize();
  threshold_fact.committee_size = kPbftConfig.committee_size;
  threshold_fact.number_of_proposers = kPbftConfig.number_of_proposers;

  auto threshold_plan = verified_votes_.twoTPlusOneThreshold(threshold_fact);
  if (threshold_plan.status == kPbftTwoTPlusOneThresholdStatusAvailable && threshold_plan.has_threshold) {
    return threshold_plan.threshold;
  }
  if (threshold_plan.status != kPbftTwoTPlusOneThresholdStatusNeedsDposTotal ||
      !threshold_plan.needs_total_dpos_votes) {
    LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period << ". Rust threshold status "
                 << static_cast<uint32_t>(threshold_plan.status) << " error "
                 << static_cast<std::string>(threshold_plan.error_code);
    return {};
  }

  uint64_t total_dpos_votes_count = 0;
  try {
    const auto dpos_facts = collectPbftDposFacts(final_chain_, pbft_period, true, {});
    if (!finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      threshold_fact.future_dpos_state = true;
      (void)verified_votes_.twoTPlusOneThreshold(threshold_fact);
      LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period
                   << ". Period is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                   << "). Err msg: " << finalChainFactError(dpos_facts);
      return {};
    }
    total_dpos_votes_count = dpos_facts.total_vote_count;
  } catch (const std::exception& e) {
    threshold_fact.unknown_error = true;
    threshold_plan = verified_votes_.twoTPlusOneThreshold(threshold_fact);
    LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period << ". Err msg: " << e.what()
                 << ". Rust threshold status " << static_cast<uint32_t>(threshold_plan.status) << " error "
                 << static_cast<std::string>(threshold_plan.error_code);
    return {};
  } catch (...) {
    threshold_fact.unknown_error = true;
    threshold_plan = verified_votes_.twoTPlusOneThreshold(threshold_fact);
    LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period
                 << ". Unknown error. Rust threshold status " << static_cast<uint32_t>(threshold_plan.status)
                 << " error " << static_cast<std::string>(threshold_plan.error_code);
    return {};
  }

  threshold_fact.current_pbft_chain_size = pbft_chain_->getPbftChainSize();
  threshold_fact.has_total_dpos_votes_count = true;
  threshold_fact.total_dpos_votes_count = total_dpos_votes_count;
  threshold_plan = verified_votes_.twoTPlusOneThreshold(threshold_fact);
  if (threshold_plan.status == kPbftTwoTPlusOneThresholdStatusAvailable && threshold_plan.has_threshold) {
    return threshold_plan.threshold;
  }

  LOG(log_er_) << "Unable to calculate 2t + 1 for period: " << pbft_period << ". Rust threshold status "
               << static_cast<uint32_t>(threshold_plan.status) << " error "
               << static_cast<std::string>(threshold_plan.error_code);
  return {};
}

bool VoteManager::voteAlreadyValidated(const vote_hash_t& vote_hash) const {
  return verified_votes_.replayContains(vote_hash);
}

bool VoteManager::genAndValidateVrfSortition(PbftPeriod pbft_period, PbftRound pbft_round,
                                             const WalletConfig& wallet) const {
  VrfPbftSortition vrf_sortition(wallet.vrf_secret, {PbftVoteTypes::propose_vote, pbft_period, pbft_round, 1});
  auto sortition_fact = makeProposerSortitionFact(kPbftConfig);
  rustaxa::PbftFinalChainFacts dpos_facts{};

  try {
    dpos_facts = collectPbftDposFacts(final_chain_, pbft_period - 1, true, {wallet.node_addr});
    if (dpos_facts.address_facts.empty() || !finalChainFactReady(dpos_facts.address_facts[0].status)) {
      sortition_fact.future_dpos_state = true;
      (void)rustaxa::pbft_proposer_sortition_plan(sortition_fact);
      LOG(log_er_) << "Unable to generate vrf sortition for period " << pbft_period << ", round " << pbft_round
                   << ". Period is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                   << "). Err msg: "
                   << (dpos_facts.address_facts.empty()
                           ? finalChainFactError(dpos_facts)
                           : finalChainAddressFactError(dpos_facts.address_facts[0], dpos_facts));
      return false;
    }
    const uint64_t voter_dpos_votes_count = dpos_facts.address_facts[0].vote_count;
    sortition_fact.dpos_vote_count_ready = true;
    sortition_fact.dpos_vote_count = voter_dpos_votes_count;
    auto sortition_plan = rustaxa::pbft_proposer_sortition_plan(sortition_fact);
    if (sortition_plan.rejected) {
      LOG(log_er_) << "Generated vrf sortition for period " << pbft_period << ", round " << pbft_round
                   << " is invalid. Voter dpos vote count is zero";
      return false;
    }

    if (!finalChainFactReady(dpos_facts.total_vote_count_status) || !dpos_facts.has_total_vote_count) {
      sortition_fact.future_dpos_state = true;
      (void)rustaxa::pbft_proposer_sortition_plan(sortition_fact);
      LOG(log_er_) << "Unable to generate vrf sortition for period " << pbft_period << ", round " << pbft_round
                   << ". Period is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                   << "). Err msg: " << finalChainFactError(dpos_facts);
      return false;
    }
    const uint64_t total_dpos_votes_count = dpos_facts.total_vote_count;
    sortition_fact.total_dpos_vote_count_ready = true;
    sortition_fact.total_dpos_vote_count = total_dpos_votes_count;
    sortition_plan = rustaxa::pbft_proposer_sortition_plan(sortition_fact);
    if (!sortition_plan.has_sortition_threshold) {
      throw std::runtime_error("Rust PBFT proposer sortition did not return a sortition threshold");
    }

    sortition_fact.weight_ready = true;
    sortition_fact.weight = vrf_sortition.calculateWeight(voter_dpos_votes_count, total_dpos_votes_count,
                                                          sortition_plan.sortition_threshold, wallet.node_pk);
    sortition_plan = rustaxa::pbft_proposer_sortition_plan(sortition_fact);
    if (!sortition_plan.accepted) {
      LOG(log_dg_) << "Generated vrf sortition for period " << pbft_period << ", round " << pbft_round
                   << " is invalid. Vrf sortition is zero";
      return false;
    }
  } catch (state_api::ErrFutureBlock& e) {
    sortition_fact.future_dpos_state = true;
    (void)rustaxa::pbft_proposer_sortition_plan(sortition_fact);
    LOG(log_er_) << "Unable to generate vrf sortition for period " << pbft_period << ", round " << pbft_round
                 << ". Period is too far ahead of actual finalized pbft chain size (" << dpos_facts.last_block_number
                 << "). Err msg: " << e.what();
    return false;
  } catch (...) {
    sortition_fact.unknown_error = true;
    (void)rustaxa::pbft_proposer_sortition_plan(sortition_fact);
    return false;
  }

  return true;
}

std::optional<blk_hash_t> VoteManager::getTwoTPlusOneVotedBlock(PbftPeriod period, PbftRound round,
                                                                TwoTPlusOneVotedBlockType type) const {
  const auto voted_block = verified_votes_.getTwoTPlusOneVotedBlock(period, round, type);
  if (!voted_block) {
    return {};
  }
  return voted_block->hash;
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

VoteManager::PreviousRoundNextVoteLogFacts VoteManager::previousRoundNextVoteLogFacts(
    PbftPeriod period, PbftRound previous_round) const {
  PreviousRoundNextVoteLogFacts facts;
  facts.next_voted_block =
      getTwoTPlusOneVotedBlock(period, previous_round, TwoTPlusOneVotedBlockType::NextVotedBlock);
  facts.next_voted_null_block =
      getTwoTPlusOneVotedBlock(period, previous_round, TwoTPlusOneVotedBlockType::NextVotedNullBlock).has_value();
  return facts;
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
                                                                                          PbftRound round) const {
  StuckRoundVoteBroadcastPayloads payloads;
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
  return verified_votes_.getTwoTPlusOneVotedBlockVotes(period, round, type);
}

rustaxa::PbftNextVotesBundleEgressPlan VoteManager::planNextVotesBundleEgress(PbftPeriod period,
                                                                              PbftRound round) const {
  return verified_votes_.planNextVotesBundleEgress(period, round);
}

rustaxa::PbftOptimizedVoteBundleBuildResult VoteManager::buildOptimizedVotesBundleEgress(
    rustaxa::PbftOptimizedVoteBundleBuildRequest request) const {
  return verified_votes_.buildOptimizedVotesBundleEgress(std::move(request));
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
  return verified_votes_.getStepVotes(period, round, step).value_or(StepVotes{});
}

bool VoteManager::submitRustPlannedSlashingProof(const rustaxa::PbftVoteStorageRecord& incoming_vote,
                                                 const rustaxa::PbftVoteStorageRecord& conflicting_vote,
                                                 PbftPeriod period, PbftRound round, PbftStep step) {
  return slashing_manager_->submitDoubleVotingProof(incoming_vote, conflicting_vote, period, round, step);
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

    persistVoteProgressToRustStorage(verified_votes_, nullptr, two_t_plus_one_voted_block_type, votes);
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

  const auto found_two_t_plus_one_voted_block =
      verified_votes_.getTwoTPlusOneVotedBlock(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (!found_two_t_plus_one_voted_block.has_value()) {
    LOG(log_er_) << "resetRewardVotes missing cert voted block for period " << period << ", round " << round;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_CERT_BLOCK");
  }
  if (found_two_t_plus_one_voted_block->hash != block_hash) {
    LOG(log_er_) << "resetRewardVotes incorrect block " << found_two_t_plus_one_voted_block->hash << " expected "
                 << block_hash;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_REWARD_VOTES_CERT_BLOCK_MISMATCH");
  }

  if (found_two_t_plus_one_voted_block->step != step) {
    LOG(log_er_) << "resetRewardVotes incorrect cert-vote step " << found_two_t_plus_one_voted_block->step
                 << " expected " << step;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_REWARD_VOTES_CERT_BLOCK_STEP_MISMATCH");
  }

  auto votes = verified_votes_.getTwoTPlusOneVotedBlockVotes(period, round, TwoTPlusOneVotedBlockType::CertVotedBlock);
  if (votes.empty()) {
    LOG(log_er_) << "resetRewardVotes missing cert voted block payloads for period " << period << ", round " << round
                 << ", step " << step;
    assert(false);
    throw std::runtime_error("PBFT_FINALIZE_MISSING_REWARD_VOTES_CERT_BLOCK_PAYLOADS");
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
  (void)batch;

  rustaxa::PbftFinalizationStorageWriteStage stage{};
  try {
    stage = rewardVotesResetStageForFinalization(write_intent);
  } catch (const std::exception& e) {
    return rewardResetResult(kPbftFinalizedPeriodApplyStatusRejected, period, block_hash, e.what());
  }

  rust::Vec<rustaxa::PbftFinalizationStorageWriteStage> stages;
  stages.push_back(std::move(stage));
  auto result =
      verified_votes_.applyPbftFinalizationStorageWrites(write_intent, std::move(stages), false);
  if (result.status != kPbftFinalizedPeriodApplyStatusApplied &&
      result.status != kPbftFinalizedPeriodApplyStatusAlreadyApplied) {
    return result;
  }

  commitRewardVotesResetForFinalization(write_intent);
  return result;
}

}  // namespace taraxa
