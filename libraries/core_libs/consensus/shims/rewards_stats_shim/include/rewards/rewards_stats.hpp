#pragma once

// Rust build overlay for rewards::Stats:
// - legacy header is imported as StatsOld
// - shim header provides a Rust-backed rewards::Stats facade for Rust-enabled final-chain mode

#pragma push_macro("Stats")
#undef Stats
#define Stats StatsOld
#include "../../../../include/rewards/rewards_stats.hpp"
#pragma pop_macro("Stats")

#ifndef Stats
#include "rewards/rewards_stats_shim.hpp"
#endif
