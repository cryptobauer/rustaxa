#include <stdexcept>
#include <utility>

#include "pbft/pbft_block.hpp"
#include "pbft/proposed_blocks.hpp"
#include "storage/storage.hpp"

namespace taraxa {

std::array<uint8_t, 32> ProposedBlocks::toBridgeHash(const blk_hash_t& hash) { return hash.asArray(); }

blk_hash_t ProposedBlocks::fromBridgeHash(const std::array<uint8_t, 32>& hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

rust::Vec<uint8_t> ProposedBlocks::toBridgeBytes(const bytes& block_rlp) {
  rust::Vec<uint8_t> out;
  out.reserve(block_rlp.size());
  for (const auto byte : block_rlp) {
    out.push_back(byte);
  }
  return out;
}

std::shared_ptr<PbftBlock> ProposedBlocks::makeBlock(const rust::Vec<uint8_t>& block_rlp) {
  return std::make_shared<PbftBlock>(bytes(block_rlp.begin(), block_rlp.end()));
}

namespace {

::rust::Box<rustaxa::BridgeProposedBlocks> makeRustProposedBlocks(std::shared_ptr<DbStorage> const& storage_owner) {
  if (!storage_owner) {
    throw std::runtime_error("Rust-backed ProposedBlocks requires DbStorage");
  }
  return rustaxa::create_proposed_blocks_index_from_storage(storage_owner->rustStorage());
}

}  // namespace

ProposedBlocks::ProposedBlocks(std::shared_ptr<DbStorage> db)
    : storage_owner_(std::move(db)), rust_blocks_(makeRustProposedBlocks(storage_owner_)) {}

ProposedBlocks::~ProposedBlocks() = default;

bool ProposedBlocks::pushProposedPbftBlock(const std::shared_ptr<PbftBlock>& proposed_block, bool save_to_db) {
  if (!proposed_block) {
    throw std::runtime_error("Cannot push null proposed PBFT block");
  }

  std::unique_lock lock(proposed_blocks_mutex_);
  auto block_rlp = proposed_block->rlp(true);
  const auto period = proposed_block->getPeriod();
  const auto block_hash = proposed_block->getBlockHash();
  if (save_to_db) {
    try {
      return rust_blocks_->proposed_blocks_push_with_storage(period, toBridgeHash(block_hash),
                                                             toBridgeHash(proposed_block->getPivotDagBlockHash()),
                                                             toBridgeBytes(block_rlp));
    } catch (const std::exception& e) {
      throw std::runtime_error(e.what());
    } catch (...) {
      throw std::runtime_error("Failed to persist proposed PBFT block using Rust storage");
    }
  }

  return rust_blocks_->proposed_blocks_push(
      period, toBridgeHash(block_hash), toBridgeHash(proposed_block->getPivotDagBlockHash()), toBridgeBytes(block_rlp));
}

void ProposedBlocks::markBlockAsValid(const std::shared_ptr<PbftBlock>& proposed_block) {
  if (!proposed_block) {
    throw std::runtime_error("Cannot mark null proposed PBFT block as valid");
  }

  markBlockAsValid(proposed_block->getPeriod(), proposed_block->getBlockHash());
}

void ProposedBlocks::markBlockAsValid(PbftPeriod period, const blk_hash_t& block_hash) {
  std::unique_lock lock(proposed_blocks_mutex_);
  try {
    rust_blocks_->proposed_blocks_mark_valid(period, toBridgeHash(block_hash));
  } catch (const std::exception& e) {
    throw std::runtime_error(e.what());
  } catch (...) {
    throw std::runtime_error("Failed to mark proposed PBFT block as valid");
  }
}

size_t ProposedBlocks::restoreFromStorage() {
  if (!storage_owner_) {
    throw std::runtime_error("Cannot restore proposed PBFT blocks without Rust storage");
  }

  std::unique_lock lock(proposed_blocks_mutex_);
  try {
    return rust_blocks_->proposed_blocks_restore_from_storage();
  } catch (const std::exception& e) {
    throw std::runtime_error(e.what());
  } catch (...) {
    throw std::runtime_error("Failed to restore proposed PBFT blocks from storage");
  }
}

std::optional<std::pair<std::shared_ptr<PbftBlock>, bool>> ProposedBlocks::getPbftProposedBlock(
    PbftPeriod period, const blk_hash_t& block_hash) const {
  std::shared_lock lock(proposed_blocks_mutex_);
  const auto lookup = rust_blocks_->proposed_blocks_get(period, toBridgeHash(block_hash));
  if (!lookup.found) {
    return {};
  }

  return std::make_pair(makeBlock(lookup.block_rlp), lookup.is_valid);
}

std::optional<ProposedBlocks::ProposedBlockMetadata> ProposedBlocks::getPbftProposedBlockMetadata(
    PbftPeriod period, const blk_hash_t& block_hash) const {
  std::shared_lock lock(proposed_blocks_mutex_);
  const auto lookup = rust_blocks_->proposed_blocks_metadata(period, toBridgeHash(block_hash));
  if (!lookup.found) {
    return {};
  }

  return ProposedBlockMetadata{fromBridgeHash(lookup.pivot_hash), lookup.is_valid};
}

bool ProposedBlocks::isInProposedBlocks(PbftPeriod period, const blk_hash_t& block_hash) const {
  std::shared_lock lock(proposed_blocks_mutex_);
  return rust_blocks_->proposed_blocks_contains(period, toBridgeHash(block_hash));
}

void ProposedBlocks::cleanupProposedPbftBlocksByPeriod(PbftPeriod period) {
  std::unique_lock lock(proposed_blocks_mutex_);
  try {
    rust_blocks_->proposed_blocks_cleanup_with_storage(period);
  } catch (const std::exception& e) {
    throw std::runtime_error(e.what());
  } catch (...) {
    throw std::runtime_error("Failed to cleanup proposed PBFT blocks using storage-backed index");
  }
}

std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> ProposedBlocks::getProposedBlocks() const {
  std::shared_lock lock(proposed_blocks_mutex_);
  std::map<PbftPeriod, std::vector<std::shared_ptr<PbftBlock>>> result;
  auto snapshot = rust_blocks_->proposed_blocks_snapshot_entries();
  for (const auto& entry : snapshot) {
    result[entry.period].push_back(makeBlock(entry.block_rlp));
  }
  return result;
}

}  // namespace taraxa
