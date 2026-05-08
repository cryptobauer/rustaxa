#pragma once

// Rust build overlay for KeyManager:
// - legacy header is imported as KeyManagerOld
// - shim header provides KeyManager facade with Rust-mode behavior overrides

#pragma push_macro("KeyManager")
#undef KeyManager
#define KeyManager KeyManagerOld
#include "../../../include/key_manager/key_manager.hpp"
#pragma pop_macro("KeyManager")

#ifndef KeyManager
#include "key_manager/key_manager_shim.hpp"
#endif
