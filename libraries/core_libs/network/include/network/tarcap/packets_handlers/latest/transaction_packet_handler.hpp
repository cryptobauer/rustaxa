#pragma once

#include <memory>
#include <string>

#include "network/tarcap/packets/latest/transaction_packet.hpp"
#include "network/tarcap/packets_handlers/interface/transaction_packet_handler.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "transaction/transaction.hpp"

namespace taraxa {
class TransactionManager;
enum class TransactionStatus;
}  // namespace taraxa

namespace taraxa::network::tarcap {

class TransactionPacketHandler : public ITransactionPacketHandler {
 public:
  TransactionPacketHandler(const FullNodeConfig& conf, std::shared_ptr<PeersState> peers_state,
                           std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                           std::shared_ptr<TransactionManager> trx_mgr, const addr_t& node_addr,
                           const std::string& logs_prefix = "");
  ~TransactionPacketHandler() override;

  /**
   * @brief Send transactions
   *
   * @param peer peer to send transactions to
   * @param transactions serialized transactions
   *
   */
  void sendTransactions(std::shared_ptr<TaraxaPeer> peer,
                        std::pair<SharedTransactions, std::vector<trx_hash_t>>&& transactions) override;

  // Packet type that is processed by this handler
  static constexpr SubprotocolPacketType kPacketType_ = SubprotocolPacketType::kTransactionPacket;

 private:
  virtual void process(const threadpool::PacketData& packet_data, const std::shared_ptr<TaraxaPeer>& peer) override;

  struct TransactionProcessingResult {
    bool already_known = false;
    bool accepted = false;
    bool inserted = false;
    bool overflow = false;
    bool overflow_over_limit = false;
    bool validation_failed = false;
    std::string error;
  };

#ifdef RUSTAXA_ENABLE
  rustaxa::NetworkIngressDecision queueTransactionAdmissionRequestEffects(
      const rustaxa::NetworkTransactionAdmissionRequestEffects& effects);
  TransactionProcessingResult executeTransactionAdmissionEffect(std::shared_ptr<Transaction>&& transaction);
#endif

 protected:
  std::shared_ptr<TransactionManager> trx_mgr_;
#ifdef RUSTAXA_ENABLE
  struct RustConsensusNetworkApiHolder;
  std::unique_ptr<RustConsensusNetworkApiHolder> rust_consensus_network_api_;
#endif

  std::atomic<uint64_t> received_trx_count_{0};
  std::atomic<uint64_t> unique_received_trx_count_{0};
};

}  // namespace taraxa::network::tarcap
