#ifdef RUSTAXA_ENABLE

#include "final_chain/final_chain.hpp"

namespace taraxa::final_chain {

void FinalChain::prune(EthBlockNumber blk_n) {
  const auto last_block_to_keep = blockHeader(blk_n);
  if (!last_block_to_keep) {
    return;
  }

  const auto evm_head = external_evm_state_api_.lastCommittedStateDescriptor().blk_num;
  if (evm_head >= last_block_to_keep->number) {
    std::vector<h256> state_roots_to_keep;
    for (auto block_to_keep = last_block_to_keep; block_to_keep && block_to_keep->number <= evm_head;
         block_to_keep = blockHeader(block_to_keep->number + 1)) {
      state_roots_to_keep.push_back(block_to_keep->state_root);
    }
    std::scoped_lock lock(state_api_mutex_);
    state_api_.prune(state_roots_to_keep, last_block_to_keep->number);
  }

  consensus_application_->service().prune_final_chain_before(last_block_to_keep->number);
}

}  // namespace taraxa::final_chain

#endif
