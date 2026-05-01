#pragma once

// Rust build overlay for DbStorage:
// - legacy header is imported as DbStorageOld
// - shim header provides DbStorage facade forwarding to DbStorageOld

#pragma push_macro("DbStorage")
#undef DbStorage
#define DbStorage DbStorageOld
#include "../../../storage/include/storage/storage.hpp"
#pragma pop_macro("DbStorage")

#ifndef DbStorage
#include "storage/storage_shim.hpp"
#endif
