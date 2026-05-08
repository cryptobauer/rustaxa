#pragma once

// Rust build overlay for ProposedBlocks:
// - legacy header is imported as ProposedBlocksOld
// - shim header provides a standalone Rust-backed ProposedBlocks facade

#pragma push_macro("ProposedBlocks")
#undef ProposedBlocks
#define ProposedBlocks ProposedBlocksOld
#include "../../../include/pbft/proposed_blocks.hpp"
#pragma pop_macro("ProposedBlocks")

#ifndef ProposedBlocks
#include "pbft/proposed_blocks_shim.hpp"
#endif
