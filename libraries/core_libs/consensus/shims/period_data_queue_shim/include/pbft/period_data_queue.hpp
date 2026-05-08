#pragma once

// Rust build overlay for PeriodDataQueue:
// - legacy header is imported as PeriodDataQueueOld
// - shim header provides a standalone PeriodDataQueue facade for Rust-enabled mode

#pragma push_macro("PeriodDataQueue")
#undef PeriodDataQueue
#define PeriodDataQueue PeriodDataQueueOld
#include "../../../include/pbft/period_data_queue.hpp"
#pragma pop_macro("PeriodDataQueue")

#ifndef PeriodDataQueue
#include "pbft/period_data_queue_shim.hpp"
#endif
