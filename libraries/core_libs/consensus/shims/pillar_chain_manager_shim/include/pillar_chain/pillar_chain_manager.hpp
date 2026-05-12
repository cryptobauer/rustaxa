#pragma once

// Rust build overlay for PillarChainManager:
// - legacy header is imported as PillarChainManagerOld
// - shim header provides a standalone PillarChainManager facade for Rust-enabled mode

#pragma push_macro("PillarChainManager")
#undef PillarChainManager
#define PillarChainManager PillarChainManagerOld
#include "../../../include/pillar_chain/pillar_chain_manager.hpp"
#pragma pop_macro("PillarChainManager")

#ifndef PillarChainManager
#include "pillar_chain/pillar_chain_manager_shim.hpp"
#endif
