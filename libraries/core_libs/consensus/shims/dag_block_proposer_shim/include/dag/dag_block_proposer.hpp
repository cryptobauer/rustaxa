#pragma once

// Rust build overlay for DagBlockProposer:
// - legacy header is imported as DagBlockProposerOld
// - shim header provides a standalone DagBlockProposer facade for Rust-enabled mode

#pragma push_macro("DagBlockProposer")
#undef DagBlockProposer
#define DagBlockProposer DagBlockProposerOld
#include "../../../include/dag/dag_block_proposer.hpp"
#pragma pop_macro("DagBlockProposer")

#ifndef DagBlockProposer
#include "dag/dag_block_proposer_shim.hpp"
#endif
