#include "network/tarcap/packets_handlers/latest/transaction_packet_handler.hpp"

#include <cassert>
#include <stdexcept>

#include "network/tarcap/packets/latest/transaction_packet.hpp"
#ifdef RUSTAXA_ENABLE
#include "rustaxa-bridge/ffi.rs.h"
#endif
#include "transaction/transaction.hpp"
#include "transaction/transaction_manager.hpp"

namespace taraxa::network::tarcap {

#ifdef RUSTAXA_ENABLE
namespace {

constexpr uint8_t kNetworkEffectResultStatusOk = 0;
constexpr uint8_t kNetworkEffectResultStatusFailed = 1;
constexpr uint8_t kNetworkEffectKindRecordConsensusObject = 8;
constexpr uint8_t kNetworkObjectKindTransaction = 2;
constexpr uint32_t kNetworkPacketKindTransaction = 7;

rustaxa::NetworkApiConfig defaultNetworkApiConfig() {
  rustaxa::NetworkApiConfig config{};
  config.max_payload_bytes = 64 * 1024 * 1024;
  config.max_retained_payloads = 4096;
  config.max_effects_per_drain = 1024;
  return config;
}

rust::Vec<uint8_t> toBridgeBytes(const bytes &input) {
  rust::Vec<uint8_t> output;
  output.reserve(input.size());
  for (const auto byte : input) {
    output.push_back(static_cast<uint8_t>(byte));
  }
  return output;
}

}  // namespace

struct TransactionPacketHandler::RustConsensusNetworkApiHolder {
  RustConsensusNetworkApiHolder() : api(rustaxa::create_consensus_network_api(defaultNetworkApiConfig())) {}

