#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <vector>

namespace rustaxa {
class BridgeConsensusNetworkApi;
class BridgeConsensusApplication;
}  // namespace rustaxa

namespace taraxa {
class ConsensusApplication;
struct FullNodeConfig;
class TransactionManager;
using SharedConsensusApplication = std::shared_ptr<ConsensusApplication>;
}  // namespace taraxa

namespace taraxa::final_chain {
class FinalChain;
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

/** Physical tarcap leaves for one native Get-PBFT-sync response plan. */
struct PbftSyncRequestExecutor {
  std::function<bool(uint32_t, const std::vector<uint8_t>&)> send_packet;
  std::function<void()> clear_peer_syncing;
  std::function<void(uint8_t)> report_peer;
  std::function<void()> disconnect_peer;
};

/** Terminal native decision for one Get-PBFT-sync request. */
struct PbftSyncRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Physical tarcap leaf for one native previous-round next-vote response. */
struct PbftNextVotesBundleExecutor {
  std::function<bool(const std::vector<uint8_t>&)> send_bundle;
};

/** Terminal native decision for one previous-round next-vote request. */
struct PbftNextVotesBundleRequestOutcome {
  uint8_t status = 0;
  uint32_t queued_effect_count = 0;
  std::string error_code;
};

/** Terminal native decision for one proposed-PBFT-block bundle. */
struct PbftBlocksBundleOutcome {
  uint8_t status = 0;
  std::string error_code;
};

/** Transport-facing action selected by native PBFT-sync ingress. */
enum class PbftSyncIngressAction : uint8_t {
  kContinue,
  kDuplicate,
  kSyncComplete,
  kDrop,
  kStopSyncing,
  kMalicious,
  kQueueRejected,
};

/** Terminal native decision and compact transport facts for one PBFT-sync packet. */
struct PbftSyncIngressOutcome {
  PbftSyncIngressAction action = PbftSyncIngressAction::kDrop;
  std::string error_code;
  std::array<uint8_t, 32> block_hash{};
  uint64_t block_period = 0;
  uint64_t max_dag_level = 0;
  bool last_block = false;
  bool current_cert_present = false;
};

/** Narrow C++ transaction facts for one native double-voting slashing effect. */
struct PbftSyncSlashingTransaction {
  uint8_t status = 0;
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 20> contract_address{};
  std::array<uint8_t, 32> value{};
  uint64_t gas_limit = 0;
  std::vector<uint8_t> call_data;
};

/** Canonical fields for the retained PBFT-vote slashing transaction leaf. */
struct PbftVoteSlashingTransaction {
  uint8_t status = 0;
  std::array<uint8_t, 32> proof_hash{};
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 20> contract_address{};
  std::array<uint8_t, 32> value{};
  uint64_t gas_limit = 0;
  std::vector<uint8_t> call_data;
};

/** Concrete-EVM account facts used only by native PBFT-sync slashing admission. */
struct PbftSyncSlashingSubmitterFact {
  size_t wallet_index = 0;
  std::array<uint8_t, 32> nonce{};
  std::array<uint8_t, 32> balance{};
};

/** Narrow physical executor for a native double-voting slashing transaction. */
struct PbftSyncIngressExecutor {
  std::function<bool(const PbftSyncSlashingTransaction&)> submit_slashing_transaction;
};

/**
 * Owns the main-only Rust consensus network facade for one Network instance.
 *
 * Construction retains the application root and clones its network service for effect dispatch; it cannot create a
 * second protocol runtime or queue. Destruction releases the adapter and its shared root ownership. Native state access
 * is synchronized in Rust, while callers retain the lane lock across physical transport and acknowledgement.
 */
class ConsensusNetworkApi final {
 public:
  explicit ConsensusNetworkApi(SharedConsensusApplication consensus_application);
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
   * Routes and executes one canonical Get-PBFT-sync request on its transport lane.
   *
   * Rust validates the request, reads canonical period data and native proposed
   * blocks, selects reward-vote attachment, and orders all response effects.
   * The callbacks retain only packet sealing and peer-state operations. Version
   * 5 intentionally emits no proposed-block bundles; versions other than 5 and
   * 6 are rejected by the native request planner.
   */
  PbftSyncRequestOutcome servePbftSyncRequest(uint32_t tarcap_version, const std::array<uint8_t, 64>& peer_id,
                                              const std::vector<uint8_t>& request_rlp, uint64_t source_payload_id,
                                              const PbftSyncRequestExecutor& executor);

  /**
   * Routes and executes one previous-round next-vote request on its transport lane.
   *
   * Rust owns request eligibility, the live PBFT cursor snapshot, verified-vote
   * lookup, bundle validation, chunking, and send ordering. The caller supplies
   * only peer request fields and physical packet transport.
   */
  PbftNextVotesBundleRequestOutcome servePbftNextVotesBundleRequest(uint32_t transport_lane,
                                                                    const std::array<uint8_t, 64>& peer_id,
                                                                    uint64_t peer_period, uint64_t peer_round,
                                                                    uint64_t source_payload_id,
                                                                    const PbftNextVotesBundleExecutor& executor);

  /**
   * Admits one latest-tarcap proposed-block bundle into native PBFT state.
   *
   * Rust owns raw decoding, relevance and author checks, FinalChain DPoS
   * queries, and storage-first proposal publication. The caller retains only
   * peer-level error handling.
   */
  PbftBlocksBundleOutcome admitPbftBlocksBundle(const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id);

  /**
   * Admits one original PBFT-sync packet through the native application root.
   *
   * Rust owns exact decoding, deterministic prechecks, sequential weighted vote
   * admission, reward selection, and queue mutation. The executor is invoked
   * synchronously only for a typed slashing transaction effect; its Boolean
   * insertion result is reported before native ingress advances to the next
   * vote. Transport facts remain available for legacy peer bookkeeping without
   * materializing consensus objects in C++.
   */
  PbftSyncIngressOutcome admitPbftSyncPacket(const std::vector<uint8_t>& packet_rlp, uint64_t source_payload_id,
                                             const std::array<uint8_t, 64>& source_peer_id,
                                             const std::vector<PbftSyncSlashingSubmitterFact>& slashing_submitters,
                                             const PbftSyncIngressExecutor& executor);

  /** Reports the concrete transaction-insertion result for one native vote slashing effect. */
  bool reportPbftVoteSlashingSubmission(const std::array<uint8_t, 32>& proof_hash, bool transaction_inserted);

  /**
   * Signs, inserts, and reports one native PBFT-vote slashing transaction.
   *
   * The native effect selects the configured wallet and supplies canonical
   * nonce/value/calldata. C++ retains secret custody and concrete transaction
   * insertion. Invalid effects throw before signing; the returned value is the
   * native acknowledgement of the actual insertion result.
   */
  bool executePbftVoteSlashingTransaction(const PbftVoteSlashingTransaction& effect, const FullNodeConfig& config,
                                          TransactionManager& transaction_manager);

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
