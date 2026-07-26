#pragma once

#include <config/config.hpp>

#include "pbft/period_data.hpp"

namespace taraxa {

/**
 * Persisted sortition threshold transition used by the Rust storage overlay.
 *
 * The payload preserves the legacy RLP field order so Rust-mode storage can
 * read and write the same database records without retaining the legacy
 * SortitionParamsManager facade.
 */
struct SortitionParamsChange {
  PbftPeriod period = 0;
  VrfParams vrf_params;
  uint16_t interval_efficiency = 0;

  SortitionParamsChange() = default;
  SortitionParamsChange(PbftPeriod period, uint16_t efficiency, const VrfParams& vrf);
  static SortitionParamsChange from_rlp(const dev::RLP& rlp);
  bytes rlp() const;
};

}  // namespace taraxa
