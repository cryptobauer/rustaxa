#pragma once

// Rust build overlay for FinalChain:
// - legacy header is imported as FinalChainOld
// - shim header provides FinalChain facade forwarding to FinalChainOld

#pragma push_macro("FinalChain")
#undef FinalChain
#define FinalChain FinalChainOld
#include "../../../consensus/include/final_chain/final_chain.hpp"
#pragma pop_macro("FinalChain")

#ifndef FinalChain
#include "final_chain/final_chain_shim.hpp"
#endif

