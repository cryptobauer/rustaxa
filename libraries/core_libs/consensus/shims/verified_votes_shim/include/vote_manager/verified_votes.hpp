#pragma once

// Rust build overlay for VerifiedVotes:
// - legacy header is imported as VerifiedVotesOld
// - shim header provides a standalone VerifiedVotes facade
//
// This overlay keeps legacy data model types (TwoTPlusOneVotedBlockType,
// StepVotes, RoundVerifiedVotesMap, ... ) available to all existing call sites
// while swapping only the concrete VerifiedVotes class in Rust-enabled mode.

#pragma push_macro("VerifiedVotes")
#undef VerifiedVotes
#define VerifiedVotes VerifiedVotesOld
#include "../../../include/vote_manager/verified_votes.hpp"
#pragma pop_macro("VerifiedVotes")

#ifndef VerifiedVotes
#include "vote_manager/verified_votes_shim.hpp"
#endif
