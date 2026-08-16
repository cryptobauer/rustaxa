#pragma once

#include "transaction/transaction.hpp"

namespace taraxa::consensus {

/**
 * Restores nonce order for senders whose DAG transaction order regressed.
 *
 * Transactions from unaffected senders retain their relative position. For
 * each affected sender, all of its transactions are ordered by nonce and
 * emitted at that sender's final original position. Empty and already ordered
 * inputs are unchanged; equal nonces retain multimap-equivalent ordering.
 */
void reorderTransactionsForExecution(SharedTransactions& transactions);

}  // namespace taraxa::consensus
