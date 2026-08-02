#pragma once

#include <array>
#include <cstdint>
#include <memory>
#include <mutex>
#include <optional>
#include <vector>

namespace rustaxa {
class BridgeConsensusNetworkApi;
}

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

/**
 * Owns the main-only Rust consensus network facade for one Network instance.
 *
 * Construction applies the sole production queue limits. Destruction releases
 * the opaque Rust handle after all shared users have released this wrapper.
 * The facade is internally synchronized, so capability and packet-handler
 * callers may share it without additional C++ locking.
 */
class ConsensusNetworkApi final {
 public:
  ConsensusNetworkApi();
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
