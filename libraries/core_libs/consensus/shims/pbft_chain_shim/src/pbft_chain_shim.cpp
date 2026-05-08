#include <libdevcore/CommonJS.h>

#include <array>
#include <sstream>
#include <stdexcept>
#include <utility>

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

rustaxa::PbftChainHeadPayload make_head_payload(blk_hash_t const& head_hash, PbftPeriod size, PbftPeriod non_empty_size,
                                                blk_hash_t const& last_pbft_block_hash,
                                                blk_hash_t const& last_non_null_anchor_hash) {
  rustaxa::PbftChainHeadPayload payload;
  payload.head_hash = to_bridge_hash(head_hash);
  payload.size = size;
  payload.non_empty_size = non_empty_size;
  payload.last_pbft_block_hash = to_bridge_hash(last_pbft_block_hash);
  payload.last_non_null_anchor_hash = to_bridge_hash(last_non_null_anchor_hash);
  return payload;
}

rustaxa::PbftChainHeadPayload default_head_payload() {
  return make_head_payload(blk_hash_t(), 0, 0, blk_hash_t(), blk_hash_t());
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

blk_hash_t recover_last_non_null_anchor(blk_hash_t last_pbft_block_hash, DbStorage& db) {
  while (last_pbft_block_hash) {
    auto pbft_block = db.getPbftBlock(last_pbft_block_hash);
    if (!pbft_block.has_value()) {
      throw std::runtime_error("Cannot recover PBFT chain head: missing PBFT block " + last_pbft_block_hash.toString());
    }
    auto anchor = pbft_block->getPivotDagBlockHash();
    if (anchor) {
      return anchor;
    }
    last_pbft_block_hash = pbft_block->getPrevBlockHash();
  }
  return {};
}

rustaxa::PbftChainHeadPayload parse_persisted_head(std::string const& pbft_head, DbStorage& db) {
  Json::Value doc;
  std::istringstream(pbft_head) >> doc;
  auto last_pbft_block_hash = blk_hash_t(doc["last_pbft_block_hash"].asString());
  return make_head_payload(blk_hash_t(doc["head_hash"].asString()), doc["size"].asUInt64(),
                           doc["non_empty_size"].asUInt64(), last_pbft_block_hash,
                           recover_last_non_null_anchor(last_pbft_block_hash, db));
}

}  // namespace

PbftChain::PbftChain(addr_t node_addr, std::shared_ptr<DbStorage> db) : db_(std::move(db)) {
  (void)node_addr;
  LOG_OBJECTS_CREATE("PBFT_CHAIN");

  auto head_payload = default_head_payload();
  auto pbft_head_str = db_->getPbftHead(from_bridge_hash(head_payload.head_hash));
  if (pbft_head_str.empty()) {
    rust_chain_ = rustaxa::create_pbft_chain(head_payload);
    auto head_json_str = getJsonStr();
    db_->savePbftHead(getHeadHash(), head_json_str);
    LOG(log_nf_) << "Initialize PBFT chain head " << head_json_str;
    return;
  }

  head_payload = parse_persisted_head(pbft_head_str, *db_);
  rust_chain_ = rustaxa::create_pbft_chain(head_payload);
  LOG(log_nf_) << "Retrieve from DB, PBFT chain head " << getJsonStr();
}

PbftChain::~PbftChain() = default;

blk_hash_t PbftChain::getHeadHash() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(rust_chain_.value()->pbft_chain_head().head_hash);
}

PbftPeriod PbftChain::getPbftChainSize() const {
  std::shared_lock lock(chain_head_access_);
  return rust_chain_.value()->pbft_chain_head().size;
}

PbftPeriod PbftChain::getPbftChainSizeExcludingEmptyPbftBlocks() const {
  std::shared_lock lock(chain_head_access_);
  return rust_chain_.value()->pbft_chain_head().non_empty_size;
}

blk_hash_t PbftChain::getLastPbftBlockHash() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(rust_chain_.value()->pbft_chain_head().last_pbft_block_hash);
}

blk_hash_t PbftChain::getLastNonNullPbftBlockAnchor() const {
  std::shared_lock lock(chain_head_access_);
  return from_bridge_hash(rust_chain_.value()->pbft_chain_head().last_non_null_anchor_hash);
}

bool PbftChain::findPbftBlockInChain(taraxa::blk_hash_t const& pbft_block_hash) {
  return db_->pbftBlockInDb(pbft_block_hash);
}

PbftBlock PbftChain::getPbftBlockInChain(const taraxa::blk_hash_t& pbft_block_hash) {
  auto pbft_block = db_->getPbftBlock(pbft_block_hash);
  if (!pbft_block.has_value()) {
    LOG(log_er_) << "Cannot find PBFT block hash " << pbft_block_hash << " in DB";
    throw std::runtime_error("Cannot find PBFT block hash " + pbft_block_hash.toString() + " in DB");
  }
  return *pbft_block;
}

void PbftChain::updatePbftChain(blk_hash_t const& pbft_block_hash, blk_hash_t const& anchor_hash) {
  std::scoped_lock lock(chain_head_access_);
  rust_chain_.value()->pbft_chain_update(to_bridge_hash(pbft_block_hash), to_bridge_hash(anchor_hash));
}

bool PbftChain::checkPbftBlockValidation(const std::shared_ptr<PbftBlock>& pbft_block) const {
  std::shared_lock lock(chain_head_access_);
  auto validation = rust_chain_.value()->pbft_chain_validate_block(pbft_block->getPeriod(),
                                                                   to_bridge_hash(pbft_block->getPrevBlockHash()));
  if (validation.code == kPbftValidationValid) {
    return true;
  }
  if (validation.code == kPbftValidationPeriodMismatch) {
    LOG(log_er_) << "Pbft validation failed. PBFT chain size " << rust_chain_.value()->pbft_chain_head().size
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
  return head_json_string(rust_chain_.value()->pbft_chain_head());
}

std::string PbftChain::getJsonStrForBlock(blk_hash_t const& block_hash, bool null_anchor) const {
  std::shared_lock lock(chain_head_access_);
  auto projected = rust_chain_.value()->pbft_chain_project_legacy_json_head(to_bridge_hash(block_hash), !null_anchor);
  return head_json_string(projected);
}

std::ostream& operator<<(std::ostream& strm, PbftChain const& pbft_chain) {
  strm << pbft_chain.getJsonStr();
  return strm;
}

}  // namespace taraxa
