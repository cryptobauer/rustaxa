#pragma once

// Rust build overlay for SortitionParamsManager:
// - legacy header is imported as SortitionParamsManagerOld
// - shim header provides a standalone SortitionParamsManager facade

#pragma push_macro("SortitionParamsManager")
#undef SortitionParamsManager
#define SortitionParamsManager SortitionParamsManagerOld
#include "../../../consensus/include/dag/sortition_params_manager.hpp"
#pragma pop_macro("SortitionParamsManager")

#ifndef SortitionParamsManager
#include "dag/sortition_params_manager_shim.hpp"
#endif
