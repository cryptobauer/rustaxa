#pragma once

#include <array>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace rustaxa {
class BridgeConsensusNetworkApi;
class BridgePbftService;
}  // namespace rustaxa

namespace taraxa::network {

/** Plain network-owned facts for one max-chain peer candidate. */
struct ConsensusPeerCandidate {
  std::array<uint8_t, 64> peer_id{};
  uint64_t pbft_chain_size = 0;
  uint64_t dag_level = 0;
  bool is_light_node = false;
  uint64_t light_node_history = 0;
  bool peer_dag_synced = false;
  bool peer_dag_syncing = false;
  bool dag_sync_allowed = false;
};

/** Physical tarcap leaves for one native pillar-vote bundle response. */
struct PillarVotesBundleExecutor {
  std::function<bool(const std::vector<uint8_t>&)> send_bundle;
  std::function<void(const std::array<uint8_t, 32>&)> mark_vote_known;
  std::function<void(uint8_t)> report_peer;
  std::function<void()> disconnect_peer;
};

/** Terminal native decision for one pillar-vote bundle request. */
struct PillarVotesBundleRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/**
 * Owns the main-only Rust consensus network facade for one Network instance.
 *
 * Construction clones the network service already owned by the application
 * PBFT root; it cannot create a second protocol runtime or queue. Destruction
 * releases only this opaque adapter. Native state access is synchronized in
 * Rust, while callers retain the lane lock across physical transport and
 * acknowledgement.
 */
class ConsensusNetworkApi final {
 public:
  explicit ConsensusNetworkApi(const rustaxa::BridgePbftService& service);
  ~ConsensusNetworkApi();

  ConsensusNetworkApi(const ConsensusNetworkApi&) = delete;
  ConsensusNetworkApi(ConsensusNetworkApi&&) = delete;
  ConsensusNetworkApi& operator=(const ConsensusNetworkApi&) = delete;
  ConsensusNetworkApi& operator=(ConsensusNetworkApi&&) = delete;

  /** Returns the live Rust facade owned by this wrapper. */
  rustaxa::BridgeConsensusNetworkApi& api() noexcept;
  /** Returns the live Rust facade owned by this wrapper. */
  const rustaxa::BridgeConsensusNetworkApi& api() const noexcept;

  /**
   * Locks one transport lane across effect drain, physical execution, and acknowledgement.
   *
   * Callers for the same lane are serialized in drain order, while distinct lanes may execute concurrently. The
   * returned lock releases the lane when destroyed.
   */
  std::unique_lock<std::mutex> lockTransportLane(uint32_t transport_lane);

  /**
   * Routes and executes one pillar-vote bundle request on its transport lane.
   *
   * Rust owns schedule validation, vote lookup/validation, chunking, effect
   * ordering, and dependency acknowledgement. The callbacks perform only
   * packet transport and peer bookkeeping; a failed chunk send suppresses its
   * marks without preventing later independent chunks.
   */
  PillarVotesBundleRequestOutcome servePillarVotesBundleRequest(uint32_t transport_lane,
                                                                const std::array<uint8_t, 64>& peer_id, uint64_t period,
                                                                const std::array<uint8_t, 32>& pillar_block_hash,
                                                                uint64_t source_payload_id,
                                                                const PillarVotesBundleExecutor& executor);

  /**
   * Selects a serviceable max-chain peer from network-owned facts.
   *
   * The returned peer id identifies the selected candidate; no value means
   * that Rust found no serviceable peer. FFI conversion remains private to
   * this wrapper.
   */
  std::optional<std::array<uint8_t, 64>> selectMaxChainPeer(
      uint64_t local_pbft_syncing_period, const std::vector<ConsensusPeerCandidate>& candidates) const;

 private:
  class Impl;
  std::unique_ptr<Impl> impl_;
};

/** Shared lifetime used by Network and every Rust-mode tarcap consumer. */
using ConsensusNetworkApiShared = std::shared_ptr<ConsensusNetworkApi>;

}  // namespace taraxa::network
