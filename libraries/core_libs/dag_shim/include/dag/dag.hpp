#pragma once

// Rust build overlay for Dag/PivotTree:
// - legacy header is imported as DagOld/PivotTreeOld
// - shim header provides standalone Dag/PivotTree facades

#pragma push_macro("Dag")
#pragma push_macro("PivotTree")
#undef Dag
#undef PivotTree
#define Dag DagOld
#define PivotTree PivotTreeOld
#include "../../../consensus/include/dag/dag.hpp"
#pragma pop_macro("PivotTree")
#pragma pop_macro("Dag")

#if !defined(Dag) && !defined(PivotTree)
#include "dag/dag_shim.hpp"
#endif
