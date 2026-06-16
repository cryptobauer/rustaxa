#include "key_manager/key_manager.hpp"

#include <mutex>

namespace taraxa {

namespace {
static const vrf_wrapper::vrf_pk_t kEmptyVrfKey;

vrf_wrapper::vrf_pk_t fromRustVrfKey(const rust::Vec<uint8_t>& key) {
  if (key.empty()) {
    return {};
  }
  return vrf_wrapper::vrf_pk_t(dev::bytes(key.begin(), key.end()));
}
}

KeyManager::KeyManager(std::shared_ptr<final_chain::FinalChain> final_chain)
    : KeyManagerOld(final_chain), shim_final_chain_(std::move(final_chain)) {}

std::shared_ptr<vrf_wrapper::vrf_pk_t> KeyManager::getVrfKey(EthBlockNumber blk_n, const addr_t& addr) {
  {
    std::shared_lock lock(shim_vrf_keys_mutex_);
    if (const auto it = shim_vrf_keys_.find(addr); it != shim_vrf_keys_.end()) {
      return it->second;
    }
  }

  auto read_key = [&](EthBlockNumber block_number) -> std::shared_ptr<vrf_wrapper::vrf_pk_t> {
    auto key = fromRustVrfKey(
        shim_final_chain_->rustFinalChainForRust().get_vrf_key_at_block(static_cast<uint64_t>(block_number),
                                                                        addr.asArray()));
    if (key == kEmptyVrfKey) {
      return nullptr;
    }
    std::unique_lock lock(shim_vrf_keys_mutex_);
    return shim_vrf_keys_.insert_or_assign(addr, std::make_shared<vrf_wrapper::vrf_pk_t>(std::move(key))).first->second;
  };

  try {
    if (auto key = read_key(blk_n)) {
      return key;
    }

    if (blk_n > 0) {
      if (auto key = read_key(blk_n - 1)) {
        return key;
      }
    }

    if (auto key = read_key(blk_n + 1)) {
      return key;
    }
  } catch (state_api::ErrFutureBlock&) {
    return nullptr;
  }

  return nullptr;
}

}  // namespace taraxa
