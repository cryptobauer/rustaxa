#pragma once

// Rust build overlay for PbftManager:
// - legacy header is imported as PbftManagerOld
// - shim header provides the Rust-mode PbftManager facade

namespace taraxa {
class PbftManagerOld;
}

#pragma push_macro("PbftManager")
#undef PbftManager
#define PbftManager PbftManagerOld
#include "../../../include/pbft/pbft_manager.hpp"
#pragma pop_macro("PbftManager")

#ifndef PbftManager
#include "pbft/pbft_manager_shim.hpp"
#endif
