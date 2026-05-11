#pragma once

// Rust build overlay for TransactionManager:
// - legacy header is imported as TransactionManagerOld
// - shim header provides a standalone TransactionManager facade for Rust-enabled mode

#pragma push_macro("TransactionManager")
#undef TransactionManager
#define TransactionManager TransactionManagerOld
#include "../../../include/transaction/transaction_manager.hpp"
#pragma pop_macro("TransactionManager")

#ifndef TransactionManager
#include "transaction/transaction_manager_shim.hpp"
#endif