  rust::Box<rustaxa::BridgeConsensusNetworkApi> api;
};
#endif

TransactionPacketHandler::TransactionPacketHandler(const FullNodeConfig &conf, std::shared_ptr<PeersState> peers_state,
                                                   std::shared_ptr<TimePeriodPacketsStats> packets_stats,
                                                   std::shared_ptr<TransactionManager> trx_mgr, const addr_t &node_addr,
                                                   const std::string &logs_prefix)
    : ITransactionPacketHandler(conf, std::move(peers_state), std::move(packets_stats), node_addr,
                                logs_prefix + "TRANSACTION_PH"),
      trx_mgr_(std::move(trx_mgr)) {
#ifdef RUSTAXA_ENABLE
  rust_consensus_network_api_ = std::make_unique<RustConsensusNetworkApiHolder>();
#endif
}

TransactionPacketHandler::~TransactionPacketHandler() = default;

inline void TransactionPacketHandler::process(const threadpool::PacketData &packet_data,
                                              const std::shared_ptr<TaraxaPeer> &peer) {
  // Decode packet rlp into packet object
  auto packet = decodePacketRlp<TransactionPacket>(packet_data.rlp_);

  if (packet.transactions.size() > kMaxTransactionsInPacket) {
    throw InvalidRlpItemsCountException("TransactionPacket:transactions", packet.transactions.size(),
                                        kMaxTransactionsInPacket);
  }

  if (packet.extra_transactions_hashes.size() > kMaxHashesInPacket) {
    throw InvalidRlpItemsCountException("TransactionPacket:hashes", packet.extra_transactions_hashes.size(),
                                        kMaxHashesInPacket);
  }

  // Extra hashes are hashes of transactions that were not sent as full transactions due to max limit, just mark them as
  // known for sender
  for (const auto &extra_tx_hash : packet.extra_transactions_hashes) {
    peer->markTransactionAsKnown(extra_tx_hash);
  }

  size_t unseen_txs_count = 0;
  // size_t data_size = 0;
  for (auto &transaction : packet.transactions) {
    const auto tx_hash = transaction->getHash();
    // data_size += transaction->getData().size();
    peer->markTransactionAsKnown(tx_hash);

#ifdef RUSTAXA_ENABLE
    rustaxa::NetworkTransactionAdmissionRequestEffects effects{};
    effects.peer_id = peer->getId().asArray();
    effects.transaction_hash = tx_hash.asArray();
    effects.transaction_rlp = toBridgeBytes(transaction->rlp());
    effects.source_payload_id = 0;
    effects.admit_transaction = true;
    (void)queueTransactionAdmissionRequestEffects(effects);

    auto processing_result = executeTransactionAdmissionEffect(std::move(transaction));
    if (processing_result.already_known) {
      continue;
    }
    unseen_txs_count++;
    if (processing_result.validation_failed) {
      std::ostringstream err_msg;
      err_msg << "DagBlock transaction " << tx_hash << " validation failed: " << processing_result.error;
      throw MaliciousPeerException(err_msg.str());
    }
    if (!processing_result.accepted) {
      std::ostringstream err_msg;
      err_msg << "DagBlock transaction " << tx_hash << " admission failed: " << processing_result.error;
      throw MaliciousPeerException(err_msg.str());
    }

    received_trx_count_++;
    if (processing_result.inserted) {
      unique_received_trx_count_++;
    }
    if (processing_result.overflow) {
      // Raise exception in trx pool is over the limit and this peer already has too many suspicious packets
      if (peer->reportSuspiciousPacket() && processing_result.overflow_over_limit) {
        std::ostringstream err_msg;
        err_msg << "Suspicious packets over the limit on DagBlock transaction " << tx_hash
                << " validation: " << processing_result.error;
        throw MaliciousPeerException(err_msg.str());
      }
    }
#else
    // Skip any transactions that are already known to the trx mgr
    if (trx_mgr_->isTransactionKnown(tx_hash)) {
      continue;
    }

    unseen_txs_count++;

    const auto [verified, reason] = trx_mgr_->verifyTransaction(transaction);
    if (!verified) {
      std::ostringstream err_msg;
      err_msg << "DagBlock transaction " << tx_hash << " validation failed: " << reason;
      throw MaliciousPeerException(err_msg.str());
    }

    received_trx_count_++;

    const auto status = trx_mgr_->insertValidatedTransaction(std::move(transaction));
    if (status == TransactionStatus::Inserted) {
      unique_received_trx_count_++;
    }
    if (status == TransactionStatus::Overflow) {
      // Raise exception in trx pool is over the limit and this peer already has too many suspicious packets
      if (peer->reportSuspiciousPacket() && trx_mgr_->nonProposableTransactionsOverTheLimit()) {
        std::ostringstream err_msg;
        err_msg << "Suspicious packets over the limit on DagBlock transaction " << tx_hash << " validation: " << reason;
        throw MaliciousPeerException(err_msg.str());
      }
    }
#endif
  }

  // // Allow 30% bigger size to support old version, to be removed
  // if (data_size > kMaxTransactionsSizeInPacket * 1.3) {
  //   std::ostringstream err_msg;
  //   err_msg << "Transactions packet data size over limit " << data_size;
  //   throw MaliciousPeerException(err_msg.str());
  // }

  if (!packet.transactions.empty()) {
    LOG(log_tr_) << "Received TransactionPacket with " << packet.transactions.size() << " transactions";
    LOG(log_dg_) << "Received TransactionPacket with " << packet.transactions.size()
                 << " unseen transactions:" << unseen_txs_count << " from: " << peer->getId().abridged();
  }
}

void TransactionPacketHandler::sendTransactions(std::shared_ptr<TaraxaPeer> peer,
                                                std::pair<SharedTransactions, std::vector<trx_hash_t>> &&transactions) {
  if (!peer) return;
  const auto peer_id = peer->getId();

  LOG(log_tr_) << "sendTransactions " << transactions.first.size() << " to " << peer_id;
  TransactionPacket packet{.transactions = std::move(transactions.first),
                           .extra_transactions_hashes = std::move(transactions.second)};

  if (sealAndSend(peer_id, SubprotocolPacketType::kTransactionPacket, encodePacketRlp(packet))) {
    for (const auto &trx : packet.transactions) {
      peer->markTransactionAsKnown(trx->getHash());
    }
    // Note: do not mark packet.extra_transactions_hashes as known for peer - we are sending just hashes, not full txs
  }
}

#ifdef RUSTAXA_ENABLE
rustaxa::NetworkIngressDecision TransactionPacketHandler::queueTransactionAdmissionRequestEffects(
    const rustaxa::NetworkTransactionAdmissionRequestEffects &effects) {
  assert(rust_consensus_network_api_);
  return rust_consensus_network_api_->api->consensus_network_queue_transaction_admission_request_effects(effects);
}

TransactionPacketHandler::TransactionProcessingResult TransactionPacketHandler::executeTransactionAdmissionEffect(
    std::shared_ptr<Transaction> &&transaction) {
  assert(rust_consensus_network_api_);
  const auto batch = rust_consensus_network_api_->api->consensus_network_drain_work(1);
  rust::Vec<rustaxa::NetworkEffectResult> results;
  results.reserve(batch.effects.size());

  TransactionProcessingResult processing_result{};
  for (const auto &effect : batch.effects) {
    rustaxa::NetworkEffectResult result{};
    result.effect_id = effect.effect_id;
    result.kind = effect.kind;
    result.peer_id = effect.peer_id;
    result.packet_kind = effect.packet_kind;
    result.object_kind = effect.object_kind;
    result.object_hash = effect.object_hash;
    result.status = kNetworkEffectResultStatusOk;

    try {
      if (effect.kind != kNetworkEffectKindRecordConsensusObject ||
          effect.object_kind != kNetworkObjectKindTransaction || effect.packet_kind != kNetworkPacketKindTransaction ||
          !transaction || transaction->getHash().asArray() != effect.object_hash ||
          transaction->rlp() != bytes(effect.payload_bytes.begin(), effect.payload_bytes.end())) {
        throw std::runtime_error("Network API transaction admission effect missing matching live transaction");
      }

      if (trx_mgr_->isTransactionKnown(transaction->getHash())) {
        processing_result.already_known = true;
      } else {
        const auto [verified, reason] = trx_mgr_->verifyTransaction(transaction);
        if (!verified) {
          processing_result.validation_failed = true;
          processing_result.error = reason;
          throw std::runtime_error(reason);
        }

        const auto status = trx_mgr_->insertValidatedTransaction(std::move(transaction));
        processing_result.accepted = true;
        processing_result.inserted = status == TransactionStatus::Inserted;
        processing_result.overflow = status == TransactionStatus::Overflow;
        processing_result.overflow_over_limit =
            processing_result.overflow && trx_mgr_->nonProposableTransactionsOverTheLimit();
      }
    } catch (const std::exception &e) {
      result.status = kNetworkEffectResultStatusFailed;
      result.diagnostic = e.what();
      if (!processing_result.validation_failed) {
        processing_result.error = e.what();
      }
    }

    results.push_back(std::move(result));
  }

  if (!results.empty()) {
    (void)rust_consensus_network_api_->api->consensus_network_report_effect_results(std::move(results));
  }

  return processing_result;
}
#endif

}  // namespace taraxa::network::tarcap
