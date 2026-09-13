// Minimal recording leaves for exact extracted ExternalEvmStateOwner methods.
// A leaf records selection by throwing before executing or returning state.
#include <algorithm>
#include <array>
#include <cstdint>
#include <iostream>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <vector>
using EthBlockNumber = uint64_t;
using addr_t = std::string;
using u256 = uint64_t;
using h256 = uint64_t;
using bytes = std::vector<uint8_t>;
h256 ZeroHash() { return 0; }
struct DbException : std::runtime_error {
  using runtime_error::runtime_error;
};
struct Selected {
  std::string kind;
  uint64_t period;
};
namespace state_api {
struct Account {};
struct StateDescriptor {
  uint64_t blk_num;
};
struct ErrFutureBlock : std::runtime_error {
  using runtime_error::runtime_error;
};
struct EVMTransaction {
  std::optional<addr_t> to;
  addr_t from;
  uint64_t value{}, gas_price{}, gas{};
  bytes input;
};
struct LogRecord {
  addr_t address;
  std::vector<bytes> topics;
  bytes data;
};
struct ExecutionResult {
  bytes code_retval;
  std::vector<LogRecord> logs;
  uint64_t gas_used{};
  std::string code_err, consensus_err;
};
struct Tracing {};
struct Context {
  uint64_t author, gas_limit, timestamp, difficulty;
};
}  // namespace state_api
namespace rustaxa {
struct FinalChainNativeCall {
  uint64_t block_number{}, value{}, gas_price{}, gas_limit{};
  addr_t sender, receiver;
  bool receiver_found{};
  bytes input;
};
struct Topic {
  bytes topic;
};
struct Log {
  addr_t address;
  std::vector<Topic> topics;
  bytes data;
};
struct Outcome {
  bytes code_retval;
  std::vector<Log> logs;
  uint64_t gas_used{};
  std::string code_err, consensus_err;
};
}  // namespace rustaxa
addr_t toArray(const addr_t& x) { return x; }
uint64_t toBigEndianBytes(uint64_t x) { return x; }
bytes toRustBytes(const bytes& x) { return x; }
bytes fromRustBytes(const bytes& x) { return x; }
addr_t toAddress(const addr_t& x) { return x; }
namespace final_chain {
struct BlockHeader {
  uint64_t number{}, author{}, gas_limit{}, timestamp{};
  static uint64_t difficulty() { return 0; }
};
}  // namespace final_chain
namespace dev {
std::string asString(const bytes&) { return {}; }
}  // namespace dev
struct QueryClient {
  rustaxa::Outcome consensus_query_final_chain_native_call(rustaxa::FinalChainNativeCall request) {
    throw Selected{"native", request.block_number};
  }
};
struct Application {
  std::shared_ptr<QueryClient> query = std::make_shared<QueryClient>();
  std::shared_ptr<QueryClient>* queryClient() { return &query; }
};
struct StateAPI {
  uint64_t head{};
  state_api::StateDescriptor get_last_committed_state_descriptor() const { return {head}; }
  std::optional<state_api::Account> get_account(uint64_t p, const addr_t&) const { throw Selected{"account", p}; }
  h256 get_account_storage(uint64_t p, const addr_t&, u256) const { throw Selected{"storage", p}; }
  bytes get_code_by_address(uint64_t p, const addr_t&) const { throw Selected{"code", p}; }
  state_api::ExecutionResult dry_run_transaction(uint64_t p, state_api::Context,
                                                 const state_api::EVMTransaction&) const {
    throw Selected{"call", p};
  }
  bytes trace(uint64_t p, state_api::Context, const std::vector<state_api::EVMTransaction>&,
              const std::vector<state_api::EVMTransaction>&, std::optional<state_api::Tracing>) const {
    throw Selected{"trace", p};
  }
};
class ExternalEvmStateOwner {
 public:
  uint64_t app_head{};
  StateAPI state_api_;
  mutable std::mutex mutex_;
  void ensureReadableLocked() const {}
  uint64_t lastBlockNumber() const { return app_head; }
  state_api::StateDescriptor lastCommittedStateDescriptor() const { return {state_api_.head}; }
  std::shared_ptr<Application> application() const { return std::make_shared<Application>(); }
  std::shared_ptr<final_chain::BlockHeader> blockHeader(uint64_t p) const {
    auto h = std::make_shared<final_chain::BlockHeader>();
    h->number = p;
    return h;
  }
  std::optional<state_api::Account> account(const addr_t&, std::optional<EthBlockNumber>) const;
  h256 accountStorage(const addr_t&, const u256&, std::optional<EthBlockNumber>) const;
  bytes code(const addr_t&, std::optional<EthBlockNumber>) const;
  state_api::ExecutionResult call(const state_api::EVMTransaction&, std::optional<EthBlockNumber>) const;
  std::string trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>, EthBlockNumber,
                    std::optional<state_api::Tracing>) const;
};
// EXTRACTED_METHODS
int main() {
  for (const auto heads :
       std::vector<std::pair<uint64_t, uint64_t>>{{0, 0}, {10, 7}, {7, 10}, {UINT64_MAX, UINT64_MAX - 1}}) {
    for (int op = 0; op < 6; ++op) {
      for (const auto requested : std::vector<std::optional<uint64_t>>{std::nullopt, 0, 6, 7, 8, 10, 11, UINT64_MAX}) {
        if (op == 5 && !requested) continue;
        ExternalEvmStateOwner owner;
        owner.app_head = heads.first;
        owner.state_api_.head = heads.second;
        std::cout << "{\"op\":" << op << ",\"app_head\":" << heads.first << ",\"concrete_head\":" << heads.second
                  << ",\"requested\":";
        if (requested)
          std::cout << *requested;
        else
          std::cout << "null";
        try {
          if (op == 0) owner.account("", requested);
          if (op == 1) owner.accountStorage("", 0, requested);
          if (op == 2) owner.code("", requested);
          if (op == 3 || op == 4) {
            state_api::EVMTransaction tx;
            if (op == 4) tx.to = "0x00000000000000000000000000000000000000FE";
            owner.call(tx, requested);
          }
          if (op == 5) owner.trace({}, {}, *requested, {});
          std::cout << ",\"kind\":\"" << (op == 1 ? "zero_storage" : "empty_code") << "\"";
        } catch (const Selected& selected) {
          std::cout << ",\"kind\":\"" << selected.kind << "\",\"period\":" << selected.period;
        } catch (const std::exception& error) {
          std::cout << ",\"error\":\"" << error.what() << "\"";
        }
        std::cout << "}\n";
      }
    }
  }
}
