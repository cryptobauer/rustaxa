#include "consensus/finalization_transaction_order.hpp"

#include <cstdint>
#include <map>
#include <unordered_map>
#include <utility>

namespace taraxa::consensus {

void reorderTransactionsForExecution(SharedTransactions& transactions) {
  SharedTransactions ordered_transactions;
  std::unordered_map<addr_t, uint32_t> account_reverse_order;
  std::unordered_map<addr_t, val_t> account_nonce;

  for (uint32_t i = 0; i < transactions.size(); ++i) {
    const auto& transaction = transactions[i];
    auto reverse_order = account_reverse_order.find(transaction->getSender());
    if (reverse_order == account_reverse_order.end()) {
      auto nonce = account_nonce.find(transaction->getSender());
      if (nonce == account_nonce.end() || nonce->second < transaction->getNonce()) {
        account_nonce[transaction->getSender()] = transaction->getNonce();
      } else if (nonce->second > transaction->getNonce()) {
        account_reverse_order.insert({transaction->getSender(), i});
      }
    } else {
      reverse_order->second = i;
    }
  }

  if (account_reverse_order.empty()) {
    return;
  }

  std::unordered_map<addr_t, std::multimap<val_t, std::shared_ptr<Transaction>>> account_transactions;
  for (uint32_t i = 0; i < transactions.size(); ++i) {
    const auto& transaction = transactions[i];
    const auto reverse_order = account_reverse_order.find(transaction->getSender());
    if (reverse_order == account_reverse_order.end()) {
      ordered_transactions.push_back(transaction);
      continue;
    }

    account_transactions[transaction->getSender()].insert({transaction->getNonce(), transaction});
    if (reverse_order->second == i) {
      for (const auto& [nonce, ordered_transaction] : account_transactions[transaction->getSender()]) {
        static_cast<void>(nonce);
        ordered_transactions.push_back(ordered_transaction);
      }
    }
  }
  transactions = std::move(ordered_transactions);
}

}  // namespace taraxa::consensus
