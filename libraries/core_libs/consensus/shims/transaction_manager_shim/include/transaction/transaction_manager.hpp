#pragma once

// Rust build overlay for TransactionManager. Rust-enabled builds expose only
// the standalone shim facade; the legacy header/source remain untouched for
// pure C++ reference builds.
#include "transaction/transaction_manager_shim.hpp"
