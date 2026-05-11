#pragma once

// Rust build overlay for GasPricer:
// - legacy header is imported as GasPricerOld
// - shim header provides a standalone GasPricer facade for Rust-backed mode.

#pragma push_macro("GasPricer")
#undef GasPricer
#define GasPricer GasPricerOld
#include "../../../include/transaction/gas_pricer.hpp"
#pragma pop_macro("GasPricer")

#ifndef GasPricer
#include "transaction/gas_pricer_shim.hpp"
#endif
