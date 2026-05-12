#pragma once

// Temporary PbftManager overlay:
// - imports the upstream header unchanged
// - exposes shim-owned Rust-mode helper declarations to PbftManager sources
//
// This intentionally does not remap PbftManager to PbftManagerOld. A full
// PbftManager class overlay would be much larger and should only be introduced
// when the shim can own the manager routing without broad legacy fallback.
#include "../../../include/pbft/pbft_manager.hpp"

#include "pbft/pbft_manager_shim.hpp"
