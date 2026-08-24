#include "consensus/consensus_host_ports.hpp"

#ifdef RUSTAXA_ENABLE

#include <libdevcrypto/Common.h>

#include <algorithm>
#include <array>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <exception>
#include <mutex>
#include <stdexcept>
#include <thread>
#include <utility>
#include <vector>

#include "common/vrf_wrapper.hpp"
#include "config/config.hpp"
#include "consensus/consensus_application.hpp"
#include "dag/dag_block.hpp"
#include "final_chain/final_chain.hpp"
#include "network/network.hpp"
#include "pbft/pbft_block.hpp"
#include "pbft/period_data.hpp"
#include "rustaxa-bridge/application_host_ffi.rs.h"
#include "vote/pbft_vote.hpp"
#include "vote/pillar_vote.hpp"
#include "vote/votes_bundle_rlp.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kWaitElapsed = 0;
constexpr uint8_t kWaitStopped = 1;

template <typename Report>
Report failedReport(const rustaxa::HostEffectId& effect_id, const char* error) {
  Report report{};
  report.effect_id = effect_id;
  report.succeeded = false;
  report.error_code = rust::String(error);
  return report;
}

rust::Vec<uint8_t> toRustBytes(const dev::bytes& bytes) {
  rust::Vec<uint8_t> result;
  result.reserve(bytes.size());
  for (const auto byte : bytes) {
    result.push_back(byte);
  }
  return result;
}

dev::bytes fromRustBytes(const rust::Vec<uint8_t>& bytes) { return {bytes.begin(), bytes.end()}; }

template <typename Value>
std::array<uint8_t, 32> toBridgeU256(const Value& value) {
  std::array<uint8_t, 32> out{};
  const auto bytes = dev::toBigEndian(value);
  if (bytes.size() > out.size()) {
    throw std::runtime_error("host integer exceeds 32 bytes");
  }
  std::copy(bytes.begin(), bytes.end(), out.begin() + static_cast<std::ptrdiff_t>(out.size() - bytes.size()));
  return out;
}

addr_t fromHostAddress(const std::array<uint8_t, 20>& address) {
  return addr_t(address.data(), addr_t::ConstructFromPointer);
}

rustaxa::HostTransportReport successfulTransportReport(const rustaxa::HostEffectId& effect_id) {
  rustaxa::HostTransportReport report{};
  report.effect_id = effect_id;
  report.succeeded = true;
  return report;
}

}  // namespace

class ConsensusProcessPort::Impl final {
 public:
  void reset() {
    const std::scoped_lock lock(mutex);
    stop_requested = false;
  }

  void stop() {
    {
      const std::scoped_lock lock(mutex);
      stop_requested = true;
    }
    cv.notify_all();
  }

  mutable std::mutex mutex;
  mutable std::condition_variable cv;
  bool stop_requested{false};
};

ConsensusProcessPort::ConsensusProcessPort() : impl_(std::make_unique<Impl>()) {}
ConsensusProcessPort::~ConsensusProcessPort() = default;

rustaxa::HostWaitReport ConsensusProcessPort::consensusWait(const rustaxa::HostWaitRequest& request) const {
  std::unique_lock lock(impl_->mutex);
  const auto stopped =
      impl_->cv.wait_for(lock, std::chrono::milliseconds(request.delay_ms), [this] { return impl_->stop_requested; });
  rustaxa::HostWaitReport report{};
  report.effect_id = request.effect_id;
  report.outcome = stopped ? kWaitStopped : kWaitElapsed;
  return report;
}

bool ConsensusProcessPort::consensusStopRequested([[maybe_unused]] uint64_t generation) const {
  const std::scoped_lock lock(impl_->mutex);
  return impl_->stop_requested;
}

uint64_t ConsensusProcessPort::consensusNowMillis() const {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch())
          .count());
}

uint64_t ConsensusProcessPort::consensusUnixTimeSeconds() const {
  return static_cast<uint64_t>(
      std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count());
}

class ConsensusSignerPort::Impl final {
 public:
  explicit Impl(const FullNodeConfig& config) : wallets(config.wallets) {}

  std::vector<WalletConfig> wallets;
};

ConsensusSignerPort::ConsensusSignerPort(const FullNodeConfig& config) : impl_(std::make_unique<Impl>(config)) {}
ConsensusSignerPort::~ConsensusSignerPort() = default;

