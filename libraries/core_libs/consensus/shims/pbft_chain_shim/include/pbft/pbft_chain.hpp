#pragma once

// Rust build overlay for PbftChain:
// - legacy header is imported as PbftChainOld
// - shim header provides a standalone Rust-backed PbftChain facade

#pragma push_macro("PbftChain")
#undef PbftChain
#define PbftChain PbftChainOld
#include "../../../include/pbft/pbft_chain.hpp"
#pragma pop_macro("PbftChain")

#ifndef PbftChain
#include "pbft/pbft_chain_shim.hpp"
#endif
