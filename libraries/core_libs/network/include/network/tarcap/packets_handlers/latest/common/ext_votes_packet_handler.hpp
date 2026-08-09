#pragma once

#include <memory>
#include <optional>

#include "network/tarcap/packets/latest/get_pbft_sync_packet.hpp"
#include "network/tarcap/packets/latest/votes_bundle_packet.hpp"
#include "network/tarcap/packets_handlers/latest/common/exceptions.hpp"
#include "network/tarcap/tarcap_version.hpp"
#include "packet_handler.hpp"
#include "pbft/pbft_manager.hpp"
#include "vote/pbft_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"
#include "vote_manager/vote_manager.hpp"
#ifdef RUSTAXA_ENABLE
#include "network/consensus_network_api.hpp"
#include "rustaxa-bridge/ffi.rs.h"
#endif

namespace taraxa::network::tarcap {

/**
 * @brief ExtVotesPacketHandler is extended abstract PacketHandler with added functions that are used in packet
 *        handlers that process pbft votes
 */
class ExtVotesPacketHandler : public PacketHandler {
 public:
  /**
   * Outcome returned by the PBFT vote application-effect executor.
   *
   * Rust-enabled builds populate these fields from the exact-ID-correlated
   * VoteManager application effect. `accepted` and `already_present` are
   * mutually exclusive; the remaining flags describe dependent work that
   * Rust may release after the application result is acknowledged. Legacy
   * builds use only `accepted` and retain their existing mark/gossip path.
   * `cancelled` is set only for a bundle member whose exact admission effect
   * Rust removed after an earlier member terminated the bundle session.
   */
  struct VoteProcessingResult {
    bool accepted = false;
    bool already_present = false;
    bool mark_vote_known = false;
    bool gossip_vote = false;
    bool report_slashing = false;
    bool cancelled = false;
  };

  ExtVotesPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                        std::shared_ptr<TimePeriodPacketsStats> packets_stats, std::shared_ptr<PbftManager> pbft_mgr,
                        std::shared_ptr<PbftChain> pbft_chain, std::shared_ptr<VoteManager> vote_mgr,
#ifndef RUSTAXA_ENABLE
                        std::shared_ptr<SlashingManager> slashing_manager,
#else
                        network::ConsensusNetworkApiShared consensus_network_api, TarcapVersion transport_lane,
#endif
                        const addr_t& node_addr, const std::string& log_channel_name);

  virtual ~ExtVotesPacketHandler();
  ExtVotesPacketHandler(const ExtVotesPacketHandler&) = delete;
  ExtVotesPacketHandler(ExtVotesPacketHandler&&) = delete;
  ExtVotesPacketHandler& operator=(const ExtVotesPacketHandler&) = delete;
  ExtVotesPacketHandler& operator=(ExtVotesPacketHandler&&) = delete;

  /**
   * @brief Process vote
   *
   * @param vote
   * @param pbft_block
   * @param peer
   * @param validate_max_round_step
   * @param allow_gossip whether an accepted vote may be regossiped
   * @return result reported by the VoteManager application leaf
   */
  VoteProcessingResult processVote(const std::shared_ptr<PbftVote>& vote, const std::shared_ptr<PbftBlock>& pbft_block,
                                   const std::shared_ptr<TaraxaPeer>& peer, bool validate_max_round_step,
                                   bool allow_gossip);

  /**
   * @brief Checks is vote is relevant for current pbft state in terms of period, round and type
   * @param vote
   * @return true if vote is relevant for current pbft state, otherwise false
   */
  bool isPbftRelevantVote(const std::shared_ptr<PbftVote>& vote) const;

  void requestPbftNextVotesAtPeriodRound(const dev::p2p::NodeID& peerID, PbftPeriod pbft_period, PbftRound pbft_round);

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkIngressDecision ingestPbftVote(const rustaxa::PbftVoteIngressFact& fact,
                                                 const rustaxa::NetworkPbftVoteIngressContext& context);
  rust::Vec<rustaxa::NetworkIngressDecision> ingestPbftVoteBundle(
      const rustaxa::PbftVoteIngressFact& reference, rust::Vec<rustaxa::PbftVoteIngressFact> votes,
      rust::Vec<rustaxa::NetworkPbftVoteIngressContext> contexts);
  /**
   * Executes Rust-owned effects and returns the matching vote-admission leaf result, if any.
   * The caller must retain this handler's transport-lane lock from ingress through completion.
   * A matching executor failure is acknowledged to Rust so dependent work is cancelled, then
   * surfaced as an exception; callers must not request later admission IDs from that operation.
   */
  VoteProcessingResult executeConsensusNetworkEffects(size_t budget, std::optional<uint64_t> application_effect_id,
                                                      bool stop_after_correlated_application = false,
                                                      bool allow_cancelled_application = false,
                                                      std::optional<uint64_t> source_payload_id = std::nullopt);
#endif

 private:
  /**
   * @brief Validates vote period, round and step against max values from config
   *
   * @param vote to be validated
   * @param peer
   * @param validate_max_round_step validate also max round and step
   * @return <true, ""> vote validation passed, otherwise <false, "err msg">
   */
  std::pair<bool, std::string> validateVotePeriodRoundStep(const std::shared_ptr<PbftVote>& vote,
                                                           const std::shared_ptr<TaraxaPeer>& peer,
                                                           bool validate_max_round_step);

  /**
   * @brief Validates provided vote if voted value == provided block
   *
   * @param vote
   * @param pbft_block
   * @return true if validation successful, otherwise false
   */
  bool validateVoteAndBlock(const std::shared_ptr<PbftVote>& vote, const std::shared_ptr<PbftBlock>& pbft_block) const;

 protected:
  constexpr static size_t kMaxVotesInBundleRlp{1000};
  constexpr static std::chrono::seconds kSyncRequestInterval = std::chrono::seconds(10);

  mutable std::chrono::system_clock::time_point last_votes_sync_request_time_;
  mutable std::chrono::system_clock::time_point last_pbft_block_sync_request_time_;

  std::shared_ptr<PbftManager> pbft_mgr_;
  std::shared_ptr<PbftChain> pbft_chain_;
  std::shared_ptr<VoteManager> vote_mgr_;
#ifndef RUSTAXA_ENABLE
  std::shared_ptr<SlashingManager> slashing_manager_;
#endif

#ifdef RUSTAXA_ENABLE
  network::ConsensusNetworkApiShared rust_consensus_network_api_;
  const TarcapVersion transport_lane_;
#endif
};

}  // namespace taraxa::network::tarcap