rustaxa::HostSignReport ConsensusSignerPort::consensusSignDigest(const rustaxa::HostSignRequest& request) const {
  if (request.wallet_index >= impl_->wallets.size()) {
    return failedReport<rustaxa::HostSignReport>(request.effect_id, "SIGNER_WALLET_INDEX_OUT_OF_RANGE");
  }

  rustaxa::HostSignReport report{};
  report.effect_id = request.effect_id;
  const auto digest = dev::h256(request.digest.data(), dev::h256::ConstructFromPointer);
  report.signature = toRustBytes(dev::sign(impl_->wallets[request.wallet_index].node_secret, digest).asBytes());
  report.succeeded = true;
  return report;
}

rustaxa::HostVrfReport ConsensusSignerPort::consensusProveVrf(const rustaxa::HostVrfRequest& request) const {
  if (request.wallet_index >= impl_->wallets.size()) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_WALLET_INDEX_OUT_OF_RANGE");
  }

  const auto& wallet = impl_->wallets[request.wallet_index];
  const auto message = fromRustBytes(request.message);
  const auto proof = vrf_wrapper::getVrfProof(wallet.vrf_secret, message);
  if (!proof) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_PROOF_FAILED");
  }
  const auto output = vrf_wrapper::getVrfOutput(wallet.vrf_pk, *proof, message);
  if (!output) {
    return failedReport<rustaxa::HostVrfReport>(request.effect_id, "VRF_OUTPUT_FAILED");
  }

  rustaxa::HostVrfReport report{};
  report.effect_id = request.effect_id;
  report.succeeded = true;
  report.proof = toRustBytes(proof->asBytes());
  report.output = toRustBytes(output->asBytes());
  return report;
}

class ConsensusTransportPort::Impl final {
 public:
  std::shared_ptr<Network> network() const {
    const std::scoped_lock lock(mutex);
    return network_.lock();
  }

  void attach(std::weak_ptr<Network> network) {
    const std::scoped_lock lock(mutex);
    network_ = std::move(network);
  }

 private:
  mutable std::mutex mutex;
  std::weak_ptr<Network> network_;
};

