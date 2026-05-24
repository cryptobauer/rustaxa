#pragma once

// Rust build overlay for VoteManager:
// - legacy header is imported as VoteManagerOld
// - shim header provides a Rust-mode VoteManager overlay that inherits from VoteManagerOld
//
// The override surface is shim-owned so Rust-mode reward-vote reset persistence
// can call the PBFT finalization storage appender while unimplemented behavior
// continues through the inherited VoteManagerOld state machine.

#pragma push_macro("VoteManager")
#undef VoteManager
#define VoteManager VoteManagerOld
#include "../../../include/vote_manager/vote_manager.hpp"
#pragma pop_macro("VoteManager")

#ifndef VoteManager
#include "vote_manager/vote_manager_shim.hpp"
#endif
