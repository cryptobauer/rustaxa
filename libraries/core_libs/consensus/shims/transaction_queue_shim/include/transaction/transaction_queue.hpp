#pragma once

// Rust build overlay for TransactionQueue:
// - legacy header is imported as TransactionQueueOld
// - shim header provides a standalone Rust-backed TransactionQueue facade

#pragma push_macro("TransactionQueue")
#undef TransactionQueue
#define TransactionQueue TransactionQueueOld
#include "../../../include/transaction/transaction_queue.hpp"
#pragma pop_macro("TransactionQueue")

#ifndef TransactionQueue
#include "transaction/transaction_queue_shim.hpp"
#endif
