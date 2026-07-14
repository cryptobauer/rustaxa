#pragma once

#include <memory>
#include <shared_mutex>
#include <unordered_map>

#include "common/vrf_wrapper.hpp"
#include "final_chain/final_chain.hpp"

namespace taraxa {

/**
 * Rust-mode KeyManager facade.
 *
 * Purpose:
 * - Preserve the public key-manager API while sourcing validator VRF public-key
 *   facts from the Rust FinalChain runtime instead of the C++ FinalChain
 *   compatibility method.
 *
 * Invariants:
 * - Returned keys are cached by validator address, matching the legacy
 *   KeyManager behavior.
 * - Missing or future-block facts return `nullptr`.
 */
class KeyManager {
 public:
  explicit KeyManager(std::shared_ptr<final_chain::FinalChain> final_chain);
  KeyManager(const KeyManager&) = delete;
  KeyManager(KeyManager&&) = delete;
  KeyManager& operator=(const KeyManager&) = delete;
  KeyManager& operator=(KeyManager&&) = delete;

  /**
   * Returns the validator VRF public key for a block-scoped DPoS snapshot.
   *
   * Inputs:
   * - `blk_n`: primary snapshot block, with the legacy neighbor fallback order
   *   of `blk_n`, `blk_n - 1` when nonzero, then `blk_n + 1`.
   * - `addr`: validator address.
   *
   * Output:
   * - shared cached VRF public key when Rust FinalChain has a key for the
   *   address in one of the queried snapshots.
   * - `nullptr` when no key is present or the snapshot is not yet available.
   */
  std::shared_ptr<vrf_wrapper::vrf_pk_t> getVrfKey(EthBlockNumber blk_n, const addr_t& addr);

 private:
  std::shared_mutex shim_vrf_keys_mutex_;
  std::unordered_map<addr_t, std::shared_ptr<vrf_wrapper::vrf_pk_t>> shim_vrf_keys_;
  std::shared_ptr<final_chain::FinalChain> shim_final_chain_;
};

}  // namespace taraxa
