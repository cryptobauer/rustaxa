#pragma once

// Rust build overlay for VoteManager:
// - legacy header is imported as VoteManagerOld
// - shim header provides a standalone Rust-mode VoteManager overlay
//
// The complete public surface and live compatibility state are shim-owned.
// VoteManagerOld remains renamed only so the upstream implementation can be
// compiled as a pure-C++ reference artifact without defining VoteManager.

#pragma push_macro("VoteManager")
#undef VoteManager
#define VoteManager VoteManagerOld
#include "../../../include/vote_manager/vote_manager.hpp"
#pragma pop_macro("VoteManager")

#ifndef VoteManager
#include "vote_manager/vote_manager_shim.hpp"
#endif
