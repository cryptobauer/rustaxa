#pragma once

#include <array>
#include <utility>

#include "rustaxa-bridge/ffi.rs.h"

namespace rustaxa::test {

/** Constructs the complete production-shaped root used by CXX boundary tests. */
inline rust::Box<BridgeConsensusApplication> createConsensusApplication(const BridgeStorage& storage,
                                                                        PbftServiceConfig pbft_config,
                                                                        uint32_t dag_expiry_limit = 32,
                                                                        uint16_t changing_interval = 10) {
  SortitionRuntimeConfig sortition{};
  sortition.threshold_upper = 0x100;
  sortition.difficulty_min = 1;
  sortition.difficulty_max = 10;
  sortition.difficulty_stale = 5;
  sortition.lambda_bound = 100;
  sortition.changes_count_for_average = 8;
  sortition.dag_efficiency_target_low = 5000;
  sortition.dag_efficiency_target_high = 10000;
  sortition.changing_interval = changing_interval;
  sortition.computation_interval = changing_interval;

  GasPricerConfig gas_pricer{};
  gas_pricer.percentile = 50;
  return create_consensus_application_from_storage(storage, std::array<uint8_t, 32>{1}, dag_expiry_limit, 100,
                                                   sortition, TransactionQueueConfig{16}, gas_pricer, 1'000'000,
                                                   std::move(pbft_config));
}

}  // namespace rustaxa::test