ConsensusTransportPort::ConsensusTransportPort() : impl_(std::make_unique<Impl>()) {}
ConsensusTransportPort::~ConsensusTransportPort() = default;

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipVote(
    const rustaxa::HostGossipVoteRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    auto vote = std::make_shared<PbftVote>(fromRustBytes(request.vote_rlp));
    std::shared_ptr<PbftBlock> block;
    if (!request.proposed_block_rlp.empty()) {
      block = std::make_shared<PbftBlock>(fromRustBytes(request.proposed_block_rlp));
    }
    network->gossipVote(vote, block, request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_VOTE_FAILED");
  }
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipVoteBundle(
    const rustaxa::HostGossipVoteBundleRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    const auto votes_bundle_rlp = fromRustBytes(request.votes_bundle_rlp);
    auto votes = decodePbftVotesBundleRlp(dev::RLP(votes_bundle_rlp));
    network->gossipVotesBundle(votes, request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_VOTE_BUNDLE_FAILED");
  }
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusGossipPillarVote(
    const rustaxa::HostGossipPillarVoteRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  try {
    network->gossipPillarBlockVote(std::make_shared<PillarVote>(fromRustBytes(request.pillar_vote_rlp)),
                                   request.rebroadcast);
    return successfulTransportReport(request.effect_id);
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "GOSSIP_PILLAR_VOTE_FAILED");
  }
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusSetSyncPeriod(
    const rustaxa::HostSetSyncPeriodRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  network->setSyncStatePeriod(request.period);
  return successfulTransportReport(request.effect_id);
}

rustaxa::HostTransportStatus ConsensusTransportPort::consensusTransportStatus() const {
  rustaxa::HostTransportStatus status{};
  if (const auto network = impl_->network()) {
    status.available = true;
    status.pbft_syncing = network->pbft_syncing();
  }
  return status;
}

rustaxa::HostTransportReport ConsensusTransportPort::consensusReportMaliciousPeer(
    const rustaxa::HostMaliciousPeerRequest& request) const {
  const auto network = impl_->network();
  if (!network) {
    return failedReport<rustaxa::HostTransportReport>(request.effect_id, "TRANSPORT_UNAVAILABLE");
  }
  network->handleMaliciousSyncPeer(dev::p2p::NodeID(request.peer_id.data(), dev::p2p::NodeID::ConstructFromPointer));
  return successfulTransportReport(request.effect_id);
}

class ExternalEvmPort::Impl final {
 public:
  explicit Impl(std::shared_ptr<final_chain::FinalChain> final_chain) : final_chain(std::move(final_chain)) {
    if (!this->final_chain) {
      throw std::invalid_argument("ExternalEvmPort requires FinalChain");
    }
  }

  std::shared_ptr<final_chain::FinalChain> final_chain;
};

ExternalEvmPort::ExternalEvmPort(std::shared_ptr<final_chain::FinalChain> final_chain)
    : impl_(std::make_unique<Impl>(std::move(final_chain))) {}
ExternalEvmPort::~ExternalEvmPort() = default;

rustaxa::HostPillarAnchorStateReport ExternalEvmPort::consensusLoadPillarAnchorState(
    const rustaxa::HostPillarAnchorStateRequest& request) const {
  rustaxa::HostPillarAnchorStateReport report{};
  report.effect_id = request.effect_id;
  try {
    const auto header = impl_->final_chain->blockHeader(request.period);
    if (!header) {
      return failedReport<rustaxa::HostPillarAnchorStateReport>(request.effect_id, "PILLAR_ANCHOR_HEADER_MISSING");
    }
    report.succeeded = true;
    report.block_header_rlp = toRustBytes(util::rlp_enc(*header));
    report.state_root = header->state_root.asArray();
    report.bridge_root = impl_->final_chain->getBridgeRoot(request.period).asArray();
    report.bridge_epoch = impl_->final_chain->getBridgeEpoch(request.period).asArray();
    const auto validator_vote_counts =
        impl_->final_chain->dposValidatorsEligibleVoteCounts(request.pillar_block_period);
    report.validator_vote_counts.reserve(validator_vote_counts.size());
    for (const auto& validator : validator_vote_counts) {
      rustaxa::HostValidatorVoteCount fact{};
      fact.address = validator.addr.asArray();
      fact.vote_count = validator.vote_count;
      report.validator_vote_counts.push_back(std::move(fact));
    }
    report.signer_vote_counts.reserve(request.signer_addresses.size());
    for (const auto& signer : request.signer_addresses) {
      report.signer_vote_counts.push_back(
          impl_->final_chain->dposEligibleVoteCount(request.pillar_block_period, fromHostAddress(signer.bytes)));
    }
    report.total_eligible_vote_count = impl_->final_chain->dposEligibleTotalVoteCount(request.pillar_block_period);
  } catch (const std::exception& error) {
    const auto error_code = std::string("PILLAR_ANCHOR_STATE_READ_FAILED: ") + error.what();
    return failedReport<rustaxa::HostPillarAnchorStateReport>(request.effect_id, error_code.c_str());
  }
  return report;
}

rustaxa::HostFinalChainAccountFactsReport ExternalEvmPort::consensusLoadFinalChainAccountFacts(
    const rustaxa::HostFinalChainAccountFactsRequest& request) const {
  rustaxa::HostFinalChainAccountFactsReport report{};
  report.effect_id = request.effect_id;
  try {
    report.observed_block = impl_->final_chain->lastBlockNumber();
    report.accounts.reserve(request.addresses.size());
    for (const auto& requested : request.addresses) {
      const auto address = fromHostAddress(requested.bytes);
      const auto account = impl_->final_chain->getAccount(address, report.observed_block);
      rustaxa::HostFinalChainAccountFact fact{};
      fact.address = requested.bytes;
      fact.found = account.has_value();
      if (account) {
        fact.nonce = toBridgeU256(account->nonce);
        fact.balance = toBridgeU256(account->balance);
      }
      report.accounts.push_back(std::move(fact));
    }
    report.succeeded = true;
  } catch (const std::exception&) {
    return failedReport<rustaxa::HostFinalChainAccountFactsReport>(request.effect_id,
                                                                   "FINAL_CHAIN_ACCOUNT_FACTS_READ_FAILED");
  }
  return report;
}

rustaxa::HostEvmFinalizationReport ExternalEvmPort::consensusExecuteFinalization(
    const rustaxa::HostEvmFinalizationRequest& request) const {
  rustaxa::HostEvmFinalizationReport report{};
  report.effect_id = request.effect_id;
  try {
    PeriodData period_data(fromRustBytes(request.period_data_rlp));
    if (period_data.previous_block_cert_votes.size() != request.previous_cert_vote_rlps.size()) {
      throw std::runtime_error("FinalChain previous-cert vote count mismatch: period data has " +
                               std::to_string(period_data.previous_block_cert_votes.size()) +
                               ", executor request has " + std::to_string(request.previous_cert_vote_rlps.size()));
    }
    period_data.previous_block_cert_votes.clear();
    period_data.previous_block_cert_votes.reserve(request.previous_cert_vote_rlps.size());
    for (const auto& vote : request.previous_cert_vote_rlps) {
      period_data.previous_block_cert_votes.emplace_back(std::make_shared<PbftVote>(fromRustBytes(vote.data)));
    }
    std::vector<h256> finalized_dag_hashes;
    finalized_dag_hashes.reserve(request.finalized_dag_hashes.size());
    for (const auto& hash : request.finalized_dag_hashes) {
      finalized_dag_hashes.emplace_back(hash.hash.data(), h256::ConstructFromPointer);
    }
    std::shared_ptr<DagBlock> anchor;
    if (!request.anchor_block_rlp.empty()) {
      anchor = std::make_shared<DagBlock>(fromRustBytes(request.anchor_block_rlp));
    }

    auto future = impl_->final_chain->finalize(std::move(period_data), std::move(finalized_dag_hashes),
                                               request.blocks_per_year, std::move(anchor));
    if (request.synchronous) {
      future.wait();
    }
    report.succeeded = true;
    report.status = 0;
    report.last_block_number = impl_->final_chain->lastBlockNumber();
  } catch (const std::exception& error) {
    report.succeeded = false;
    report.status = 1;
    report.last_block_number = impl_->final_chain->lastBlockNumber();
    report.error_code = rust::String(std::string("FINALIZATION_EXECUTION_FAILED: ") + error.what());
  }
  return report;
}

class ConsensusProcess::Impl final {
 public:
  enum class LifecycleState : uint8_t { kStopped, kRunning };

  Impl(std::shared_ptr<ConsensusApplication> application, const FullNodeConfig& config,
       std::shared_ptr<final_chain::FinalChain> final_chain)
      : application(std::move(application)), signer(config), evm(std::move(final_chain)) {
    if (!this->application) {
      throw std::invalid_argument("ConsensusProcess requires ConsensusApplication");
    }
  }

  std::shared_ptr<ConsensusApplication> application;
  ConsensusProcessPort process;
  ConsensusSignerPort signer;
  ConsensusTransportPort transport;
  ExternalEvmPort evm;
  // Serializes complete public start/stop transitions, including joins. The
  // worker never acquires this mutex, so stop may retain it while joining.
  std::mutex lifecycle_mutex;
  // Protects only the worker handle and state; never held while joining or
  // while the native runner blocks.
  std::mutex state_mutex;
  std::thread worker;
  LifecycleState state{LifecycleState::kStopped};
};

ConsensusProcess::ConsensusProcess(std::shared_ptr<ConsensusApplication> application, const FullNodeConfig& config,
                                   std::shared_ptr<final_chain::FinalChain> final_chain)
    : impl_(std::make_unique<Impl>(std::move(application), config, std::move(final_chain))) {}

ConsensusProcess::~ConsensusProcess() { stop(); }

void ConsensusProcess::attachNetwork(std::weak_ptr<Network> network) {
  impl_->transport.impl_->attach(std::move(network));
}

void ConsensusProcess::start() {
  const std::scoped_lock lifecycle_lock(impl_->lifecycle_mutex);
  std::thread finished_worker;
  {
    const std::scoped_lock state_lock(impl_->state_mutex);
    if (impl_->state == Impl::LifecycleState::kRunning) {
      return;
    }
    if (impl_->worker.joinable()) {
      finished_worker = std::move(impl_->worker);
    }
  }
  if (finished_worker.joinable()) {
    finished_worker.join();
  }

  const std::scoped_lock state_lock(impl_->state_mutex);
  impl_->process.impl_->reset();
  impl_->state = Impl::LifecycleState::kRunning;
  try {
    impl_->worker = std::thread([impl = impl_.get()] {
      try {
        static_cast<void>(rustaxa::consensus_application_run(impl->application->service(), impl->process, impl->signer,
                                                             impl->transport, impl->evm));
      } catch (const std::exception& error) {
        std::fprintf(stderr, "native consensus process terminated: %s\n", error.what());
      } catch (...) {
        std::fprintf(stderr, "native consensus process terminated with an unknown error\n");
      }
      const std::scoped_lock state_lock(impl->state_mutex);
      impl->state = Impl::LifecycleState::kStopped;
    });
  } catch (...) {
    impl_->state = Impl::LifecycleState::kStopped;
    throw;
  }
}

void ConsensusProcess::stop() noexcept {
  const std::scoped_lock lifecycle_lock(impl_->lifecycle_mutex);
  std::thread worker;
  {
    const std::scoped_lock state_lock(impl_->state_mutex);
    if (impl_->state == Impl::LifecycleState::kStopped && !impl_->worker.joinable()) {
      return;
    }
    impl_->process.impl_->stop();
    worker = std::move(impl_->worker);
    impl_->state = Impl::LifecycleState::kStopped;
  }
  if (worker.joinable()) {
    worker.join();
  }
}

}  // namespace taraxa

#endif  // RUSTAXA_ENABLE
