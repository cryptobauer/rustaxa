#pragma once

#include <array>
#include <cstdint>

namespace taraxa {

/**
 * C++-owned transaction identity input for finalized-status verification.
 *
 * PBFT and TransactionManager compatibility code assemble canonical transaction
 * identity facts before the TransactionManager shim enriches them with the
 * latest external FinalChain account nonce. The enriched facts alone cross the
 * CXX boundary; this input therefore remains shim-owned and must not become a
 * Rust bridge DTO.
 *
 * Fields preserve the original input order, canonical transaction hash and
 * nonce bytes, and recovered sender address. Rust validates the index and hash
 * again when returning a finalized-status outcome.
 */
struct TransactionManagerVerifyNotFinalizedInput {
  uint64_t input_index = 0;
  std::array<uint8_t, 32> hash{};
  std::array<uint8_t, 32> transaction_nonce{};
  std::array<uint8_t, 20> sender{};
};

}  // namespace taraxa
