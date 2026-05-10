#include "vdf/sortition.hpp"

#include <libdevcore/CommonData.h>
#include <libdevcore/CommonJS.h>

#ifdef RUSTAXA_ENABLE_VDF
#include "rustaxa-bridge/ffi.rs.h"
#else
#include "ProverWesolowski.h"
#endif
#include "common/encoding_rlp.hpp"
#include "common/util.hpp"
namespace taraxa::vdf_sortition {
#ifndef RUSTAXA_ENABLE_VDF
using namespace vdf;
#else
namespace {
rustaxa::LegacySortitionParams toRustSortitionParams(SortitionParams const& config) {
  return rustaxa::LegacySortitionParams{
      .vrf_threshold_upper = static_cast<uint16_t>(config.vrf.threshold_upper),
      .vdf_difficulty_min = static_cast<uint16_t>(config.vdf.difficulty_min),
      .vdf_difficulty_max = static_cast<uint16_t>(config.vdf.difficulty_max),
      .vdf_difficulty_stale = static_cast<uint16_t>(config.vdf.difficulty_stale),
      .vdf_lambda_bound = static_cast<uint16_t>(config.vdf.lambda_bound),
  };
}

std::string rustSortitionError(uint8_t status, rust::String const& error) {
  auto message = "Rust VDF/VRF sortition status " + std::to_string(status);
  if (!error.empty()) {
    message += ": " + static_cast<std::string>(error);
  }
  return message;
}
}  // namespace
#endif

VdfSortition::VdfSortition(const SortitionParams& config, const vrf_sk_t& sk, const bytes& vrf_input,
                           uint64_t vote_count, uint64_t total_vote_count)
#ifdef RUSTAXA_ENABLE_VDF
{
  const auto normalized_vote_count = static_cast<uint16_t>(vote_count * kVotesProportion / total_vote_count);
  rust::Slice<const uint8_t> vrf_input_slice{vrf_input.data(), vrf_input.size()};
  const auto result = rustaxa::prove_legacy_vrf_sortition(sk.asArray(), vrf_input_slice, normalized_vote_count);
  if (!result.ok) {
    throw InvalidVdfSortition("VRF proof creation failed. " + rustSortitionError(result.status, result.error));
  }

  proof_ = vrf_proof_t(bytes(result.proof.begin(), result.proof.end()));
  output_ = vrf_output_t(bytes(result.output.begin(), result.output.end()));
  threshold_ = result.threshold;
  difficulty_ = calculateDifficulty(config);
}
#else
    : VrfSortitionBase(sk, vrf_input, vote_count * kVotesProportion / total_vote_count) {
  difficulty_ = calculateDifficulty(config);
}
#endif

bool VdfSortition::isStale(SortitionParams const& config) const { return difficulty_ == config.vdf.difficulty_stale; }

uint16_t VdfSortition::calculateDifficulty(SortitionParams const& config) const {
  uint16_t difficulty = 0;
  // Threshold is the minimum for all the individual stake votes. Increase it by kThresholdCorrection for easier
  // difficulty adjustment
  uint32_t corrected_threshold = threshold_ * kThresholdCorrection;
  const auto number_of_difficulties = config.vdf.difficulty_max - config.vdf.difficulty_min + 1;
  if (corrected_threshold >= config.vrf.threshold_upper) {
    difficulty = config.vdf.difficulty_stale;
  } else {
    difficulty =
        config.vdf.difficulty_min + corrected_threshold / (config.vrf.threshold_upper / number_of_difficulties);
  }

  return difficulty;
}

VdfSortition::VdfSortition(bytes const& b) {
  if (b.empty()) {
    return;
  }

  dev::RLP rlp(b);
  util::rlp_tuple(util::RLPDecoderRef(rlp, true), proof_, vdf_sol_.first, vdf_sol_.second, difficulty_);
}

VdfSortition::VdfSortition(Json::Value const& json) {
  proof_ = vrf_proof_t(json["proof"].asString());
  vdf_sol_.first = dev::fromHex(json["sol1"].asString());
  vdf_sol_.second = dev::fromHex(json["sol2"].asString());
  difficulty_ = dev::jsToInt(json["difficulty"].asString());
}

bytes VdfSortition::rlp() const {
  dev::RLPStream s;
  s.appendList(4);
  s << proof_;
  s << vdf_sol_.first;
  s << vdf_sol_.second;
  s << difficulty_;
  return s.invalidate();
}

Json::Value VdfSortition::getJson() const {
  Json::Value res;
  res["proof"] = dev::toJS(proof_);
  res["sol1"] = dev::toJS(dev::toHex(vdf_sol_.first));
  res["sol2"] = dev::toJS(dev::toHex(vdf_sol_.second));
  res["difficulty"] = dev::toJS(difficulty_);
  return res;
}

void VdfSortition::computeVdfSolution(const SortitionParams& config, const bytes& msg,
                                      const std::atomic_bool& cancelled) {
  auto t1 = getCurrentTimeMilliSeconds();
#ifdef RUSTAXA_ENABLE_VDF
  rust::Slice<const uint8_t> msgSlice{msg.data(), msg.size()};
  rust::Slice<const uint8_t> NSlice{N.data(), N.size()};
  const auto vdf = rustaxa::make_vdf(config.vdf.lambda_bound, difficulty_, msgSlice, NSlice);
  auto cancellation_token = rustaxa::make_cancellation_token_with_atomic(reinterpret_cast<const bool*>(&cancelled));
  const auto solution = rustaxa::prove(*vdf, *cancellation_token);
  const auto proof = rustaxa::solution_get_proof(*solution);
  const auto output = rustaxa::solution_get_output(*solution);
  vdf_sol_ = std::make_pair(bytes(proof.begin(), proof.end()), bytes(output.begin(), output.end()));
#else
  VerifierWesolowski verifier(config.vdf.lambda_bound, difficulty_, msg, N);
  ProverWesolowski prover;
  vdf_sol_ = prover(verifier, cancelled);  // this line takes time ...
#endif
  auto t2 = getCurrentTimeMilliSeconds();
  vdf_computation_time_ = t2 - t1;
}

void VdfSortition::verifyVdf(SortitionParams const& config, bytes const& vrf_input, const vrf_pk_t& pk,
                             bytes const& vdf_input, uint64_t vote_count, uint64_t total_vote_count) const {
#ifdef RUSTAXA_ENABLE_VDF
  const auto encoded = rlp();
  rust::Slice<const uint8_t> sortition_rlp_slice{encoded.data(), encoded.size()};
  rust::Slice<const uint8_t> vrf_input_slice{vrf_input.data(), vrf_input.size()};
  rust::Slice<const uint8_t> vdf_input_slice{vdf_input.data(), vdf_input.size()};
  const auto result = rustaxa::verify_legacy_vdf_sortition(toRustSortitionParams(config), pk.asArray(),
                                                           sortition_rlp_slice, vrf_input_slice, vdf_input_slice,
                                                           vote_count, total_vote_count);
  output_ = vrf_output_t(bytes(result.vrf_output.begin(), result.vrf_output.end()));
  threshold_ = result.vrf_threshold;
  if (!result.ok) {
    throw InvalidVdfSortition("VDF solution verification failed. " + rustSortitionError(result.status, result.error) +
                              ", VDF input " + dev::toHex(vdf_input) +
                              ", lambda " + std::to_string(config.vdf.lambda_bound) +
                              ", difficulty " + std::to_string(getDifficulty()) +
                              ", expected: " + std::to_string(result.expected_difficulty) +
                              ", vrf_params: ( threshold_upper: " + std::to_string(config.vrf.threshold_upper) +
                              ") THRESHOLD: " + std::to_string(threshold_));
  }
  return;
#endif
  // Verify VRF output
  if (!verifyVrf(pk, vrf_input, vote_count * kVotesProportion / total_vote_count)) {
    throw InvalidVdfSortition("VRF verify failed. VRF input " + dev::toHex(vrf_input));
  }

  const auto expected = calculateDifficulty(config);
  if (difficulty_ != expected) {
    throw InvalidVdfSortition("VDF solution verification failed. Incorrect difficulty. VDF input " +
                              dev::toHex(vdf_input) + ", lambda " + std::to_string(config.vdf.lambda_bound) +
                              ", difficulty " + std::to_string(getDifficulty()) +
                              ", expected: " + std::to_string(expected) +
                              ", vrf_params: ( threshold_upper: " + std::to_string(config.vrf.threshold_upper) +
                              ") THRESHOLD: " + std::to_string(threshold_));
  }

  // Verify VDF solution
#ifdef RUSTAXA_ENABLE_VDF
  rust::Slice<const uint8_t> msgSlice{vdf_input.data(), vdf_input.size()};
  rust::Slice<const uint8_t> NSlice{N.data(), N.size()};
  const auto vdf = rustaxa::make_vdf(config.vdf.lambda_bound, getDifficulty(), msgSlice, NSlice);
  rust::Slice<const uint8_t> proofSlice{vdf_sol_.first.data(), vdf_sol_.first.size()};
  rust::Slice<const uint8_t> outputSlice{vdf_sol_.second.data(), vdf_sol_.second.size()};
  const auto solution = rustaxa::make_solution(proofSlice, outputSlice);
  if (!rustaxa::verify(*vdf, *solution)) {
#else
  VerifierWesolowski verifier(config.vdf.lambda_bound, getDifficulty(), vdf_input, N);
  if (!verifier(vdf_sol_)) {
#endif
    throw InvalidVdfSortition("VDF solution verification failed. VDF input " + dev::toHex(vdf_input) + ", lambda " +
                              std::to_string(config.vdf.lambda_bound) + ", difficulty " +
                              std::to_string(getDifficulty()));
  }
}

bool VdfSortition::verifyVrf(const vrf_pk_t& pk, const bytes& vrf_input, uint16_t vote_count) const {
  return VrfSortitionBase::verify(pk, vrf_input, vote_count);
}

uint16_t VdfSortition::getDifficulty() const { return difficulty_; }

}  // namespace taraxa::vdf_sortition
