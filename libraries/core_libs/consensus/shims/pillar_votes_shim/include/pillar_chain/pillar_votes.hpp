#pragma once

// Rust build overlay for PillarVotes:
// - legacy header is imported as PillarVotesOld
// - shim header provides a standalone PillarVotes facade for Rust bridge mode

#pragma push_macro("PillarVotes")
#undef PillarVotes
#define PillarVotes PillarVotesOld
#include "../../../include/pillar_chain/pillar_votes.hpp"
#pragma pop_macro("PillarVotes")

#ifndef PillarVotes
#include "pillar_chain/pillar_votes_shim.hpp"
#endif

