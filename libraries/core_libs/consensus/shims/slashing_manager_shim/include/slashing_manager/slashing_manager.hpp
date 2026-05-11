#pragma once

// Rust build overlay for SlashingManager:
// - legacy header is imported as SlashingManagerOld
// - shim header provides a standalone SlashingManager facade

#pragma push_macro("SlashingManager")
#undef SlashingManager
#define SlashingManager SlashingManagerOld
#include "../../../include/slashing_manager/slashing_manager.hpp"
#pragma pop_macro("SlashingManager")

#ifndef SlashingManager
#include "slashing_manager/slashing_manager_shim.hpp"
#endif
