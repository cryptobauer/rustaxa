#include <libdevcore/CommonJS.h>

#include <array>
#include <mutex>
#include <stdexcept>

#include "pbft/pbft_chain.hpp"
#include "storage/storage.hpp"

namespace taraxa {
namespace {

constexpr uint8_t kPbftValidationValid = 0;
constexpr uint8_t kPbftValidationPeriodMismatch = 1;
constexpr uint8_t kPbftValidationPreviousHashMismatch = 2;
std::array<uint8_t, 32> to_bridge_hash(blk_hash_t const& hash) { return hash.asArray(); }

blk_hash_t from_bridge_hash(std::array<uint8_t, 32> const& hash) {
  return blk_hash_t(hash.data(), blk_hash_t::ConstructFromPointer);
}

Json::Value head_json(rustaxa::PbftChainHeadPayload const& head) {
  Json::Value json;
  json["head_hash"] = from_bridge_hash(head.head_hash).toString();
  json["size"] = static_cast<Json::Value::UInt64>(head.size);
  json["non_empty_size"] = static_cast<Json::Value::UInt64>(head.non_empty_size);
  json["last_pbft_block_hash"] = from_bridge_hash(head.last_pbft_block_hash).toString();
  return json;
}

std::string head_json_string(rustaxa::PbftChainHeadPayload const& head) { return head_json(head).toStyledString(); }

}  // namespace

PbftChain::PbftChain([[maybe_unused]] addr_t node_addr, std::shared_ptr<DbStorage> db) {
  LOG_OBJECTS_CREATE("PBFT_CHAIN");

  pbft_service_ = std::make_shared<PbftService>(rustaxa::create_pbft_chain_service_from_storage(db->rustStorage()));
  if (pbft_service_->service().pbft_chain_initialized_default()) {
    LOG(log_nf_) << "Initialize PBFT chain head " << getJsonStr();
    return;
  }
  LOG(log_nf_) << "Retrieve from DB, PBFT chain head " << getJsonStr();
}

PbftChain::PbftChain([[maybe_unused]] addr_t node_addr, SharedPbftService pbft_service)
    : pbft_service_(std::move(pbft_service)) {
  if (!pbft_service_) {
    throw std::invalid_argument("PBFT chain requires a shared PBFT service");
  }
  LOG_OBJECTS_CREATE("PBFT_CHAIN");
  if (pbft_service_->service().pbft_chain_initialized_default()) {
    LOG(log_nf_) << "Initialize PBFT chain head " << getJsonStr();
    return;
  }
  LOG(log_nf_) << "Retrieve from DB, PBFT chain head " << getJsonStr();
}

PbftChain::~PbftChain() = default;

blk_hash_t PbftChain::getHeadHash() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(pbft_service_->service().pbft_chain_head().head_hash);
}

PbftPeriod PbftChain::getPbftChainSize() const {
  std::shared_lock lock(chain_head_access_);
  return pbft_service_->service().pbft_chain_head().size;
}

PbftPeriod PbftChain::getPbftChainSizeExcludingEmptyPbftBlocks() const {
  std::shared_lock lock(chain_head_access_);
  return pbft_service_->service().pbft_chain_head().non_empty_size;
}

blk_hash_t PbftChain::getLastPbftBlockHash() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(pbft_service_->service().pbft_chain_head().last_pbft_block_hash);
}

blk_hash_t PbftChain::getLastNonNullPbftBlockAnchor() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(pbft_service_->service().pbft_chain_head().last_non_null_anchor_hash);
}

bool PbftChain::findPbftBlockInChain(taraxa::blk_hash_t const& pbft_block_hash) {
  return pbft_service_->service().pbft_chain_block_exists(to_bridge_hash(pbft_block_hash));
}

PbftBlock PbftChain::getPbftBlockInChain(const taraxa::blk_hash_t& pbft_block_hash) {
  auto pbft_block = pbft_service_->service().pbft_chain_block_rlp(to_bridge_hash(pbft_block_hash));
  if (!pbft_block.found) {
    LOG(log_er_) << "Cannot find PBFT block hash " << pbft_block_hash << " in DB";
    throw std::runtime_error("Cannot find PBFT block hash " + pbft_block_hash.toString() + " in DB");
  }
  return PbftBlock(bytes(pbft_block.block_rlp.begin(), pbft_block.block_rlp.end()));
}

void PbftChain::updatePbftChain(blk_hash_t const& pbft_block_hash, blk_hash_t const& anchor_hash) {
  std::scoped_lock lock(chain_head_access_);
  pbft_service_->service().pbft_chain_update(to_bridge_hash(pbft_block_hash), to_bridge_hash(anchor_hash));
}

bool PbftChain::checkPbftBlockValidation(const std::shared_ptr<PbftBlock>& pbft_block) const {
  std::shared_lock lock(chain_head_access_);
  auto validation = pbft_service_->service().pbft_chain_validate_block(pbft_block->getPeriod(),
                                                                       to_bridge_hash(pbft_block->getPrevBlockHash()));
  if (validation.code == kPbftValidationValid) {
    return true;
  }
  if (validation.code == kPbftValidationPeriodMismatch) {
    LOG(log_er_) << "Pbft validation failed. PBFT chain size " << pbft_service_->service().pbft_chain_head().size
                 << ". Pbft block period: " << pbft_block->getPeriod() << " for block " << pbft_block->getBlockHash();
    return false;
  }
  if (validation.code == kPbftValidationPreviousHashMismatch) {
    LOG(log_er_) << "PBFT chain last block hash " << from_bridge_hash(validation.expected_prev_hash)
                 << " Invalid PBFT prev block hash " << pbft_block->getPrevBlockHash() << " in block "
                 << pbft_block->getBlockHash();
    return false;
  }
  throw std::runtime_error("Unknown PBFT validation result code: " + std::to_string(validation.code));
}

std::string PbftChain::getJsonStr() const {
  std::shared_lock lock(chain_head_access_);
  return head_json_string(pbft_service_->service().pbft_chain_head());
}

std::string PbftChain::getJsonStrForBlock(blk_hash_t const& block_hash, bool null_anchor) const {
  std::shared_lock lock(chain_head_access_);
  auto projected =
      pbft_service_->service().pbft_chain_project_legacy_json_head(to_bridge_hash(block_hash), !null_anchor);
  return head_json_string(projected);
}

std::ostream& operator<<(std::ostream& strm, PbftChain const& pbft_chain) {
  strm << pbft_chain.getJsonStr();
  return strm;
}

}  // namespace taraxa
