#pragma once

// Rust build overlay for DagManager:
// - legacy header is imported as DagManagerOld
// - shim header provides a standalone DagManager facade for Rust-enabled mode

#pragma push_macro("DagManager")
#undef DagManager
#define DagManager DagManagerOld
#include "../../../include/dag/dag_manager.hpp"
#pragma pop_macro("DagManager")

#ifndef DagManager
#include "dag/dag_manager_shim.hpp"
#endif
